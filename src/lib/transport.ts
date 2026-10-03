import type { ApprovalStatus, ApprovalRepairSession } from "./approval-readiness";
import type {
  AmbiguityScore,
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
  GoalContract,
  GrillNote,
  GrillRound,
  GrillTurn,
  HunkRef,
  InterviewAnswer,
  InterviewAssessment,
  PartialApplyResult,
  QuickOpenTaskCandidate,
  RematchedAnnotation,
  ReviewAnnotation,
  Schedule,
  SessionHomeEntry,
  SessionHomePreview,
  SkillMeta,
  Task,
  VerifyPreview,
  VerifyReport,
} from "./ipc";
import type { AgentRole } from "./agent-role";
import type { MessageReceipt, SideQuestionInput, SideQuestionSnapshot } from "./side-question";

export type TransportKind = "local" | "remote";

/**
 * 작업이 사는 머신의 좌표. `"local"`(이 PC) 또는 등록된 원격 호스트의 이름이다.
 *
 * **서버가 모르는 클라이언트 측 값이다** — Runner는 "누가 어느 프로필로 붙었나"를 알 수 없고,
 * 같은 Runner에 프로필 둘로 붙는 것도 원리상 가능하다. 그래서 `/v1` 계약에 넣지 않고
 * transport가 응답에 주입한다 (ADR 0133 결정 2).
 */
export type HostId = string;

/** 로컬(이 PC) 호스트. 부팅 시 등록되고 해제되지 않는다. */
export const LOCAL_HOST: HostId = "local";

/**
 * 작업의 신원. `id`는 각 호스트 DB의 정수라 **호스트 없이는 유일하지 않다** —
 * 로컬 3번과 원격 3번은 서로 다른 작업이다.
 */
export interface TaskRef {
  host: HostId;
  id: number;
}

/**
 * 세션홈 행 — host 태그가 붙은 뒤의 화면 타입. `Task`가 서버 행에 host를 얻는 것과 같은 자리
 * (`own()` 관례). 다른 호스트로의 제출을 막는 **첫 겹**이다(설계 2026-09-17 결정 11) — 서버가
 * 두 번째 겹(해석으로 존재 확인)을 맡는다. 클라이언트 태그만으로는 강제되지 않는다.
 */
export interface SessionHomeSession extends SessionHomeEntry {
  host: HostId;
  /** 목록 응답 capability로 정규화한다. 구형 Runner는 false다. */
  preview_supported?: boolean;
}

/**
 * 세션 승계(`resumeSession`) 거절 — 로컬 문자열 오류와 원격 JSON 오류를 하나의 모양으로
 * 통일한다. 두 거절은 서로 겹치지 않는다(설계 2026-09-17 결정 9): `not_found`는 "세션이
 * 없거나 인가 밖"을 뭉친 것(존재 열거 방지, 두 사유를 구분해 보여주지 않는다),
 * `conflict`는 같은 세션을 이미 물고 있는 살아 있는 작업이 있다는 뜻이고 `taskId`를 싣는다.
 */
export type SessionResumeErrorKind = "not_found" | "conflict";

export class SessionResumeError extends Error {
  constructor(
    message: string,
    readonly kind: SessionResumeErrorKind,
    readonly taskId?: number,
  ) {
    super(message);
    this.name = "SessionResumeError";
  }
}

/**
 * 로컬 `task_create`의 세션 승계 실패 문구에서 충돌 작업 id를 뽑는다. 로컬(`AdoptError::Conflict`)과
 * 원격(`CreateTaskError::Conflict`) 둘 다 같은 Display 문구 "이 세션은 이미 #<id> 작업이 이어가고
 * 있습니다"를 쓰므로, 정규식 하나로 문구에서 충돌 여부를 판별할 수 있다. 로컬은 상태 코드가 없어
 * 이 방법이 유일하다 — 원격은 409 JSON의 `task_id` 필드를 직접 읽는다(`transport/runner.ts`).
 */
export function parseSessionResumeConflictTaskId(message: string): number | null {
  const match = message.match(/이미 #(\d+) 작업이 이어가고 있습니다/);
  return match ? Number(match[1]) : null;
}

/**
 * React key·Map 키용 문자열 좌표(`"local:3"`). 호스트 인덱스를 상위 비트에 넣은 합성 정수를
 * 쓰지 않는 이유는 ADR 0133 결정 4 — 그 값이 서버 오류·로그에 새면 사람이 읽을 수 없다.
 */
export function taskKey(ref: TaskRef): string {
  return `${ref.host}:${ref.id}`;
}

export interface ReviewProcessQuarantine {
  receipt_id: number;
  task_id: number;
  operation: "verify" | "challenge" | "repair";
  phase: "verify_build" | "verify_test" | "reviewer" | "repair_agent" | "repair_check";
  pgid: number;
  state: "quarantined";
  reason: string;
  detail: string;
  created_at: number;
  updated_at: number;
}

export type ReviewProcessRepairStatus =
  | "resolved_absent"
  | "resolved_terminated"
  | "still_quarantined"
  | "already_resolved";

export interface ReviewProcessRepairResult {
  receipt_id: number;
  status: ReviewProcessRepairStatus;
  reason: string | null;
  detail: string | null;
}

/** P0 화면이 local Tauri와 Runner에 공통으로 요구하는 호출 계약. */
/** diff를 어디부터 뜰 것인가. 생략하면 세션 전체다. */
export type DiffRange = "session" | "uncommitted";

/** 기준점 상태 — 화면이 근사치를 보고 있는지 알리는 데 쓴다. */
export type BaselineStatus = { kind: "pinned" | "degraded" | "legacy" };

/** diff 응답 — 파일 목록과 그것을 만든 기준점의 상태. */
export interface TaskDiffResult {
  files: FileDiff[];
  baseline: BaselineStatus;
  /** 새 Runner가 파일 diff와 같은 기준점에서 만든 리뷰 데이터. */
  review?: { hunks: DiffHunk[]; annotations: RematchedAnnotation[]; warning?: string };
}

export interface PraxisTransport {
  kind: TransportKind;
  /** 이 transport가 대표하는 호스트. 레지스트리의 키이자 `taskList()`가 행에 주입하는 값. */
  hostId: HostId;
  taskList(): Promise<Task[]>;
  taskDiffStat(id: number): Promise<string>;
  taskDiff(id: number, range?: DiffRange, includeReview?: boolean): Promise<TaskDiffResult>;
  fsTree(id: number): Promise<FsNode[]>;
  fsTreePath(repository: string): Promise<FsNode[]>;
  /** 디렉터리 한 단계 나열 — 파일 브라우저의 지연 로딩. 경로가 비면 첫 root를 연다. */
  fsBrowse(path: string): Promise<BrowseResult>;
  /** 브라우저 시작점 목록 (원격은 Runner의 repository_roots, 로컬은 홈). */
  fsRoots(): Promise<string[]>;
  /** 파일 브라우저 조작 — 로컬 전용(설계 0024 D4). Runner에는 대응 엔드포인트가 없어 거절한다.
   *  변경 계열은 백엔드에서 활성 작업 워크트리를 재판정해 막는다. */
  fsCreateFile(dir: string, name: string): Promise<string>;
  fsCreateDir(dir: string, name: string): Promise<string>;
  fsRename(path: string, name: string): Promise<string>;
  fsTrash(path: string): Promise<void>;
  fsCopy(src: string, destDir: string): Promise<string>;
  fsOpenTerminal(path: string): Promise<void>;
  /** 경로가 git 저장소인지 — false면 작업이 격리 없이 직접 모드로 실행된다. */
  gitStatus(path: string): Promise<boolean>;
  /** 폴더를 git 저장소로 초기화(현재 내용을 초기 커밋). 사용자가 명시적으로 요청할 때만. */
  gitInit(path: string): Promise<boolean>;
  /** 새 작업의 base 후보 — 로컬 브랜치 목록과 현재 체크아웃. 로컬 전용. */
  gitBranches(path: string): Promise<BranchList>;
  /** 메인 체크아웃을 기존 로컬 브랜치로 전환하고 갱신된 목록을 돌려준다. 로컬 전용. */
  gitCheckoutBranch(path: string, branch: string): Promise<BranchList>;
  repositoryList(): Promise<string[]>;
  fsRead(id: number, path: string): Promise<FileContent>;
  fsWrite(id: number, path: string, content: string): Promise<number>;
  scheduleList(): Promise<Schedule[]>;
  taskCreate(request: TaskCreateRequest): Promise<Task>;
  /** 끝난 대화 작업을 새 작업으로 이어받는다(벤더 세션 승계) — 로컬 전용, 원격은 거절한다. */
  taskResume(id: number, message: string, clientRef?: string): Promise<Task>;
  /** 실행 중 terminal 작업의 PTY stdin에 원시 입력 전달. 활성 세션이 없으면 reject. */
  taskInput(id: number, data: string): Promise<void>;
  /** 검토 대기 conversation 작업에 후속 턴 메시지 전송(세션 resume). */
  taskMessage(id: number, message: string): Promise<void>;
  /** 세션 모델 오버라이드 교체 — 다음 턴부터 적용된다. 빈 문자열=해제(벤더 기본으로 복귀).
   *  진행 중인 턴은 프로세스가 이미 떠 있어 바뀌지 않는다. 로컬·원격 모두 지원한다. */
  taskModelSet(id: number, model: string): Promise<void>;
  conversationSubmit(id: number, requestId: string, message: string, imagePaths: string[], previewClientRef?: string): Promise<MessageReceipt>;
  conversationReceipt(id: number, requestId: string): Promise<MessageReceipt>;
  sideQuestionRead(id: number): Promise<SideQuestionSnapshot>;
  sideQuestionSend(id: number, input: SideQuestionInput): Promise<SideQuestionSnapshot>;
  sideQuestionCancel(id: number, turnId: number): Promise<SideQuestionSnapshot>;
  sideQuestionReset(id: number, generation: number): Promise<SideQuestionSnapshot>;
  taskCancel(id: number): Promise<void>;
  taskDelete(id: number): Promise<void>;
  taskApprove(id: number): Promise<void>;
  taskApprovalStatus(id: number): Promise<ApprovalStatus>;
  approvalRepairStatus(id: number): Promise<ApprovalRepairSession | null>;
  approvalRepairPrepare(id: number): Promise<ApprovalRepairSession>;
  approvalRepairRun(id: number, sessionId: string): Promise<ApprovalRepairSession>;
  approvalRepairAccept(id: number, sessionId: string): Promise<ApprovalRepairSession>;
  approvalRepairCancel(id: number, sessionId: string): Promise<void>;
  taskDiscard(id: number): Promise<void>;
  taskRunApprove(id: number): Promise<void>;
  taskRunReject(id: number): Promise<void>;
  githubIssuesList(repo: string): Promise<GithubIssuesResult>;
  githubReposList(repos: string[]): Promise<GhRepo[]>;
  githubCreateTaskFromIssue(repo: string, issueNumber: number, agent: string): Promise<Task>;
  githubIssueDelete(repo: string, issueNumber: number): Promise<void>;
  verifySpec(id: number): Promise<VerifyPreview>;
  taskVerify(id: number, previewToken: string): Promise<VerifyReport>;
  reviewProcessQuarantines(): Promise<ReviewProcessQuarantine[]>;
  reviewProcessReconcile(receiptId: number): Promise<ReviewProcessRepairResult>;
  scheduleAdd(request: ScheduleCreateRequest): Promise<number>;
  scheduleRemove(id: number): Promise<void>;
  scheduleSetEnabled(id: number, enabled: boolean): Promise<void>;
  reminderAdd(text: string, delayMinutes: number): Promise<number>;
  /**
   * `/스킬` 자동완성 목록 — 벤더 디렉터리 실측.
   *
   * 원격은 그 호스트의 스킬을 돌려준다. 스킬은 Runner 머신의 홈·프로젝트 레이어에 살기
   * 때문에 로컬 목록을 보여 주면 거짓말이 된다. 구버전 Runner는 엔드포인트가 없어 404로
   * 답하고, 그때는 빈 목록이 정답이다(오류가 아니다).
   */
  skillsList(repo: string): Promise<SkillMeta[]>;
  quickopenSearch(query: string, scopes: string[]): Promise<QuickOpenTaskCandidate[]>;
  /**
   * 세션홈(`~/.claude/projects`)에서 벤더 세션 목록을 가져온다 — 새 대화 작업으로 이어받을
   * 후보(설계 2026-09-17). 반환 행은 `host`가 태그된다(결정 11). `repo`는 기본 필터,
   * `all=true`면 무시된다. 서버가 200개로 자른다.
   */
  sessionHomeIndex(repo: string, all: boolean, query?: string): Promise<SessionHomeSession[]>;
  sessionHomePreview(repo: string, vendor: "claude" | "codex", sessionId: string): Promise<SessionHomePreview>;
  diffHunks(id: number, range?: DiffRange): Promise<DiffHunk[]>;
  annotationsList(taskId: number, range?: DiffRange): Promise<RematchedAnnotation[]>;
  annotationSave(taskId: number, input: AnnotationSaveInput): Promise<ReviewAnnotation>;
  annotationsResend(taskId: number, ids: string[]): Promise<void>;
  partialApply(taskId: number, hunkIds: string[]): Promise<PartialApplyResult>;
  partialRollback(taskId: number): Promise<void>;
  ensembleMatrix(ensemble: string): Promise<EnsembleMatrix>;
  ensembleCompose(ensemble: string, winnerTaskId: number, selections: HunkRef[]): Promise<ComposeOutcome>;
  /** 인터뷰 1차(채점+질문) — 로컬 전용, Runner transport는 명시적 에러(v1 제한). */
  interviewStart(repo: string, instruction: string, agent: string): Promise<InterviewAssessment>;
  /** 인터뷰 2차(결정화) — 로컬 전용, Runner transport는 명시적 에러(v1 제한). */
  interviewCrystallize(
    repo: string,
    instruction: string,
    answers: InterviewAnswer[],
    agent: string,
  ): Promise<CrystallizeResult>;
  /** 그릴 인터뷰 라운드 — 로컬 전용, Runner transport는 명시적 에러. */
  grillRound(
    repo: string,
    instruction: string,
    transcript: GrillTurn[],
    agent: string,
  ): Promise<GrillRound>;
  /** 그릴 인터뷰 노트 생성 — 로컬 전용, Runner transport는 명시적 에러. */
  grillNote(
    repo: string,
    instruction: string,
    transcript: GrillTurn[],
    agent: string,
  ): Promise<GrillNote>;
  /** 노트 파일 저장 — 로컬 전용. LLM 호출이 아니라 파일시스템 쓰기다. */
  grillSaveNote(repo: string, slug: string, markdown: string, date: string): Promise<string>;
}

/** 새 작업 화면이 local Tauri와 Runner에 넘기는 공통 생성 입력. */
export interface TaskCreateRequest {
  purpose_selection?: { source_id: string; canonical_repo: string; plan_revision: number; phase_id: string | null };
  /** Claude 질문 세션에서 도구 실행 전 승인을 받는다. 로컬 Claude 질문 세션에서만 보낸다. */
  approvals?: boolean;
  /** 이 세션이 살 호스트. 생성 시점에 고정되고 이후 바뀌지 않는다 (ADR 0133). */
  host: HostId;
  repo: string;
  instruction: string;
  agent: string;
  role: AgentRole;
  model: string;
  /** Codex 세션 단위 reasoning override. 미전달/빈 값이면 Codex 설정 기본값을 따른다. */
  reasoning_effort?: string;
  /** Local Codex conversation speed. Missing preserves existing CLI defaults. */
  service_tier?: "default" | "fast";
  headless: boolean;
  ensemble: string;
  mode: string;
  cmd: string;
  args: string[];
  cols: number;
  rows: number;
  goal_contract?: GoalContract | null;
  /** 인터뷰 결정화 모호성 점수 — 계약과 함께 1회 기록. 로컬 전용(Runner 경로는 미전송). */
  ambiguity?: AmbiguityScore | null;
  /**
   * 로컬 시작 브랜치. 격리는 worktree 분기 기준, 직접 실행은 메인 checkout 대상이다.
   * 미전달/빈 값이면 레포의 현재 checkout을 쓴다.
   * 로컬 전용 — 원격 Runner는 러너 머신의 체크아웃을 따르므로 미전송.
   */
  base_branch?: string;
  /**
   * 이 생성 요청의 클라이언트 식별자. 넘기면 백엔드가 `task://creating` 진행 이벤트를
   * 이 값과 함께 보낸다. 로컬 전용 — 원격 Runner는 무시한다.
   */
  client_ref?: string;
  /**
   * 세션홈에서 고른 벤더 세션 id — 이 값이 있으면 새 대화 작업이 그 세션을 이어받는다
   * (설계 2026-09-17). `mode`가 `conversation`이어야 한다. 로컬·원격 둘 다 지원 —
   * `taskResume`(작업 id 기반, 로컬 전용)과 다른 경로다.
   */
  resumeSession?: string;
  /** resumeSession의 출처 공급자. 구형 요청에서만 Claude로 기본 처리된다. */
  resumeVendor?: "claude" | "codex";
}

export interface ScheduleCreateRequest {
  label: string;
  cron: string;
  kind: string;
  payload: string;
  tzOffsetSecs: number;
}

const transports = new Map<HostId, PraxisTransport>();
/** 호스트별 연결 세대. 한 호스트의 등록/해제가 **다른 호스트의 세션을 무효화하지 않는다.** */
const hostRevisions = new Map<HostId, number>();
/** 전역 단조 카운터 — `useSyncExternalStore` 재렌더 트리거 전용이지 정합성 판정에 쓰지 않는다. */
let activeTransportRevision = 0;
const transportListeners = new Set<() => void>();

export interface TransportSession {
  readonly host: HostId;
  readonly transport: PraxisTransport;
  readonly revision: number;
}

export type ReviewTransportSession = TransportSession;

function notify(): void {
  activeTransportRevision += 1;
  transportListeners.forEach((listener) => listener());
}

/** 호스트를 레지스트리에 올린다. 같은 인스턴스의 재등록은 세대를 올리지 않는다 —
 *  바뀐 것이 없는데 진행 중인 승인·복원을 무효화하지 않기 위해서다. */
export function registerTransport(transport: PraxisTransport): void {
  const host = transport.hostId;
  if (transports.get(host) === transport) {
    notify();
    return;
  }
  transports.set(host, transport);
  hostRevisions.set(host, (hostRevisions.get(host) ?? 0) + 1);
  notify();
}

/** 호스트를 내린다. 로컬은 내릴 수 없다 — 레지스트리는 항상 최소 한 칸을 갖는다. */
export function unregisterTransport(host: HostId): void {
  if (host === LOCAL_HOST) {
    throw new Error("로컬 호스트는 해제할 수 없습니다");
  }
  if (!transports.delete(host)) return;
  hostRevisions.set(host, (hostRevisions.get(host) ?? 0) + 1);
  notify();
}

export function getTransport(host: HostId): PraxisTransport {
  const transport = transports.get(host);
  if (!transport) throw new Error(`호스트 '${host}'에 연결되어 있지 않습니다`);
  return transport;
}

export function hasHost(host: HostId): boolean {
  return transports.has(host);
}

/** 등록된 호스트. 로컬이 항상 먼저 오고 나머지는 이름순 — 연결 순서에 흔들리지 않는다. */
export function listHosts(): HostId[] {
  const rest = [...transports.keys()].filter((host) => host !== LOCAL_HOST).sort();
  return transports.has(LOCAL_HOST) ? [LOCAL_HOST, ...rest] : rest;
}

export function getTransportRevision(): number {
  return activeTransportRevision;
}

export function captureTransportSession(host: HostId): TransportSession {
  return {
    host,
    transport: getTransport(host),
    revision: hostRevisions.get(host) ?? 0,
  };
}

export function assertTransportSessionCurrent(session: TransportSession): void {
  if (
    session.transport !== transports.get(session.host)
    || session.revision !== (hostRevisions.get(session.host) ?? 0)
  ) {
    throw new Error("연결된 호스트가 변경되었습니다 — 새 호스트에서 다시 시도하세요");
  }
}

export const captureReviewTransportSession = captureTransportSession;
export const assertReviewTransportSessionCurrent = assertTransportSessionCurrent;

export function subscribeTransportChange(listener: () => void): () => void {
  transportListeners.add(listener);
  return () => transportListeners.delete(listener);
}
