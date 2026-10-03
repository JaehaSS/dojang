//! Tauri command 레이어 (Phase 2) — 다중 활성 Task 오케스트레이션.
//!
//! AppState가 `HashMap<id, ActiveTask>`로 N개 동시 작업을 보유(cap=max_concurrent).
//! 이벤트는 task id로 라우팅: `pty://output/{id}`{id,data}, `pty://exit/{id}`{id,code}, `task://state`{id,state}.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::annotations;
use crate::capsule;
use crate::db::{self, state as tstate, Task};
use crate::decision;
use crate::diffmodel;
use crate::editorwindow;
use crate::followup_observation::{self, ConversationInputOrigin};
use crate::fonts;
use crate::fsapi;
use crate::github;
use crate::insights;
use crate::mcp_registry;
use crate::memory;
pub(crate) use crate::orchestrator::{CreateTaskParams, TaskOrigin};
use crate::orchestrator::{TaskDraft, TaskService};
use crate::pty::{OutputCoalescer, PtyEvent, PtySession};
use crate::review_ops::{self, ReviewClaims};
use crate::rewind;
use crate::sessionhome;
use crate::shellreap::ShellSlot;
use crate::theme_store;
use crate::verify;
use crate::worktree::{self, Worktree};

fn wiki_sync_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// 현재 실행 중인 작업 (worktree + PTY 세션).
pub struct ActiveTask {
    pub worktree: Worktree,
    /// PTY 세션 — 터미널 모드만 보유. 대화 모드(convo)는 None(PTY 없이 stream-json).
    pub session: Option<PtySession>,
    /// 터미널 모드 spawn이 쥔 프리뷰 MCP 토큰·설정. 항목이 맵에서 빠지면 함께 폐기된다.
    pub preview_mcp: Option<crate::preview_bridge::mcp::PreviewMcpLease>,
}

/// 실행 중인 대화 turn의 관측 메타데이터. 키 존재 자체가 in-flight 계약이며,
/// 세부 필드는 상태 카드·인터럽트·재시작 조정에서 공용으로 사용한다.
#[derive(Clone)]
pub struct ActiveConvo {
    pub pgid: Option<u32>,
    pub vendor_bin: String,
    pub started_at: i64,
    pub last_event_at: i64,
    pub last_operation: Option<String>,
    /// 사용자가 이 턴에 인터럽트를 요청했다 — result 미수신 종료 시 사인 구분용.
    pub interrupted: bool,
}

type ActiveConvos = Arc<Mutex<HashMap<i64, ActiveConvo>>>;

#[derive(Clone)]
pub(crate) struct PreviewReceiptAcceptance {
    pub(crate) workbench: crate::preview_workbench::PreviewWorkbench,
    pub(crate) request_id: String,
    pub(crate) context: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConvoAdmissionAction {
    PreserveTakeover,
    ReleaseManualTakeover,
}

/// 에이전트 전환 동안 턴 시작·절단·되감기를 막는 짧은 점유다. mutex는 등록/해제만 지키며
/// DB·git I/O를 기다리는 동안 잡지 않는다.
struct ConvoReservation {
    active: ActiveConvos,
    id: i64,
    remove_on_drop: bool,
}

impl Drop for ConvoReservation {
    fn drop(&mut self) {
        if !self.remove_on_drop {
            return;
        }
        release_convo_if_finalized(&self.active,self.id);
    }
}

impl ConvoReservation {
    fn handoff_to_turn(&mut self) {
        self.remove_on_drop = false;
    }
}

fn reserve_convo_switch(active: ActiveConvos, id: i64) -> Result<ConvoReservation, String> {
    use std::collections::hash_map::Entry;

    let mut turns = active.lock().unwrap_or_else(|error| error.into_inner());
    if matches!(turns.entry(id), Entry::Occupied(_)) {
        return Err("대화 턴이 아직 종료 처리 중입니다 — 잠시 후 다시 시도하세요".into());
    }
    let ts = now();
    turns.insert(
        id,
        ActiveConvo {
            pgid: None,
            vendor_bin: String::new(),
            started_at: ts,
            last_event_at: ts,
            last_operation: None,
            interrupted: false,
        },
    );
    drop(turns);
    Ok(ConvoReservation {
        active,
        id,
        remove_on_drop: true,
    })
}

/// The structured controller is removed only after cleanup and the task-state commit.
fn release_convo_if_finalized(active: &ActiveConvos, id: i64) -> bool {
    if crate::convo::app_server::execution(id).is_some() {
        crate::convo::app_server::mark_cleanup_failed(id);
        return false;
    }
    active.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
    true
}

/// 프리뷰 웹뷰 핸들과 그것이 사는 곳.
///
/// 창 모드에서도 `Webview` 핸들을 담는다 — `WebviewWindow`는 `AsRef<Webview<R>>`이므로
/// (`Deref`가 아니다. 필드는 `pub(crate)`라 직접 접근할 수 없다) `as_ref().clone()`으로
/// 뽑아 담으면 `eval`·`navigate`·캡처 인터셉트 경로를 모드와 무관하게 재사용할 수 있다.
/// 창 핸들이 필요하면 `webview.window()`로 되찾는다.
#[derive(Clone)]
pub struct PreviewHandle {
    pub webview: tauri::Webview,
    pub toolbar: Option<tauri::Webview>,
    pub toolbar_height: Option<Arc<Mutex<f64>>>,
    pub window: Option<tauri::Window>,
    pub generation: u64,
    pub mode: crate::designmode::PreviewMode,
}

/// 동시 실행 상한 기본값 — 설정(`max_concurrent` 키)이 없을 때 쓰인다.
pub const DEFAULT_MAX_CONCURRENT: usize = 8;

/// 설정으로 지정할 수 있는 동시 실행 상한의 범위. 작업 하나가 CLI 프로세스 + PTY +
/// worktree를 각각 차지하므로 무제한을 열지 않는다 — 실질 한계는 머신 자원(CPU·RAM·API
/// rate limit)이고, 상한은 오타로 세 자리를 넣었을 때의 사고를 막는 방어선이다.
pub const MAX_CONCURRENT_MIN: usize = 1;
pub const MAX_CONCURRENT_MAX: usize = 64;

/// 앱 전역 상태 — 다중 활성 작업 + DB 풀 + 동시 실행 상한.
pub struct AppState {
    pub pool: Mutex<Option<SqlitePool>>,
    pub tasks: Mutex<HashMap<i64, ActiveTask>>,
    /// 동시 실행 상한 — 설정 패널에서 런타임에 바뀐다(기동 시 DB에서 복원).
    /// 낮추더라도 이미 실행 중인 작업을 죽이지 않는다. 초과분은 자연 종료로 흡수되고
    /// 그동안 새 생성만 막힌다.
    pub max_concurrent: AtomicUsize,
    /// 생성 진행 중(아직 `tasks`에 삽입 전)인 슬롯 수 — cap 체크는 `tasks.len() + reserved`로
    /// 계산해 TOCTOU를 막는다(무거운 생성 파이프라인을 락 밖에서 병렬 수행하기 위함, `reserve_slot` 참고).
    pub reserved: Mutex<usize>,
    /// 직접 실행의 브랜치 전환과 Task 행 생성을 canonical repo 단위로 직렬화한다.
    /// Unix에서는 Git common dir 파일 락으로 다른 Praxis 프로세스와도 조율한다.
    pub direct_repo_locks: worktree::DirectCheckoutLocks,
    /// Verify/Challenge와 Approve/Discard/Delete의 task별 수명주기 점유.
    pub review_claims: ReviewClaims,
    /// 진행 중인 대화 턴 — 키 존재=in-flight(중복 전송 방지·busy 복원), 값은 관측 메타데이터.
    /// (기존 inflight HashSet + pids HashMap 두 락을 하나로 — "진행 중" 사실의 단일 소스.)
    pub convo_active: ActiveConvos,
    /// Isolated questions own their cancellation and lifetime independently of main turns.
    pub side_question_active: Arc<Mutex<HashSet<i64>>>,
    /// 채널 아웃바운드(텔레그램 등) 전송용 공용 HTTP 클라이언트 — 재사용(커넥션 풀).
    pub http: reqwest::Client,
    /// allowlist 미등록 chat에서 온 메시지(온보딩) — 최신 `MAX_SEEN_CHATS`개, in-memory only.
    /// 작업별 워크스페이스 셸(도구 패널 터미널) — 작업 PTY(`ActiveTask.session`)와 별개로,
    /// 대화 모드 작업의 워크트리에서 사용자가 직접 명령을 실행하는 인터랙티브 셸.
    /// 슬롯이 attach 수와 유휴 관측을 함께 들고 있다 — 유휴 회수의 재료다(ADR 0163 결정 4).
    pub shells: Mutex<HashMap<i64, ShellSlot>>,
    /// 유휴로 회수된 작업 id — 그 자리에 새 셸을 열 때 `shell_replay`가 한 번 안내하고 뺀다.
    /// 빈 화면만 주면 사용자는 히스토리를 잃은 것인지 새 셸인지 구분할 수 없다.
    pub reaped_shells: Mutex<HashSet<i64>>,
    /// 작업별 Python REPL(IPython) PTY — `shells`와 같은 키(task id)지만 별도 맵이다.
    /// 유휴 회수 대상이 아니다: REPL의 가치는 살아 있는 인터프리터 상태(변수·import)라서
    /// "안 보면 죽인다"를 적용하면 그 상태가 날아간다. 종료는 `repl_close`나 앱 종료뿐.
    pub repls: Mutex<HashMap<i64, ReplSlot>>,
    /// 인증·업데이트·자유 셸 액션의 PTY — 키는 "<kind>:<vendor>". 작업에 묶이지 않으므로
    /// task id로 키잉된 `shells`와 섞지 않는다(이벤트 채널도 `action://`로 분리).
    pub action_shells: Mutex<HashMap<String, PtySession>>,
    /// 시작 시 자동 업데이트가 도는 동안 참. 그동안 작업 생성을 막는다 — 업데이트는
    /// 바이너리를 갈아치우므로 도중에 시작된 작업은 발밑이 바뀐다.
    /// 내리는 일은 `autoupdate::UpdateGuard`가 Drop 에서 책임진다.
    pub updating: Arc<AtomicBool>,
    /// 마지막 자동 업데이트 결과. 이벤트만으로는 부족하다 — 시작 직후라 설정 패널이 아직
    /// 리스너를 붙이기 전에 끝난다. 나중에 열어도 읽을 수 있어야 조용한 실패가 없다.
    pub last_autoupdate: Arc<Mutex<crate::agenthealth::autoupdate::AutoUpdateReport>>,
    /// Design Mode 프리뷰 탭의 자식 웹뷰 핸들(task id 키) — local 전용, runner 미노출(D-1).
    pub designmode_webviews: Mutex<HashMap<i64, PreviewHandle>>,
    pub preview_openings: crate::preview_control::openings::PreviewOpenings,
    /// Design Mode remote result의 task/session/generation 단일 소스.
    pub preview_bridge: crate::preview_bridge::PreviewBridge,
    /// Preview Workbench 제출 receipt의 앱 실행 중 단일 소스.
    pub preview_workbench: crate::preview_workbench::PreviewWorkbench,
    /// 에디터 "정의로 이동"용 언어 서버 풀 (작업 × 언어). 로컬 전용 — 서버가 워크트리
    /// 파일시스템을 직접 읽으므로 원격 워크트리에는 붙일 수 없다.
    pub lsp: Arc<crate::lspclient::LspPool>,
    /// 에이전트가 프리뷰를 몰 때 쓰는 Bearer 토큰 맵 — task 종결 시 폐기된다.
    pub control_tokens: crate::preview_bridge::mcp::ControlTokens,
    /// 앱 인스턴스마다 새로 뽑는 MCP 경로 조각(`/mcp/<instance>`). 난수를 못 얻으면 None —
    /// 빈 문자열로 열어두면 경로를 아는 것이 아무 방어도 되지 못한다.
    pub mcp_instance: Option<String>,
    /// 인앱 MCP 서버가 실제로 잡은 포트. 기동 실패·프로브 모드에서는 None.
    pub mcp_port: Mutex<Option<u16>>,
    /// task별 마지막 제어 명령 시각 — 유휴 3초 뒤 제어 표시를 내리는 판정에 쓴다.
    pub control_last: Mutex<HashMap<i64, std::time::Instant>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            pool: Mutex::new(None),
            tasks: Mutex::new(HashMap::new()),
            max_concurrent: AtomicUsize::new(DEFAULT_MAX_CONCURRENT),
            reserved: Mutex::new(0),
            direct_repo_locks: worktree::DirectCheckoutLocks::default(),
            review_claims: ReviewClaims::default(),
            convo_active: Arc::new(Mutex::new(HashMap::new())),
            side_question_active: Arc::new(Mutex::new(HashSet::new())),
            http: reqwest::Client::new(),
            shells: Mutex::new(HashMap::new()),
            reaped_shells: Mutex::new(HashSet::new()),
            repls: Mutex::new(HashMap::new()),
            action_shells: Mutex::new(HashMap::new()),
            updating: Arc::new(AtomicBool::new(false)),
            last_autoupdate: Arc::new(Mutex::new(Default::default())),
            designmode_webviews: Mutex::new(HashMap::new()),
            preview_openings: Default::default(),
            preview_bridge: crate::preview_bridge::PreviewBridge::new(),
            preview_workbench: crate::preview_workbench::PreviewWorkbench::new(),
            lsp: Arc::new(crate::lspclient::LspPool::default()),
            control_tokens: crate::preview_bridge::mcp::ControlTokens::default(),
            mcp_instance: crate::preview_bridge::random_hex_id().ok(),
            mcp_port: Mutex::new(None),
            control_last: Mutex::new(HashMap::new()),
        }
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub mod service_tier;
mod side_question_slots;
mod task_results;
mod project_plan;
pub use project_plan::*;
pub(crate) mod pipeline_driver;
mod pipeline;
pub use pipeline::*;
pub use task_results::*;

mod quiz;
pub use quiz::*;
mod ensemble;
pub use ensemble::*;
mod voice;
pub use voice::*;
mod translate;
pub use translate::*;
mod vocab;
pub use vocab::*;
mod shell;
pub use shell::*;
mod repl;
pub use repl::*;
mod wiki_workspace;
pub use wiki_workspace::*;
mod knowledge_vault;
pub use knowledge_vault::*;
mod knowledge_vault_session;
pub use knowledge_vault_session::*;
mod memory_file;
pub use memory_file::*;
mod knowledge_vault_settings;
pub use knowledge_vault_settings::*;
mod knowledge_vault_usage;
pub use knowledge_vault_usage::*;
mod knowledge_vault_recovery;
pub use knowledge_vault_recovery::*;
mod local_file;
pub use local_file::*;
mod mcp;
pub use mcp::*;
mod schedule;
pub use schedule::*;
mod designmode;
pub use designmode::*;
mod preview_workbench;
pub use preview_workbench::*;
mod preview_window;
pub use preview_window::*;
mod today;
pub use today::*;
mod gmail;
pub use gmail::*;
mod mobile;
pub use mobile::*;

pub(crate) fn pool_of(state: &AppState) -> Result<SqlitePool, String> {
    // poison 복구: 다른 스레드 패닉으로 잠금이 오염돼도 전체 커맨드가 패닉하지 않게.
    state
        .pool
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or_else(|| "DB가 초기화되지 않았습니다".to_string())
}

/// 에이전트(벤더)별 설정된 기본 모델 조회 (`model:<agent>`). 미설정/조회 실패는 None.
async fn agent_model_of(pool: &SqlitePool, agent: &str) -> Option<String> {
    db::get_setting(pool, &format!("model:{}", agent.trim()))
        .await
        .ok()
        .flatten()
        .filter(|v| !v.trim().is_empty())
}

/// 작업 실행 모델 해석 — 세션 오버라이드(`tasks.model`) 우선, 없으면 벤더 기본(`model:<agent>`).
/// 매 호출 시 DB를 새로 읽어 재시작/이어하기에도 최신 값을 반영한다.
async fn model_for_task(pool: &SqlitePool, id: i64, agent: &str) -> Option<String> {
    if let Ok(Some(t)) = db::get_task(pool, id).await {
        if let Some(m) = t.model.filter(|v| !v.trim().is_empty()) {
            return Some(m);
        }
    }
    agent_model_of(pool, agent).await
}

#[derive(Clone, Serialize)]
struct OutputPayload {
    id: i64,
    data: String,
}

#[derive(Clone, Serialize)]
struct ExitPayload {
    id: i64,
    code: i32,
}

#[derive(Clone, Serialize)]
struct StatePayload {
    id: i64,
    state: String,
    /// 검토 대기 성격(`db::awaiting_kind::*`). 그 외 상태에서는 항상 None —
    /// 프런트가 이 값으로 "질문 대기"와 "결과 검토 대기"를 갈라 표시한다.
    awaiting_kind: Option<String>,
}

/// 검토 대기의 성격을 모바일 원장 이벤트 종류로 옮긴다.
///
/// 질문 대기와 결과 검토 대기는 폰에서 할 일이 다르다 — 하나로 뭉개면 푸시 문구도 뭉개진다.
fn awaiting_ledger_kind(awaiting_kind: Option<&str>) -> &'static str {
    if awaiting_kind == Some(db::awaiting_kind::QUESTION) {
        db::runner_event_kind::AWAITING_ANSWER
    } else {
        db::runner_event_kind::AWAITING_REVIEW
    }
}

/// 검토 대기 전이를 화면(`task://state`)과 **모바일 원장**(`runner_events`)에 함께 알린다.
///
/// 설계 2026-09-13 D3 — 폰의 라이브 뷰(`/v1/events/live`)와 Web Push는 `runner_events` 하나만
/// 본다. 러너는 큐 worker가 그 행을 쓰지만 데스크톱에는 큐가 없다. 그래서 전이 지점마다 같은
/// 이름으로 직접 써 넣지 않으면, 화면은 갱신되는데 폰은 영영 깨어나지 않는다.
///
/// 원장 쓰기는 best-effort로 떼어 둔다 — 이 관심사가 전이 자체를 막을 이유가 없다.
fn emit_awaiting_review(app: &AppHandle, pool: &SqlitePool, id: i64, awaiting_kind: Option<&str>) {
    let _ = app.emit(
        "task://state",
        StatePayload {
            id,
            state: tstate::AWAITING_REVIEW.to_string(),
            awaiting_kind: awaiting_kind.map(str::to_string),
        },
    );
    let kind = awaiting_ledger_kind(awaiting_kind);
    let pool = pool.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = db::append_runner_event(&pool, id, now(), kind, None).await {
            eprintln!("모바일 원장 기록 실패(무시): {error}");
        }
    });
}

/// 인앱 MCP 서버가 떠 있으면 이 spawn 몫의 토큰·설정을 발급한다. 실패하면 주입 없이 진행한다 —
/// 프리뷰 제어만 빠질 뿐, 턴 자체는 예전처럼 돌아야 한다(설계 0058 D-12).
fn issue_preview_mcp(
    app: &AppHandle,
    state: &AppState,
    id: i64,
    vendor: crate::convo::Vendor,
    questions: bool,
) -> Option<crate::preview_bridge::mcp::PreviewMcpLease> {
    use crate::preview_bridge::mcp::inject;

    let port = (*state
        .mcp_port
        .lock()
        .unwrap_or_else(|error| error.into_inner()))?;
    let instance = state.mcp_instance.as_deref()?;
    let data_dir = app.path().app_data_dir().ok()?;
    let endpoint = inject::endpoint_url(port, instance);
    let dir = inject::config_dir(&data_dir, instance);
    match inject::PreviewMcpLease::issue_for(
        &state.control_tokens,
        id,
        vendor,
        &endpoint,
        &dir,
        questions,
    ) {
        Ok(lease) => Some(lease),
        Err(error) => {
            eprintln!("프리뷰 MCP 주입 생략(토큰·설정 발급 실패): {error}");
            None
        }
    }
}

/// worktree cwd에서 에이전트를 PTY로 실행. 출력/종료를 task id와 함께 이벤트로 포워딩하고,
/// 종료 시 DB를 AwaitingReview로 갱신하고 durable 결과 원천에 기록한다.
#[allow(clippy::too_many_arguments)]
fn spawn_agent(
    app: AppHandle,
    pool: SqlitePool,
    id: i64,
    repo: String,
    cwd: String,
    cmd: &str,
    args: &[String],
    cols: u16,
    rows: u16,
    env: &[(String, String)],
) -> Result<PtySession, String> {
    let argv: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (session, rx) = PtySession::spawn_with_env(cmd, &argv, Some(cwd.as_str()), cols, rows, env)
        .map_err(|e| format!("PTY 생성 실패: {e}"))?;
    std::thread::spawn(move || {
        let output_event = format!("pty://output/{id}");
        let exit_event = format!("pty://exit/{id}");
        let mut coalescer = OutputCoalescer::new();
        while let Ok(ev) = rx.recv() {
            // 청크당 emit 대신 창당 emit — 합치는 도중 종료를 만나면 그대로 Exit 분기로 넘긴다.
            let code = match ev {
                PtyEvent::Output(b) => {
                    let (bytes, pending_exit) = coalescer.gather(&rx, b);
                    let _ = app.emit(
                        &output_event,
                        OutputPayload {
                            id,
                            data: STANDARD.encode(&bytes),
                        },
                    );
                    match pending_exit {
                        Some(code) => code,
                        None => continue,
                    }
                }
                PtyEvent::Exit(code) => code,
            };
            let _ = app.emit(&exit_event, ExitPayload { id, code });
            // 에이전트 종료 → AwaitingReview + 메모리 파일 상태 갱신(best-effort)
            tauri::async_runtime::block_on(async {
                // 가드된 전이: 이미 승인/폐기된 작업을 AwaitingReview로 되돌리지 않음.
                // 터미널 모드는 구조화 이벤트가 없어 질문 판별 근거가 없다 → 통상 검토.
                let notice_kind = if code == 0 { "result" } else { "failure" };
                let _ = db::mark_awaiting_review_with_notification(
                    &pool, id, now(), None, notice_kind,
                ).await;
                // 파일 정본의 크기·수정 시각만 기록한다(설계 2026-09-13 R8). 추출·회고·인용
                // 관측은 없다 — 메모리는 세션 안에서 에이전트가 직접 파일을 고쳐 남긴다.
                let data_dir = app
                    .path()
                    .app_data_dir()
                    .unwrap_or_else(|_| crate::memory::file::data_dir(&pool));
                let _ = crate::memory::file::record_exit(&pool, &data_dir, &repo, now()).await;
            });
            emit_awaiting_review(&app, &pool, id, None);
            return;
        }
        let transitioned = tauri::async_runtime::block_on(async {
            db::mark_awaiting_review_with_notification(&pool, id, now(), None, "failure")
                .await
                .unwrap_or(false)
        });
        if transitioned {
            emit_awaiting_review(&app, &pool, id, None);
        }
    });
    Ok(session)
}

/// 외부기원(봇/크론) repo 화이트리스트 검증 — 이미 알려진(과거 사용된) repo 경로이거나 그 하위일 때만
/// 통과. canonicalize 실패(경로 미존재)도 거부. Ui 경로는 호출하지 않는다(사용자가 직접 고른 경로).
async fn check_repo_allowed(pool: &SqlitePool, repo: &str) -> Result<(), String> {
    let target =
        std::fs::canonicalize(repo).map_err(|e| format!("repo 경로를 확인할 수 없습니다: {e}"))?;
    let known = db::known_repos(pool).await.map_err(|e| e.to_string())?;
    let allowed = known.iter().any(|k| {
        std::fs::canonicalize(k)
            .map(|kc| target == kc || target.starts_with(&kc))
            .unwrap_or(false)
    });
    if !allowed {
        return Err(
            "등록되지 않은 repo 경로: 봇/크론은 이미 사용한 프로젝트에서만 실행 가능".to_string(),
        );
    }
    Ok(())
}

/// 게이트웨이 수동 등록 서버에 워크트리 언어 기반 LSP 자동주입 스펙을 병합한다.
/// `lsp_autoinject` 설정이 꺼져 있으면(기본 켜짐) 원본을 그대로 반환.
/// 이름 충돌 시 게이트웨이 수동 등록분을 우선(자동 스펙 제외). best-effort — 감지 실패가
/// 작업 생성을 막지 않는다.
async fn merge_lsp_autoinject(
    pool: &SqlitePool,
    servers: &[mcp_registry::McpServer],
    worktree_path: &Path,
) -> Vec<mcp_registry::McpServer> {
    let autoinject_on = db::get_setting(pool, "lsp_autoinject")
        .await
        .ok()
        .flatten()
        .as_deref()
        != Some("false");
    if !autoinject_on {
        return servers.to_vec();
    }
    let rust_analyzer_available = crate::reviewer::which("rust-analyzer").is_some();
    let detected = crate::lspdetect::detect_lsp_servers(worktree_path, rust_analyzer_available);
    let mut merged = servers.to_vec();
    for spec in detected {
        if servers.iter().any(|s| s.name == spec.name) {
            continue;
        }
        merged.push(mcp_registry::McpServer {
            id: 0,
            name: spec.name,
            command: spec.command,
            args: serde_json::to_string(&spec.args).unwrap_or_else(|_| "[]".to_string()),
            enabled: 1,
            created_at: now(),
        });
    }
    merged
}

/// 헤드리스 터미널 fallback(cmd/args/cols/rows) — 에이전트 해석 실패 시에만 쓰이는 bare 셸 스펙.
/// External 작업(승인 대기 후 spawn)은 항상 `headless_terminal`로 생성되어 빈 cmd/기본 크기다.
struct SpawnFallback {
    cmd: String,
    args: Vec<String>,
    cols: u16,
    rows: u16,
}

/// cap 도달 시 반환하는 공용 에러 메시지 — 생성 경로 전체에서 문구 동일 유지.
fn cap_reached_error(max_concurrent: usize) -> String {
    format!("동시 실행 한도({max_concurrent})에 도달 — 진행 중 작업을 승인하거나 버린 후 다시 시도하세요")
}

/// `cap_reached_error`가 만든 오류인가. 파이프라인 드라이버가 "자리가 날 때까지 대기"와
/// 진짜 실패를 가르는 데 쓴다.
pub(crate) fn is_cap_reached_error(error: &str) -> bool {
    error.starts_with("동시 실행 한도(")
}

/// 예약 카운터 핵심 로직(순수 함수, 유닛테스트 대상) — "활성 + 예약중" 합이 상한 미만이면
/// `reserved`를 1 증가시키고 Ok, 아니면 건드리지 않고 Err. 호출자가 `active_count`를 실제
/// 상한 체크와 같은 임계구역에서 넘겨야 TOCTOU가 없다(`reserve_slot` 참고).
fn try_reserve_count(
    active_count: usize,
    reserved: &mut usize,
    max_concurrent: usize,
) -> Result<(), String> {
    if active_count + *reserved >= max_concurrent {
        return Err(cap_reached_error(max_concurrent));
    }
    *reserved += 1;
    Ok(())
}

/// 예약 성공 시 발급되는 RAII 가드 — Drop에서 `reserved`를 1 감소시켜 예약을 반납한다.
/// 에러 조기 반환(`?`) 등 모든 실패 경로에서 스코프를 벗어나며 자동 반납되어 누수를 막는다.
/// `state.tasks`에 실제 삽입한 직후 명시적으로 `drop(guard)`하면 예약→실제 카운트 전환이 끝난다
/// (삽입 전에 drop하면 그 순간 cap을 과소평가해 초과 허용될 수 있으므로 반드시 삽입 "후"에 drop).
struct SlotReservation<'a> {
    reserved: &'a Mutex<usize>,
}

impl Drop for SlotReservation<'_> {
    fn drop(&mut self) {
        let mut r = self.reserved.lock().unwrap_or_else(|e| e.into_inner());
        *r = r.saturating_sub(1);
    }
}

/// cap 체크 + 슬롯 예약을 원자적으로 수행한다. 호출자는 `state.tasks` 락을 쥔 상태에서
/// `tasks.len()`을 `active_count`로 넘겨야 한다 — 그래야 이 함수가 잡는 `reserved` 락과 합쳐
/// "tasks 락 → reserved 락" 순서로 중첩되어, 동시 호출(`insert`가 tasks 락을 필요로 함)과
/// 완전히 직렬화된다(락 순서 고정으로 데드락 없음). 반환된 가드를 쥔 동안은 worktree 생성·임베딩
/// 등 무거운 작업을 **락 없이** 수행할 수 있다 — 이게 이 함수를 두는 이유(생성 파이프라인 병렬화).
fn reserve_slot(
    active_count: usize,
    reserved: &Mutex<usize>,
    max_concurrent: usize,
) -> Result<SlotReservation<'_>, String> {
    let mut r = reserved.lock().unwrap_or_else(|e| e.into_inner());
    try_reserve_count(active_count, &mut r, max_concurrent)?;
    Ok(SlotReservation { reserved })
}

/// 앱 재시작 뒤 in-memory 핸들이 없어도 같은 checkout을 공유하는 비종료 direct 작업을 찾는다.
async fn ensure_no_open_direct_task(pool: &SqlitePool, repo: &Path) -> Result<(), String> {
    let tasks = db::list_open_direct_tasks(pool)
        .await
        .map_err(|error| error.to_string())?;
    let repo = repo.to_path_buf();
    let conflict = tauri::async_runtime::spawn_blocking(move || {
        let canonical_repo = std::fs::canonicalize(repo)
            .map_err(|error| format!("직접 실행 레포를 확인할 수 없습니다: {error}"))?;
        Ok::<_, String>(tasks.into_iter().find(|task| {
            std::fs::canonicalize(&task.repo).is_ok_and(|path| path == canonical_repo)
        }))
    })
    .await
    .map_err(|error| error.to_string())??;
    match conflict {
        Some(task) => Err(format!(
            "같은 레포의 직접 실행 작업 #{}이 아직 끝나지 않아 브랜치를 전환할 수 없습니다",
            task.id
        )),
        None => Ok(()),
    }
}

/// 직접 모드는 선택한 기존 브랜치로 메인 체크아웃을 옮긴 뒤, diff 기준점을 불변 SHA로 고정한다.
fn direct_worktree(repo: &Path, start_branch: Option<&str>) -> Result<Worktree, String> {
    if let Some(branch) = start_branch {
        worktree::checkout_local_branch(repo, branch).map_err(|error| error.to_string())?;
    }
    let branch = worktree::current_branch_or_direct(repo);
    let is_git = worktree::is_git_repository(repo);
    let base = if is_git {
        worktree::current_revision(repo).map_err(|error| error.to_string())?
    } else {
        branch.clone()
    };
    Ok(Worktree {
        repo: repo.to_path_buf(),
        path: repo.to_path_buf(),
        branch,
        // 직접 모드의 base는 이미 시작 순간의 SHA다 — 같은 값이 기준점 역할도 한다.
        // git이 아니면 base가 브랜치 대체값(DIRECT_BRANCH)이라 기준점이 될 수 없다.
        base_revision: is_git.then(|| base.clone()),
        base,
    })
}

/// direct checkout을 직렬화하고, 실제 브랜치 전환은 DB의 기존 direct 작업과 충돌하지 않을 때만 한다.
async fn prepare_direct_worktree(
    pool: &SqlitePool,
    locks: &worktree::DirectCheckoutLocks,
    repo: &Path,
    start_branch: Option<&str>,
) -> Result<(Worktree, worktree::DirectCheckoutGuard), String> {
    let claim = locks.acquire(repo).await.map_err(|error| error.to_string())?;
    let selected = start_branch.map(str::trim).filter(|branch| !branch.is_empty());
    if let Some(branch) = selected {
        let current = worktree::current_branch(repo).map_err(|error| error.to_string())?;
        if current != branch {
            if !worktree::local_branch_exists(repo, branch) {
                return Err(format!("'{branch}' 로컬 브랜치를 찾을 수 없습니다"));
            }
            ensure_no_open_direct_task(pool, repo).await?;
        }
    }
    let repo = repo.to_path_buf();
    let selected = selected.map(str::to_string);
    let worktree =
        tauri::async_runtime::spawn_blocking(move || direct_worktree(&repo, selected.as_deref()))
            .await
            .map_err(|error| error.to_string())??;
    Ok((worktree, claim))
}

/// 작업 생성과 별개로 메인 체크아웃만 전환한다. 직접 실행 생성·승인 머지와 같은 레포 락을
/// 쥐고, 끝나지 않은 직접 실행 작업이 그 체크아웃을 쓰고 있으면 거절한다.
async fn checkout_main_branch(
    pool: &SqlitePool,
    locks: &worktree::DirectCheckoutLocks,
    repo: &Path,
    branch: &str,
) -> Result<BranchList, String> {
    let branch = branch.trim();
    if branch.is_empty() {
        return Err("전환할 브랜치를 고르세요".to_string());
    }
    let _claim = locks.acquire(repo).await.map_err(|error| error.to_string())?;
    let current = worktree::current_branch(repo).map_err(|error| error.to_string())?;
    if current != branch {
        ensure_no_open_direct_task(pool, repo).await?;
    }
    let repo = repo.to_path_buf();
    let branch = branch.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        worktree::checkout_local_branch(&repo, &branch).map_err(|error| error.to_string())?;
        branch_list(&repo)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// 파일형 메모리를 worktree의 컨텍스트 파일에 투영한다(설계 2026-09-13 R1–R3).
/// 실패하면 작업을 시작하지 않는다 — 블록이 없는 채로 도는 세션은 규칙도 못 받는다.
async fn project_memory_or_fail(
    pool: &SqlitePool,
    data_dir: &Path,
    repo: &str,
    task_id: i64,
    worktree: &Worktree,
    targets: &[&str],
    direct_mode: bool,
) -> Result<(), String> {
    let result = memory::file::project(
        pool,
        data_dir,
        repo,
        &worktree.path,
        targets,
        task_id,
        now(),
    )
    .await;
    let had_content = match result {
        Ok(had_content) => had_content,
        Err(error) => {
            let reason = error.to_string();
            let _ = db::update_state(pool, task_id, tstate::FAILED, now()).await;
            let _ = db::append_event(
                pool,
                task_id,
                "memory_projection_failed",
                Some(&reason),
                now(),
            )
            .await;
            if !direct_mode {
                let _ = worktree.discard();
            }
            return Err(format!(
                "메모리 파일 투영에 실패해 작업을 시작하지 않았습니다: {reason}"
            ));
        }
    };
    let _ = db::append_event(pool, task_id, "memory_projection_applied", None, now()).await;
    // 실제 내용이 실렸을 때만 "메모리 안내 후 재설명 없음" 인사이트의 마커를 남긴다 —
    // 규칙만 실린 블록은 안내한 것이 없으니 재설명 여부를 셀 자격이 없다.
    if had_content {
        let _ = followup_observation::insert_observation_start(pool, task_id, now()).await;
    }
    Ok(())
}

#[cfg(test)]
mod task_base_tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static DB_COUNTER: AtomicU32 = AtomicU32::new(0);

    async fn task_base_pool() -> SqlitePool {
        let n = DB_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = crate::testtmp::dir().join(format!(
            "praxis-direct-branch-{}-{n}.db",
            std::process::id()
        ));
        db::init_pool(path.to_string_lossy().as_ref()).await.unwrap()
    }

    fn direct_git_repo(label: &str) -> PathBuf {
        let dir = crate::testtmp::dir().join(format!(
            "praxis-direct-{label}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("note.txt"), "before\n").unwrap();
        worktree::init_repository(&dir).unwrap();
        dir
    }

    fn create_local_branch(repo: &Path, branch: &str) {
        let status = std::process::Command::new("git")
            .current_dir(repo)
            .args(["branch", branch])
            .status()
            .unwrap();
        assert!(status.success());
    }

    async fn insert_direct_task(pool: &SqlitePool, repo: &Path, branch: &str, base: &str) -> i64 {
        let repo = repo.to_string_lossy().into_owned();
        db::insert_task(
            pool,
            &repo,
            branch,
            base,
            &repo,
            "direct task",
            Some("codex"),
            None,
            "conversation",
            1,
        )
        .await
        .unwrap()
    }

    #[test]
    fn non_git_directory_is_allowed_only_for_direct_execution() {
        let dir = crate::testtmp::dir().join(format!("praxis-direct-no-git-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let worktree = direct_worktree(&dir, None).unwrap();
        assert_eq!(worktree.branch, worktree::DIRECT_BRANCH);
        assert_eq!(worktree.base, worktree::DIRECT_BRANCH);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn git_direct_execution_keeps_branch_and_immutable_diff_baseline() {
        let dir = crate::testtmp::dir().join(format!("praxis-direct-git-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("note.txt"), "before\n").unwrap();
        worktree::init_repository(&dir).unwrap();

        let direct = direct_worktree(&dir, None).unwrap();

        assert_eq!(direct.branch, worktree::current_branch(&dir).unwrap());
        assert_eq!(direct.base, worktree::current_revision(&dir).unwrap());
        assert_ne!(direct.base, direct.branch);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn git_direct_execution_uses_the_requested_start_branch() {
        let dir = crate::testtmp::dir().join(format!(
            "praxis-direct-selected-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("note.txt"), "before\n").unwrap();
        worktree::init_repository(&dir).unwrap();
        let status = std::process::Command::new("git")
            .current_dir(&dir)
            .args(["branch", "dev"])
            .status()
            .unwrap();
        assert!(status.success());

        let direct = direct_worktree(&dir, Some("dev")).unwrap();

        assert_eq!(worktree::current_branch(&dir).unwrap(), "dev");
        assert_eq!(direct.branch, "dev");
        assert_eq!(direct.base, worktree::current_revision(&dir).unwrap());
        assert_eq!(direct.base_revision.as_deref(), Some(direct.base.as_str()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn nonterminal_direct_task_blocks_another_branch_switch_after_restart() {
        let pool = task_base_pool().await;
        let dir = crate::testtmp::dir().join(format!(
            "praxis-direct-active-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let alias = dir.join(".").to_string_lossy().into_owned();
        let id = db::insert_task(
            &pool,
            &alias,
            "main",
            "sha",
            &alias,
            "active direct task",
            Some("codex"),
            None,
            "conversation",
            1,
        )
        .await
        .unwrap();

        let error = ensure_no_open_direct_task(&pool, &dir).await.unwrap_err();
        assert!(error.contains(&format!("#{id}")));

        db::update_state(&pool, id, tstate::DONE, 2).await.unwrap();
        ensure_no_open_direct_task(&pool, &dir).await.unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn open_direct_task_allows_an_unchanged_checkout() {
        let pool = task_base_pool().await;
        let dir = direct_git_repo("unchanged-active");
        let current = worktree::current_branch(&dir).unwrap();
        insert_direct_task(&pool, &dir, &current, "sha").await;
        let locks = worktree::DirectCheckoutLocks::default();

        let selected = prepare_direct_worktree(&pool, &locks, &dir, Some(&current))
            .await
            .unwrap();
        assert_eq!(selected.0.branch, current);
        drop(selected);
        let implicit = prepare_direct_worktree(&pool, &locks, &dir, None)
            .await
            .unwrap();
        assert_eq!(implicit.0.branch, current);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn manual_checkout_switches_and_returns_the_new_current_branch() {
        let pool = task_base_pool().await;
        let dir = direct_git_repo("manual-checkout");
        create_local_branch(&dir, "dev");
        let locks = worktree::DirectCheckoutLocks::default();

        let list = checkout_main_branch(&pool, &locks, &dir, "dev").await.unwrap();
        assert_eq!(list.current, "dev");
        assert_eq!(worktree::current_branch(&dir).unwrap(), "dev");
        assert!(list.branches.contains(&"dev".to_string()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn manual_checkout_refuses_while_a_direct_task_uses_the_checkout() {
        let pool = task_base_pool().await;
        let dir = direct_git_repo("manual-checkout-busy");
        let current = worktree::current_branch(&dir).unwrap();
        create_local_branch(&dir, "dev");
        let id = insert_direct_task(&pool, &dir, &current, "sha").await;
        let locks = worktree::DirectCheckoutLocks::default();

        let error = checkout_main_branch(&pool, &locks, &dir, "dev").await.unwrap_err();
        assert!(error.contains(&format!("#{id}")));
        assert_eq!(worktree::current_branch(&dir).unwrap(), current);
        // 이미 체크아웃된 브랜치를 고르면 전환이 없으므로 막지 않는다.
        let list = checkout_main_branch(&pool, &locks, &dir, &current).await.unwrap();
        assert_eq!(list.current, current);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn concurrent_direct_branch_switch_waits_for_the_db_claim() {
        let pool = task_base_pool().await;
        let dir = direct_git_repo("concurrent");
        let initial = worktree::current_branch(&dir).unwrap();
        create_local_branch(&dir, "dev");
        let locks = worktree::DirectCheckoutLocks::default();

        let (first, claim) = prepare_direct_worktree(&pool, &locks, &dir, Some("dev"))
            .await
            .unwrap();
        assert_eq!(first.branch, "dev");

        let second_pool = pool.clone();
        let second_locks = locks.clone();
        let second_dir = dir.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let mut second = tokio::spawn(async move {
            let _ = started_tx.send(());
            prepare_direct_worktree(&second_pool, &second_locks, &second_dir, Some(&initial))
                .await
                .map(|_| ())
        });
        tokio::time::timeout(std::time::Duration::from_secs(1), started_rx)
            .await
            .expect("두 번째 요청이 시작되지 않았다")
            .unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut second)
                .await
                .is_err(),
            "같은 레포 checkout이 직렬화되지 않았다"
        );

        let id = insert_direct_task(&pool, &dir, "dev", &first.base).await;
        drop(claim);

        let error = second.await.unwrap().unwrap_err();
        assert!(error.contains(&format!("#{id}")));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn separate_direct_lock_instances_serialize_the_same_repository() {
        let dir = direct_git_repo("cross-process-lock");
        let first = worktree::DirectCheckoutLocks::default();
        let second = worktree::DirectCheckoutLocks::default();
        let claim = first.acquire(&dir).await.unwrap();

        let second_dir = dir.clone();
        let mut waiter = tokio::spawn(async move { second.acquire(&second_dir).await });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut waiter)
                .await
                .is_err(),
            "별도 락 인스턴스가 같은 레포를 동시에 점유했다"
        );
        drop(claim);
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("락 반납 후 대기 요청이 깨어나지 않았다")
            .unwrap()
            .unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn stale_created_direct_task_is_failed_on_restart() {
        let pool = task_base_pool().await;
        let dir = direct_git_repo("stale-created");
        let id = insert_direct_task(&pool, &dir, "main", "sha").await;

        let locks = worktree::DirectCheckoutLocks::default();
        assert_eq!(
            fail_stale_created_direct_tasks(&pool, &locks, 2)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db::get_task(&pool, id).await.unwrap().unwrap().state,
            tstate::FAILED
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn live_created_direct_task_is_not_failed_by_another_startup() {
        let pool = task_base_pool().await;
        let dir = direct_git_repo("live-created");
        let id = insert_direct_task(&pool, &dir, "main", "sha").await;
        let creation_locks = worktree::DirectCheckoutLocks::default();
        let claim = creation_locks.acquire(&dir).await.unwrap();

        let recovery_pool = pool.clone();
        let recovery_locks = worktree::DirectCheckoutLocks::default();
        let mut recovery = tokio::spawn(async move {
            fail_stale_created_direct_tasks(&recovery_pool, &recovery_locks, 2).await
        });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut recovery)
                .await
                .is_err(),
            "live 생성 파이프라인이 가진 repo 락을 재시작 복구가 기다리지 않았다"
        );
        db::update_state(&pool, id, tstate::RUNNING, 2).await.unwrap();
        drop(claim);

        assert_eq!(recovery.await.unwrap().unwrap(), 0);
        assert_eq!(
            db::get_task(&pool, id).await.unwrap().unwrap().state,
            tstate::RUNNING
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// 새 Task 생성 (cap 초과 시 거부). worktree + 브랜치 → DB 삽입까지 공통.
/// origin==Ui는 즉시 에이전트 spawn(`spawn_task_agent`, 회귀 0). origin==External(봇/크론)은
/// repo 화이트리스트 검증 후 PENDING_APPROVAL로 멈추고 spawn은 사용자 승인(`task_run_approve`) 시 수행.
/// Tauri 비의존 — IPC(`task_create`) 외에 크론/텔레그램 봇도 재사용(Phase 2·3).
/// `worktree://base-refresh` 페이로드 — 어느 작업의 최신화인지 붙여 보낸다.
#[derive(Clone, Serialize)]
struct BaseRefreshPayload {
    id: i64,
    outcome: worktree::refresh::RefreshOutcome,
}

/// 생성 진행 알림 — 어느 요청의 것인지 프런트가 판별할 수 있게 client_ref를 함께 싣는다.
#[derive(Clone, Serialize)]
struct CreatingPayload {
    client_ref: String,
    stage: String,
}

/// `client_ref`가 있는 요청에만 진행을 알린다. 없는 경로(크론·봇)는 보는 사람이 없다.
fn emit_creating(app: &AppHandle, client_ref: Option<&str>, stage: &str) {
    let Some(client_ref) = client_ref else {
        return;
    };
    let _ = app.emit(
        "task://creating",
        CreatingPayload {
            client_ref: client_ref.to_string(),
            stage: stage.to_string(),
        },
    );
}

/// 단계별 소요 시간. 콜백이 `Fn`이라 `Arc<Mutex<_>>`로 공유하며 갱신한다.
#[derive(Default)]
struct StageClock {
    current: Option<(worktree::CreateStage, Instant)>,
    refresh_ms: u64,
    worktree_ms: u64,
    bootstrap_ms: u64,
}

impl StageClock {
    /// 새 단계 진입 — 직전 단계를 닫는다.
    fn enter(&mut self, stage: worktree::CreateStage) {
        self.close();
        self.current = Some((stage, Instant::now()));
    }

    /// 마지막 단계를 닫는다. 단계 밖에서 불려도 무해하다.
    fn close(&mut self) {
        let Some((stage, started)) = self.current.take() else {
            return;
        };
        let ms = started.elapsed().as_millis() as u64;
        match stage {
            worktree::CreateStage::Refresh => self.refresh_ms += ms,
            worktree::CreateStage::Worktree => self.worktree_ms += ms,
            worktree::CreateStage::Bootstrap => self.bootstrap_ms += ms,
        }
    }
}

/// 실패 사유를 표본에 실을 때의 길이 상한. git stderr는 길이 제한이 없어 그대로 두면
/// 한 행이 표본 전체를 덮는다. 경계는 문자 단위로 자른다 — 바이트로 자르면 한글이 깨진다.
const REFRESH_ERROR_MAX: usize = 200;

fn truncated_reason(reason: &str) -> String {
    let trimmed = reason.trim();
    if trimmed.chars().count() <= REFRESH_ERROR_MAX {
        return trimmed.to_string();
    }
    trimmed.chars().take(REFRESH_ERROR_MAX).collect::<String>() + "…"
}

/// `metric.prepared`의 detail JSON. 직접 모드는 최신화·부트스트랩 **단계 자체가 없어서**
/// 필드를 뺀다 — 0으로 채우면 표본에서 "빨랐던 격리 생성"으로 읽힌다(설계 0059 §5.2).
/// 같은 이유로 `check_version_ms`·`refresh_outcome`도 그 단계를 실제로 거친 생성에만 붙인다.
fn prepared_metric_detail(
    origin: TaskOrigin,
    direct: bool,
    refresh_enabled: bool,
    clock: &StageClock,
    prep_ms: u64,
    memory_ms: u64,
    embed_ready: bool,
    check_version_ms: Option<u64>,
    refresh: Option<&worktree::refresh::RefreshOutcome>,
) -> String {
    let mut detail = serde_json::json!({
        "origin": origin.label(),
        "direct": direct,
        "refresh_enabled": refresh_enabled,
        "worktree_ms": clock.worktree_ms,
        "prep_ms": prep_ms,
        "memory_ms": memory_ms,
        "embed_ready": embed_ready,
    });
    if !direct {
        detail["refresh_ms"] = clock.refresh_ms.into();
        detail["bootstrap_ms"] = clock.bootstrap_ms.into();
        // `refresh_ms`만으로는 **느린 성공과 상한에 걸린 실패를 가를 수 없다** —
        // `refresh_base`는 어떤 실패도 `Failed`로 접어 Err를 올리지 않으므로(refresh.rs),
        // 30초 상한에 걸린 fetch가 정상 fetch와 똑같은 모양으로 남는다. 종류를 함께 적어야
        // "네트워크 때문에 창이 늦게 열렸다"를 표본 한 행으로 판정할 수 있다.
        if let Some(outcome) = refresh {
            detail["refresh_outcome"] = outcome.label().into();
            if let worktree::refresh::RefreshOutcome::Failed { reason } = outcome {
                detail["refresh_error"] = truncated_reason(reason).into();
            }
        }
    }
    // 질문 세션(로컬 Codex)만 거치는 게이트다. 다른 생성에는 필드 자체를 두지 않는다 —
    // 0으로 채우면 표본에서 "버전 확인이 빨랐던 생성"으로 읽힌다.
    if let Some(ms) = check_version_ms {
        detail["check_version_ms"] = ms.into();
    }
    detail.to_string()
}

/// 실패한 생성의 계측 행(`metric.create_failed`). `metric.prepared`와 **kind를 가른다** —
/// 설계 0059 §6의 판정 SQL은 성공한 생성만 세도록 짜여 있어서, 같은 kind로 섞으면 꼬리
/// 판정이 실패 건으로 오염된다.
///
/// 그래도 소요 시간은 남겨야 한다. 30초를 기다린 끝에 실패한 생성과 곧바로 실패한 생성은
/// 사용자가 겪는 일이 전혀 다른데, 지금은 둘 다 사유 한 줄만 남기고 시간은 어디에도 없다.
///
/// `failed_at`보다 뒤에 오는 단계의 필드는 0이다 — 그 단계를 거치지 않았기 때문이고,
/// 어느 단계에서 멈췄는지는 `failed_at`이 말해 준다.
fn create_failure_detail(prepared: &str, failed_at: &str) -> String {
    let mut detail: serde_json::Value =
        serde_json::from_str(prepared).unwrap_or_else(|_| serde_json::json!({}));
    detail["failed_at"] = failed_at.into();
    detail.to_string()
}

#[cfg(test)]
mod prepared_metric_tests {
    use super::*;

    fn fields(detail: &str) -> serde_json::Map<String, serde_json::Value> {
        serde_json::from_str(detail).expect("detail은 JSON 객체")
    }

    /// 필드 수가 곧 표본의 모양이다. 직접 모드에 0ms 최신화가 섞이면 Phase 2 판정에서
    /// "격리 생성이 빨랐다"로 읽힌다.
    #[test]
    fn direct_mode_omits_the_stages_it_never_runs() {
        let clock = StageClock::default();

        let isolated = fields(&prepared_metric_detail(
            TaskOrigin::Ui,
            false,
            true,
            &clock,
            2810,
            640,
            true,
            None,
            None,
        ));
        assert_eq!(isolated.len(), 9, "격리+최신화는 9개 필드");
        assert_eq!(isolated["origin"], "ui");
        assert_eq!(isolated["prep_ms"], 2810);
        assert_eq!(isolated["memory_ms"], 640);

        let direct = fields(&prepared_metric_detail(
            TaskOrigin::External,
            true,
            false,
            &clock,
            12,
            3,
            false,
            None,
            None,
        ));
        assert_eq!(direct.len(), 7, "직접 모드는 7개 필드");
        assert!(!direct.contains_key("refresh_ms"));
        assert!(!direct.contains_key("bootstrap_ms"));
        assert_eq!(direct["origin"], "external", "TaskOrigin은 여기에만 남는다");

        let pipeline = fields(&prepared_metric_detail(
            TaskOrigin::Pipeline,
            false,
            false,
            &clock,
            5,
            1,
            false,
            None,
            None,
        ));
        assert_eq!(pipeline["origin"], "pipeline");
    }

    /// Pipeline은 UI가 아니므로 최신화·직접 모드·질문 응답에서 External과 같이 빠지지만,
    /// 승인 대기에는 들어가지 않는다(분기가 External 한정이다).
    #[test]
    fn pipeline_origin_is_not_ui_and_not_external() {
        assert_ne!(TaskOrigin::Pipeline, TaskOrigin::Ui);
        assert_ne!(TaskOrigin::Pipeline, TaskOrigin::External);
        assert_eq!(TaskOrigin::Pipeline.label(), "pipeline");
    }

    /// 상한에 걸린 fetch와 느린 성공은 `refresh_ms`가 같은 모양이라 가려지지 않는다.
    /// 종류와 사유가 함께 남아야 표본 한 행으로 판정된다.
    #[test]
    fn a_timed_out_refresh_is_told_apart_from_a_slow_one() {
        let mut clock = StageClock::default();
        clock.refresh_ms = 30_012;

        let failed = fields(&prepared_metric_detail(
            TaskOrigin::Ui,
            false,
            true,
            &clock,
            30_450,
            9,
            false,
            None,
            Some(&worktree::refresh::RefreshOutcome::Failed {
                reason: "git [\"fetch\"]가 30초 안에 끝나지 않았습니다".into(),
            }),
        ));
        assert_eq!(failed["refresh_ms"], 30_012);
        assert_eq!(failed["refresh_outcome"], "failed");
        assert!(
            failed["refresh_error"]
                .as_str()
                .expect("사유는 문자열")
                .contains("30초"),
            "타임아웃 사유가 표본에 남지 않았다"
        );

        let slow = fields(&prepared_metric_detail(
            TaskOrigin::Ui,
            false,
            true,
            &clock,
            30_450,
            9,
            false,
            None,
            Some(&worktree::refresh::RefreshOutcome::FastForwarded { commits: 4 }),
        ));
        assert_eq!(slow["refresh_outcome"], "fast_forwarded");
        assert!(
            !slow.contains_key("refresh_error"),
            "성공한 최신화에 사유가 붙었다"
        );
    }

    /// 최신화 단계를 거치지 않은 생성에 종류를 적으면 "최신화가 건너뛰어졌다"가 아니라
    /// "최신화가 있었다"로 읽힌다 — 직접 모드에는 필드 자체가 없어야 한다.
    #[test]
    fn direct_mode_keeps_its_shape_even_with_an_outcome_at_hand() {
        let clock = StageClock::default();
        let direct = fields(&prepared_metric_detail(
            TaskOrigin::Ui,
            true,
            false,
            &clock,
            41,
            4,
            false,
            None,
            Some(&worktree::refresh::RefreshOutcome::Skipped),
        ));
        assert_eq!(direct.len(), 7, "직접 모드는 7개 필드 그대로");
        assert!(!direct.contains_key("refresh_outcome"));
    }

    /// 버전 게이트는 `prep_ms` 시계 밖에서 돈다. 거친 생성에만 붙고, 거치지 않은 생성에는
    /// 0이 아니라 필드가 없어야 한다.
    #[test]
    fn the_version_gate_is_recorded_only_where_it_runs() {
        let clock = StageClock::default();

        let gated = fields(&prepared_metric_detail(
            TaskOrigin::Ui,
            true,
            false,
            &clock,
            41,
            4,
            false,
            Some(2_140),
            None,
        ));
        assert_eq!(gated["check_version_ms"], 2_140);
        assert_eq!(gated.len(), 8, "직접 모드 7개 + 버전 게이트 1개");

        let ungated = fields(&prepared_metric_detail(
            TaskOrigin::Ui,
            true,
            false,
            &clock,
            41,
            4,
            false,
            None,
            None,
        ));
        assert!(
            !ungated.contains_key("check_version_ms"),
            "게이트를 안 거친 생성에 0이 들어갔다"
        );
    }

    /// 실패 행은 성공 표본과 같은 필드를 그대로 갖되, 멈춘 단계가 더해져야 한다.
    /// kind를 가르는 것으로 판정 SQL을 지키므로 detail 모양은 일부러 같게 둔다.
    #[test]
    fn a_failed_creation_keeps_the_timing_and_names_the_stage() {
        let mut clock = StageClock::default();
        clock.refresh_ms = 30_004;

        let prepared = prepared_metric_detail(
            TaskOrigin::Ui,
            false,
            true,
            &clock,
            30_120,
            0,
            false,
            None,
            Some(&worktree::refresh::RefreshOutcome::Failed {
                reason: "timeout".into(),
            }),
        );
        let failed = fields(&create_failure_detail(&prepared, "memory"));

        assert_eq!(failed["failed_at"], "memory");
        assert_eq!(failed["prep_ms"], 30_120, "기다린 시간이 실패 행에도 남는다");
        assert_eq!(failed["refresh_outcome"], "failed");
        assert_eq!(
            failed.len(),
            fields(&prepared).len() + 1,
            "실패 행은 단계 하나만 더 갖는다"
        );
    }

    /// git stderr에는 길이 제한이 없다. 자르되 한글 경계에서 깨지면 안 된다.
    #[test]
    fn a_long_reason_is_cut_on_a_character_boundary() {
        let long = "가".repeat(REFRESH_ERROR_MAX + 50);
        let cut = truncated_reason(&long);
        assert_eq!(cut.chars().count(), REFRESH_ERROR_MAX + 1, "말줄임표 한 자 포함");
        assert!(cut.ends_with('…'));

        let short = truncated_reason("  네트워크에 연결할 수 없습니다  ");
        assert_eq!(short, "네트워크에 연결할 수 없습니다", "짧은 사유는 그대로 남는다");
    }
}

/// `resume_from`·`resume_session`은 배타다 — 작업 id와 세션 id가 함께 오면 승계 분기가
/// 갈리므로(설계 2026-09-17 결정 2) 어느 쪽을 따를지 정할 근거가 없다. `create_task_internal`은
/// `AppHandle`이 필요해 통합 테스트가 무겁다 — 판정만 순수 함수로 뽑아 단위 테스트한다.
fn resume_targets_exclusive(resume_from: Option<i64>, resume_session: Option<&str>) -> Result<(), String> {
    if resume_from.is_some() && resume_session.is_some() {
        return Err("이어받기 대상은 작업 또는 세션 중 하나만 지정할 수 있습니다".into());
    }
    Ok(())
}

/// `resume_external` 대화 이벤트의 JSON 모양(설계 2026-09-17 결정 6) — 세션홈 승계 직후,
/// 원장에 앞선 이벤트가 하나도 없는 상태에서 출처를 알린다. 세션 id는 앞 8자만 남긴다.
///
/// 메타는 항상 있다 — 승계 성공은 `sessionhome::describe`가 해석과 인덱싱을 **한 번에** 끝냈다는
/// 뜻이라, 둘 사이에 파일이 사라지는 창이 없다.
fn resume_external_event(session_id: &str, meta: &sessionhome::SessionMeta) -> serde_json::Value {
    let short_id: String = session_id.chars().take(8).collect();
    serde_json::json!({
        "kind": "resume_external",
        "session_id": short_id,
        "cwd": meta.cwd.clone(),
        "vendor": meta.vendor,
        "last_active": meta.last_active,
        "messages": meta.messages,
    })
}

#[cfg(test)]
mod resume_session_dispatch_tests {
    use super::*;

    #[test]
    fn resume_from_alone_is_allowed() {
        assert!(resume_targets_exclusive(Some(7), None).is_ok());
    }

    #[test]
    fn resume_session_alone_is_allowed() {
        assert!(resume_targets_exclusive(None, Some("s-1")).is_ok());
    }

    #[test]
    fn neither_is_allowed() {
        assert!(resume_targets_exclusive(None, None).is_ok());
    }

    #[test]
    fn both_together_are_rejected() {
        let error = resume_targets_exclusive(Some(7), Some("s-1")).unwrap_err();
        assert!(error.contains("하나만"), "배타 위반 메시지여야 함: {error}");
    }

    fn sample_meta(session_id: &str) -> sessionhome::SessionMeta {
        sessionhome::SessionMeta {
            session_id: session_id.to_string(),
            vendor: "claude".into(),
            recent_user_message: None,
            cwd: Some("/repo".to_string()),
            last_cwd: None,
            git_branch: None,
            title: None,
            first_message: None,
            last_active: 1_700_000_000,
            messages: 42,
            vendor_version: None,
        }
    }

    #[test]
    fn event_truncates_session_id_to_eight_chars_and_carries_meta() {
        let meta = sample_meta("0123456789abcdef");
        let event = resume_external_event("0123456789abcdef", &meta);
        assert_eq!(event["kind"], "resume_external");
        assert_eq!(event["session_id"], "01234567");
        assert_eq!(event["cwd"], "/repo");
        assert_eq!(event["last_active"], 1_700_000_000);
        assert_eq!(event["messages"], 42);
    }

    /// 벤더가 cwd를 한 번도 적지 않은 세션이면 `cwd`는 `null`로 나간다 — 프론트가 그 자리에
    /// 무엇을 그릴지는 이벤트가 아니라 렌더가 정한다.
    #[test]
    fn event_keeps_cwd_null_when_the_session_never_recorded_one() {
        let mut meta = sample_meta("0123456789abcdef");
        meta.cwd = None;
        let event = resume_external_event("0123456789abcdef", &meta);
        assert!(event["cwd"].is_null());
        assert_eq!(event["messages"], 42);
    }

    #[test]
    fn event_handles_short_session_id_without_panicking() {
        let meta = sample_meta("abc");
        let event = resume_external_event("abc", &meta);
        assert_eq!(event["session_id"], "abc");
    }
}

async fn ensure_managed_codex(app: &AppHandle, version: String) -> Result<String, String> {
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?
        .join("cli-runtimes").join("codex");
    tauri::async_runtime::spawn_blocking(move || crate::convo::cli_runtime::ensure(&root, &version))
        .await.map_err(|e| e.to_string())?
}

pub(crate) async fn create_task_internal(
    app: &AppHandle,
    state: &AppState,
    p: CreateTaskParams,
) -> Result<Task, String> {
    let CreateTaskParams {
        repo,
        instruction,
        agent,
        role,
        model,
        reasoning_effort,
        service_tier,
        headless,
        ensemble,
        mode,
        cmd,
        args,
        cols,
        rows,
        origin,
        goal_contract,
        ambiguity,
        base_branch,
        client_ref,
        resume_from,
        resume_session,
        resume_vendor,
        purpose_selection,
        approvals,
    } = p;
    // 두 이어받기 입력은 배타다 — 작업 id와 세션 id가 함께 오면 어느 쪽을 따를지 정할 근거가
    // 없다(설계 2026-09-17 결정 2). 승계 분기 자체가 갈리므로 여기서 바로 거절한다.
    resume_targets_exclusive(resume_from, resume_session.as_deref())?;
    if resume_session.is_some() {
        let vendor = resume_vendor.as_deref().unwrap_or("claude");
        if !matches!(vendor, "claude" | "codex") || agent.trim() != vendor
            || mode != "conversation" || !ensemble.trim().is_empty() {
            return Err("세션 공급자와 단일 대화 에이전트가 일치해야 합니다".into());
        }
    } else if resume_vendor.is_some() {
        return Err("세션 없이 공급자를 지정할 수 없습니다".into());
    }
    let creation_generation=crate::convo::interaction_commands::generation();
    let questions = mode == crate::convo::interaction::CREATE_MODE;
    // 버전 게이트는 `prep_ms`의 시계(`t0`)가 시작되기 **전에** 돈다. 따로 재지 않으면 이
    // 구간은 어느 계측에도 잡히지 않는데, 창이 열릴 때까지의 대기에는 그대로 포함된다.
    let mut check_version_ms: Option<u64> = None;
    // 승인 통로(`--permission-prompt-tool`)는 Claude 질문 세션에만 있다. 조용히 무시하면 승인을
    // 요청한 세션이 자동 승인으로 돈다.
    if approvals
        && (!questions
            || crate::convo::interaction::runtime_for_agent(&agent)
                != Some(crate::convo::interaction::RUNTIME_LOCAL))
    {
        return Err("실행 전 승인은 로컬 Claude 질문 세션에서 지원합니다".into());
    }
    let question_runtime = if questions {
        let runtime = crate::convo::interaction::runtime_for_agent(&agent);
        let Some(runtime) = runtime else {
            return Err("질문 응답은 새 로컬 Codex·Claude 일반 대화에서 지원합니다".into());
        };
        // 세션홈 승계는 벤더 세션을 이어받는 경로라 질문형 새 대화와 배타다.
        if origin != TaskOrigin::Ui || headless || !ensemble.trim().is_empty() || resume_from.is_some() || resume_session.is_some() {
            return Err("질문 응답은 새 로컬 Codex·Claude 일반 대화에서 지원합니다".into());
        }
        // Codex만 app-server 프로토콜에 묶여 있다. MCP 런타임은 문서화된 표면만 써서 고정하지 않는다.
        if runtime == crate::convo::interaction::RUNTIME {
            let checked = Instant::now();
            ensure_managed_codex(app, crate::convo::cli_runtime::DEFAULT_VERSION.into()).await?;
            check_version_ms = Some(checked.elapsed().as_millis() as u64);
        } else {
            crate::reviewer::which(&agent).ok_or("에이전트 실행 파일을 찾을 수 없습니다")?;
        }
        Some(runtime)
    } else {
        None
    };
    let mode = if questions { "conversation".to_string() } else { mode };
    let service_tier = crate::agent::service_tier::normalize(service_tier.as_deref())?.map(str::to_string);
    if service_tier.is_some() {
        if mode != "conversation" || headless || !ensemble.trim().is_empty() || origin != TaskOrigin::Ui {
            return Err("실행 속도는 로컬 Codex 일반 대화에서만 선택할 수 있습니다".into());
        }
        crate::agent::service_tier::validate(&agent, &model, service_tier.as_deref(), &crate::agent::service_tier::supported_models())?;
    }
    // 사용자가 Enter를 누른 뒤 실제로 기다리는 구간의 시작점.
    let t0 = Instant::now();
    crate::agent::reasoning_effort_override_for_model(
        &agent,
        Some(&model),
        Some(&reasoning_effort),
    )?;
    let role = crate::agent::normalize_role_or_default(&role)?.to_string();
    let pool = pool_of(state)?;
    if let Some(source) = resume_from {
        if crate::convo::interaction::is_bound(&pool, source).await? {
            return Err("질문 세션은 원래 작업에서 계속하세요. 다른 작업으로 이어받기는 아직 지원하지 않습니다".into());
        }
    }

    // cap 체크 + 슬롯 예약(짧은 임계구역, tasks 락 → reserved 락 순서로 원자적) — 이 아래
    // worktree 생성·임베딩·메모리 주입 등 무거운 작업은 락 없이 수행되어 동시 생성이 직렬화되지 않는다.
    // 가드는 실제 `state.tasks` 삽입 직후 drop되어 예약을 반납한다(그 전 조기 반환 시 자동 반납).
    let reservation = {
        let tasks = state.tasks.lock().unwrap();
        let main = state.convo_active.lock().unwrap_or_else(|e| e.into_inner());
        let questions = state.side_question_active.lock().unwrap_or_else(|e| e.into_inner());
        reserve_slot(
            side_question_slots::occupied_slots(&tasks, &main, &questions),
            &state.reserved,
            state.max_concurrent.load(Ordering::Relaxed),
        )?
    };
    // Pipeline은 사용자가 계획 승인 단계에서 이미 허락한 repo라 화이트리스트를 다시 묻지 않는다.
    if origin == TaskOrigin::External {
        check_repo_allowed(&pool, &repo).await?;
    }
    if let Some(contract) = &goal_contract {
        contract.validate()?;
    }
    // GoalContract::validate와 동일하게 공용 생성 경로에서 검증 — IPC 핸들러에만 두면
    // 비UI 기원이 ambiguity를 얻게 될 때 검증이 누락된다.
    if let Some(score) = &ambiguity {
        score.validate()?;
    }

    // 직접 모드(워크트리 미격리): 설정이 꺼져 있고 UI 기원이며 앙상블이 아닐 때만 허용.
    // 설정은 이 repo의 유효값 — 프로젝트 오버라이드가 있으면 그것이, 없으면 전역 기본이 적용된다.
    // 앙상블(후보 여러 개가 동시에 같은 repo를 건드림)과 External(봇/크론, 무인 승인)은
    // 안전상 항상 격리를 강제 — 이 설정으로도 우회할 수 없다.
    let direct_mode = origin == TaskOrigin::Ui
        && ensemble.trim().is_empty()
        && !use_worktree_on(&pool, &repo).await;
    let repo_path = Path::new(&repo);
    // 직접 실행에는 fetch 최신화가 없다. 명시한 로컬 브랜치 전환만 허용한다.
    let mut refresh_outcome: Option<worktree::refresh::RefreshOutcome> = None;
    let mut direct_claim = None;
    // 단계 콜백은 `Fn`이라 시각을 여기(스폰 블로킹 밖)에서 쥔다.
    let clock = Arc::new(Mutex::new(StageClock::default()));
    let mut refresh_enabled = false;
    let wt = if direct_mode {
        // 별도 브랜치/워크트리 없이 선택한 로컬 브랜치의 메인 체크아웃을 사용한다.
        // claim은 Task 행 생성까지 유지되어 동시 요청도 아래 DB 가드를 반드시 보게 한다.
        // 직접 모드에는 최신화도 부트스트랩도 없다 — 단계는 체크아웃 준비 하나뿐이다.
        emit_creating(app, client_ref.as_deref(), "worktree");
        clock.lock().unwrap().enter(worktree::CreateStage::Worktree);
        let (worktree, claim) = prepare_direct_worktree(
            &pool,
            &state.direct_repo_locks,
            repo_path,
            base_branch.as_deref(),
        )
        .await?;
        clock.lock().unwrap().close();
        direct_claim = Some(claim);
        worktree
    } else {
        // 유일성: 나노초 접미 (동초·동일지시문 충돌 방지).
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let branch = format!("dojang/{}-{}", worktree::slugify(&instruction), suffix);
        // git 서브프로세스 동기 대기 — async 워커 점유 방지를 위해 blocking 풀로 분리.
        let repo_owned = repo_path.to_path_buf();
        let base = base_branch.clone();
        // 최신화도 여기서 함께 일어난다(네트워크 왕복 포함) — blocking 풀이라 async 워커를
        // 점유하지 않는다. UI 기원만 최신화한다: 무인 실행에는 자격증명 프롬프트를 받아 줄
        // 사람이 없다.
        let refresh = origin == TaskOrigin::Ui && refresh_base_on(&pool, &repo).await;
        refresh_enabled = refresh;
        let stage_app = app.clone();
        let stage_ref = client_ref.clone();
        let stage_clock = clock.clone();
        let (wt, outcome) = tauri::async_runtime::spawn_blocking(move || {
            let on_stage = move |stage: worktree::CreateStage| {
                stage_clock.lock().unwrap().enter(stage);
                emit_creating(&stage_app, stage_ref.as_deref(), stage.as_str());
            };
            worktree::create(&repo_owned, &branch, base.as_deref(), refresh, &on_stage)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
        clock.lock().unwrap().close();
        refresh_outcome = Some(outcome);
        wt
    };

    let cwd = wt.path.to_string_lossy().into_owned();

    let agent_label = (!agent.trim().is_empty()).then_some(agent.as_str());
    let ensemble_label = (!ensemble.trim().is_empty()).then_some(ensemble.as_str());
    let service = TaskService::new(pool.clone());
    let task = service
        .create_task(
            TaskDraft {
                repo: repo.clone(),
                branch: wt.branch.clone(),
                base: wt.base.clone(),
                worktree_path: cwd.clone(),
                instruction: instruction.clone(),
                agent: agent_label.map(str::to_string),
                role,
                ensemble: ensemble_label.map(str::to_string),
                mode: mode.clone(),
                goal_contract: goal_contract.clone(),
                ambiguity: ambiguity.clone(),
            },
            Some(&model),
            Some(&reasoning_effort),
            now(),
        )
        .await?;
    let id = task.id;
    let purpose_binding = if let Some(selection) = purpose_selection.as_ref() {
        crate::project_plan::bind_initial(&pool, id, selection, now()).await
    } else if let Some(parent) = resume_from {
        crate::project_plan::inherit_task(&pool, parent, id, now()).await
    } else { Ok(()) };
    if let Err(error) = purpose_binding {
        let _ = db::update_state(&pool, id, tstate::FAILED, now()).await;
        if !direct_mode { let _ = wt.discard(); }
        return Err(format!("목적 선택이 변경되어 작업을 시작하지 않았습니다: {error}"));
    }
    if let Some(tier) = service_tier.as_deref() {
        let saved = db::set_task_service_tier(&pool, &task, tier).await;
        if !matches!(saved, Ok(true)) {
            let _ = db::update_state(&pool, id, tstate::FAILED, now()).await;
            if !direct_mode { let _ = wt.discard(); }
            return Err(saved.err().map(|e| e.to_string()).unwrap_or_else(|| "실행 속도를 저장하지 못했습니다".into()));
        }
    }
    if let Some(runtime) = question_runtime {
        let bound = match crate::convo::interaction::bind_runtime(&pool, id, runtime).await {
            Ok(()) if approvals => crate::convo::interaction::set_approvals(&pool, id, true).await,
            other => other,
        };
        if let Err(error) = bound {
            let _ = db::update_state(&pool, id, tstate::FAILED, now()).await;
            if !direct_mode { let _ = wt.discard(); }
            return Err(error);
        }
    }
    let prep_ms = t0.elapsed().as_millis() as u64;
    // 여기서부터 spawn 직전까지의 실패는 `metric.prepared`에 닿지 못한다(설계 0059 §5.2).
    // 사유는 각 경로가 이미 이벤트로 남기지만 **소요 시간은 아무 데도 남지 않아서**,
    // 오래 기다린 끝의 실패와 즉시 실패가 기록에서 같은 모양이 된다. 단계마다 아는 값이
    // 달라 `memory_ms`만 인자로 받는다.
    let failure_detail = |memory_ms: u64, failed_at: &str| {
        let clock = clock.lock().unwrap();
        create_failure_detail(
            &prepared_metric_detail(
                origin,
                direct_mode,
                refresh_enabled,
                &clock,
                prep_ms,
                memory_ms,
                false,
                check_version_ms,
                refresh_outcome.as_ref(),
            ),
            failed_at,
        )
    };

    // diff 기준점은 생성 직후 1회만 기록한다. TaskDraft에 필드를 더하지 않는 이유는 삽입
    // 체인 세 함수와 orchestrator 경로까지 인자가 번지기 때문이다(설계 0053 D6).
    // 실패해도 작업 생성을 막지 않는다 — 기록이 없으면 레거시 경로로 돈다.
    if let Some(revision) = wt.base_revision.as_deref() {
        let _ = db::set_base_revision(&pool, id, revision).await;
    }

    // 이어받기 — 원본의 벤더 세션을 물려받는다. **spawn 앞**이어야 한다: 뒤로 미루면 첫 턴만
    // 문맥 없이 돌고 세션은 다음 턴부터 이어져, 사용자에게는 에이전트가 한 번 기억을 잃었다가
    // 되찾는 것으로 보인다. 실패를 삼키지 않는 이유도 같다 — 조용히 새 대화로 시작하면
    // 이어받기를 눌렀다는 사실만 남고 결과는 아무것도 이어지지 않는다.
    if let Some(source_id) = resume_from {
        match db::adopt_conversation(&pool, id, source_id, now()).await {
            // 세션이 안 넘어왔다 — 원장만 이어지고 에이전트는 아무것도 기억하지 못한다. 막지는
            // 않는다(브랜치와 이력은 여전히 쓸모가 있다). 대신 원장에 남겨 사용자가 화면에서
            // 본다: 이 줄이 없으면 앞의 이력을 읽은 에이전트인 줄 알고 말을 건다.
            Ok(false) => {
                let _ = db::append_convo_event(
                    &pool,
                    id,
                    &serde_json::json!({
                        "kind": "resume_no_context",
                        "source_task_id": source_id,
                    })
                    .to_string(),
                    now(),
                )
                .await;
            }
            Ok(true) => {}
            // 이어받기가 실패했는데 새 대화로 시작해 버리면, 사용자에게는 이어받기를 눌렀다는
            // 사실만 남고 문맥은 하나도 오지 않는다. 시작하지 않고 자리를 치운다 —
            // 메모리 투영 실패와 같은 모양이다.
            Err(error) => {
                let reason = error.to_string();
                let _ = db::update_state(&pool, id, tstate::FAILED, now()).await;
                let _ =
                    db::append_event(&pool, id, "resume_adopt_failed", Some(&reason), now()).await;
                let detail = failure_detail(0, "resume_task");
                let _ =
                    db::append_event(&pool, id, "metric.create_failed", Some(&detail), now()).await;
                if !direct_mode {
                    let _ = wt.discard();
                }
                return Err(format!("이어받기에 실패해 작업을 시작하지 않았습니다: {reason}"));
            }
        }
    }

    // 세션홈 승계 — 작업 행이 없는 벤더 세션(터미널에서 만든 세션 등)을 새 작업이 물려받는다.
    // 위 작업 id 승계와 자리는 같다(spawn 앞) 이유도 같다. 다른 점은 실패 모양뿐이다: 원본
    // 작업 행이 없으므로 `Ok(false)`(문맥 없이 이어받음) 3상태가 없고 실패 아니면 성공이다
    // (설계 2026-09-17 결정 4). 원장에는 앞선 이벤트가 하나도 없으므로(화면은 빈 대화인데
    // 벤더는 문맥을 가짐, `resume_no_context`의 정반대) 성공 시 출처를 알리는 이벤트를 남긴다
    // (결정 6).
    if let Some(session_id) = resume_session {
        // 해석(디렉터리 순회)과 출처 메타 추출(선두/말미 샘플링)은 동기 파일시스템 IO다 —
        // `describe`가 둘을 한 번에 하고, blocking 풀로 내보내 UI 스레드를 막지 않는다.
        let lookup = session_id.clone();
        let vendor = resume_vendor.as_deref().unwrap_or("claude").to_string();
        let lookup_vendor = vendor.clone();
        let described = match tauri::async_runtime::spawn_blocking(move || {
            sessionhome::describe_vendor(&lookup_vendor, &lookup)
        })
        .await
        {
            Ok(result) => result.map_err(|e| e.to_string()),
            Err(join) => Err(join.to_string()),
        };
        let adopted = match described {
            Ok(meta) => db::adopt_external_session_vendor(&pool, id, &vendor, &session_id, now())
                .await
                .map(|()| meta)
                .map_err(|e| e.to_string()),
            Err(reason) => Err(reason),
        };
        match adopted {
            Ok(meta) => {
                let _ = db::append_convo_event(
                    &pool,
                    id,
                    &resume_external_event(&session_id, &meta).to_string(),
                    now(),
                )
                .await;
            }
            // resolve 실패("세션 없음"/"인가되지 않음"을 뭉친 404 상당)와 adopt_external_session의
            // 충돌(진행 중인 작업 id를 담은 409 상당, `AdoptError::Conflict`의 Display가 이미
            // 그 id를 문구에 싣는다) 모두 여기로 모인다 — 로컬 커맨드는 상태 코드가 없으므로
            // 메시지 문구로 그 구분을 낸다(결정 9).
            Err(reason) => {
                let _ = db::update_state(&pool, id, tstate::FAILED, now()).await;
                let _ =
                    db::append_event(&pool, id, "resume_adopt_failed", Some(&reason), now()).await;
                let detail = failure_detail(0, "resume_session");
                let _ =
                    db::append_event(&pool, id, "metric.create_failed", Some(&detail), now()).await;
                if !direct_mode {
                    let _ = wt.discard();
                }
                return Err(format!("이어받기에 실패해 작업을 시작하지 않았습니다: {reason}"));
            }
        }
    }

    // 최신화 결과를 알린다. **작업 생성 성공 여부와 무관한 부가 정보**이므로 이벤트로 보낸다 —
    // 반환값에 실으면 이 정보를 안 쓰는 모든 호출부가 타입을 떠안는다.
    // 정상(`Skipped`·`AlreadyCurrent`)은 보내지 않는다: 매번 알리면 그 줄은 곧 안 읽힌다.
    if let Some(outcome) = refresh_outcome.clone().filter(|o| o.is_noteworthy()) {
        let _ = app.emit(
            "worktree://base-refresh",
            BaseRefreshPayload { id, outcome },
        );
    }

    // 메모리 파일(`<memory_root>/<repo-key>/MEMORY.md`·`USER.md`)을 agent spawn 전에 투영한다.
    // 검색도 임베딩도 없다 — 저장소의 파일 하나가 정본이고 블록은 그 사본이다(설계 2026-09-13).
    emit_creating(app, client_ref.as_deref(), "memory");
    let memory_started = Instant::now();
    let data_dir = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| crate::memory::file::data_dir(&pool));
    let targets = crate::projector::project_targets();
    if let Err(error) =
        project_memory_or_fail(&pool, &data_dir, &repo, id, &wt, &targets, direct_mode).await
    {
        // 정리와 사유 기록은 `project_memory_or_fail`이 이미 했다. 여기서 더하는 것은
        // **투영까지 얼마나 걸렸는가** 하나다.
        let detail = failure_detail(memory_started.elapsed().as_millis() as u64, "memory");
        let _ = db::append_event(&pool, id, "metric.create_failed", Some(&detail), now()).await;
        return Err(error);
    }
    let memory_ms = memory_started.elapsed().as_millis() as u64;
    // 격리 worktree에만 활성 MCP 서버를 주입한다. 직접 모드의 사용자 체크아웃은 변경하지 않는다.
    // 기존 .mcp.json은 사용자 소유이므로 보존하고, 실제 생성한 경우만 task event로 표시한다.
    if !direct_mode {
        if let Ok(servers) = mcp_registry::list_servers(&pool).await {
            let merged = merge_lsp_autoinject(&pool, &servers, &wt.path).await;
            if matches!(
                mcp_registry::write_mcp_config(&merged, &wt.path),
                Ok(mcp_registry::McpConfigWrite::Created(_))
            ) {
                let _ = db::append_event(&pool, id, "mcp_generated", None, now()).await;
            }
        }
    }

    // 계측 행은 **External 조기 반환 앞**에 남긴다 — 뒤에 두면 승인 대기 작업의 행이 통째로
    // 빠져 표본이 UI 기원만 남는다. 투영이 실패한 작업은 여기 닿지 않는다(설계 0059 §5.2).
    let detail = {
        let clock = clock.lock().unwrap();
        prepared_metric_detail(
            origin,
            direct_mode,
            refresh_enabled,
            &clock,
            prep_ms,
            memory_ms,
            // 파일형 메모리는 임베딩을 쓰지 않는다. 필드는 표본의 모양을 지키려고 남긴다 —
            // 지우면 이전 표본과 필드 수가 달라져 비교가 끊긴다(P2에서 함께 정리).
            false,
            check_version_ms,
            refresh_outcome.as_ref(),
        )
    };
    let _ = db::append_event(&pool, id, "metric.prepared", Some(&detail), now()).await;

    if questions {
        if let Err(error)=crate::convo::interaction_commands::ensure_generation(creation_generation) {
            db::update_state(&pool,id,tstate::FAILED,now()).await.map_err(|e|e.to_string())?;
            if !direct_mode {let _=wt.discard();}
            return Err(error);
        }
    }

    if origin == TaskOrigin::External {
        // 외부기원 — 즉시 spawn하지 않고 승인 대기. worktree는 이미 생성됨(승인 시 재사용).
        state.tasks.lock().unwrap().insert(
            id,
            ActiveTask {
                worktree: wt,
                session: None,
                preview_mcp: None,
            },
        );
        // 삽입 직후 예약 반납 — 실제 카운트(tasks.len())가 이제 이 슬롯을 커버한다.
        drop(reservation);
        db::update_state(&pool, id, tstate::PENDING_APPROVAL, now())
            .await
            .map_err(|e| e.to_string())?;
        let _ = db::append_event(&pool, id, "pending_approval", None, now()).await;
        return db::get_task(&pool, id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "생성된 Task를 찾을 수 없음".to_string());
    }

    emit_creating(app, client_ref.as_deref(), "spawn");
    let fallback = SpawnFallback {
        cmd,
        args,
        cols,
        rows,
    };
    spawn_task_agent_inner(
        app,
        state,
        &pool,
        id,
        &repo,
        &cwd,
        &instruction,
        &agent,
        headless,
        &mode,
        wt,
        &fallback,
        client_ref.as_deref(),
        Some(reservation),
    )
    .await?;
    // startup 복구는 같은 repo 락을 잡고 `Created`를 재확인한다. 실제 생성 파이프라인이
    // Running/Failed로 전이하기 전에는 live 행을 stale로 오판할 수 없도록 여기까지 유지한다.
    drop(direct_claim);

    db::get_task(&pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "생성된 Task를 찾을 수 없음".to_string())
}

/// 승인/생성 공용 spawn 로직 — worktree/repo/cwd/instruction/agent/headless/mode를 받아
/// conversation은 stream-json 턴 시작, terminal은 PTY 에이전트 실행. 실패 시 FAILED 전이.
#[allow(clippy::too_many_arguments)]
async fn spawn_task_agent_inner(
    app: &AppHandle,
    state: &AppState,
    pool: &SqlitePool,
    id: i64,
    repo: &str,
    cwd: &str,
    instruction: &str,
    agent: &str,
    headless: bool,
    mode: &str,
    wt: Worktree,
    fallback: &SpawnFallback,
    client_ref: Option<&str>,
    // 생성 경로(create_task_internal)에서만 Some — 삽입 직후 drop해 예약을 실제 카운트로 전환.
    // 승인 경로(spawn_task_agent)는 이미 tasks에 등록된 task를 재사용하므로 None(예약 불필요).
    reservation: Option<SlotReservation<'_>>,
) -> Result<(), String> {
    // 여기가 모든 작업 spawn 의 초크포인트다. 호출부마다 거는 대신 여기서 한 번 막는다 —
    // 승인 경로(`spawn_task_agent`)와 텔레그램 `/approve` 는 사람이 보지 않는 시점에도
    // 들어오므로, 호출부에 거는 방식은 새 경로가 생길 때마다 조용히 빠진다.
    let managed_codex = mode == "conversation" && agent == "codex"
        && crate::convo::interaction::runtime_of(pool, id).await?.as_deref()
            == Some(crate::convo::interaction::RUNTIME);
    if !managed_codex {
        refuse_while_updating(state)?;
    }
    let attempt = if mode == "conversation" || !cfg!(target_os = "macos") {
        None
    } else {
        begin_vault_attempt(pool, id, repo, instruction, client_ref)
            .await
            .map_err(|error| error.to_string())?
    };
    if let Some(attempt) = attempt.as_deref() {
        crate::knowledge::vault::provenance::record_unknown_input_for_attempt(
            pool,
            attempt,
            crate::knowledge::vault::provenance::InputOrigin::ToolResult,
            "terminal transcript may include untracked tool output",
            now(),
        )
        .await
        .map_err(|error| error.to_string())?;
    }
    if mode == "conversation" {
        // 대화 모드: PTY 없이 stream-json — 초기 지시를 convo로 실행(후속은 convo_send가 --resume).
        // 턴이 끝나면 `start_convo_turn`의 에필로그가 AwaitingReview로 전이하고 `task://state`를
        // 보낸다. 여기서 쓰는 RUNNING은 첫 턴 동안의 상태이며, 생성 응답이 이 값을 싣고 돌아간다.
        state.tasks.lock().unwrap().insert(
            id,
            ActiveTask {
                worktree: wt,
                session: None,
                preview_mcp: None,
            },
        );
        drop(reservation);
        db::update_state(pool, id, tstate::RUNNING, now())
            .await
            .map_err(|e| e.to_string())?;
        let _ = db::append_event(pool, id, "running", None, now()).await;
        // Admission failure must leave a reviewable task, including the opt-in adapter setup.
        let start_result = start_convo_turn(
            app.clone(),
            pool.clone(),
            state.convo_active.clone(),
            id,
            repo.to_string(),
            cwd.to_string(),
            instruction.to_string(),
            Vec::new(),
            None,
            ConversationInputOrigin::InitialTask,
            client_ref.map(str::to_owned),
            state.updating.clone(),
            None,
            ConvoAdmissionAction::PreserveTakeover,
        )
        .await;
        if let Err(error)=start_result {
            db::mark_awaiting_review_with_notification(pool,id,now(),None,"failure").await.map_err(|e|e.to_string())?;
            return Err(error);
        }

    } else {
        // 터미널 모드: 리드 에이전트(선택형)를 PTY로 직접 실행 — bare 셸 대신 에이전트와 바로 대화.
        // headless=true(앙상블): 자율수행+자동승인 호출 → 종료 시 diff 후보 생성.
        // headless=false(단일): 인터랙티브 + 지시문 첫 프롬프트 시드.
        // 선택 에이전트를 PATH에서 못 찾으면 전달된 셸로 폴백(하드 실패 방지).
        // ⚠️ PTY(portable_pty)는 프로그램명을 PATH로 해석하지 않으므로 **절대경로**로 스폰한다.
        let model = model_for_task(pool, id, agent).await;
        let task = db::get_task(pool, id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
        let instruction = vault_delivery(pool, id, repo, instruction, client_ref, attempt.as_ref())
            .await
            .map_err(|error| error.to_string())?
            .map(|item| crate::knowledge::vault::retrieval::delivery_payload(instruction, &item.preview))
            .unwrap_or_else(|| instruction.to_string());
        let prompt =
            crate::goal_contract::execution_prompt(&instruction, task.goal_contract.as_deref());
        let round = crate::preview_bridge::random_hex_id()?;
        let purpose = crate::project_plan::prepare_purpose(pool, id, &round, now()).await.map_err(|e|e.to_string())?;
        let prompt = if let Some(candidate) = purpose.as_ref() {
            let mut tx=pool.begin().await.map_err(|e|e.to_string())?;
            crate::project_plan::persist_purpose_tx(&mut tx,candidate).await.map_err(|e|e.to_string())?;
            tx.commit().await.map_err(|e|e.to_string())?;
            crate::project_plan::render_purpose(candidate,&prompt)
        } else { prompt };
        let resolved = if headless {
            crate::agent::headless_args_with_effort(
                agent,
                &prompt,
                model.as_deref(),
                task.reasoning_effort.as_deref(),
                Some(&crate::agent::session_name(id)),
            )
        } else {
            crate::agent::agent_args_with_effort(
                agent,
                &prompt,
                model.as_deref(),
                task.reasoning_effort.as_deref(),
            )
        };
        let (run_cmd, mut run_args, prompt_supplied, resolved_agent):
            (String, Vec<String>, bool, bool) =
            match resolved.and_then(|(bin, a)| crate::reviewer::which(&bin).map(|abs| (abs, a))) {
                Some((bin, args)) => (bin, args, true, true),
                None => (fallback.cmd.clone(), fallback.args.clone(), false, false),
            };

        // 프리뷰 MCP 주입은 인자 형태를 아는 두 벤더에만 건다 — 셸 폴백·커스텀 명령에 넣으면
        // 알 수 없는 플래그로 즉사한다.
        let vendor = (resolved_agent && matches!(agent.trim(), "claude" | "codex"))
            .then(|| crate::convo::Vendor::from_agent(agent));
        let lease = vendor.and_then(|vendor| issue_preview_mcp(app, state, id, vendor, false));
        let run_env = match &lease {
            Some(lease) => {
                crate::preview_bridge::mcp::inject::splice_before_instruction(
                    &mut run_args,
                    lease.injection().args.clone(),
                    prompt.trim(),
                );
                lease.injection().env.clone()
            }
            None => Vec::new(),
        };

        let session = match spawn_agent(
            app.clone(),
            pool.clone(),
            id,
            repo.to_string(),
            cwd.to_string(),
            &run_cmd,
            &run_args,
            fallback.cols,
            fallback.rows,
            &run_env,
        ) {
            Ok(s) => s,
            Err(e) => {
                if let Some(attempt) = attempt.as_deref() {
                    let _ = crate::knowledge::vault::usage::mark_delivery(
                        pool,
                        id,
                        attempt,
                        crate::knowledge::vault::usage::DeliveryState::NotDelivered,
                        now(),
                    )
                    .await;
                }
                // 직접 모드는 메인 체크아웃 그 자체이므로 절대 discard(worktree remove/branch -D)하지 않는다.
                if !is_direct_mode(&wt) {
                    let _ = wt.discard();
                }
                let _ = db::fail_created_task_with_notification(pool, id, now()).await;
                return Err(e);
            }
        };

        if let Some(attempt) = attempt.as_deref() {
            use crate::knowledge::vault::usage::DeliveryState;
            let state = if prompt_supplied {
                DeliveryState::Delivered
            } else {
                DeliveryState::NotDelivered
            };
            let _ = crate::knowledge::vault::usage::mark_delivery(pool, id, attempt, state, now())
                .await;
        }

        db::update_state(pool, id, tstate::RUNNING, now())
            .await
            .map_err(|e| e.to_string())?;
        let _ = db::append_event(pool, id, "running", None, now()).await;
        state.tasks.lock().unwrap().insert(
            id,
            ActiveTask {
                worktree: wt,
                session: Some(session),
                preview_mcp: lease,
            },
        );
        drop(reservation);
    }
    Ok(())
}

async fn begin_vault_attempt(
    pool: &SqlitePool,
    task_id: i64,
    repo: &str,
    input: &str,
    client_ref: Option<&str>,
) -> anyhow::Result<Option<VaultAttempt>> {
    let explicit_snapshot = match client_ref {
        Some(client_ref) => crate::knowledge::vault::selected_attachments::has_explicit_snapshot(pool, client_ref).await?,
        None => false,
    };
    let Some(vault_id) = crate::knowledge::vault::provenance::active_writable_vault(pool).await?
    else {
        if explicit_snapshot {
            anyhow::bail!("selected attachments cannot be delivered because the vault is not active")
        }
        return Ok(None);
    };
    let Some(binding) = crate::knowledge::vault::resolve_project(pool, Path::new(repo)).await?
    else {
        if explicit_snapshot {
            anyhow::bail!("selected attachments cannot be delivered because the repository binding changed")
        }
        return Ok(None);
    };
    let profile = crate::capture::invoke::profile(pool).await;
    let provider = crate::capture::invoke::provider_identity(&profile);
    let selected_policy = match client_ref {
        Some(client_ref) => crate::knowledge::vault::selected_attachments::pending_policy(
            pool, &binding, input, client_ref,
        ).await?,
        None => None,
    };
    if explicit_snapshot && selected_policy.is_none() {
        anyhow::bail!("selected attachments no longer belong to this repository binding; prepare them again")
    }
    let selected_preview_id = selected_policy.as_ref().map(|item| item.preview_id.clone());
    let restrictive_policy_seen = match client_ref {
        Some(client_ref) => crate::knowledge::vault::provenance::restrictive_draft_policy_seen(
            pool, &binding, client_ref,
        )
        .await?,
        None => false,
    };
    let policy = if selected_policy.is_some() {
        None
    } else { match client_ref {
        Some(client_ref) => {
            crate::knowledge::vault::provenance::consume_draft_policy(
                pool, &binding, client_ref, input, task_id,
            )
            .await?
        }
        None => None,
    }};
    let attempt = crate::knowledge::vault::provenance::start_attempt(
        pool,
        task_id,
        &vault_id,
        &binding,
        &provider,
        client_ref,
        now(),
    )
    .await?;
    let policy_is_current = match &policy {
        Some(policy) => {
            crate::knowledge::vault::provenance::draft_policy_current(pool, policy).await?
        }
        None => false,
    };
    match selected_policy.map(|item| item.policy).or_else(|| policy.filter(|_| policy_is_current)) {
        Some(policy)
            if policy.input_mode == "task_only" || policy.input_mode == "private_attachment" =>
        {
            crate::knowledge::vault::provenance::record_task_only_input(
                pool,
                &attempt,
                input,
                now(),
            )
            .await?;
            for source in &policy.sources {
                crate::knowledge::vault::provenance::record_private_revision_input(
                    pool,
                    &attempt,
                    source,
                    now(),
                )
                .await?;
            }
        }
        None if restrictive_policy_seen => crate::knowledge::vault::provenance::record_task_only_input(
            pool,
            &attempt,
            input,
            now(),
        )
        .await?,
        _ => {
            crate::knowledge::vault::provenance::record_user_input(
                pool,
                task_id,
                input,
                now(),
            )
            .await?
        }
    }
    Ok(Some(VaultAttempt {
        id: attempt,
        selected_preview_id,
    }))
}

#[derive(Clone)]
struct VaultAttempt {
    id: String,
    selected_preview_id: Option<String>,
}

impl std::ops::Deref for VaultAttempt {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.id
    }
}

struct VaultDelivery {
    preview: crate::knowledge::vault::retrieval::ReferencePreview,
}

async fn vault_delivery(
    pool: &SqlitePool,
    task_id: i64,
    repo: &str,
    query: &str,
    client_ref: Option<&str>,
    attempt: Option<&VaultAttempt>,
) -> anyhow::Result<Option<VaultDelivery>> {
    let (Some(client_ref), Some(attempt)) = (client_ref, attempt) else {
        return Ok(None);
    };
    let expected_selected_snapshot = attempt.selected_preview_id.as_deref();
    let Some(binding) = crate::knowledge::vault::resolve_project(pool, Path::new(repo)).await?
    else {
        if expected_selected_snapshot.is_some() {
            anyhow::bail!("selected attachments cannot be delivered because the repository binding changed")
        }
        return Ok(None);
    };
    if let Some(expected_preview_id) = expected_selected_snapshot {
        let preview = crate::knowledge::vault::selected_attachments::consume_expected(
            pool, &binding, query, client_ref, expected_preview_id, task_id,
        ).await?;
        for reference in &preview.references {
            let document_id: String = sqlx::query_scalar("SELECT document_id FROM vault_revisions WHERE id = ?")
                .bind(&reference.revision_id).fetch_one(pool).await?;
            if matches!(crate::knowledge::vault::scope_for_sources(pool, std::slice::from_ref(&reference.revision_id)).await?, Some(crate::knowledge::vault::Scope::PrivateData)) {
                continue;
            }
            crate::knowledge::vault::provenance::record_revision_input(pool, &attempt.id, &document_id, &reference.revision_id, &reference.revision_hash, now()).await?;
        }
        crate::knowledge::vault::usage::record_pending(pool, task_id, &attempt.id, &preview, now()).await?;
        return Ok(Some(VaultDelivery { preview }));
    }
    if crate::knowledge::vault::selected_attachments::has_explicit_snapshot(pool, client_ref).await? {
        anyhow::bail!("selected attachments changed after delivery preflight; prepare them again")
    }
    let preview = crate::knowledge::vault::retrieval::consume_preview(
        pool, &binding, query, client_ref, task_id,
    )
    .await?;
    let mut references = preview.map_or_else(Vec::new, |preview| preview.references);
    let remaining_references = 5usize.saturating_sub(references.len());
    let remaining_bytes = 8 * 1024usize
        - references.iter().map(|reference| reference.snippet.len()).sum::<usize>();
    references.extend(
        crate::knowledge::vault::retrieval::consumed_private_policy_references(
            pool,
            &binding,
            client_ref,
            task_id,
            remaining_references,
            remaining_bytes,
        )
        .await?,
    );
    let preview = crate::knowledge::vault::retrieval::ReferencePreview {
        id: format!("delivery:{task_id}:{client_ref}"),
        query_hash: String::new(),
        created_at: now(),
        references,
    };
    if preview.references.is_empty() {
        return Ok(None);
    }
    for reference in &preview.references {
        let document_id: String =
            sqlx::query_scalar("SELECT document_id FROM vault_revisions WHERE id = ?")
                .bind(&reference.revision_id)
                .fetch_one(pool)
                .await?;
        if matches!(
            crate::knowledge::vault::scope_for_sources(
                pool,
                std::slice::from_ref(&reference.revision_id),
            )
                .await?,
            Some(crate::knowledge::vault::Scope::PrivateData)
        ) {
            continue;
        }
        crate::knowledge::vault::provenance::record_revision_input(
            pool,
            &attempt.id,
            &document_id,
            &reference.revision_id,
            &reference.revision_hash,
            now(),
        )
        .await?;
    }
    crate::knowledge::vault::usage::record_pending(pool, task_id, attempt, &preview, now()).await?;
    Ok(Some(VaultDelivery { preview }))
}

/// PENDING_APPROVAL 작업 승인 시 재사용하는 spawn 진입점 — DB에서 task 재구성 후 spawn.
/// worktree는 활성 맵(생성 시 넣어둔 것)에서 가져오고, 없으면 DB 행에서 재구성(폴백).
pub(crate) async fn spawn_task_agent(
    app: &AppHandle,
    state: &AppState,
    task: &Task,
) -> Result<(), String> {
    let pool = pool_of(state)?;
    let wt = match state.tasks.lock().unwrap().remove(&task.id) {
        Some(a) => a.worktree,
        None => worktree_from_task(task),
    };
    // External 작업은 항상 headless_terminal로 생성됨 — fallback은 빈 cmd/기본 크기로 충분.
    let fallback = SpawnFallback {
        cmd: String::new(),
        args: Vec::new(),
        cols: 80,
        rows: 24,
    };
    spawn_task_agent_inner(
        app,
        state,
        &pool,
        task.id,
        &task.repo,
        &task.worktree_path,
        &task.instruction,
        task.agent.as_deref().unwrap_or(""),
        true,
        &task.mode,
        wt,
        &fallback,
        None,
        None,
    )
    .await
}

/// `client_ref` 허용 길이 — UUID(36자)면 충분하다.
const CLIENT_REF_MAX: usize = 64;

fn valid_vault_followup_client_ref(id: i64, client_ref: &str) -> bool {
    let legacy = format!("vault-followup:{id}");
    client_ref == legacy
        || (client_ref.len() <= CLIENT_REF_MAX
            && client_ref
                .strip_prefix(&(legacy + ":"))
                .is_some_and(|nonce| !nonce.is_empty()))
}

/// 새 Task 생성 IPC 래퍼 — 본 로직은 `create_task_internal`(크론/봇과 공용).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn task_create(
    app: AppHandle,
    state: State<'_, AppState>,
    repo: String,
    instruction: String,
    agent: String,
    // 작업 책임 역할. 미전달/빈 값은 기존 클라이언트 호환을 위해 implementer.
    role: Option<String>,
    // 세션 단위 모델 오버라이드 — 미전달/빈 값이면 설정의 벤더 기본(`model:<agent>`) 사용.
    model: Option<String>,
    // Codex 세션 단위 reasoning override — 미전달/빈 값이면 Codex 설정 기본값 사용.
    reasoning_effort: Option<String>,
    service_tier: Option<String>,
    headless: bool,
    ensemble: String,
    // "terminal"(PTY) 또는 "conversation"(stream-json). 빈 값은 terminal로 취급(구 호출 호환).
    mode: String,
    cmd: String,
    args: Vec<String>,
    cols: u16,
    rows: u16,
    // Immutable task-scoped goal contract. 미전달 시 기존 instruction-only 의미를 유지한다.
    goal_contract: Option<crate::goal_contract::GoalContract>,
    // 인터뷰 결정화 모호성 점수 — 계약과 함께 1회 기록. 미전달이면 NULL 유지.
    ambiguity: Option<crate::interview::AmbiguityScore>,
    // 로컬 시작 브랜치. 격리는 분기 기준, 직접 실행은 메인 checkout 대상이다.
    // 미전달이면 레포의 현재 checkout에서 시작한다(종전 동작).
    base_branch: Option<String>,
    // 이 생성의 진행 이벤트를 프런트가 자기 것으로 식별하는 토큰. 미전달이면 이벤트 없음
    // (구 호출 호환). 길이만 본다 — 우리가 발급한 토큰은 짧고, 긴 값은 우리 것이 아니다.
    client_ref: Option<String>,
    // 세션홈에서 고른 벤더 세션 id. 지정하면 삽입 직후 그 세션을 물려받는다
    // (`sessionhome::resolve` + `db::adopt_external_session`). 미전달이면 새 대화다.
    // 작업 id 이어받기(`task_resume`)와 달리 원본 작업 행이 없어 agent·model 등 나머지 필드를
    // 대신 채워 줄 곳이 없다 — 그래서 `task_resume`처럼 별도 얇은 커맨드를 두지 않고, 이미
    // 전체 생성 파라미터를 받는 이 커맨드에 선택 필드로 얹는다(설계 2026-09-17 결정 2).
    resume_session: Option<String>,
    resume_vendor: Option<String>,
    purpose_selection: Option<crate::project_plan::PurposeSelection>,
    // Claude 질문 세션의 실행 전 승인. 미전달이면 끈다(종전 동작).
    approvals: Option<bool>,
) -> Result<Task, String> {
    create_task_internal(
        &app,
        &state,
        CreateTaskParams {
            repo,
            instruction,
            agent,
            role: role.unwrap_or_default(),
            model: model.unwrap_or_default(),
            reasoning_effort: reasoning_effort.unwrap_or_default(),
            service_tier,
            headless,
            ensemble,
            mode,
            cmd,
            args,
            cols,
            rows,
            origin: TaskOrigin::Ui,
            goal_contract,
            ambiguity,
            base_branch,
            client_ref: client_ref.filter(|r| r.len() <= CLIENT_REF_MAX),
            // 작업 id 이어받기는 `task_resume`만 통한다.
            resume_from: None,
            resume_session,
            resume_vendor,
            purpose_selection,
            approvals: approvals.unwrap_or(false),
        },
    )
    .await
}

/// 끝난 대화를 이어받는다 — **새 작업이 옛 벤더 세션을 물려받고**, `message`로 첫 턴을 돈다.
///
/// 되살리지 않고 새로 만드는 것이 이 기능의 전부다. 종결 상태는 그 작업이 무엇으로 끝났는지의
/// 기록이고 승인된 diff·폐기 시점 커밋·주석이 거기 매달려 있다 — 되돌리면 그 기록이 한 번 더
/// 움직인다. 벤더 쪽 의미론도 같은 모양이다: `--resume`은 죽은 프로세스를 살리는 것이 아니라
/// **새 프로세스가 세션 파일을 물려받는 것**이다. 우리가 하는 일을 거기 맞춘다.
///
/// 워크트리는 원본 브랜치가 아직 살아 있으면 거기서, 없으면 원본 base에서 갈라진다. 승인으로
/// 끝난 작업은 브랜치까지 지워지므로(`cleanup_after_finalization`) 후자가 이상 경로가 아니다 —
/// 그 변경은 이미 base에 들어가 있어서 base에서 갈라도 이어받은 코드 위에 선다.
///
/// 원본 행은 읽기만 한다. 상태도 워크트리도 건드리지 않으므로, 이어받기를 몇 번 하든 원본
/// 카드는 끝난 그대로 남는다.
#[tauri::command]
pub async fn task_resume(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    message: String,
    client_ref: Option<String>,
) -> Result<Task, String> {
    let pool = pool_of(&state)?;
    let message = message.trim().to_string();
    if message.is_empty() {
        return Err("이어받으려면 첫 메시지가 필요합니다".into());
    }
    let source = db::get_task(&pool, id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
    if source.mode != "conversation" {
        return Err("대화 작업만 이어받을 수 있습니다".into());
    }
    // 진행 중인 작업을 이어받게 두면 같은 벤더 세션을 두 작업이 동시에 resume한다. 그쪽은
    // 이어받기가 아니라 그냥 메시지를 보내면 되는 자리다 — 가드가 그 사실을 말해 준다.
    if !tstate::is_terminal(&source.state) {
        return Err("끝난 대화만 이어받을 수 있습니다 — 진행 중인 작업에는 그대로 메시지를 보내세요".into());
    }
    let session = source
        .convo_session_id
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    // agy가 저장하는 것은 세션 id가 아니라 "직전 대화"라는 센티널이다(print 모드에서 대화 id를
    // 얻을 수 없다). 물려받아 봐야 새 워크트리에는 직전 대화가 없고, 있다면 그것은 이 대화가
    // 아니다 — 이어받았다는 표시만 남고 문맥은 엉뚱한 곳에서 오거나 아예 오지 않는다.
    if session == crate::convo::AGY_CONTINUE {
        return Err(
            "agy 대화는 아직 이어받을 수 없습니다 — 세션 식별자 대신 '직전 대화' 표시만 저장되어 새 작업이 다른 대화를 이어가게 됩니다"
                .into(),
        );
    }
    // 같은 세션을 두 작업이 동시에 resume하면 벤더 쪽 세션 파일을 둘이 번갈아 덮어쓴다.
    // 이어받기가 느릴 때 한 번 더 누르는 것만으로 걸리는 자리라 가드가 필요하다.
    if !session.is_empty() {
        if let Some(live) = db::live_task_with_session_vendor(&pool, source.agent.as_deref().unwrap_or("claude"), session)
            .await
            .map_err(|error| error.to_string())?
        {
            return Err(format!(
                "이 대화는 #{live} 작업이 이미 이어가고 있습니다 — 그 작업에 메시지를 보내세요"
            ));
        }
    }
    // 시작점은 살아 있는 것 중 원본에 가장 가까운 브랜치다. 폐기된 작업은 폐기 시점 커밋이
    // 브랜치에 앵커링되어 있으므로(`preserve_and_retire`) 하던 작업 위에서 이어진다.
    //
    // **직접 모드에서는 승계하지 않는다.** 그쪽의 시작 브랜치는 분기 기준이 아니라 사용자의
    // 메인 체크아웃을 실제로 `git checkout` 하라는 지시다 — 이어받기를 눌렀을 뿐인데 열어 둔
    // 작업물 위에서 브랜치가 바뀐다. 그 경로는 지금 체크아웃한 자리에서 이어간다.
    let repo_path = std::path::PathBuf::from(&source.repo);
    let isolated = use_worktree_on(&pool, &source.repo).await;
    let base_branch = isolated
        .then(|| {
            [source.branch.as_str(), source.base.as_str()]
                .into_iter()
                .map(str::trim)
                .find(|name| !name.is_empty() && worktree::local_branch_exists(&repo_path, name))
                .map(str::to_string)
        })
        .flatten();
    let base_missing = isolated && base_branch.is_none();
    let task = create_task_internal(
        &app,
        &state,
        CreateTaskParams {
            repo: source.repo.clone(),
            instruction: message,
            agent: source.agent.clone().unwrap_or_default(),
            role: source.role.clone(),
            model: source.model.clone().unwrap_or_default(),
            reasoning_effort: source.reasoning_effort.clone().unwrap_or_default(),
            // adopt_conversation copies the stored speed with the matching model/provider.
            service_tier: None,
            headless: false,
            // 앙상블은 승계하지 않는다. 후보를 이어받은 것은 새 후보가 아니라 새 대화다 —
            // 같은 그룹에 넣으면 비교 화면에 끝난 후보와 이어받은 대화가 나란히 선다.
            ensemble: String::new(),
            mode: "conversation".to_string(),
            cmd: String::new(),
            args: Vec::new(),
            cols: 100,
            rows: 30,
            origin: TaskOrigin::Ui,
            // Goal Contract는 승계하지 않는다. 이어받은 세션에는 그 계약이 이미 들어가 있고,
            // 다시 합성하면 같은 요구가 두 번 적힌 프롬프트가 된다.
            goal_contract: None,
            ambiguity: None,
            base_branch,
            client_ref: client_ref.filter(|r| r.len() <= CLIENT_REF_MAX),
            resume_from: Some(id),
            resume_session: None,
            resume_vendor: None,
            purpose_selection: None,
            approvals: false,
        },
    )
    .await?;
    // 원본의 브랜치도 base도 남아 있지 않으면 지금 HEAD에서 갈린다 — 코드는 이어받은 대화가
    // 말하는 자리가 아닐 수 있다. 막을 일은 아니지만(문맥 쪽이 이어받기의 본체다) 조용히
    // 넘기면 나중에 "왜 딴 코드 위에 있나"를 설명할 근거가 없다.
    if base_missing {
        let _ = db::append_event(
            &pool,
            task.id,
            "resume_base_missing",
            Some(&format!(
                "원본 브랜치 '{}'·base '{}' 모두 없어 현재 HEAD에서 시작",
                source.branch, source.base
            )),
            now(),
        )
        .await;
    }
    Ok(task)
}

/// 세션홈(벤더 세션) 목록 — 이어받기 대상을 고르는 화면의 데이터 소스(설계
/// 2026-09-17 결정 10).
///
/// 기본(`all=false`)은 `repo`를 접두로 갖는 cwd(선두 또는 말미)만 낸다 — Praxis 워크트리는
/// `<repo>/.praxis/worktrees/…`라 같은 접두로 함께 잡힌다. `all=true`면 접두 필터를 걷는다.
/// `query`는 제목·첫 메시지·cwd에 대한 대소문자 무시 부분 문자열 검색이다. 정렬은 최근순,
/// 상한은 항상 200 — 호출부가 더 큰 값을 요구할 방법이 없다(파라미터로 받지 않는다).
///
/// 인가 없이 로컬 파일시스템을 읽는 커맨드라 `state`는 지금 쓰지 않는다 — 그래도 `task_resume`과
/// 같은 시그니처 모양을 유지해, 이 커맨드가 페어링·풀을 나중에 필요로 하게 되어도 호출부
/// 시그니처가 바뀌지 않게 한다.
#[tauri::command]
pub async fn session_home_index(
    _app: AppHandle,
    _state: State<'_, AppState>,
    repo: String,
    all: bool,
    query: Option<String>,
) -> Result<Vec<sessionhome::SessionMeta>, String> {
    const LIMIT: usize = 200;
    let filter = sessionhome::ScanFilter {
        cwd_prefixes: if all { Vec::new() } else { vec![repo] },
        query: query.filter(|q| !q.trim().is_empty()),
    };
    // `SessionMeta`를 그대로 낸다 — 필드가 1:1인 IPC 사본을 두면 두 벌이 어긋난다.
    tauri::async_runtime::spawn_blocking(move || sessionhome::scan(&filter, LIMIT))
        .await
        .map_err(|e| e.to_string())
}

/// 공급자와 세션 ID만으로 조회한다. 클라이언트 파일 경로는 받지 않는다.
#[tauri::command]
pub async fn session_home_preview(
    repo: String, vendor: String, session_id: String,
) -> Result<sessionhome::SessionPreview, String> {
    // 로컬은 목록과 동일하게 다른 저장소 선택을 허용한다. repo는 선택 맥락이며 인가가 아니다.
    let _ = repo;
    tauri::async_runtime::spawn_blocking(move || {
        sessionhome::preview_vendor(&vendor, &session_id)
            .map_err(|_| "찾을 수 없거나 접근 권한이 없습니다".to_string())
    }).await.map_err(|e| e.to_string())?
}

/// 작업의 worktree 핸들 스냅샷 — `tasks` 락을 git 서브프로세스 실행 전에 즉시 반납하기 위한
/// 짧은 임계구역(락을 쥔 채 git을 기다리면 spawn/resize 등 다른 커맨드까지 연쇄 지연).
fn task_worktree_snapshot(state: &AppState, id: i64) -> Result<Worktree, String> {
    let tasks = state.tasks.lock().unwrap();
    let a = tasks.get(&id).ok_or("작업을 찾을 수 없습니다")?;
    Ok(a.worktree.clone())
}

/// 워크트리 전체 내용 검색 — QuickOpen의 코드 스코프.
///
/// **경로 가드가 필수다.** 임의 경로를 받으면 앱이 홈 디렉터리 전체를 grep하는 도구가 된다.
/// 작업 id로만 대상을 정해 워크트리 밖으로 나갈 수 없게 한다.
#[tauri::command]
pub async fn project_search(
    state: State<'_, AppState>,
    id: i64,
    query: String,
    case_sensitive: bool,
) -> Result<fsapi::search::SearchResult, String> {
    let wt = task_worktree_snapshot(&state, id)?;
    // CPU 바운드 — 큰 레포에서 수 초가 걸린다. async 워커를 점유하면 UI 전체가 멎는다.
    tauri::async_runtime::spawn_blocking(move || {
        fsapi::search::search(
            &wt.path,
            &query,
            &fsapi::search::SearchOptions {
                case_sensitive,
                include_hidden: false,
            },
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

/// 특정 작업의 변경 요약 (`git diff --stat`).
#[tauri::command]
pub async fn task_diff_stat(state: State<'_, AppState>, id: i64) -> Result<String, String> {
    let wt = task_worktree_snapshot(&state, id)?;
    tauri::async_runtime::spawn_blocking(move || {
        if !worktree::is_git_repository(&wt.path) {
            return Ok(String::new());
        }
        wt.diff_stat().map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 특정 작업의 파일 단위 상세 diff (DiffViewer S-04).
///
/// `range`를 생략하면 세션 전체다. Option으로 받는 이유는 구버전 프론트·모바일이
/// 인자 없이 불러도 깨지지 않게 하기 위해서다.
#[tauri::command]
pub async fn task_diff(
    state: State<'_, AppState>,
    id: i64,
    range: Option<worktree::DiffRange>,
    include_review: Option<bool>,
) -> Result<worktree::TaskDiffResult, String> {
    let wt = task_worktree_snapshot(&state, id)?;
    let range = range.unwrap_or_default();
    let review_inputs = if include_review.unwrap_or(false) {
        let pool = pool_of(&state)?;
        let task = db::get_task(&pool, id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("작업을 찾을 수 없습니다")?;
        let (stored, warning) = match annotations::list_by_task(&pool, id).await {
            Ok(stored) => (stored, None),
            Err(_) => (
                Vec::new(),
                Some("주석을 불러오지 못했지만 diff는 최신 상태입니다.".to_string()),
            ),
        };
        let patterns = task
            .goal_contract
            .as_deref()
            .map(|contract| contract.protected_paths.clone())
            .unwrap_or_default();
        Some((patterns, stored, warning))
    } else {
        None
    };
    tauri::async_runtime::spawn_blocking(move || {
        if !worktree::is_git_repository(&wt.path) {
            return Ok(worktree::TaskDiffResult {
                files: Vec::new(),
                baseline: worktree::BaselineStatus::Legacy,
                review: review_inputs.map(|(_, stored, warning)| worktree::ReviewSnapshot {
                    hunks: Vec::new(),
                    annotations: annotations::rematch(&stored, &[]),
                    warning,
                }),
            });
        }
        let files = wt.diff_detailed_range(range).map_err(|e| e.to_string())?;
        let review = review_inputs
            .map(|(patterns, stored, warning)| {
                wt.review_snapshot(&files, range, &patterns, &stored)
                    .map(|mut review| {
                        review.warning = warning;
                        review
                    })
                    .map_err(|e| e.to_string())
            })
            .transpose()?;
        Ok(worktree::TaskDiffResult {
            files,
            baseline: wt.baseline_status(),
            review,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 구조화 hunk 목록(`diffmodel`) — B-1 주석·B-2 부분 승인·B-3 ensemble 조합의 공통 조회 API.
/// Goal Contract `protected_paths`/`risk::assess_blast` 주석까지 포함해 반환한다.
#[tauri::command]
pub async fn diff_hunks(
    state: State<'_, AppState>,
    id: i64,
    range: Option<worktree::DiffRange>,
) -> Result<Vec<diffmodel::DiffHunk>, String> {
    let pool = pool_of(&state)?;
    let task = db::get_task(&pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    task_hunks_range(&task, range.unwrap_or_default())
}

/// task의 현재 구조화 hunk 목록 — `diff_hunks`·annotations 재매칭/재전송이
/// 공유하는 조회 로직.
pub(crate) fn task_hunks(task: &Task) -> Result<Vec<diffmodel::DiffHunk>, String> {
    task_hunks_range(task, worktree::DiffRange::Session)
}

/// 범위를 골라 뜨는 hunk 목록. 화면이 보는 것과 주석이 재매칭되는 것이 어긋나지 않도록,
/// diff를 그리는 쪽과 주석을 붙이는 쪽이 같은 범위를 넘겨야 한다.
pub(crate) fn task_hunks_range(
    task: &Task,
    range: worktree::DiffRange,
) -> Result<Vec<diffmodel::DiffHunk>, String> {
    let worktree = worktree_from_task(task);
    if !worktree::is_git_repository(&worktree.path) {
        return Ok(Vec::new());
    }
    let diff_text = worktree
        .diff_unified_range(3, range)
        .map_err(|e| e.to_string())?;
    let patterns = task
        .goal_contract
        .as_deref()
        .map(|contract| contract.protected_paths.clone())
        .unwrap_or_default();
    let mut hunks = diffmodel::build_hunks(&diff_text, &patterns);
    mark_committed(&worktree, &mut hunks, &patterns, range)?;
    Ok(hunks)
}

/// 어떤 hunk가 이미 커밋됐는지 표시한다.
///
/// 두 diff의 new side가 **똑같이 working tree**라 `new_range` 좌표계를 공유한다. 미커밋
/// diff와 겹치지 않는 hunk가 곧 커밋된 것이다.
///
/// 미커밋 범위에서는 물어볼 필요가 없다 — 정의상 전부 미커밋이고, 자기 자신과 비교하는
/// 셈이라 diff를 한 번 더 뜨는 비용만 든다.
fn mark_committed(
    worktree: &Worktree,
    hunks: &mut [diffmodel::DiffHunk],
    patterns: &[String],
    range: worktree::DiffRange,
) -> Result<(), String> {
    if range == worktree::DiffRange::Uncommitted || hunks.is_empty() {
        return Ok(());
    }
    let pending = worktree
        .diff_unified_range(3, worktree::DiffRange::Uncommitted)
        .map_err(|e| e.to_string())?;
    let pending = diffmodel::build_hunks(&pending, patterns);
    for hunk in hunks.iter_mut() {
        hunk.committed = !pending.iter().any(|other| diffmodel::overlaps(hunk, other));
    }
    Ok(())
}

/// 부분 적용 결과 — 체크포인트 sha와 유지/폐기된 hunk id(확인 스텝 요약·롤백 버튼 활성화용).
#[derive(Debug, Clone, Serialize)]
pub struct PartialApplyResult {
    pub checkpoint: String,
    pub kept_hunk_ids: Vec<String>,
    pub discarded_hunk_ids: Vec<String>,
}

/// hunk 부분 승인(B-2): 선택 hunk만 worktree에 남긴다 — 체크포인트 커밋 → 비선택 hunk
/// 역패치 적용(파일 단위 fail-closed, `partial::apply`) → 성공 시 체크포인트를 영속해 롤백
/// 가능 상태로 만든다. 성공/실패 모두 `partial_apply` 이벤트로 남겨 적용 성공률(KPI Tech)을 관측한다.
#[tauri::command]
pub async fn partial_apply(
    state: State<'_, AppState>,
    id: i64,
    hunk_ids: Vec<String>,
) -> Result<PartialApplyResult, String> {
    let pool = pool_of(&state)?;
    let task = db::get_task(&pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    if task.state != tstate::AWAITING_REVIEW {
        return Err("검토 대기 중인 작업만 부분 적용할 수 있습니다".into());
    }
    let _review_claim = state.review_claims.claim_finalization(id)?;
    crate::task_review::guard_apply(&pool, id).await?;
    let hunks = task_hunks(&task)?;
    let worktree = worktree_from_task(&task);
    let result = crate::partial::apply(&worktree, &hunks, &hunk_ids);
    let _ = db::append_event(
        &pool,
        id,
        "partial_apply",
        Some(&partial_apply_kpi_detail(&result)),
        now(),
    )
    .await;
    let outcome = result.map_err(|e| e.to_string())?;

    crate::partial::save_checkpoint(&pool, id, &outcome.checkpoint, now())
        .await
        .map_err(|e| e.to_string())?;
    Ok(PartialApplyResult {
        checkpoint: outcome.checkpoint,
        kept_hunk_ids: outcome.kept_hunk_ids,
        discarded_hunk_ids: outcome.discarded_hunk_ids,
    })
}

/// KPI Tech(적용 성공률) 관측용 이벤트 상세 — 순수 함수로 분리해 커밋/롤백 없이 문자열만 검증 가능.
fn partial_apply_kpi_detail(
    result: &Result<crate::partial::ApplyOutcome, crate::partial::PartialError>,
) -> String {
    match result {
        Ok(o) => format!(
            "ok kept={} discarded={}",
            o.kept_hunk_ids.len(),
            o.discarded_hunk_ids.len()
        ),
        Err(crate::partial::PartialError::ApplyConflict(ids)) => {
            format!("conflict failed={}", ids.len())
        }
        Err(crate::partial::PartialError::ProtectedHunkRejected(ids)) => {
            format!("protected_rejected count={}", ids.len())
        }
        Err(e) => format!("error {e}"),
    }
}

/// hunk 부분 승인 롤백: 저장된 체크포인트로 worktree를 완전히 원복하고 체크포인트를 정리한다.
/// `partial_apply` 실패(verify 실패 포함, 프론트가 검증 후 호출) 시 원클릭 복구 경로.
#[tauri::command]
pub async fn partial_rollback(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let pool = pool_of(&state)?;
    let task = db::get_task(&pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    let checkpoint = crate::partial::get_checkpoint(&pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("되돌릴 부분 적용 체크포인트가 없습니다")?;
    let worktree = worktree_from_task(&task);
    crate::partial::rollback(&worktree, &checkpoint).map_err(|e| e.to_string())?;
    crate::partial::clear_checkpoint(&pool, id)
        .await
        .map_err(|e| e.to_string())?;
    let _ = db::append_event(&pool, id, "partial_rollback", None, now()).await;
    Ok(())
}

/// 특정 작업의 주석 목록 — 저장된 주석을 현재 diff에 재매칭(hunk_id 우선 → path+line 근사
/// → orphaned)해 반환한다. Diff 뷰 거터·스레드·고아 배지가 이 결과를 그대로 사용.
#[tauri::command]
pub async fn annotations_list(
    state: State<'_, AppState>,
    task_id: i64,
    range: Option<worktree::DiffRange>,
) -> Result<Vec<annotations::RematchedAnnotation>, String> {
    let pool = pool_of(&state)?;
    let task = db::get_task(&pool, task_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    let stored = annotations::list_by_task(&pool, task_id)
        .await
        .map_err(|e| e.to_string())?;
    if stored.is_empty() { return Ok(Vec::new()); }
    // 화면이 보고 있는 것과 같은 범위로 재매칭해야 hunk_id가 맞는다. 범위가 어긋나면
    // 라인 범위가 달라 id가 전부 갈리고, 붙여둔 주석이 한꺼번에 고아로 떨어진다.
    let hunks = task_hunks_range(&task, range.unwrap_or_default())?;
    Ok(annotations::rematch(&stored, &hunks))
}

/// draft 주석 생성/저장(onBlur 자동 저장) — `id`가 있으면 본문을 갱신(draft 상태만), 없으면 새로 만든다.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn annotation_save(
    state: State<'_, AppState>,
    task_id: i64,
    id: Option<String>,
    hunk_id: String,
    path: String,
    line: i64,
    side: String,
    body_md: String,
) -> Result<annotations::ReviewAnnotation, String> {
    let pool = pool_of(&state)?;
    if let Some(existing_id) = id {
        annotations::update_draft_body(&pool, &existing_id, &body_md)
            .await
            .map_err(|e| e.to_string())?;
        let updated = annotations::list_by_ids(&pool, task_id, std::slice::from_ref(&existing_id))
            .await
            .map_err(|e| e.to_string())?;
        return updated
            .into_iter()
            .next()
            .ok_or_else(|| "주석을 찾을 수 없습니다".to_string());
    }
    annotations::create_draft(
        &pool,
        task_id,
        &hunk_id,
        &path,
        line,
        &side,
        &body_md,
        now(),
    )
    .await
    .map_err(|e| e.to_string())
}

/// 추측 어려운 nonce용 난수 (OS 시드 기반 RandomState, 무의존). PID+타임스탬프보다 스푸핑 저항.
fn rand_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    // RandomState는 인스턴스마다 OS 난수로 시드된다.
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0),
    );
    h.finish()
}

#[derive(Clone, serde::Serialize)]
struct ConvoPayload {
    id: i64,
    /// 발화한 면 — 토론이 아니면 키가 아예 빠진다(= 미상). `event`가 flatten이므로 루트의
    /// 형제 자리다. 적재 쪽 짝은 `convo::stored_event_json`이다(설계 §4-4).
    #[serde(skip_serializing_if = "Option::is_none")]
    speaker: Option<crate::convo::Side>,
    #[serde(flatten)]
    event: crate::convo::ConvoEvent,
}

/// 실전송문이 원문과 달라졌을 때(슬래시 스킬 확장·Goal Contract 합성) 원문 이벤트 직후 기록할
/// `user_expanded` 이벤트(JSON) — 달라지지 않았으면 None.
/// 순수 함수로 분리(부작용은 호출부의 `db::append_convo_event`) — 스키마 변경 없이 kind로 구분(설계 0008 §D).
fn expansion_event(expanded: Option<&str>) -> Option<String> {
    expanded.map(|exp| serde_json::json!({ "kind": "user_expanded", "text": exp }).to_string())
}

/// convo 턴 유휴(무출력) 상한 — 긴 도구/원격 작업의 출력 공백을 허용한다.
/// 총 실행 시간은 무제한(유휴 워치독) — stdout 라인이 오면 데드라인이 리셋된다.
const CONVO_IDLE_TIMEOUT_SECS: u64 = 43_200;

/// 대화 한 턴 시작 — 초기 지시(task_create)와 후속(convo_send) 공용.
/// in-flight 가드(원자적 insert) → user 메시지·이벤트를 convo_events에 적재,
/// session_id를 DB에 영속(재시작 후 --resume), 이벤트를 `convo://event`로 스트리밍.
/// 벤더는 task.agent로 선택(claude/codex/agy — gemini는 agy 라우팅).
async fn persist_convo_admission(
    pool: &SqlitePool,
    task: &Task,
    input_origin: ConversationInputOrigin,
    user_event: &str,
    expanded_event: Option<&str>,
    ts: i64,
    purpose: Option<&crate::project_plan::PreparedPurpose>,
) -> Result<bool, String> {
    let mut tx = pool.begin().await.map_err(|error| error.to_string())?;
    if let Some(candidate) = purpose {
        crate::project_plan::persist_purpose_tx(&mut tx, candidate).await.map_err(|e|e.to_string())?;
    }
    if !input_origin.is_initial() {
        sqlx::query("INSERT INTO task_events (task_id, ts, kind, detail) VALUES (?, ?, 'user_followup_input_observed', NULL) ON CONFLICT(task_id, kind) WHERE kind IN ('followup_observation_started', 'user_followup_input_observed') DO NOTHING")
            .bind(task.id).bind(ts).execute(&mut *tx).await.map_err(|error| error.to_string())?;
    }
    let started = if task.state == tstate::AWAITING_REVIEW {
        let updated = sqlx::query("UPDATE tasks SET state = ?, updated_at = ?, awaiting_kind = NULL WHERE id = ? AND state = ? AND NOT EXISTS (SELECT 1 FROM review_process_leases lease WHERE lease.task_id = tasks.id)")
            .bind(tstate::RUNNING).bind(ts).bind(task.id).bind(tstate::AWAITING_REVIEW)
            .execute(&mut *tx).await.map_err(|error| error.to_string())?;
        if updated.rows_affected() == 0 {
            return Err("작업이 이미 완료 처리 중이거나 종료되었습니다".into());
        }
        crate::notifications::clear_cancel_tx(&mut tx, task.id)
            .await
            .map_err(|error| error.to_string())?;
        true
    } else {
        false
    };
    sqlx::query("INSERT INTO convo_events (task_id, ts, event) VALUES (?, ?, ?)")
        .bind(task.id)
        .bind(ts)
        .bind(user_event)
        .execute(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
    if let Some(event) = expanded_event {
        sqlx::query("INSERT INTO convo_events (task_id, ts, event) VALUES (?, ?, ?)")
            .bind(task.id)
            .bind(ts)
            .bind(event)
            .execute(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;
    }
    tx.commit().await.map_err(|error| error.to_string())?;
    Ok(started)
}

async fn admit_convo_turn(
    pool: &SqlitePool,
    reservation: ConvoReservation,
    task: &Task,
    input_origin: ConversationInputOrigin,
    user_event: &str,
    expanded_event: Option<&str>,
    receipt_reservation: Option<crate::preview_workbench::ReceiptReservation>,
    action: ConvoAdmissionAction,
    ts: i64,
    purpose: Option<&crate::project_plan::PreparedPurpose>,
) -> Result<(ConvoReservation, bool), String> {
    persist_convo_admission(pool, task, input_origin, user_event, expanded_event, ts, purpose).await?;
    if let Some(receipt) = receipt_reservation {
        receipt.commit();
    }
    Ok((
        reservation,
        action == ConvoAdmissionAction::ReleaseManualTakeover,
    ))
}

fn reserve_preview_receipt(
    receipt: Option<&PreviewReceiptAcceptance>,
    task_id: i64,
    ts: i64,
) -> Result<Option<crate::preview_workbench::ReceiptReservation>, String> {
    receipt
        .map(|receipt| {
            receipt
                .workbench
                .reserve(task_id, &receipt.request_id, &receipt.context, ts)
        })
        .transpose()
}

/// 라운드 상한 설정 키. 값은 문자열 정수이고 범위는 `debate::ROUND_CAP_RANGE`다.
const DEBATE_ROUND_CAP_KEY: &str = "debate_round_cap";

/// 좌측이 아닌 자리 하나의 상태. 원천은 `convo_debate_sides` 행이다.
struct DebateSeat {
    side: crate::convo::Side,
    agent: String,
    model: Option<String>,
    session: Option<String>,
}

/// 라운드 루프가 도는 동안만 사는 토론 상태. 좌측은 `tasks` 행이 원천이므로 자리 목록에 없다.
struct DebateRuntime {
    state: crate::convo::debate::DebateState,
    /// 비좌측 자리들, 자리 순서.
    seats: Vec<DebateSeat>,
    /// 이번 라운드 시퀀스를 연 사용자 발화. 모두에게 같은 문자열로 간다.
    user_message: String,
    /// 자리별(`Side::index()`) **가장 최근** 발화 하나. 전체 로그를 중계하면 라운드마다
    /// 컨텍스트가 선형으로 분다(설계 2026-09-23 D2).
    last: [Option<String>; 3],
}

impl DebateRuntime {
    fn new(cap: u32, rows: Vec<db::DebateSideRow>, user_message: String) -> Self {
        let seats = rows
            .into_iter()
            .filter_map(|row| {
                // `db::debate_sides`가 이미 검증했다 — 여기서 떨어지는 행은 없다.
                crate::convo::Side::parse(&row.side).map(|side| DebateSeat {
                    side,
                    agent: row.agent,
                    model: row.model,
                    session: row.vendor_session_id,
                })
            })
            .collect::<Vec<_>>();
        let order = std::iter::once(crate::convo::Side::Left)
            .chain(seats.iter().map(|seat| seat.side))
            .collect();
        Self {
            state: crate::convo::debate::DebateState::with_seats(cap, order),
            seats,
            user_message,
            last: Default::default(),
        }
    }

    fn seat(&self, side: crate::convo::Side) -> Option<&DebateSeat> {
        self.seats.iter().find(|seat| seat.side == side)
    }

    fn seat_mut(&mut self, side: crate::convo::Side) -> Option<&mut DebateSeat> {
        self.seats.iter_mut().find(|seat| seat.side == side)
    }

    fn round_context<'a>(
        &'a self,
        side: crate::convo::Side,
        left_agent: &'a str,
    ) -> crate::convo::debate::RoundContext<'a> {
        let peers = self
            .state
            .seats()
            .iter()
            .filter(|peer| **peer != side)
            .map(|peer| crate::convo::debate::Peer {
                agent: match peer {
                    crate::convo::Side::Left => left_agent,
                    crate::convo::Side::Right | crate::convo::Side::Third => {
                        self.seat(*peer).map(|seat| seat.agent.as_str()).unwrap_or_default()
                    }
                },
                last: self.last[peer.index()].as_deref(),
            })
            .collect();
        crate::convo::debate::RoundContext {
            round: self.state.round(),
            cap: self.state.cap(),
            user_message: &self.user_message,
            peers,
        }
    }
}

/// 이번 턴이 확립한 벤더 세션 id를 그 자리의 원천에 적는다. 비좌측은 `set_convo_session`을
/// 지나지 않는다 — 그 함수가 좌측이 받아야 할 `pending_capsule`을 함께 지운다(ADR 0170).
/// 와일드카드를 쓰지 않는다: 셋째가 좌측 경로로 떨어지면 좌측 세션을 덮는다(설계 2026-09-23 D4).
async fn persist_turn_session(
    pool: &SqlitePool,
    id: i64,
    side: Option<crate::convo::Side>,
    session_id: &str,
) -> anyhow::Result<()> {
    match side {
        None | Some(crate::convo::Side::Left) => db::set_convo_session(pool, id, session_id).await,
        // 빈 id는 적지 않는다 — resume 인자가 빈 문자열이 되면 그 자리의 다음 턴이 인계 조립 대신
        // `--resume ""`으로 죽는다. 좌측은 오늘의 계약을 그대로 둔다.
        Some(crate::convo::Side::Right | crate::convo::Side::Third) if session_id.is_empty() => Ok(()),
        Some(side @ (crate::convo::Side::Right | crate::convo::Side::Third)) => {
            db::set_debate_side_session(pool, id, side, session_id).await
        }
    }
}

fn convo_interrupted(active: &ActiveConvos, id: i64) -> bool {
    active
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .map(|turn| turn.interrupted)
        .unwrap_or(false)
}

/// 설정의 라운드 상한. 미설정·파싱 실패는 기본값이다.
async fn debate_round_cap(pool: &SqlitePool) -> u32 {
    let stored = db::get_setting(pool, DEBATE_ROUND_CAP_KEY)
        .await
        .ok()
        .flatten();
    crate::convo::debate::round_cap_from_setting(stored.as_deref())
}

/// 범위 밖은 **거부한다** — 클램프하면 사용자가 무엇이 저장됐는지 알 수 없다.
pub(crate) async fn set_debate_round_cap(pool: &SqlitePool, cap: u32) -> Result<(), String> {
    let range = crate::convo::debate::ROUND_CAP_RANGE;
    if !range.contains(&cap) {
        return Err(format!(
            "라운드 상한은 {}~{} 사이여야 합니다: {cap}",
            range.start(),
            range.end()
        ));
    }
    db::set_setting(pool, DEBATE_ROUND_CAP_KEY, &cap.to_string())
        .await
        .map_err(|error| error.to_string())
}

/// 토론 라운드 상한 조회.
#[tauri::command]
pub async fn debate_round_cap_get(state: State<'_, AppState>) -> Result<u32, String> {
    let pool = pool_of(&state)?;
    Ok(debate_round_cap(&pool).await)
}

/// 토론 라운드 상한 설정.
#[tauri::command]
pub async fn debate_round_cap_set(state: State<'_, AppState>, cap: u32) -> Result<(), String> {
    let pool = pool_of(&state)?;
    set_debate_round_cap(&pool, cap).await
}

/// 토론 상대 한 명 — 프론트가 넘기는 형태.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DebateOpponent {
    pub agent: String,
    #[serde(default)]
    pub model: Option<String>,
}

/// 비좌측 자리 순서. 상대는 이 순서로 자리를 받는다.
const DEBATE_OPPONENT_SEATS: [crate::convo::Side; 2] =
    [crate::convo::Side::Right, crate::convo::Side::Third];

pub(crate) async fn debate_start_checked(
    pool: &SqlitePool,
    active: ActiveConvos,
    id: i64,
    opponents: &[DebateOpponent],
) -> Result<(), String> {
    if opponents.is_empty() || opponents.len() > DEBATE_OPPONENT_SEATS.len() {
        return Err(format!(
            "토론 상대는 1~{}명이어야 합니다",
            DEBATE_OPPONENT_SEATS.len()
        ));
    }
    let mut agents: Vec<&str> = Vec::with_capacity(opponents.len());
    for opponent in opponents {
        let agent = opponent.agent.trim();
        if !supports_convo_agent_switch(agent) {
            return Err(format!("토론 상대로 쓸 수 없는 에이전트입니다: {agent}"));
        }
        if agents.contains(&agent) {
            return Err(format!("같은 에이전트를 두 번 상대로 고를 수 없습니다: {agent}"));
        }
        agents.push(agent);
    }
    let _reservation = reserve_convo_switch(active, id)?;
    crate::convo::interaction::runtime_of(pool, id).await?;
    crate::convo::interaction::ensure_idle(pool, id).await?;

    let task = db::get_task(pool, id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
    if task.mode != "conversation" || task.state != tstate::AWAITING_REVIEW {
        return Err("검토 대기 중인 대화 작업만 토론을 시작할 수 있습니다".into());
    }
    let current = task.agent.as_deref().unwrap_or_default().trim();
    if agents.contains(&current) {
        return Err("토론 상대는 현재 에이전트와 달라야 합니다".into());
    }
    if !db::debate_sides(pool, id)
        .await
        .map_err(|error| error.to_string())?
        .is_empty()
    {
        return Err("이미 토론 중입니다 — 먼저 토론을 끝내세요".into());
    }
    let rows = DEBATE_OPPONENT_SEATS
        .iter()
        .zip(opponents.iter().zip(agents))
        .map(|(side, (opponent, agent))| {
            let model = opponent.model.as_deref().map(str::trim).filter(|model| !model.is_empty());
            (*side, agent, model)
        })
        .collect::<Vec<_>>();
    db::insert_debate_sides(pool, id, &rows)
        .await
        .map_err(|error| error.to_string())
}

/// 프론트가 "토론 중"을 판정하는 유일한 신호 — 사이드 행이 있으면 그 작업은 토론 중이다.
#[derive(Debug, Serialize)]
pub struct DebateSideView {
    pub side: &'static str,
    pub agent: String,
    pub model: Option<String>,
}

/// 비좌측 자리 조회, 자리 순서. 빈 배열이면 토론이 아니다. 앱 재시작 뒤에도 저장된 행에서 읽는다.
#[tauri::command]
pub async fn debate_sides(
    state: State<'_, AppState>,
    task_id: i64,
) -> Result<Vec<DebateSideView>, String> {
    let pool = pool_of(&state)?;
    let rows = db::debate_sides(&pool, task_id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            crate::convo::Side::parse(&row.side).map(|side| DebateSideView {
                side: side.as_str(),
                agent: row.agent,
                model: row.model,
            })
        })
        .collect())
}

/// 토론을 연다 — 상대마다 사이드 행 하나를 만드는 것이 전부다. 벤더 세션은 그 자리의 첫 턴이 판다.
#[tauri::command]
pub async fn debate_start(
    state: State<'_, AppState>,
    task_id: i64,
    opponents: Vec<DebateOpponent>,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    debate_start_checked(&pool, state.convo_active.clone(), task_id, &opponents).await
}

/// 사이드 행 전부를 지우고 종료 경계를 원장에 남긴다. 사용자가 손으로 끝냈으므로 사유는 중단이다.
/// 적재 실패는 삼키지 않는다 — 경계가 없으면 재진입 때 토론이 끝난 적 없는 것처럼 보인다
/// (`ContextCleared` 적재와 같은 계약).
pub(crate) async fn debate_end_checked(
    pool: &SqlitePool,
    active: ActiveConvos,
    id: i64,
) -> Result<crate::convo::ConvoEvent, String> {
    // 시작과 같은 점유를 잡는다 — 라운드 시퀀스가 도는 중에 행을 지우면 진행 중인 턴이
    // 근거 없는 사이드 세션으로 계속 돌고 종료 경계가 두 번 남는다.
    let _reservation = reserve_convo_switch(active, id)?;
    crate::convo::interaction::ensure_idle(pool, id).await?;
    db::delete_debate_sides(pool, id)
        .await
        .map_err(|error| error.to_string())?;
    let event = crate::convo::ConvoEvent::DebateEnded {
        reason: crate::convo::DebateEndReason::Aborted,
    };
    let encoded =
        crate::convo::stored_event_json(&event, None).map_err(|error| error.to_string())?;
    db::append_convo_event(pool, id, &encoded, now())
        .await
        .map_err(|error| error.to_string())?;
    Ok(event)
}

/// 토론을 끝낸다. 좌측 세션은 그대로이므로 다음 턴부터 단일 대화로 돌아간다.
#[tauri::command]
pub async fn debate_end(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: i64,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    let event = debate_end_checked(&pool, state.convo_active.clone(), task_id).await?;
    let _ = app.emit(
        "convo://event",
        ConvoPayload {
            id: task_id,
            speaker: None,
            event,
        },
    );
    Ok(())
}

/// 한 발화의 질문 실행. 토론의 점유는 유지하되 질문 실행은 매 차례 새로 만든다.
/// 이전 프로세스와 질문이 정리되지 않았으면 컨트롤을 교체하지 않는다.
async fn next_question_turn(
    pool: &SqlitePool,
    id: i64,
    agent: &str,
    side: Option<crate::convo::Side>,
    generation: u64,
    changed: Arc<dyn Fn() + Send + Sync>,
) -> Result<Option<crate::convo::app_server::Context>, String> {
    use crate::convo::{app_server, interaction, interaction_commands};
    interaction_commands::ensure_generation(generation)?;
    interaction::ensure_idle(pool, id).await?;
    if app_server::cleanup_failed(id) {
        return Err("이전 질문 실행의 정리를 확인하세요".into());
    }
    app_server::unregister(id);
    let Some(runtime) = interaction::runtime_for_agent(agent) else { return Ok(None) };
    let execution = interaction::begin(pool, id, now()).await?;
    let control = interaction_commands::ensure_generation(generation).and_then(|()| {
        app_server::register_turn(id, execution.clone(), runtime == interaction::RUNTIME_LOCAL, side)
    });
    let control = match control {
        Ok(control) => control,
        Err(error) => {
            interaction::finish(pool, &execution, "failed", Some(&error)).await?;
            return Err(error);
        }
    };
    Ok(Some(app_server::Context { pool: pool.clone(), task_id: id, control, changed }))
}

/// CLI 누락이나 중단으로 spawn 전에 끝난 실행도 정리한다.
async fn finish_unstarted_question(ctx: &crate::convo::app_server::Context) -> Result<(), String> {
    let starting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM convo_executions WHERE id=? AND state='starting')")
        .bind(&ctx.control.execution).fetch_one(&ctx.pool).await.map_err(|error| error.to_string())?;
    if starting {
        let state = if ctx.control.cancelled.load(Ordering::SeqCst) { "cancelled" } else { "failed" };
        crate::convo::interaction::finish(&ctx.pool, &ctx.control.execution, state, Some("turn_not_started")).await?;
        ctx.changed();
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn start_convo_turn(
    app: AppHandle,
    pool: SqlitePool,
    active: ActiveConvos,
    id: i64,
    repo: String,
    cwd: String,
    message: String,
    image_paths: Vec<String>,
    receipt_request_id: Option<&str>,
    input_origin: ConversationInputOrigin,
    preview_client_ref: Option<String>,
    // 자동 업데이트 플래그. 이 함수는 `AppState`를 통째로 받지 않으므로 필요한 것만 넘긴다.
    updating: Arc<AtomicBool>,
    preview_receipt: Option<PreviewReceiptAcceptance>,
    admission_action: ConvoAdmissionAction,
) -> Result<(), String> {
    let admission_generation=crate::convo::interaction_commands::generation();
    // 대화 턴의 초크포인트. `convo_send`뿐 아니라 주석 재전송·원격 리뷰 재시도도 여기를 지난다.
    let bound_runtime = crate::convo::interaction::runtime_of(&pool, id).await?;
    if bound_runtime.as_deref() != Some(crate::convo::interaction::RUNTIME) {
        refuse_if_updating(&updating)?;
    }
    let reservation = side_question_slots::reserve_turn(
        &app.state::<AppState>(), active.clone(), id, input_origin.is_initial(),
        bound_runtime.as_deref() != Some(crate::convo::interaction::RUNTIME),
    ).await?;
    if crate::side_question::blocks_main_execution(&pool, id).await? {
        return Err("별도 질의의 프로세스 소유권 확인이 필요합니다".into());
    }
    let receipt_reservation = reserve_preview_receipt(preview_receipt.as_ref(), id, now())?;
    let task = match db::get_task(&pool, id).await {
        Ok(Some(task)) => task,
        Ok(None) => return Err("작업을 찾을 수 없습니다".into()),
        Err(error) => return Err(error.to_string()),
    };
    if task.state != tstate::AWAITING_REVIEW && task.state != tstate::RUNNING {
        return Err("검토 대기 또는 실행 중인 대화 작업만 계속할 수 있습니다".into());
    }
    let structured = bound_runtime.is_some();
    if structured && crate::convo::interaction_commands::shutting_down() {
        return Err("앱 종료를 위해 질문 세션을 정리하고 있습니다".into());
    }
    // 공급자 전환은 작업과 바인딩을 함께 바꾼다. 불일치는 손상된 계약이다.
    if structured
        && (task
            .agent
            .as_deref()
            .and_then(crate::convo::interaction::runtime_for_agent)
            != bound_runtime.as_deref())
    {
        return Err("질문 실행 계약 또는 정리 상태를 확인하세요".into());
    }
    if structured {
        crate::convo::interaction::ensure_idle(&pool, id).await?;
    }
    let debate_rows = db::debate_sides(&pool, id).await.map_err(|error| error.to_string())?;
    // 상대가 전역 CLI를 쓰면 업데이트와 경합할 수 있다. Codex 좌측만 보고 우회하지 않는다.
    if !debate_rows.is_empty() {
        refuse_if_updating(&updating)?;
    }
    // Claude가 좌측이어도 Codex 상대의 고정 런타임을 입력 접수 전에 준비한다.
    let needs_codex = structured && (task.agent.as_deref() == Some("codex")
        || debate_rows.iter().any(|row| row.agent == "codex"));
    let mut codex_runtime_version = None;
    let managed_codex_bin = if needs_codex {
        let version = crate::convo::interaction::ensure_codex_version(&pool, id).await?;
        codex_runtime_version = Some(version.clone());
        Some(ensure_managed_codex(&app, version).await?)
    } else {
        None
    };
    let preview_client_ref = preview_client_ref.unwrap_or_else(|| format!("vault-followup:{id}"));
    let attempt = if cfg!(target_os = "macos") {
        match begin_vault_attempt(&pool, id, &repo, &message, Some(&preview_client_ref)).await {
            Ok(attempt) => attempt,
            Err(error) => return Err(error.to_string()),
        }
    } else {
        None
    };
    if let Some(attempt) = attempt.as_deref() {
        if let Err(error) =
            crate::knowledge::vault::provenance::begin_conversation_provenance(&pool, attempt).await
        {
            return Err(error.to_string());
        }
    }
    if !image_paths.is_empty() {
        if let Some(attempt) = attempt.as_deref() {
            if let Err(error) =
                crate::knowledge::vault::provenance::record_unknown_input_for_attempt(
                    &pool,
                    attempt,
                    crate::knowledge::vault::provenance::InputOrigin::DocumentRevision,
                    "image attachments have no tracked analysis-purpose consent",
                    now(),
                )
                .await
            {
                return Err(error.to_string());
            }
        }
    }
    // 이어받은 작업의 **첫** 턴은 이 작업 기준으로는 초기 턴이지만, 벤더 세션에는 앞 대화가
    // 통째로 들어 있다. `is_initial`만 보면 이어받기로 들어온 이전 대화가 provenance에 한 번도
    // 안 적힌다 — 이어받기는 정확히 "이전 대화가 입력으로 들어오는" 경우다.
    if !input_origin.is_initial() || task.resumed_from.is_some() {
        if let Some(attempt) = attempt.as_deref() {
            if let Err(error) =
                crate::knowledge::vault::provenance::record_unknown_input_for_attempt(
                    &pool,
                    attempt,
                    crate::knowledge::vault::provenance::InputOrigin::PriorConversation,
                    "resumed conversation includes prior context",
                    now(),
                )
                .await
            {
                return Err(error.to_string());
            }
        }
    }
    let delivery = match vault_delivery(
        &pool,
        id,
        &repo,
        &message,
        Some(&preview_client_ref),
        attempt.as_ref(),
    )
    .await
    {
        Ok(delivery) => delivery,
        Err(error) => return Err(error.to_string()),
    };
    // guard를 잡은 뒤의 행이 유일한 실행 근거다. 전송 직전에 에이전트 전환이 끝나도
    // 호출부에서 읽은 옛 vendor/session을 스레드로 넘기면 다른 CLI가 세션을 resume한다.
    let agent = task.agent.clone().unwrap_or_default();
    let resume = task.convo_session_id.clone();
    // 슬래시 스킬 발동 — 대상이 스스로 해석하면 원문을 그대로 보내고(claude), 못 하면
    // 그 스킬이 사는 디렉터리에서 읽어 확장한다(codex·agy). 사본은 만들지 않는다.
    // 실전송문(user_expanded) 기록은 Goal Contract 합성까지 끝난 뒤 한 번만 한다 —
    // 매퍼는 user 직후의 user_expanded 하나만 흡수하므로 단계별로 나눠 적으면 뒤 것이 유실된다.
    let vendor = crate::convo::Vendor::from_agent(task.agent.as_deref().unwrap_or_default());
    let mut prompt_metrics = crate::convo::prompt_metrics::PromptMetrics::new(
        vendor, structured, resume.is_some(), &message,
    );
    if vendor == crate::convo::Vendor::Codex && structured {
        prompt_metrics.set_instructions(&crate::convo::app_server::instructions(
            codex_runtime_version.as_deref() == Some(crate::convo::cli_runtime::DEFAULT_VERSION),
        ));
    }
    let sent =
        crate::skills::resolve_message(&repo, vendor, &message).unwrap_or_else(|| message.clone());
    prompt_metrics.skill_added_bytes = crate::convo::prompt_metrics::added_bytes(&message, &sent);
    let execution_round_id = crate::preview_bridge::random_hex_id()?;
    let purpose = crate::project_plan::prepare_purpose(&pool, id, &execution_round_id, now()).await.map_err(|e|e.to_string())?;
    let sent = purpose.as_ref().map(|p| crate::project_plan::render_purpose(p, &sent)).unwrap_or(sent);
    let before_goal_bytes = sent.len();

    // Goal Contract는 첫 턴에만 합성한다. 후속 resume 턴은 사용자의 새 메시지만 전달한다.
    let sent = if input_origin.is_initial() {
        crate::goal_contract::execution_prompt(&sent, task.goal_contract.as_deref())
    } else {
        sent
    };
    prompt_metrics.goal_added_bytes = sent.len().saturating_sub(before_goal_bytes) as u64;
    // 토론 중인가 — 사이드 행의 **존재**가 유일한 판정이다(설계 §4-1). 조회 실패를 삼키고
    // 단일 턴으로 내려가면 상대가 빠진 라운드가 조용히 남는다. 상대에게 중계할 사용자 발화는
    // **확장이 끝난 뒤**의 문장이다 — 원문을 쥐면 슬래시 스킬·Goal Contract가 풀리기 전 요청을
    // 두고 토론하게 된다. 캡슐은 좌측 세션의 맥락이므로 붙기 전에 찍는다.
    let debate = if debate_rows.is_empty() { None } else {
        Some(DebateRuntime::new(debate_round_cap(&pool).await, debate_rows, sent.clone()))
    };
    // 절단이 남긴 캡슐을 **맨 앞에** 붙인다(ADR 0170). 캡슐은 맥락이고 나머지는 이번 턴의
    // 요청이므로 순서가 이렇다. guard 뒤에 읽은 task 행을 쓴다 — 별도 조회 실패를 무시한 채
    // 새 세션을 만들면 `set_convo_session`이 아직 전달하지 못한 핸드오프를 지워 버린다.
    // 지우는 것은 새 세션이 확립될 때(`set_convo_session`)뿐이다.
    let before_capsule_bytes = sent.len();
    let sent = match task.pending_capsule.as_deref() {
        Some(capsule) => format!("{capsule}\n{sent}"),
        None => sent,
    };
    prompt_metrics.capsule_added_bytes = sent.len().saturating_sub(before_capsule_bytes) as u64;
    if sent != message {
        if let Some(attempt) = attempt.as_deref() {
            if let Err(error) =
                crate::knowledge::vault::provenance::record_unknown_input_for_attempt(
                    &pool,
                    attempt,
                    crate::knowledge::vault::provenance::InputOrigin::SkillExpanded,
                    &sent,
                    now(),
                )
                .await
            {
                return Err(error.to_string());
            }
        }
    }

    let delivered_message = match delivery.as_ref() {
        Some(item) => crate::knowledge::vault::retrieval::delivery_payload(&sent, &item.preview),
        None => sent.clone(),
    };
    prompt_metrics.vault_added_bytes = crate::convo::prompt_metrics::added_bytes(&sent, &delivered_message);
    prompt_metrics.finish(&delivered_message);
    let mut user_event = serde_json::json!({ "kind": "user", "text": &message, "prompt_metrics": if debate.is_none() { Some(prompt_metrics) } else { None } });
    if let Some(request_id) = receipt_request_id {
        user_event["receipt_request_id"] = serde_json::Value::String(request_id.to_string());
    }
    user_event["execution_round_id"] = serde_json::json!(execution_round_id);
    if let Some(candidate) = purpose.as_ref() { user_event["purpose_snapshot"] = serde_json::to_value(candidate.snapshot()).map_err(|e|e.to_string())?; }
    let user_ev = user_event.to_string();
    let expanded_ev = expansion_event((sent != message).then_some(sent.as_str()));
    let (mut reservation, release_manual) = admit_convo_turn(
        &pool,
        reservation,
        &task,
        input_origin,
        &user_ev,
        expanded_ev.as_deref(),
        receipt_reservation,
        admission_action,
        now(),
        purpose.as_ref(),
    )
    .await?;
    if task.state == tstate::AWAITING_REVIEW {
        let _ = app.emit(
            "task://state",
            StatePayload {
                id,
                state: tstate::RUNNING.to_string(),
                awaiting_kind: None,
            },
        );
    }
    if release_manual {
        let state = app.state::<AppState>();
        state.preview_bridge.release(id);
        crate::preview_control::emit_control_state(&app, &state, id, "release", true);
    }
    let message = delivered_message;

    let notify = app.clone();
    let question_changed: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        let _ = notify.emit("convo-interaction://changed", serde_json::json!({"taskId":id}));
    });
    let mut structured_ctx = if structured {
        let setup = next_question_turn(&pool, id, &agent, debate.as_ref().map(|_| crate::convo::Side::Left),
            admission_generation, question_changed.clone()).await;
        match setup {
            Ok(ctx) => ctx,
            Err(error) => {
                db::mark_awaiting_review_with_notification(&pool, id, now(), None, "failure")
                    .await
                    .map_err(|e| e.to_string())?;
                return Err(error);
            }
        }
    } else {
        None
    };

    if let Some(ctx)=structured_ctx.as_ref() {
        if let Err(error)=crate::convo::interaction_commands::ensure_generation(admission_generation) {
            // On a storage error the reservation Drop keeps ownership while a control exists.
            crate::convo::interaction::finish(&pool,&ctx.control.execution,"cancelled",Some(&error)).await?;
            db::mark_awaiting_review_with_notification(&pool,id,now(),None,"failure").await.map_err(|e|e.to_string())?;
            crate::convo::app_server::unregister(id);
            return Err(error);
        }
    }

    // Freeze all session settings at admission; later edits belong to the next message.
    let settings_task = task;
    reservation.handoff_to_turn();

    std::thread::spawn(move || {
        // 턴 종료 시(패닉 포함) 진행중 엔트리 제거 — busy 영구 고착/유령 pid 방지.
        struct Done(
            ActiveConvos,
            i64,
            Option<PreviewReceiptAcceptance>,
            AppHandle,
        );
        impl Drop for Done {
            fn drop(&mut self) {
                if !release_convo_if_finalized(&self.0, self.1) {
                    if let Ok(pool) = pool_of(&self.3.state::<AppState>()) {
                        let _ = tauri::async_runtime::block_on(async {
                            if let Some(execution) = crate::convo::app_server::execution(self.1) {
                                crate::convo::interaction::phase(
                                    &pool,
                                    &execution,
                                    "cleanup_failed",
                                )
                                .await?;
                            }
                            Ok::<_, String>(())
                        });
                    }
                    let _ = self.3.emit(
                        "convo-interaction://changed",
                        serde_json::json!({"taskId":self.1}),
                    );
                    return;
                }
                if let Some(receipt) = &self.2 {
                    receipt.workbench.finish(
                        self.1,
                        &receipt.request_id,
                        format!("convo:{}:{}", self.1, now()),
                    );
                }
                let _ = self.3.emit(
                    "preview-workbench://idle",
                    serde_json::json!({ "taskId": self.1 }),
                );
            }
        }
        let _done = Done(active.clone(), id, preview_receipt, app.clone());

        // 이벤트 영속을 전용 스레드로 분리 — 읽기 루프가 per-event DB 쓰기에 막히지 않게(백프레셔 방지).
        // mpsc는 순서 보존 + 논블로킹 송신. 턴 종료 시 tx drop → 수신측이 잔여 배수 후 종료.
        let (persist_tx, persist_rx) = std::sync::mpsc::channel::<String>();
        let persist_pool = pool.clone();
        let persist_handle = std::thread::spawn(move || {
            while let Ok(json) = persist_rx.recv() {
                tauri::async_runtime::block_on(async {
                    let _ = db::append_convo_event(&persist_pool, id, &json, now()).await;
                });
            }
        });

        // 합성 오류 Result — 인터럽트/타임아웃(result 미수신)과 실행 오류에서 공용.
        let synthetic_error = |text: String| crate::convo::ConvoEvent::Result {
            text,
            is_error: true,
            session_id: String::new(),
            cost_usd: 0.0,
            num_turns: 0,
            tokens_in: 0,
            tokens_out: 0,
        };

        // 관측 누적은 라운드 루프 **밖**에 산다 — 턴 에필로그(질문 판별·캡처 게이트)는 라운드
        // 시퀀스 전체를 한 턴으로 본다.
        // result 미수신으로 끝나면(인터럽트 또는 유휴 타임아웃 kill) 합성 Result로 busy 해제.
        // tool_seen이 없으면 파일 변경이 불가능 → 턴 종료 후 has_changes(git 셸아웃)를 건너뛴다.
        let mut result_seen = false;
        let mut tool_seen = false;
        let mut untracked_input_seen = false;
        let mut delivery_accepted = false;
        let delivery_pool = pool.clone();
        let delivery_attempt = attempt.clone();
        // 메인 스레드 마지막 어시스턴트 텍스트 + 오류 여부 — 턴 종료 후 질문 대기 판별 재료.
        let mut last_text = String::new();
        let mut result_error = false;

        // 라운드 루프. 토론이 아니면 정확히 한 바퀴 돌고 나가므로 오늘과 같은 경로다.
        // 토론이면 **한 시퀀스 전체가 한 점유**여야 한다 — 턴마다 `Done`을 놓으면 그 틈에
        // `reserve_convo_switch`가 열려 R이 없는 라운드가 남는다(설계 §4-2).
        let mut debate = debate;
        let left_agent = agent.clone();
        let mut left_resume = resume.clone();
        let mut agent = agent;
        let mut resume = resume;
        let mut turn_side = debate.as_ref().map(|_| crate::convo::Side::Left);
        let mut first_turn = true;
        let mut debate_end = None;
        loop {
            // 라운드 사이의 중단. `interrupt_conversation`이 pgid와 무관하게 플래그를 세우므로
            // 다음 spawn을 여기서 막는 것으로 중단이 성립한다.
            if debate.is_some() && !first_turn && convo_interrupted(&active, id) {
                debate_end = Some(crate::convo::DebateEndReason::Aborted);
                break;
            }
            // 이번 턴의 벤더·세션. 좌측의 원천은 `tasks`, 나머지는 `convo_debate_sides`뿐이다.
            if let (Some(rt), Some(side)) = (debate.as_ref(), turn_side) {
                let (next_agent, next_resume) = match side {
                    crate::convo::Side::Left => (left_agent.clone(), left_resume.clone()),
                    crate::convo::Side::Right | crate::convo::Side::Third => rt
                        .seat(side)
                        .map(|seat| (seat.agent.clone(), seat.session.clone()))
                        .unwrap_or_default(),
                };
                agent = next_agent;
                resume = next_resume;
            }
            if structured && !first_turn {
                match tauri::async_runtime::block_on(next_question_turn(
                    &pool, id, &agent, turn_side, admission_generation, question_changed.clone(),
                )) {
                    Ok(ctx) => structured_ctx = ctx,
                    Err(error) => {
                        result_error = true;
                        let ev = synthetic_error(format!("토론 질문 실행 준비 실패: {error}"));
                        if let Ok(json) = crate::convo::stored_event_json(&ev, turn_side) {
                            let _ = persist_tx.send(json);
                        }
                        let _ = app.emit("convo://event", ConvoPayload { id, speaker: turn_side, event: ev });
                        debate_end = Some(crate::convo::DebateEndReason::Error);
                        break;
                    }
                }
            }
            if structured && (convo_interrupted(&active, id)
                || crate::convo::interaction_commands::ensure_generation(admission_generation).is_err())
            {
                result_error = true;
                if let Some(ctx) = structured_ctx.as_ref() {
                    ctx.control.cancelled.store(true, Ordering::SeqCst);
                }
                let ev = synthetic_error("턴 시작 전에 실행이 중단되었습니다".into());
                if let Ok(json) = crate::convo::stored_event_json(&ev, turn_side) {
                    let _ = persist_tx.send(json);
                }
                let _ = app.emit("convo://event", ConvoPayload { id, speaker: turn_side, event: ev });
                debate_end = debate.as_ref().map(|_| crate::convo::DebateEndReason::Aborted);
                break;
            }
            // 프롬프트: 좌측 첫 턴만 기존 조립(캡슐·스킬·Goal Contract)을 그대로 쓰고, 세션 없는 비좌측 턴은
            // 에이전트 전환의 인계 조립을 앞에 둔다. 나머지는 각 세션이 resume이므로 접미만 중계한다.
            let prompt = match debate.as_ref() {
                None => message.clone(),
                Some(rt) => {
                    let side = turn_side.unwrap_or(crate::convo::Side::Left);
                    let ctx = rt.round_context(side, &left_agent);
                    if first_turn {
                        format!("{message}\n{}", crate::convo::debate::debate_suffix(&ctx))
                    } else if side != crate::convo::Side::Left
                        && rt.seat(side).is_some_and(|seat| seat.session.is_none())
                    {
                        // 인계 조립 실패를 기본값으로 덮으면 그 자리가 캡슐도 직전 대화도 없이
                        // 시작한다 — 라운드를 열지 않고 오류로 끝낸다(계획 §4-T3).
                        match tauri::async_runtime::block_on(agent_switch_handoff(&pool, id)) {
                            Ok(handoff) => crate::convo::debate::first_right_prompt(&handoff, &ctx),
                            Err(error) => {
                                let ev = synthetic_error(format!("토론 인계 조립 실패: {error}"));
                                if let Ok(j) = crate::convo::stored_event_json(&ev, turn_side) {
                                    let _ = persist_tx.send(j);
                                }
                                let _ = app.emit(
                                    "convo://event",
                                    ConvoPayload {
                                        id,
                                        speaker: turn_side,
                                        event: ev,
                                    },
                                );
                                debate_end = Some(crate::convo::DebateEndReason::Error);
                                break;
                            }
                        }
                    } else {
                        crate::convo::debate::debate_suffix(&ctx)
                    }
                }
            };
            // 이번 턴이 확립한 벤더 세션 id — 그 자리의 다음 턴의 resume 근거가 된다.
            let mut turn_session = None;
            let vendor = crate::convo::Vendor::from_agent(&agent);
            // 벤더 실행 파일 해석은 여기서(core convo는 PATH 탐색 비의존) — 없으면 에러 result 후 종료.
            let selected_bin = if structured_ctx.is_some() && vendor == crate::convo::Vendor::Codex {
                managed_codex_bin.clone()
            } else {
                crate::reviewer::which(vendor.bin())
            };
            let bin = match selected_bin {
                Some(b) => b,
                None => {
                    let ev = synthetic_error(format!(
                        "{}를 PATH에서 찾지 못함 (설치/PATH 확인)",
                        vendor.bin()
                    ));
                    if let Ok(j) = crate::convo::stored_event_json(&ev, turn_side) {
                        let _ = persist_tx.send(j);
                    }
                    let _ = app.emit(
                        "convo://event",
                        ConvoPayload {
                            id,
                            speaker: turn_side,
                            event: ev,
                        },
                    );
                    // 시작 전 오류도 질문 실행 정리와 상태 전이를 거친다.
                    result_error = true;
                    debate_end = debate.as_ref().map(|_| crate::convo::DebateEndReason::Error);
                    break;
                }
            };
            if let Some(turn) = active
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_mut(&id)
            {
                turn.vendor_bin = bin.clone();
            }
            // 매 턴 최신 값을 읽어 재시작/이어하기에도 반영 (task_create 시점 캐시 금지).
            // 세션 오버라이드(tasks.model)가 있으면 그것을, 없으면 설정의 벤더 기본을 사용.
            // Model, effort and speed share the admission snapshot.
            let service_tier = if debate.is_none() { settings_task.service_tier.as_deref() } else { None };
            // 비좌측 자리의 모델. 와일드카드를 쓰지 않는다 — 셋째가 좌측 경로로 떨어지면 좌측의
            // `tasks.model` 오버라이드가 다른 벤더에 넘어간다(설계 2026-09-23 D4).
            let seat_model = match turn_side {
                None | Some(crate::convo::Side::Left) => None,
                Some(side @ (crate::convo::Side::Right | crate::convo::Side::Third)) => debate
                    .as_ref()
                    .and_then(|rt| rt.seat(side))
                    .map(|seat| seat.model.clone()),
            };
            let model = match seat_model {
                // 모델 미지정이면 설정의 벤더 기본(`model:<agent>`)까지 좌측과 같게 내려간다.
                // `tasks.model`은 좌측 세션의 오버라이드라 다른 자리에 물려주지 않는다.
                Some(model) => model
                    .or_else(|| tauri::async_runtime::block_on(agent_model_of(&pool, &agent))),
                None => settings_task.model.as_deref().map(str::trim).filter(|model| !model.is_empty()).map(str::to_string)
                    .or_else(|| tauri::async_runtime::block_on(agent_model_of(&pool, &agent))),
            };
            let reasoning_effort = crate::agent::reasoning_effort_override(&agent, settings_task.reasoning_effort.as_deref()).ok().flatten();
            // 이 턴 동안만 사는 프리뷰 MCP 토큰·설정. 서버가 안 떴으면 None — 주입 없이 예전 경로.
            let mcp_lease = {
                let state = app.state::<AppState>();
                issue_preview_mcp(&app, &state, id, vendor, structured_ctx.is_some())
            };
            // 실행 전 승인은 질문 세션의 Claude 차례에만 걸린다. 값을 못 읽으면 켠다 — 승인을
            // 요청한 세션이 조용히 자동 승인으로 도는 쪽이, 한 번 더 묻는 쪽보다 되돌리기 어렵다.
            let approvals = structured_ctx.is_some()
                && vendor == crate::convo::Vendor::Claude
                && tauri::async_runtime::block_on(crate::convo::interaction::approvals_of(&pool, id))
                    .unwrap_or(true);
            // 임대가 없으면 승인 도구를 줄 수 없고, 그대로 돌리면 권한 확인 없이 실행된다 — 턴을 막는다.
            let approvals_unavailable = approvals && mcp_lease.is_none();
            let mcp_lease = mcp_lease.map(|lease| if approvals { lease.with_approvals(vendor) } else { lease });
            let res = if approvals_unavailable {
                Err("실행 전 승인 도구를 준비하지 못해 이 차례를 실행하지 않았습니다".to_string())
            } else { crate::convo::app_server::run_selected(
                structured_ctx.as_ref(),
                &cwd,
                &prompt,
                resume.as_deref(),
                CONVO_IDLE_TIMEOUT_SECS,
                vendor,
                &bin,
                model.as_deref(),
                reasoning_effort.as_deref(),
                service_tier,
                &image_paths,
                Some(&crate::agent::session_name(id)),
                mcp_lease.as_ref(),
                |pid| {
                    // 진행 중 엔트리에 pgid 기록(인터럽트용). 가드에서 이미 키를 넣었으므로 값만 갱신.
                    if let Some(turn) = active
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get_mut(&id)
                    {
                        turn.pgid = Some(pid);
                        turn.last_event_at = now();
                    }
                    // 재시작 조정(Plan 0012)의 근거로 DB에도 영속 — 인메모리 소실 대비.
                    let p = pool.clone();
                    tauri::async_runtime::block_on(async move {
                        let _ = db::set_convo_pgid(&p, id, Some(pid as i64)).await;
                    });
                    // 차례 사이의 중단과 spawn이 경합해도 새 프로세스를 계속 실행하지 않는다.
                    if convo_interrupted(&active, id)
                        || (structured && crate::convo::interaction_commands::ensure_generation(admission_generation).is_err())
                    {
                        if !crate::convo::app_server::cancel(id) {
                            crate::verify::kill_group(pid);
                        }
                    }
                },
                |ev| {
                    match &ev {
                        crate::convo::ConvoEvent::SessionInit { session_id } => {
                            // 턴당 1회 — 직접 쓰기(빈도 낮음).
                            let p = pool.clone();
                            let sid = session_id.clone();
                            turn_session = Some(sid.clone());
                            tauri::async_runtime::block_on(async move {
                                let _ = persist_turn_session(&p, id, turn_side, &sid).await;
                            });
                            if !delivery_accepted {
                                if let Some(attempt) = delivery_attempt.as_deref() {
                                    let _ = tauri::async_runtime::block_on(
                                        crate::knowledge::vault::usage::mark_delivery(
                                            &delivery_pool,
                                            id,
                                            attempt,
                                            crate::knowledge::vault::usage::DeliveryState::Delivered,
                                            now(),
                                        ),
                                    );
                                }
                                delivery_accepted = true;
                            }
                        }
                        crate::convo::ConvoEvent::Other
                            if vendor == crate::convo::Vendor::Codex =>
                        {
                            untracked_input_seen = true;
                            if let Some(attempt) = delivery_attempt.as_deref() {
                                let _ = tauri::async_runtime::block_on(
                                    crate::knowledge::vault::provenance::record_unknown_or_fail_closed(
                                        &pool,
                                        attempt,
                                        crate::knowledge::vault::provenance::InputOrigin::ToolResult,
                                        "untrackable Codex stream event",
                                        now(),
                                    ),
                                );
                            }
                        }
                        crate::convo::ConvoEvent::Other => {}
                        _ => {
                            if let Some(turn) = active
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .get_mut(&id)
                            {
                                turn.last_event_at = now();
                                // "최근" 표시는 메인 스레드 도구만 — 서브 에이전트 내부 호출(parent_id)이
                                // 덮어쓰면 메인이 대기 중인데 서브 작업이 현재 작업처럼 보인다.
                                if let crate::convo::ConvoEvent::ToolUse {
                                    name,
                                    summary,
                                    parent_id: None,
                                    ..
                                } = &ev
                                {
                                    turn.last_operation =
                                        Some(format!("{name} {summary}").trim().to_string());
                                }
                            }
                            match &ev {
                                crate::convo::ConvoEvent::Result { text, is_error, .. } => {
                                    result_seen = true;
                                    result_error = *is_error;
                                    // 벤더에 따라 최종 text가 비어 오기도 한다 — 그때는 직전 Text를 쓴다.
                                    if !text.trim().is_empty() {
                                        last_text = text.clone();
                                    }
                                }
                                crate::convo::ConvoEvent::ToolUse { .. }
                                | crate::convo::ConvoEvent::ToolResult { .. }
                                | crate::convo::ConvoEvent::Interaction { .. } => {
                                    tool_seen |=
                                        matches!(&ev, crate::convo::ConvoEvent::ToolUse { .. });
                                    untracked_input_seen = true;
                                    if let Some(attempt) = delivery_attempt.as_deref() {
                                        let payload =
                                            crate::convo::stored_event_json(&ev, turn_side)
                                                .unwrap_or_else(|_| {
                                                    "unserializable tool event".into()
                                                });
                                        let _ = tauri::async_runtime::block_on(crate::knowledge::vault::provenance::record_unknown_or_fail_closed(&pool, attempt, crate::knowledge::vault::provenance::InputOrigin::ToolResult, &payload, now()));
                                    }
                                }
                                // 서브 에이전트 텍스트(parent_id)는 사용자에게 던진 말이 아니다.
                                crate::convo::ConvoEvent::Text {
                                    text,
                                    parent_id: None,
                                }
                                | crate::convo::ConvoEvent::TextUpdate {
                                    text,
                                    complete: true,
                                    ..
                                } => {
                                    if !text.trim().is_empty() {
                                        last_text = text.clone();
                                    }
                                }
                                _ => {}
                            }
                            if !matches!(
                                &ev,
                                crate::convo::ConvoEvent::TextUpdate {
                                    complete: false,
                                    ..
                                }
                            ) {
                                if let Ok(j) = crate::convo::stored_event_json(&ev, turn_side) {
                                    let _ = persist_tx.send(j);
                                }
                            }
                        }
                    }
                    let _ = app.emit(
                        "convo://event",
                        ConvoPayload {
                            id,
                            speaker: turn_side,
                            event: ev,
                        },
                    );
                },
            ) };
            drop(mcp_lease); // 턴 종료 즉시 토큰 폐기 — 이후 호출은 401(AC-12).
                             // 끝난 턴의 pgid를 지운다 — 라운드 사이의 인터럽트가 이미 죽은(또는 OS가 재활용한)
                             // pgid를 죽이는 것을 막는다. 단일 턴 경로는 에필로그가 지우므로 토론에서만 한다.
            if debate.is_some() {
                if let Some(turn) = active
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get_mut(&id)
                {
                    turn.pgid = None;
                }
                tauri::async_runtime::block_on(async {
                    let _ = db::set_convo_pgid(&pool, id, None).await;
                });
            }
            match res {
                Ok(out) => {
                    let p = pool.clone();
                    let sid = out.session_id.clone();
                    if !sid.is_empty() {
                        turn_session = Some(sid.clone());
                    }
                    tauri::async_runtime::block_on(async move {
                        let _ = persist_turn_session(&p, id, turn_side, &sid).await;
                    });
                    if !result_seen {
                        // 사인 구분: 사용자 인터럽트 / 유휴 워치독 / 프로세스 자체 사망(stderr 표면화).
                        let interrupted = active
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .get(&id)
                            .map(|t| t.interrupted)
                            .unwrap_or(false);
                        let text = if interrupted {
                            "턴이 중단되었습니다 (사용자 인터럽트)".to_string()
                        } else if out.timed_out {
                            format!(
                                "턴이 유휴 타임아웃으로 중단되었습니다 ({}시간 무출력)",
                                CONVO_IDLE_TIMEOUT_SECS / 3600
                            )
                        } else {
                            let mut t = format!(
                                "에이전트 프로세스가 결과 없이 종료되었습니다 ({})",
                                out.exit_desc
                            );
                            let tail = out.stderr_tail.trim();
                            if !tail.is_empty() {
                                t.push_str("\n\nstderr:\n");
                                t.push_str(tail);
                            } else {
                                t.push_str(" — 디스크 여유 공간/인증 상태를 확인하세요");
                            }
                            t
                        };
                        let ev = synthetic_error(text);
                        if let Ok(j) = crate::convo::stored_event_json(&ev, turn_side) {
                            let _ = persist_tx.send(j);
                        }
                        let _ = app.emit(
                            "convo://event",
                            ConvoPayload {
                                id,
                                speaker: turn_side,
                                event: ev,
                            },
                        );
                    }
                }
                Err(e) => {
                    result_error = true;
                    let ev = synthetic_error(format!("대화 오류: {e}"));
                    if let Ok(j) = crate::convo::stored_event_json(&ev, turn_side) {
                        let _ = persist_tx.send(j);
                    }
                    let _ = app.emit(
                        "convo://event",
                        ConvoPayload {
                            id,
                            speaker: turn_side,
                            event: ev,
                        },
                    );
                }
            }

            // 라운드 판정. 토론이 아니면 한 바퀴로 끝난다.
            let Some(rt) = debate.as_mut() else { break };
            let side = turn_side.unwrap_or(crate::convo::Side::Left);
            // 다음 라운드의 resume 근거. 첫 턴이 판 세션을 여기서 받지 않으면 라운드 2가
            // 맥락 없는 새 세션으로 시작한다.
            if let Some(session) = turn_session {
                match side {
                    crate::convo::Side::Left => left_resume = Some(session),
                    crate::convo::Side::Right | crate::convo::Side::Third => {
                        if let Some(seat) = rt.seat_mut(side) {
                            seat.session = Some(session);
                        }
                    }
                }
            }
            // 조건부로 두면 이번 턴이 말이 없을 때 **자기 직전 발화**가 다음 차례에 다시 중계된다.
            rt.last[side.index()] = (!last_text.trim().is_empty()).then(|| last_text.clone());
            let outcome = if convo_interrupted(&active, id) {
                crate::convo::debate::TurnOutcome::Interrupted
            } else if !result_seen || result_error {
                crate::convo::debate::TurnOutcome::Failed
            } else {
                crate::convo::debate::TurnOutcome::Spoke {
                    consensus: crate::convo::debate::is_consensus(&last_text),
                }
            };
            match rt.state.advance(side, outcome) {
                crate::convo::debate::Step::End(reason) => {
                    debate_end = Some(reason);
                    break;
                }
                crate::convo::debate::Step::Next(next) => {
                    turn_side = Some(next);
                    first_turn = false;
                    // 턴 단위 관측만 되돌린다 — 누적분(tool_seen 등)은 시퀀스 전체의 것이다.
                    result_seen = false;
                    result_error = false;
                    last_text.clear();
                }
            }
        }
        // 종료 경계는 삼키지 않고 남긴다 — 경계 상실이 적재 실패보다 위험하다.
        if let Some(reason) = debate_end {
            let ev = crate::convo::ConvoEvent::DebateEnded { reason };
            if let Ok(j) = crate::convo::stored_event_json(&ev, None) {
                let _ = persist_tx.send(j);
            }
            let _ = app.emit(
                "convo://event",
                ConvoPayload {
                    id,
                    speaker: None,
                    event: ev,
                },
            );
        }
        if result_seen && !result_error && !untracked_input_seen {
            if let Some(attempt) = delivery_attempt.as_deref() {
                let _ = tauri::async_runtime::block_on(
                    crate::knowledge::vault::provenance::complete_conversation_provenance(
                        &pool, attempt,
                    ),
                );
            }
        }
        if !delivery_accepted {
            if let Some(attempt) = delivery_attempt.as_deref() {
                let _ =
                    tauri::async_runtime::block_on(crate::knowledge::vault::usage::mark_delivery(
                        &delivery_pool,
                        id,
                        attempt,
                        crate::knowledge::vault::usage::DeliveryState::NotDelivered,
                        now(),
                    ));
            }
        }
        // 영속 스레드 종료 대기 — 모든 이벤트가 DB에 기록된 뒤 상태 전이로 진행.
        drop(persist_tx);
        let _ = persist_handle.join();

        if let Some(ctx) = structured_ctx.as_ref() {
            if tauri::async_runtime::block_on(finish_unstarted_question(ctx)).is_err() {
                crate::convo::app_server::mark_cleanup_failed(id);
            }
        }
        if structured && crate::convo::app_server::cleanup_failed(id) {
            return;
        }

        // 상태 전이는 in-flight를 쥔 채 수행 — drop(_done)을 먼저 하면 새 턴이 RUNNING을 쓴 직후
        // 이 스레드가 AwaitingReview로 덮어써 busy 표시가 깨진다(리뷰 지적 레이스).
        // 턴 종료 시 **항상** 검토 대기로 전이 — 순수 Q&A 턴(툴 미사용)도 Running에 고착되지 않게
        // (안 그러면 대시보드 "진행 중"·앙상블 allReady가 영영 안 풀린다). git has_changes·알림·캡처는
        // 툴을 실제 쓴 경우에만(변경 가능성 있을 때). task_row는 repo 확보 위해 항상 조회.
        let task_row =
            tauri::async_runtime::block_on(async { db::get_task(&pool, id).await.ok().flatten() });
        let changed = tool_seen
            && task_row
                .as_ref()
                .map(|t| worktree_from_task(t).has_changes().unwrap_or(false))
                .unwrap_or(false);
        // 검토 대기의 성격 판별 — 정상 종료한 턴이 워크트리를 바꾸지 않았고 마지막 텍스트가
        // 질문이면, 이건 결과 검토가 아니라 사용자 답을 기다리는 상태다. 기준은 러너와 공유한다.
        let awaiting_kind =
            crate::convo::question::awaits_answer(&crate::convo::question::TurnEpilogue {
                last_text: &last_text,
                worktree_changed: changed,
                result_seen,
                result_error,
            })
            .then_some(db::awaiting_kind::QUESTION);
        let epilogue = tauri::async_runtime::block_on(async {
            db::set_convo_pgid(&pool, id, None).await?; // 완료 turn은 재시작 조정 대상 아님(Plan 0012).
            let notice_kind = if result_error {
                "failure"
            } else if awaiting_kind.is_some() {
                "question"
            } else {
                "result"
            };
            db::mark_awaiting_review_with_notification(
                &pool,
                id,
                now(),
                awaiting_kind,
                notice_kind,
            )
            .await?;
            Ok::<_, anyhow::Error>(())
            // 가드: RUNNING→AwaitingReview
        });
        if structured {
            if epilogue.is_err() {
                crate::convo::app_server::mark_cleanup_failed(id);
                return;
            }
            crate::convo::app_server::unregister(id);
        }
        emit_awaiting_review(&app, &pool, id, awaiting_kind);
        // in-flight 해제 — 이후 새 턴 허용. 캡처(느림)는 해제 뒤라 다음 턴을 막지 않는다.
        drop(_done);
        if structured {
            let _ = app.emit(
                "convo-interaction://changed",
                serde_json::json!({"taskId":id}),
            );
        }
        // 자동 캡처·회고(P2)와 수동 회고(자기개선 제안)는 모두 제거됐다(설계 2026-09-13
        // §P2 제거, 1.0 자기개선 제거) — 메모리 정본이 파일이 된 뒤로 자동 추출·관측할
        // 대상이 없고, 도달 불가 상태였던 제안 검토 화면도 함께 없앴다.
    });
    Ok(())
}

/// 대화 모드(Phase 2) 한 턴 — task.agent 벤더(claude/codex/agy)를 worktree에서 실행,
/// 이벤트를 `convo://event`로 스트리밍. 멀티턴은 저장된 세션 토큰으로 벤더별 resume.
/// 백그라운드 스레드에서 진행(즉시 반환).
#[tauri::command]
pub async fn convo_send(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    message: String,
    image_paths: Option<Vec<String>>,
) -> Result<(), String> {
    convo_send_with_receipt(app, state, id, message, image_paths, None, None).await
}

async fn convo_send_with_receipt(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    message: String,
    image_paths: Option<Vec<String>>,
    receipt_request_id: Option<&str>,
    preview_client_ref: Option<&str>,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    let task = db::get_task(&pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    let cwd = task.worktree_path.clone();
    if !std::path::Path::new(&cwd).is_dir() {
        return Err(worktree::missing_worktree_error(&cwd));
    }
    let image_paths = crate::designmode::validate_capture_image_paths(
        Path::new(&task.worktree_path),
        id,
        &image_paths.unwrap_or_default(),
    )?;
    let preview_client_ref = match preview_client_ref {
        Some(client_ref) if valid_vault_followup_client_ref(id, client_ref) => Some(client_ref.to_owned()),
        Some(_) => return Err("vault composer request is invalid".into()),
        None => Some(format!("vault-followup:{id}")),
    };
    // resume 근거는 DB(convo_session_id) — 앱 재시작 후에도 이전 세션으로 이어진다.
    start_convo_turn(
        app,
        pool,
        state.convo_active.clone(),
        id,
        task.repo.clone(),
        cwd,
        message,
        image_paths,
        receipt_request_id,
        ConversationInputOrigin::UserMessage,
        preview_client_ref,
        state.updating.clone(),
        None,
        ConvoAdmissionAction::ReleaseManualTakeover,
    )
    .await
}

#[tauri::command]
pub async fn side_question_read(state: State<'_, AppState>, task_id: i64) -> Result<crate::side_question::SideQuestionSnapshot, String> {
    crate::side_question::read(&pool_of(&state)?, task_id, now()).await
}

#[tauri::command]
pub async fn side_question_send(
    app: AppHandle, state: State<'_, AppState>, task_id: i64, input: crate::side_question::SideQuestionSend,
) -> Result<crate::side_question::SideQuestionSnapshot, String> {
    let pool = pool_of(&state)?;
    let (turn_id, inserted) = crate::side_question::send(&pool, task_id, input, now()).await?;
    if inserted {
        let pool = pool.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                let queued: Option<String> = sqlx::query_scalar("SELECT state FROM side_question_turns WHERE id=? AND task_id=?")
                    .bind(turn_id).bind(task_id).fetch_optional(&pool).await.ok().flatten();
                if queued.as_deref() != Some("queued") { break; }
                let parent = db::get_task(&pool, task_id).await.ok().flatten();
                let Some(parent) = parent else {
                    let _ = crate::side_question::cancel(&pool, task_id, turn_id, now()).await;
                    break;
                };
                if !crate::side_question::parent_allows_side_question(&parent) {
                    let _ = crate::side_question::cancel(&pool, task_id, turn_id, now()).await;
                    break;
                }
                let reservation = side_question_slots::reserve_question(&app.state::<AppState>(), task_id);
                match reservation {
                    Ok(_reservation) => {
                        crate::side_question::run_turn(pool.clone(), task_id, turn_id, now()).await;
                        break;
                    }
                    Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
                }
            }
        });
    }
    crate::side_question::read(&pool, task_id, now()).await
}

#[tauri::command]
pub async fn side_question_cancel(state: State<'_, AppState>, task_id: i64, turn_id: i64) -> Result<crate::side_question::SideQuestionSnapshot, String> {
    let pool = pool_of(&state)?;
    crate::side_question::cancel(&pool, task_id, turn_id, now()).await?;
    crate::side_question::read(&pool, task_id, now()).await
}

#[tauri::command]
pub async fn side_question_reset(state: State<'_, AppState>, task_id: i64, generation: i64) -> Result<crate::side_question::SideQuestionSnapshot, String> {
    let pool = pool_of(&state)?;
    crate::side_question::reset(&pool, task_id, generation, now()).await?;
    crate::side_question::read(&pool, task_id, now()).await
}

#[tauri::command]
pub async fn conversation_submit(
    app: AppHandle, state: State<'_, AppState>, task_id: i64, request_id: String,
    message: String, image_paths: Option<Vec<String>>, preview_client_ref: Option<String>,
) -> Result<crate::side_question::ConversationReceipt, String> {
    let pool = pool_of(&state)?;
    if db::get_task(&pool, task_id).await.map_err(|e| e.to_string())?.is_none() {
        return Ok(crate::side_question::ConversationReceipt { request_id, status: "not_found".into(), error: None });
    }
    let _receipt_admission = crate::side_question::receipt_admission_lock(task_id, &request_id).await;
    let images = image_paths.unwrap_or_default();
    if let Some(receipt) = crate::side_question::receipt_begin(&pool, task_id, &request_id, &message, &images, now()).await? { return Ok(receipt); }
    if crate::side_question::blocks_main_execution(&pool, task_id).await? {
        return crate::side_question::receipt_read(&pool, task_id, &request_id).await;
    }
    if crate::side_question::receipt_main_admitted(&pool, task_id, &request_id).await? {
        return crate::side_question::receipt_finish(&pool, task_id, &request_id, "accepted", None).await;
    }
    match convo_send_with_receipt(app, state, task_id, message, Some(images), Some(&request_id), preview_client_ref.as_deref()).await {
        Ok(()) => crate::side_question::receipt_finish(&pool, task_id, &request_id, "accepted", None).await,
        Err(error) => crate::side_question::receipt_finish(&pool, task_id, &request_id, "failed", Some(&error)).await,
    }
}

#[tauri::command]
pub async fn conversation_receipt(state: State<'_, AppState>, task_id: i64, request_id: String) -> Result<crate::side_question::ConversationReceipt, String> {
    crate::side_question::receipt_read(&pool_of(&state)?, task_id, &request_id).await
}

/// 선택한 리뷰 주석 n건을 한 번에 재전송 — 규정 포맷(designs/0012 §6.2)으로 합성해
/// `start_convo_turn`(convo_send와 동일 경로)의 resume 첫 메시지로 주입한다.
/// status=sent 갱신은 트랜잭션으로 먼저 반영하고, resume 트리거의 동기 가드가 실패하면
/// (작업 없음/워크트리 없음/이미 진행 중인 턴 등) 즉시 draft로 롤백해 부분 갱신을 막는다.
#[tauri::command]
pub async fn annotations_resend(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: i64,
    ids: Vec<String>,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    let task = db::get_task(&pool, task_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    let targets = annotations::list_by_ids(&pool, task_id, &ids)
        .await
        .map_err(|e| e.to_string())?;
    if targets.is_empty() {
        return Err("재전송할 주석을 찾을 수 없습니다".into());
    }
    let hunks = task_hunks(&task)?;
    let message = annotations::format_resend(&annotations::build_resend_items(&targets, &hunks));

    let updated = annotations::mark_sent(&pool, task_id, &ids)
        .await
        .map_err(|e| e.to_string())?;
    if updated == 0 {
        return Err("재전송 가능한 초안 주석이 없습니다(이미 전송됨)".into());
    }

    let cwd = task.worktree_path.clone();
    if !std::path::Path::new(&cwd).is_dir() {
        let _ = annotations::mark_draft(&pool, task_id, &ids).await;
        return Err(worktree::missing_worktree_error(&cwd));
    }
    let result = start_convo_turn(
        app,
        pool.clone(),
        state.convo_active.clone(),
        task_id,
        task.repo.clone(),
        cwd,
        message,
        Vec::new(),
        None,
        ConversationInputOrigin::AnnotationResend,
        Some(format!("vault-followup:{task_id}")),
        state.updating.clone(),
        None,
        ConvoAdmissionAction::PreserveTakeover,
    )
    .await;
    if result.is_err() {
        let _ = annotations::mark_draft(&pool, task_id, &ids).await;
    }
    result
}

/// 플래그와 kill을 가른다. **플래그는 pgid와 무관하게 세운다** — 라운드 사이(직전 턴 종료 ~
/// 다음 spawn)에는 pgid가 없고, 그 구간의 중단은 다음 spawn을 막는 것으로 성립하기 때문이다
/// (설계 §4-2). 진행 중(active에 키 존재)이 아니면 여전히 거절한다 — 잔류/재활용 pgid에
/// SIGKILL을 보내지 않기 위한 스테일 kill 가드다.
fn interrupt_convo_entry(active: &ActiveConvos, id: i64) -> Result<(), String> {
    let pgid = {
        let mut turns = active.lock().unwrap_or_else(|e| e.into_inner());
        let Some(turn) = turns.get_mut(&id) else {
            return Err("진행 중인 턴이 없습니다".into());
        };
        // 턴 스레드가 result 미수신 종료의 사인을 "사용자 인터럽트"로 표시하는 근거이기도 하다.
        turn.interrupted = true;
        turn.pgid
    };
    if let Some(pid) = pgid {
        crate::verify::kill_group(pid);
    }
    Ok(())
}

fn interrupt_conversation(state: &AppState, id: i64) -> Result<(), String> {
    if crate::convo::app_server::cancel(id) {
        if let Some(entry)=state.convo_active.lock().unwrap_or_else(|e|e.into_inner()).get_mut(&id) {entry.interrupted=true;}
        return Ok(());
    }
    interrupt_convo_entry(&state.convo_active, id)
}

/// 실행 중인 대화 턴 인터럽트 — 자식 프로세스 그룹 kill. 합성 중단 Result는 턴 스레드가 emit.
#[tauri::command]
pub async fn convo_interrupt(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let pool = pool_of(&state)?;
    if !db::record_notification_cancel_intent(&pool, id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("실행 중인 작업만 중단할 수 있습니다".to_string());
    }
    if let Err(error) = interrupt_conversation(&state, id) {
        db::clear_notification_cancel_if_signal_not_sent(&pool, id)
            .await
            .map_err(|clear_error| clear_error.to_string())?;
        return Err(error);
    }
    Ok(())
}

/// 실행 중인 로컬 작업을 종료하고 검토 대기 전이가 끝날 때까지 기다린다.
#[tauri::command]
pub async fn task_cancel(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    cancel_task_inner(&state, id).await
}

/// `task_cancel`의 본체. 파이프라인 드라이버도 티켓 작업을 멈출 때 쓴다.
pub(crate) async fn cancel_task_inner(state: &AppState, id: i64) -> Result<(), String> {
    let pool = pool_of(state)?;
    let task = db::get_task(&pool, id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
    if task.state == tstate::AWAITING_REVIEW {
        return Ok(());
    }
    if !matches!(task.state.as_str(), tstate::CREATED | tstate::RUNNING) {
        return Err(format!(
            "실행 중인 작업만 중단할 수 있습니다 (현재: {})",
            task.state
        ));
    }
    if !db::record_notification_cancel_intent(&pool, id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("작업 상태가 동시에 변경되었습니다".to_string());
    }
    let signal = if task.mode == "conversation" {
        interrupt_conversation(state, id)
    } else {
        let tasks = state
            .tasks
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let result = tasks
            .get(&id)
            .and_then(|active| active.session.as_ref())
            .map(|session| {
                session.terminate();
            })
            .ok_or_else(|| "실행 중인 터미널 세션이 없습니다".to_string());
        result
    };
    if let Err(error) = signal {
        db::clear_notification_cancel_if_signal_not_sent(&pool, id)
            .await
            .map_err(|clear_error| clear_error.to_string())?;
        return Err(error);
    }
    for _ in 0..100 {
        let current = db::get_task(&pool, id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
        let conversation_done = task.mode != "conversation"
            || !state
                .convo_active
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .contains_key(&id);
        if current.state == tstate::AWAITING_REVIEW && conversation_done {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Err("중단은 요청했지만 검토 대기 전이가 완료되지 않았습니다".to_string())
}

#[tauri::command]
pub async fn notification_source_page(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    after: Option<i64>,
) -> Result<crate::notifications::SourcePage, String> {
    require_main_notification_window(&window)?;
    crate::notifications::source_page(&pool_of(&state)?, after)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn notification_snapshot(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<crate::notifications::Snapshot, String> {
    require_notification_window(&window)?;
    crate::notifications::snapshot(&pool_of(&state)?)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn notification_ingest(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    host: String,
    page: crate::notifications::SourcePage,
    notify: bool,
) -> Result<crate::notifications::Snapshot, String> {
    require_main_notification_window(&window)?;
    let pool = pool_of(&state)?;
    let delivered = crate::notifications::ingest(&pool, &host, &page)
        .await
        .map_err(|error| error.to_string())?;
    let mut snapshot = crate::notifications::snapshot(&pool)
        .await
        .map_err(|error| error.to_string())?;
    if notify && snapshot.enabled && !delivered.is_empty() {
        use tauri_plugin_notification::NotificationExt;
        let mut delivery_failed = false;
        for item in delivered {
            let title = format!("Dojang · 작업 #{}", item.task_id);
            let body = format!(
                "{} · {} · {}",
                notification_kind_label(&item.kind),
                notification_repo_label(&item.repo),
                notification_host_label(&host),
            );
            if let Err(error) = app.notification().builder().title(title).body(body).show() {
                let error = error.to_string();
                crate::notifications::set_delivery_error(&pool, Some(&error))
                    .await
                    .map_err(|error| error.to_string())?;
                snapshot.delivery_error = Some(error);
                delivery_failed = true;
                break;
            }
        }
        if !delivery_failed {
            crate::notifications::set_delivery_error(&pool, None)
                .await
                .map_err(|error| error.to_string())?;
            snapshot.delivery_error = None;
        }
    }
    let _ = app.emit("notification://changed", ());
    Ok(snapshot)
}

#[tauri::command]
pub async fn notification_acknowledge(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    host: String,
    source_id: String,
    task_id: i64,
    through_sequence: i64,
) -> Result<crate::notifications::Snapshot, String> {
    require_notification_window(&window)?;
    let pool = pool_of(&state)?;
    crate::notifications::acknowledge(&pool, &host, &source_id, task_id, through_sequence)
        .await
        .map_err(|error| error.to_string())?;
    let _ = app.emit("notification://changed", ());
    crate::notifications::snapshot(&pool).await.map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn notification_reconcile(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    host: String,
    task_ids: Vec<i64>,
) -> Result<crate::notifications::Snapshot, String> {
    require_main_notification_window(&window)?;
    let pool = pool_of(&state)?;
    crate::notifications::reconcile(&pool, &host, &task_ids)
        .await
        .map_err(|error| error.to_string())?;
    let _ = app.emit("notification://changed", ());
    crate::notifications::snapshot(&pool).await.map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn notification_settings_set(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<crate::notifications::Snapshot, String> {
    require_notification_window(&window)?;
    let pool = pool_of(&state)?;
    crate::notifications::set_enabled(&pool, enabled)
        .await
        .map_err(|error| error.to_string())?;
    crate::notifications::snapshot(&pool).await.map_err(|error| error.to_string())
}

#[tauri::command]
pub fn notification_permission(window: tauri::WebviewWindow) -> Result<&'static str, String> {
    require_notification_window(&window)?;
    // The plugin currently reports granted even where desktop permission cannot be queried.
    Ok("unknown")
}

#[tauri::command]
pub async fn notification_test(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_notification_window(&window)?;
    use tauri_plugin_notification::NotificationExt;
    app.notification()
        .builder()
        .title("Dojang 알림 테스트")
        .body("알림이 정상적으로 전달됩니다")
        .show()
        .map_err(|error| error.to_string())?;
    crate::notifications::set_delivery_error(&pool_of(&state)?, None)
        .await
        .map_err(|error| error.to_string())
}

fn require_main_notification_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        return Ok(());
    }
    Err("알림 수집은 메인 창에서만 수행할 수 있습니다".to_string())
}

fn require_notification_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if matches!(window.label(), "main" | "editor") {
        return Ok(());
    }
    Err("알림 정보는 메인 또는 에디터 창에서만 사용할 수 있습니다".to_string())
}

fn notification_host_label(host: &str) -> &str {
    if host == "local" {
        "로컬"
    } else {
        host
    }
}

fn notification_kind_label(kind: &str) -> &'static str {
    match kind {
        "question" => "답변 필요",
        "failure" => "실행 실패",
        _ => "결과 도착",
    }
}

fn notification_repo_label(repo: &str) -> &str {
    repo.rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or("프로젝트")
}

/// 대화 히스토리 + 진행 상태 — 작업 재진입/앱 재시작 시 트랜스크립트·busy 복원.
#[derive(Clone, Serialize)]
pub struct ConvoHistory {
    pub items: Vec<serde_json::Value>,
    pub busy: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConvoLiveness {
    Starting,
    Running,
    EndedWithoutResult,
    Unknown,
}

#[derive(Clone, Serialize)]
pub struct ConvoStatus {
    pub state: ConvoLiveness,
    pub started_at: i64,
    pub last_event_at: i64,
    pub last_operation: Option<String>,
    pub checked_at: i64,
}

/// 실행 중인 턴 하나의 활동 신호.
#[derive(Clone, Serialize)]
pub struct TaskActivity {
    pub task_id: i64,
    pub last_operation: Option<String>,
    pub last_event_at: i64,
    /// 이번 턴이 시작된 시각. 대기 퀴즈의 경과 게이팅이 이것을 쓴다(설계 0044 DR-2).
    /// `last_event_at`으로 대신할 수 없다 — 도구를 쉬지 않고 돌리는 긴 작업에서는
    /// 그 값이 계속 갱신돼 경과가 늘 0에 가깝다.
    pub started_at: i64,
}

/// 실행 중인 모든 대화 턴의 현재 작업을 한 번에 뜬다.
///
/// 새 이벤트 스트림을 뚫지 않는 이유는 `ActiveConvo.last_operation`이 이미 존재하고
/// **메인 스레드 도구만** 담기 때문이다(서브 에이전트 제외는 위 `ToolUse` 처리 참조).
/// 같은 사실을 두 경로로 흘리면 정합성 부채가 생긴다.
///
/// 배치인 이유는 `convo_status(id)`를 작업 수만큼 부르면 활동을 훑을 때마다 뮤텍스를
/// N번 잡기 때문이다.
pub(crate) fn snapshot_task_activity(convos: &HashMap<i64, ActiveConvo>) -> Vec<TaskActivity> {
    convos
        .iter()
        .map(|(id, turn)| TaskActivity {
            task_id: *id,
            last_operation: turn.last_operation.clone(),
            last_event_at: turn.last_event_at,
            started_at: turn.started_at,
        })
        .collect()
}

#[tauri::command]
pub fn task_activity(state: State<AppState>) -> Vec<TaskActivity> {
    let convos = state
        .convo_active
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    snapshot_task_activity(&convos)
}

// ── 대기 퀴즈 (설계 0044) ──

#[cfg(test)]
mod task_activity_tests {
    use super::{snapshot_task_activity, ActiveConvo};
    use std::collections::HashMap;

    fn turn(last_operation: Option<&str>) -> ActiveConvo {
        ActiveConvo {
            pgid: Some(1234),
            vendor_bin: "claude".to_string(),
            started_at: 100,
            last_event_at: 140,
            last_operation: last_operation.map(str::to_string),
            interrupted: false,
        }
    }

    #[test]
    fn snapshots_every_active_turn() {
        let convos = HashMap::from([(1, turn(Some("Read a.rs"))), (2, turn(None))]);
        let mut rows = snapshot_task_activity(&convos);
        rows.sort_by_key(|row| row.task_id);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].task_id, 1);
        assert_eq!(rows[0].last_operation.as_deref(), Some("Read a.rs"));
        assert_eq!(rows[0].last_event_at, 140);
        // None은 0이 아니라 "아직 도구를 쓰지 않았다"다 — 빈 문자열로 접으면 말풍선이 뜬다.
        assert!(rows[1].last_operation.is_none());
    }

    #[test]
    fn no_active_turn_yields_no_rows() {
        assert!(snapshot_task_activity(&HashMap::new()).is_empty());
    }
}

/// 활성 turn을 변경하지 않고 현재 앱 세션이 만든 프로세스 그룹의 생존만 관측한다.
#[tauri::command]
pub fn convo_status(state: State<AppState>, id: i64) -> ConvoStatus {
    let checked_at = now();
    let turn = state
        .convo_active
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .cloned();
    let Some(turn) = turn else {
        return ConvoStatus {
            state: ConvoLiveness::Unknown,
            started_at: checked_at,
            last_event_at: checked_at,
            last_operation: None,
            checked_at,
        };
    };
    let state = match turn.pgid {
        None => ConvoLiveness::Starting,
        Some(pgid) if verify::process_group_alive(pgid) => ConvoLiveness::Running,
        Some(_) => ConvoLiveness::EndedWithoutResult,
    };
    ConvoStatus {
        state,
        started_at: turn.started_at,
        last_event_at: turn.last_event_at,
        last_operation: turn.last_operation,
        checked_at,
    }
}

/// 이어받은 대화에서 물려받은 이력이 차지할 수 있는 최대 이벤트 수.
///
/// 상한이 있는 이유는 체인이 길어질 수 있어서다 — 이어받기를 반복하면 이력이 매번 통째로
/// 누적되고, 그 전부를 창에 밀어 넣으면 세션을 여는 것만으로 화면이 멈춘다. 넘치면 **오래된
/// 쪽부터** 버린다. 직전 대화의 끝이 지금 이어가는 맥락이므로, 잘라야 한다면 먼 과거여야 한다.
const INHERITED_HISTORY_MAX: usize = 2_000;

/// 이어받기 체인에서 물려받은 이력 — 오래된 원본부터, 각 경계에 표시 이벤트를 끼워 반환한다.
///
/// 승계분에 `inherited: true`를 박는 것이 여기서 가장 중요한 일이다. 이 이벤트들은 **다른
/// 작업의 것**이라, 되감기처럼 이벤트에 걸리는 동작을 지금 작업 id로 실행하면 엉뚱한 워크트리를
/// 건드린다. 플래그가 있으면 프런트가 그 동작을 잠글 수 있다.
///
/// 예산은 **최신 원본부터** 집행한다. 상한을 만든 뒤에 자르면 버릴 것까지 전부 읽고 파싱한
/// 다음에 버리게 되어, 상한이 화면 크기만 줄이고 세션을 여는 비용은 그대로 둔다 — 체인이
/// 깊고 대화가 길수록 작업을 선택할 때마다 그 비용을 다시 치른다. 세는 것(`COUNT`)과 읽는
/// 것을 갈라, 무엇을 얼마나 버렸는지는 알면서 버릴 것은 읽지 않는다.
async fn inherited_convo_history(
    pool: &SqlitePool,
    id: i64,
) -> Result<Vec<serde_json::Value>, String> {
    let chain = db::resume_chain(pool, id).await.map_err(|e| e.to_string())?;
    let mut budget = INHERITED_HISTORY_MAX;
    let mut dropped = 0usize;
    let mut blocks: Vec<Vec<serde_json::Value>> = Vec::new();
    for &source_id in chain.iter().rev() {
        let total = db::convo_event_count(pool, source_id)
            .await
            .map_err(|e| e.to_string())?;
        let take = total.min(budget);
        dropped += total - take;
        if take == 0 {
            continue;
        }
        let rows = db::recent_convo_events_tail(pool, source_id, take)
            .await
            .map_err(|e| e.to_string())?;
        budget -= take;
        let mut block: Vec<serde_json::Value> = Vec::with_capacity(rows.len() + 1);
        for row in rows.iter() {
            let Ok(mut event) = serde_json::from_str::<serde_json::Value>(row) else {
                continue;
            };
            if let Some(object) = event.as_object_mut() {
                object.insert("inherited".into(), serde_json::Value::Bool(true));
                object.insert("source_task_id".into(), source_id.into());
            }
            block.push(event);
        }
        // 보여 줄 것이 하나도 없는 원본에는 경계를 붙이지 않는다. 삭제된 원본이 남긴 빈
        // 구분선은 "여기 뭔가 있었다"고만 말하고 아무것도 보여 주지 않는다.
        if !block.is_empty() {
            block.push(serde_json::json!({
                "kind": "resumed_from",
                "source_task_id": source_id,
                "inherited": true,
            }));
        }
        blocks.push(block);
    }
    // 최신 원본부터 채웠으므로 시간순으로 되돌린다.
    blocks.reverse();
    let mut items: Vec<serde_json::Value> = blocks.into_iter().flatten().collect();
    if dropped > 0 {
        items.insert(
            0,
            serde_json::json!({
                "kind": "history_truncated",
                "dropped": dropped,
                "inherited": true,
            }),
        );
    }
    Ok(items)
}

#[tauri::command]
pub async fn convo_history(state: State<'_, AppState>, id: i64) -> Result<ConvoHistory, String> {
    let pool = pool_of(&state)?;
    let rows = db::list_convo_events(&pool, id)
        .await
        .map_err(|e| e.to_string())?;
    // 물려받은 이력이 먼저다. 벤더 세션은 이미 이 대화를 이어가고 있으므로, 화면이 첫 턴부터
    // 시작해 보이면 에이전트만 기억을 가진 상태가 된다 — 사용자는 자기가 하지 않은 말을
    // 근거로 답하는 화면을 보게 된다.
    let mut items = inherited_convo_history(&pool, id).await?;
    items.extend(rows.iter().filter_map(|r| serde_json::from_str(r).ok()));
    let busy = state
        .convo_active
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(&id);
    Ok(ConvoHistory { items, busy })
}

/// 이 작업에서 어느 툴이 컨텍스트를 얼마나 먹었는지. 원장(`convo_events`)에서 매번 파생
/// 계산한다 — 별도 저장소를 두지 않는 이유는 계획 0033 DR-3.
#[tauri::command]
pub async fn task_tool_cost(
    state: State<'_, AppState>,
    id: i64,
) -> Result<crate::convo::tool_cost::ToolCostReport, String> {
    let pool = pool_of(&state)?;
    let rows = db::list_convo_events(&pool, id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(crate::convo::tool_cost::analyze(&rows))
}

// ── 앱 재시작 상태 조정 (Plan 0012) ─────────────────────────────────────────
// 재시작 시 running Task 중 살아있는 대화 turn은 복원(adopt)하고 나머지는 Failed로 정리.
// 구 `mark_stale_running_failed`(무조건 Failed)를 대체 — 창을 닫아도 대화 자식은 살려두므로
// (BR-2) 재시작 시점에 실제 프로세스 생존을 확인해 UI와 실제 상태의 갭을 없앤다.

/// 재시작 시 running Task 1건의 처리 방향.
#[derive(Debug, PartialEq)]
pub(crate) enum StaleAction {
    /// 대화 turn이 살아있음 → Running 유지 + reaper 감시(pgid).
    Adopt(u32),
    /// 죽었거나 비대화/미실행 → Failed로 정리.
    Fail,
}

/// BR-1: `conversation` 모드 + pgid 존재 + 그룹 생존이면 Adopt, 그 외 Fail.
/// `alive`는 주입(테스트 결정성 — 실 호출은 `verify::process_group_alive`).
pub(crate) fn classify_stale(
    mode: &str,
    convo_pgid: Option<u32>,
    alive: impl Fn(u32) -> bool,
) -> StaleAction {
    match convo_pgid {
        Some(pgid) if mode == "conversation" && alive(pgid) => StaleAction::Adopt(pgid),
        _ => StaleAction::Fail,
    }
}

/// 복원된 대화 turn 감시 — pgid 사망까지 폴링 후 정상 종료 경로(AwaitingReview)로 수렴.
/// BR-4 워치독: adopted 턴은 재시작으로 stdout 파이프가 소실돼 유휴 감지가 불가능 — adoption 시점
/// 기준 고정 상한(본선 유휴 상한과 동일한 CONVO_IDLE_TIMEOUT_SECS)으로 대체해 초과 시 강제 종료한다.
fn spawn_convo_reaper(
    app: AppHandle,
    pool: SqlitePool,
    active: ActiveConvos,
    id: i64,
    pgid: u32,
    vendor_bin: String,
    adopted_at: i64,
) {
    const POLL_SECS: u64 = 3;
    const WATCHDOG_SECS: i64 = CONVO_IDLE_TIMEOUT_SECS as i64;
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(POLL_SECS));
            if !verify::process_group_alive(pgid) {
                break; // 자연 종료.
            }
            if now() - adopted_at >= WATCHDOG_SECS {
                // adoption 시점 기준 고정 상한 초과 — 고아 무한 실행 차단. 단, pgid 재사용 오탐으로
                // 무관 프로세스를 SIGKILL하지 않도록 **우리 벤더 프로세스로 확인될 때만** kill(DR-5 완화, 리뷰 지적).
                if verify::process_group_matches(pgid, &vendor_bin) {
                    verify::kill_group(pgid);
                }
                break;
            }
        }
        // 갭 안내 노트 — 크래시 이후 실시간 출력 미복원(DR-4)을 트랜스크립트에 명시.
        let note = crate::convo::ConvoEvent::Result {
            text: "앱이 재시작되어 이 턴의 실시간 출력 일부가 복원되지 않았습니다. \
                   대화 맥락은 유지되어 이어서 계속할 수 있습니다."
                .into(),
            is_error: false,
            session_id: String::new(),
            cost_usd: 0.0,
            num_turns: 0,
            tokens_in: 0,
            tokens_out: 0,
        };
        let note_json = serde_json::to_string(&note).ok();
        tauri::async_runtime::block_on(async {
            if let Some(j) = &note_json {
                let _ = db::append_convo_event(&pool, id, j, now()).await;
            }
            let _ = db::set_convo_pgid(&pool, id, None).await;
            let _ =
                db::mark_awaiting_review_with_notification(&pool, id, now(), None, "failure").await;
            // 가드 전이(BR-3)
        });
        active.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        let _ = app.emit("convo://event", ConvoPayload { id, speaker: None, event: note });
        emit_awaiting_review(&app, &pool, id, None);
    });
}

/// 재시작 조정 — running Task를 순회해 생존 대화 turn은 복원(adopt), 나머지는 Failed.
/// lib.rs `.setup()`에서 1회 호출(구 `mark_stale_running_failed` 대체).
pub async fn reconcile_stale_running(
    app: &AppHandle,
    pool: &SqlitePool,
    direct_repo_locks: &worktree::DirectCheckoutLocks,
    active: ActiveConvos,
    now_ts: i64,
) {
    if let Err(error) = fail_stale_created_direct_tasks(pool, direct_repo_locks, now_ts).await {
        eprintln!("재시작 조정 실패(stale direct Created): {error}");
    }
    let running = match db::list_running_tasks(pool).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("재시작 조정 실패(list_running_tasks): {e}");
            return;
        }
    };
    for t in running {
        if crate::convo::interaction::is_bound(pool,t.id).await.unwrap_or(true) {
            // Structured stdio recovery runs before this legacy adoption pass.
            continue;
        }

        let pgid = t.convo_pgid.map(|v| v as u32);
        // adopt 판정에 벤더명 대조를 더한다 — 살아있고 **우리 벤더 프로세스일 때만** 복원(pgid 재사용 오탐 차단).
        let vendor_bin =
            crate::convo::Vendor::from_agent(t.agent.as_deref().unwrap_or_default()).bin();
        let alive = |pg: u32| {
            verify::process_group_alive(pg) && verify::process_group_matches(pg, vendor_bin)
        };
        match classify_stale(&t.mode, pgid, alive) {
            StaleAction::Adopt(pgid) => {
                // busy/interrupt 복원 — 재등록해야 convo_history.busy=true, convo_interrupt 동작.
                active.lock().unwrap_or_else(|e| e.into_inner()).insert(
                    t.id,
                    ActiveConvo {
                        pgid: Some(pgid),
                        vendor_bin: vendor_bin.to_string(),
                        started_at: now_ts,
                        last_event_at: now_ts,
                        last_operation: None,
                        interrupted: false,
                    },
                );
                spawn_convo_reaper(
                    app.clone(),
                    pool.clone(),
                    active.clone(),
                    t.id,
                    pgid,
                    vendor_bin.to_string(),
                    now_ts,
                );
                // 상태는 Running 유지 — 프론트가 taskList로 Running 로드(별도 emit 불필요).
            }
            StaleAction::Fail => {
                let _ = db::update_state(pool, t.id, tstate::FAILED, now_ts).await;
                let _ = db::set_convo_pgid(pool, t.id, None).await;
                let _ = app.emit(
                    "task://state",
                    StatePayload {
                        id: t.id,
                        state: tstate::FAILED.to_string(),
                        awaiting_kind: None,
                    },
                );
            }
        }
    }
}

async fn fail_stale_created_direct_tasks(
    pool: &SqlitePool,
    locks: &worktree::DirectCheckoutLocks,
    now_ts: i64,
) -> anyhow::Result<u64> {
    let tasks = db::list_created_direct_tasks(pool).await?;
    let mut failed = 0;
    for task in tasks {
        let repo = Path::new(&task.repo);
        if !worktree::is_git_repository(repo) {
            continue;
        }
        let _claim = locks.acquire(repo).await?;
        failed += db::mark_created_direct_failed(pool, task.id, now_ts).await?;
    }
    Ok(failed)
}

/// KPI Tech(조합 적용 성공률) 관측용 이벤트 상세 — 순수 함수로 분리해 검증 가능.
fn compose_kpi_detail(
    result: &Result<crate::ensemble::ComposeOutcome, crate::ensemble::ComposeError>,
) -> String {
    match result {
        Ok(o) => format!("ok applied={}", o.applied.len()),
        Err(crate::ensemble::ComposeError::ApplyConflict(ids)) => {
            format!("conflict failed={}", ids.len())
        }
        Err(crate::ensemble::ComposeError::ExclusiveGroupViolation(ids)) => {
            format!("exclusive_violation count={}", ids.len())
        }
        Err(crate::ensemble::ComposeError::ProtectedHunkRejected(ids)) => {
            format!("protected_rejected count={}", ids.len())
        }
        Err(e) => format!("error {e}"),
    }
}

/// 작업의 worktree 루트를 DB에서 해석 (파일 API용 — 세션 활성 여부와 무관).
/// 종료 상태(Done/Discarded/Failed)는 worktree가 머지·제거됐을 수 있어 거부.
async fn worktree_root(pool: &SqlitePool, id: i64) -> Result<PathBuf, String> {
    let task = db::get_task(pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    if matches!(
        task.state.as_str(),
        tstate::DONE | tstate::DISCARDED | tstate::FAILED
    ) {
        return Err("종료된 작업의 워크트리는 사용할 수 없습니다".into());
    }
    let p = PathBuf::from(task.worktree_path);
    if !p.is_dir() {
        return Err("워크트리 디렉터리가 존재하지 않습니다".into());
    }
    Ok(p)
}

/// IDE 에디터: worktree 파일 트리 (.git/node_modules/target 등 제외).
#[tauri::command]
pub async fn fs_tree(state: State<'_, AppState>, id: i64) -> Result<Vec<fsapi::FsNode>, String> {
    let pool = pool_of(&state)?;
    let root = worktree_root(&pool, id).await?;
    fsapi::build_tree(&root).map_err(|e| e.to_string())
}

/// IDE 홈: 임의 레포 경로의 파일 트리 (worktree 없이 @멘션 자동완성용).
#[tauri::command]
pub async fn fs_tree_path(path: String) -> Result<Vec<fsapi::FsNode>, String> {
    // 재귀 디렉터리 walk — 대형 레포에서 수백 ms 가능, blocking 풀로 분리.
    tauri::async_runtime::spawn_blocking(move || {
        fsapi::build_tree(std::path::Path::new(&path)).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 파일 브라우저: 디렉터리 한 단계 나열. 로컬은 원격 Runner와 달리 root 제한이 없다
/// (네이티브 폴더 선택 다이얼로그와 같은 범위).
#[tauri::command]
pub fn fs_browse(path: String) -> Result<BrowseResult, String> {
    let start = if path.trim().is_empty() {
        default_browse_root()
    } else {
        std::path::PathBuf::from(&path)
    };
    let dir = start.canonicalize().map_err(|e| e.to_string())?;
    let entries = fsapi::browse_dir(&dir).map_err(|e| e.to_string())?;
    Ok(BrowseResult {
        path: fsapi::display_path(&dir),
        parent: fsapi::browse_parent(&dir),
        entries,
    })
}

/// 경로가 격리 실행이 가능한 git 저장소인지 — 아니면 작업은 격리 없이 직접 모드로 실행된다.
///
/// 커밋이 0개인 저장소(`git init`만 한 상태)는 여기서 false다. 저장소인 것은 맞지만 `HEAD`가
/// 없어 격리도 diff도 성립하지 않으므로, true를 주면 UI가 준비 안내를 숨겨 사용자가 그 상태를
/// 빠져나갈 길이 사라진다. 화면에 노출되는 판정은 `is_ready_repository`가 맡는다.
#[tauri::command]
pub fn git_status_path(path: String) -> bool {
    worktree::is_ready_repository(Path::new(&path))
}

/// 새 작업의 base 후보 — 레포의 로컬 브랜치와 지금 체크아웃한 브랜치.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BranchList {
    /// 레포가 지금 체크아웃한 브랜치. 고르지 않으면 이 값이 base가 된다.
    pub current: String,
    /// 최근 커밋순 로컬 브랜치. `current`도 포함된다.
    pub branches: Vec<String>,
}

/// 레포의 로컬 브랜치 목록 — 홈 컴포저의 base 브랜치 선택에 쓴다.
#[tauri::command]
pub fn git_branches_path(path: String) -> Result<BranchList, String> {
    branch_list(Path::new(&path))
}

fn branch_list(repo: &Path) -> Result<BranchList, String> {
    Ok(BranchList {
        current: worktree::current_branch(repo).map_err(|e| e.to_string())?,
        branches: worktree::list_local_branches(repo).map_err(|e| e.to_string())?,
    })
}

/// 브랜치 피커에서 고른 로컬 브랜치로 메인 체크아웃을 바로 전환하고, 갱신된 목록을 돌려준다.
/// 변경 사항은 직접 실행의 전환과 같은 규칙으로 stash에 보관한다(`checkout_local_branch`).
#[tauri::command]
pub async fn git_checkout_branch_path(
    state: State<'_, AppState>,
    path: String,
    branch: String,
) -> Result<BranchList, String> {
    let pool = pool_of(&state)?;
    checkout_main_branch(&pool, &state.direct_repo_locks, Path::new(&path), &branch).await
}

/// 폴더를 git 저장소로 초기화한다(현재 내용을 초기 커밋으로). 이미 저장소면 멱등하게 통과.
#[tauri::command]
pub fn git_init_path(path: String) -> Result<bool, String> {
    worktree::init_repository(Path::new(&path)).map_err(|e| e.to_string())?;
    Ok(true)
}

/// 파일 브라우저 시작점 — 로컬은 홈 디렉터리 하나.
#[tauri::command]
pub fn fs_roots() -> Vec<String> {
    vec![fsapi::display_path(&default_browse_root())]
}

fn default_browse_root() -> std::path::PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// 변경 계열 파일 브라우저 커맨드가 공통으로 통과하는 문 — **쓰기가 일어나는 경로 전부**를 넘긴다.
///
/// 반환한 claim은 **조작이 끝날 때까지 들고 있어야** 한다. drop되면 점유가 풀려
/// 승인·폐기가 그 사이에 끼어든다.
///
/// 경로를 슬라이스로 받는 이유: `claim_finalization`은 재진입 불가다
/// (`review_ops/claims.rs:79` — finalization은 자기 자신과도 배타적). 복사처럼 두 경로가
/// 같은 워크트리를 가리키면 경로마다 claim할 때 두 번째가 스스로를 막는다. 소유 작업을
/// 먼저 합쳐 dedup한 뒤 작업당 한 번만 claim한다.
async fn assert_paths_mutable(
    state: &State<'_, AppState>,
    targets: &[&Path],
) -> Result<Vec<crate::review_ops::ReviewClaim>, String> {
    let pool = pool_of(state)?;
    let mut owners: Vec<fsapi::guard::OwnerTask> = Vec::new();
    for target in targets {
        // 판정 전에 정규화한다. `owner_tasks`는 워크트리 루트만 canonicalize하므로,
        // 대상이 심볼릭 경유 별칭(macOS `/var` → `/private/var`)이면 prefix 비교가 빗나가
        // 워크트리 안 파일이 밖으로 보인다 — 가드 미탐이 된다.
        let canonical = target.canonicalize().map_err(|e| e.to_string())?;
        let found = fsapi::guard::owner_tasks(&pool, &canonical)
            .await
            .map_err(|e| e.to_string())?;
        for owner in found {
            if !owners.iter().any(|seen| seen.id == owner.id) {
                owners.push(owner);
            }
        }
    }
    let claims = owners
        .iter()
        .map(|owner| state.review_claims.claim_finalization(owner.id))
        .collect::<Result<Vec<_>, _>>()?;
    fsapi::guard::assert_owners_mutable(&pool, &owners)
        .await
        .map_err(|e| e.to_string())?;
    Ok(claims)
}

/// 파일 브라우저: 새 파일. 가드 기준은 **부모 폴더**(PRD F-02).
#[tauri::command]
pub async fn fs_create_file(
    state: State<'_, AppState>,
    dir: String,
    name: String,
) -> Result<String, String> {
    let dir = PathBuf::from(dir);
    let _claims = assert_paths_mutable(&state, &[&dir]).await?;
    fsapi::mutate::create_file(&dir, &name)
        .map(|p| fsapi::display_path(&p))
        .map_err(|e| e.to_string())
}

/// 파일 브라우저: 새 폴더. 가드 기준은 **부모 폴더**(PRD F-02).
#[tauri::command]
pub async fn fs_create_dir(
    state: State<'_, AppState>,
    dir: String,
    name: String,
) -> Result<String, String> {
    let dir = PathBuf::from(dir);
    let _claims = assert_paths_mutable(&state, &[&dir]).await?;
    fsapi::mutate::create_dir(&dir, &name)
        .map(|p| fsapi::display_path(&p))
        .map_err(|e| e.to_string())
}

/// 파일 브라우저: 이름 변경. 가드 기준은 **대상 자신**(PRD F-03).
#[tauri::command]
pub async fn fs_rename(
    state: State<'_, AppState>,
    path: String,
    name: String,
) -> Result<String, String> {
    let path = PathBuf::from(path);
    let _claims = assert_paths_mutable(&state, &[&path]).await?;
    fsapi::mutate::rename(&path, &name)
        .map(|p| fsapi::display_path(&p))
        .map_err(|e| e.to_string())
}

/// 파일 브라우저: 휴지통으로 이동. 가드 기준은 **대상 자신**(PRD F-04).
/// 영구 삭제 커맨드는 두지 않는다(설계 0024 D3).
#[tauri::command]
pub async fn fs_trash(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    let _claims = assert_paths_mutable(&state, &[&path]).await?;
    fsapi::mutate::trash(&path).map_err(|e| e.to_string())
}

/// 파일 브라우저: 복제. 가드 기준은 **원본과 대상 양쪽**(PRD F-05, 계획 DR-P1) —
/// 원본이 워크트리 밖이어도 대상이 안이면 워크트리가 오염된다.
#[tauri::command]
pub async fn fs_copy(
    state: State<'_, AppState>,
    src: String,
    dest_dir: String,
) -> Result<String, String> {
    let src = PathBuf::from(src);
    let dest_dir = PathBuf::from(dest_dir);
    let _claims = assert_paths_mutable(&state, &[&src, &dest_dir]).await?;
    fsapi::mutate::copy_into(&src, &dest_dir)
        .map(|p| fsapi::display_path(&p))
        .map_err(|e| e.to_string())
}

/// 파일 브라우저: 외부 터미널을 그 폴더에서 연다. 파일을 받으면 부모 폴더를 연다.
/// 읽기 계열이라 가드가 없다(PRD F-01).
#[tauri::command]
pub fn fs_open_terminal(path: String) -> Result<(), String> {
    let p = PathBuf::from(path);
    let dir = if p.is_dir() {
        p.clone()
    } else {
        p.parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| "상위 디렉터리가 없습니다".to_string())?
    };
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-a")
            .arg("Terminal")
            .arg(&dir)
            .status()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = dir;
        Err("이 플랫폼에서는 터미널 열기를 지원하지 않습니다".to_string())
    }
}

/// `fs_browse` 응답 — Runner의 `/v1/files/browse`와 같은 형태로 맞춰 프런트가 소스에
/// 무관하게 같은 타입을 쓴다.
#[derive(serde::Serialize)]
pub struct BrowseResult {
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<fsapi::DirEntryInfo>,
}

/// IDE 에디터: 파일 읽기 (경로는 worktree 하위로 제한). 종류(텍스트/이미지/바이너리/초과)는
/// `FileContent::kind`로 구분되며, 파일이 아니거나 접근이 차단된 경우만 에러.
#[tauri::command]
pub async fn fs_read(
    state: State<'_, AppState>,
    id: i64,
    path: String,
) -> Result<fsapi::FileContent, String> {
    let pool = pool_of(&state)?;
    let root = worktree_root(&pool, id).await?;
    fsapi::read_file(&root, &path).map_err(|e| e.to_string())
}

/// IDE 에디터: 파일 저장 (덮어쓰기). 새 mtime(ms) 반환 — 외부 변경 감지용.
#[tauri::command]
pub async fn fs_write(
    state: State<'_, AppState>,
    id: i64,
    path: String,
    content: String,
) -> Result<i64, String> {
    let pool = pool_of(&state)?;
    let root = worktree_root(&pool, id).await?;
    fsapi::write_file(&root, &path, &content).map_err(|e| e.to_string())
}

/// IDE 에디터: 커서 심볼의 정의/구현/사용처 위치 (JetBrains ⌘B 계열).
///
/// `text`는 에디터의 **현재 버퍼**다 — 저장 전 편집도 그대로 반영해 좌표가 어긋나지 않게
/// 요청 직전에 서버로 밀어넣는다. `line`/`column`은 Monaco 좌표(1-based).
#[tauri::command]
pub async fn lsp_goto(
    state: State<'_, AppState>,
    id: i64,
    path: String,
    text: String,
    line: u32,
    column: u32,
    kind: String,
) -> Result<Vec<crate::lspclient::LspTarget>, String> {
    let kind = crate::lspclient::GotoKind::parse(&kind)?;
    let pool = pool_of(&state)?;
    let root = worktree_root(&pool, id).await?;
    // worktree 밖 경로 차단은 fs_read와 같은 규칙을 쓴다.
    fsapi::safe_join(&root, &path).map_err(|e| e.to_string())?;
    let lsp = Arc::clone(&state.lsp);
    lsp.goto(id, &root, &path, &text, line, column, kind).await
}

/// IDE 에디터: 시맨틱 토큰 — 클래스·함수·변수를 **구조적으로** 갈라 색을 준다.
///
/// Monarch(정규식 렉서)는 `foo`가 변수인지 함수인지 클래스인지 알 수 없다. VS Code가
/// 그것을 아는 이유는 언어 서버의 시맨틱 토큰을 받기 때문이고, 이 커맨드가 그 통로다.
///
/// 서버가 지원하지 않으면 `None`이다 — 에러가 아니다.
#[tauri::command]
pub async fn lsp_semantic_tokens(
    state: State<'_, AppState>,
    id: i64,
    path: String,
    text: String,
) -> Result<Option<crate::lspclient::SemanticTokens>, String> {
    let wt = task_worktree_snapshot(&state, id)?;
    let lsp = Arc::clone(&state.lsp);
    lsp.semantic_tokens(id, &wt.path, &path, &text).await
}

/// IDE 에디터: 이 파일에서 정의 이동을 쓸 수 있는지 (서버 미설치·미지원 언어 안내용).
#[tauri::command]
pub async fn lsp_status(
    state: State<'_, AppState>,
    id: i64,
    path: String,
) -> Result<crate::lspclient::LspStatus, String> {
    let pool = pool_of(&state)?;
    let root = worktree_root(&pool, id).await?;
    Ok(crate::lspclient::status_for(&root, &path))
}

/// IDE: worktree 하위 rel 경로를 절대경로 문자열로 해석 (opener로 기본앱/Finder 열기용).
/// worktree 밖/심볼릭은 `safe_join`이 차단.
#[tauri::command]
pub async fn resolve_abs_path(
    state: State<'_, AppState>,
    id: i64,
    path: String,
) -> Result<String, String> {
    let pool = pool_of(&state)?;
    let root = worktree_root(&pool, id).await?;
    let abs = fsapi::safe_join(&root, &path).map_err(|e| e.to_string())?;
    Ok(abs.to_string_lossy().into_owned())
}

/// 검증 게이트: 해석된 명령 미리보기 (UI가 표시 + 레포별 첫 실행 확인용).
#[tauri::command]
pub async fn verify_spec(
    state: State<'_, AppState>,
    id: i64,
) -> Result<review_ops::verify::VerifyPreview, String> {
    let pool = pool_of(&state)?;
    let root = worktree_root(&pool, id).await?;
    review_ops::verify::preview(&pool, id, &root).await
}

/// 검증 게이트: worktree에서 빌드/테스트 실행 → 증거 + 게이트 + 영속.
#[tauri::command]
pub async fn task_verify(
    state: State<'_, AppState>,
    id: i64,
    preview_token: String,
) -> Result<verify::VerifyReport, String> {
    let pool = pool_of(&state)?;
    let root = worktree_root(&pool, id).await?;
    review_ops::verify::run(pool, state.review_claims.clone(), id, root, preview_token).await
}


/// Capsule 조립 (read-only; 종료 상태도 허용) — task_capsule/capsule_inject 공용.
async fn assemble_capsule(pool: &SqlitePool, id: i64) -> Result<capsule::Capsule, String> {
    let task = db::get_task(pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;

    // diff (best-effort — 종료 상태면 worktree가 없을 수 있음).
    let wt = worktree_from_task(&task);
    let (changed, diff_stat) = if wt.path.is_dir() {
        let files = wt.diff_detailed().unwrap_or_default();
        let changed = files.iter().map(|f| f.path.clone()).collect::<Vec<_>>();
        // git 실패는 빈값이 아니라 표면화(오해 소지 있는 "변경 없음" 방지).
        let stat = wt
            .diff_stat()
            .unwrap_or_else(|e| format!("(diff 불가: {e})"));
        (changed, stat)
    } else {
        (Vec::new(), String::new())
    };
    let has_diff = !changed.is_empty();

    let ev = db::get_evidence(pool, id).await.ok().flatten();
    let current_ready = crate::task_results::refresh_evidence(pool, id).await?;
    let evidence_ready = ev.as_ref().map(|e| e.ready && current_ready);
    let evidence_summary = ev.as_ref().map(|e| {
        format!(
            "{} passed / {} failed (ready={})",
            e.passed, e.failed, e.ready
        )
    });

    let recent = db::recent_events(pool, id, 8)
        .await
        .unwrap_or_default()
        .iter()
        .map(|e| match &e.detail {
            Some(d) => format!("{}: {}", e.kind, d),
            None => e.kind.clone(),
        })
        .collect::<Vec<_>>();

    let manual_acceptance_pending = task
        .goal_contract
        .as_deref()
        .is_some_and(|contract| !contract.acceptance.is_empty());
    let next_action = capsule::infer_next_action(has_diff, evidence_ready, manual_acceptance_pending);

    // 캔버스도 브리핑처럼 best-effort — 원장 조회 실패가 핸드오프 전체를 막지 않는다.
    let canvas = db::list_convo_events(pool, id)
        .await
        .map(|events| crate::convo::canvas::latest_from_events(&events))
        .unwrap_or_default();

    Ok(capsule::Capsule {
        task_id: id,
        instruction: task.instruction,
        goal_contract: task.goal_contract.map(|contract| contract.0),
        branch: task.branch,
        state: task.state,
        changed,
        diff_stat,
        evidence_ready,
        evidence_summary,
        recent,
        next_action,
        canvas,
    })
}

/// Capsule 브리핑 (read-only).
#[tauri::command]
pub async fn task_capsule(state: State<'_, AppState>, id: i64) -> Result<capsule::Capsule, String> {
    let pool = pool_of(&state)?;
    assemble_capsule(&pool, id).await
}

/// Capsule을 worktree 컨텍스트 파일에 주입하는 본체 — `State`를 벗겨 다른 커맨드가 재사용한다.
async fn inject_capsule_block(pool: &SqlitePool, id: i64) -> Result<Vec<String>, String> {
    let task = db::get_task(pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    let wt = worktree_from_task(&task);
    if !wt.path.is_dir() {
        return Err("워크트리 디렉터리가 없어 저장할 수 없습니다".into());
    }
    let cap = assemble_capsule(pool, id).await?;
    let block = capsule::render_capsule_block(&cap);
    let targets = crate::projector::project_targets();
    crate::projector::write_block(
        &wt.path,
        &targets,
        capsule::CAP_START,
        capsule::CAP_END,
        &block,
    )
    .map_err(|e| e.to_string())?;
    let _ = db::append_event(pool, id, "capsule", Some("injected"), now()).await;
    Ok(targets.iter().map(|s| s.to_string()).collect())
}

/// Capsule을 worktree 컨텍스트 파일(AGENTS.md)에 주입 — **다음 세션용**.
#[tauri::command]
pub async fn capsule_inject(state: State<'_, AppState>, id: i64) -> Result<Vec<String>, String> {
    let pool = pool_of(&state)?;
    inject_capsule_block(&pool, id).await
}

/// 컨텍스트를 비우고 캡슐만 들고 이어간다 — 단계가 바뀌었을 때의 의도적 절단.
///
/// 문서화를 마치고 구현에 들어갈 때, 앞선 탐색과 시행착오는 이미 산출물로 압축돼 있다.
/// 그것을 다음 단계까지 들고 가면 토큰만 먹는다. compact와 다른 점은 **경계를 사람이 긋고
/// 남길 것을 이미 확정해 뒀다**는 것이다 — 무엇이 버려질지 추측할 필요가 없다.
///
/// 순서가 계약이다. **캡슐 주입이 실패하면 세션을 끊지 않는다** — 핸드오프 없이 컨텍스트만
/// 날아가는 것이 이 기능이 만들 수 있는 최악의 결과다. 되돌릴 방법도 없다.
///
/// worktree·컨텍스트 파일·대화 이력을 건드리지 않는다. `convo_rewind`가 `restore_to_checkpoint`로
/// 코드까지 되감는 것과 다르다 — 여기서 지우는 것은 **벤더 세션 하나뿐**이다.
///
/// 캡슐은 `tasks.pending_capsule`에 적히고 다음 턴의 프롬프트가 앞에 붙여 읽는다(ADR 0170).
/// 컨텍스트 파일에 쓰던 이전 방식은 제거 경로가 없어 영구히 남았고, tracked 파일이라 diff와
/// 턴 종료 판정까지 흔들었다.
#[tauri::command]
pub async fn convo_context_reset(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    // 턴이 도는 중에는 끊을 수 없다. 턴 에필로그가 종료 시 `set_convo_session`으로 세션 id를
    // **무조건** 다시 써 넣으므로(이 파일의 턴 완료 처리), 여기서 지워도 곧 되살아난다 —
    // 사용자에게는 "새 대화로 시작한다"고 알린 뒤다. `convo_rewind`가 같은 이유로 막는다.
    let _reservation = reserve_convo_switch(state.convo_active.clone(), id)?;
    let event = context_reset_inner(&pool, id).await?;
    // 원장에만 쓰면 구분선은 이 작업을 떠났다 돌아오기 전까지 화면에 없다. 그 사이에 이어서
    // 말하면 위아래가 하나의 대화로 읽히는데 에이전트는 위를 기억하지 못한다 — 이 기능이
    // 막으려던 바로 그 상태다. 벤더 스트림과 같은 채널로 흘려 즉시 그리게 한다.
    let _ = app.emit("convo://event", ConvoPayload { id, speaker: None, event });
    Ok(())
}

/// `convo_context_reset`의 본체 — `State`를 벗겨 테스트가 순서 계약을 직접 검증한다.
pub(crate) async fn context_reset_inner(
    pool: &SqlitePool,
    id: i64,
) -> Result<crate::convo::ConvoEvent, String> {
    // 끝난 작업에는 걸지 않는다. 이어질 턴이 없으므로 캡슐을 적어도 아무도 읽지 않고,
    // 미소비 캡슐만 행에 남는다. `convo_rewind`가 같은 세 상태를 거부하는 것과 같은 이유다.
    //
    // 파일에 쓰던 시절에는 이 가드가 더 무거웠다 — direct 모드(worktree_path == repo)에서
    // 폐기된 작업의 캡슐이 메인 체크아웃의 CLAUDE.md에 영구히 써졌다(ADR 0170).
    let task = db::get_task(pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
    let state = task.state.clone();
    if matches!(
        state.as_str(),
        tstate::DONE | tstate::DISCARDED | tstate::FAILED
    ) {
        return Err(format!("종료된 작업의 컨텍스트는 비울 수 없습니다: {state}"));
    }
    // worktree가 없으면 거부한다. 파일에 쓰던 시절에는 주입이 알아서 실패해 **우연히** 막혔지만,
    // 이제는 `assemble_capsule`이 전 구간 best-effort라 조용히 통과한다 — 변경 파일도 diff도 빈
    // 반쪽 캡슐을 만들어 놓고 세션만 끊는 것이다. 그 작업은 `convo_send`의 가드에 막혀 다음 턴을
    // 시작할 수도 없으므로, 끊어서 얻는 것 없이 컨텍스트만 잃는다.
    if !worktree_from_task(&task).path.is_dir() {
        return Err(worktree::missing_worktree_error(&task.worktree_path));
    }
    // `?`가 여기서 끊는 것이 계약의 전부다. 저장에 실패했는데 아래로 내려가면 핸드오프 없이
    // 세션만 사라진다 — 되돌릴 수 없고, 사용자는 방금 무엇을 잃었는지도 알 수 없다.
    //
    // 저장처는 컨텍스트 파일이 아니라 `tasks.pending_capsule`이다(ADR 0170). 파일에 쓰면
    // 제거 경로가 없어 영구히 남고, tracked 파일이라 diff와 턴 종료 판정까지 흔든다.
    let capsule = assemble_capsule(pool, id).await?;
    let block = capsule::render_capsule_block(&capsule);
    db::set_pending_capsule(pool, id, &block)
        .await
        .map_err(|e| e.to_string())?;
    db::clear_convo_session(pool, id)
        .await
        .map_err(|e| e.to_string())?;
    let ts = now();
    let text = "컨텍스트를 비웠습니다. 다음 턴은 새 대화로 시작하며, 지금까지의 작업 요약이 첫 메시지에 함께 전달됩니다.".to_string();
    let event = crate::convo::ConvoEvent::ContextCleared { text };
    // 구분선을 못 남기면 사용자는 대화가 이어진다고 믿는다. 세션은 이미 끊긴 뒤라 되돌릴 수도
    // 없으므로, 이 실패는 삼키지 않고 올린다 — 절단 자체보다 경계의 상실이 더 위험하다.
    let encoded = serde_json::to_string(&event).map_err(|e| e.to_string())?;
    db::append_convo_event(pool, id, &encoded, ts)
        .await
        .map_err(|e| e.to_string())?;
    let _ = db::append_event(pool, id, "context_cleared", Some("pending_capsule"), ts).await;
    Ok(event)
}

/// 컨텍스트 절단의 순서 계약 — 주입이 실패하면 세션은 살아남는다.
#[cfg(test)]
mod context_reset_tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// 프로세스 전용 루트 아래에 둔다 — `std::env::temp_dir()`에 pid 이름을 쓰면 macOS의 pid
    /// 재사용 + 남은 `-wal`로 옛 스키마가 되살아난다(testtmp.rs, 이슈 #144·#153).
    fn temp_paths() -> (String, std::path::PathBuf) {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = crate::testtmp::dir();
        let db = root.join(format!("ctx-reset-{n}.db"));
        let wt = root.join(format!("ctx-reset-wt-{n}"));
        (db.to_string_lossy().into_owned(), wt)
    }

    async fn task_at(pool: &SqlitePool, worktree: &str) -> i64 {
        let id = db::insert_task(
            pool,
            "/r",
            "b",
            "main",
            worktree,
            "i",
            Some("claude"),
            None,
            "conversation",
            1,
        )
        .await
        .unwrap();
        db::set_convo_session(pool, id, "sess-abc").await.unwrap();
        id
    }

    #[tokio::test]
    async fn capsule_failure_leaves_the_session_alive() {
        let (path, _) = temp_paths();
        let pool = db::init_pool(&path).await.unwrap();
        // worktree 경로가 실재하지 않으므로 캡슐을 남길 수 없다.
        let id = task_at(&pool, "/nonexistent/praxis-worktree").await;

        let result = context_reset_inner(&pool, id).await;

        assert!(result.is_err(), "저장 실패는 그대로 올라와야 한다");
        let task = db::get_task(&pool, id).await.unwrap().unwrap();
        assert_eq!(
            task.convo_session_id.as_deref(),
            Some("sess-abc"),
            "핸드오프를 못 남겼으면 컨텍스트를 끊어서는 안 된다"
        );
        assert_eq!(
            task.pending_capsule, None,
            "실패했으면 반쪽 캡슐도 남기지 않는다"
        );
        let events = db::list_convo_events(&pool, id).await.unwrap();
        assert!(
            !events.iter().any(|e| e.contains("context_cleared")),
            "끊지 않았으면 구분선도 긋지 않는다 — 화면과 실제가 정반대가 된다"
        );
    }

    #[tokio::test]
    async fn success_stores_the_capsule_and_writes_no_file() {
        let (path, wt) = temp_paths();
        std::fs::create_dir_all(&wt).unwrap();
        let pool = db::init_pool(&path).await.unwrap();
        let id = task_at(&pool, wt.to_str().unwrap()).await;

        let event = context_reset_inner(&pool, id).await.unwrap();

        assert!(
            matches!(event, crate::convo::ConvoEvent::ContextCleared { .. }),
            "emit용 이벤트를 그대로 돌려줘야 프론트가 즉시 그린다"
        );
        let task = db::get_task(&pool, id).await.unwrap().unwrap();
        assert_eq!(task.convo_session_id, None, "세션을 놓아야 새 대화로 시작한다");
        let capsule = task
            .pending_capsule
            .expect("캡슐이 행에 남아야 다음 턴이 읽는다");
        // 블록이 온전히 닫혀야 하고(경계가 없으면 프롬프트에서 어디까지가 캡슐인지 알 수 없다),
        // 그 안에 작업 지시가 실려야 핸드오프가 성립한다. 픽스처의 instruction은 "i"다.
        assert!(
            capsule.starts_with(crate::capsule::CAP_START)
                && capsule.trim_end().ends_with(crate::capsule::CAP_END)
                && capsule.contains("- 작업: i\n"),
            "캡슐은 닫힌 블록에 작업 지시를 담아야 한다: {capsule}"
        );
        // 이 기능의 핵심 계약이다 — 컨텍스트 파일을 **하나도** 건드리지 않는다(ADR 0170).
        // 파일에 쓰면 제거 경로가 없어 영구히 남고, tracked라 diff와 턴 종료 판정까지 흔든다.
        for target in crate::projector::all_targets() {
            assert!(
                !wt.join(target).exists(),
                "{target}을 만들면 안 된다 — 캡슐은 DB로만 간다"
            );
        }
        let events = db::list_convo_events(&pool, id).await.unwrap();
        assert!(
            events.iter().any(|e| e.contains("context_cleared")),
            "구분선이 원장에 없으면 재진입 후 경계가 사라진다"
        );
        let _ = std::fs::remove_dir_all(&wt);
    }

    /// peek는 지우지 않는다. 이 계약이 깨지면 벤더 스폰이 실패하는 순간 캡슐이 사라지고,
    /// 세션은 이미 끊긴 뒤라 되돌릴 방법이 없다(ADR 0170 §결정3).
    #[tokio::test]
    async fn peeking_does_not_consume_the_capsule() {
        let (path, wt) = temp_paths();
        std::fs::create_dir_all(&wt).unwrap();
        let pool = db::init_pool(&path).await.unwrap();
        let id = task_at(&pool, wt.to_str().unwrap()).await;
        context_reset_inner(&pool, id).await.unwrap();

        let first = db::peek_pending_capsule(&pool, id).await.unwrap();
        let second = db::peek_pending_capsule(&pool, id).await.unwrap();

        assert!(first.is_some(), "절단 직후에는 캡슐이 있어야 한다");
        assert_eq!(first, second, "읽었다고 사라지면 실패한 턴이 핸드오프를 태운다");
        let _ = std::fs::remove_dir_all(&wt);
    }

    /// 지우는 것은 세션 확립 시점뿐이다 — "세션이 생겼다 = 캡슐이 도착했다".
    #[tokio::test]
    async fn establishing_a_session_consumes_the_capsule() {
        let (path, wt) = temp_paths();
        std::fs::create_dir_all(&wt).unwrap();
        let pool = db::init_pool(&path).await.unwrap();
        let id = task_at(&pool, wt.to_str().unwrap()).await;
        context_reset_inner(&pool, id).await.unwrap();

        db::set_convo_session(&pool, id, "sess-next").await.unwrap();

        let task = db::get_task(&pool, id).await.unwrap().unwrap();
        assert_eq!(
            task.convo_session_id.as_deref(),
            Some("sess-next"),
            "새 세션은 그대로 기록돼야 한다"
        );
        assert_eq!(
            task.pending_capsule, None,
            "세션이 생겼으면 캡슐은 도착한 것이다 — 다음 턴에 또 붙으면 안 된다"
        );
        let _ = std::fs::remove_dir_all(&wt);
    }

    #[tokio::test]
    async fn finished_tasks_are_refused() {
        // 이어질 턴이 없는 작업에 캡슐을 남기면 아무도 읽지 않는다. worktree는 실재하게 두어
        // "worktree 가드로 우연히 막히는" 것이 아니라 상태 검사가 막는다는 것을 보인다.
        let (path, wt) = temp_paths();
        std::fs::create_dir_all(&wt).unwrap();
        let pool = db::init_pool(&path).await.unwrap();
        let id = task_at(&pool, wt.to_str().unwrap()).await;
        db::update_state(&pool, id, tstate::DONE, 2).await.unwrap();

        let result = context_reset_inner(&pool, id).await;

        assert!(result.is_err(), "종료된 작업은 거부해야 한다");
        let task = db::get_task(&pool, id).await.unwrap().unwrap();
        assert_eq!(task.pending_capsule, None, "거부했으면 캡슐도 남기지 않는다");
        let _ = std::fs::remove_dir_all(&wt);
    }
}

/// 워크트리 언어 기반 LSP 자동주입 토글 조회. 미설정 시 기본 켜짐(자동주입이 기능 취지).
#[tauri::command]
pub async fn lsp_autoinject_get(state: State<'_, AppState>) -> Result<bool, String> {
    let pool = pool_of(&state)?;
    Ok(db::get_setting(&pool, "lsp_autoinject")
        .await
        .ok()
        .flatten()
        .as_deref()
        != Some("false"))
}

/// 워크트리 언어 기반 LSP 자동주입 토글 설정.
#[tauri::command]
pub async fn lsp_autoinject_set(state: State<'_, AppState>, on: bool) -> Result<(), String> {
    let pool = pool_of(&state)?;
    db::set_setting(&pool, "lsp_autoinject", if on { "true" } else { "false" })
        .await
        .map_err(|e| e.to_string())
}

/// 프로젝트별 워크트리 오버라이드 설정 키 — 전역 키와 같은 `settings` 테이블에 네임스페이스로 공존한다.
/// repo는 절대경로 원문을 그대로 쓴다(경로에 `:`가 있어도 prefix 길이로 자르므로 무해).
const USE_WORKTREE_PREFIX: &str = "use_worktree:";

fn use_worktree_key(repo: &str) -> String {
    format!("{USE_WORKTREE_PREFIX}{repo}")
}

/// 프로젝트 오버라이드만 읽는다 — 미설정이면 None(= "전역 기본 따름").
async fn use_worktree_override(pool: &SqlitePool, repo: &str) -> Option<bool> {
    db::get_setting(pool, &use_worktree_key(repo))
        .await
        .ok()
        .flatten()
        .map(|v| v != "false")
}

/// `use_worktree` 유효값 읽기 — 프로젝트 오버라이드 → 전역 기본 → 켜짐 순으로 폴백한다.
/// (격리가 기본 동작, 안전 우선.)
async fn use_worktree_on(pool: &SqlitePool, repo: &str) -> bool {
    if let Some(on) = use_worktree_override(pool, repo).await {
        return on;
    }
    db::get_setting(pool, "use_worktree")
        .await
        .ok()
        .flatten()
        .as_deref()
        != Some("false")
}

/// base 최신화 토글의 유효값. 미설정 시 켜짐.
async fn refresh_base_on(pool: &SqlitePool, repo: &str) -> bool {
    if !repo.is_empty() {
        if let Ok(Some(v)) = db::get_setting(pool, &format!("refresh_base:{repo}")).await {
            return v != "false";
        }
    }
    db::get_setting(pool, "refresh_base")
        .await
        .ok()
        .flatten()
        .as_deref()
        != Some("false")
}

/// base 최신화 토글 조회. `repo`를 주면 그 프로젝트의 유효값, 생략하면 전역 기본.
#[tauri::command]
pub async fn refresh_base_get(
    state: State<'_, AppState>,
    repo: Option<String>,
) -> Result<bool, String> {
    let pool = pool_of(&state)?;
    Ok(refresh_base_on(&pool, repo.as_deref().unwrap_or("")).await)
}

/// base 최신화 토글 설정.
#[tauri::command]
pub async fn refresh_base_set(
    state: State<'_, AppState>,
    on: bool,
    repo: Option<String>,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    let key = match repo.as_deref() {
        Some(r) if !r.is_empty() => format!("refresh_base:{r}"),
        _ => "refresh_base".to_string(),
    };
    db::set_setting(&pool, &key, if on { "true" } else { "false" })
        .await
        .map_err(|e| e.to_string())
}

/// 워크트리 격리 토글 조회. `repo`를 주면 그 프로젝트의 유효값(오버라이드 반영),
/// 생략하면 전역 기본값. 미설정 시 기본 켜짐.
#[tauri::command]
pub async fn use_worktree_get(
    state: State<'_, AppState>,
    repo: Option<String>,
) -> Result<bool, String> {
    let pool = pool_of(&state)?;
    Ok(use_worktree_on(&pool, repo.as_deref().unwrap_or("")).await)
}

/// 프로젝트 오버라이드 조회 — `None`이면 전역 기본을 따르는 상태(UI의 3-state 표시용).
#[tauri::command]
pub async fn use_worktree_override_get(
    state: State<'_, AppState>,
    repo: String,
) -> Result<Option<bool>, String> {
    let pool = pool_of(&state)?;
    Ok(use_worktree_override(&pool, &repo).await)
}

/// 워크트리 격리 토글 설정 — `repo`를 주면 그 프로젝트만, 생략하면 전역 기본을 바꾼다.
/// 꺼도 앙상블/외부기원(봇·크론) 작업은 항상 격리한다
/// (`create_task_internal`에서 안전상 강제, 이 설정과 무관).
#[tauri::command]
pub async fn use_worktree_set(
    state: State<'_, AppState>,
    on: bool,
    repo: Option<String>,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    let key = match repo.as_deref() {
        Some(r) if !r.is_empty() => use_worktree_key(r),
        _ => "use_worktree".to_string(),
    };
    db::set_setting(&pool, &key, if on { "true" } else { "false" })
        .await
        .map_err(|e| e.to_string())
}

/// 프로젝트 오버라이드 해제 — 이후 그 프로젝트는 다시 전역 기본을 따른다.
#[tauri::command]
pub async fn use_worktree_override_clear(
    state: State<'_, AppState>,
    repo: String,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    db::delete_setting(&pool, &use_worktree_key(&repo))
        .await
        .map_err(|e| e.to_string())
}

/// UI/코드 2-레지스터 폰트 설정. `*_family`가 빈 문자열이면 미설정(기본 폴백 체인 사용).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontSettings {
    pub ui_family: String,
    pub ui_size: u32,
    pub code_family: String,
    pub code_size: u32,
}

/// 폰트 설정 조회 — 미설정 키는 기본값(패밀리 "", ui 14px, code 13px)으로 채운다.
#[tauri::command]
pub async fn font_settings_get(state: State<'_, AppState>) -> Result<FontSettings, String> {
    let pool = pool_of(&state)?;
    let str_setting = |v: Option<String>, default: &str| v.unwrap_or_else(|| default.to_string());
    let num_setting =
        |v: Option<String>, default: u32| v.and_then(|s| s.parse::<u32>().ok()).unwrap_or(default);
    Ok(FontSettings {
        ui_family: str_setting(
            db::get_setting(&pool, "font_ui_family")
                .await
                .ok()
                .flatten(),
            "",
        ),
        ui_size: num_setting(
            db::get_setting(&pool, "font_ui_size").await.ok().flatten(),
            14,
        ),
        code_family: str_setting(
            db::get_setting(&pool, "font_code_family")
                .await
                .ok()
                .flatten(),
            "",
        ),
        code_size: num_setting(
            db::get_setting(&pool, "font_code_size")
                .await
                .ok()
                .flatten(),
            13,
        ),
    })
}

/// 폰트 설정 저장 — 크기는 안전 범위로 clamp(code 10..=24, ui 12..=16) 후 4키를 저장.
#[tauri::command]
pub async fn font_settings_set(
    state: State<'_, AppState>,
    settings: FontSettings,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    let ui_size = settings.ui_size.clamp(12, 16);
    let code_size = settings.code_size.clamp(10, 24);
    db::set_setting(&pool, "font_ui_family", &settings.ui_family)
        .await
        .map_err(|e| e.to_string())?;
    db::set_setting(&pool, "font_ui_size", &ui_size.to_string())
        .await
        .map_err(|e| e.to_string())?;
    db::set_setting(&pool, "font_code_family", &settings.code_family)
        .await
        .map_err(|e| e.to_string())?;
    db::set_setting(&pool, "font_code_size", &code_size.to_string())
        .await
        .map_err(|e| e.to_string())
}

/// 파일 에디터 설정 — 파일 트리의 치수와 Monaco 동작 옵션.
///
/// 폰트 설정(`FontSettings`)과 나눠 두는 이유는 소비자가 다르기 때문이다. 폰트는 터미널과
/// 팝아웃 창까지 구독하지만 이쪽은 에디터 화면만 쓴다. 한 구조체로 합치면 필드를 더할 때마다
/// 무관한 소비자가 딸려온다.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorSettings {
    /// 파일 트리 글자 크기(px). 행 높이·아이콘·들여쓰기는 프런트가 이 값에서 **파생**한다
    /// (`lib/editor-settings.ts`) — 치수를 개별로 저장하면 다시 갈라진다.
    pub tree_font_size: u32,
    pub minimap: bool,
    pub word_wrap: bool,
    pub tab_size: u32,
}

/// 에디터 설정 조회 — 미설정 키는 **지금까지 하드코딩돼 있던 값**으로 채운다
/// (트리 16px, 미니맵 켬, 줄 바꿈 끔, 탭 2). 기본값이 곧 이전 동작이라, 설정을 건드리지 않은
/// 사용자에게는 화면이 그대로다.
#[tauri::command]
pub async fn editor_settings_get(state: State<'_, AppState>) -> Result<EditorSettings, String> {
    let pool = pool_of(&state)?;
    let num_setting =
        |v: Option<String>, default: u32| v.and_then(|s| s.parse::<u32>().ok()).unwrap_or(default);
    // 미설정("키 없음")과 꺼짐("0")은 다르다 — Option을 먼저 가른 뒤에 값을 본다.
    let bool_setting = |v: Option<String>, default: bool| v.map(|s| s == "1").unwrap_or(default);
    Ok(EditorSettings {
        tree_font_size: num_setting(
            db::get_setting(&pool, "editor_tree_font_size")
                .await
                .ok()
                .flatten(),
            16,
        ),
        minimap: bool_setting(
            db::get_setting(&pool, "editor_minimap").await.ok().flatten(),
            true,
        ),
        word_wrap: bool_setting(
            db::get_setting(&pool, "editor_word_wrap")
                .await
                .ok()
                .flatten(),
            false,
        ),
        tab_size: num_setting(
            db::get_setting(&pool, "editor_tab_size").await.ok().flatten(),
            2,
        ),
    })
}

/// 에디터 설정 저장 — 크기는 안전 범위로 clamp(트리 10..=24px, 탭 1..=8).
#[tauri::command]
pub async fn editor_settings_set(
    state: State<'_, AppState>,
    settings: EditorSettings,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    let tree_font_size = settings.tree_font_size.clamp(10, 24);
    let tab_size = settings.tab_size.clamp(1, 8);
    let flag = |on: bool| if on { "1" } else { "0" };
    db::set_setting(&pool, "editor_tree_font_size", &tree_font_size.to_string())
        .await
        .map_err(|e| e.to_string())?;
    db::set_setting(&pool, "editor_minimap", flag(settings.minimap))
        .await
        .map_err(|e| e.to_string())?;
    db::set_setting(&pool, "editor_word_wrap", flag(settings.word_wrap))
        .await
        .map_err(|e| e.to_string())?;
    db::set_setting(&pool, "editor_tab_size", &tab_size.to_string())
        .await
        .map_err(|e| e.to_string())
}

/// 시스템 설치 폰트 목록 (설정 패널이 열릴 때마다 재스캔 — 새 설치 폰트 반영).
#[tauri::command]
pub async fn system_fonts_list() -> Result<Vec<fonts::FontInfo>, String> {
    Ok(fonts::list_system_fonts())
}

fn editor_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    app.get_webview_window(editorwindow::WINDOW_LABEL)
        .ok_or_else(|| "에디터 창을 찾을 수 없습니다".to_string())
}

/// 창이 없으면 `tauri.conf.json`의 같은 설정으로 다시 만든다.
///
/// 창은 부팅 때 한 번 만들어지고 팝인은 숨김이라 보통 살아 있다. 그러나 세션이 없는 채로 닫히거나
/// 웹뷰가 죽은 채로 닫히면 Tauri는 창을 destroy하고, 그 뒤로는 재시작 전까지 팝아웃이 통째로
/// 막혔다("에디터 창을 찾을 수 없습니다"). 같은 label로 만들면 capability(`capabilities/editor.json`)와
/// `editor://ready` 핸드셰이크가 그대로 붙는다.
pub(crate) fn editor_window_or_create<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Result<tauri::WebviewWindow<R>, String> {
    if let Some(window) = app.get_webview_window(editorwindow::WINDOW_LABEL) {
        return Ok(window);
    }
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == editorwindow::WINDOW_LABEL)
        .cloned()
        .ok_or_else(|| "에디터 창 설정을 찾을 수 없습니다".to_string())?;
    tauri::WebviewWindowBuilder::from_config(app, &config)
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())
}

/// 창이 살아 있는지. 메인 창이 focus 실패를 "창이 죽었다"와 "focus만 실패했다"로 가르는 데 쓴다.
#[tauri::command]
pub async fn editor_window_alive(app: AppHandle) -> Result<bool, String> {
    Ok(app.get_webview_window(editorwindow::WINDOW_LABEL).is_some())
}

/// 팝아웃 — 저장된 자리로 되돌린 뒤 창을 보인다.
///
/// 자리가 지금 화면 구성에서 닿지 않으면(듀얼 모니터를 떼고 나온 경우) 복원하지 않는다.
/// 보이지 않는 곳에 뜬 창은 사용자가 되찾을 방법이 없다.
#[tauri::command]
pub async fn editor_window_open(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let window = editor_window_or_create(&app)?;
    let pool = pool_of(&state)?;
    if let Ok(Some(raw)) = db::get_setting(&pool, editorwindow::geometry_key()).await {
        if let Ok(saved) = serde_json::from_str::<editorwindow::Geometry>(&raw) {
            let monitors: Vec<editorwindow::Geometry> = window
                .available_monitors()
                .unwrap_or_default()
                .iter()
                .map(|m| editorwindow::Geometry {
                    x: m.position().x,
                    y: m.position().y,
                    width: m.size().width,
                    height: m.size().height,
                })
                .collect();
            if editorwindow::is_reachable(&saved, &monitors) {
                let _ = window.set_size(tauri::PhysicalSize::new(saved.width, saved.height));
                let _ = window.set_position(tauri::PhysicalPosition::new(saved.x, saved.y));
            }
        }
    }
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}

/// 팝인 — 창을 숨긴다. 닫기와 달리 웹뷰를 살려 두어 다음 팝아웃이 즉시 뜬다.
#[tauri::command]
pub async fn editor_window_hide(app: AppHandle) -> Result<(), String> {
    editor_window(&app)?.hide().map_err(|e| e.to_string())
}

/// 창을 앞으로 가져온다 — 자동 저장이 실패해 사용자의 판단이 필요할 때.
#[tauri::command]
pub async fn editor_window_focus(app: AppHandle) -> Result<(), String> {
    let window = editor_window(&app)?;
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}

/// 창 자리 저장 — 이동·리사이즈가 멎은 뒤 디바운스로 호출된다.
#[tauri::command]
pub async fn editor_window_geometry_save(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let window = editor_window(&app)?;
    let position = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let geometry = editorwindow::Geometry {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
    };
    let pool = pool_of(&state)?;
    let raw = serde_json::to_string(&geometry).map_err(|e| e.to_string())?;
    db::set_setting(&pool, editorwindow::geometry_key(), &raw)
        .await
        .map_err(|e| e.to_string())
}

/// 열린 파일 목록 저장 — 변경 시마다 부른다.
///
/// `editor://closed`는 정상 닫기에서만 오므로 크래시·강제 종료에서는 목록이 사라진다.
/// 이벤트가 아니라 이 저장이 지속성을 책임진다.
#[tauri::command]
pub async fn editor_window_files_save(
    state: State<'_, AppState>,
    task_id: i64,
    open_paths: Vec<String>,
    active_path: Option<String>,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    let value = editorwindow::OpenFilesState {
        open_paths,
        active_path,
    };
    let raw = serde_json::to_string(&value).map_err(|e| e.to_string())?;
    db::set_setting(&pool, &editorwindow::open_files_key(task_id), &raw)
        .await
        .map_err(|e| e.to_string())
}

/// 열린 파일 목록 조회. 저장된 적이 없거나 형식이 깨졌으면 빈 상태로 읽는다 —
/// 목록을 못 읽는 것이 창을 못 여는 이유가 되어서는 안 된다.
#[tauri::command]
pub async fn editor_window_files_load(
    state: State<'_, AppState>,
    task_id: i64,
) -> Result<editorwindow::OpenFilesState, String> {
    let pool = pool_of(&state)?;
    let raw = db::get_setting(&pool, &editorwindow::open_files_key(task_id))
        .await
        .ok()
        .flatten();
    Ok(raw
        .and_then(|r| serde_json::from_str(&r).ok())
        .unwrap_or_default())
}

/// 에이전트(CLI 벤더)별 기본 모델 조회 — 프리셋 3종(claude/codex/agy)을
/// 한 번에 반환. 미설정은 빈 문자열.
#[tauri::command]
pub async fn agent_models_get(
    state: State<'_, AppState>,
) -> Result<HashMap<String, String>, String> {
    let pool = pool_of(&state)?;
    let mut out = HashMap::new();
    for (key, _label) in crate::agent::PRESETS {
        let v = agent_model_of(&pool, key).await.unwrap_or_default();
        out.insert((*key).to_string(), v);
    }
    Ok(out)
}

/// 에이전트(CLI 벤더)별 기본 모델 설정. agent는 프리셋 3종만 허용, model은 trim 후 저장
/// (빈 값 저장 = 해제).
#[tauri::command]
pub async fn agent_model_set(
    state: State<'_, AppState>,
    agent: String,
    model: String,
) -> Result<(), String> {
    if !crate::agent::is_preset(&agent) {
        return Err(format!("알 수 없는 에이전트: {agent}"));
    }
    let pool = pool_of(&state)?;
    db::set_setting(&pool, &format!("model:{}", agent.trim()), model.trim())
        .await
        .map_err(|e| e.to_string())
}

/// 세션 모델 오버라이드 저장의 본체 — `State`를 벗겨 테스트가 직접 부를 수 있게 분리한다.
///
/// `tasks.model`은 이 커맨드가 생기기 전까지 **생성 시점에만 기록되는 값**이었다. 세션 수명 동안
/// 가변으로 바뀌었으므로, 생성 경로(`TaskService`)가 지키던 짝 규칙을 여기서도 지켜야 한다.
pub(crate) async fn set_task_model_checked(
    pool: &SqlitePool,
    id: i64,
    model: &str,
) -> Result<(), String> {
    let task = db::get_task(pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
    let stored_agent = task.agent.as_deref().unwrap_or_default();
    let agent = stored_agent.trim().to_string();
    // 커스텀 에이전트는 `agent_args`의 커스텀 분기가 model을 아예 싣지 않는다 — 저장해도
    // CLI에 닿지 못하므로 애초에 받지 않는다. 형제 커맨드 `agent_model_set`과 같은 게이트다.
    if !crate::agent::is_preset(&agent) {
        return Err(format!("모델을 바꿀 수 없는 에이전트입니다: {agent}"));
    }
    let model = model.trim();
    // 새 모델이 지금의 effort를 지원하지 않으면 effort를 함께 지운다. 거부하는 편이 엄격하지만
    // 세션 헤더에는 effort를 낮출 수단이 없어 막다른 길이 된다 — codex의 모델별 매트릭스가 그렇다.
    // 이 정리를 빠뜨리면 다음 턴이 `-m <새 모델> -c model_reasoning_effort=<옛 effort>`로 나가
    // CLI가 턴 자체를 거부한다.
    let clear_effort = crate::agent::reasoning_effort_override_for_model(
        &agent,
        Some(model),
        task.reasoning_effort.as_deref(),
    )
    .is_err();
    if !db::set_task_model_for_agent(pool, id, stored_agent, model, clear_effort, now())
        .await
        .map_err(|e| e.to_string())?
    {
        return Err("에이전트가 전환되어 모델을 바꾸지 못했습니다".into());
    }
    Ok(())
}

/// 세션 단위 모델 오버라이드 교체 — 다음 resume 턴부터 `--model`로 실린다.
///
/// 진행 중인 턴은 프로세스가 이미 떠 있으므로 바뀌지 않는다. 빈 문자열이면 오버라이드를 지우고
/// 설정의 벤더 기본으로 되돌린다 — `agent_args`가 빈 모델을 걸러내므로 그대로 안전하다.
/// 이 명령은 모델만 바꾼다. 에이전트 전환은 `task_agent_set`의 새 세션·핸드오프 경로를 쓴다.
#[tauri::command]
pub async fn task_model_set(
    state: State<'_, AppState>,
    id: i64,
    model: String,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    set_task_model_checked(&pool, id, &model).await
}

const AGENT_SWITCH_DIALOGUE_EVENTS: i64 = 64;
const AGENT_SWITCH_DIALOGUE_LINES: usize = 12;
const AGENT_SWITCH_DIALOGUE_CHARS: usize = 6_000;
const AGENT_SWITCH_LINE_CHARS: usize = 1_200;

fn supports_convo_agent_switch(agent: &str) -> bool {
    matches!(agent.trim(), "claude" | "codex" | "agy")
}

fn recent_agent_switch_dialogue(events: Vec<String>) -> String {
    let mut remaining = AGENT_SWITCH_DIALOGUE_CHARS;
    let mut lines = Vec::new();
    for event in events.into_iter().rev() {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(&event) else {
            continue;
        };
        let Some(kind) = event.get("kind").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some(text) = event.get("text").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let speaker = match kind {
            "user" => "사용자",
            "text" | "result" => "에이전트",
            _ => continue,
        };
        let text = text.trim();
        if text.is_empty() || remaining == 0 {
            continue;
        }
        let max = remaining.min(AGENT_SWITCH_LINE_CHARS);
        let marker = " … [truncated]";
        let truncated = text.chars().count() > max;
        let limit = if truncated && max >= marker.chars().count() {
            max.saturating_sub(marker.chars().count())
        } else {
            max
        };
        let mut text = text.chars().take(limit).collect::<String>();
        if truncated && limit < max {
            text.push_str(marker);
        }
        let line = format!("- {speaker}: {text}");
        if lines.last().is_some_and(|last| last == &line) {
            continue;
        }
        remaining -= text.chars().count();
        lines.push(line);
        if lines.len() == AGENT_SWITCH_DIALOGUE_LINES {
            break;
        }
    }
    lines.reverse();
    lines.join("\n")
}

async fn agent_switch_handoff(pool: &SqlitePool, id: i64) -> Result<String, String> {
    let capsule = capsule::render_capsule_block(&assemble_capsule(pool, id).await?);
    let dialogue = db::recent_handoff_dialogue_events(pool, id, AGENT_SWITCH_DIALOGUE_EVENTS)
        .await
        .map_err(|error| error.to_string())?;
    let dialogue = recent_agent_switch_dialogue(dialogue);
    let mut handoff = format!(
        "{capsule}\n## Cross-agent handoff\n\
         Read existing AGENTS.md project guidance as relevant, alongside your native instruction files.\n"
    );
    if !dialogue.is_empty() {
        handoff.push_str("\n## Recent conversation\n");
        handoff.push_str(&dialogue);
        handoff.push('\n');
    }
    // 질문 카드의 답변은 일반 user 이벤트가 아니다. 전환과 새 토론 상대에게도 전달한다.
    let mut answers: Vec<(String, String)> = sqlx::query_as(
        "SELECT i.questions,a.payload FROM convo_interactions i JOIN convo_executions e ON e.id=i.execution_id JOIN convo_interaction_answers a ON a.interaction_id=i.id WHERE e.task_id=? AND a.state='acknowledged' ORDER BY i.created_at DESC,i.rowid DESC LIMIT 8",
    ).bind(id).fetch_all(pool).await.map_err(|error| error.to_string())?;
    answers.reverse();
    if !answers.is_empty() {
        handoff.push_str("\n## Recent clarification answers\n");
        for (questions, answers) in answers {
            let questions: crate::convo::interaction::Questions = serde_json::from_str(&questions).map_err(|error| error.to_string())?;
            let answers: Vec<crate::convo::interaction::Answer> = serde_json::from_str(&answers).map_err(|error| error.to_string())?;
            for answer in answers {
                if let Some(question) = questions.questions.iter().find(|question| question.id == answer.question_id) {
                    let answer = answer.text.as_deref().or_else(|| {
                        question.options.iter().find(|option| Some(&option.id) == answer.option_id.as_ref()).map(|option| option.label.as_str())
                    }).unwrap_or_default();
                    let question: String = question.question.chars().take(300).collect();
                    let answer: String = answer.chars().take(500).collect();
                    handoff.push_str(&format!("- {question}\n  Answer: {answer}\n"));
                }
            }
        }
    }
    Ok(handoff)
}

pub(crate) async fn switch_task_agent_checked(
    pool: &SqlitePool,
    active: ActiveConvos,
    id: i64,
    agent: &str,
    model: &str,
) -> Result<(Task, crate::convo::ConvoEvent), String> {
    let agent = agent.trim();
    if !supports_convo_agent_switch(agent) {
        return Err(format!("대화 에이전트로 전환할 수 없습니다: {agent}"));
    }
    let _reservation = reserve_convo_switch(active, id)?;
    crate::convo::interaction::runtime_of(pool, id).await?;
    crate::convo::interaction::ensure_idle(pool, id).await?;

    let task = db::get_task(pool, id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
    if task.mode != "conversation" || task.state != tstate::AWAITING_REVIEW {
        return Err("검토 대기 중인 대화 작업만 에이전트를 바꿀 수 있습니다".into());
    }
    // UI를 우회하는 원격·재시도 경로가 여기를 지난다. 전환은 세션을 버리므로(핸드오프 재조립)
    // 토론 중에 허용하면 좌측 세션만 사라진 반쪽 토론이 남는다.
    if !db::debate_sides(pool, id)
        .await
        .map_err(|error| error.to_string())?
        .is_empty()
    {
        return Err("토론 중에는 에이전트를 바꿀 수 없습니다 — 먼저 토론을 끝내세요".into());
    }
    let current = task.agent.as_deref().unwrap_or_default().trim();
    if !supports_convo_agent_switch(current) {
        return Err(format!("현재 대화 에이전트를 전환할 수 없습니다: {current}"));
    }
    if current == agent {
        return Err("같은 에이전트의 모델 변경은 task_model_set을 사용하세요".into());
    }
    if !Path::new(&task.worktree_path).is_dir() {
        return Err(worktree::missing_worktree_error(&task.worktree_path));
    }
    let handoff = agent_switch_handoff(pool, id).await?;
    let text = format!(
        "에이전트를 {current}에서 {agent}(으)로 전환했습니다. 다음 턴은 새 대화와 핸드오프로 시작합니다."
    );
    let event = crate::convo::ConvoEvent::ContextCleared { text };
    let encoded = serde_json::to_string(&event).map_err(|error| error.to_string())?;
    let task = db::switch_convo_agent(pool, id, agent, model.trim(), &handoff, &encoded, now())
        .await
        .map_err(|error| error.to_string())?;
    Ok((task, event))
}

/// 검토 대기 대화의 에이전트와 모델을 함께 전환한다. 벤더 세션은 호환되지 않으므로
/// 다음 전송은 저장한 핸드오프를 첫 메시지에 붙인 새 세션으로 시작한다.
#[tauri::command]
pub async fn task_agent_set(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    agent: String,
    model: String,
) -> Result<Task, String> {
    let pool = pool_of(&state)?;
    let (task, event) =
        switch_task_agent_checked(&pool, state.convo_active.clone(), id, &agent, &model).await?;
    let _ = app.emit("convo://event", ConvoPayload { id, speaker: None, event });
    Ok(task)
}

/// "검증 실패 시 Approve 차단" 토글 조회 (기본 OFF).
#[tauri::command]
pub async fn block_unverified_get(state: State<'_, AppState>) -> Result<bool, String> {
    let pool = pool_of(&state)?;
    Ok(db::get_setting(&pool, "block_unverified")
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("true"))
}

/// "검증 실패 시 Approve 차단" 토글 설정.
#[tauri::command]
pub async fn block_unverified_set(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    let pool = pool_of(&state)?;
    db::set_setting(
        &pool,
        "block_unverified",
        if enabled { "true" } else { "false" },
    )
    .await
    .map_err(|e| e.to_string())
}

/// 원격(텔레그램) 리뷰 커맨드(`/approve`, `/rollback`, `/retry`) 허용 여부 설정 키 — 기본 OFF
/// (PRD 0003 §9). OFF면 요약 카드에 커맨드 안내를 숨기고, 수신한 커맨드는 거부 회신한다.
const REMOTE_REVIEW_TOGGLE_KEY: &str = "remote_review_commands_enabled";

/// 원격 리뷰 커맨드 토글 조회(공용) — IPC 커맨드와 텔레그램 핸들러가 함께 사용한다.
pub(crate) async fn remote_review_enabled(pool: &SqlitePool) -> bool {
    db::get_setting(pool, REMOTE_REVIEW_TOGGLE_KEY)
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("true")
}

/// 원격 리뷰 커맨드 허용 토글 조회(설정 UI).
#[tauri::command]
pub async fn remote_review_commands_get(state: State<'_, AppState>) -> Result<bool, String> {
    let pool = pool_of(&state)?;
    Ok(remote_review_enabled(&pool).await)
}

/// 원격 리뷰 커맨드 허용 토글 설정(설정 UI). 기본 OFF.
#[tauri::command]
pub async fn remote_review_commands_set(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    db::set_setting(
        &pool,
        REMOTE_REVIEW_TOGGLE_KEY,
        if enabled { "true" } else { "false" },
    )
    .await
    .map_err(|e| e.to_string())
}

/// AwaitingReview 작업을 완료 처리로 점유하고 task 메타데이터를 반환한다.
fn ensure_convo_idle(convo_active: &ActiveConvos, id: i64) -> Result<(), String> {
    if convo_active
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains_key(&id)
    {
        return Err("대화 턴이 아직 종료 처리 중입니다 — 잠시 후 다시 시도하세요".into());
    }
    Ok(())
}

async fn claim_review_finalization(
    pool: &SqlitePool,
    convo_active: &ActiveConvos,
    id: i64,
) -> Result<Task, String> {
    ensure_convo_idle(convo_active, id)?;
    TaskService::new(pool.clone())
        .claim_review_finalization(id, false, now())
        .await
}

async fn retire_projection_for_review(pool: &SqlitePool, id: i64) -> Result<(), String> {
    // 파일형 투영은 원장이 없다 — 블록 자체를 걷어내야 승인 커밋에 섞이지 않는다.
    // 옛 DB 투영(P2)의 retire는 제거됐다(설계 2026-09-13 §P2 제거).
    memory::file::retire_task(pool, id)
        .await
        .map_err(|error| error.to_string())
}

async fn approve_with_decision_ledger(
    state: &AppState,
    pool: &SqlitePool,
    id: i64,
    attempt: &mut crate::approval::Attempt,
) -> Result<(), String> {
    ensure_convo_idle(&state.convo_active, id)?;
    let exclude_generated_mcp = db::has_task_event(pool, id, "mcp_generated")
        .await
        .map_err(|error| error.to_string())?;
    let task = decision::local_approval::claim(pool, id, exclude_generated_mcp, now())
        .await
        .map_err(|error| error.to_string())?;
    close_shell_of(state, id);
    state.lsp.shutdown_task(id).await;
    let mut restore = state.tasks.lock().unwrap().remove(&id);
    if let Some(session) = restore.as_ref().and_then(|active| active.session.as_ref()) {
        session.terminate();
    }
    attempt.stage = "durable_approval".into();
    if let Err(error) = decision::local_approval::resume(pool, &task, now()).await {
        if let Ok(journal) = decision::approval_journal::load(pool, id).await {
            attempt.stage = journal.failure_code.unwrap_or(journal.state);
            if journal.commit_sha.is_some() { attempt.source_sha = journal.commit_sha; }
        }
        let reviewable = db::get_task(pool, id)
            .await
            .ok()
            .flatten()
            .is_some_and(|current| current.state == tstate::AWAITING_REVIEW);
        if reviewable && Path::new(&task.worktree_path).exists() {
            if let Some(active) = restore.take() {
                state.tasks.lock().unwrap().insert(id, active);
            }
        }
        return Err(error.to_string());
    }
    attempt.stage = "completion".into();
    if let Ok(journal) = decision::approval_journal::load(pool, id).await {
        if journal.commit_sha.is_some() { attempt.source_sha = journal.commit_sha; }
    }
    let worktree = worktree_from_task(&task);
    close_designmode_webview_of(state, id);
    crate::designmode::cleanup_captures(&worktree.path, id);
    state.preview_workbench.invalidate(id);
    Ok(())
}

/// Read-only advisory inspection; final approval still runs all existing guards.
#[tauri::command]
pub async fn task_approval_status(state: State<'_, AppState>, id: i64) -> Result<crate::approval::Status, String> {
    let pool = pool_of(&state)?;
    let task = db::get_task(&pool, id).await.map_err(|e| e.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
    crate::approval::inspect(&pool, &task, worktree_from_task(&task)).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn approval_repair_status(state: State<'_, AppState>, id: i64) -> Result<Option<crate::approval::repair::Session>, String> {
    crate::approval::repair::observed_status(&pool_of(&state)?, id, &state.review_claims).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn approval_repair_prepare(state: State<'_, AppState>, id: i64) -> Result<crate::approval::repair::Session, String> {
    ensure_convo_idle(&state.convo_active, id)?;
    let pool = pool_of(&state)?;
    let task = db::get_task(&pool, id).await.map_err(|e| e.to_string())?.ok_or("작업을 찾을 수 없습니다")?;
    crate::approval::repair::prepare(&pool, &task, worktree_from_task(&task), &state.review_claims).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn approval_repair_run(state: State<'_, AppState>, id: i64, session_id: String) -> Result<crate::approval::repair::Session, String> {
    ensure_convo_idle(&state.convo_active, id)?;
    let pool = pool_of(&state)?;
    let task = db::get_task(&pool, id).await.map_err(|e| e.to_string())?.ok_or("작업을 찾을 수 없습니다")?;
    let model = model_for_task(&pool, id, task.agent.as_deref().unwrap_or("claude")).await;
    crate::approval::repair::run(pool, task, state.review_claims.clone(), session_id, model).await
}

#[tauri::command]
pub async fn approval_repair_cancel(state: State<'_, AppState>, id: i64, session_id: String) -> Result<(), String> {
    crate::approval::repair::cancel(&pool_of(&state)?, id, &session_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn approval_repair_accept(state: State<'_, AppState>, id: i64, session_id: String) -> Result<crate::approval::repair::Session, String> {
    ensure_convo_idle(&state.convo_active, id)?;
    let pool = pool_of(&state)?;
    let task = db::get_task(&pool, id).await.map_err(|e| e.to_string())?.ok_or("작업을 찾을 수 없습니다")?;
    let (result, _claim) = crate::approval::repair::accept_held(&pool, &task, &state.review_claims, &session_id).await.map_err(|e| e.to_string())?;
    close_shell_of(&state, id);
    if let Some(active) = state.tasks.lock().unwrap().remove(&id) { if let Some(session) = active.session { session.terminate(); } }
    state.tasks.lock().unwrap().insert(id, ActiveTask { worktree: crate::approval::repair::git::candidate(&result), session: None, preview_mcp: None });
    state.preview_workbench.invalidate(id);
    state.lsp.shutdown_task(id).await;
    Ok(result)
}

/// 파이프라인이 관리하는 작업(진행 중 실행의 티켓, 최종 승인 대기 밖의 통합 작업)은 일반 승인·폐기·충돌 해소 경로로 건드리지 못하게 한다.
pub(crate) async fn reject_if_pipeline_managed(pool: &SqlitePool, id: i64) -> Result<(), String> {
    match crate::pipeline::db::pipeline_guard_for_task(pool, id).await {
        Ok(None) => Ok(()),
        Ok(Some(message)) => Err(message),
        Err(error) => Err(format!("파이프라인 소속 여부를 확인하지 못했습니다: {error}")),
    }
}

/// 승인 시도 이력은 복구 저널과 분리해 실패 후 성공도 보존한다.
#[tauri::command]
pub async fn task_approve(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let pool = pool_of(&state)?;
    reject_if_pipeline_managed(&pool, id).await?;
    let _review_claim = state.review_claims.claim_finalization(id)?;
    let task = db::get_task(&pool, id).await.map_err(|e| e.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
    let mut attempt = crate::approval::Attempt::start(&pool, &task).await.map_err(|e| e.to_string())?;
    let result = approve_task_inner(&state, id, &mut attempt).await;
    attempt.finish(&pool, &result).await;
    result
}

async fn approve_task_inner(state: &AppState, id: i64, attempt: &mut crate::approval::Attempt) -> Result<(), String> {
    let pool = pool_of(state)?;
    crate::task_results::refresh_evidence(&pool, id).await?;
    crate::task_review::guard_apply(&pool, id).await?;
    // opt-in 검증 게이트: 설정 ON이면 검증 통과(evidence.ready) 전 Approve 차단(작업 보존).
    let block_on = db::get_setting(&pool, "block_unverified")
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("true");
    if block_on {
        let ready = db::get_evidence(&pool, id)
            .await
            .ok()
            .flatten()
            .map(|e| e.ready);
        if verify::approve_blocked(true, ready) {
            return Err(
                "검증을 통과하지 않았습니다 (설정: 검증 실패 시 Approve 차단) — Verify로 통과 후 다시 시도하세요".into(),
            );
        }
    }
    if decision::is_enabled(&pool)
        .await
        .map_err(|error| error.to_string())?
    {
        return approve_with_decision_ledger(state, &pool, id, attempt).await;
    }
    if decision::approval_journal::has_incomplete(&pool, id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("미완료 local approval journal은 ledger를 다시 켜서 복구해야 합니다".into());
    }
    let task = claim_review_finalization(&pool, &state.convo_active, id).await?;
    attempt.stage = "projection".into();
    if let Err(error) = retire_projection_for_review(&pool, id).await {
        let _ = db::restore_awaiting_review(&pool, id, now()).await;
        return Err(error);
    }
    let inspection_worktree = state
        .tasks
        .lock()
        .unwrap()
        .get(&id)
        .map(|active| active.worktree.clone())
        .unwrap_or_else(|| worktree_from_task(&task));
    attempt.stage = "policy".into();
    if let Err(error) = ensure_protected_paths_unchanged(&task, &inspection_worktree) {
        let _ = db::restore_awaiting_review(&pool, id, now()).await;
        return Err(error);
    }
    close_shell_of(state, id); // 머지/워크트리 제거 전에 워크스페이스 셸 정리
    state.lsp.shutdown_task(id).await; // 언어 서버도 함께 — 사라질 워크트리를 물고 있으면 안 된다
                                       // 원자적 take: DB 점유를 얻은 요청만 활성 핸들을 꺼낼 수 있다.
    let active = state.tasks.lock().unwrap().remove(&id);
    let (worktree, mut restore) = match active {
        Some(a) => {
            if let Some(s) = &a.session {
                s.terminate();
            }
            (a.worktree.clone(), Some(a))
        }
        None => (worktree_from_task(&task), None),
    };
    // 직접 모드는 별도 브랜치/워크트리가 없다 — 이미 메인 체크아웃에 있으므로 merge/cleanup 없이
    // 상태만 완료로 전환한다(worktree.approve() 호출 시 자기 자신을 merge/remove하게 되어 위험).
    if !is_direct_mode(&worktree) {
        let generated_mcp = match db::has_task_event(&pool, id, "mcp_generated").await {
            Ok(generated_mcp) => generated_mcp,
            Err(e) => {
                if let Some(active_task) = restore.take() {
                    state.tasks.lock().unwrap().insert(id, active_task);
                }
                let _ = db::restore_awaiting_review(&pool, id, now()).await;
                return Err(e.to_string());
            }
        };
        let approve_result = worktree.approve_observed(generated_mcp, |stage, commit| {
            attempt.stage = stage.into();
            if let Some(commit) = commit { attempt.source_sha = Some(commit.into()); }
        });
        if let Err(e) = approve_result {
            // 머지 실패 → 활성 작업이었으면 맵 복원(재시도/폐기 가능).
            if let Some(a) = restore.take() {
                state.tasks.lock().unwrap().insert(id, a);
            }
            let _ = db::restore_awaiting_review(&pool, id, now()).await;
            // Keep the original failure. The readiness panel offers conflict resolution
            // separately, so a latent conflict never masks a failed commit hook.
            return Err(e.to_string());
        }
    }
    attempt.stage = "completion".into();
    db::update_state(&pool, id, tstate::DONE, now())
        .await
        .map_err(|e| e.to_string())?;
    let _ = db::append_event(&pool, id, "approved", None, now()).await;
    close_designmode_webview_of(state, id); // D-1: 종결 시 프리뷰 웹뷰·캡처 정리(non-direct는 worktree 제거로 이미 사라짐 — 직접 모드 대비 방어)
    crate::designmode::cleanup_captures(&worktree.path, id);
    state.preview_workbench.invalidate(id);
    Ok(())
}

/// 체크포인트 생성 — 파일(worktree 커밋)과 대화(이벤트 경계)를 **함께** 잡는다.
#[tauri::command]
pub async fn checkpoint_create(
    state: State<'_, AppState>,
    id: i64,
    label: String,
) -> Result<db::ConvoCheckpoint, String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("체크포인트 이름을 입력하세요".into());
    }
    let pool = pool_of(&state)?;
    let worktree = conflict_worktree(&pool, id).await?;
    let commit = worktree
        .checkpoint_commit(&format!("dojang: checkpoint — {label}"))
        .map_err(|e| e.to_string())?;
    let max_event = db::max_convo_event_id(&pool, id)
        .await
        .map_err(|e| e.to_string())?;
    db::insert_convo_checkpoint(&pool, id, label, &commit, max_event, now())
        .await
        .map_err(|e| e.to_string())
}

/// 작업의 체크포인트 (최신 순).
#[tauri::command]
pub async fn checkpoint_list(
    state: State<'_, AppState>,
    id: i64,
) -> Result<Vec<db::ConvoCheckpoint>, String> {
    let pool = pool_of(&state)?;
    db::list_convo_checkpoints(&pool, id)
        .await
        .map_err(|e| e.to_string())
}

/// 되감기 — 파일은 진짜로, 대화는 재구성으로.
///
/// 순서가 설계의 핵심이다: **요약(LLM)을 파괴적 단계보다 먼저** 만든다. 모델 호출은 실패할 수 있고,
/// 실패가 파일 원복 뒤에 오면 되돌릴 방법이 없다. 요약이 실패하면 아무것도 파괴되지 않은 채 멈춘다.
#[tauri::command]
pub async fn convo_rewind(
    state: State<'_, AppState>,
    id: i64,
    checkpoint_id: i64,
) -> Result<rewind::RewindSummary, String> {
    let pool = pool_of(&state)?;
    let checkpoint = db::get_convo_checkpoint(&pool, checkpoint_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("체크포인트를 찾을 수 없습니다")?;
    if checkpoint.task_id != id {
        return Err("다른 작업의 체크포인트입니다".into());
    }
    let _reservation = reserve_convo_switch(state.convo_active.clone(), id)
        .map_err(|_| "대화가 진행 중입니다 — 턴이 끝난 뒤 되감으세요".to_string())?;
    let worktree = conflict_worktree(&pool, id).await?;

    // ① 절단 대상 수집
    let events = db::convo_events_after(&pool, id, checkpoint.convo_event_max_id)
        .await
        .map_err(|e| e.to_string())?;
    if events.is_empty() {
        return Err("이 체크포인트 이후에 되감을 기록이 없습니다".into());
    }

    // ② 요약 — 여기까지는 아무것도 파괴하지 않는다.
    let nonce = format!("PRAXIS-REWIND-{}-{:016x}", std::process::id(), rand_u64());
    let prompt = rewind::build_rewind_summary_prompt(&events, &checkpoint.label, &nonce);
    let model = crate::reviewer::detect_reviewer("");
    let nonce_c = nonce.clone();
    let raw = tauri::async_runtime::spawn_blocking(move || {
        crate::reviewer::run_reviewer(&model, &prompt, 180)
    })
    .await
    .map_err(|e| e.to_string())??;
    let summary = rewind::parse_rewind_summary(&raw, &nonce_c)?;

    // ③~⑥ 여기서부터 파괴적이다.
    worktree
        .restore_to_checkpoint(&checkpoint.worktree_commit)
        .map_err(|e| e.to_string())?;
    let ts = now();
    db::mark_convo_events_rewound(&pool, id, checkpoint.convo_event_max_id, ts)
        .await
        .map_err(|e| e.to_string())?;
    db::clear_convo_session(&pool, id)
        .await
        .map_err(|e| e.to_string())?;
    let rendered = rewind::render_summary(&summary, &checkpoint.label);
    let event = serde_json::json!({ "kind": "rewind_summary", "text": rendered }).to_string();
    let _ = db::append_convo_event(&pool, id, &event, ts).await;
    let _ = db::append_event(&pool, id, "rewound", Some(&checkpoint.label), ts).await;
    Ok(summary)
}

/// 충돌 해소용 worktree 핸들. 종료된 작업은 worktree가 이미 사라졌을 수 있어 거부한다.
async fn conflict_worktree(pool: &SqlitePool, id: i64) -> Result<Worktree, String> {
    reject_if_pipeline_managed(pool, id).await?;
    let task = db::get_task(pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    if matches!(
        task.state.as_str(),
        tstate::DONE | tstate::DISCARDED | tstate::FAILED
    ) {
        return Err("종료된 작업은 충돌을 해소할 수 없습니다".into());
    }
    let worktree = worktree_from_task(&task);
    if !worktree.path.is_dir() {
        return Err(worktree::missing_worktree_error(
            &worktree.path.to_string_lossy(),
        ));
    }
    Ok(worktree)
}

/// 충돌 해소 세션을 연다 — 충돌을 worktree 안에 가두고 파일별 양쪽 내용을 돌려준다.
/// 이미 열려 있으면(앱 재시작 등) 현재 상태를 그대로 읽는다.
#[tauri::command]
pub async fn conflict_begin(
    state: State<'_, AppState>,
    id: i64,
) -> Result<Vec<worktree::ConflictFile>, String> {
    let pool = pool_of(&state)?;
    let worktree = conflict_worktree(&pool, id).await?;
    worktree
        .begin_conflict_resolution()
        .map_err(|e| e.to_string())
}

/// 파일 하나를 해소한다. 남은 미해결 목록을 돌려줘 UI가 진행도를 그대로 그릴 수 있게 한다.
#[tauri::command]
pub async fn conflict_resolve(
    state: State<'_, AppState>,
    id: i64,
    path: String,
    resolution: worktree::Resolution,
) -> Result<Vec<String>, String> {
    let pool = pool_of(&state)?;
    let worktree = conflict_worktree(&pool, id).await?;
    worktree
        .resolve_conflict(&path, &resolution)
        .map_err(|e| e.to_string())?;
    worktree
        .unresolved_conflict_paths()
        .map_err(|e| e.to_string())
}

/// 해소를 마치고 머지 커밋을 만든다. 이후 승인은 fast-forward로 통과한다 —
/// 상태 전이·검증 게이트를 우회하지 않도록 여기서 승인까지 하지는 않는다.
#[tauri::command]
pub async fn conflict_finish(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let pool = pool_of(&state)?;
    let worktree = conflict_worktree(&pool, id).await?;
    worktree
        .finish_conflict_resolution()
        .map_err(|e| e.to_string())?;
    let _ = db::append_event(&pool, id, "conflict_resolved", None, now()).await;
    Ok(())
}

/// 세션을 버리고 체크포인트로 원복한다.
#[tauri::command]
pub async fn conflict_abort(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let pool = pool_of(&state)?;
    let worktree = conflict_worktree(&pool, id).await?;
    worktree
        .abort_conflict_resolution()
        .map_err(|e| e.to_string())
}

/// 직접 모드(워크트리 미격리) 판별 — 스키마 변경 없이 "경로 == repo 경로" 동일성으로 판별한다
/// (`create_task_internal`이 직접 모드일 때 그렇게 생성). true면 merge/cleanup(worktree
/// add/remove, 브랜치 삭제)을 절대 호출하면 안 된다 — 메인 체크아웃 자체를 건드리게 된다.
fn is_direct_mode(wt: &Worktree) -> bool {
    wt.is_direct()
}

/// DB 행에서 Worktree 핸들 재구성 (세션 없이 머지/제거만 수행할 때).
pub(crate) fn worktree_from_task(task: &Task) -> Worktree {
    Worktree {
        repo: PathBuf::from(&task.repo),
        path: PathBuf::from(&task.worktree_path),
        branch: task.branch.clone(),
        base: task.base.clone(),
        base_revision: task.base_revision.clone(),
    }
}

fn ensure_protected_paths_unchanged(task: &Task, worktree: &Worktree) -> Result<(), String> {
    let Some(contract) = task.goal_contract.as_deref() else {
        return Ok(());
    };
    if contract.protected_paths.is_empty() {
        return Ok(());
    }
    let changed = worktree
        .changed_paths()
        .map_err(|error| error.to_string())?;
    let violations =
        crate::goal_contract::protected_path_violations(&contract.protected_paths, &changed);
    if violations.is_empty() {
        return Ok(());
    }
    Err(format!(
        "Goal Contract 보호 경로가 변경되어 승인할 수 없습니다: {}",
        violations.join(", ")
    ))
}

/// 폐기가 워크트리 내용을 어떻게 다루는지. 브랜치는 어느 쪽이든 남는다 — 갈리는 것은
/// **지우기 전에 커밋하는가**이고, 그래서 워크트리가 이미 사라진 작업에서는 둘이 같아진다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BranchDisposal {
    /// 사용자 폐기 — 버리기 직전 상태를 커밋해 브랜치에 남기고 워크트리만 걷어낸다.
    ///
    /// 파괴적 처분(`worktree remove --force` + `branch -D`)을 남겨 두지 않는 이유는 남길 호출자가
    /// 없기 때문이다. 쓰지 않는 파괴 경로를 열거형에 두면 다음 사람이 그것을 고른다(설계 0056).
    CommitAndPreserve,
    /// 고아 종결 — 브랜치를 남긴다. 워크트리가 없는 지금 브랜치는 커밋된 작업물의 유일한 사본이다.
    Preserve,
}

/// 폐기: AwaitingReview 점유 → 폐기 시점 커밋 → worktree 제거 → Discarded.
///
/// 브랜치는 남는다. 폐기된 작업의 산출물이 문서뿐인 경우(조사·설계 단계) 그것이 근거의 유일한
/// 사본이고, 워크트리를 지우면 커밋되지 않은 파일은 어디에도 남지 않는다(설계 0056).
#[tauri::command]
pub async fn task_discard(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    reject_if_pipeline_managed(&pool_of(&state)?, id).await?;
    discard_task(&state, id, BranchDisposal::CommitAndPreserve).await
}

/// `task_discard`의 본체 — 브랜치 처분만 호출자가 정한다.
///
/// 고아 일괄 종결(`tasks_discard_orphans`)이 같은 골격을 그대로 탄다. 점유·투영 은퇴·세션 종료·
/// 실패 시 복원까지 폐기의 계약은 하나뿐이어야 한다 — 갈라 두면 한쪽만 고쳐진다.
pub(crate) async fn discard_task(
    state: &AppState,
    id: i64,
    disposal: BranchDisposal,
) -> Result<(), String> {
    let _review_claim = state.review_claims.claim_finalization(id)?;
    let pool = pool_of(state)?;
    if decision::approval_journal::has_incomplete(&pool, id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("미완료 local approval journal이 있어 작업을 폐기할 수 없습니다".into());
    }
    let task = claim_review_finalization(&pool, &state.convo_active, id).await?;
    if let Err(error) = retire_projection_for_review(&pool, id).await {
        let _ = db::restore_awaiting_review(&pool, id, now()).await;
        return Err(error);
    }
    close_shell_of(state, id); // 워크트리 제거 전에 워크스페이스 셸 정리
    state.lsp.shutdown_task(id).await;
    let active = state.tasks.lock().unwrap().remove(&id);
    let (worktree, restore) = match active {
        Some(a) => {
            if let Some(s) = &a.session {
                s.terminate();
            }
            (a.worktree.clone(), Some(a))
        }
        None => (worktree_from_task(&task), None),
    };
    // 직접 모드는 메인 체크아웃 그 자체 — 절대 discard(worktree remove/branch -D)하지 않는다.
    // 변경사항은 그대로 두고(사용자가 직접 git으로 되돌림) 작업 레코드만 정리한다.
    let mut preserved = None;
    if !is_direct_mode(&worktree) {
        let cleanup = match disposal {
            BranchDisposal::CommitAndPreserve => worktree.preserve_and_retire(),
            BranchDisposal::Preserve => worktree.retire_preserving_branch().map(|_| None),
        };
        match cleanup {
            Ok(evidence) => preserved = evidence,
            Err(e) => {
                if let Some(a) = restore {
                    state.tasks.lock().unwrap().insert(id, a);
                }
                let _ = db::restore_awaiting_review(&pool, id, now()).await;
                return Err(e.to_string());
            }
        }
    }
    db::update_state(&pool, id, tstate::DISCARDED, now())
        .await
        .map_err(|e| e.to_string())?;
    // 보존이 있었으면 근거가 어디 남았는지 종결 이벤트에 적는다 — 워크트리는 사라지고
    // 커밋은 브랜치만이 붙들고 있으므로, 이 한 줄이 나중에 되찾는 유일한 좌표다.
    let detail = preserved.map(|e| format!("preserved {}@{}", e.branch, e.commit));
    let _ = db::append_event(&pool, id, "discarded", detail.as_deref(), now()).await;
    close_designmode_webview_of(state, id); // D-1: 종결 시 프리뷰 웹뷰·캡처 정리
    crate::designmode::cleanup_captures(&worktree.path, id);
    state.preview_workbench.invalidate(id);
    Ok(())
}

/// 원격 승인 게이트 순수 판정(F-06, PRD §5.5) — 두 조건 중 위반된 항목을 우선순위
/// (증거→protected paths)로 하나만 사유로 반환한다(가장 먼저 고쳐야 할 항목을 안내).
/// 통과 시 None. 로컬 `task_approve`의 opt-in evidence 차단과 달리 원격은 항상 강제한다
/// (`verify::approve_blocked(true, ..)` 고정 호출).
pub(crate) fn remote_approval_violation(
    evidence_ready: Option<bool>,
    protected_violation: Option<&str>,
) -> Option<String> {
    if crate::verify::approve_blocked(true, evidence_ready) {
        return Some("verify 증거 미확보/미통과 — Verify 실행 후 다시 시도하세요".to_string());
    }
    protected_violation.map(|v| v.to_string())
}

/// 원격(모바일) 승인 게이트 — verify evidence ∧ protected_paths. 판정 우선순위는
/// `remote_approval_violation`(순수 함수)이 소유하고, 여기서는 두 조건의 DB/worktree 조회만
/// 담당한다. protected_paths 검사는 `task_approve`가 이미 쓰는 `ensure_protected_paths_unchanged`를
/// 그대로 재사용(중복 구현 금지).
pub(crate) async fn remote_approval_gate(pool: &SqlitePool, task: &Task) -> Result<(), String> {
    if decision::is_enabled(pool)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("Decision/Provenance Ledger P0는 데스크톱 로컬 승인만 지원합니다".into());
    }
    crate::task_review::guard_apply(pool, task.id).await?;
    crate::task_results::refresh_evidence(pool, task.id).await?;
    let evidence_ready = db::get_evidence(pool, task.id)
        .await
        .ok()
        .flatten()
        .map(|e| e.ready);
    let worktree = worktree_from_task(task);
    let protected_violation = ensure_protected_paths_unchanged(task, &worktree).err();
    match remote_approval_violation(evidence_ready, protected_violation.as_deref()) {
        Some(reason) => Err(reason),
        None => Ok(()),
    }
}

#[cfg(test)]
mod mobile_ledger_tests {
    use super::*;

    #[test]
    fn 질문_대기와_결과_검토_대기를_다른_이벤트로_가른다() {
        assert_eq!(
            awaiting_ledger_kind(Some(db::awaiting_kind::QUESTION)),
            db::runner_event_kind::AWAITING_ANSWER
        );
        assert_eq!(
            awaiting_ledger_kind(None),
            db::runner_event_kind::AWAITING_REVIEW
        );
        // 모르는 성격은 통상 검토로 접는다 — 새 값이 생겨도 원장이 비지 않게.
        assert_eq!(
            awaiting_ledger_kind(Some("something-new")),
            db::runner_event_kind::AWAITING_REVIEW
        );
    }

    #[test]
    fn 원장에_쓰는_두_종류는_모두_푸시_대상이다() {
        // 이 둘이 어긋나면 원장은 차는데 폰은 조용하다 — 증상이 "가끔 안 온다"로만 보인다.
        assert!(crate::runner::push::should_notify(
            db::runner_event_kind::AWAITING_REVIEW
        ));
        assert!(crate::runner::push::should_notify(
            db::runner_event_kind::AWAITING_ANSWER
        ));
    }
}

#[cfg(test)]
mod remote_ledger_boundary_tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    #[tokio::test]
    async fn remote_approval_is_rejected_while_the_local_ledger_is_enabled() {
        let sequence = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = crate::testtmp::dir().join(format!(
            "praxis-remote-ledger-boundary-{}-{sequence}.sqlite",
            std::process::id()
        ));
        let pool = db::init_pool(path.to_str().unwrap()).await.unwrap();
        let id = db::insert_task(
            &pool,
            "/repo",
            "branch",
            "main",
            "/worktree",
            "remote",
            None,
            None,
            "terminal",
            1,
        )
        .await
        .unwrap();
        db::set_setting(&pool, decision::FLAG_KEY, "true")
            .await
            .unwrap();
        let task = db::get_task(&pool, id).await.unwrap().unwrap();

        let error = remote_approval_gate(&pool, &task).await.unwrap_err();

        assert!(error.contains("데스크톱 로컬 승인만 지원"));
        assert_eq!(
            db::get_task(&pool, id).await.unwrap().unwrap().state,
            db::state::CREATED
        );
        drop(pool);
        let _ = std::fs::remove_file(path);
    }
}

/// 원격(모바일) 후속 지시 — AwaitingReview 대화에 후속 지시를 주입해
/// resume한다. `annotations_resend`와 동일하게 `start_convo_turn`(convo_send 공용 경로)을
/// 재사용한다(중복 구현 금지). 상태가 AwaitingReview가 아니면 현재 상태를 에러로 반환한다.
pub(crate) async fn remote_review_retry(
    app: &AppHandle,
    state: &AppState,
    id: i64,
    instruction: String,
) -> Result<(), String> {
    let pool = pool_of(state)?;
    let task = db::get_task(&pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    if task.state != tstate::AWAITING_REVIEW {
        return Err(format!(
            "검토 대기 상태가 아닙니다 (현재 상태: {})",
            task.state
        ));
    }
    let cwd = task.worktree_path.clone();
    if !std::path::Path::new(&cwd).is_dir() {
        return Err(worktree::missing_worktree_error(&cwd));
    }
    start_convo_turn(
        app.clone(),
        pool,
        state.convo_active.clone(),
        id,
        task.repo.clone(),
        cwd,
        instruction,
        Vec::new(),
        None,
        ConversationInputOrigin::RemoteReviewRetry,
        Some(format!("vault-followup:{id}")),
        state.updating.clone(),
        None,
        ConvoAdmissionAction::PreserveTakeover,
    )
    .await
}

/// 외부기원(크론/모바일) 승인 대기 작업 승인(공용 로직) — IPC(`task_run_approve`)와 모바일
/// 승인 경로가 공유. PENDING_APPROVAL이 아니면 거부.
pub(crate) async fn approve_pending_task(
    app: &AppHandle,
    state: &AppState,
    id: i64,
) -> Result<(), String> {
    let pool = pool_of(state)?;
    let task = db::get_task(&pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    if task.state != tstate::PENDING_APPROVAL {
        return Err("승인 대기 상태가 아닙니다".into());
    }
    spawn_task_agent(app, state, &task).await
}

/// 외부기원(봇/크론) 승인 대기 작업 거부(공용 로직) — worktree 정리 + Discarded 전이.
/// IPC(`task_run_reject`)와 텔레그램 `/reject` 명령이 공유. PENDING_APPROVAL이 아니면 거부.
pub(crate) async fn reject_pending_task(state: &AppState, id: i64) -> Result<(), String> {
    let _review_claim = state.review_claims.claim_finalization(id)?;
    let pool = pool_of(state)?;
    let task = db::get_task(&pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    if task.state != tstate::PENDING_APPROVAL {
        return Err("승인 대기 상태가 아닙니다".into());
    }
    retire_projection_for_review(&pool, id).await?;
    let active = state.tasks.lock().unwrap().remove(&id);
    let worktree = match active {
        Some(a) => a.worktree,
        None => worktree_from_task(&task),
    };
    // 이 경로는 PENDING_APPROVAL(External 기원)만 타므로 direct 모드일 수 없다(항상 격리 강제,
    // `create_task_internal` 참고) — 그래도 방어적으로 가드해 향후 변경에도 안전하게 둔다.
    if !is_direct_mode(&worktree) {
        worktree.discard().map_err(|e| e.to_string())?;
    }
    db::update_state(&pool, id, tstate::DISCARDED, now())
        .await
        .map_err(|e| e.to_string())?;
    let _ = db::append_event(&pool, id, "discarded", None, now()).await;
    Ok(())
}

/// 외부기원(봇/크론) 승인 대기 작업 승인 IPC 래퍼 — 본 로직은 `approve_pending_task`(봇과 공용).
/// (`task_approve`는 이미 AwaitingReview→Done 머지 승인에 쓰이므로 별도 이름.)
#[tauri::command]
pub async fn task_run_approve(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    approve_pending_task(&app, &state, id).await
}

/// 외부기원(봇/크론) 승인 대기 작업 거부 IPC 래퍼 — 본 로직은 `reject_pending_task`(봇과 공용).
#[tauri::command]
pub async fn task_run_reject(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    reject_pending_task(&state, id).await
}

/// 종료 상태 작업을 이력에서 영구 삭제 (tasks + convo/task_events/evidence).
/// 진행 중(Created/Running/AwaitingReview/PendingApproval) 작업은 거부 — 그건 폐기(Discard/Reject)로 정리한다.
#[tauri::command]
pub async fn task_delete(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let _review_claim = state.review_claims.claim_finalization(id)?;
    let pool = pool_of(&state)?;
    let task = db::get_task(&pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    if matches!(
        task.state.as_str(),
        tstate::CREATED
            | tstate::RUNNING
            | tstate::AWAITING_REVIEW
            | tstate::FINALIZING
            | tstate::PENDING_APPROVAL
    ) {
        return Err("진행 중인 작업은 먼저 '버리기'로 정리하세요".into());
    }
    // 활성 맵에 남아 있으면 함께 제거(방어적 — 종료 상태면 보통 없음).
    state.tasks.lock().unwrap().remove(&id);
    close_shell_of(&state, id);
    state.lsp.shutdown_task(id).await; // 방어적 — 종료 상태면 보통 이미 정리돼 있다.
    close_designmode_webview_of(&state, id); // 방어적 — approve/discard 캡처 정리가 이미 끝났어도 웹뷰 핸들은 남을 수 있음.
    db::delete_task(&pool, id)
        .await
        .map_err(|e| e.to_string())?;
    state.preview_workbench.invalidate(id);
    Ok(())
}

/// 고아 판정 — 워크트리가 사라졌는데 DB 상태는 아직 살아 있는 작업.
///
/// 종료 상태는 건너뛴다: 워크트리가 없는 것이 정상이고, 이력 전체에 stat을 거는 비용도 아깝다.
/// 직접 모드(메인 체크아웃)는 경로가 그대로 있으므로 자연히 false로 남는다.
///
/// 배지 표시(`mark_missing_worktrees`)와 일괄 종결(`tasks_discard_orphans`)이 이 하나를 함께
/// 본다 — 보이는 것과 지워지는 것이 갈리면 사용자는 배지가 사라지지 않는 이유를 알 수 없다.
fn is_orphan(task: &Task) -> bool {
    !tstate::is_terminal(&task.state) && !std::path::Path::new(&task.worktree_path).is_dir()
}

/// 비종료 작업의 `worktree_missing`을 조회 시점에 관측해 채운다.
fn mark_missing_worktrees(mut tasks: Vec<Task>) -> Vec<Task> {
    for task in &mut tasks {
        task.worktree_missing = is_orphan(task);
    }
    tasks
}

/// 고아 일괄 종결 결과 — 무엇이 정리됐고 무엇이 왜 남았는지 그대로 돌려준다.
#[derive(Debug, Serialize)]
pub struct OrphanCleanup {
    pub retired: Vec<i64>,
    pub failed: Vec<OrphanFailure>,
}

#[derive(Debug, Serialize)]
pub struct OrphanFailure {
    pub id: i64,
    pub reason: String,
}

/// 워크트리가 사라진 비종료 작업을 일괄 종결한다 — **브랜치는 남긴다**.
///
/// 워크트리 없이는 열어도 아무것도 실행되지 않으므로 이 작업들은 사실상 이미 끝났는데, DB에는
/// 살아 있어 대시보드를 채운다. 한 건씩 폐기하는 것과 다른 점은 브랜치 처분 하나다
/// ([`Worktree::retire_preserving_branch`]).
#[tauri::command]
pub async fn tasks_discard_orphans(state: State<'_, AppState>) -> Result<OrphanCleanup, String> {
    let pool = pool_of(&state)?;
    let tasks = db::list_tasks(&pool).await.map_err(|e| e.to_string())?;
    let orphans: Vec<i64> = tasks.iter().filter(|t| is_orphan(t)).map(|t| t.id).collect();
    let mut retired = Vec::new();
    let mut failed = Vec::new();
    for id in orphans {
        // 한 건의 실패가 나머지를 막지 않는다 — 고아끼리는 독립이고, 지금 못 지우는 것(진행 중인
        // 턴을 붙들었거나 미완료 approval journal이 있는 작업)이 섞여 있어도 나머지는 정리된다.
        match discard_task(&state, id, BranchDisposal::Preserve).await {
            Ok(()) => retired.push(id),
            Err(reason) => failed.push(OrphanFailure { id, reason }),
        }
    }
    Ok(OrphanCleanup { retired, failed })
}

/// 전체 Task 이력 (대시보드는 비종료 상태를 카드로 표시).
#[tauri::command]
pub async fn task_list(state: State<'_, AppState>) -> Result<Vec<Task>, String> {
    let pool = pool_of(&state)?;
    let tasks = db::list_tasks(&pool).await.map_err(|e| e.to_string())?;
    Ok(mark_missing_worktrees(tasks))
}

/// 지금까지 관측된 실행 모델 (벤더별, 최근 사용순) — 모델 드롭다운의 하드코딩 카탈로그 보강용.
/// 벤더 CLI가 모델 목록을 노출하지 않아 관측이 유일한 자동 축적 경로다. 로컬 DB 전용.
#[tauri::command]
pub async fn observed_models(state: State<'_, AppState>) -> Result<Vec<db::ObservedModel>, String> {
    let pool = pool_of(&state)?;
    db::observed_models(&pool).await.map_err(|e| e.to_string())
}

/// Quick Open(⌘K) 백엔드 소스 — tasks/sessions LIKE+최근성 검색. 파일/스킬/커맨드는 프론트가
/// 로컬로 조회해 `quickopen.ts`에서 병합한다(runner/http.rs의 동명 핸들러와 로직 공유·parity 보장).
#[tauri::command]
pub async fn quickopen_search(
    state: State<'_, AppState>,
    query: String,
    scopes: Vec<String>,
) -> Result<Vec<db::QuickOpenCandidate>, String> {
    let pool = pool_of(&state)?;
    db::quickopen_search(&pool, &query, &scopes, 50)
        .await
        .map_err(|e| e.to_string())
}

/// 특정 작업 stdin 전달 (R5).
#[tauri::command]
pub async fn task_write(state: State<'_, AppState>, id: i64, data: String) -> Result<(), String> {
    let pool = pool_of(&state)?;
    {
        let tasks = state.tasks.lock().unwrap();
        let active = tasks.get(&id).ok_or("활성 세션이 없습니다")?;
        active
            .session
            .as_ref()
            .ok_or("대화 모드 작업에는 터미널 입력을 보낼 수 없습니다")?;
    }
    followup_observation::record_followup_before_forward(&pool, id, now(), || {
        let tasks = state.tasks.lock().unwrap();
        let active = tasks
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("활성 세션이 없습니다"))?;
        let session = active
            .session
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("대화 모드 작업에는 터미널 입력을 보낼 수 없습니다"))?;
        session.write(data.as_bytes())
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn knowledge_vault_local_composer_send(
    state: State<'_, AppState>,
    id: i64,
    message: String,
    expected_client_ref: String,
) -> Result<(), String> {
    if !valid_vault_followup_client_ref(id, &expected_client_ref) || message.trim().is_empty() {
        return Err("vault composer request is invalid".into());
    }
    let pool = pool_of(&state)?;
    let task = db::get_task(&pool, id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
    let (attempt, delivery) = if cfg!(target_os = "macos") {
        let attempt =
            begin_vault_attempt(&pool, id, &task.repo, &message, Some(&expected_client_ref))
                .await
                .map_err(|error| error.to_string())?;
        if attempt.is_none() {
            let pending = crate::knowledge::vault::retrieval::pending_reference_request(
                &pool,
                &expected_client_ref,
            )
            .await
            .map_err(|error| error.to_string())?
                || crate::knowledge::vault::provenance::pending_restrictive_draft_policy(
                    &pool,
                    &expected_client_ref,
                )
                .await
                .map_err(|error| error.to_string())?;
            if pending {
                return Err(
                    "vault delivery is unavailable; review selected references before sending".into(),
                );
            }
        }
        if let Some(attempt) = attempt.as_deref() {
            crate::knowledge::vault::provenance::record_unknown_input_for_attempt(
                &pool,
                attempt,
                crate::knowledge::vault::provenance::InputOrigin::ToolResult,
                "terminal follow-up may include untracked PTY output",
                now(),
            )
            .await
            .map_err(|error| error.to_string())?;
            crate::knowledge::vault::provenance::record_unknown_input_for_attempt(
                &pool,
                attempt,
                crate::knowledge::vault::provenance::InputOrigin::PriorConversation,
                "terminal follow-up may include untracked prior context",
                now(),
            )
            .await
            .map_err(|error| error.to_string())?;
        }
        let delivery = vault_delivery(
            &pool,
            id,
            &task.repo,
            &message,
            Some(&expected_client_ref),
            attempt.as_ref(),
        )
        .await
        .map_err(|error| error.to_string())?;
        (attempt, delivery)
    } else {
        (None, None)
    };
    let payload = delivery.as_ref().map_or_else(
        || message.clone(),
        |item| crate::knowledge::vault::retrieval::delivery_payload(&message, &item.preview),
    );
    let framed = format!("\u{1b}[200~{payload}\u{1b}[201~\r");
    crate::knowledge::vault::usage::write_with_receipt(
        &pool,
        id,
        attempt.as_deref(),
        framed.as_bytes(),
        now(),
        |bytes| {
            let tasks = state.tasks.lock().unwrap();
            let active = tasks
                .get(&id)
                .ok_or_else(|| "활성 세션이 없습니다".to_string())?;
            let session = active
                .session
                .as_ref()
                .ok_or_else(|| "대화 모드 작업에는 터미널 입력을 보낼 수 없습니다".to_string())?;
            session.write(bytes).map_err(|error| error.to_string())
        },
    )
    .await
}

/// 작업 PTY 스크롤백 replay(base64) — attach 시 라이브 스트림(`pty://output/{id}`) 구독보다
/// 먼저 호출해 xterm에 선기록한다. 세션이 없거나 대화 모드(PTY 없음)면 빈 문자열(에러 아님).
#[tauri::command]
pub async fn task_pty_replay(state: State<'_, AppState>, id: i64) -> Result<String, String> {
    // base64 인코딩은 락 밖에서 — 입력 경로가 스크롤백 크기만큼 락을 잡지 않게 한다.
    let snapshot = {
        let tasks = state.tasks.lock().unwrap_or_else(|e| e.into_inner());
        let Some(active) = tasks.get(&id) else {
            return Ok(String::new());
        };
        let Some(session) = active.session.as_ref() else {
            return Ok(String::new());
        };
        session.scrollback_snapshot()
    };
    Ok(STANDARD.encode(snapshot))
}

/// 특정 작업 터미널 리사이즈.
#[tauri::command]
pub async fn task_resize(
    state: State<'_, AppState>,
    id: i64,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let tasks = state.tasks.lock().unwrap_or_else(|e| e.into_inner());
    let a = tasks.get(&id).ok_or("활성 세션이 없습니다")?;
    match &a.session {
        Some(s) => s.resize(cols, rows).map_err(|e| e.to_string()),
        None => Ok(()), // 대화 모드: PTY 없음 → 리사이즈 무시
    }
}

#[derive(Clone, Serialize)]
struct ActionOutputPayload {
    /// `shell://`·`pty://`와 같은 필드명을 쓴다 — 프론트 `attachPtyStream`이 `id`로
    /// 필터링하므로, 값 타입만 다르고 계약은 같다("<kind>:<vendor>").
    id: String,
    data: String,
}

#[derive(Clone, Serialize)]
struct ActionExitPayload {
    id: String,
    code: i32,
}

/// 실행 중인 작업 수 — PTY 작업과 대화 턴, **그리고 아직 등록 전인 생성분**을 합친다.
///
/// `reserved`를 빠뜨리면 안 된다. `create_task_internal`은 슬롯을 예약한 뒤 worktree 생성과
/// 임베딩을 거쳐서야 `tasks`에 넣는다 — 그 수 초 동안 이 함수가 0을 반환하면, 자동 업데이트는
/// "아무도 없다"고 판단하고 방금 만들어지는 작업의 발밑에서 바이너리를 갈아치운다.
/// 동시 실행 상한이 이미 같은 이유로 `reserved`를 세고 있다(`reserve_slot` 참고).
///
/// 업데이트 PTY도 센다. 손으로 연 업데이트가 돌고 있는데 자동 업데이트가 같은 패키지에
/// 두 번째 설치를 걸면 전역 트리가 깨진다.
pub(crate) fn active_work_count(state: &AppState) -> usize {
    let tasks = state.tasks.lock().unwrap_or_else(|e| e.into_inner()).len();
    let convos = state
        .convo_active
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .len();
    let reserved = *state.reserved.lock().unwrap_or_else(|e| e.into_inner());
    let questions = state.side_question_active.lock().unwrap_or_else(|e| e.into_inner()).len();
    let updating_shells = state
        .action_shells
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .filter(|key| key.starts_with("update:"))
        .count();
    tasks + convos + questions + reserved + updating_shells
}

/// 업데이트 중 거부 문구. 가드가 두 군데라 문구가 갈리면 같은 사건이 다르게 보인다.
pub(crate) const UPDATING_REFUSAL: &str = "CLI 자동 업데이트가 진행 중입니다 — 바이너리가 \
    교체되는 동안 시작한 작업은 중간에 깨집니다. 끝난 뒤 다시 시도하세요.";

/// 자동 업데이트가 도는 동안은 새 작업을 시작하지 않는다.
///
/// `agent_action_open`의 가드와 방향이 반대다 — 저쪽은 "작업이 있으면 업데이트를 막고",
/// 이쪽은 "업데이트 중이면 작업을 막는다". 둘 다 있어야 창이 닫힌다.
pub(crate) fn refuse_while_updating(state: &AppState) -> Result<(), String> {
    refuse_if_updating(&state.updating)
}

/// `AppState`를 통째로 받지 못하는 자리(대화 턴)를 위한 같은 판정.
pub(crate) fn refuse_if_updating(updating: &AtomicBool) -> Result<(), String> {
    if updating.load(Ordering::SeqCst) {
        return Err(UPDATING_REFUSAL.into());
    }
    Ok(())
}

/// 액션 PTY 열기. 이미 열려 있으면 재사용하고 `true`를 반환한다.
///
/// `Update`는 실행 중인 바이너리를 갈아치우므로 **활성 작업이 있으면 거부한다.**
/// 강행 옵션은 두지 않는다 — 깨진 작업을 되살리는 비용이 기다리는 비용보다 크다.
#[tauri::command]
pub fn agent_action_open(
    app: AppHandle,
    state: State<AppState>,
    kind: crate::agenthealth::action::ActionKind,
    vendor: String,
    cols: u16,
    rows: u16,
) -> Result<bool, String> {
    use crate::agenthealth::action::ActionKind;

    let key = format!("{}:{}", kind.key(), vendor);
    if state.action_shells.lock().unwrap().contains_key(&key) {
        return Ok(true);
    }

    if kind.replaces_binary() {
        refuse_while_updating(&state)?;
        let active = active_work_count(&state);
        if active > 0 {
            return Err(format!(
                "실행 중인 작업이 {active}개 있습니다 — 업데이트는 바이너리를 교체하므로 \
                 작업이 중간에 깨집니다. 끝난 뒤 다시 시도하세요."
            ));
        }
    }

    let (cmd, args, cwd) = if kind == ActionKind::Scratch {
        let spec = default_shell();
        let home = crate::usage::home_dir().map(|home| home.to_string_lossy().into_owned());
        (spec.cmd, spec.args, home)
    } else {
        let health_method = crate::agenthealth::detect::install_method_of_bin(
            crate::agenthealth::bin_of(&vendor).ok_or_else(|| format!("알 수 없는 벤더입니다: {vendor}"))?,
        );
        let command = crate::agenthealth::action::command_for(
            kind,
            &vendor,
            health_method,
            crate::agenthealth::package_of(&vendor),
        )?;
        let resolved = crate::agenthealth::detect::resolve_bin(&command.bin)
            .ok_or_else(|| format!("PATH에서 찾을 수 없습니다: {}", command.bin))?;
        (resolved, command.args, None)
    };

    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let (session, rx) = PtySession::spawn(&cmd, &argv, cwd.as_deref(), cols, rows)
        .map_err(|e| format!("액션 PTY 생성 실패: {e}"))?;
    {
        // 삽입 직전 재확인(동시 open 레이스) — 진 쪽의 새 세션은 정리하고 기존 것을 쓴다.
        let mut shells = state.action_shells.lock().unwrap();
        if shells.contains_key(&key) {
            session.terminate();
            return Ok(true);
        }
        shells.insert(key.clone(), session);
    }

    std::thread::spawn(move || {
        let mut coalescer = OutputCoalescer::new();
        while let Ok(ev) = rx.recv() {
            // 이벤트명은 그대로 둔다(키에 `:`가 들어가고 볼륨이 낮다) — 합치기만 적용.
            let code = match ev {
                PtyEvent::Output(b) => {
                    let (bytes, pending_exit) = coalescer.gather(&rx, b);
                    let _ = app.emit(
                        "action://output",
                        ActionOutputPayload {
                            id: key.clone(),
                            data: STANDARD.encode(&bytes),
                        },
                    );
                    match pending_exit {
                        Some(code) => code,
                        None => continue,
                    }
                }
                PtyEvent::Exit(code) => code,
            };
            app.state::<AppState>()
                .action_shells
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&key);
            let _ = app.emit("action://exit", ActionExitPayload { id: key, code });
            break;
        }
    });
    Ok(false)
}

/// 액션 PTY 스크롤백 replay(base64). 열린 세션이 없으면 빈 문자열(에러 아님).
#[tauri::command]
pub async fn agent_action_replay(
    state: State<'_, AppState>,
    key: String,
) -> Result<String, String> {
    // base64 인코딩은 락 밖에서 — 입력 경로가 스크롤백 크기만큼 락을 잡지 않게 한다.
    let snapshot = {
        let shells = state.action_shells.lock().unwrap_or_else(|e| e.into_inner());
        let Some(session) = shells.get(&key) else {
            return Ok(String::new());
        };
        session.scrollback_snapshot()
    };
    Ok(STANDARD.encode(snapshot))
}

/// 액션 PTY stdin 전달 — device code 붙여넣기·확인 입력이 여기로 간다.
#[tauri::command]
pub async fn agent_action_write(
    state: State<'_, AppState>,
    key: String,
    data: String,
) -> Result<(), String> {
    let shells = state.action_shells.lock().unwrap_or_else(|e| e.into_inner());
    let session = shells.get(&key).ok_or("열린 액션 세션이 없습니다")?;
    session.write(data.as_bytes()).map_err(|e| e.to_string())
}

/// 액션 PTY 리사이즈.
#[tauri::command]
pub async fn agent_action_resize(
    state: State<'_, AppState>,
    key: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let shells = state.action_shells.lock().unwrap_or_else(|e| e.into_inner());
    let session = shells.get(&key).ok_or("열린 액션 세션이 없습니다")?;
    session.resize(cols, rows).map_err(|e| e.to_string())
}


/// 작업 종결(승인/폐기/삭제) 시 워크스페이스 셸 정리 — 워크트리가 사라지기 전에 죽인다.
fn close_shell_of(state: &AppState, id: i64) {
    if let Some(slot) = state.shells.lock().unwrap().remove(&id) {
        slot.session.terminate();
    }
    state.reaped_shells.lock().unwrap().remove(&id);
}

/// 작업 종결 시 Design Mode 프리뷰 웹뷰 정리(D-1) — 고아 웹뷰·고아 창 방지.
fn close_designmode_webview_of(state: &AppState, id: i64) {
    state.control_tokens.revoke_task(id);
    close_preview_surface(state, id);
}

/// 창 닫기는 MCP 연결을 폐기하지 않는다. 작업 종료만 토큰을 회수한다.
pub(crate) fn close_preview_surface(state: &AppState, id: i64) {
    let handle = state.preview_openings.close(id, || {
        state.preview_bridge.close(id);
        state
            .control_last
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&id);
        state.designmode_webviews.lock().unwrap().remove(&id)
    });
    if let Some(handle) = handle {
        let app = handle.webview.app_handle().clone();
        destroy_preview(&handle);
        let _ = app.emit("designmode://closed", id);
    }
}

/// 프리뷰 하나를 없앤다. **창 모드는 창을 파괴해야 한다.**
///
/// `Webview::close()`는 창 안의 웹뷰만 떼어낸다(wry `WebviewMessage::Close`는
/// `window.webviews`에서 빼는 것이 전부다) — 창은 빈 채로 화면에 남고, tauri의 창 맵에도
/// 그 라벨이 그대로 있다. 모드를 바꾸거나 프리뷰를 닫을 때마다 "Praxis Preview" 유령 창이
/// 하나씩 쌓이고, 그 창이 메인 창 위에 떠 URL 입력줄·선택 버튼을 덮었다.
///
/// `destroy()`는 즉시 닫는다. 표면 닫기 호출부가 상태 제거 후 closed 이벤트를 보낸다.
fn destroy_preview(handle: &PreviewHandle) {
    match handle.mode {
        // 자식 웹뷰는 창이 없다 — 웹뷰를 닫는 것이 곧 없애는 것이다.
        crate::designmode::PreviewMode::Inline => {
            let _ = handle.webview.close();
        }
        crate::designmode::PreviewMode::Window => {
            if let Some(window) = &handle.window {
                let _ = window.destroy();
            } else {
                let _ = handle.webview.window().destroy();
            }
        }
    }
}











// ── 설정: 캡처 실행 프로파일 (설계 0055) ──

/// 설정 화면이 보여줄 캡처 프로파일. **실효값과 원본을 함께 낸다** — 빈칸만 보여주면
/// "무엇이 도는지 모른다"는 원래 문제가 설정 화면에서 재현된다(AD-4).
#[derive(Serialize)]
pub struct CaptureProfileView {
    /// 실제로 CLI에 실릴 값.
    pub model: String,
    pub effort: String,
    pub lean: bool,
    /// 사용자가 명시한 값(미설정이면 빈 문자열) — 입력란은 이것을 보여준다.
    pub model_raw: String,
    pub effort_raw: String,
}

#[tauri::command]
pub async fn capture_profile_get(state: State<'_, AppState>) -> Result<CaptureProfileView, String> {
    let pool = pool_of(&state)?;
    let raw = |k: &'static str| {
        let pool = pool.clone();
        async move {
            db::get_setting(&pool, k)
                .await
                .ok()
                .flatten()
                .unwrap_or_default()
        }
    };
    let effective = crate::capture::invoke::profile(&pool).await;
    Ok(CaptureProfileView {
        model: effective.model,
        effort: effective.effort,
        lean: effective.lean,
        model_raw: raw(crate::capture::invoke::KEY_MODEL).await,
        effort_raw: raw(crate::capture::invoke::KEY_EFFORT).await,
    })
}

/// 캡처 프로파일 저장. 빈 값은 **해제**(코드 기본값으로 복귀)이며, effort는 벤더 검증을
/// 통과해야 한다 — 무효값을 그대로 두면 CLI가 즉사하고 그 실패는 조용하다.
#[tauri::command]
pub async fn capture_profile_set(
    state: State<'_, AppState>,
    model: String,
    effort: String,
    lean: bool,
) -> Result<(), String> {
    let effort = effort.trim();
    if !effort.is_empty() {
        crate::agent::reasoning_effort_override("claude", Some(effort))?;
    }
    let pool = pool_of(&state)?;
    for (key, value) in [
        (crate::capture::invoke::KEY_MODEL, model.trim()),
        (crate::capture::invoke::KEY_EFFORT, effort),
        (
            crate::capture::invoke::KEY_LEAN,
            if lean { "true" } else { "false" },
        ),
    ] {
        db::set_setting(&pool, key, value)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ── 설정: 동시 실행 상한 ──

/// 동시 실행 상한 + 허용 범위. 범위를 함께 실어 UI가 경계를 따로 하드코딩하지 않게 한다
/// (프런트에 복제하면 백엔드 상수를 고칠 때 조용히 어긋난다).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ConcurrencyLimit {
    pub value: usize,
    pub min: usize,
    pub max: usize,
}

impl ConcurrencyLimit {
    fn of(value: usize) -> Self {
        Self {
            value,
            min: MAX_CONCURRENT_MIN,
            max: MAX_CONCURRENT_MAX,
        }
    }
}

/// 허용 범위로 접는다. 0은 "작업을 아예 만들 수 없음"이라 하한이 1이다.
pub fn clamp_max_concurrent(value: usize) -> usize {
    value.clamp(MAX_CONCURRENT_MIN, MAX_CONCURRENT_MAX)
}

/// 저장된 설정 문자열 → 동시 실행 상한. 미설정·파싱 실패는 기본값, 범위 밖은 경계로 접는다.
/// 손상된 값 하나로 기동이 막히면 안 되므로 어떤 입력에도 유효한 값을 돌려준다.
pub fn parse_max_concurrent(raw: Option<&str>) -> usize {
    raw.and_then(|s| s.trim().parse::<usize>().ok())
        .map(clamp_max_concurrent)
        .unwrap_or(DEFAULT_MAX_CONCURRENT)
}

#[tauri::command]
pub fn max_concurrent_get(state: State<AppState>) -> ConcurrencyLimit {
    ConcurrencyLimit::of(state.max_concurrent.load(Ordering::Relaxed))
}

/// 동시 실행 상한 저장 — 범위로 접은 뒤 즉시 적용하고 DB에 남긴다. 접힌 실제 값을 돌려주므로
/// UI는 반환값으로 입력칸을 갱신하면 된다(요청값을 그대로 믿으면 화면과 실제가 어긋난다).
///
/// 이미 상한을 넘겨 실행 중인 작업은 건드리지 않는다 — 낮추면 새 생성만 막히고 초과분은
/// 자연 종료로 흡수된다.
#[tauri::command]
pub async fn max_concurrent_set(
    state: State<'_, AppState>,
    value: usize,
) -> Result<ConcurrencyLimit, String> {
    let applied = clamp_max_concurrent(value);
    let pool = pool_of(&state)?;
    db::set_setting(&pool, "max_concurrent", &applied.to_string())
        .await
        .map_err(|e| e.to_string())?;
    state.max_concurrent.store(applied, Ordering::Relaxed);
    Ok(ConcurrencyLimit::of(applied))
}

#[derive(Serialize)]
pub struct ShellSpec {
    pub cmd: String,
    pub args: Vec<String>,
}

/// 플랫폼 기본 셸 (unix=$SHELL 또는 zsh + 로그인, windows=powershell). 프론트가 작업 생성 시 사용.
#[tauri::command]
pub fn default_shell() -> ShellSpec {
    #[cfg(windows)]
    return ShellSpec {
        cmd: "powershell.exe".into(),
        args: vec![],
    };
    #[cfg(unix)]
    return ShellSpec {
        cmd: std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into()),
        args: vec!["-l".into()],
    };
    #[cfg(not(any(unix, windows)))]
    return ShellSpec {
        cmd: "/bin/sh".into(),
        args: vec![],
    };
}

/// 공급자 로컬 세션 사용량과 Praxis 주 대화 접수 입력 메타데이터를 집계한다.
/// range: "all" | "30d" | "7d". tz_offset_secs: 로컬 UTC 오프셋(KST=32400, 프론트 제공).
/// 공급자 파일 IO는 blocking 풀로, 앱 입력 메타데이터는 기존 대화 원장에서 읽는다.
#[tauri::command]
pub async fn insights_compute(
    state: State<'_, AppState>,
    range: String,
    tz_offset_secs: i64,
) -> Result<insights::Insights, String> {
    let pool = pool_of(&state)?;
    // A metrics read failure must not hide provider usage or masquerade as zero.
    let prompt_injection = crate::convo::prompt_metrics::summarize(&pool, &range, now()).await.ok();
    let mut result = tauri::async_runtime::spawn_blocking(move || insights::compute(&range, tz_offset_secs))
        .await
        .map_err(|e| e.to_string())?;
    result.prompt_injection = prompt_injection;
    Ok(result)
}

/// 에이전트 × 스킬 사용 통계 — 메인 세션 + 서브에이전트 트랜스크립트 스캔.
/// 서브에이전트까지 훑어 `insights_compute`보다 파일이 많으므로 별도 커맨드로 두고 화면에서 따로 로드한다.
#[tauri::command]
pub async fn insights_agent_skills(
    range: String,
    tz_offset_secs: i64,
) -> Result<insights::AgentSkillUsage, String> {
    tauri::async_runtime::spawn_blocking(move || {
        insights::compute_agent_skills(&range, tz_offset_secs)
    })
    .await
    .map_err(|e| e.to_string())
}

/// 벤더별 사용 한도 잔량 스냅샷 — 하단 상태바용.
/// force=true면 OAuth 재조회 간격 캐시를 건너뛴다(수동 새로고침).
#[tauri::command]
pub async fn usage_snapshot(force: bool) -> crate::usage::UsageSnapshot {
    crate::usage::snapshot(force).await
}

/// 벤더 CLI의 인증 상태·설치 버전·업데이트 가용성 — 설정 패널과 상태바용.
/// force=true면 npm registry 6시간 캐시를 건너뛴다(수동 새로고침).
#[tauri::command]
pub async fn agent_health(force: bool) -> crate::agenthealth::HealthSnapshot {
    crate::agenthealth::snapshot(force).await
}

/// 시작 시 자동 업데이트 on/off. 미설정은 꺼짐이다.
#[tauri::command]
pub async fn auto_update_get(state: State<'_, AppState>) -> Result<bool, String> {
    let pool = pool_of(&state)?;
    let raw = db::get_setting(&pool, crate::agenthealth::autoupdate::SETTING_KEY)
        .await
        .map_err(|e| e.to_string())?;
    Ok(crate::agenthealth::autoupdate::enabled_from(raw.as_deref()))
}

/// 자동 업데이트 on/off 저장. 다음 시작부터 적용된다.
#[tauri::command]
pub async fn auto_update_set(state: State<'_, AppState>, enabled: bool) -> Result<bool, String> {
    let pool = pool_of(&state)?;
    db::set_setting(
        &pool,
        crate::agenthealth::autoupdate::SETTING_KEY,
        crate::agenthealth::autoupdate::setting_value(enabled),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(enabled)
}

/// 이번 실행에서 자동 업데이트가 무엇을 했는지.
///
/// 이벤트를 놓친 뒤에도 읽을 수 있어야 한다 — 업데이트는 설정 패널이 열리기 한참 전에 끝난다.
#[tauri::command]
pub fn auto_update_last(
    state: State<AppState>,
) -> crate::agenthealth::autoupdate::AutoUpdateReport {
    state
        .last_autoupdate
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// Antigravity Hub가 재시작을 기다리는 업데이트를 갖고 있는가.
///
/// 설치하지 않는다 — Hub의 설치는 앱 종료를 요구하고, 남의 앱을 강제 종료하면 사용자가
/// 그 앱에서 하던 일이 사라진다. 여기서는 사실만 전한다.
///
/// 파일 두 개를 읽으므로 `spawn_blocking`으로 보낸다(이 모듈의 다른 조사와 같은 방식).
#[tauri::command]
pub async fn antigravity_hub_update() -> crate::agenthealth::antigravity::HubUpdate {
    tokio::task::spawn_blocking(crate::agenthealth::antigravity::detect)
        .await
        .unwrap_or_default()
}

/// 로그인이 회복된 벤더의 인증 차단을 푼다. 반환: 다시 대기열로 돌아간 작업 수.
///
/// 로그인 액션이 끝난 직후 프론트가 부른다. 작업은 `Queued`를 떠난 적이 없으므로
/// 컬럼을 비우는 것만으로 다음 lease가 집어간다 — 재개 전이가 따로 없다.
#[tauri::command]
pub async fn agent_auth_reconcile(state: State<'_, AppState>) -> Result<u64, String> {
    let pool = pool_of(&state)?;
    crate::agenthealth::reconcile_blocks(&pool)
        .await
        .map_err(|error| error.to_string())
}

/// Claude statusline 브리지 설치 상태.
#[tauri::command]
pub fn usage_bridge_status() -> Result<crate::usage::bridge::BridgeStatus, String> {
    let home =
        crate::usage::home_dir().ok_or_else(|| "홈 디렉터리를 찾지 못했습니다".to_string())?;
    Ok(crate::usage::bridge::status(&home))
}

/// 브리지 설치 — 사용자의 `~/.claude/settings.json`을 수정하므로 UI의 명시적 액션에서만 호출한다.
#[tauri::command]
pub fn usage_bridge_install() -> Result<crate::usage::bridge::BridgeStatus, String> {
    let home =
        crate::usage::home_dir().ok_or_else(|| "홈 디렉터리를 찾지 못했습니다".to_string())?;
    crate::usage::bridge::install(&home)
}

/// 브리지 제거 — 감싸 두었던 원래 statusLine 명령을 되돌린다.
#[tauri::command]
pub fn usage_bridge_uninstall() -> Result<crate::usage::bridge::BridgeStatus, String> {
    let home =
        crate::usage::home_dir().ok_or_else(|| "홈 디렉터리를 찾지 못했습니다".to_string())?;
    crate::usage::bridge::uninstall(&home)
}

/// 사용량 조회 전용 장기 토큰 저장 — 실제 조회로 검증한 뒤에만 키체인에 넣는다.
/// 성공하면 그 조회 결과를 그대로 돌려줘 UI가 즉시 값을 보여줄 수 있다.
#[tauri::command]
pub async fn usage_claude_token_set(token: String) -> Result<crate::usage::VendorUsage, String> {
    crate::usage::set_manual_token(token).await
}

/// 저장된 조회용 토큰 삭제 — 다음 조회는 CLI 자격증명으로 돌아간다.
#[tauri::command]
pub async fn usage_claude_token_clear() -> Result<(), String> {
    crate::usage::clear_manual_token().await
}

/// 조회용 토큰 저장 여부. 토큰 값은 어떤 경우에도 돌려주지 않는다.
#[tauri::command]
pub async fn usage_claude_token_status() -> Result<bool, String> {
    crate::usage::manual_token_stored().await
}

/// 작업 DB 기반 AX 결과 집계. 사용량 통계와 독립적으로 실패·갱신한다.
#[tauri::command]
pub async fn outcome_insights(
    state: State<'_, AppState>,
    range: String,
) -> Result<insights::OutcomeInsights, String> {
    let pool = pool_of(&state)?;
    insights::compute_outcomes(&pool, &range, now())
        .await
        .map_err(|error| error.to_string())
}

// ── 인사이트 재구성 (설계 0054) ──

/// 작업 패턴 집계 — 퍼널·폐기 추세·후속 입력·역할별 결말·소요.
///
/// `tz_offset_secs`를 받는 이유는 월 버킷이 로컬 기준이어야 하기 때문이다. UTC로 자르면
/// 월 경계 근처 작업이 옆 달로 새고, 그러면 폐기율 추세가 실제와 어긋난다.
#[tauri::command]
pub async fn task_patterns(
    state: State<'_, AppState>,
    range: String,
    tz_offset_secs: i64,
) -> Result<insights::TaskPatterns, String> {
    let pool = pool_of(&state)?;
    insights::compute_patterns(&pool, &range, tz_offset_secs, now())
        .await
        .map_err(|error| error.to_string())
}

/// 신호 카드(설계 §5, T2·T3) — S1(한 번 보고 닫은 작업) · S2(저장소 집중) · S6(토큰 사용량 급증) ·
/// L1(교훈 주제 반복).
///
/// "all" 구간은 직전 구간이 없어 S1·S6을 낼 수 없다 — 이때는 `insights::compute`의 무거운
/// 파일 스캔(`spawn_blocking`)을 아예 건너뛴다. L1은 변화가 아니라 반복을 재는 신호라 "all"에서도
/// 낸다(§9 T3) — 교훈 투영 읽기(파일 IO)만 `spawn_blocking`으로 보낸다.
#[tauri::command]
pub async fn insight_cards(
    state: State<'_, AppState>,
    range: String,
    tz_offset_secs: i64,
) -> Result<Vec<insights::InsightCard>, String> {
    let pool = pool_of(&state)?;
    let now = now();
    let spend = if range == "all" {
        None
    } else {
        let range_for_blocking = range.clone();
        let snapshot = tauri::async_runtime::spawn_blocking(move || {
            insights::compute(&range_for_blocking, tz_offset_secs)
        })
        .await
        .map_err(|e| e.to_string())?;
        Some(insights::SpendSnapshot::from_insights(&snapshot))
    };
    let repos = db::known_repos(&pool).await.map_err(|e| e.to_string())?;
    let known_repos = repos.clone();
    let projections = tauri::async_runtime::spawn_blocking(move || {
        insights::load_lesson_projections(&repos)
    })
    .await
    .map_err(|e| e.to_string())?;
    insights::compute_cards(
        &pool,
        &range,
        tz_offset_secs,
        now,
        spend.as_ref(),
        &projections,
        &known_repos,
    )
    .await
    .map_err(|error| error.to_string())
}

/// 발견 카드 무시(설계 §6.4·§8, T4) — `(signal, repo)` 쌍을 7일 동안 가라앉힌다.
/// 저장소에 묶이지 않은 카드(S1)는 `repo`가 `None`으로 온다.
#[tauri::command]
pub async fn insight_card_dismiss(
    state: State<'_, AppState>,
    signal: insights::SignalId,
    repo: Option<String>,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    insights::dismiss_card(&pool, signal.as_str(), repo.as_deref().unwrap_or(""), now())
        .await
        .map_err(|error| error.to_string())
}

/// 교훈 주제(설계 §6, T3) — 저장소별 subject 빈도 · 직전 구간 대비 증감 · 버린 길 목록.
///
/// 교훈 투영(`docs/lessons.json`)이 없는 저장소는 조용히 빠진다(§6.3) — 빈 "0" 상태를
/// 그리지 않는다(ledger #554). 파일 IO(투영 읽기)만 `spawn_blocking`으로 보낸다.
#[tauri::command]
pub async fn lesson_themes(
    state: State<'_, AppState>,
    range: String,
    tz_offset_secs: i64,
) -> Result<Vec<insights::RepoLessons>, String> {
    let pool = pool_of(&state)?;
    let now = now();
    let repos = db::known_repos(&pool).await.map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let projections = insights::load_lesson_projections(&repos);
        insights::compute_lesson_themes(&projections, &range, now, tz_offset_secs)
    })
    .await
    .map_err(|e| e.to_string())
}

/// 레포의 스킬 목록 (프로젝트 + 글로벌, 이름순). 동기 파일 IO.
#[tauri::command]
pub fn skills_list(repo: String) -> Vec<crate::skills::SkillMeta> {
    crate::skills::list_skills(&repo)
}

/// 스킬 파일 전체 내용 반환. 동기 파일 IO.
#[tauri::command]
pub fn skills_read(repo: String, name: String) -> Result<String, String> {
    crate::skills::read_skill(&repo, &name)
}

/// 등록된 로컬 프로젝트의 고정 경험 문서 목록. 파일 IO는 UI 런타임 밖에서만 수행한다.
#[tauri::command]
pub async fn harness_experience_list(
    state: State<'_, AppState>,
    repo: String,
    harness: String,
) -> Result<crate::skills::experience::ListResult, String> {
    let Some(harness) = crate::skills::experience::HarnessName::parse(&harness) else {
        return Ok(crate::skills::experience::ListResult::Error {
            error: crate::skills::experience::RequestError::InvalidRequest,
        });
    };
    let pool = pool_of(&state)?;
    let known = db::known_repos(&pool).await.map_err(|error| error.to_string())?;
    let roots = tauri::async_runtime::spawn_blocking(move || experience_roots(&repo, &known))
        .await
        .map_err(|error| error.to_string())?;
    let Some((project_root, home)) = roots? else {
        return Ok(crate::skills::experience::ListResult::Error {
            error: crate::skills::experience::RequestError::UnregisteredProject,
        });
    };
    tauri::async_runtime::spawn_blocking(move || {
        Ok::<_, String>(crate::skills::experience::list(
            &project_root,
            &home,
            harness,
        ))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// 허용 목록의 위치자만 다시 해석해 경험 원문을 읽는다.
#[tauri::command]
pub async fn harness_experience_read(
    state: State<'_, AppState>,
    repo: String,
    owner: crate::skills::experience::DocumentOwner,
    document_key: String,
) -> Result<crate::skills::experience::ReadResult, String> {
    let pool = pool_of(&state)?;
    let known = db::known_repos(&pool).await.map_err(|error| error.to_string())?;
    let roots = tauri::async_runtime::spawn_blocking(move || experience_roots(&repo, &known))
        .await
        .map_err(|error| error.to_string())?;
    let Some((project_root, home)) = roots? else {
        return Ok(crate::skills::experience::ReadResult::Error {
            error: crate::skills::experience::ReadErrorOrRequest::Request(
                crate::skills::experience::RequestError::UnregisteredProject,
            ),
        });
    };
    tauri::async_runtime::spawn_blocking(move || {
        Ok::<_, String>(crate::skills::experience::read(
            &project_root,
            &home,
            owner,
            &document_key,
        ))
    })
    .await
    .map_err(|error| error.to_string())?
}

fn experience_roots(repo: &str, known: &[String]) -> Result<Option<(PathBuf, PathBuf)>, String> {
    let Some(project_root) = crate::skills::experience::registered_root(repo, known) else {
        return Ok(None);
    };
    let home = crate::usage::home_dir()
        .ok_or_else(|| "홈 디렉터리를 찾을 수 없습니다".to_string())?
        .canonicalize()
        .map_err(|error| format!("홈 디렉터리를 확인할 수 없습니다: {error}"))?;
    Ok(Some((project_root, home)))
}

/// 멀티벤더 리뷰가 지원하는 벤더 화이트리스트 (reviewer::invocation과 동일 범위).
const MULTIREVIEW_VENDORS: [&str; 4] = ["claude", "codex", "agy", "gemini"];

/// 멀티벤더 리뷰 결과 항목 — 벤더별 성공 여부 + 리뷰(또는 에러) 텍스트.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiReviewItem {
    pub vendor: String,
    pub ok: bool,
    pub text: String,
}

/// 멀티벤더 리뷰 결과 — 항목들 + 선택적 종합. `reviews` 테이블에 result_json으로 영속화.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiReviewResult {
    pub items: Vec<MultiReviewItem>,
    pub synthesis: Option<String>,
    /// 파이프라인 리뷰의 구조화 데이터. 일반 멀티 리뷰는 채우지 않고, 읽을 때 그대로 통과시킨다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor_reviews: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synthesis_structured: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocking: Option<bool>,
}

/// source_kind별 리뷰 콘텐츠 준비. plan은 repo 하위로 제한(fsapi 트래버설 가드),
/// diff는 task worktree에서 patch 합성, text는 그대로.
async fn multi_review_content(
    pool: &SqlitePool,
    repo: &str,
    source_kind: &str,
    source_ref: &str,
) -> Result<String, String> {
    match source_kind {
        "plan" => {
            let root = PathBuf::from(repo);
            fsapi::read_file(&root, source_ref)
                .map(|fc| fc.content)
                .map_err(|e| format!("계획 문서 읽기 실패: {e}"))
        }
        "diff" => {
            let id: i64 = source_ref
                .trim()
                .parse()
                .map_err(|_| "diff 소스는 유효한 task id여야 합니다".to_string())?;
            let task = db::get_task(pool, id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("작업을 찾을 수 없습니다")?;
            let wt = worktree_from_task(&task);
            if !wt.path.is_dir() {
                return Err("워크트리가 없습니다(종료된 작업일 수 있음)".into());
            }
            let files = wt.diff_detailed().map_err(|e| e.to_string())?;
            Ok(files
                .iter()
                .map(|f| f.patch.clone())
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "text" => Ok(source_ref.to_string()),
        _ => Err(format!("알 수 없는 source_kind: {source_kind}")),
    }
}

/// 벤더 하나에 대한 헤드리스 리뷰 실행(블로킹) — 화이트리스트 밖이면 ok=false로 즉시 반환.
fn run_one_review(vendor: String, prompt: String) -> MultiReviewItem {
    if !MULTIREVIEW_VENDORS.contains(&vendor.as_str()) {
        return MultiReviewItem {
            vendor,
            ok: false,
            text: "지원하지 않는 벤더입니다".into(),
        };
    }
    match crate::reviewer::run_reviewer(&vendor, &prompt, 120) {
        Ok(text) => MultiReviewItem {
            vendor,
            ok: true,
            text,
        },
        Err(e) => MultiReviewItem {
            vendor,
            ok: false,
            text: e,
        },
    }
}

/// 멀티벤더 리뷰 반환 타입.
#[derive(Debug, Clone, Serialize)]
pub struct MultiReviewRun {
    pub result: MultiReviewResult,
    pub detail: crate::multireview::ReviewDetail,
}

/// 멀티벤더 리뷰: 계획/diff/텍스트를 여러 벤더가 read-only로 병렬 리뷰 + 선택적 종합.
/// 결과는 `reviews` 테이블에 best-effort 영속화(저장 실패해도 결과는 그대로 반환).
#[tauri::command]
pub async fn multi_review(
    state: State<'_, AppState>,
    repo: String,
    source_kind: String,
    source_ref: String,
    focus: String,
    vendors: Vec<String>,
    synthesize: bool,
) -> Result<MultiReviewRun, String> {
    let pool = pool_of(&state)?;
    let content = multi_review_content(&pool, &repo, &source_kind, &source_ref).await?;
    if content.trim().is_empty() {
        return Err("리뷰할 콘텐츠가 비어 있습니다".into());
    }

    // 실행당 nonce 1개 — 모든 벤더가 동일 프롬프트 사용.
    let nonce = format!("PRAXIS-REVIEW-{}-{:016x}", std::process::id(), rand_u64());
    let prompt_review = crate::multireview::build_review_prompt(&focus, &content, &nonce);

    // 벤더별 model 정보 사전 수집.
    let mut model_infos: HashMap<String, crate::multireview::ModelInfo> = HashMap::new();
    for vendor in &vendors {
        if crate::multireview::MULTIREVIEW_VENDORS.contains(&vendor.as_str()) {
            let model = agent_model_of(&pool, vendor).await.unwrap_or_default();
            let cmd = crate::reviewer::describe_invocation(vendor);
            model_infos.insert(
                vendor.clone(),
                crate::multireview::ModelInfo {
                    vendor: vendor.clone(),
                    model,
                    cmd,
                },
            );
        }
    }

    // 벤더별 병렬 리뷰 실행.
    let mut handles = Vec::with_capacity(vendors.len());
    for vendor in vendors {
        let prompt = prompt_review.clone();
        handles.push(tauri::async_runtime::spawn_blocking(move || {
            run_one_review(vendor, prompt)
        }));
    }
    let mut items = Vec::with_capacity(handles.len());
    for h in handles {
        items.push(h.await.map_err(|e| e.to_string())?);
    }

    // 종합 판정 (선택적).
    let ok_reviews: Vec<(String, String)> = items
        .iter()
        .filter(|i| i.ok)
        .map(|i| (i.vendor.clone(), i.text.clone()))
        .collect();
    let (synthesis, prompt_synthesis_str, synthesis_model) = if synthesize && ok_reviews.len() >= 2
    {
        let judge = {
            let ok_vendors: std::collections::HashSet<&str> =
                ok_reviews.iter().map(|(v, _)| v.as_str()).collect();
            let d = crate::reviewer::detect_reviewer("");
            if ok_vendors.contains(d.as_str()) {
                ok_reviews[0].0.clone()
            } else {
                d
            }
        };
        let nonce_synth = format!("PRAXIS-SYNTH-{}-{:016x}", std::process::id(), rand_u64());
        let prompt_synthesis =
            crate::multireview::build_synthesis_prompt(&focus, &ok_reviews, &nonce_synth);
        let prompt_for_storage = prompt_synthesis.clone();
        let synth_result = tauri::async_runtime::spawn_blocking({
            let judge_clone = judge.clone();
            move || crate::reviewer::run_reviewer(&judge_clone, &prompt_synthesis, 120)
        })
        .await
        .ok()
        .and_then(|r| r.ok());

        let synth_model = if synth_result.is_some() {
            let model = agent_model_of(&pool, &judge).await.unwrap_or_default();
            let cmd = crate::reviewer::describe_invocation(&judge);
            Some(crate::multireview::ModelInfo {
                vendor: judge,
                model,
                cmd,
            })
        } else {
            None
        };
        (synth_result, Some(prompt_for_storage), synth_model)
    } else {
        (None, None, None)
    };

    let result = MultiReviewResult { items, synthesis, vendor_reviews: None, synthesis_structured: None, blocking: None };
    let detail = crate::multireview::ReviewDetail {
        content,
        prompt_review,
        prompt_synthesis: prompt_synthesis_str,
        model_info: model_infos.values().cloned().collect(),
        synthesis_model,
    };
    persist_review(
        &pool,
        &repo,
        &source_kind,
        &source_ref,
        &focus,
        &result,
        &detail,
    )
    .await;
    Ok(MultiReviewRun { result, detail })
}

/// 리뷰 결과 영속화(best-effort) — 실패해도 호출측 흐름(결과 반환)을 막지 않는다.
async fn persist_review(
    pool: &SqlitePool,
    repo: &str,
    source_kind: &str,
    source_ref: &str,
    focus: &str,
    result: &MultiReviewResult,
    detail: &crate::multireview::ReviewDetail,
) {
    let ok_count = result.items.iter().filter(|i| i.ok).count() as i64;
    let total = result.items.len() as i64;
    let result_json = match serde_json::to_string(result) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("리뷰 결과 직렬화 실패: {e}");
            return;
        }
    };
    let model_info_json =
        match serde_json::to_string(&(detail.model_info.clone(), detail.synthesis_model.clone())) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("모델 정보 직렬화 실패: {e}");
                return;
            }
        };
    if let Err(e) = crate::multireview::insert_review(
        pool,
        now(),
        repo,
        source_kind,
        source_ref,
        focus,
        &result_json,
        ok_count,
        total,
        &detail.content,
        &detail.prompt_review,
        detail.prompt_synthesis.as_deref(),
        &model_info_json,
    )
    .await
    {
        eprintln!("리뷰 결과 저장 실패: {e}");
    }
}

/// 리뷰 이력 + 결과 + 상세 정보 묶음 — 프론트가 한 번에 받을 수 있게 감싼 반환 타입.
#[derive(Debug, Clone, Serialize)]
pub struct ReviewRecord {
    pub meta: crate::multireview::ReviewMeta,
    pub result: MultiReviewResult,
    pub detail: crate::multireview::ReviewDetail,
}

/// 리뷰 이력 목록(최신순).
#[tauri::command]
pub async fn review_history_list(
    state: State<'_, AppState>,
) -> Result<Vec<crate::multireview::ReviewMeta>, String> {
    let pool = pool_of(&state)?;
    crate::multireview::list_reviews(&pool)
        .await
        .map_err(|e| e.to_string())
}

/// 단일 리뷰 이력 조회 — 메타 + 역직렬화된 결과 + 상세 정보.
#[tauri::command]
pub async fn review_get(state: State<'_, AppState>, id: i64) -> Result<ReviewRecord, String> {
    let pool = pool_of(&state)?;
    let (meta, result_json, detail) = crate::multireview::get_review(&pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("리뷰를 찾을 수 없습니다")?;
    let result: MultiReviewResult =
        serde_json::from_str(&result_json).map_err(|e| format!("리뷰 결과 파싱 실패: {e}"))?;
    Ok(ReviewRecord {
        meta,
        result,
        detail,
    })
}

/// 리뷰 이력 삭제.
#[tauri::command]
pub async fn review_delete(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let pool = pool_of(&state)?;
    crate::multireview::delete_review(&pool, id)
        .await
        .map_err(|e| e.to_string())
}

/// gh 미설치/미인증·비 GitHub 레포를 프론트가 구분해 처리하도록 태그된 응답(PRD F-07 —
/// "조용한 비활성"). `Err`는 실제 명령 실패(예: rate limit)일 때만 사용한다.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum GithubIssuesResult {
    /// `owner_repo`는 목록이 어느 레포의 것인지 화면에 밝히기 위한 것이다 — 경로만으로는
    /// 워크트리·동명 디렉터리를 구분할 수 없다.
    Ready {
        owner_repo: String,
        issues: Vec<github::GhIssue>,
    },
    Unavailable,
    NotGithubRepo,
}

/// GitHub 이슈 목록(HomeView 섹션, C-2). 비 GitHub 레포는 `NotGithubRepo`(섹션 숨김),
/// gh 미설치/미인증은 `Unavailable`(안내 카드)로 반환 — 둘 다 에러 토스트를 띄우지 않는다.
#[tauri::command]
pub async fn github_issues_list(repo: String) -> Result<GithubIssuesResult, String> {
    // `gh` CLI(네트워크 요청) 동기 대기 — blocking 풀로 분리해 IPC 스레드를 막지 않는다.
    tauri::async_runtime::spawn_blocking(move || {
        let path = Path::new(&repo);
        let Some(owner_repo) = github::remote_owner_repo(path) else {
            return Ok(GithubIssuesResult::NotGithubRepo);
        };
        match github::list_issues(path) {
            Ok(issues) => Ok(GithubIssuesResult::Ready { owner_repo, issues }),
            Err(github::GhError::GhUnavailable) => Ok(GithubIssuesResult::Unavailable),
            Err(e @ github::GhError::CommandFailed(_)) => Err(e.to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 후보 경로 중 이슈를 볼 수 있는 레포만 추린다(홈의 레포 전환 버튼). 프론트가 최근 작업에서
/// 뽑은 경로 목록을 그대로 넘기면, 여기서 GitHub 레포만 걸러 `owner/repo`와 함께 돌려준다.
#[tauri::command]
pub async fn github_repos_list(repos: Vec<String>) -> Result<Vec<github::GhRepo>, String> {
    // `git remote get-url`을 경로마다 띄우므로 blocking 풀로 분리한다(네트워크는 타지 않음).
    tauri::async_runtime::spawn_blocking(move || github::resolve_repos(&repos))
        .await
        .map_err(|e| e.to_string())
}

/// 이슈 번호로 태스크 생성(C-2) — 지시문은 제목+본문+`#N` 참조, 기존 `create_task_internal`
/// 경로를 그대로 재사용(origin=External → repo 화이트리스트 검증 후 승인 대기로 생성).
/// 생성 성공 시 `owner/repo#N`을 `task_issue_refs`에 저장(태스크 생성 자체 실패와 달리
/// 저장 실패는 태스크 생성을 되돌리지 않고 best-effort로 남긴다 — 참조 메타데이터일 뿐).
#[tauri::command]
pub async fn github_create_task_from_issue(
    app: AppHandle,
    state: State<'_, AppState>,
    repo: String,
    number: u64,
    agent: String,
) -> Result<Task, String> {
    let path = PathBuf::from(&repo);
    let owner_repo = github::remote_owner_repo(&path).ok_or("GitHub 레포가 아닙니다")?;
    let detail = github::view_issue(&path, number).map_err(|e| e.to_string())?;
    let instruction = github::build_instruction(&owner_repo, number, &detail);
    let params =
        CreateTaskParams::headless_terminal(repo.clone(), instruction, agent, TaskOrigin::External);
    let task = create_task_internal(&app, &state, params).await?;
    let pool = pool_of(&state)?;
    let issue_ref = format!("{owner_repo}#{number}");
    let _ = github::set_issue_ref(&pool, task.id, &issue_ref).await;
    Ok(task)
}

/// 이슈 삭제(홈 섹션 C-2). **close가 아니라 완전 삭제**라 되돌릴 수 없고 레포 admin 권한을
/// 요구한다 — 확인은 화면이 받고, 여기서는 권한 부족·미존재를 `Err`로 그대로 올려 행에
/// 사유를 남긴다. 조용히 성공한 척하면 사용자는 지워진 줄 알고 목록을 다시 본다.
#[tauri::command]
pub async fn github_issue_delete(repo: String, number: u64) -> Result<(), String> {
    // `gh` CLI(네트워크 요청) 동기 대기 — blocking 풀로 분리해 IPC 스레드를 막지 않는다.
    tauri::async_runtime::spawn_blocking(move || {
        github::delete_issue(Path::new(&repo), number).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

// 알파벳 순서 밖(신규 커맨드, D-1) — Design Mode 프리뷰 탭. **local 전용** — 원격 Runner에는
// 노출하지 않는다(designs/0012 §6.8: "runner parity 없음"). 외부 dev server는 capture에는 custom
// scheme을, Phase 1 result에는 origin·webview가 제한된 preview-bridge IPC를 각각 사용한다.

/// PreviewTab 컨테이너의 뷰포트 좌표(논리 픽셀) — 자식 웹뷰 위치/크기 동기화에 사용.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct DesignBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EditorCaptureRequest {
    pub bounds: DesignBounds,
    pub file_path: String,
    pub selection_text: Option<String>,
    pub selection_start_line: Option<u32>,
    pub selection_end_line: Option<u32>,
}

type LogicalPoint = (f64, f64);
type DesignModeScreenGeometry = (LogicalPoint, LogicalPoint, LogicalPoint);

/// inject.js가 캡처를 저장한 뒤 프론트에 알리는 이벤트 페이로드.
#[derive(Debug, Clone, Serialize)]
struct DesignCapturePayload {
    task_id: i64,
    record: crate::designmode::CaptureRecord,
}

/// 클립보드 base64 페이로드 디코드 — 디코드 전 문자열 상한(20MB 바이너리의 base64 ≈ 27MB).
fn decode_pasted_image(data_base64: &str) -> Result<Vec<u8>, String> {
    if data_base64.len() > 28 * 1024 * 1024 {
        return Err("이미지가 너무 큽니다 (20MB 상한)".into());
    }
    STANDARD
        .decode(data_base64.as_bytes())
        .map_err(|error| format!("이미지 데이터 디코드 실패: {error}"))
}

/// 작업 컴포저 클립보드 이미지 → 캡처 레코드 저장. Design Mode/에디터 캡처와 같은
/// 파이프라인(칩 표시·전송 시 프롬프트 주입·`image_paths` 검증·종결 정리)을 그대로 탄다.
#[tauri::command]
pub async fn paste_capture_save(
    state: State<'_, AppState>,
    id: i64,
    data_base64: String,
    mime: String,
) -> Result<crate::designmode::CaptureRecord, String> {
    let bytes = decode_pasted_image(&data_base64)?;
    let pool = pool_of(&state)?;
    let task = db::get_task(&pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    crate::designmode::save_pasted_capture(&PathBuf::from(&task.worktree_path), id, &bytes, &mime)
}

/// 홈 컴포저(작업 생성 전) 클립보드 이미지 저장 — 아직 worktree가 없어 캡처 파이프라인을
/// 탈 수 없다. `repo`의 `.praxis/pasted/`에 남기고 절대경로를 돌려주면, 프론트가 지시문
/// 텍스트에 삽입한다(에이전트 CLI가 경로의 파일을 직접 읽는다).
#[tauri::command]
pub fn paste_image_save(repo: String, data_base64: String, mime: String) -> Result<String, String> {
    let bytes = decode_pasted_image(&data_base64)?;
    crate::paste::save_repo_image(&PathBuf::from(&repo), &bytes, &mime)
}

#[cfg(test)]
#[path = "commands/followup_boundary_tests.rs"]
mod followup_boundary_tests;

#[cfg(test)]
#[path = "commands/debate_tests.rs"]
mod debate_tests;

#[cfg(all(test, unix))]
#[path = "commands/question_debate_tests.rs"]
mod question_debate_tests;

#[cfg(test)]
#[path = "commands/resume_history_tests.rs"]
mod resume_history_tests;

#[cfg(test)]
mod slot_reservation_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn try_reserve_count_rejects_when_sum_reaches_cap() {
        let mut reserved = 3usize;
        // active(5) + reserved(3) == max(8) -> 여유 없음, 거부되고 reserved는 그대로.
        let err = try_reserve_count(5, &mut reserved, 8);
        assert!(err.is_err());
        assert_eq!(reserved, 3);
    }

    #[test]
    fn try_reserve_count_accepts_below_cap_and_increments() {
        let mut reserved = 2usize;
        // active(5) + reserved(2) == 7 < max(8) -> 예약 성공, reserved 1 증가.
        assert!(try_reserve_count(5, &mut reserved, 8).is_ok());
        assert_eq!(reserved, 3);
    }

    #[test]
    fn slot_reservation_drop_releases_and_allows_reacquire() {
        let reserved = Mutex::new(0usize);
        {
            let _g = reserve_slot(0, &reserved, 1).expect("첫 예약은 성공");
            // cap=1이고 이미 예약 1개 보유 중 -> 두 번째 예약은 거부.
            assert!(reserve_slot(0, &reserved, 1).is_err());
        } // 가드 drop -> 예약 반납
        assert_eq!(
            *reserved.lock().unwrap(),
            0,
            "가드 drop 후 예약이 반납되어야 함"
        );
        assert!(
            reserve_slot(0, &reserved, 1).is_ok(),
            "반납 후 재예약은 성공해야 함"
        );
    }

    /// 동시(스레드) 예약 시도가 cap을 절대 넘지 않는지 검증 — "동시에 살아있는 예약 수"의
    /// 관측 최대치(peak)가 max_concurrent를 넘지 않아야 하고, 종료 후엔 전부 반납되어 0이어야 함.
    #[test]
    fn reserve_slot_concurrent_never_exceeds_cap() {
        let reserved = Mutex::new(0usize);
        let max_concurrent = 4usize;
        let held = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..50 {
                scope.spawn(|| {
                    if let Ok(guard) = reserve_slot(0, &reserved, max_concurrent) {
                        let now = held.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(now, Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(2));
                        held.fetch_sub(1, Ordering::SeqCst);
                        drop(guard);
                    }
                });
            }
        });
        assert!(
            peak.load(Ordering::SeqCst) <= max_concurrent,
            "동시 보유 예약 최대치가 cap을 초과함: {} > {}",
            peak.load(Ordering::SeqCst),
            max_concurrent
        );
        assert_eq!(
            *reserved.lock().unwrap(),
            0,
            "모든 스레드 종료 후 예약은 0이어야 함(누수 없음)"
        );
    }

    #[test]
    fn parse_max_concurrent_falls_back_and_clamps() {
        assert_eq!(parse_max_concurrent(None), DEFAULT_MAX_CONCURRENT);
        assert_eq!(parse_max_concurrent(Some("")), DEFAULT_MAX_CONCURRENT);
        assert_eq!(parse_max_concurrent(Some("abc")), DEFAULT_MAX_CONCURRENT);
        assert_eq!(parse_max_concurrent(Some("-1")), DEFAULT_MAX_CONCURRENT);
        assert_eq!(parse_max_concurrent(Some(" 16 ")), 16);
        // 범위 밖은 거부가 아니라 경계로 접는다 — 손상된 값 하나로 기동이 막히면 안 된다.
        assert_eq!(parse_max_concurrent(Some("0")), MAX_CONCURRENT_MIN);
        assert_eq!(parse_max_concurrent(Some("9999")), MAX_CONCURRENT_MAX);
    }

    /// 상한을 올리면 그 자리에서 더 많은 예약이 통과해야 한다 — 재시작 없이 반영되는지 확인.
    #[test]
    fn raising_limit_admits_more_reservations() {
        let limit = AtomicUsize::new(DEFAULT_MAX_CONCURRENT);
        let reserved = Mutex::new(0usize);
        // 기본 상한(8)에서 8개가 이미 활성이면 새 예약은 거부.
        assert!(reserve_slot(8, &reserved, limit.load(Ordering::Relaxed)).is_err());
        limit.store(clamp_max_concurrent(20), Ordering::Relaxed);
        assert!(
            reserve_slot(8, &reserved, limit.load(Ordering::Relaxed)).is_ok(),
            "상한을 올린 뒤에는 같은 활성 수에서도 예약이 통과해야 함"
        );
    }

    /// 상한을 활성 수 아래로 낮춰도 예약 경로가 패닉 없이 거부만 한다(실행 중 작업은 그대로).
    #[test]
    fn lowering_limit_below_active_only_blocks_new_reservations() {
        let reserved = Mutex::new(0usize);
        assert!(reserve_slot(10, &reserved, 4).is_err());
        assert_eq!(*reserved.lock().unwrap(), 0, "거부 시 예약은 늘지 않아야 함");
    }
}


/// 슬래시 스킬 확장문 기록(설계 0008 §D) — `start_convo_turn`은 AppHandle 의존이라 직접 단위테스트가
/// 어려워, 이벤트 생성 여부를 가르는 순수 로직만 분리 테스트(부작용측 db::append_convo_event 호출은
/// 기존 convo 통합 테스트들이 이미 검증하는 append_convo_event 자체를 재사용).
#[cfg(test)]
mod convo_expansion_tests {
    use super::*;

    #[test]
    fn expansion_event_none_when_not_expanded() {
        assert!(
            expansion_event(None).is_none(),
            "확장 없으면 추가 이벤트도 없음"
        );
    }

    #[test]
    fn expansion_event_some_when_expanded_carries_kind_and_text() {
        let ev = expansion_event(Some("확장된 본문")).expect("확장됐으면 이벤트 생성");
        let v: serde_json::Value = serde_json::from_str(&ev).unwrap();
        assert_eq!(v["kind"], "user_expanded");
        assert_eq!(v["text"], "확장된 본문");
    }

    #[test]
    fn finalization_rejects_active_conversation_turn() {
        let active = Arc::new(Mutex::new(HashMap::from([(
            42,
            ActiveConvo {
                pgid: Some(4242),
                vendor_bin: "codex".into(),
                started_at: 1,
                last_event_at: 1,
                last_operation: None,
                interrupted: false,
            },
        )])));
        assert!(ensure_convo_idle(&active, 42).is_err());
        assert!(ensure_convo_idle(&active, 7).is_ok());
    }
}

#[cfg(test)]
mod convo_admission_tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    async fn fixture() -> (SqlitePool, Task, String) {
        let sequence = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = crate::testtmp::dir();
        let path = root.join(format!("praxis-convo-admission-{sequence}.sqlite"));
        let worktree = root.join(format!("praxis-convo-admission-worktree-{sequence}"));
        std::fs::create_dir_all(&worktree).unwrap();
        let pool = db::init_pool(path.to_str().unwrap()).await.unwrap();
        let id = db::insert_task(
            &pool,
            "/repo",
            "praxis/admission",
            "main",
            worktree.to_str().unwrap(),
            "admission",
            Some("codex"),
            None,
            "conversation",
            1,
        )
        .await
        .unwrap();
        db::update_state(&pool, id, tstate::AWAITING_REVIEW, 2)
            .await
            .unwrap();
        let task = db::get_task(&pool, id).await.unwrap().unwrap();
        (pool, task, path.to_string_lossy().into_owned())
    }

    fn receipt(task_id: i64) -> (PreviewReceiptAcceptance, String) {
        let workbench = crate::preview_workbench::PreviewWorkbench::with_epoch("epoch".into());
        let context = "bound".to_string();
        let request = workbench.prepare(task_id, &context, 10).unwrap();
        let request_id = request.request_id.clone();
        (
            PreviewReceiptAcceptance {
                workbench,
                request_id,
                context,
            },
            request.request_id,
        )
    }

    #[tokio::test]
    async fn admission_preserves_prompt_metrics_without_changing_user_text() {
        let (pool, task, path) = fixture().await;
        let mut metrics = crate::convo::prompt_metrics::PromptMetrics::new(
            crate::convo::Vendor::Claude, true, true, "요청",
        );
        metrics.finish("요청");
        let event = serde_json::json!({"kind":"user", "text":"요청", "prompt_metrics":metrics}).to_string();
        persist_convo_admission(&pool, &task, ConversationInputOrigin::UserMessage, &event, None, 10, None).await.unwrap();
        let stored: String = sqlx::query_scalar("SELECT event FROM convo_events WHERE task_id=?")
            .bind(task.id).fetch_one(&pool).await.unwrap();
        let stored: serde_json::Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(stored["text"], "요청");
        assert_eq!(stored["prompt_metrics"]["user_bytes"], 6);
        let summary = crate::convo::prompt_metrics::summarize(&pool, "all", 10).await.unwrap();
        assert_eq!(summary.observed_turns, 1);
        assert_eq!(summary.unknown_turns, 0);
        pool.close().await;
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn admission_rolls_back_each_database_boundary_before_receipt_commit() {
        for trigger in [
            "CREATE TRIGGER fail_observation BEFORE INSERT ON task_events WHEN NEW.kind = 'user_followup_input_observed' BEGIN SELECT RAISE(ABORT, 'observation'); END",
            "CREATE TRIGGER fail_state BEFORE UPDATE ON tasks WHEN NEW.state = 'Running' BEGIN SELECT RAISE(ABORT, 'state'); END",
            "CREATE TRIGGER fail_event BEFORE INSERT ON convo_events BEGIN SELECT RAISE(ABORT, 'event'); END",
        ] {
            let (pool, task, path) = fixture().await;
            sqlx::query(trigger).execute(&pool).await.unwrap();
            let active = Arc::new(Mutex::new(HashMap::new()));
            let (receipt, request_id) = receipt(task.id);
            let reservation = reserve_convo_switch(active.clone(), task.id).unwrap();
            let receipt_reservation = reserve_preview_receipt(Some(&receipt), task.id, 10).unwrap();
            let result = admit_convo_turn(
                &pool,
                reservation,
                &task,
                ConversationInputOrigin::UserMessage,
                r#"{"kind":"user","text":"ask"}"#,
                Some(r#"{"kind":"user_expanded","text":"expanded"}"#),
                receipt_reservation,
                ConvoAdmissionAction::ReleaseManualTakeover,
                10,
                None,
            )
            .await;

            assert!(result.is_err());
            assert!(active.lock().unwrap().is_empty());
            assert_eq!(receipt.workbench.receipt(task.id, &request_id).status, crate::preview_workbench::ReceiptStatus::Prepared);
            assert_eq!(db::get_task(&pool, task.id).await.unwrap().unwrap().state, tstate::AWAITING_REVIEW);
            assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM convo_events WHERE task_id = ?").bind(task.id).fetch_one(&pool).await.unwrap(), 0);
            std::fs::remove_file(path).unwrap();
        }
    }

    #[tokio::test]
    async fn purpose_and_user_admission_roll_back_together() {
        let (pool,task,path)=fixture().await;
        let source=crate::notifications::source_id(&pool).await.unwrap();
        let plan=crate::project_plan::ProjectPlan{schema_version:2,source_id:source.clone(),revision:0,repo:task.repo.clone(),objective:"목적".into(),phases:vec![],task_bindings:vec![],dependencies:vec![]};
        let saved=crate::project_plan::save(&pool,&source,&task.repo,plan,0,&source,3).await.unwrap();
        crate::project_plan::bind_initial(&pool,task.id,&crate::project_plan::PurposeSelection{source_id:source.clone(),canonical_repo:task.repo.clone(),plan_revision:saved.revision,phase_id:None},4).await.unwrap();
        let candidate=crate::project_plan::prepare_purpose(&pool,task.id,"round",5).await.unwrap().unwrap();
        sqlx::query("CREATE TRIGGER fail_purpose_event BEFORE INSERT ON convo_events BEGIN SELECT RAISE(ABORT,'event failed'); END").execute(&pool).await.unwrap();
        let event=r#"{"kind":"user","text":"ask","receipt_request_id":"review:atomic"}"#;
        assert!(persist_convo_admission(&pool,&task,ConversationInputOrigin::UserMessage,event,None,6,Some(&candidate)).await.is_err());
        assert!(crate::project_plan::purpose_get(&pool,&source,task.id,"round").await.unwrap().is_none());
        assert_eq!(db::get_task(&pool,task.id).await.unwrap().unwrap().state,tstate::AWAITING_REVIEW);
        sqlx::query("DROP TRIGGER fail_purpose_event").execute(&pool).await.unwrap();
        persist_convo_admission(&pool,&task,ConversationInputOrigin::UserMessage,event,None,6,Some(&candidate)).await.unwrap();
        assert_eq!(crate::project_plan::purpose_get(&pool,&source,task.id,"round").await.unwrap().unwrap().project_objective,"목적");
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM convo_events WHERE task_id=?").bind(task.id).fetch_one(&pool).await.unwrap(),1);
        pool.close().await;std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn admission_commits_receipt_and_releases_only_manual_takeover() {
        for (action, release_manual) in [
            (ConvoAdmissionAction::ReleaseManualTakeover, true),
            (ConvoAdmissionAction::PreserveTakeover, false),
        ] {
            let (pool, task, path) = fixture().await;
            let active = Arc::new(Mutex::new(HashMap::new()));
            let (receipt, request_id) = receipt(task.id);
            let reserved = reserve_convo_switch(active.clone(), task.id).unwrap();
            let receipt_reservation = reserve_preview_receipt(Some(&receipt), task.id, 10).unwrap();
            let (reservation, release) = admit_convo_turn(
                &pool,
                reserved,
                &task,
                ConversationInputOrigin::UserMessage,
                r#"{"kind":"user","text":"ask"}"#,
                None,
                receipt_reservation,
                action,
                10,
                None,
            )
            .await
            .unwrap();

            assert_eq!(release, release_manual);
            assert_eq!(
                receipt.workbench.receipt(task.id, &request_id).status,
                crate::preview_workbench::ReceiptStatus::Accepted
            );
            drop(reservation);
            assert!(active.lock().unwrap().is_empty());
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn reserved_receipt_survives_ttl_until_admission_commits() {
        let workbench = crate::preview_workbench::PreviewWorkbench::with_epoch("epoch".into());
        let request = workbench.prepare(7, "bound", 10).unwrap();
        let reservation = workbench
            .reserve(7, &request.request_id, "bound", 10)
            .unwrap();

        assert_eq!(
            workbench
                .receipt_at(7, &request.request_id, 10 + 86_400)
                .status,
            crate::preview_workbench::ReceiptStatus::Prepared
        );
        reservation.commit();
        assert_eq!(
            workbench.receipt(7, &request.request_id).status,
            crate::preview_workbench::ReceiptStatus::Accepted
        );
    }

    #[test]
    fn invalid_or_expired_receipt_rejects_before_vault_side_effects() {
        let (mut invalid, _) = receipt(7);
        invalid.context = "different".into();
        assert!(reserve_preview_receipt(Some(&invalid), 7, 10).is_err());

        let (expired, _) = receipt(8);
        assert!(reserve_preview_receipt(Some(&expired), 8, 10 + 86_400).is_err());

        let source = include_str!("commands.rs");
        let start = source.find("async fn start_convo_turn(").unwrap();
        let end = source[start..].find("reservation.handoff_to_turn()").unwrap() + start;
        let turn = &source[start..end];
        let receipt = turn.find("reserve_preview_receipt").unwrap();
        for vault_phase in [
            "begin_vault_attempt",
            "begin_conversation_provenance",
            "vault_delivery",
        ] {
            assert!(receipt < turn.find(vault_phase).unwrap());
        }
    }
}

/// 재시작 조정(Plan 0012) — 생존 판정 순수 로직 단위테스트.
/// 세션 모델 해석(model_for_task) — 오버라이드 우선순위·폴백을 in-process SQLite로 검증.
#[cfg(test)]
mod model_resolution_tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    // pid+카운터로 유일성 보장 — 나노초 타임스탬프는 동시 시작 테스트끼리 충돌(DB 공유 오염) 가능.
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_db() -> String {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        crate::testtmp::dir()
            .join(format!("praxis-model-res-{}-{}.db", std::process::id(), n))
            .to_string_lossy()
            .into_owned()
    }

    async fn claude_task(pool: &SqlitePool, model: Option<&str>) -> i64 {
        let id = db::insert_task(
            pool,
            "/r",
            "b",
            "main",
            "/p",
            "i",
            Some("claude"),
            None,
            "conversation",
            1,
        )
        .await
        .unwrap();
        if let Some(m) = model {
            db::set_task_model(pool, id, m).await.unwrap();
        }
        id
    }

    #[tokio::test]
    async fn session_override_beats_vendor_default() {
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        db::set_setting(&pool, "model:claude", "sonnet")
            .await
            .unwrap();
        let id = claude_task(&pool, Some("haiku")).await;
        assert_eq!(
            model_for_task(&pool, id, "claude").await.as_deref(),
            Some("haiku")
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn falls_back_to_vendor_default_when_override_unset() {
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        db::set_setting(&pool, "model:claude", "sonnet")
            .await
            .unwrap();
        let id = claude_task(&pool, None).await;
        assert_eq!(
            model_for_task(&pool, id, "claude").await.as_deref(),
            Some("sonnet")
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn none_when_neither_override_nor_default() {
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        let id = claude_task(&pool, None).await;
        assert_eq!(model_for_task(&pool, id, "claude").await, None);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn blank_override_falls_back_to_vendor_default() {
        // 공백뿐인 오버라이드(stale/edge 데이터)는 무시 — 읽기 쪽 trim 필터가 폴백을 담당.
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        db::set_setting(&pool, "model:claude", "sonnet")
            .await
            .unwrap();
        let id = claude_task(&pool, Some("   ")).await;
        assert_eq!(
            model_for_task(&pool, id, "claude").await.as_deref(),
            Some("sonnet")
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn blank_vendor_default_is_ignored_too() {
        // 설정값이 공백뿐이면 agent_model_of가 None — CLI 기본 모델로 귀결.
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        db::set_setting(&pool, "model:claude", "  ").await.unwrap();
        let id = claude_task(&pool, None).await;
        assert_eq!(model_for_task(&pool, id, "claude").await, None);
        let _ = std::fs::remove_file(&path);
    }

    async fn task_with(pool: &SqlitePool, agent: &str, model: &str, effort: &str) -> i64 {
        let id = db::insert_task(
            pool,
            "/r",
            "b",
            "main",
            "/p",
            "i",
            Some(agent),
            None,
            "conversation",
            1,
        )
        .await
        .unwrap();
        db::set_task_model(pool, id, model).await.unwrap();
        if !effort.is_empty() {
            db::set_task_reasoning_effort(pool, id, effort).await.unwrap();
        }
        id
    }

    #[tokio::test]
    async fn set_task_model_checked_replaces_the_override() {
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        let id = claude_task(&pool, Some("haiku")).await;
        set_task_model_checked(&pool, id, "opus").await.unwrap();
        assert_eq!(
            model_for_task(&pool, id, "claude").await.as_deref(),
            Some("opus")
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn model_change_persists_one_context_invalidation() {
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        let id = claude_task(&pool, Some("haiku")).await;
        set_task_model_checked(&pool, id, "opus").await.unwrap();
        set_task_model_checked(&pool, id, "opus").await.unwrap();
        assert_eq!(
            db::list_convo_events(&pool, id).await.unwrap(),
            vec![r#"{"kind":"context_usage","context_tokens":0,"source":"model_change","valid":false}"#]
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn failed_context_invalidation_rolls_back_model_and_effort() {
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        let id = task_with(&pool, "codex", "gpt-5.6-sol", "ultra").await;
        sqlx::query("DROP TABLE convo_events")
            .execute(&pool)
            .await
            .unwrap();
        assert!(set_task_model_checked(&pool, id, "gpt-5.5").await.is_err());
        let task = db::get_task(&pool, id).await.unwrap().unwrap();
        assert_eq!(task.model.as_deref(), Some("gpt-5.6-sol"));
        assert_eq!(task.reasoning_effort.as_deref(), Some("ultra"));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn set_task_model_checked_trims_before_storing() {
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        let id = claude_task(&pool, None).await;
        set_task_model_checked(&pool, id, "  opus  ").await.unwrap();
        assert_eq!(
            model_for_task(&pool, id, "claude").await.as_deref(),
            Some("opus")
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn blank_model_releases_the_override_to_the_vendor_default() {
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        db::set_setting(&pool, "model:claude", "sonnet")
            .await
            .unwrap();
        let id = claude_task(&pool, Some("haiku")).await;
        set_task_model_checked(&pool, id, "").await.unwrap();
        assert_eq!(
            model_for_task(&pool, id, "claude").await.as_deref(),
            Some("sonnet")
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn set_task_model_checked_rejects_unknown_task() {
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        assert!(set_task_model_checked(&pool, 9999, "opus").await.is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn set_task_model_checked_rejects_custom_agent() {
        // 커스텀 에이전트는 agent_args의 커스텀 분기가 model을 싣지 않는다 — 저장해도 CLI에 닿지 못한다.
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        let id = task_with(&pool, "mybin --foo", "", "").await;
        assert!(set_task_model_checked(&pool, id, "opus").await.is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn switching_to_a_model_without_that_effort_clears_the_effort() {
        // gpt-5.6-sol만 ultra를 받는다. 그대로 두면 다음 턴이 -m gpt-5.5 -c ultra로 나가 거부된다.
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        let id = task_with(&pool, "codex", "gpt-5.6-sol", "ultra").await;
        set_task_model_checked(&pool, id, "gpt-5.5").await.unwrap();
        let task = db::get_task(&pool, id).await.unwrap().unwrap();
        assert_eq!(task.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(task.reasoning_effort.as_deref().unwrap_or_default(), "");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn a_compatible_effort_survives_the_switch() {
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        let id = task_with(&pool, "codex", "gpt-5.6-sol", "high").await;
        set_task_model_checked(&pool, id, "gpt-5.5").await.unwrap();
        let task = db::get_task(&pool, id).await.unwrap().unwrap();
        assert_eq!(task.reasoning_effort.as_deref(), Some("high"));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn same_model_still_clears_an_incompatible_effort() {
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        let id = task_with(&pool, "codex", "gpt-5.5", "ultra").await;
        set_task_model_checked(&pool, id, "gpt-5.5").await.unwrap();
        let task = db::get_task(&pool, id).await.unwrap().unwrap();
        assert_eq!(task.reasoning_effort.as_deref().unwrap_or_default(), "");
        assert!(db::list_convo_events(&pool, id).await.unwrap().is_empty());
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod agent_switch_tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn paths() -> (String, PathBuf) {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = crate::testtmp::dir();
        (
            root.join(format!("agent-switch-{n}.db"))
                .to_string_lossy()
                .into_owned(),
            root.join(format!("agent-switch-wt-{n}")),
        )
    }

    async fn task(pool: &SqlitePool, worktree: &Path, agent: &str) -> i64 {
        std::fs::create_dir_all(worktree).unwrap();
        let id = db::insert_task(
            pool,
            "/repo",
            "praxis/switch",
            "main",
            worktree.to_str().unwrap(),
            "전환 테스트",
            Some(agent),
            None,
            "conversation",
            1,
        )
        .await
        .unwrap();
        db::update_state(pool, id, tstate::AWAITING_REVIEW, 2).await.unwrap();
        db::set_convo_session(pool, id, "old-session").await.unwrap();
        id
    }

    fn active() -> ActiveConvos {
        Arc::new(Mutex::new(HashMap::new()))
    }

    async fn event(pool: &SqlitePool, id: i64, value: serde_json::Value, ts: i64) {
        db::append_convo_event(pool, id, &value.to_string(), ts).await.unwrap();
    }

    #[tokio::test]
    async fn question_binding_supports_switch_and_debate_and_preserves_answers() {
        use crate::convo::interaction as ledger;
        let (path, worktree) = paths();
        let pool = db::init_pool(&path).await.unwrap();
        let id = task(&pool, &worktree, "codex").await;
        ledger::bind(&pool, id).await.unwrap();
        let execution = ledger::begin(&pool, id, 100).await.unwrap();
        ledger::started(&pool, &execution, "old-session", "turn").await.unwrap();
        let question = ledger::open(&pool, &execution, &serde_json::json!(1), "call", &serde_json::json!({
            "kind":"clarification", "questions":[{"id":"scope","question":"변경 범위?", "options":[{"id":"keep","label":"질문 응답 유지","description":""}], "allow_free_text":true,"is_secret":false}]
        }), 100).await.unwrap();
        ledger::submit(&pool, id, &execution, &question, "answer", &[ledger::Answer {
            question_id: "scope".into(), option_id: Some("keep".into()), text: None,
        }], 101).await.unwrap();
        ledger::take_dispatch(&pool, &execution, 102).await.unwrap().unwrap();
        ledger::written(&pool, "answer").await.unwrap();
        ledger::settle(&pool, &execution, "call").await.unwrap();
        ledger::finish(&pool, &execution, "completed", None).await.unwrap();

        let active = active();
        let (switched, _) = switch_task_agent_checked(&pool, active.clone(), id, "claude", "sonnet").await.unwrap();
        assert!(active.lock().unwrap().is_empty());
        assert_eq!(ledger::runtime_of(&pool, id).await.unwrap().as_deref(), Some(ledger::RUNTIME_LOCAL));
        assert!(switched.convo_session_id.is_none());
        assert!(switched.pending_capsule.as_deref().unwrap().contains("질문 응답 유지"));
        let snapshot = ledger::snapshot(&pool, id).await.unwrap();
        assert_eq!(snapshot.items[0].id, question);
        assert_eq!(snapshot.items[0].receipt.as_ref().unwrap().state, "acknowledged");

        debate_start_checked(&pool, active.clone(), id, &[DebateOpponent { agent: "codex".into(), model: None }, DebateOpponent { agent: "agy".into(), model: None }]).await.unwrap();
        assert!(active.lock().unwrap().is_empty());
        assert_eq!(db::debate_sides(&pool, id).await.unwrap().len(), 2);
        assert!(switch_task_agent_checked(&pool, active.clone(), id, "codex", "").await.unwrap_err().contains("토론 중"));
        debate_end_checked(&pool, active.clone(), id).await.unwrap();
        switch_task_agent_checked(&pool, active.clone(), id, "codex", "").await.unwrap();
        assert_eq!(ledger::runtime_of(&pool, id).await.unwrap().as_deref(), Some(ledger::RUNTIME));
        assert!(active.lock().unwrap().is_empty());
        pool.close().await;
        let _ = std::fs::remove_dir_all(worktree);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn question_execution_blocks_switch_and_debate_until_fully_finished() {
        use crate::convo::interaction as ledger;
        let (path, worktree) = paths();
        let pool = db::init_pool(&path).await.unwrap();
        let id = task(&pool, &worktree, "codex").await;
        ledger::bind(&pool, id).await.unwrap();
        let execution = ledger::begin(&pool, id, 100).await.unwrap();
        let active = active();
        for phase in ["starting", "running", "cancelling", "finalizing", "cleanup_failed"] {
            ledger::phase(&pool, &execution, phase).await.unwrap();
            assert!(switch_task_agent_checked(&pool, active.clone(), id, "claude", "").await.is_err(), "{phase}");
            assert!(debate_start_checked(&pool, active.clone(), id, &[DebateOpponent { agent: "claude".into(), model: None }]).await.is_err(), "{phase}");
            assert!(active.lock().unwrap().is_empty());
            assert_eq!(db::get_task(&pool, id).await.unwrap().unwrap().convo_session_id.as_deref(), Some("old-session"));
        }
        ledger::finish(&pool, &execution, "failed", None).await.unwrap();
        assert!(switch_task_agent_checked(&pool, active.clone(), id, "agy", "").await.unwrap_err().contains("Codex"));
        assert_eq!(ledger::runtime_of(&pool, id).await.unwrap().as_deref(), Some(ledger::RUNTIME));
        assert_eq!(db::get_task(&pool, id).await.unwrap().unwrap().agent.as_deref(), Some("codex"));
        pool.close().await;
        let _ = std::fs::remove_dir_all(worktree);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn switches_claude_to_codex_then_back_with_model_provenance() {
        let (path, worktree) = paths();
        let pool = db::init_pool(&path).await.unwrap();
        let id = task(&pool, &worktree, "claude").await;
        db::set_task_reasoning_effort(&pool, id, "high")
            .await
            .unwrap();
        event(
            &pool,
            id,
            serde_json::json!({"kind":"model_snapshot","resolved":"claude-model"}),
            3,
        )
        .await;

        let (first, _) = switch_task_agent_checked(&pool, active(), id, "codex", "gpt-5.6-sol")
            .await
            .unwrap();
        assert_eq!(first.agent.as_deref(), Some("codex"));
        assert_eq!(first.model.as_deref(), Some("gpt-5.6-sol"));
        assert_eq!(first.convo_session_id, None);
        assert_eq!(first.reasoning_effort, None);
        assert!(
            !db::set_task_model_for_agent(&pool, id, "claude", "stale", false, 4)
                .await
                .unwrap()
        );
        event(
            &pool,
            id,
            serde_json::json!({"kind":"model_snapshot","resolved":"codex-model"}),
            4,
        )
        .await;

        let (second, _) = switch_task_agent_checked(&pool, active(), id, "claude", "sonnet")
            .await
            .unwrap();
        assert_eq!(second.agent.as_deref(), Some("claude"));
        assert_eq!(second.model.as_deref(), Some("sonnet"));
        let models = db::observed_models(&pool).await.unwrap();
        assert!(models.iter().any(|row| row.agent == "claude" && row.model == "claude-model"));
        assert!(models.iter().any(|row| row.agent == "codex" && row.model == "codex-model"));
        assert_eq!(db::list_convo_events(&pool, id).await.unwrap().iter().filter(|event| event.contains("context_cleared")).count(), 2);
        let _ = std::fs::remove_dir_all(worktree);
    }

    #[tokio::test]
    async fn rejects_invalid_active_terminal_and_missing_worktree_tasks() {
        let (path, worktree) = paths();
        let pool = db::init_pool(&path).await.unwrap();
        let id = task(&pool, &worktree, "claude").await;
        assert!(
            switch_task_agent_checked(&pool, active(), id, "crush", "")
                .await
                .is_err()
        );

        let busy = active();
        busy.lock().unwrap().insert(
            id,
            ActiveConvo {
                pgid: None,
                vendor_bin: String::new(),
                started_at: 0,
                last_event_at: 0,
                last_operation: None,
                interrupted: false,
            },
        );
        assert!(switch_task_agent_checked(&pool, busy, id, "codex", "")
            .await
            .is_err());
        db::update_state(&pool, id, tstate::DONE, 3).await.unwrap();
        assert!(switch_task_agent_checked(&pool, active(), id, "codex", "")
            .await
            .is_err());

        let missing = db::insert_task(
            &pool,
            "/repo",
            "b",
            "main",
            "/missing/agent-switch",
            "i",
            Some("claude"),
            None,
            "conversation",
            1,
        )
        .await
        .unwrap();
        db::update_state(&pool, missing, tstate::AWAITING_REVIEW, 2)
            .await
            .unwrap();
        assert!(
            switch_task_agent_checked(&pool, active(), missing, "codex", "")
                .await
                .is_err()
        );
        let _ = std::fs::remove_dir_all(worktree);
    }

    #[tokio::test]
    async fn transaction_failure_preserves_agent_session_and_handoff() {
        let (path, worktree) = paths();
        let pool = db::init_pool(&path).await.unwrap();
        let id = task(&pool, &worktree, "claude").await;
        crate::convo::interaction::bind_runtime(&pool, id, crate::convo::interaction::RUNTIME_LOCAL).await.unwrap();
        db::set_pending_capsule(&pool, id, "old handoff")
            .await
            .unwrap();
        db::set_task_model(&pool, id, "old-model").await.unwrap();
        db::set_task_reasoning_effort(&pool, id, "high")
            .await
            .unwrap();
        event(
            &pool,
            id,
            serde_json::json!({"kind":"model_snapshot","resolved":"old-observation"}),
            3,
        )
        .await;
        sqlx::query("CREATE TRIGGER fail_agent_switch_boundary BEFORE INSERT ON convo_events BEGIN SELECT RAISE(ABORT, 'boundary failure'); END")
            .execute(&pool)
            .await
            .unwrap();

        assert!(
            db::switch_convo_agent(&pool, id, "codex", "gpt", "new handoff", "{}", 3)
                .await
                .is_err()
        );
        let unchanged = db::get_task(&pool, id).await.unwrap().unwrap();
        assert_eq!(unchanged.agent.as_deref(), Some("claude"));
        assert_eq!(unchanged.model.as_deref(), Some("old-model"));
        assert_eq!(unchanged.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(unchanged.convo_session_id.as_deref(), Some("old-session"));
        assert_eq!(unchanged.pending_capsule.as_deref(), Some("old handoff"));
        assert_eq!(crate::convo::interaction::runtime_of(&pool, id).await.unwrap().as_deref(), Some(crate::convo::interaction::RUNTIME_LOCAL));
        assert!(!db::list_convo_events(&pool, id).await.unwrap()[0].contains("\"agent\""));
        let _ = std::fs::remove_dir_all(worktree);
    }

    #[tokio::test]
    async fn handoff_keeps_recent_corrections_after_tool_heavy_turns() {
        let (path, worktree) = paths();
        let pool = db::init_pool(&path).await.unwrap();
        let id = task(&pool, &worktree, "claude").await;
        for n in 0..14 {
            event(
                &pool,
                id,
                serde_json::json!({"kind":"user","text":format!("correction-{n}")}),
                n + 3,
            )
            .await;
        }
        for n in 0..70 {
            event(
                &pool,
                id,
                serde_json::json!({"kind":"tool_use","summary":format!("tool-{n}")}),
                n + 30,
            )
            .await;
        }
        event(
            &pool,
            id,
            serde_json::json!({"kind":"text","text":"subagent-only","parent_id":"nested"}),
            200,
        )
        .await;
        event(
            &pool,
            id,
            serde_json::json!({"kind":"user","text":"latest correction"}),
            201,
        )
        .await;

        let (switched, _) = switch_task_agent_checked(&pool, active(), id, "codex", "")
            .await
            .unwrap();
        let handoff = switched.pending_capsule.unwrap();
        assert!(handoff.contains("latest correction"));
        assert!(handoff.contains("correction-13"));
        assert!(!handoff.contains("correction-0"));
        assert!(!handoff.contains("tool-69"));
        assert!(!handoff.contains("subagent-only"));
        let _ = std::fs::remove_dir_all(worktree);
    }

    #[test]
    fn dialogue_bound_handles_a_short_multibyte_remainder() {
        let mut events = vec![
            serde_json::json!({"kind":"user","text":"가나다라마바사라마바사라마바사"}).to_string(),
        ];
        events.extend(
            (0..5)
                .map(|n| {
                    serde_json::json!({"kind":"user","text":format!("{n}{}", "x".repeat(1_198))})
                        .to_string()
                })
                .collect::<Vec<_>>(),
        );

        let dialogue = recent_agent_switch_dialogue(events);
        let last = dialogue.lines().next().unwrap().split_once(": ").unwrap().1;
        assert_eq!(last.chars().count(), 5);
        assert!(!last.contains("truncated"));
    }
}

#[cfg(test)]
mod reconcile_tests {
    use super::*;

    #[test]
    fn adopt_when_conversation_pgid_alive() {
        assert_eq!(
            classify_stale("conversation", Some(4242), |_| true),
            StaleAction::Adopt(4242),
        );
    }

    #[test]
    fn fail_when_conversation_pgid_dead() {
        assert_eq!(
            classify_stale("conversation", Some(4242), |_| false),
            StaleAction::Fail
        );
    }

    #[test]
    fn fail_when_terminal_even_if_alive() {
        // 터미널(PTY) 모드는 앱과 함께 죽는 전제 → 생존해 보여도 Fail.
        assert_eq!(
            classify_stale("terminal", Some(4242), |_| true),
            StaleAction::Fail
        );
    }

    #[test]
    fn fail_when_no_pgid() {
        // pgid가 없으면 alive를 아예 호출하지 않고 Fail(스폰 전 크래시 등).
        assert_eq!(
            classify_stale("conversation", None, |_| panic!("alive는 호출되면 안 됨")),
            StaleAction::Fail,
        );
    }
}


/// 워크트리 격리 설정 해석 — 프로젝트 오버라이드 → 전역 기본 → 켜짐 폴백을 in-process SQLite로 검증.
#[cfg(test)]
mod use_worktree_resolution_tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_db() -> String {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        crate::testtmp::dir()
            .join(format!("praxis-use-wt-{}-{}.db", std::process::id(), n))
            .to_string_lossy()
            .into_owned()
    }

    const REPO: &str = "/Users/me/work/alpha";
    const OTHER: &str = "/Users/me/work/beta";

    #[tokio::test]
    async fn defaults_to_on_when_nothing_is_set() {
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        assert!(use_worktree_on(&pool, REPO).await, "미설정이면 격리가 기본");
        assert_eq!(use_worktree_override(&pool, REPO).await, None);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn follows_global_default_without_override() {
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        db::set_setting(&pool, "use_worktree", "false")
            .await
            .unwrap();
        assert!(!use_worktree_on(&pool, REPO).await);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn project_override_beats_global_default() {
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        db::set_setting(&pool, "use_worktree", "false")
            .await
            .unwrap();
        db::set_setting(&pool, &use_worktree_key(REPO), "true")
            .await
            .unwrap();
        assert!(
            use_worktree_on(&pool, REPO).await,
            "전역이 꺼져도 이 프로젝트는 격리"
        );
        assert!(
            !use_worktree_on(&pool, OTHER).await,
            "다른 프로젝트는 전역을 그대로 따름"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn project_override_can_turn_isolation_off() {
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        // 전역은 미설정(=켜짐)인데 이 프로젝트만 직접 실행.
        db::set_setting(&pool, &use_worktree_key(REPO), "false")
            .await
            .unwrap();
        assert!(!use_worktree_on(&pool, REPO).await);
        assert!(use_worktree_on(&pool, OTHER).await);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn clearing_override_restores_the_global_default() {
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        db::set_setting(&pool, "use_worktree", "false")
            .await
            .unwrap();
        db::set_setting(&pool, &use_worktree_key(REPO), "true")
            .await
            .unwrap();
        assert!(use_worktree_on(&pool, REPO).await);

        db::delete_setting(&pool, &use_worktree_key(REPO))
            .await
            .unwrap();
        assert_eq!(use_worktree_override(&pool, REPO).await, None);
        assert!(
            !use_worktree_on(&pool, REPO).await,
            "해제 후에는 전역 기본으로 복귀"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn repo_paths_containing_colons_stay_separate() {
        let path = temp_db();
        let pool = db::init_pool(&path).await.unwrap();
        // 키는 prefix + 경로 원문이므로 경로 안의 `:`가 다른 프로젝트와 섞이면 안 된다.
        let odd = "/Users/me/work/a:b";
        db::set_setting(&pool, &use_worktree_key(odd), "false")
            .await
            .unwrap();
        assert!(!use_worktree_on(&pool, odd).await);
        assert!(use_worktree_on(&pool, "/Users/me/work/a").await);
        let _ = std::fs::remove_file(&path);
    }
}

// ── 지식 그래프 (설계 0020) ──
//
// **Tauri 경계는 여기까지다.** `knowledge` 모듈은 Tauri를 모른다.
// Runner·모바일에는 대응 라우트를 만들지 않는다 — 로컬 경계(DR-6)를
// `crate::knowledge::tests::isolation`이 강제한다.

#[tauri::command]
pub async fn knowledge_search(
    state: State<'_, AppState>,
    query: String,
    limit: Option<i64>,
) -> Result<Vec<crate::knowledge::search::SearchHit>, String> {
    let pool = pool_of(&state)?;
    let spaces = crate::knowledge::wiki::active_space_ids(&pool)
        .await
        .map_err(|e| e.to_string())?;
    crate::knowledge::search::search_visible(&pool, &query, limit.unwrap_or(8), &spaces)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn knowledge_vaults_get(
    state: State<'_, AppState>,
) -> Result<crate::knowledge::config::ObsidianConfig, String> {
    let _lock = wiki_sync_lock().lock().await;
    let pool = pool_of(&state)?;
    crate::knowledge::wiki::legacy_view(&pool)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn knowledge_vaults_set(
    state: State<'_, AppState>,
    config: crate::knowledge::config::ObsidianConfig,
) -> Result<(), String> {
    let _lock = wiki_sync_lock().lock().await;
    let pool = pool_of(&state)?;
    crate::knowledge::wiki::replace_legacy(&pool, &config)
        .await
        .map_err(|e| e.to_string())
}

/// vault를 훑어 색인한다. 변경분만 재임베딩하므로 반복 호출이 싸다.
#[tauri::command]
pub async fn knowledge_sync(state: State<'_, AppState>) -> Result<KnowledgeSyncResult, String> {
    let _lock = wiki_sync_lock().lock().await;
    let pool = pool_of(&state)?;
    if !crate::knowledge::wiki::spaces(&pool)
        .await
        .map_err(|e| e.to_string())?
        .is_empty()
    {
        let report = crate::knowledge::wiki::sync(&pool, now())
            .await
            .map_err(|e| e.to_string())?;
        if !report.complete {
            return Err(format!(
                "Wiki synchronization is incomplete: {}",
                report.warnings.join("; ")
            ));
        }
        let embedded = embed_all(&pool).await?;
        return Ok(KnowledgeSyncResult {
            indexed: report.indexed,
            skipped: report.skipped,
            deleted: report.deleted,
            edges: report.edges,
            embedded,
        });
    }
    let cfg = crate::knowledge::config::load_obsidian(&pool)
        .await
        .map_err(|e| e.to_string())?;
    if cfg.vaults.is_empty() {
        return Err("연결된 vault가 없습니다. 설정에서 폴더를 먼저 지정하세요.".into());
    }
    let source = crate::knowledge::config::MultiVault {
        entries: cfg.vaults,
    };
    let report = crate::knowledge::sync::sync_source(&pool, &source, now())
        .await
        .map_err(|e| e.to_string())?;

    // 임베딩은 색인과 분리돼 있다(graph.rs 주석). 여기서 배치로 채우되, 실패해도
    // 색인 자체는 이미 커밋됐으므로 다음 호출에서 남은 것만 이어서 채운다.
    let embedded = embed_all(&pool).await?;

    Ok(KnowledgeSyncResult {
        indexed: report.indexed,
        skipped: report.skipped,
        deleted: report.deleted,
        edges: report.edges,
        embedded,
    })
}

async fn embed_all(pool: &SqlitePool) -> Result<usize, String> {
    let mut embedded = 0;
    loop {
        let count = crate::knowledge::graph::embed_pending(pool, 256)
            .await
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Ok(embedded);
        }
        embedded += count;
    }
}

#[derive(serde::Serialize)]
pub struct KnowledgeSyncResult {
    pub indexed: usize,
    pub skipped: usize,
    pub deleted: usize,
    pub edges: usize,
    pub embedded: usize,
}

#[tauri::command]
pub async fn wiki_spaces(
    state: State<'_, AppState>,
) -> Result<Vec<crate::knowledge::wiki::WikiSpace>, String> {
    let _lock = wiki_sync_lock().lock().await;
    crate::knowledge::wiki::spaces(&pool_of(&state)?)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn wiki_connect(
    state: State<'_, AppState>,
    root: String,
) -> Result<crate::knowledge::wiki::WikiSpace, String> {
    let _lock = wiki_sync_lock().lock().await;
    crate::knowledge::wiki::connect(&pool_of(&state)?, &root)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn wiki_sync(
    state: State<'_, AppState>,
) -> Result<crate::knowledge::wiki::WikiSyncResult, String> {
    let _lock = wiki_sync_lock().lock().await;
    crate::knowledge::wiki::sync(&pool_of(&state)?, now())
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn wiki_documents(
    state: State<'_, AppState>,
    space_id: Option<String>,
    query: String,
) -> Result<crate::knowledge::wiki::WikiDocumentsResult, String> {
    let _lock = wiki_sync_lock().lock().await;
    crate::knowledge::wiki::documents(&pool_of(&state)?, space_id.as_deref(), &query)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn wiki_read_document(
    state: State<'_, AppState>,
    node_id: i64,
) -> Result<crate::knowledge::wiki::WikiReadDocument, String> {
    let _lock = wiki_sync_lock().lock().await;
    crate::knowledge::wiki::read_document(&pool_of(&state)?, node_id)
        .await
        .map_err(|error| error.to_string())
}


// ── 금일 할 일 (설계 0021 · 플랜 0026) ─────────────────────────────────────
// 로컬 전용이다 — Runner/모바일 transport를 태우지 않는다 (설계 §3 Scope).

/// 날짜 미지정이면 오늘(로컬 달력일). tz는 스케줄과 같은 규약(KST 기본) —
/// 설정으로 뺄 필요가 생기면 그때 `db::get_setting`을 태운다.
fn resolve_day(day: Option<String>) -> Result<String, String> {
    match day {
        Some(d) if !d.trim().is_empty() => {
            // 백로그도 유효한 레인 키다 — `today_list`·`today_add`·`today_reorder`가
            // 그대로 백로그에 동작한다 (플랜 0054 DR-3). 날짜만 받아야 하는 자리
            // (`today_close`)는 본문에서 `validate`로 한 번 더 좁힌다.
            crate::today::day::validate_key(&d)?;
            Ok(d)
        }
        _ => crate::today::day::local_day(now(), crate::today::day::KST_OFFSET_SECS),
    }
}

/// 설정 화면이 필요로 하는 상태 한 묶음.
#[derive(serde::Serialize)]
pub struct GmailStatus {
    /// refresh token이 키체인에 있는가.
    pub connected: bool,
    pub client_id: String,
    /// secret은 값을 절대 돌려주지 않는다 — **있는지 여부만** 알려준다.
    pub client_secret_set: bool,
    pub query: String,
    /// "미연결" | "백필 중" | "최신" — 커서에서 읽는다(상태를 두 곳에 두지 않는다).
    pub stage: String,
    pub indexed: i64,
    pub last_error: Option<String>,
    /// 빌드에 client가 박혀 있는가 (ADR 0147). 화면은 이걸로 직접 입력 경로를
    /// 접을지 정한다.
    pub bundled_available: bool,
    /// 지금 연결에 실제로 쓰일 credential이 번들인가. 사용자가 자기 값을 넣으면 false다.
    pub using_bundled: bool,
}

#[derive(serde::Serialize)]
pub struct KnowledgeGmailSyncResult {
    pub indexed: usize,
    pub skipped: usize,
    pub deleted: usize,
    pub embedded: usize,
    pub has_more: bool,
}

/// 저장된 refresh token으로 access token을 갱신한다.
///
/// 액세스 토큰을 캐시하지 않는 이유: 유효기간이 1시간이라 동기화 한 번마다 한 번
/// 갱신하면 충분하고, 캐시를 두면 만료 시각 관리가 또 하나의 상태가 된다.
async fn gmail_access_token(
    state: &State<'_, AppState>,
    pool: &sqlx::SqlitePool,
) -> Result<String, String> {
    let cfg = crate::knowledge::config::load_gmail(pool)
        .await
        .map_err(|e| e.to_string())?;
    let user_secret =
        crate::secret::get_secret(crate::knowledge::source::gmail::CLIENT_SECRET_KEY).await?;
    let credentials =
        crate::knowledge::source::gmail_auth::resolve(&cfg.client_id, user_secret.as_deref())
            .map_err(|e| e.to_string())?;
    let refresh = crate::secret::get_secret(crate::knowledge::source::gmail::REFRESH_TOKEN_KEY)
        .await?
        .ok_or("Gmail이 연결되지 않았습니다.")?;

    crate::knowledge::source::gmail_auth::refresh_access_token(
        &state.http,
        &credentials.client_id,
        &credentials.client_secret,
        &refresh,
    )
    .await
    .map(|t| t.access_token)
    .map_err(|e| e.to_string())
}

/// 커스텀 테마 저장 디렉터리 — `<app_config_dir>/themes` (설계 0049).
fn theme_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_config_dir().map_err(|e| e.to_string())?.join("themes"))
}

/// 저장된 커스텀 테마 spec 원문 목록 (정렬됨).
#[tauri::command]
pub async fn theme_list(app: AppHandle) -> Result<Vec<String>, String> {
    Ok(theme_store::list(&theme_dir(&app)?))
}

/// 커스텀 테마 spec을 원자적으로 저장한다.
#[tauri::command]
pub async fn theme_save(app: AppHandle, id: String, json: String) -> Result<(), String> {
    theme_store::save(&theme_dir(&app)?, &id, &json)
}

/// 커스텀 테마 파일을 삭제한다.
#[tauri::command]
pub async fn theme_delete(app: AppHandle, id: String) -> Result<(), String> {
    theme_store::delete(&theme_dir(&app)?, &id)
}

/// 저장된 테마 파일을 사용자가 고른 목적지로 복사한다 (`dest`는 dialog가 고른 값만 들어온다).
#[tauri::command]
pub async fn theme_export(app: AppHandle, id: String, dest: String) -> Result<(), String> {
    theme_store::export(&theme_dir(&app)?, &id, Path::new(&dest))
}

/// 외부 경로(dialog가 고른 값)에서 테마 spec 원문을 읽는다. 저장은 별도 `theme_save` 호출.
#[tauri::command]
pub async fn theme_import(src: String) -> Result<String, String> {
    theme_store::read_external(Path::new(&src))
}


#[cfg(test)]
mod autoupdate_guard_tests {
    use super::*;

    #[test]
    fn work_in_the_middle_of_creation_still_counts_as_work() {
        // 이 단언이 이 기능의 핵심 회귀 방지다. `create_task_internal`은 슬롯을 예약한 뒤
        // worktree 생성과 임베딩을 거쳐서야 `tasks`에 넣는다 — 그 수 초 동안 0이 나오면
        // 자동 업데이트가 "아무도 없다"고 보고 방금 만들어지는 작업의 발밑을 갈아치운다.
        let state = AppState::default();
        assert_eq!(active_work_count(&state), 0);
        *state.reserved.lock().unwrap() = 1;
        assert_eq!(
            active_work_count(&state),
            1,
            "예약된 슬롯도 실행 중인 작업으로 세어야 한다"
        );
    }

    #[test]
    fn the_guard_only_refuses_while_an_update_is_running() {
        let state = AppState::default();
        assert!(refuse_while_updating(&state).is_ok());

        state.updating.store(true, Ordering::SeqCst);
        let error = refuse_while_updating(&state).expect_err("업데이트 중이면 거부해야 한다");
        assert!(error.contains("자동 업데이트"), "{error}");

        // 일회성이 아니다 — 업데이트가 끝나면 다시 통과해야 한다.
        state.updating.store(false, Ordering::SeqCst);
        assert!(refuse_while_updating(&state).is_ok());
    }

    #[test]
    fn both_guard_entry_points_speak_with_one_voice() {
        // 대화 턴 경로는 `AppState`를 받지 못해 판정 함수가 따로다. 문구가 갈리면
        // 같은 사건이 사용자에게 다르게 보인다.
        let state = AppState::default();
        state.updating.store(true, Ordering::SeqCst);
        assert_eq!(
            refuse_while_updating(&state).unwrap_err(),
            refuse_if_updating(&state.updating).unwrap_err()
        );
    }
}

#[cfg(test)]
mod question_ownership_tests {
    use super::*;
    #[test]
    fn failed_setup_after_controller_registration_keeps_the_reservation() {
        let id=9_999_732;let active=Arc::new(Mutex::new(HashMap::new()));
        let reservation=reserve_convo_switch(active.clone(),id).unwrap();
        crate::convo::app_server::register(id,"uncommitted-setup".into()).unwrap();
        drop(reservation);
        assert!(ensure_convo_idle(&active,id).is_err());assert!(crate::convo::app_server::cleanup_failed(id));
        crate::convo::app_server::unregister(id);assert!(release_convo_if_finalized(&active,id));
    }
    #[test]
    fn cleanup_failure_keeps_write_approve_discard_and_new_turn_locked() {
        let id=9_999_731;let active=Arc::new(Mutex::new(HashMap::new()));
        let mut reservation=reserve_convo_switch(active.clone(),id).unwrap();reservation.handoff_to_turn();drop(reservation);
        crate::convo::app_server::register(id,"execution-fixture".into()).unwrap();
        assert!(!release_convo_if_finalized(&active,id));
        assert!(crate::convo::app_server::cleanup_failed(id));
        assert!(ensure_convo_idle(&active,id).is_err());assert!(reserve_convo_switch(active.clone(),id).is_err());
        crate::convo::app_server::unregister(id);
        assert!(release_convo_if_finalized(&active,id));assert!(ensure_convo_idle(&active,id).is_ok());
    }
}
