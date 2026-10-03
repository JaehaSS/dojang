# Praxis — Architecture (코드 구조)

> 코드베이스의 **현재 구조**를 기술하는 living doc. 설계 의도는 [DESIGN.md](../DESIGN.md)·설계·PRD 참조.
> 마지막 갱신: 2026-09-09 (홈 독립 프로젝트 에디터·창별 원본 경로와 PTY).

## 개요

Rust + Tauri 2 데스크톱 앱. AI 코딩 에이전트 오케스트레이션 IDE. 병렬 Task·세션 메모리·검토 가능한 L1 회고·MCP 서버 연동을 제공한다. 메인 창과 같은 프로세스의 `aquarium` 보조 창이 Task 상태를 항상 위 soft pixel-art 물멍 장면으로 투영한다. 작업 완료 처리는 `AwaitingReview → Finalizing → Done|Discarded` 단독 점유로 보호한다. 메모리 주입 이력과 승인/폐기는 보존하지만, 개별 메모리의 인과적 품질 데이터가 충분해질 때까지 confidence로 자동 랭킹·제외하지 않는다. MCP는 기존 사용자 `.mcp.json`을 보존하고 격리 worktree에 새로 만든 파일만 task 단위로 정리한다.

## 기술 스택

| 레이어 | 기술 |
|--------|------|
| 데스크톱 셸 | Tauri 2 (IPC + 이벤트) |
| 백엔드 | Rust, std::thread (PTY I/O), portable-pty, nix(cfg unix), base64, anyhow |
| 프론트엔드 | React 19 + TypeScript + Vite (Node 20+ 필요) |
| 코드 에디터 | Monaco (@monaco-editor/react, **로컬 번들 + ?worker** — CDN 미사용) |
| 터미널 | xterm.js + @xterm/addon-fit |
| 스타일 | Tailwind CSS (DESIGN.md 토큰 미러) |

## 디렉토리 / 레이어

```
src-tauri/                # Rust 백엔드
  src/
    main.rs               # 엔트리 → praxis_lib::run()
    lib.rs                # Tauri Builder: setup(비동기 DB풀·모듈 migrate) + command/state 등록 + 종료 훅 + 백그라운드 루프 spawn(schedule::runner::tick_loop)
    aquarium.rs           # 어항 창 상호작용 정본: click-through/focus 전환, 전역 단축키, 숨김·작업 열기 command
    commands.rs           # Tauri 어댑터: task_create/list/diff_stat/approve/discard/write/resize + fs_tree/fs_read/fs_write(에디터) + AppState. approve/discard는 AwaitingReview CAS 점유·convo_active 가드 후 worktree 부작용 수행
    knowledge/vault/     # 개인 자료·불변 Markdown revision. manual_analysis_status는 수동 review/job/trigger 결과를 읽고, proposals의 save_result는 기존 operations와 accepted 상태로 저장 결과를 판정. 계약: designs/wiki-authoring-design.md D5 / ADR 0186
    hotkeys.rs            # 글로벌 단축키 **단일 디스패처**: 플러그인은 앱당 1회 등록·핸들러도 전역 1개라 "어느 키가 눌렸는가" 판별을 여기서만 하고 마을/음성에 위임(DR-1). 기능 모듈이 서로를 알지 않게 하는 지점
    voice/{mod,capture,stt}.rs # ★ 음성 입력: push-to-talk 3상태 머신 + cpal 캡처(!Send라 전용 스레드) + hound 인메모리 WAV + OpenAI 호환 전사 클라이언트. STT는 의존성이 아니라 **HTTP 계약**이라 trait·사이드카 spawn 없음(ADR 0103). 상태 전이는 캡처 스레드가 소유 — Released가 오지 않는 60초 자동 종료와 한 경로에서 만나야 Idle 복귀가 보장된다. 이벤트 `voice://state|transcript|error`
    pty/mod.rs            # ★ PTY 코어 (Tauri 비의존): PtySession, PtyEvent
    project_editor/      # canonical root별 독립 창 등록·호출 창 기반 파일 IPC·본문 CAS·창별 PTY/session/출력 sequence. Destroyed 정리, 메인 닫기는 셸만 정리. 계약: designs/home-project-editor-terminal-design.md / ADR 0187
    worktree/mod.rs       # ★ git worktree 관리 (셸아웃, Tauri 비의존): Worktree(create/diff_stat/diff_detailed/approve/discard) + FileDiff. task 생성 MCP만 staging 제외 가능
    worktree/conflict.rs  # ★ 머지 충돌 해소 (Tauri 비의존): 충돌을 **worktree 안에 가둔다** — repo가 아니라 worktree에서 base를 역방향 머지(ours=작업/theirs=base)해 begin/resolve/finish/abort. repo는 어떤 순간에도 MERGE_HEAD 상태가 되지 않고(테스트가 단언), 세션 상태는 DB가 아니라 MERGE_HEAD 존재가 정본(ADR 0100)
    rewind/mod.rs         # ★ 대화 되감기 (Tauri 비의존): 요약 프롬프트 + nonce 가드 파서. 파일은 worktree::restore_to_checkpoint로 진짜 되감고 대화는 새 세션+요약으로 **재구성**(벤더 CLI가 컨텍스트 절단을 안 줌). 영속은 convo_checkpoints + convo_events.rewound_at(논리 삭제)
    fsapi/mod.rs          # ★ worktree 스코프 파일시스템 API (Tauri 비의존): build_tree/read_file/write_file + safe_join(절대·.. 거부, **심볼릭(dangling 포함)·심볼릭 조상 차단**, 2MB 캡). 에디터 백엔드
    db/mod.rs             # ★ SQLite 영속화 (sqlx WAL): Task, Finalizing CAS, task_events, init_pool, insert/update_state/list, 재시작 복원
    memory/file.rs        # ★ 파일형 메모리(설계 2026-09-13, P1): 창고 MEMORY.md/USER.md가 정본, 마커블록으로 worktree 컨텍스트 파일에 투영. DB 티어·FTS5·승인·증거 재검증(P2)은 1.0에서 제거
    embed/mod.rs          # 로컬 임베딩 (fastembed BGE-small 384d, OnceLock 캐시) — knowledge/graph RAG에 임베딩 주입
    convo/mod.rs          # ★ 구조화 대화 (Tauri 비의존): Vendor(claude/codex/agy·gemini→agy)별 run_turn(한 턴, on_spawn pgid 훅) + JSONL→ConvoEvent 정규화 파서(실측 fixture 테스트) — 영속은 convo_events + tasks.convo_session_id, 대화 작업은 PTY 없이 이 경로(ActiveTask.session=None)
    designmode/{capture,editor,attachments,pasted}.rs # 프리뷰 요소·에디터/붙여넣기 이미지 캡처 저장 + task 캡처 디렉터리로 첨부 경계 제한
    preview_workbench/    # task별 요청 수락 기록·재전송 복구 + 별도 창 툴바 전용 relay/publish plugin
    commands/preview_workbench.rs # 실제 URL·task 지원 범위 확인 후 기존 conversation 수락 경로 호출
    capture/invoke.rs     # 캡처 호출 표면: 실행 프로파일 해석(profile)·`--output-format json` 봉투 파싱(parse_envelope)만 남는다 — 자동 추출·수동 회고 파이프라인(selfimprove/si_proposals 포함)은 1.0에서 함께 제거됐다
    interview/mod.rs      # ★ 수렴 인터뷰 (Tauri 비의존): 지시문 명확도 3차원 채점 + 부족 차원 객관식 질문 → Goal Contract 초안 결정화. 레포 요약(8KB)은 검토 콘텐츠일 뿐이며 nonce 이후 JSON만 신뢰(스푸핑 차단). 소프트 게이트 — 어떤 점수도 작업 생성을 막지 않음
    interview/grill.rs    # ★ 발산 인터뷰 "깊게 파기" (Tauri 비의존): 라운드당 주관식 질문 1개 + 모델의 추천 답(객관식 보기 없음) + 미해결 논점. 종료는 백엔드 강제(transcript ≥ 11) — 모델 자율 판정 불신. 산출물은 계약이 아니라 생각 정리 노트 + 개선된 지시문(docs/explorations/, 명시적 저장). slug는 모델 출력이므로 경로 구분자 제거 + canonicalize로 레포 밖 탈출 이중 차단
    mcp_registry/mod.rs   # MCP 서버 레지스트리(mcp_servers) + 비파괴적 write_mcp_config(기존 .mcp.json 보존, 새 파일만 task 소유)
    skills/mod.rs         # ★ 벤더 중립 슬래시 스킬 (Tauri 비의존): .praxis/skills/*.md(프로젝트+~/.praxis 글로벌, 프로젝트 우선) 스캔 + frontmatter description + $ARGUMENTS 확장 — 컴포저 /name을 resolve_message가 확장해 CLI로(user 이벤트는 원문, 스캔 목록 매칭이라 경로 트래버설 원천 차단) + 파일 CRUD add/delete/read(skills_add/delete/read 커맨드, 검증된 이름으로만 경로 조립·덮어쓰기 금지·64KB 가드) → SkillsView(사이드바 "스킬")에서 목록/추가/삭제/본문 열람
    schedule/{mod,runner}.rs # 크론 스케줄러(schedules): mod=is_due(순수 cron due판정) + migrate, runner=60초 틱루프(mark-before-run 멱등, task=create_task_internal). SchedulesView(사이드바 "스케줄")
    runner/{auth,http,mobile_http,queue,push,…}.rs # 모바일 PWA 서피스: pairing HTTP/WS, durable queue/replay, 웹 푸시. 독립 Runner 바이너리는 1.0에서 제거
  src/fsapi/mod.rs (#[cfg(test)])  # 보안 단위 테스트(traversal·dangling symlink·심볼릭 조상 거부)
  tests/{pty,worktree,db,memory,transcript}_test.rs  # 코어 통합 테스트 (cargo test 총 39개)
src/                      # React 프론트엔드 — Claude-desktop식 셸
  App.tsx                 # 셸 오케스트레이터: view(home/workspace/mcp/settings 등) + task·파일·테마 상태
  components/
    knowledge-vault/      # 설정 › 메모리 탭의 VaultMemoryPanel만 남음(2026-09-28 Wiki 채널 제거). IPC: lib/knowledge-vault-ipc.ts·memory-file-ipc.ts
    TerminalView.tsx      # 라이브 PTY 페인 (dark prop, unlisten 가드, rAF fit, RO 디바운스)
    DiffViewer.tsx        # 파일 단위 diff (unified/split)
    McpServersView.tsx    # 빠른링크 섹션(풀영역) 패널
    ide/
      Sidebar.tsx         # 단일 사이드바(새 작업 + 빠른링크 + 프로젝트별 작업 그룹 + 푸터)
      HomeView.tsx        # 홈 대시보드(인사말 + 통계 + 최근 작업)
      HomeProjectEditor.tsx / ProjectEditorWindow.tsx / ProjectTerminal.tsx # 홈 최근 로컬 폴더 → 작업 없는 원본 편집기·하단 셸. useWorkspaceFiles source로 파일 상태 공유, task null로 작업 기능 비활성
      PixelOffice.tsx     # Task[]를 역할·상태별 픽셀 에이전트로 투영
      AquariumWindow.tsx  # 보조 창 전용 셸 + ready/snapshot/event controller
      aquarium/           # Canvas 2D 장면·sprite atlas·motion engine·DOM 접근성 overlay
      Composer.tsx        # 하단 컴포저(레포/브랜치/worktree 칩 + 지시문 → 작업 생성)
      FileTree.tsx        # worktree 파일 트리(재귀, 키보드 접근)
      EditorPane.tsx      # Monaco 멀티파일 탭 에디터(⌘S 저장, 모델 dispose, 라이브 내용 저장) + 현재 파일·선택 줄·화면 영역 캡처 타깃 등록
      EditorSplitView.tsx # 창별 사용처·그래프 도구 탭과 위치 탐색 소유. useEditorNavigation의 epoch/navId·reveal acknowledgement로 pane·커서·스크롤 복귀, sourceVersion으로 낡은 결과 차단
      PreviewTab.tsx      # Design Mode 프리뷰 + 프리뷰 요소/중앙 에디터 캡처 진입점
      PreviewWorkbenchStrip.tsx / PreviewToolbarWindow.tsx # 인라인·별도 창 공통 질문/대기/제어 UI + toolbar 전용 진입점
      AgentComposer.tsx   # 작업별 캡처 칩을 프롬프트에 합치고 로컬 대화 IPC에 이미지 경로 전달
      SettingsPanel.tsx   # 테마/캡처 opt-in/기본 셸/음성
      VoiceHUD.tsx        # 음성 상태 오버레이(듣는 중·전사 중·결과/에러 플래시)
      VoiceSettingsCard.tsx # STT 주소·모델·키·핫키 + 연결 테스트(전사 경로를 실제로 왕복)
      icons.tsx           # 의존성 없는 인라인 SVG 아이콘
  lib/
    ipc.ts                # 호스트 레지스트리(기본은 local Tauri) transport facade + 나머지 Tauri invoke
    transport/{tauri,runner}.ts # local IPC와 `/v1` JSON/WebSocket adapter
    preview-workbench/    # 요청 맥락·task store·toolbar 상태 전달. 메인 usePreviewWorkbenchHost가 단일 소유자
    monaco.ts             # Monaco 로컬 번들 구성(워커 + praxis-dark/light 테마 + langFromPath)
    aquarium-{creatures,celebrations,projection}.ts # Task 상태→생물/완료 수명/검증된 payload 투영
    voice-router.ts       # 전사 텍스트 → VoiceAction(화면·새 작업·전송·비우기). 접두 + 꼬리말 허용 목록 매칭이라 "리뷰 코멘트를 정리해 줘"는 커맨드로 새지 않는다. 미매칭은 null = 무실행
    diff.ts / bytes.ts    # 순수 로직(테스트 대상)
  main.tsx                # window query 판별 → 메인 App 또는 AquariumWindow + 저장 테마 적용
DESIGN.md / design-tokens.json / tailwind.config.js  # 디자인 토큰 정본/미러
```

### 셸 레이아웃 (2026-06-30 — Claude-desktop식)

```
홈(작업 미선택):                        작업 열림(workspace):
┌사이드바───┬ 메인 ──────────────┐     ┌사이드바───┬파일트리┬에디터/Diff┬컨텍스트┐
│+ 새 작업  │  ✦ 인사말          │     │+ 새 작업  │ src/   │[코드|Diff]│상태    │
│메모리     │  [통계 대시보드]    │     │메모리     │ a.rs   │ Monaco    │Approve │
│자기개선   │  총작업/진행/성공률 │     │자기개선   │ ...    │ ─리사이즈─│Discard │
│MCP 서버   │  [최근 작업]        │     │MCP 서버   │        │ 터미널도크│DiffStat│
│─project── │                    │     │─project── │        │  (PTY)    │        │
│ test +    │  ┌하단 컴포저─────┐ │     │ ●작업1    │        │           │        │
│ ●작업1    │  │칩+지시문 입력→ │ │     │           │        │           │        │
│Praxis·설정│  └───────────────┘ │     │Praxis·설정│        │           │        │
└───────────┴────────────────────┘     └───────────┴────────┴───────────┴────────┘
```
사이드바는 진행 중 작업을 **레포(프로젝트)별 그룹**으로 표시(레포별 "+"로 신규). 홈 통계는 taskList/memoryList/mcpList로 프론트 계산. 파일 ops는 DB `worktree_path`로 스코프(세션 활성 무관), 종료 상태 작업 거부. 에이전트(PTY claude)와 사용자가 같은 worktree 동시 편집 → 저장 시 **내용 기반** 외부변경 가드(mtime 1초 해상도 회피).

### Agent Studio 어항 창

```text
AquariumWindow
  → tasks/interaction 리스너 둘 다 등록
  → aquarium_ready(caller=어항 검증)
  → aquarium.rs → main에 aquarium://ready

main App(Task[] 정본)
  → ready 수신 후 aquarium://tasks snapshot 발행
  → 이후 Task[] 변경 때 snapshot 갱신
  → AquariumWindow → 상태별 생물 roster
  → Canvas 2D 장면 + 같은 좌표의 DOM 접근성 overlay

AquariumWindow
  → aquarium_interactive / aquarium_hide / aquarium_open_task
  → aquarium.rs(네이티브 hit-test·focus·window 수명주기)
  → aquarium://interaction 또는 aquarium://open-task
  → main App의 기존 openTask
```

어항은 `tauri.conf.json`의 정적 `aquarium` WebviewWindow다. 기본 상태는
click-through이며 `CmdOrCtrl+Shift+A` 전역 단축키 또는 홈 `어항 조작` 버튼으로
상호작용 모드를 켠다. `Created|Queued|Starting`은 해마, `Running|Finalizing`은
테트라, `AwaitingReview|PendingApproval`은 복어, 관측된 `Done`은 8초 금붕어,
열기 전 `Failed`는 베타로 투영한다. 최대 8마리를 상태 우선순위로 보여 주며
상태별 zone과 300ms 전환 버블을 사용한다. Canvas는 DPR을 2로 제한하고
`document.hidden` 또는 reduced-motion에서는 RAF를 중지한다.

프런트 이벤트 이름과 창 진입 판별은 `src/lib/aquarium-events.ts`, 메인 창의 Task
snapshot 소유는 `src/components/use-aquarium-host.ts`, roster 정본은
`src/lib/aquarium-creatures.ts`, Canvas 세션은
`src/components/ide/aquarium/aquarium-animation-session.ts`, 네이티브 상태 정본은
`src-tauri/src/aquarium.rs`에 있다.

### Design Mode 에디터 캡처 → 로컬 대화 첨부

```
EditorPane(DOM bounds + 현재 파일/선택 줄)
  → PreviewTab의 에디터 캡처
  → designmode_capture_editor
  → <worktree>/.praxis/captures/<task_id>/{id}.png + {id}.json
  → taskId별 capture store → AgentComposer 칩/프롬프트
  → convo_send(image_paths)
      Codex: `--image <canonical capture path>`
      Claude/Agy: 프롬프트의 캡처 경로·선택 코드 문맥
```

대화 IPC는 첨부 경로를 현재 task의 캡처 디렉터리 안에서 canonicalize해 검증한다.

**레이어 경계**: 코어 모듈(`pty`/`worktree`/`db`, 순수 Rust·Tauri 비의존) ← `commands.rs`(Tauri 바인딩·통합) ← `lib.rs`(앱 조립). 프론트는 `ipc.ts`를 통해서만 백엔드 호출. 코어가 Tauri를 모르므로 `cargo test`로 독립 검증 가능.

### Task 라이프사이클 (Phase 2, 다중 동시)

```
대시보드: New Task(레포+지시문) → task_create (create_lock 직렬화, cap=8 체크)
  → worktree::create(<repo>/.praxis/worktrees/<slug-nanos>, branch=praxis/<slug>-<nanos>)
  → db::insert_task(Created) → PTY 스폰(cwd=worktree) → Running → tasks.insert(id, ActiveTask)
  → 포워더 스레드: PtyEvent → 16ms 창으로 합쳐 emit pty://output/{id} | pty://exit/{id}
  → 에이전트 종료 → mark_awaiting_review(가드) + task://state{id,AwaitingReview} + 네이티브 알림
  → 카드 클릭 → 해당 id 터미널(이벤트 id 필터) · task_diff_stat(id)
  → task_approve(id): 원자적 remove → 세션 종료 → worktree.approve(커밋+머지, 충돌 시 --abort+복원) → Done
  또는 task_discard(id): 원자적 remove → worktree.discard → Discarded
재시작: lib.rs setup → db.init_pool(WAL) → mark_stale_running_failed(Running→Failed)
```

여러 Task가 각자 worktree + PTY + 포워더 스레드를 갖고 병렬 실행되며, 이벤트는 id로 라우팅된다.

### 메모리 루프 (파일형, 설계 2026-09-13)

메모리 정본은 창고(`<memory_root>/<repo-key>/`) 안 `MEMORY.md`/`USER.md` 파일이다. DB 티어·FTS5·
승인 원장·증거 재검증(P2)은 1.0에서 전부 제거했다 — 지금 남은 것은 파일을 읽어 worktree 컨텍스트
파일에 마커블록으로 옮겨 쓰는 투영뿐이다.

```
task_create: memory::file::project(창고 MEMORY.md/USER.md 읽기 → worktree 컨텍스트 파일 마커블록)  [spawn 전]
창고 파일 자체의 조회·편집은 설정 › 메모리 탭이 맡는다(VaultMemoryPanel)
```

자동 추출·수동 회고 파이프라인(`capture::generate_convo_reflection`·`selfimprove::insert_proposal`)은
1.0에서 자기개선 기능 자체와 함께 제거됐다 — `capture::invoke`에 남은 실행 프로파일·봉투 파싱은
Wiki 번역 등 다른 호출자가 쓴다.

## 데이터 플로우 (PTY 라이프사이클)

```
프론트 ipc.ptySpawn(cmd,args,cwd,cols,rows)
  → commands.pty_spawn: 기존 세션 take().terminate() → PtySession::spawn()
      → openpty + spawn_command → reader 스레드(master→채널 Output), waiter 스레드(wait→채널 Exit)
  → forwarder 스레드: 채널 → coalesce_output(16ms·64KiB) → app.emit("pty://output/{id}"{base64}) / ("pty://exit/{id}"{code})
프론트 listen("pty://output/{id}") → atob → Uint8Array → term.write   (R3/R4) — id별 이벤트라 다른 세션 출력은 JS에 오지 않는다
프론트 term.onData → ipc.ptyWrite → commands.pty_write → session.write(stdin)  (R5)
ResizeObserver → ipc.ptyResize → session.resize                              
창 닫기 → on_window_event(CloseRequested) → session.take().terminate()        (R7)
  terminate(): killpg(pgid, SIGTERM) → (분리 스레드 800ms) → SIGKILL  [비블로킹]
```

## 핵심 불변식 / 규칙

- **단일 세션**: `AppState { session: Mutex<Option<PtySession>> }`. Phase 1에서 `HashMap<SessionId, _>`로 확장 예정.
- **표시 ↔ 데이터 경로 분리**: 라이브 표시는 PTY 바이트, 학습 데이터는 (후속) 트랜스크립트 JSONL. 혼용 금지.
- **종료 정리 비블로킹**: `terminate()`는 시그널 시퀀스를 분리 스레드에 위임하고 즉시 반환(이벤트 루프/IPC 비블로킹). `pid==0` 가드로 자기 그룹 종료 방지.
- **best-effort 부가기능**(메모리/자기개선, 후속)은 오케스트레이션 코어를 막지 않는다.

## 보안

- **CSP 적용**: `tauri.conf.json`에 prod `csp`(`script-src 'self'`) + dev `devCsp`(Vite HMR 허용) 분리. webview에 주입된 원격/인라인 스크립트가 `invoke`를 호출하는 XSS 경로를 차단.
- **참고**: Tauri 2 앱 커맨드(`generate_handler`)는 capability 게이트 대상이 아님 → 통제는 CSP. 터미널 앱 특성상 `pty_spawn` 인자 allowlist는 미적용(셸 임의 입력이 정상 동작).
- **어항 경계**: `src-tauri/capabilities/aquarium.json`은 event listen/unlisten, 자신의 window set-size, window drag만 허용한다. snapshot ready·상태 조회·숨김·작업 열기 command는 `WebviewWindow` caller label을 검사하고 작업 ID는 양수만 허용한다. Tauri event listen 권한은 이벤트 이름 단위로 좁혀지지 않으므로 어항 Webview의 번들 무결성은 CSP와 lockfile에 의존한다.
- **잔여 위험**: AppManifest 기반 custom-command ACL은 아직 없어 등록된 다른 앱 command는 local Webview 전체에 노출된다. 이번 어항 command는 자체 caller 검사로 보호하지만, 전체 command 권한 분리는 별도 아키텍처 과제다.
- **공급망(후속)**: in-bundle 악성 의존성은 CSP로 못 막음 → 락파일 커밋 + audit 점검.

## 플랫폼 지원

- **종료**: unix=`killpg` SIGTERM→SIGKILL(`cfg(unix)`), windows=`taskkill /T /F`(`cfg(windows)`).
- **셸**: `default_shell()` 커맨드 — unix=`$SHELL`(폴백 `/bin/zsh`)+`-l`, windows=`powershell.exe`. 프론트가 작업 생성 시 사용.
- **경로**: 트랜스크립트 HOME→USERPROFILE 폴백.
- **어항 창**: Tauri 공통 window API로 항상 위·모든 workspace·click-through를 제어하고, Windows에서는 taskbar에서 제외한다. macOS 투명 창은 `app.macOSPrivateApi=true`라 Mac App Store가 아닌 직접 배포를 전제로 한다.
- **검증 상태**: unix(macOS)에서 기존 GUI와 빌드·테스트를 검증했다. 새 어항은 Chrome 시각·성능 QA, 네이티브 debug 빌드와 자동 테스트까지 통과했으며 운영 루프 부작용을 피하려고 실제 앱 프로세스 실행은 생략했다. **Windows는 실제 Windows 머신에서 빌드·실행 검증이 필요하다.** 트랜스크립트 경로 인코딩(`\` 처리)은 Windows 실검증 시 점검 필요.
