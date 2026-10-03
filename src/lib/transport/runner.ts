import type { ApprovalStatus, ApprovalRepairSession } from "../approval-readiness";
import type { MessageReceipt, SideQuestionInput, SideQuestionSnapshot } from "../side-question";
import type {
  AnnotationSaveInput,
  BranchList,
  BrowseResult,
  ComposeOutcome,
  CrystallizeResult,
  DiffHunk,
  EnsembleMatrix,
  FileContent,
  FileDiff,
  FsNode,
  GhRepo,
  GithubIssuesResult,
  GrillNote,
  GrillRound,
  HunkRef,
  InterviewAssessment,
  MobilePairing,
  MobileSession,
  PartialApplyResult,
  QuickOpenTaskCandidate,
  RematchedAnnotation,
  ReviewAnnotation,
  Schedule,
  SessionHomeEntry,
  SessionHomePreview,
  SkillMeta,
  Task,
  TaskRow,
  VerifyPreview,
  VerifyReport,
} from "../ipc";
import type { SourcePage } from "../notifications";
import type {
  DiffRange,
  HostId,
  PraxisTransport,
  ReviewProcessQuarantine,
  ReviewProcessRepairResult,
  ScheduleCreateRequest,
  SessionHomeSession,
  TaskCreateRequest,
  TaskDiffResult,
} from "../transport";
import { SessionResumeError } from "../transport";
import { formatDiffStat } from "../diff";

/** 범위 쿼리. 생략하면 아무것도 붙이지 않는다 — 서버 기본값(세션 전체)을 쓴다. */
function rangeQuery(range?: DiffRange, includeReview?: boolean): string {
  const params = new URLSearchParams();
  if (range) params.set("range", range);
  if (includeReview) params.set("include_review", "true");
  const query = params.toString();
  return query ? `?${query}` : "";
}

/**
 * diff 응답을 **화면이 믿는 모양으로 고정한다.**
 *
 * 기준점(baseline)이 생기기 전의 Runner는 이 엔드포인트가 `FileDiff[]`를 그대로 돌려준다.
 * 그 응답은 200이고 JSON도 멀쩡하므로 `json()`의 그물에 걸리지 않고, `files`가 `undefined`인
 * 채로 화면까지 간다 — 변경 목록이 `files === null`(로딩)과 구분하지 못하고 그 자리에서 터졌다.
 * 데스크톱과 원격 Runner는 각자 갱신되므로 버전이 어긋나는 것은 예외가 아니라 기본값이다.
 * 경계에서 한 번 정규화하면 그 뒤로는 아무도 wire 모양을 몰라도 된다.
 */
export function normalizeTaskDiff(payload: unknown): TaskDiffResult {
  const legacy: TaskDiffResult["baseline"] = { kind: "legacy" };
  if (Array.isArray(payload)) return { files: payload as FileDiff[], baseline: legacy };
  const value = (payload ?? {}) as Partial<TaskDiffResult>;
  return {
    files: Array.isArray(value.files) ? value.files : [],
    baseline: value.baseline ?? legacy,
    review:
      Array.isArray(value.review?.hunks) && Array.isArray(value.review?.annotations)
        ? value.review
        : undefined,
  };
}
import {
  backoffDelay,
  isWatermark,
  loadSequence,
  saveSequence,
  shouldResetCursor,
} from "./reconnect";

export interface RunnerConnection {
  endpoint: string;
  /**
   * Desktop의 pairing token. 모바일 PWA는 이 값을 갖지 않고 HttpOnly 세션 쿠키로 인증하므로
   * 비워 둔다 — 원문 토큰을 브라우저에 두지 않는 것이 설계 0013 §6.1의 요지다.
   */
  pairingToken?: string;
  profileName?: string;
}

export interface RunnerEvent {
  sequence: number;
  task_id: number;
  ts: number;
  kind: string;
  detail: string | null;
}

export interface RunnerTaskOutput {
  sequence: number;
  task_id: number;
  ts: number;
  data: string;
}

export interface RunnerHealth {
  status: "ok";
  bind: string;
  max_concurrent_tasks: number;
  recovered_tasks: number;
  retention_days: number;
  execution_policy: "always_approve" | "require_approval";
  /**
   * 아래는 모바일 상태 배너용 확장 필드(설계 0013 §7.2).
   * 구버전 Runner에 붙었을 때를 위해 optional로 둔다 — 없으면 배너가 해당 항목을 생략한다.
   */
  version?: string;
  started_at?: number;
  uptime_secs?: number;
  /** 마지막 Runner event 시각. null이면 기동 후 아무 일도 없었다는 뜻. */
  last_event_at?: number | null;
  queued_tasks?: number;
  running_tasks?: number;
}

export interface RunnerWebSocket {
  close(): void;
  onmessage: ((event: MessageEvent<string>) => void) | null;
  onerror: ((event: Event) => void) | null;
  onclose?: ((event: CloseEvent) => void) | null;
}

export type RunnerWebSocketFactory = (url: string, protocols: string[]) => RunnerWebSocket;

type FetchLike = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

/**
 * SSH 터널이 반쯤 죽으면(TCP는 붙었는데 응답이 없음) 여기 걸린 요청이 무한 대기에 빠진다 — 그걸
 * 막는 상한. taskList/health/taskOutput처럼 반복 폴링되는 호출에만 적용한다: 한 번 타임아웃 나도
 * 다음 폴링에서 다시 시도되니 값이 짧아도 안전하다.
 */
const POLL_TIMEOUT_MS = 10_000;

export class RunnerRequestError extends Error {
  constructor(message: string, readonly status: number) {
    super(message);
    this.name = "RunnerRequestError";
  }
}

/** SSH local-forward를 통해 Runner `/v1` 계약을 호출하는 typed transport. */
export class RunnerTransport implements PraxisTransport {
  readonly kind = "remote" as const;

  /**
   * 레지스트리 키. 프로필 이름이 정본이지만, 이름 없이 만들어진 transport(모바일 PWA·테스트)도
   * **로컬 슬롯을 차지해서는 안 되므로** endpoint에서 파생한다 — 원격이 `local`을 덮으면
   * 로컬 작업이 통째로 사라진다.
   */
  readonly hostId: HostId;
  private sessionHomeCapabilities: { vendors: ("claude" | "codex")[]; preview: boolean } | null = null;

  constructor(
    private readonly connection: RunnerConnection,
    // naked `fetch` 참조는 this 바인딩이 풀려 webview에서 Illegal invocation — 래퍼로 전역 바인딩 보존.
    private readonly request: FetchLike = (input, init) => fetch(input, init),
    private readonly socketFactory: RunnerWebSocketFactory = (url, protocols) => new WebSocket(url, protocols),
  ) {
    this.hostId =
      connection.profileName?.trim() || `runner:${connection.endpoint.replace(/\/$/, "")}`;
  }

  async taskList(): Promise<Task[]> {
    const rows = await this.json<TaskRow[]>("/v1/tasks", { signal: AbortSignal.timeout(POLL_TIMEOUT_MS) });
    return rows.map((row) => this.own(row));
  }

  /**
   * 서버 행을 화면 타입으로 승격. **transport의 hostId가 이긴다** — 응답에 host처럼 보이는
   * 값이 섞여 와도 서버는 이 필드의 권위가 아니다 (ADR 0133 결정 2).
   */
  private own(row: TaskRow): Task {
    return { ...row, host: this.hostId };
  }

  async health(): Promise<RunnerHealth> {
    const health = await this.json<RunnerHealth>("/v1/health", { signal: AbortSignal.timeout(POLL_TIMEOUT_MS) });
    if (health?.status !== "ok") throw new Error("Runner 상태 응답을 확인할 수 없습니다");
    return health;
  }

  profileName(): string | null {
    return this.connection.profileName ?? null;
  }

  reviewProcessQuarantines(): Promise<ReviewProcessQuarantine[]> {
    return this.json("/v1/review-processes/quarantined");
  }

  reviewProcessReconcile(receiptId: number): Promise<ReviewProcessRepairResult> {
    return this.json(`/v1/review-processes/${receiptId}/reconcile`, {
      method: "POST",
    });
  }

  async taskDiffStat(id: number): Promise<string> {
    const { files } = await this.taskDiff(id);
    return formatDiffStat(files);
  }

  async taskDiff(id: number, range?: DiffRange, includeReview?: boolean): Promise<TaskDiffResult> {
    return normalizeTaskDiff(await this.json(`/v1/tasks/${id}/diff${rangeQuery(range, includeReview)}`));
  }

  async fsTree(id: number): Promise<FsNode[]> {
    const task = await this.task(id);
    return this.json(`/v1/files/tree?${repositoryQuery(task.worktree_path)}`);
  }

  fsTreePath(repository: string): Promise<FsNode[]> {
    return this.json(`/v1/files/tree?${repositoryQuery(repository)}`);
  }

  fsBrowse(path: string): Promise<BrowseResult> {
    return this.json(`/v1/files/browse?${new URLSearchParams({ path })}`);
  }

  fsRoots(): Promise<string[]> {
    return this.json("/v1/files/roots");
  }

  // 파일 조작은 로컬 전용(설계 0024 D4) — Runner에는 browse/roots/tree/read/write뿐이라
  // 삭제·이름변경·생성에 대응할 엔드포인트가 없다. UI도 원격에서는 이 항목들을 감춘다.
  fsCreateFile(): Promise<string> {
    return Promise.reject(new Error("원격 Runner에서는 파일 조작을 지원하지 않습니다 (로컬 전용)"));
  }

  fsCreateDir(): Promise<string> {
    return Promise.reject(new Error("원격 Runner에서는 파일 조작을 지원하지 않습니다 (로컬 전용)"));
  }

  fsRename(): Promise<string> {
    return Promise.reject(new Error("원격 Runner에서는 파일 조작을 지원하지 않습니다 (로컬 전용)"));
  }

  fsTrash(): Promise<void> {
    return Promise.reject(new Error("원격 Runner에서는 파일 조작을 지원하지 않습니다 (로컬 전용)"));
  }

  fsCopy(): Promise<string> {
    return Promise.reject(new Error("원격 Runner에서는 파일 조작을 지원하지 않습니다 (로컬 전용)"));
  }

  fsOpenTerminal(): Promise<void> {
    return Promise.reject(
      new Error("원격 Runner에서는 터미널 열기를 지원하지 않습니다 (로컬 전용)"),
    );
  }

  gitStatus(path: string): Promise<boolean> {
    return this.json<{ is_repo: boolean }>(`/v1/git/status?${new URLSearchParams({ path })}`).then(
      (result) => result.is_repo,
    );
  }

  /** QR용 일회용 페어링 코드 발급 (설계 0013 §6.1). */
  mobilePairingCreate(): Promise<MobilePairing> {
    return this.json("/v1/mobile/pairings", { method: "POST" });
  }

  mobileSessions(): Promise<MobileSession[]> {
    return this.json("/v1/mobile/sessions");
  }

  async mobileSessionRevoke(id: number): Promise<void> {
    await this.empty(`/v1/mobile/sessions/${id}`, { method: "DELETE" });
  }

  gitInit(path: string): Promise<boolean> {
    return this.json<{ is_repo: boolean }>("/v1/git/init", {
      method: "POST",
      body: JSON.stringify({ path }),
    }).then((result) => result.is_repo);
  }

  /**
   * 원격 Runner는 base 브랜치를 고를 수 없다 — 러너 머신 레포의 현재 체크아웃에서 분기한다.
   * 빈 목록을 돌려주면 호출부가 선택 UI를 숨긴다(거절하면 오류만 띄우게 된다).
   */
  gitBranches(): Promise<BranchList> {
    return Promise.resolve({ current: "", branches: [] });
  }

  gitCheckoutBranch(): Promise<BranchList> {
    return Promise.reject(new Error("원격 Runner에서는 브랜치를 전환할 수 없습니다 (로컬 전용)"));
  }

  repositoryList(): Promise<string[]> {
    return this.json("/v1/repositories");
  }

  async fsRead(id: number, path: string): Promise<FileContent> {
    // 외부 문서 탭의 절대경로도 기존 read API로 읽는다. 부모 디렉터리는 Runner가
    // repository_roots로 검증하고 파일명은 safe_join으로 검증하므로 읽기 권한은 그대로다.
    // 로컬 IPC나 OS opener로 폴백하지 않는다.
    if (path.startsWith("/") || /^[a-z]:[\\/]/i.test(path)) {
      const normalized = path.replace(/\\/g, "/");
      const parts = normalized.split("/");
      const name = parts.pop();
      if (!name || name === "." || name === ".." || normalized.startsWith("//")
        || /[\0-\x1f\x7f]/.test(normalized) || parts.some((part) => part === "." || part === "..")) {
        throw new Error("유효하지 않은 원격 파일 경로입니다");
      }
      const parent = parts.join("/");
      const repository = /^[a-z]:$/i.test(parent) ? `${parent}/` : parent || "/";
      const query = new URLSearchParams({ repository, path: name });
      return this.json(`/v1/files/read?${query}`, { signal: AbortSignal.timeout(POLL_TIMEOUT_MS) });
    }
    const task = await this.task(id);
    const query = new URLSearchParams({ repository: task.worktree_path, path });
    return this.json(`/v1/files/read?${query}`);
  }

  async fsWrite(id: number, path: string, content: string): Promise<number> {
    const task = await this.task(id);
    return this.json("/v1/files/write", {
      method: "PUT",
      body: JSON.stringify({ repository: task.worktree_path, path, content }),
    });
  }

  scheduleList(): Promise<Schedule[]> {
    return this.json("/v1/schedules");
  }

  taskOutput(id: number, after = 0): Promise<RunnerTaskOutput[]> {
    // 리플레이 드레인 루프가 for(;;) 안에서 이걸 await한다 — 타임아웃이 없으면 SSH 터널이 반쯤
    // 죽었을 때 화면이 영원히 멈춘다(원장 #448).
    return this.json(`/v1/tasks/${id}/output?after=${after}`, {
      signal: AbortSignal.timeout(POLL_TIMEOUT_MS),
    });
  }

  notificationSourcePage(after: number | null): Promise<SourcePage> {
    const query = after == null ? "" : `?after=${after}`;
    return this.json(`/v1/notifications/results${query}`);
  }

  async taskCreate(request: TaskCreateRequest): Promise<Task> {
    if (request.service_tier != null) throw new Error("실행 속도 선택은 로컬 Codex 대화에서 지원합니다");
    if(request.mode === "conversation_questions") throw new Error("질문 응답은 로컬 Codex·Claude 대화에서 지원합니다");
    const body: Record<string, unknown> = {
      repository: request.repo,
      instruction: request.instruction,
      agent: request.agent,
      role: request.role,
      model: request.model,
      mode: request.mode,
    };
    if (request.reasoning_effort?.trim()) {
      body.reasoning_effort = request.reasoning_effort.trim();
    }
    if (request.goal_contract != null) body.goal_contract = request.goal_contract;
    if (request.resumeSession) {
      if (request.resumeVendor === "codex") {
        const capabilities = await this.loadSessionHomeCapabilities(request.repo);
        if (!capabilities.vendors.includes("codex")) {
          throw new Error("선택한 Runner는 Codex 세션 이어받기를 지원하지 않습니다");
        }
      }
      body.resume_session = request.resumeSession;
      if (request.resumeVendor) body.resume_vendor = request.resumeVendor;
    }
    // 단건 반환도 태깅한다 — 여기를 빼면 생성 직후 선택이 host를 잃는다(가장 자주 지나는 경로).
    try {
      return this.own(
        await this.json<TaskRow>("/v1/tasks", {
          method: "POST",
          body: JSON.stringify(body),
        }),
      );
    } catch (error) {
      // 세션 승계를 요청했을 때만 404/409를 승계 전용 오류로 다시 감싼다 — 다른 요청 실패
      // (400 검증 오류 등)는 기존 requestError() 메시지를 그대로 쓴다.
      if (request.resumeSession && error instanceof RunnerRequestError) {
        if (error.status === 404) {
          throw new SessionResumeError(error.message, "not_found");
        }
        if (error.status === 409) {
          const taskId = parseConflictTaskId(error.message);
          throw new SessionResumeError(error.message, "conflict", taskId ?? undefined);
        }
      }
      throw error;
    }
  }

  // 이어받기는 로컬 DB의 resume 체인(`resumed_from`)에 의존한다 — Runner에는 대응 엔드포인트가
  // 없다. UI도 원격 종결 대화에서는 이 동선을 감춘다(host-capabilities.ts convoResume).
  taskResume(_id: number, _message: string, _clientRef?: string): Promise<Task> {
    return Promise.reject(new Error("원격 Runner에서는 대화 이어받기를 지원하지 않습니다 (로컬 전용)"));
  }

  async taskInput(id: number, data: string): Promise<void> {
    await this.empty(`/v1/tasks/${id}/input`, {
      method: "POST",
      body: JSON.stringify({ data }),
    });
  }

  async taskMessage(id: number, message: string): Promise<void> {
    await this.empty(`/v1/tasks/${id}/message`, {
      method: "POST",
      body: JSON.stringify({ message }),
    });
  }

  async taskModelSet(id: number, model: string): Promise<void> {
    await this.empty(`/v1/tasks/${id}/model`, {
      method: "PUT",
      body: JSON.stringify({ model }),
    });
  }

  async taskCancel(id: number): Promise<void> {
    await this.empty(`/v1/tasks/${id}/cancel`, { method: "POST" });
  }

  conversationSubmit(id: number, requestId: string, message: string, imagePaths: string[]): Promise<MessageReceipt> {
    return this.json(`/v1/tasks/${id}/message-receipts/${encodeURIComponent(requestId)}`, {
      method: "POST", body: JSON.stringify({ message, image_paths: imagePaths }),
    });
  }

  conversationReceipt(id: number, requestId: string): Promise<MessageReceipt> {
    return this.json(`/v1/tasks/${id}/message-receipts/${encodeURIComponent(requestId)}`);
  }

  sideQuestionRead(id: number): Promise<SideQuestionSnapshot> {
    return this.json(`/v1/tasks/${id}/side-question`);
  }

  sideQuestionSend(id: number, input: SideQuestionInput): Promise<SideQuestionSnapshot> {
    return this.json(`/v1/tasks/${id}/side-question/messages`, { method: "POST", body: JSON.stringify(input) });
  }

  sideQuestionCancel(id: number, turnId: number): Promise<SideQuestionSnapshot> {
    return this.json(`/v1/tasks/${id}/side-question/cancel`, { method: "POST", body: JSON.stringify({ turn_id: turnId }) });
  }

  sideQuestionReset(id: number, generation: number): Promise<SideQuestionSnapshot> {
    return this.json(`/v1/tasks/${id}/side-question/reset`, { method: "POST", body: JSON.stringify({ generation }) });
  }

  async taskDelete(id: number): Promise<void> {
    await this.empty(`/v1/tasks/${id}`, { method: "DELETE" });
  }

  async taskRunApprove(id: number): Promise<void> {
    await this.empty(`/v1/tasks/${id}/run`, { method: "POST" });
  }

  taskRunReject(id: number): Promise<void> {
    return this.taskCancel(id);
  }

  githubIssuesList(repo: string): Promise<GithubIssuesResult> {
    return this.json(`/v1/github/issues?${repositoryQuery(repo)}`);
  }

  githubReposList(repos: string[]): Promise<GhRepo[]> {
    return this.json("/v1/github/repos", {
      method: "POST",
      body: JSON.stringify({ repositories: repos }),
    });
  }

  async githubCreateTaskFromIssue(repo: string, issueNumber: number, agent: string): Promise<Task> {
    return this.own(
      await this.json<TaskRow>("/v1/github/issues/task", {
        method: "POST",
        body: JSON.stringify({ repository: repo, number: issueNumber, agent }),
      }),
    );
  }

  async githubIssueDelete(repo: string, issueNumber: number): Promise<void> {
    const query = repositoryQuery(repo);
    query.set("number", String(issueNumber));
    await this.empty(`/v1/github/issues?${query}`, { method: "DELETE" });
  }

  taskApprovalStatus(id: number): Promise<ApprovalStatus> {
    return this.json(`/v1/tasks/${id}/approval-status`);
  }

  approvalRepairStatus(id: number): Promise<ApprovalRepairSession | null> {
    return this.json(`/v1/tasks/${id}/approval-repair`);
  }
  approvalRepairPrepare(id: number): Promise<ApprovalRepairSession> {
    return this.json(`/v1/tasks/${id}/approval-repair`, { method: "POST" });
  }
  approvalRepairRun(id: number, sessionId: string): Promise<ApprovalRepairSession> {
    return this.json(`/v1/tasks/${id}/approval-repair/run`, { method: "POST", body: JSON.stringify({ session_id: sessionId }), headers: { "Content-Type": "application/json" } });
  }
  approvalRepairAccept(id: number, sessionId: string): Promise<ApprovalRepairSession> {
    return this.json(`/v1/tasks/${id}/approval-repair/accept`, { method: "POST", body: JSON.stringify({ session_id: sessionId }), headers: { "Content-Type": "application/json" } });
  }
  async approvalRepairCancel(id: number, sessionId: string): Promise<void> {
    await this.empty(`/v1/tasks/${id}/approval-repair/cancel`, { method: "POST", body: JSON.stringify({ session_id: sessionId }), headers: { "Content-Type": "application/json" } });
  }

  async taskApprove(id: number): Promise<void> {
    await this.empty(`/v1/tasks/${id}/approve`, { method: "POST" });
  }

  async taskDiscard(id: number): Promise<void> {
    await this.empty(`/v1/tasks/${id}/discard`, { method: "POST" });
  }

  verifySpec(id: number): Promise<VerifyPreview> {
    return this.json(`/v1/tasks/${id}/verify/spec`);
  }

  taskVerify(id: number, previewToken: string): Promise<VerifyReport> {
    return this.json(`/v1/tasks/${id}/verify`, {
      method: "POST",
      body: JSON.stringify({ preview_token: previewToken }),
    });
  }

  scheduleAdd(request: ScheduleCreateRequest): Promise<number> {
    return this.json("/v1/schedules", {
      method: "POST",
      body: JSON.stringify({
        label: request.label,
        cron: request.cron,
        kind: request.kind,
        payload: request.payload,
        tz_offset_secs: request.tzOffsetSecs,
      }),
    });
  }

  async scheduleRemove(id: number): Promise<void> {
    await this.empty(`/v1/schedules/${id}`, { method: "DELETE" });
  }

  async scheduleSetEnabled(id: number, enabled: boolean): Promise<void> {
    await this.empty(`/v1/schedules/${id}`, {
      method: "PUT",
      body: JSON.stringify({ enabled }),
    });
  }

  /** 구버전 Runner에는 이 라우트가 없다 — 404는 "스킬이 없다"로 읽는다. 403(roots 밖)은 전파한다. */
  async skillsList(repo: string): Promise<SkillMeta[]> {
    const response = await this.request(`${this.endpoint()}/v1/skills?${repositoryQuery(repo)}`, {
      headers: this.headers(),
    });
    if (response.status === 404) return [];
    if (!response.ok) throw await requestError(response);
    return response.json() as Promise<SkillMeta[]>;
  }

  quickopenSearch(query: string, scopes: string[]): Promise<QuickOpenTaskCandidate[]> {
    const params = new URLSearchParams({ query, scopes: scopes.join(",") });
    return this.json(`/v1/quickopen?${params}`);
  }

  async sessionHomeIndex(repo: string, all: boolean, query?: string): Promise<SessionHomeSession[]> {
    const params = new URLSearchParams();
    if (all) params.set("all", "true");
    else params.set("repository", repo);
    if (query?.trim()) params.set("query", query.trim());
    const response = await this.json<{ sessions: SessionHomeEntry[]; capabilities?: { vendors?: string[]; preview?: boolean } }>(`/v1/sessions?${params}`);
    const capabilities = this.rememberSessionHomeCapabilities(response.capabilities);
    // 서버 응답은 host를 모른다 — 여기서 태깅해야 다른 호스트로의 제출을 막는 첫 겹이 선다
    // (설계 2026-09-17 결정 11, taskList의 own()과 같은 자리).
    return response.sessions.filter((row) => row.vendor == null || capabilities.vendors.includes(row.vendor)).map((row) => ({
      ...row,
      host: this.hostId,
      vendor: row.vendor === "codex" ? "codex" : "claude",
      preview_supported: capabilities.preview,
    }));
  }

  async sessionHomePreview(repo: string, vendor: "claude" | "codex", sessionId: string): Promise<SessionHomePreview> {
    const capabilities = await this.loadSessionHomeCapabilities(repo);
    if (!capabilities.preview || !capabilities.vendors.includes(vendor)) {
      throw new Error("선택한 Runner는 이 세션 미리보기를 지원하지 않습니다");
    }
    const params = new URLSearchParams({ repository: repo, vendor });
    const preview = await this.json<SessionHomePreview>(`/v1/sessions/${encodeURIComponent(sessionId)}/preview?${params}`);
    return { ...preview, meta: { ...preview.meta, vendor: preview.meta.vendor ?? vendor } };
  }

  private rememberSessionHomeCapabilities(raw?: { vendors?: string[]; preview?: boolean }): { vendors: ("claude" | "codex")[]; preview: boolean } {
    const vendors = raw?.vendors?.filter((vendor): vendor is "claude" | "codex" => vendor === "claude" || vendor === "codex") ?? ["claude"];
    return (this.sessionHomeCapabilities = { vendors, preview: raw?.preview === true });
  }

  private async loadSessionHomeCapabilities(repo: string): Promise<{ vendors: ("claude" | "codex")[]; preview: boolean }> {
    if (this.sessionHomeCapabilities) return this.sessionHomeCapabilities;
    await this.sessionHomeIndex(repo, false);
    return this.sessionHomeCapabilities ?? { vendors: ["claude"], preview: false };
  }

  diffHunks(id: number, range?: DiffRange): Promise<DiffHunk[]> {
    return this.json(`/v1/tasks/${id}/diff/hunks${rangeQuery(range)}`);
  }

  annotationsList(taskId: number, range?: DiffRange): Promise<RematchedAnnotation[]> {
    return this.json(`/v1/tasks/${taskId}/annotations${rangeQuery(range)}`);
  }

  annotationSave(taskId: number, input: AnnotationSaveInput): Promise<ReviewAnnotation> {
    return this.json(`/v1/tasks/${taskId}/annotations`, {
      method: "POST",
      body: JSON.stringify({
        id: input.id ?? null,
        hunk_id: input.hunk_id,
        path: input.path,
        line: input.line,
        side: input.side,
        body_md: input.body_md,
      }),
    });
  }

  annotationsResend(taskId: number, ids: string[]): Promise<void> {
    return this.empty(`/v1/tasks/${taskId}/annotations/resend`, {
      method: "POST",
      body: JSON.stringify({ ids }),
    });
  }

  partialApply(taskId: number, hunkIds: string[]): Promise<PartialApplyResult> {
    return this.json(`/v1/tasks/${taskId}/partial/apply`, {
      method: "POST",
      body: JSON.stringify({ hunk_ids: hunkIds }),
    });
  }

  partialRollback(taskId: number): Promise<void> {
    return this.empty(`/v1/tasks/${taskId}/partial/rollback`, { method: "POST" });
  }

  ensembleMatrix(ensemble: string): Promise<EnsembleMatrix> {
    return this.json(`/v1/ensembles/${encodeURIComponent(ensemble)}/matrix`);
  }

  ensembleCompose(ensemble: string, winnerTaskId: number, selections: HunkRef[]): Promise<ComposeOutcome> {
    return this.json(`/v1/ensembles/${encodeURIComponent(ensemble)}/compose`, {
      method: "POST",
      body: JSON.stringify({ winner_task_id: winnerTaskId, selections }),
    });
  }

  // 인터뷰는 로컬 전용(v1 제한) — Runner에는 헤드리스 CLI 실행 위임 경로가 없어 명시적으로 거절한다.
  interviewStart(): Promise<InterviewAssessment> {
    return Promise.reject(new Error("원격 Runner에서는 인터뷰를 지원하지 않습니다 (로컬 전용)"));
  }

  interviewCrystallize(): Promise<CrystallizeResult> {
    return Promise.reject(new Error("원격 Runner에서는 인터뷰를 지원하지 않습니다 (로컬 전용)"));
  }

  // 컴포저가 원격에서 패널 자체를 렌더하지 않지만, UI 가드가 바뀌어도 호출이 조용히
  // 성공하지 않도록 transport 레벨에서도 막는다(Plan 0039 DR-P3).
  grillRound(): Promise<GrillRound> {
    return Promise.reject(new Error("원격 Runner에서는 인터뷰를 지원하지 않습니다 (로컬 전용)"));
  }

  grillNote(): Promise<GrillNote> {
    return Promise.reject(new Error("원격 Runner에서는 인터뷰를 지원하지 않습니다 (로컬 전용)"));
  }

  grillSaveNote(): Promise<string> {
    return Promise.reject(new Error("원격 Runner에서는 인터뷰를 지원하지 않습니다 (로컬 전용)"));
  }

  reminderAdd(text: string, delayMinutes: number): Promise<number> {
    return this.json("/v1/reminders", {
      method: "POST",
      body: JSON.stringify({ text, delay_minutes: delayMinutes }),
    });
  }

  subscribeEvents(
    after: number,
    onEvent: (event: RunnerEvent) => void,
    onError: () => void,
  ): () => void {
    const endpoint = this.endpoint();
    // 영속 커서는 하한으로만 쓴다 — 호출자가 더 뒤를 요청했으면 그쪽을 존중한다.
    let lastSequence = Math.max(after, loadSequence(endpoint));
    let stopped = false;
    let attempt = 0;
    let retry: ReturnType<typeof setTimeout> | undefined;
    let socket: RunnerWebSocket | undefined;

    const advance = (sequence: number) => {
      lastSequence = sequence;
      saveSequence(endpoint, sequence);
    };

    // 핸들러를 떼고 닫는다. 그냥 close()하면 onclose가 reconnect를 예약해, 곧바로
    // 새로 연결할 때 소켓이 두 개가 된다.
    const dropSocket = () => {
      if (!socket) return;
      socket.onmessage = null;
      socket.onerror = null;
      socket.onclose = null;
      socket.close();
      socket = undefined;
    };

    const reconnect = () => {
      onError();
      if (stopped || retry) return;
      const delay = backoffDelay(attempt);
      attempt += 1;
      retry = setTimeout(() => {
        retry = undefined;
        connect();
      }, delay);
    };

    const connect = () => {
      if (stopped) return;
      // 토큰이 없으면 서브프로토콜로 자격을 싣지 않는다 — 동일 출처 WS 핸드셰이크에는
      // 세션 쿠키가 자동으로 붙는다.
      const protocols = this.connection.pairingToken
        ? ["praxis", this.connection.pairingToken]
        : ["praxis"];
      socket = this.socketFactory(this.websocketUrl(lastSequence), protocols);
      socket.onmessage = (message) => {
        try {
          const payload: unknown = JSON.parse(message.data);
          if (isWatermark(payload)) {
            // 연결이 성립하고 서버 상태를 받은 시점에만 백오프를 리셋한다.
            attempt = 0;
            if (shouldResetCursor(payload.sequence, lastSequence)) {
              // Runner ledger가 초기화됐다 — 커서를 유지하면 이후 이벤트를 전부 놓친다.
              advance(0);
              dropSocket();
              connect();
            }
            return;
          }
          if (!isRunnerEvent(payload) || payload.sequence <= lastSequence) return;
          advance(payload.sequence);
          onEvent(payload);
        } catch {
          reconnect();
        }
      };
      socket.onerror = reconnect;
      socket.onclose = reconnect;
    };

    // 모바일은 화면 잠금·앱 전환 뒤 소켓이 죽은 채로 남는다. 포그라운드 복귀와 네트워크
    // 복구 시점에 즉시 다시 붙는다 — 백오프가 길게 늘어난 뒤라도 기다리지 않는다.
    const wake = () => {
      if (stopped) return;
      if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
      attempt = 0;
      if (retry) {
        clearTimeout(retry);
        retry = undefined;
      }
      dropSocket();
      connect();
    };
    const target = typeof window === "undefined" ? undefined : window;
    target?.addEventListener("online", wake);
    target?.addEventListener("visibilitychange", wake);

    connect();
    return () => {
      stopped = true;
      target?.removeEventListener("online", wake);
      target?.removeEventListener("visibilitychange", wake);
      if (retry) clearTimeout(retry);
      dropSocket();
    };
  }

  private async task(id: number): Promise<Task> {
    return this.own(await this.json<TaskRow>(`/v1/tasks/${id}`));
  }

  private async json<T>(path: string, init?: RequestInit): Promise<T> {
    const response = await this.request(`${this.endpoint()}${path}`, {
      ...init,
      headers: this.headers(init),
    });
    if (!response.ok) throw await requestError(response);
    return response.json() as Promise<T>;
  }

  private async empty(path: string, init: RequestInit): Promise<void> {
    const response = await this.request(`${this.endpoint()}${path}`, {
      ...init,
      headers: this.headers(init),
    });
    if (!response.ok) throw await requestError(response);
  }

  private headers(init?: RequestInit): HeadersInit {
    return {
      ...(init?.body ? { "Content-Type": "application/json" } : {}),
      // 토큰이 없으면 쿠키 자격으로 간다. 빈 Bearer를 보내면 서버가 그걸 먼저 평가하고
      // 실패시키므로 헤더 자체를 생략해야 한다.
      ...(this.connection.pairingToken
        ? { Authorization: `Bearer ${this.connection.pairingToken}` }
        : {}),
    };
  }

  private endpoint(): string {
    return this.connection.endpoint.replace(/\/$/, "");
  }

  private websocketUrl(after: number): string {
    return `${this.endpoint().replace(/^http/, "ws")}/v1/events/live?after=${after}`;
  }
}


/**
 * Runner 오류 응답 본문(핸들러의 안내문)을 사용자에게 그대로 노출한다. `RunnerRequestError`로
 * 감싸 상태 코드를 보존한다 — `taskCreate`의 세션 승계 거절(404/409) 재분류가 이를 쓴다.
 */
async function requestError(response: Response): Promise<RunnerRequestError> {
  const body = await response.text().catch(() => "");
  const detail = body.trim();
  return new RunnerRequestError(
    detail ? `Runner 요청 실패 (${response.status}): ${detail}` : `Runner 요청 실패 (${response.status})`,
    response.status,
  );
}

/** 409 승계 충돌 본문 `{"error": "...", "task_id": N}`에서 `task_id`를 뽑는다. */
function parseConflictTaskId(message: string): number | null {
  const match = message.match(/"task_id"\s*:\s*(\d+)/);
  return match ? Number(match[1]) : null;
}

function repositoryQuery(repository: string): URLSearchParams {
  return new URLSearchParams({ repository });
}

/** 절대 경로를 (부모 디렉터리, 파일명)으로 나눈다. 루트 직하 파일은 부모가 "/". */
export function splitPath(path: string): { dir: string; name: string } {
  const normalized = path.replace(/\\/g, "/").replace(/\/+$/, "");
  const cut = normalized.lastIndexOf("/");
  if (cut < 0) return { dir: ".", name: normalized };
  return { dir: normalized.slice(0, cut) || "/", name: normalized.slice(cut + 1) };
}

function isRunnerEvent(value: unknown): value is RunnerEvent {
  return (
    typeof value === "object" &&
    value !== null &&
    "sequence" in value &&
    "task_id" in value &&
    "kind" in value
  );
}
