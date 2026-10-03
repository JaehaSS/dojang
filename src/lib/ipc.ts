import { invoke } from "@tauri-apps/api/core";
import {
  assertReviewTransportSessionCurrent,
  LOCAL_HOST,
  registerTransport,
  getTransport,
  hasHost,
  listHosts,
  unregisterTransport,
  type DiffRange,
  type HostId,
  type ReviewTransportSession,
  type TaskCreateRequest,
  type TaskRef,
} from "./transport";
import { tauriTransport } from "./transport/tauri";
import { mergeTaskLists, type HostTaskResult, type MergedTaskList } from "./task-list-merge";
import { taskListCache } from "./task-list-cache";
import type { AgentRole } from "./agent-role";
import type { ObservedModel } from "./models";
import type { DesignBoundingRect, DesignCaptureRecord } from "./designmode/types";
import type { EditorCaptureTarget } from "./designmode/editor-capture-target";

registerTransport(tauriTransport);

export type ServiceTier = "default" | "fast";

export interface GoalContract {
  schema_version: 1;
  objective: string;
  acceptance: string[];
  stop_conditions: string[];
  must_preserve: string[];
  protected_paths: string[];
  non_goals: string[];
}

/** 인터뷰 모호성 점수(`interview::AmbiguityScore` 미러) — 가중 계산은 Rust가 수행. */
export interface AmbiguityScore {
  /** 종합 모호성 0.0(명확)~1.0(모호) = 1 − (goal×0.4 + constraints×0.3 + success×0.3) */
  score: number;
  goal: number;
  constraints: number;
  success: number;
}

/** 인터뷰 질문 한 건(`interview::InterviewQuestion` 미러). */
export interface InterviewQuestion {
  id: string;
  dimension: string; // "goal" | "constraints" | "success"
  text: string;
  reason: string;
  options: string[];
}

/** 인터뷰 1차(채점+질문) 결과. questions가 비면 프론트가 즉시 결정화를 연쇄 호출한다. */
export interface InterviewAssessment {
  ambiguity: AmbiguityScore;
  questions: InterviewQuestion[];
}

export interface InterviewAnswer {
  question_id: string;
  answer: string;
}

/** 인터뷰 2차(결정화) 결과 — Goal Contract 드래프트 채움용. dropped = 상한 절단·불량 패턴 드랍 수. */
export interface CrystallizeResult {
  ambiguity: AmbiguityScore;
  acceptance: string[];
  stop_conditions: string[];
  must_preserve: string[];
  protected_paths: string[];
  non_goals: string[];
  dropped: number;
}

/** 그릴 질문 한 건(`interview::grill::GrillQuestion` 미러). 보기(options)가 없는 것이 핵심 —
 *  선택지 목록 대신 모델의 추천 답 하나를 주어 반박할 대상을 만든다(설계 0026 DR-2). */
export interface GrillQuestion {
  id: string;
  text: string;
  recommendation: string;
  why: string;
}

/** 한 라운드의 문답. stateless 호출이라 다음 라운드 프롬프트에 누적 전달된다. */
export interface GrillTurn {
  question: string;
  recommendation: string;
  answer: string;
}

/** 라운드 결과. question이 null이면 종료 신호(프론티어 소진 또는 상한 도달). */
export interface GrillRound {
  question: GrillQuestion | null;
  open_threads: string[];
  round: number;
  /** 상한 11에 닿아 백엔드가 끊었는지 — 자연 종료와 문구를 달리한다. */
  forced_end: boolean;
}

/** 인터뷰 종료 후 생각 정리 노트. 계약이 아니라 문서와 개선된 지시문이 산출물이다. */
export interface GrillNote {
  slug: string;
  markdown: string;
  revised_instruction: string;
  unresolved: string[];
  dropped: number;
}

/** 인터뷰 1차 — 레포 요약 기반 명확도 채점 + 부족 차원 질문 생성 (로컬 전용, Runner 미지원). */
export const interviewStart = (repo: string, instruction: string, agent: string) =>
  getTransport(LOCAL_HOST).interviewStart(repo, instruction, agent);

/** 인터뷰 2차 — 답변 반영 재채점 + Goal Contract 초안 결정화 (로컬 전용, Runner 미지원). */
export const interviewCrystallize = (
  repo: string,
  instruction: string,
  answers: InterviewAnswer[],
  agent: string,
) => getTransport(LOCAL_HOST).interviewCrystallize(repo, instruction, answers, agent);

/** 그릴 인터뷰 라운드 — 질문 1개 + 추천 답 (로컬 전용, Runner 미지원). */
export const grillRound = (
  repo: string,
  instruction: string,
  transcript: GrillTurn[],
  agent: string,
) => getTransport(LOCAL_HOST).grillRound(repo, instruction, transcript, agent);

/** 그릴 인터뷰 노트 생성 — 마크다운 + 개선된 지시문 (로컬 전용, Runner 미지원). */
export const grillNote = (
  repo: string,
  instruction: string,
  transcript: GrillTurn[],
  agent: string,
) => getTransport(LOCAL_HOST).grillNote(repo, instruction, transcript, agent);

/** 노트를 `docs/explorations/`에 저장하고 레포 상대 경로를 반환한다. LLM 호출 아님. */
export const grillSaveNote = (repo: string, slug: string, markdown: string, date: string) =>
  getTransport(LOCAL_HOST).grillSaveNote(repo, slug, markdown, date);

/**
 * 서버(로컬 Tauri command / Runner `/v1`)가 실제로 돌려주는 작업 행. **`host`가 없다** —
 * 어느 프로필로 붙었는지는 붙은 쪽만 아는 사실이기 때문이다 (ADR 0133 결정 2).
 * 화면이 쓰는 타입은 아래 `Task`이고, transport가 경계에서 `host`를 붙여 승격시킨다.
 */
export interface TaskRow {
  id: number;
  repo: string;
  branch: string;
  base: string;
  worktree_path: string;
  instruction: string;
  state: string;
  created_at: number;
  updated_at: number;
  agent?: string | null;
  /** 작업 책임 역할. 구버전/구형 Runner 응답은 프런트에서 implementer로 보강한다. */
  role?: AgentRole | null;
  ensemble?: string | null;
  /** 세션 단위 모델 오버라이드 — null이면 설정의 벤더 기본(`model:<agent>`)을 따른다. */
  model?: string | null;
  /** Codex 세션 단위 reasoning override — null이면 Codex 설정 기본값을 따른다. */
  reasoning_effort?: string | null;
  service_tier?: ServiceTier | null;
  /** 실행 모드 — "terminal"(PTY) 또는 "conversation"(stream-json). */
  mode: string;
  /** 대화 모드 claude session_id — 재시작 후 --resume 근거. 터미널 작업은 null. */
  convo_session_id?: string | null;
  /** 명시적 immutable Goal Contract. null이면 기존 instruction-only 작업. */
  goal_contract?: GoalContract | null;
  /** 인터뷰 결정화 모호성 점수 — 생성 시 1회 기록. 인터뷰 미사용은 null. */
  ambiguity?: AmbiguityScore | null;
  /**
   * 검토 대기의 성격("question" = 에이전트가 답을 기다림). null이면 통상적인 결과 검토 대기.
   * AwaitingReview가 아닌 상태에서는 항상 null. 판독은 lib/task-status.ts.
   */
  awaiting_kind?: string | null;
  /**
   * 큐에 있으나 지금 시작할 수 없는 이유("auth:<vendor>"). null이면 정상 대기.
   * 실패가 아니라 대기이며, 차단이 풀리면 그대로 재개된다. 판독은 lib/task-status.ts.
   */
  blocked_reason?: string | null;
  /**
   * `worktree_path`가 실재하지 않는 비종료 작업 — DB 값이 아니라 목록 조회 시점의 관측이다.
   * 구형 Runner 응답에는 없으므로 optional. 종료 상태에서는 항상 false.
   */
  worktree_missing?: boolean;
  /** 이 작업이 이어받은 원본(종결된) 작업 id. null이면 처음부터 시작한 작업 — `task_resume` 참고. */
  resumed_from?: number | null;
}

/**
 * 화면이 다루는 작업. `id`는 각 호스트 DB의 정수라 **`host`와 함께여야 유일하다** —
 * 로컬 3번과 원격 3번은 서로 다른 작업이다. `taskRef(task)`로 좌표를 뽑아 쓴다.
 */
export interface Task extends TaskRow {
  host: HostId;
  /** 마지막 성공 목록에서 복원한 offline 행. 실행 상태가 아니며 패널 요청은 막는다. */
  stale?: boolean;
}

/** 작업에서 라우팅 좌표만 뽑는다. ipc 래퍼에 넘기는 값이다. */
export const taskRef = (task: Task): TaskRef => ({ host: task.host, id: task.id });

/** QR로 표시할 일회용 페어링 코드. 원문은 발급 응답에만 실린다. */
export interface MobilePairing {
  code: string;
  /** 만료 시각(epoch 초). 기본 5분. */
  expires_at: number;
}

/** 페어링을 마친 모바일 기기 한 대. 토큰 해시는 내려오지 않는다. */
export interface MobileSession {
  id: number;
  label: string;
  scope: string;
  created_at: number;
  last_seen_at: number;
  expires_at: number;
}

/**
 * 지금까지 관측된 실행 모델 (벤더별, 최근 사용순) — 모델 드롭다운 보강용.
 * 로컬 DB 전용(Runner 미지원): 원격 연결 중에는 빈 목록이 되고 카탈로그만 보인다.
 */
export const observedModels = () => invoke<ObservedModel[]>("observed_models");

/** 봇 `/run`·크론 task가 만든 작업의 실행 전 게이트 상태 — 사용자 승인 전까지 에이전트 미실행. */
export const PENDING_APPROVAL = "PendingApproval";

// PendingApproval도 진행 중 취급 — worktree가 이미 존재해 삭제 시 taskDiscard(정리) 경로를 타야 함
// (taskDelete는 종료 상태 전용이며 서버가 PendingApproval을 거부한다).
export const ACTIVE_STATES = ["Created", "Queued", "Running", "AwaitingReview", "Finalizing", PENDING_APPROVAL];

/** 종료 상태 작업을 이력에서 영구 삭제 (진행 중 작업은 taskDiscard 사용). */
export const taskDelete = (ref: TaskRef) => getTransport(ref.host).taskDelete(ref.id);

/** worktree + 브랜치 생성 → 에이전트 PTY 실행 → Task 반환 (cap 초과 시 reject) */
export const taskCreate = (request: TaskCreateRequest) =>
  getTransport(request.host).taskCreate(request);

/**
 * 끝난(Done/Failed/Discarded) 대화 작업을 새 작업으로 이어받는다 — 원본의 벤더 세션을 물려받아
 * `--resume`으로 첫 턴을 돌리고, 새로 생성된 Task를 반환한다. 원본 행은 손대지 않는다.
 * `clientRef`는 `task://creating` 진행 이벤트를 자기 것으로 식별하는 토큰(taskCreate와 동일 관례).
 */
export const taskResume = (host: HostId, id: number, message: string, clientRef?: string) =>
  getTransport(host).taskResume(id, message, clientRef);

export interface Judgment {
  winner: string;
  ranking: string[];
  rationale: string;
  concerns: [string, string][];
}

export interface CandidateBenchmarkMetrics {
  task_id: number;
  agent: string;
  /** 실행 시 CLI에 전달된 task override 또는 agent 기본 설정. */
  model: string | null;
  /** 공급자 stream/session metadata에서 실제 관측한 모델. */
  resolved_model: string | null;
  state: string;
  active_seconds: number;
  user_turns: number;
  completed_turns: number;
  failed_turns: number;
  tool_calls: number;
  tool_errors: number;
  tokens_in: number;
  tokens_out: number;
  cost_usd: number;
}

export type EnsembleSelectionStatus = "pending" | "selected" | "ambiguous";

export interface EnsembleFeedbackEntry {
  ensemble: string;
  updated_at: number;
  candidate_count: number;
  selection_status: EnsembleSelectionStatus;
  selected_task_id: number | null;
  selected_agent: string | null;
  requested_model: string | null;
  resolved_model: string | null;
}

export interface EnsembleFeedbackHistory {
  entries: EnsembleFeedbackEntry[];
  selected_count: number;
  pending_count: number;
  ambiguous_count: number;
}

export const ensembleList = (ensemble: string) =>
  invoke<Task[]>("ensemble_list", { ensemble });

export const ensembleMetrics = (ensemble: string) =>
  invoke<CandidateBenchmarkMetrics[]>("ensemble_metrics", { ensemble });

export const ensembleFeedbackHistory = () =>
  invoke<EnsembleFeedbackHistory>("ensemble_feedback_history");

export const ensembleJudge = (ensemble: string, judgePref = "") =>
  invoke<Judgment>("ensemble_judge", { ensemble, judgePref });

/** 토론 발화자 — 이벤트에 없으면 "미상"이다. 좌측으로 접지 않는다(설계 §4-4).
 *  값은 위치가 아니라 이름이다 — 저장 이벤트가 이 문자열을 영구히 담는다(설계 2026-09-23 D1). */
export type DebateSpeaker = "left" | "right" | "third";

/** 자리 순서 — 발화 순서이자 면의 좌→우 배치다. 2자 토론은 앞의 둘만 쓴다. */
export const DEBATE_SEATS: readonly DebateSpeaker[] = ["left", "right", "third"];

/** 토론이 끝난 이유. 화면 상태는 `running | ended(reason)` 둘뿐이고 배너 문자열만 이것이 가른다. */
export type DebateEndReason = "consensus" | "round_cap" | "aborted" | "error";

/** Persisted structured output from a Code-mode or connector tool. */
export type ToolOutputContent =
  | { type: "text"; text: string }
  | { type: "image"; url: string }
  | { type: "audio"; url: string }
  | { type: "resource"; uri: string; title: string };

/** 대화 모드(Phase 2) 스트리밍 이벤트 — `convo://event` 페이로드(flatten).
 *  `speaker`는 variant가 아니라 **직렬화 지점의 형제 필드**라 유니온 바깥에서 교차로 붙인다. */
export type ConvoEvent = (
  | { id: number; kind: "session_init"; session_id: string }
  | { id: number; kind: "interaction"; interaction_id: string }
  | { id: number; kind: "text_update"; item_id: string; text: string; complete: boolean }
  | { id: number; kind: "text"; text: string; parent_id?: string }
  | { id: number; kind: "tool_use"; name: string; summary: string; tool_id?: string; parent_id?: string }
  | { id: number; kind: "tool_result"; summary: string; is_error: boolean; tool_use_id?: string; parent_id?: string }
  | { id: number; kind: "tool_output"; tool_use_id: string; contents: ToolOutputContent[]; parent_id?: string }
  /** 서브 에이전트가 실제로 돈 모델 — 세션 칩을 오염시키지 않도록 model_snapshot과 분리돼 있다. */
  | { id: number; kind: "subagent_model"; parent_id: string; model: string }
  | {
      id: number;
      kind: "model_snapshot";
      requested?: string;
      resolved?: string;
      source: string;
    }
  | {
      id: number;
      kind: "result";
      text: string;
      is_error: boolean;
      session_id: string;
      cost_usd: number;
      num_turns: number;
      tokens_in: number;
      tokens_out: number;
    }
  | {
      id: number;
      kind: "context_usage";
      context_tokens: number;
      context_window?: number | null;
      observed_at?: number | null;
      source?: string | null;
      valid?: boolean | null;
    }
  /** 의도적 컨텍스트 절단 표시 — 대화 뷰가 구분선으로 렌더한다. 이력은 지우지 않는다. */
  | { id: number; kind: "context_cleared"; text: string }
  /** 토론 경계 — 이후 발화는 단일 세션의 것이다. `context_cleared`와 같은 자리. */
  | { id: number; kind: "debate_ended"; reason: DebateEndReason }
  | { id: number; kind: "other" }
) & { speaker?: DebateSpeaker };

export const convoSend = (id: number, message: string, imagePaths: string[] = []) =>
  invoke("convo_send", { id, message, imagePaths });

/** 저장된 대화 이벤트(kind 태그 JSON, user 포함) + 턴 진행 여부 — 재진입/재시작 복원용. */
export interface ConvoHistoryPayload {
  items: unknown[];
  busy: boolean;
}

export const convoHistory = (id: number) =>
  invoke<ConvoHistoryPayload>("convo_history", { id });

export interface ToolCostRow {
  tool: string;
  calls: number;
  /** 원문 문자 수 합계. `calls_unknown_size`만큼은 여기 빠져 있다. */
  chars: number;
  calls_unknown_size: number;
  /** 모호하지 않은 구간에서만 누적된 실측 토큰 — 0은 "안 먹었다"가 아니라 "귀속 못 했다". */
  attributed_tokens: number;
  attributed_calls: number;
}

export interface ToolCostReport {
  rows: ToolCostRow[];
  total_chars: number;
  peak_context_tokens: number;
  /** 어느 툴에도 귀속시킬 수 없었던 증가분. 0이 아닌 것이 정상이다. */
  unattributed_tokens: number;
}

export const taskToolCost = (id: number) =>
  invoke<ToolCostReport>("task_tool_cost", { id });

export interface ConvoStatus {
  state: "starting" | "running" | "ended_without_result" | "unknown";
  started_at: number;
  last_event_at: number;
  last_operation: string | null;
  checked_at: number;
}

export const convoStatus = (id: number) => invoke<ConvoStatus>("convo_status", { id });

/** 실행 중인 턴 하나의 활동 신호. */
export interface TaskActivityRow {
  task_id: number;
  last_operation: string | null;
  last_event_at: number;
  /** 이번 턴 시작 시각(epoch 초 — 백엔드 `now()`가 초를 준다). 대기 퀴즈의 경과 게이팅이 쓴다. */
  started_at: number;
}

/**
 * 실행 중인 모든 대화 턴의 현재 작업을 한 번에 조회한다.
 * `convoStatus`를 작업 수만큼 부르면 조회마다 백엔드 뮤텍스를 N번 잡는다.
 */
export const taskActivity = () => invoke<TaskActivityRow[]>("task_activity");

// ── 대기 퀴즈 (설계 0044) ──

/** 출제된 문제. **정답이 없다** — 답은 `quizAnswer` 응답으로만 온다. */
export interface QuizItem {
  id: number;
  kind: string;
  question: string;
  /** 단답형이면 null. */
  choices: string[] | null;
  /** 도메인 문제의 근거 본문. */
  source_excerpt: string | null;
}

export interface QuizAnswerResult {
  correct: boolean;
  answer: string;
  explanation: string | null;
}

export interface QuizPendingItem {
  id: number;
  question: string;
  choices: string[] | null;
  answer: string;
  explanation: string | null;
  source_excerpt: string | null;
  doc_title: string | null;
  heading: string | null;
}

/** 큐에 무엇이 남았는가 — 패널을 띄울지 정하는 데만 쓴다. */
export interface QuizAvailability {
  /** 지금 낼 수 있는 문제 수. 풀다 만 문제도 포함된다. */
  askable: number;
  /** 검수 대기 중인 도메인 문제 수. */
  pending_review: number;
}

export interface InsightCard {
  key: string;
  deck: string;
  title: string;
  body: string;
  /** 필수 — 출처 없는 카드는 백엔드가 로드 단계에서 거부한다. */
  source: string;
  tags: string[];
}
/** 지식창고 최상위 폴더 하나. 루트 바로 밑 문서는 `path === "."`이다. */
export interface InsightWikiFolder {
  path: string;
  /** 이 폴더 밑 문서 수 — 범위 밖이어도 센다. */
  notes: number;
  /** 잘라 낸 카드 수. 범위 밖이면 파싱하지 않으므로 0. */
  cards: number;
  /** 이 폴더의 문서가 하나라도 범위에 들었는가. */
  included: boolean;
}
export interface InsightAvailability {
  /** 띄울 수 있는 카드 수(덱 파일 + 지식창고) — 토글과 무관한 사실이다. */
  cards: number;
  /** 덱 파일 수. */
  decks: number;
  /** 덱 파일에서 나온 카드 수. */
  deck_cards: number;
  /** 개인 지식창고에서 읽은 문서 수. 창고가 연결돼 있지 않으면 0. */
  wiki_notes: number;
  /** 지식창고 문서에서 잘라 낸 카드 수. */
  wiki_cards: number;
  /** 카드를 읽은 지식창고 루트. null이면 연결된 창고가 없다. */
  wiki_root: string | null;
  /** 창고의 최상위 폴더 목록 — 범위 밖 폴더도 들어 있다(설정 화면이 고르는 재료). */
  wiki_folders: InsightWikiFolder[];
  /** 카드로 쓸 폴더 범위. null이면 전체(아직 안 고름), 빈 배열이면 지식창고 카드 없음. */
  wiki_scope: string[] | null;
  /** 기능이 켜져 있는가. 게이트는 `enabled && cards > 0`으로 판단한다. */
  enabled: boolean;
  /** 로드 중 버린 것들 — 설정 화면이 보여 준다. */
  warnings: string[];
}

/** 띄울 카드 수. `insightNext`와 달리 **카드를 소비하지 않아** 게이트에서 부를 수 있다. */
export const insightAvailability = () => invoke<InsightAvailability>("insight_availability");
export const insightNext = () => invoke<InsightCard | null>("insight_next");
/** 카드로 쓸 지식창고 폴더 범위. null이면 전체로 되돌리고, 빈 배열이면 지식창고 카드를 끈다. */
export const insightWikiFoldersSet = (folders: string[] | null) =>
  invoke("insight_wiki_folders_set", { folders });
export const insightEnabledGet = () => invoke<boolean>("insight_enabled_get");
export const insightEnabledSet = (on: boolean) => invoke("insight_enabled_set", { on });

/**
 * 큐 개수만 센다. `quizNext`와 달리 **문제를 소비하지 않아** 폴링에서 부를 수 있다.
 * inbox 수집은 여기서도 하므로, 새로 생성된 문제가 이 프로브만으로 DB에 들어온다.
 */
export const quizAvailability = (kinds: string[] = []) =>
  invoke<QuizAvailability>("quiz_availability", { kinds });

/** 다음 문제. `kinds`가 비면 전부에서 뽑는다. 호출 시 inbox를 한 번 거둔다. */
export const quizNext = (kinds: string[] = []) =>
  invoke<QuizItem | null>("quiz_next", { kinds });

export const quizAnswer = (itemId: number, picked: string) =>
  invoke<QuizAnswerResult | null>("quiz_answer", { itemId, picked });

/** 신고 — 다시 출제되지 않는다. */
export const quizReport = (itemId: number) => invoke("quiz_report", { itemId });

/** 검수 통과 — 이 시점부터 출제 대상이 된다. */
export const quizApprove = (itemId: number) => invoke("quiz_approve", { itemId });

export const quizPending = (limit?: number) =>
  invoke<QuizPendingItem[]>("quiz_pending", { limit });

/** 실행 중인 대화 턴 중단 (자식 프로세스 그룹 kill). 합성 중단 result 이벤트가 뒤따른다. */
export const convoInterrupt = (id: number) => invoke("convo_interrupt", { id });

/** 토론의 비좌측 자리 하나 — 있으면 그 작업은 토론 중이다. 좌측은 여전히 `tasks`가 원천이다. */
export interface DebateSide {
  side: Exclude<DebateSpeaker, "left">;
  agent: string;
  model: string | null;
}

/** 비좌측 자리 조회, 자리 순서. 빈 배열이면 토론이 아니다 — "토론 중"은 컬럼이 아니라 행의 존재로 파생한다. */
export const debateSides = (taskId: number) =>
  invoke<DebateSide[]>("debate_sides", { taskId });

/** 토론 상대 한 명. 모델이 없으면 설정의 벤더 기본을 쓴다. */
export interface DebateOpponent {
  agent: string;
  model?: string | null;
}

/** 토론 시작 — 현재 에이전트가 좌측이 되고 고른 상대들이 순서대로 우측·셋째가 된다(1~2명).
 *  라운드 상한은 설정에서만 바꾼다.
 *
 *  **로컬 전용이다.** Runner의 대화 어댑터가 우측 자리가 있는 작업을 아예 거절한다
 *  (`consume_conversation_events`) — 원격에서 자리만 만들면 그 세션은 다음 턴부터 실행 불가가
 *  되고, 화면은 토론이 시작된 것처럼 보인다. 그래서 자리를 만들기 전에 여기서 막는다. */
export const debateStart = (ref: TaskRef, opponents: readonly DebateOpponent[]) => {
  if (getTransport(ref.host).kind !== "local") {
    return Promise.reject(new Error("원격 세션에서는 토론을 시작할 수 없습니다"));
  }
  return invoke("debate_start", {
    taskId: ref.id,
    opponents: opponents.map(({ agent, model }) => ({ agent, model: model ?? null })),
  });
};

/** 토론 끝내기 — 비좌측 벤더 세션만 버리고 좌측(메인) 세션이 남는다. 이벤트는 지우지 않는다. */
export const debateEnd = (taskId: number) => invoke("debate_end", { taskId });

/** 전체 Task 이력 (한 호스트) */
export const taskList = (host: HostId) => getTransport(host).taskList();

/**
 * 연결된 **모든** 호스트의 작업을 한 목록으로. 호스트별로 독립 조회하고 부분 실패를
 * 허용한다 — 순차로 기다리면 느린 호스트 하나가 목록 전체를 잡는다 (ADR 0133).
 *
 * 훑는 대상은 지금 레지스트리에 올라 있는 호스트뿐이다. 그래서 여기 실린 호스트는
 * 모두 "연결을 기대한" 호스트이고, 응답하지 않으면 그대로 실패로 알린다.
 */
export const taskListAll = async (): Promise<MergedTaskList> => {
  const hosts = listHosts();
  const transports = hosts.map((host) => getTransport(host));
  const settled = await Promise.allSettled(transports.map((transport) => transport.taskList()));
  const results: HostTaskResult[] = settled.map((result, index) => ({
      host: hosts[index],
      tasks: result.status === "fulfilled" ? result.value : null,
      error: result.status === "rejected" ? String(result.reason) : null,
    }));
  results.forEach((result, index) => {
    // A response from a replaced tunnel must not overwrite its successor or tear it down.
    if (!hasHost(result.host) || getTransport(result.host) !== transports[index]) {
      result.tasks = null;
      result.error = "연결이 변경되었습니다";
      return;
    }
    if (result.tasks !== null) taskListCache.save(result.host, result.tasks);
    // 붙어 있던 호스트가 응답을 멈췄다 — 죽은 transport를 레지스트리에 남기지 않는다.
    else if (result.host !== LOCAL_HOST && hasHost(result.host)) unregisterTransport(result.host);
  });
  return mergeTaskLists(results, (host) => taskListCache.load(host));
};

/** 특정 작업 변경 요약 (git diff --stat) */
export const taskDiffStat = (ref: TaskRef) => getTransport(ref.host).taskDiffStat(ref.id);

export interface FileDiff {
  path: string;
  status: string;
  patch: string;
}

/** 특정 작업 파일 단위 상세 diff (변경 목록·diff 탭 S-13) */
export const taskDiff = (ref: TaskRef, range?: DiffRange, includeReview?: boolean) =>
  getTransport(ref.host).taskDiff(ref.id, range, includeReview);

export interface FsNode {
  name: string;
  /** worktree 루트 기준 상대 경로 (슬래시) */
  path: string;
  is_dir: boolean;
  children: FsNode[];
}

/** 파일 브라우저가 보는 디렉터리 엔트리 한 개 (한 단계, 재귀 없음). */
export interface DirEntry {
  name: string;
  /** 절대 경로 (슬래시) — 다음 조회에 그대로 넘긴다. */
  path: string;
  is_dir: boolean;
  /** 파일 크기(바이트). 디렉터리는 0. */
  size: number;
  mtime: number;
  /** git 저장소 루트 — 작업 대상으로 바로 고를 수 있다. */
  is_repo: boolean;
  /** 읽을 수 없는 디렉터리 — 진입을 막는다. */
  denied: boolean;
}

/** 디렉터리 한 단계 조회 결과. `parent`가 없으면 더 위로 올라갈 수 없다. */
export interface BrowseResult {
  path: string;
  parent: string | null;
  entries: DirEntry[];
}

/** 파일 종류 — 백엔드 read_file이 판별. text=편집 가능 본문, image=data URL,
 *  table=표 미리보기 JSON(`TablePreview`, parquet), binary/too_large=미리보기 불가(기본 앱/Finder로 열기 유도). */
export type FileKind = "text" | "image" | "table" | "binary" | "too_large";

/** 표 파일(parquet) 미리보기 — 백엔드가 처음 N행·M열만 문자열로 펼쳐 보낸다.
 *  깨진 파일도 `error`를 채운 이 형태로 온다(kind는 그대로 table). */
export interface TablePreview {
  format: string;
  columns: { name: string; type: string }[];
  /** 셀은 문자열, null은 null. 순서는 `columns`와 같다. */
  rows: (string | null)[][];
  total_rows: number;
  total_columns: number;
  row_groups: number;
  shown_rows: number;
  shown_columns: number;
  error: string | null;
}

export interface FileContent {
  kind: FileKind;
  /** text=본문, image=data URL(data:{mime};base64,...), table=TablePreview JSON, binary/too_large="" */
  content: string;
  /** 수정 시각(ms) — 외부 변경 감지 */
  mtime: number;
}

/** IDE: worktree 파일 트리 */
export const fsTree = (ref: TaskRef) => getTransport(ref.host).fsTree(ref.id);
export const fsTreePath = (host: HostId, repository: string) =>
  getTransport(host).fsTreePath(repository);
export const fsBrowse = (host: HostId, path: string) => getTransport(host).fsBrowse(path);
export const fsRoots = (host: HostId) => getTransport(host).fsRoots();
export const fsCreateFile = (dir: string, name: string) =>
  getTransport(LOCAL_HOST).fsCreateFile(dir, name);
export const fsCreateDir = (dir: string, name: string) =>
  getTransport(LOCAL_HOST).fsCreateDir(dir, name);
export const fsRename = (path: string, name: string) =>
  getTransport(LOCAL_HOST).fsRename(path, name);
export const fsTrash = (path: string) => getTransport(LOCAL_HOST).fsTrash(path);
export const fsCopy = (src: string, destDir: string) =>
  getTransport(LOCAL_HOST).fsCopy(src, destDir);
export const fsOpenTerminal = (path: string) => getTransport(LOCAL_HOST).fsOpenTerminal(path);
export const gitStatus = (host: HostId, path: string) => getTransport(host).gitStatus(path);
export const gitInit = (host: HostId, path: string) => getTransport(host).gitInit(path);

/** 새 작업의 base 후보 — `git_branches_path`의 응답. */
export interface BranchList {
  /** 레포가 지금 체크아웃한 브랜치. 고르지 않으면 이 값이 base가 된다. */
  current: string;
  /** 최근 커밋순 로컬 브랜치. `current`도 포함된다. */
  branches: string[];
}

/** 레포의 로컬 브랜치 목록 — 홈 컴포저의 base 브랜치 선택용. */
export const gitBranches = (path: string) => getTransport(LOCAL_HOST).gitBranches(path);

/** 브랜치 피커에서 고른 브랜치로 메인 체크아웃을 전환한다 — 변경 사항은 stash에 보관된다. */
export const gitCheckoutBranch = (path: string, branch: string) =>
  getTransport(LOCAL_HOST).gitCheckoutBranch(path, branch);

/** FsNode 트리를 파일 상대경로 목록으로 평탄화 (@멘션용). */
export const flattenFiles = (nodes: FsNode[]): string[] =>
  nodes.flatMap((n) => (n.is_dir ? flattenFiles(n.children) : [n.path]));
/** IDE: 파일 읽기 (≤2MB) — 종류 판별(text/image/binary/too_large) */
export const fsRead = (ref: TaskRef, path: string) => getTransport(ref.host).fsRead(ref.id, path);
/** IDE: 파일 저장 → 새 mtime(ms) */
export const fsWrite = (ref: TaskRef, path: string, content: string) =>
  getTransport(ref.host).fsWrite(ref.id, path, content);
const LOCAL_FILE_ERROR_PREFIX = /^local_file_(?:denied|not_found|invalid|io|open_failed):\s*/;
/** IDE: 검증된 로컬 파일을 읽기 전용 탭으로 연다. */
export const readLocalFile = async (ref: TaskRef, path: string): Promise<FileContent> => {
  if (getTransport(ref.host).kind !== "local") {
    throw new Error("원격 작업에서는 열 수 없는 위치입니다");
  }
  try {
    return await invoke<FileContent>("read_local_file", { id: ref.id, path });
  } catch (error) {
    const message = typeof error === "string" ? error : error instanceof Error ? error.message : null;
    if (message == null || !LOCAL_FILE_ERROR_PREFIX.test(message)) throw error;
    throw new Error(message.replace(LOCAL_FILE_ERROR_PREFIX, ""));
  }
};
/** IDE: 검증된 로컬 파일을 OS 기본 앱으로 연다. */
export const openLocalFile = async (ref: TaskRef, path: string): Promise<void> => {
  if (getTransport(ref.host).kind !== "local") {
    throw new Error("원격 작업에서는 열 수 없는 위치입니다");
  }
  try {
    await invoke<void>("open_local_file", { id: ref.id, path });
  } catch (error) {
    const message = typeof error === "string" ? error : error instanceof Error ? error.message : null;
    if (message == null || !LOCAL_FILE_ERROR_PREFIX.test(message)) throw error;
    throw new Error(message.replace(LOCAL_FILE_ERROR_PREFIX, ""));
  }
};
/** IDE: worktree 상대 경로 → 절대 경로 (기본 앱/Finder 열기용) */
export const resolveAbsPath = (id: number, path: string) =>
  invoke<string>("resolve_abs_path", { id, path });

/** 정의 이동이 가리키는 위치. 좌표는 Monaco 기준(1-based)으로 백엔드가 맞춰 보낸다. */
export interface LspTarget {
  /** worktree 상대 경로. 밖이면 null이고 abs_path만 유효하다. */
  path: string | null;
  abs_path: string;
  line: number;
  column: number;
  /** worktree 밖(의존성·표준 라이브러리) — 탭으로 열지 않고 경로만 안내한다. */
  external: boolean;
}

export interface LspStatusInfo {
  available: boolean;
  /** 서버 표기명. null이면 지원 언어가 아니다. */
  server: string | null;
  /** 사용 불가 사유 (미설치 등). */
  detail: string | null;
}

export type LspGotoKind = "definition" | "implementation" | "references";

/** IDE: 커서 심볼의 정의/구현/사용처 (⌘B 계열). text = 미저장 편집 포함 현재 버퍼.
 *  로컬 전용 — 언어 서버가 워크트리 파일시스템을 직접 읽는다. */
export interface SearchMatch {
  path: string;
  line: number;
  column: number;
  text: string;
}
export interface SearchResult {
  matches: SearchMatch[];
  /** 상한(200)에 걸려 잘렸는가 — UI가 그 사실을 표시해야 한다. */
  truncated: boolean;
  scanned_files: number;
}

/** 워크트리 전체 내용 검색. 대상은 작업 id로만 정해지므로 워크트리 밖으로 나갈 수 없다. */
export const projectSearch = (id: number, query: string, caseSensitive = false) =>
  invoke<SearchResult>("project_search", { id, query, caseSensitive });

export const lspGoto = (
  id: number,
  path: string,
  text: string,
  line: number,
  column: number,
  kind: LspGotoKind,
) => invoke<LspTarget[]>("lsp_goto", { id, path, text, line, column, kind });

/** IDE: 이 파일에서 정의 이동을 쓸 수 있는지 */
export interface SemanticLegend {
  token_types: string[];
  token_modifiers: string[];
}
export interface SemanticTokens {
  /** 5-tuple 델타 인코딩 원본 — Monaco가 같은 형식을 기대하므로 풀지 않는다. */
  data: number[];
  legend: SemanticLegend;
}

/** 시맨틱 토큰. 서버가 지원하지 않으면 null — 에러가 아니다. */
export const lspSemanticTokens = (id: number, path: string, text: string) =>
  invoke<SemanticTokens | null>("lsp_semantic_tokens", { id, path, text });

export const lspStatus = (id: number, path: string) =>
  invoke<LspStatusInfo>("lsp_status", { id, path });

// 검증 게이트 (evidence)
export interface ValidateSpec {
  build: string | null;
  test: string | null;
  timeout_secs: number;
}
export interface VerifyPreview extends ValidateSpec {
  preview_token: string;
}
export interface CheckResult {
  command: string;
  exit_code: number;
  tail: string;
}
export interface TestSummary {
  passed: number;
  failed: number;
}
export interface VerifyReport {
  spec: ValidateSpec;
  build: CheckResult | null;
  test: CheckResult | null;
  summary: TestSummary | null;
  ready: boolean;
  checks: [string, boolean][];
  warnings: string[];
}
export interface Evidence {
  task_id: number;
  build_cmd: string | null;
  build_exit: number | null;
  test_cmd: string | null;
  test_exit: number | null;
  passed: number;
  failed: number;
  ready: boolean;
  created_at: number;
}
async function runReviewOperation<T>(
  session: ReviewTransportSession,
  operation: () => Promise<T>,
): Promise<T> {
  assertReviewTransportSessionCurrent(session);
  const result = await operation();
  assertReviewTransportSessionCurrent(session);
  return result;
}

/** 검증 명령 미리보기 (실행 전 표시 + 첫 실행 확인용) */
export const verifySpec = (id: number, session: ReviewTransportSession) =>
  runReviewOperation(session, () => session.transport.verifySpec(id));
/** 빌드/테스트 실행 → 증거 + 게이트 */
export const taskVerify = (
  id: number,
  previewToken: string,
  session: ReviewTransportSession,
) => runReviewOperation(session, () => session.transport.taskVerify(id, previewToken));

// Capsule (핸드오프/브리핑)
export interface Capsule {
  task_id: number;
  instruction: string;
  goal_contract: GoalContract | null;
  branch: string;
  state: string;
  changed: string[];
  diff_stat: string;
  evidence_ready: boolean | null;
  evidence_summary: string | null;
  recent: string[];
  next_action: string;
  /** 에이전트가 선언한 계획의 Mermaid 캔버스. 계획이 없으면 빈 문자열. */
  canvas: string;
}
/** 작업 브리핑/핸드오프 객체 조립 (read-only) */
export const taskCapsule = (id: number) => invoke<Capsule>("task_capsule", { id });
/** Capsule을 worktree 컨텍스트 파일에 주입 (다음 세션용). 주입 대상 파일 목록 반환. */
export const capsuleInject = (id: number) => invoke<string[]>("capsule_inject", { id });

/** 컨텍스트를 비우고 캡슐만 들고 이어간다. 캡슐은 파일이 아니라 `tasks.pending_capsule`에
 *  적히고 다음 턴 프롬프트 맨 앞에 붙는다(ADR 0170) — 그래서 알릴 파일 목록이 없다.
 *  worktree·컨텍스트 파일·대화 이력은 그대로다 — 지워지는 것은 벤더 세션 하나뿐이다. */
export const convoContextReset = (id: number) =>
  invoke<void>("convo_context_reset", { id });

/** 토론 라운드 상한(2~5, 기본 3). 범위 밖은 클램프하지 않고 거부한다 — 무엇이 저장됐는지 알아야 한다. */
export const debateRoundCapGet = () => invoke<number>("debate_round_cap_get");
export const debateRoundCapSet = (cap: number) => invoke("debate_round_cap_set", { cap });

/** "검증 실패 시 Approve 차단" 토글 (opt-in, 기본 OFF) */
export const blockUnverifiedGet = () => invoke<boolean>("block_unverified_get");
export const blockUnverifiedSet = (enabled: boolean) =>
  invoke("block_unverified_set", { enabled });

/** LSP 자동주입 토글 (기본 ON) — worktree 언어 감지 후 해당 LSP-MCP 브리지를 .mcp.json에 자동 추가 */
/** base 최신화 토글 (기본 ON) — 워크트리 분기 전에 고른 base를 원격 최신으로 fast-forward한다. */
export const refreshBaseGet = (repo?: string) =>
  invoke<boolean>("refresh_base_get", { repo: repo ?? null });
export const refreshBaseSet = (on: boolean, repo?: string) =>
  invoke("refresh_base_set", { on, repo: repo ?? null });

export const lspAutoinjectGet = () => invoke<boolean>("lsp_autoinject_get");
export const lspAutoinjectSet = (on: boolean) => invoke("lsp_autoinject_set", { on });

/**
 * 워크트리 격리 토글 (기본 ON) — 끄면 새 작업(UI 기원, 비앙상블)이 메인 체크아웃에서 직접 실행됨.
 * `repo`를 주면 그 프로젝트의 유효값(오버라이드 → 전역 기본 순), 생략하면 전역 기본값.
 */
export const useWorktreeGet = (repo?: string) => invoke<boolean>("use_worktree_get", { repo });
/** `repo` 생략 시 전역 기본을, 주면 그 프로젝트 오버라이드를 설정한다. */
export const useWorktreeSet = (on: boolean, repo?: string) =>
  invoke("use_worktree_set", { on, repo });
/** 프로젝트 오버라이드 조회 — null이면 전역 기본을 따르는 상태. */
export const useWorktreeOverrideGet = (repo: string) =>
  invoke<boolean | null>("use_worktree_override_get", { repo });
/** 프로젝트 오버라이드 해제 — 다시 전역 기본을 따른다. */
export const useWorktreeOverrideClear = (repo: string) =>
  invoke("use_worktree_override_clear", { repo });

/** 승인: 머지 + 정리 */
export const taskApprovalStatus = (ref: TaskRef) => getTransport(ref.host).taskApprovalStatus(ref.id);
export const taskApprove = (ref: TaskRef) => getTransport(ref.host).taskApprove(ref.id);

/** 폐기: worktree 제거 */
export const taskDiscard = (ref: TaskRef) => getTransport(ref.host).taskDiscard(ref.id);

/**
 * 승인 머지가 충돌로 실패했음을 알리는 접두어. 뒤에 충돌 파일 목록이 붙는다.
 * 이 접두어가 보이면 폐기 대신 충돌 해소로 갈 수 있다.
 */
export const CONFLICT_ERROR_PREFIX = "MERGE_CONFLICT: ";

/** 승인 실패 메시지에서 충돌 파일 목록을 뽑는다. 충돌이 아니면 null. */
export function conflictPathsFromError(error: unknown): string[] | null {
  const message = typeof error === "string" ? error : (error as Error)?.message;
  if (!message?.startsWith(CONFLICT_ERROR_PREFIX)) return null;
  return message
    .slice(CONFLICT_ERROR_PREFIX.length)
    .split("\n", 1)[0]
    .split(",")
    .map((path) => path.trim())
    .filter(Boolean);
}

/**
 * 충돌 파일 하나. 세 스테이지 모두 없을 수 있다 — both-added면 base가, 삭제/수정 충돌이면 한쪽이 없다.
 * `ours`가 작업 쪽, `theirs`가 base 쪽이다(역방향 머지라 라벨이 직관과 맞는다).
 */
export interface ConflictFile {
  path: string;
  ours: string | null;
  theirs: string | null;
  base: string | null;
  /** ours→theirs unified diff. 한쪽에 파일이 없으면(삭제/수정 충돌) null이다. */
  patch: string | null;
}

/** 파일 하나를 어떻게 해소할지. */
export type ConflictResolution =
  | { kind: "ours" }
  | { kind: "theirs" }
  | { kind: "union" }
  | { kind: "manual"; body: string };

/** 충돌 해소 세션을 연다 — 충돌을 worktree 안에 가둔다. */
export const conflictBegin = (id: number) =>
  invoke<ConflictFile[]>("conflict_begin", { id });

/** 파일 하나를 해소한다. 남은 미해결 경로 목록을 돌려준다. */
export const conflictResolve = (
  id: number,
  path: string,
  resolution: ConflictResolution,
) => invoke<string[]>("conflict_resolve", { id, path, resolution });

/** 해소를 마치고 머지 커밋을 만든다. 이후 승인은 fast-forward로 통과한다. */
export const conflictFinish = (id: number) => invoke("conflict_finish", { id });

/** 세션을 버리고 체크포인트로 원복한다. */
export const conflictAbort = (id: number) => invoke("conflict_abort", { id });

/** 대화 체크포인트 — 파일(worktree 커밋)과 대화(이벤트 경계)를 함께 잡은 한 점. */
export interface ConvoCheckpoint {
  id: number;
  task_id: number;
  label: string;
  worktree_commit: string;
  convo_event_max_id: number;
  ts: number;
}

/** 되감기 요약. `abandoned`는 다시 밟지 말아야 할 접근이다. */
export interface RewindSummary {
  kept: string;
  abandoned: string[];
}

/** 체크포인트 생성 — 지금의 파일과 대화를 함께 표시한다. */
export const checkpointCreate = (id: number, label: string) =>
  invoke<ConvoCheckpoint>("checkpoint_create", { id, label });

/** 작업의 체크포인트 (최신 순). */
export const checkpointList = (id: number) =>
  invoke<ConvoCheckpoint[]>("checkpoint_list", { id });

/**
 * 되감기 — 파일은 원복되고, 대화는 요약만 남기고 새 세션으로 다시 시작한다.
 * 요약 생성이 실패하면 아무것도 파괴되지 않은 채 거부된다.
 */
export const convoRewind = (id: number, checkpointId: number) =>
  invoke<RewindSummary>("convo_rewind", { id, checkpointId });

/** 고아 일괄 종결 결과 — 무엇이 정리됐고 무엇이 왜 남았는지. */
export interface OrphanCleanup {
  retired: number[];
  failed: { id: number; reason: string }[];
}

/**
 * 워크트리가 사라진 작업을 일괄 종결한다 — 브랜치는 남긴다(커밋된 작업물의 유일한 사본).
 *
 * 로컬 전용이라 transport를 거치지 않는다. 원격 Runner의 워크트리는 그쪽 머신에 있어 이 앱이
 * 관측할 수 없고, `worktree_missing`도 원격 응답에서는 채워지지 않는다.
 */
export const tasksDiscardOrphans = () => invoke<OrphanCleanup>("tasks_discard_orphans");

/** 실행 전 게이트 승인 — 봇/크론이 만든 PendingApproval 작업을 에이전트로 실행. */
export const taskRunApprove = (ref: TaskRef) => getTransport(ref.host).taskRunApprove(ref.id);

/** 실행 전 게이트 거부 — worktree 정리 + DISCARDED. */
export const taskRunReject = (ref: TaskRef) => getTransport(ref.host).taskRunReject(ref.id);

/** 특정 작업 stdin */
export const taskWrite = (id: number, data: string) =>
  invoke("task_write", { id, data });

/** 특정 작업 리사이즈 */
export const taskResize = (id: number, cols: number, rows: number) =>
  invoke("task_resize", { id, cols, rows });

/** 작업 PTY 스크롤백 replay(base64) — attach 시 라이브 스트림 구독 전에 호출한다. */
export const taskPtyReplay = (id: number) => invoke<string>("task_pty_replay", { id });

// 워크스페이스 셸 (도구 패널 터미널) — 작업 워크트리의 인터랙티브 셸, 작업 PTY와 별개.
/** 셸 열기 — 이미 열려 있던 세션 재사용이면 true. */
export const shellOpen = (id: number, cols: number, rows: number) =>
  invoke<boolean>("shell_open", { id, cols, rows });
export const shellWrite = (id: number, data: string) => invoke("shell_write", { id, data });
export const shellResize = (id: number, cols: number, rows: number) =>
  invoke("shell_resize", { id, cols, rows });
/** 화면이 셸에서 떨어졌음을 알린다 — 셸은 그대로 살아 있고, 유휴 회수의 첫 조건만 켠다.
 *  이 신호가 없으면 셸은 영원히 회수되지 않는다(ADR 0163 결정 4). */
export const shellDetach = (id: number) => invoke("shell_detach", { id });
/** 워크스페이스 셸 스크롤백 replay(base64) — shell_open 직후, 라이브 스트림 구독 전에 호출한다. */
export const shellReplay = (id: number) => invoke<string>("shell_replay", { id });

// Python 콘솔(IPython) — 에디터 팝아웃의 하단 도크. 워크스페이스 셸과 별개의 PTY(작업당 1개).
export interface ReplOpenResult {
  /** existed=이미 열려 있던 콘솔 재사용, started=새로 띄움, missing=ipython이 없어 띄우지 않음. */
  status: "existed" | "started" | "missing";
  /** missing일 때 설치에 쓸 python3 경로. 그것도 없으면 null — 설치 제안을 하지 않는다. */
  python: string | null;
}
/** 콘솔 열기. `install`이 true면 ipython이 없을 때 pip로 설치한 뒤 띄운다. */
export const replOpen = (id: number, cols: number, rows: number, install = false) =>
  invoke<ReplOpenResult>("repl_open", { id, cols, rows, install });
/** 코드 블록 실행 — bracketed paste로 감싸 여러 줄도 한 셀로 들어간다. 프롬프트 전이면 큐에 쌓인다. */
export const replRun = (id: number, code: string) => invoke("repl_run", { id, code });
export const replWrite = (id: number, data: string) => invoke("repl_write", { id, data });
export const replResize = (id: number, cols: number, rows: number) =>
  invoke("repl_resize", { id, cols, rows });
export const replReplay = (id: number) => invoke<string>("repl_replay", { id });
export const replDetach = (id: number) => invoke("repl_detach", { id });
export const replClose = (id: number) => invoke("repl_close", { id });

// 폰트 설정
export interface FontInfo {
  family: string;
  monospace: boolean;
}
export interface FontSettings {
  ui_family: string;
  ui_size: number;
  code_family: string;
  code_size: number;
}
/** 시스템 설치 폰트 목록 (모노스페이스 우선 정렬) */
export const systemFontsList = () => invoke<FontInfo[]>("system_fonts_list");
/** 현재 폰트 설정 조회 (미설정 키는 기본값으로 채움) */
export const fontSettingsGet = () => invoke<FontSettings>("font_settings_get");
/** 폰트 설정 저장 */
export const fontSettingsSet = (settings: FontSettings) =>
  invoke("font_settings_set", { settings });

// 파일 에디터 설정
/**
 * 파일 트리 치수와 Monaco 동작 옵션. 폰트 설정과 나눠 두는 이유는 소비자가 다르기 때문이다 —
 * 폰트는 터미널·팝아웃까지 구독하지만 이쪽은 에디터 화면만 쓴다.
 *
 * `tree_font_size` 하나만 저장한다. 행 높이·아이콘·들여쓰기는 `lib/editor-settings.ts`가
 * 여기서 파생한다 — 치수를 개별로 저장하면 다시 갈라진다.
 */
export interface EditorSettings {
  tree_font_size: number;
  minimap: boolean;
  word_wrap: boolean;
  tab_size: number;
}
/** 에디터 설정 조회 (미설정 키는 기존 하드코딩 값으로 채워짐) */
export const editorSettingsGet = () => invoke<EditorSettings>("editor_settings_get");
/** 에디터 설정 저장 */
export const editorSettingsSet = (settings: EditorSettings) =>
  invoke("editor_settings_set", { settings });

/** 팝아웃한 에디터 창이 이어받을 파일 목록. 저장된 적이 없으면 빈 상태. */
export interface EditorWindowFiles {
  open_paths: string[];
  active_path: string | null;
}

/** 에디터 창 팝아웃 — 저장된 자리로 되돌린 뒤 보인다. */
export const editorWindowOpen = () => invoke("editor_window_open");
/** 팝인 — 창을 숨긴다(웹뷰는 살려 둔다). */
export const editorWindowHide = () => invoke("editor_window_hide");
/** 창을 앞으로 — 자동 저장이 실패해 사용자의 판단이 필요할 때. */
export const editorWindowFocus = () => invoke("editor_window_focus");
/** 창이 살아 있는지 — focus 실패가 "창이 죽었다"인지 "focus만 실패했다"인지 가른다. */
export const editorWindowAlive = () => invoke<boolean>("editor_window_alive");
/** 창 자리 저장 — 이동·리사이즈가 멎은 뒤 디바운스로 부른다. */
export const editorWindowGeometrySave = () => invoke("editor_window_geometry_save");
/** 열린 파일 목록 저장 — 크래시에도 남도록 변경 시마다 부른다. */
export const editorWindowFilesSave = (
  taskId: number,
  openPaths: string[],
  activePath: string | null,
) => invoke("editor_window_files_save", { taskId, openPaths, activePath });
/** 열린 파일 목록 조회 */
export const editorWindowFilesLoad = (taskId: number) =>
  invoke<EditorWindowFiles>("editor_window_files_load", { taskId });

/** 음성 입력 설정. STT 계약은 OpenAI 호환 `/v1/audio/transcriptions` 하나다(설계 0043). */
export interface VoiceSettings {
  base_url: string;
  model: string;
  /** 빈 문자열이면 인증 헤더를 붙이지 않는다. 로컬 mlx-qwen3-asr 도 키를 요구하므로 대개 채운다. */
  api_key: string;
  language: string;
  hotkey_command: string;
  hotkey_dictation: string;
}
/** 현재 음성 설정 조회 (미설정 키는 기본값으로 채움) */
export const voiceSettingsGet = () => invoke<VoiceSettings>("voice_settings_get");
/** 음성 설정 저장 + 핫키 재등록. 핫키 등록 실패는 reject 로 온다. */
export const voiceSettingsSet = (settings: VoiceSettings) =>
  invoke("voice_settings_set", { settings });
/** STT 엔드포인트 왕복 확인 — 무음 0.5초를 실제로 전사시킨다. */
export const voiceSttTest = (settings: VoiceSettings) => invoke("voice_stt_test", { settings });

/** 로컬 STT 서버(ohr)의 설치·구동 상태. `running` 은 앱이 띄운 자식만 가리킨다. */
export type VoiceServerStatus = {
  installed: boolean;
  binary: string | null;
  running: boolean;
  pid: number | null;
  port: number | null;
};
/** 상태 조회 — 폴링용이라 실패해도 조용히 넘긴다. */
export const voiceServerStatus = () => invoke<VoiceServerStatus>("voice_server_status");
/** 서버 기동. 저장된 값이 아니라 화면의 현재 값으로 띄운다(연결 테스트와 같은 규칙). */
export const voiceServerStart = (settings: VoiceSettings) =>
  invoke<VoiceServerStatus>("voice_server_start", { settings });
/** 앱이 띄운 서버만 내린다 — 밖에서 돌던 프로세스는 건드리지 않는다. */
export const voiceServerStop = () => invoke<VoiceServerStatus>("voice_server_stop");

/** Quick Open(⌘K) 백엔드 후보 한 건 — `db::QuickOpenCandidate`(Rust) 미러.
 *  task|session만 백엔드 소스, 파일/스킬/커맨드는 프론트가 로컬로 조회해 quickopen.ts에서 병합. */
export interface QuickOpenTaskCandidate {
  scope: "task" | "session";
  id: number;
  title: string;
  subtitle: string;
  /** 최근성 가중 기준 — epoch초(Task.updated_at과 동일 단위). */
  updated_at: number;
}

/** tasks/sessions LIKE+최근성 검색 (scopes 빈 배열=전체 허용). */
export const quickopenSearch = (host: HostId, query: string, scopes: string[]) =>
  getTransport(host).quickopenSearch(query, scopes);

/** 세션홈(벤더 세션) 목록 한 건 — Rust `sessionhome::SessionMeta` 미러(로컬 IPC·Runner HTTP 공용)
 *  (설계 2026-09-17 결정 10). `title`·`first_message`는 백엔드가 120자 상한 + 제어문자 제거로
 *  반환하지만, 세션홈은 쓰기 통제가 없는 디렉터리라 렌더 쪽도 **평문으로만** 그려야 방어가
 *  닫힌다(결정 8) — 마크다운·HTML 렌더 금지. */
export interface SessionHomeEntry {
  session_id: string;
  cwd: string | null;
  last_cwd: string | null;
  git_branch: string | null;
  title: string | null;
  first_message: string | null;
  last_active: number;
  messages: number;
  vendor_version: string | null;
  /** 구형 응답은 Claude 세션만 담았으므로 vendor 누락은 claude로 정규화한다. */
  vendor?: "claude" | "codex";
  /** 목록 스캔에서 얻은 마지막 실제 사용자 발언. */
  recent_user_message?: string | null;
}

export interface SessionHomePreview {
  meta: SessionHomeEntry;
  messages: { role: string; text: string }[];
  truncated: boolean;
}

/** 세션홈 목록 조회 — 세션홈 이어받기 화면의 데이터 소스. `all=false`(기본)는 `repo`를 접두로
 *  갖는 cwd만, `all=true`면 접두 필터를 걷는다. `query`는 제목·첫 메시지·cwd 부분 문자열
 *  검색이다. 로컬·원격 모두 지원한다(설계 2026-09-17). */
export const sessionHomeIndex = (host: HostId, repo: string, all: boolean, query?: string) =>
  getTransport(host).sessionHomeIndex(repo, all, query);

export const sessionHomePreview = (host: HostId, repo: string, vendor: "claude" | "codex", sessionId: string) =>
  getTransport(host).sessionHomePreview(repo, vendor, sessionId);

/** 기존 risk::assess_blast 3단계 분류를 hunk가 상속한다(`diffmodel::RiskLevel` 미러). */
export type RiskLevel = "high" | "medium" | "low";

/** unified diff 한 줄 — 원본 접두문자(' '/'+'/'-')로 분류(`diffmodel::DiffLine` 미러). */
export type DiffLine =
  | { kind: "context"; text: string }
  | { kind: "add"; text: string }
  | { kind: "del"; text: string };

/** 구조화 diff hunk 하나(`diffmodel::DiffHunk` 미러). `id`는 세션 재개 후 재매칭의 근거. */
export interface DiffHunk {
  id: string;
  path: string;
  /** [start_line, line_count] — unified diff `@@ -start,count +start,count @@` 그대로. */
  old_range: [number, number];
  new_range: [number, number];
  lines: DiffLine[];
  protected: boolean;
  /** 이미 커밋된 변경. protected와 나란히 있지만 뜻은 정반대다 — protected는 "골라선 안
   *  된다(→ 폐기된다)"이고, 이쪽은 **"손대선 안 된다"**다. 유지도 폐기도 대상이 아니다. */
  committed: boolean;
  risk: RiskLevel;
}

/** 구조화 hunk 목록 — B-1 주석·B-2 부분 승인·B-3 ensemble 조합의 공통 조회 API. */
export const diffHunks = (ref: TaskRef, range?: DiffRange) =>
  getTransport(ref.host).diffHunks(ref.id, range);

/** 리뷰 라인 주석 상태(`annotations::status` 미러). */
export type AnnotationStatus = "draft" | "sent" | "resolved";

/** 리뷰 라인 주석 한 건(`annotations::ReviewAnnotation` 미러). */
export interface ReviewAnnotation {
  id: string;
  task_id: number;
  hunk_id: string;
  path: string;
  line: number;
  side: string;
  body_md: string;
  status: AnnotationStatus;
  created_at: number;
}

/** `annotations_list` 응답 — 현재 diff에 재매칭된 결과. `orphaned`가 true면 UI가 고아 배지를 표시. */
export interface RematchedAnnotation extends ReviewAnnotation {
  matched_hunk_id: string | null;
  orphaned: boolean;
}

/** draft 생성/저장 입력 — `id`가 있으면 본문 갱신(draft만), 없으면 새로 만든다. */
export interface AnnotationSaveInput {
  id?: string | null;
  hunk_id: string;
  path: string;
  line: number;
  side: string;
  body_md: string;
}

/** 작업의 주석 목록(현재 diff에 재매칭됨) — 거터 스레드·고아 배지가 이 결과를 그대로 사용. */
export const annotationsList = (ref: TaskRef, range?: DiffRange) =>
  getTransport(ref.host).annotationsList(ref.id, range);

/** draft 주석 생성/저장(onBlur 자동 저장). */
export const annotationSave = (ref: TaskRef, input: AnnotationSaveInput) =>
  getTransport(ref.host).annotationSave(ref.id, input);

/** 선택한 주석 n건을 재전송 — 규정 포맷으로 합성해 convo resume 첫 메시지로 주입한다. */
export const annotationsResend = (ref: TaskRef, ids: string[]) =>
  getTransport(ref.host).annotationsResend(ref.id, ids);

/** hunk 부분 적용 결과(`commands::PartialApplyResult` 미러) — 확인 스텝 요약·롤백 버튼 활성화용. */
export interface PartialApplyResult {
  checkpoint: string;
  kept_hunk_ids: string[];
  discarded_hunk_ids: string[];
}

/** 선택 hunk만 worktree에 남긴다(비선택 hunk는 역패치로 제거, 되돌리기는 `partialRollback`). */
export const partialApply = (ref: TaskRef, hunkIds: string[]) =>
  getTransport(ref.host).partialApply(ref.id, hunkIds);

/** 부분 적용 롤백 — 체크포인트로 worktree를 완전히 원복한다. */
export const partialRollback = (ref: TaskRef) => getTransport(ref.host).partialRollback(ref.id);

/** hunk 하나를 소유 후보(taskId)+id로 식별하는 참조(`ensemble::HunkRef` 미러). */
export interface HunkRef {
  task_id: number;
  hunk_id: string;
}

/** B-3 후보×파일×hunk 매트릭스(`ensemble::EnsembleMatrix` 미러) — 겹치는 hunk는 배타 그룹으로
 *  묶여 그룹 내 최대 1개만 선택 가능하다. */
export interface EnsembleMatrix {
  candidate_ids: number[];
  exclusive_groups: HunkRef[][];
}

/** ensemble 후보×hunk 매트릭스 — 겹치는 hunk 배타 그룹 계산(UI+백엔드 이중 방어의 기반). */
export const ensembleMatrix = (host: HostId, ensemble: string) =>
  getTransport(host).ensembleMatrix(ensemble);

/** ensemble 조합 병합 결과(`ensemble::ComposeOutcome` 미러) — 되돌리기는 `partialRollback(winnerTaskId)` 재사용. */
export interface ComposeOutcome {
  checkpoint: string;
  applied: HunkRef[];
}

/** 심판 추천 후보(winnerTaskId) worktree에 타 후보의 선택 hunk를 forward 적용한다. */
export const ensembleCompose = (
  host: HostId,
  ensemble: string,
  winnerTaskId: number,
  selections: HunkRef[],
) => getTransport(host).ensembleCompose(ensemble, winnerTaskId, selections);

/** 스킬이 실제로 사는 곳. 같은 이름이 여러 벤더에 있으면 거처가 여럿이다. */
export interface SkillHome {
  vendor: "claude" | "codex" | "antigravity";
  /** 절대 경로 — 화면이 그대로 보여 준다. */
  path: string;
  /** 프로젝트 레이어(`<repo>/.claude/...`)면 true. */
  project: boolean;
}

/** 스킬 메타 — 정본은 각 에이전트 디렉터리다. Praxis는 저장하지 않고 관측만 한다. */
export interface SkillMeta {
  name: string;
  description: string;
  /** `homes`가 전부 글로벌이면 true. */
  global: boolean;
  /** 주 거처의 vendor. 목록을 묶는 기준 — 고아면 `praxis`. */
  source: string;
  /** 거처 전부. 비어 있으면 고아(Praxis 사본만 남은 것). */
  homes: SkillHome[];
  /** 본문 크기 — 확장 시 프롬프트에 들어가는 양. */
  bytes: number;
  /** 원본 frontmatter의 `argument-hint`. 없으면 빈 문자열. */
  argumentHint: string;
  /** `~/.praxis/skills/`에만 존재 — 마이그레이션이 필요하다. */
  orphan: boolean;
}

/** 스킬 목록 조회 — 그 호스트의 벤더 디렉터리 실측 (조회 불가 시 빈 배열). */
export const skillsList = (host: HostId, repo: string) =>
  getTransport(host).skillsList(repo).catch(() => [] as SkillMeta[]);

/** 스킬 본문 읽기 — 주 거처의 파일을 그대로 읽는다. 로컬 고정(SkillsView 미리보기). */
export const skillsRead = (repo: string, name: string) =>
  invoke<string>("skills_read", { repo, name });

/**
 * 캡처·회고 호출의 실행 프로파일.
 *
 * `model`/`effort`는 **실효값**(비어 있으면 코드 기본값으로 접힌 결과)이고,
 * `*_raw`는 사용자가 명시한 값이다. 입력란은 raw를, 안내 문구는 실효값을 보여야 한다 —
 * 빈칸만 보이면 "무엇이 도는지 모른다"는 원래 문제가 설정 화면에서 재현된다.
 */
export interface CaptureProfile {
  model: string;
  effort: string;
  /** false면 시스템 프롬프트·출력 형식·설정 소스를 되돌린다(롤백 스위치). */
  lean: boolean;
  model_raw: string;
  effort_raw: string;
}
export const captureProfileGet = () =>
  invoke<CaptureProfile>("capture_profile_get");
export const captureProfileSet = (
  model: string,
  effort: string,
  lean: boolean,
) => invoke("capture_profile_set", { model, effort, lean });

/** 동시 실행 상한과 허용 범위. 경계는 백엔드가 소유한다 — 프런트에 복제하지 않는다. */
export interface ConcurrencyLimit {
  value: number;
  min: number;
  max: number;
}

/** 현재 동시 실행 상한 (동시에 진행할 수 있는 작업 수) */
export const maxConcurrentGet = () => invoke<ConcurrencyLimit>("max_concurrent_get");

/** 동시 실행 상한 저장. 범위 밖 입력은 경계로 접히므로 반환된 값으로 화면을 갱신할 것. */
export const maxConcurrentSet = (value: number) =>
  invoke<ConcurrencyLimit>("max_concurrent_set", { value });

export interface ShellSpec {
  cmd: string;
  args: string[];
}

/** 플랫폼 기본 셸 (unix=$SHELL, windows=powershell) */
export const defaultShell = () => invoke<ShellSpec>("default_shell");

export interface McpServer {
  id: number;
  name: string;
  command: string;
  args: string;
  enabled: number;
  created_at: number;
}

export interface ModelStat {
  model: string;
  /** 로컬 CLI 기록에서 정규화한 제공자 */
  provider: string;
  messages: number;
  sessions: number;
  input_tokens: number;
  output_tokens: number;
  cache_creation_tokens: number;
  cache_read_tokens: number;
  /** 구형 fixture/응답과의 호환을 위해 선택값이다. */
  cache_observed_messages?: number;
  cache_observed_input_tokens?: number;
  cache_observed_read_tokens?: number;
  cache_unknown_messages?: number;
  total_tokens: number;
  /** 24칸 — 모델별 시간대 분포 */
  hours: number[];
}

export interface DayStat {
  date: string; // YYYY-MM-DD (로컬)
  messages: number;
  tokens: number;
}

/** 전체 로컬 CLI 기록에서 관측한 일별 캐시 사용량. */
export interface CacheDayStat {
  date: string;
  input_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  observed_messages: number;
  unknown_messages: number;
}

/** Praxis 주 대화에서 접수 시점에 기록한 입력 바이트 요약. */
export interface PromptInjectionSummary {
  accepted_turns: number;
  observed_turns: number;
  unknown_turns: number;
  user_bytes: number;
  sent_user_bytes: number;
  instruction_bytes: number;
  skill_added_bytes: number;
  goal_added_bytes: number;
  capsule_added_bytes: number;
  vault_added_bytes: number;
  instruction_changes: number;
}

/** 프로젝트(cwd) 한 곳의 사용량. */
export interface ProjectStat {
  /** cwd 원본 경로 */
  path: string;
  /** 표시명 — 마지막 세그먼트. 충돌 시 상위 세그먼트를 붙여 구분 */
  name: string;
  sessions: number;
  messages: number;
  total_tokens: number;
  /** 마지막 활동일 YYYY-MM-DD */
  last_active: string;
}

/** 직전 동일 길이 구간 요약 — 증감(Δ) 계산용. */
export interface PeriodSummary {
  sessions: number;
  messages: number;
  total_tokens: number;
  active_days: number;
}

export interface Insights {
  sessions: number;
  messages: number;
  total_tokens: number;
  input_tokens: number;
  output_tokens: number;
  cache_creation_tokens: number;
  cache_read_tokens: number;
  cache_observed_messages?: number;
  cache_observed_input_tokens?: number;
  cache_observed_read_tokens?: number;
  cache_unknown_messages?: number;
  cache_days?: CacheDayStat[];
  /** null/누락은 아직 접수 입력을 읽지 못한 상태다. */
  prompt_injection?: PromptInjectionSummary | null;
  active_days: number;
  current_streak: number;
  longest_streak: number;
  peak_hour: number | null;
  favorite_model: string | null;
  days: DayStat[];
  hours: number[];
  /** 168칸 — 인덱스 = 요일(일=0) * 24 + 시간 */
  weekday_hours: number[];
  models: ModelStat[];
  /** 토큰 내림차순 */
  projects: ProjectStat[];
  /** range="all"이면 비교 대상이 없어 null */
  prev: PeriodSummary | null;
}

export type InsightsRange = "all" | "30d" | "7d";

/** 사용량 인사이트 집계 (Claude·Codex 로컬 CLI 기록 스캔). 로컬 시간대 기준 버킷팅. */
export const insightsCompute = (range: InsightsRange) =>
  invoke<Insights>("insights_compute", {
    range,
    tzOffsetSecs: -new Date().getTimezoneOffset() * 60,
  });

/** 롤링 윈도 한 개의 소진 상태. */
export interface UsageWindow {
  /** 0~100. */
  used_percent: number;
  /** 윈도 리셋 시각(epoch secs). 소스가 주지 않으면 null. */
  resets_at: number | null;
  /** 윈도 길이(분) — 300=5시간, 10080=주간. */
  window_minutes: number | null;
}

/** `stale`는 마지막 성공 관측을 그대로 싣되 새 값을 받지 못했다는 뜻 — 로그아웃이 아니다. */
export type UsageStatus =
  | "ok"
  | "stale"
  | "no_data"
  | "unauthenticated"
  | "unsupported"
  | "error";

/** 벤더 한 곳의 한도 잔량. 값이 없어도 상태 카드로 남아 상태바 자리가 흔들리지 않는다. */
export interface VendorUsage {
  vendor: string;
  label: string;
  status: UsageStatus;
  detail: string | null;
  plan: string | null;
  five_hour: UsageWindow | null;
  weekly: UsageWindow | null;
  /** "session-log" | "statusline" | "oauth" | "manual-token" */
  source: string | null;
  updated_at: number | null;
}

export interface UsageSnapshot {
  vendors: VendorUsage[];
  fetched_at: number;
}

/** 벤더별 한도 잔량. force=true면 OAuth 재조회 간격 캐시를 무시한다(수동 새로고침). */
export const usageSnapshot = (force = false) => invoke<UsageSnapshot>("usage_snapshot", { force });

/** Claude statusline 브리지 상태 — Claude는 잔량을 statusline JSON으로만 노출한다. */
export interface BridgeStatus {
  installed: boolean;
  /** 브리지가 대신 실행해 주는 기존 statusLine 명령. */
  wrapped: string | null;
  /** 미설치 상태에서 자리에 있는 다른 statusLine 명령. */
  foreign: string | null;
}

/** 인증 상태. `expired`를 따로 두지 않는다 — CLI가 만료와 로그아웃을 구분해주지 않고,
 *  사용자가 할 일은 어느 쪽이든 로그인 하나로 같다. */
export type AuthState = "ok" | "logged_out" | "unknown";

/** 설치 방식 — 업데이트 명령이 여기서 파생된다. `unknown`이면 버튼을 비활성화한다. */
export type InstallMethod = "native" | "npm" | "homebrew" | "unknown";

/** 벤더 CLI 한 곳의 인증·버전 상태. */
export interface VendorHealth {
  vendor: string;
  label: string;
  auth: AuthState;
  auth_detail: string | null;
  account: string | null;
  plan: string | null;
  installed: string | null;
  latest: string | null;
  update_available: boolean;
  install_method: InstallMethod;
  /** 실행 파일 실제 경로 — install_method가 unknown일 때 눈으로 확인할 근거. */
  bin_path: string | null;
  checked_at: number;
}

export interface HealthSnapshot {
  vendors: VendorHealth[];
  checked_at: number;
}

/** CLI 인증·버전 상태. force=true면 npm registry 6시간 캐시를 무시한다(수동 새로고침).
 *  인증·버전 조사는 force와 무관하게 매번 다시 한다. */
export const agentHealth = (force = false) => invoke<HealthSnapshot>("agent_health", { force });

/** 앱 안에서 실행하는 CLI 액션. `scratch`는 벤더와 무관한 자유 셸이다. */
export type AgentActionKind = "login" | "logout" | "update" | "doctor" | "scratch";

/** 액션 PTY의 세션 키 — 이벤트 필터링과 write/resize/close에 모두 이 값을 쓴다. */
export const agentActionKey = (kind: AgentActionKind, vendor: string) => `${kind}:${vendor}`;

/** 액션 PTY 열기. 이미 열려 있으면 true. `update`는 실행 중인 작업이 있으면 거부된다. */
export const agentActionOpen = (kind: AgentActionKind, vendor: string, cols: number, rows: number) =>
  invoke<boolean>("agent_action_open", { kind, vendor, cols, rows });

export const agentActionReplay = (key: string) => invoke<string>("agent_action_replay", { key });
export const agentActionWrite = (key: string, data: string) =>
  invoke<void>("agent_action_write", { key, data });
export const agentActionResize = (key: string, cols: number, rows: number) =>
  invoke<void>("agent_action_resize", { key, cols, rows });

/** 로그인이 회복된 벤더의 인증 차단을 푼다. 반환: 다시 대기열로 돌아간 작업 수. */
export const agentAuthReconcile = () => invoke<number>("agent_auth_reconcile");

/** 자동 업데이트 한 건의 결과. `to`는 명령이 끝난 뒤 **실제로 읽어낸** 버전이다 —
 *  명령이 성공해도 버전이 그대로일 수 있어 "성공"과 구분해서 보여준다. */
export interface UpdateOutcome {
  vendor: string;
  label: string;
  from: string | null;
  to: string | null;
  ok: boolean;
  error: string | null;
}

/** 이번 실행의 자동 업데이트 결과.
 *
 *  `skipped`는 업데이트를 하지 않은 이유이고, **성공적으로 아무것도 안 한 경우도 채워진다**
 *  ("모든 CLI가 최신입니다"). "돌아봤지만 최신이었다"와 "아예 돌지 않았다"가 같아 보이면
 *  조용한 실패를 알아챌 수 없다. */
export interface AutoUpdateReport {
  outcomes: UpdateOutcome[];
  skipped: string | null;
  finished_at: number;
}

/** 자동 업데이트 결과가 도착했다. 놓쳐도 `autoUpdateLast`로 읽을 수 있다. */
export const AUTO_UPDATE_EVENT = "autoupdate://done";

/** 시작 시 자동 업데이트 on/off. 미설정은 켜짐이다. */
export const autoUpdateGet = () => invoke<boolean>("auto_update_get");
export const autoUpdateSet = (enabled: boolean) =>
  invoke<boolean>("auto_update_set", { enabled });

/** 이번 실행에서 자동 업데이트가 무엇을 했는지. 업데이트는 설정 패널이 열리기 한참 전에
 *  끝나므로 이벤트만으로는 놓친다. */
export const autoUpdateLast = () => invoke<AutoUpdateReport>("auto_update_last");

/** Antigravity Hub의 재시작 대기 상태.
 *
 *  설치는 하지 않는다 — Hub의 설치는 앱 종료를 요구하고, 남의 앱을 강제로 끄면 사용자가
 *  그 앱에서 하던 일이 사라진다. 사실만 전하고 재시작은 사용자가 고른다. */
export interface HubUpdate {
  installed: string | null;
  downloaded: string | null;
  restart_required: boolean;
}

export const antigravityHubUpdate = () => invoke<HubUpdate>("antigravity_hub_update");

export const usageBridgeStatus = () => invoke<BridgeStatus>("usage_bridge_status");
export const usageBridgeInstall = () => invoke<BridgeStatus>("usage_bridge_install");
export const usageBridgeUninstall = () => invoke<BridgeStatus>("usage_bridge_uninstall");

/**
 * 사용량 조회 전용 장기 토큰(`claude setup-token`) 저장 — 백엔드가 실호출로 검증하고
 * 통과한 것만 키체인에 넣는다. 실패하면 저장되지 않고 사유가 Err로 온다.
 */
export const usageClaudeTokenSet = (token: string) =>
  invoke<VendorUsage>("usage_claude_token_set", { token });

export const usageClaudeTokenClear = () => invoke<void>("usage_claude_token_clear");

/** 저장 여부만 — 토큰 값은 돌려받지 않는다. */
export const usageClaudeTokenStatus = () => invoke<boolean>("usage_claude_token_status");

export interface OutcomeInsights {
  task_count: number;
  accepted_task_count: number;
  goal_contract_task_count: number;
  ready_accepted_task_count: number;
  ensemble_count: number;
  selected_ensemble_count: number;
  ambiguous_ensemble_count: number;
  no_selection_ensemble_count: number;
  average_accept_seconds: number | null;
  no_reexplanation_target_task_count: number;
  no_reexplanation_observed_task_count: number;
  no_reexplanation_success_task_count: number;
  no_reexplanation_unmeasured_task_count: number;
  no_reexplanation_completion_rate: number | null;
}

/** 작업 DB 기반 AX 결과 집계. Claude 사용량 인사이트와 독립적으로 호출한다. */
export const outcomeInsights = (range: InsightsRange) =>
  invoke<OutcomeInsights>("outcome_insights", { range });

// ── 인사이트 재구성 (설계 0054) ──

/** 상태별 현재 분포. */
export interface StateCount {
  state: string;
  count: number;
}

/**
 * 도달 퍼널. `started`·`reviewed`는 현재 상태가 아니라 이력(`task_events`)으로 센다 —
 * 이미 Done인 작업도 실행을 거쳐 왔다.
 */
export interface Funnel {
  total: number;
  started: number;
  reviewed: number;
  done: number;
  discarded: number;
  failed: number;
}

/** 한 달의 폐기율 원재료. */
export interface MonthRate {
  /** YYYY-MM (로컬) */
  month: string;
  total: number;
  discarded: number;
}

/** 후속 입력 **발생 여부**. 횟수가 아니다 — task당 이벤트가 한 행뿐이다. */
export interface FollowupSplit {
  total: number;
  with_followup: number;
}

export interface RoleOutcome {
  role: string;
  count: number;
  done: number;
  discarded: number;
}

export interface TaskPatterns {
  funnel: Funnel;
  states: StateCount[];
  /** **항상 전체 월별.** 범위 칩을 따르지 않는다 — 추세는 잘라내면 추세가 아니다. */
  discard_trend: MonthRate[];
  followup: FollowupSplit;
  role_outcomes: RoleOutcome[];
  /** 착수→종료 소요(초). 표본이 없으면 null. */
  duration_p50: number | null;
  duration_p90: number | null;
}

/** 작업 패턴 집계 — 작업 DB만 읽는다(트랜스크립트 비의존). */
export const taskPatterns = (range: InsightsRange) =>
  invoke<TaskPatterns>("task_patterns", {
    range,
    tzOffsetSecs: -new Date().getTimezoneOffset() * 60,
  });

// ── 발견 공간 신호 카드 (설계 2026-09-28, 슬라이스 T2·T3) ──

/** 신호 세트. L1(교훈 주제, T3)을 더했다 — 나머지 후보는 아직 선택되지 않았다. */
export type SignalId = "S1" | "S2" | "S6" | "L1";

/**
 * 카드 수치를 손으로 다시 셀 수 있게 하는 근거. `Ledger`(교훈 근거, T3)는 원장 번호를 담는다.
 * `Evidence`라는 이름은 `VerifyReport`(§ 검증 리포트)가 이미 쓰고 있어 `CardEvidence`로 둔다.
 */
export type CardEvidence =
  | { kind: "Tasks"; ids: number[] }
  | { kind: "Spend"; project: string | null }
  | { kind: "Ledger"; repo: string; numbers: number[] };

/**
 * 신호 카드 한 장.
 *
 * 이름을 `InsightCard`가 아니라 `SignalCard`로 둔다 — `InsightCard`는 지식창고 덱 카드
 * (`deck`/`title`/`body`/`source`/`tags`, 위 §지식창고)가 이미 쓰고 있는 이름이라, 그대로
 * 쓰면 이름은 같은데 완전히 다른 두 도메인 타입이 충돌한다.
 */
export interface SignalCard {
  /** signal + range + 구간 시작일 — 무시 판정 키였으나, 롤링 윈도우 탓에 날마다 바뀐다.
   *  실제 무시는 `(signal, repo)` 쌍으로 한다(T4) — 이 필드는 표시·정렬용으로만 쓴다. */
  key: string;
  signal: SignalId;
  sentence: string;
  value: number;
  /** 직전 동일 길이 구간의 같은 지표. S2는 예외로 "구간 평균"을 담는다. */
  baseline: number | null;
  sample: number;
  evidence: CardEvidence;
  /** 저장소에 묶인 신호만 값이 있다(S2·L1, S6은 top project가 알려진 저장소일 때만).
   *  메모리 저장 대상 기본값(§8 O2)·무시 키(§6.4)에 쓴다(T4). */
  repo: string | null;
}

/**
 * 신호 카드 조회 — S1(한 번 보고 닫은 작업) · S2(저장소 집중) · S6(토큰 사용량 급증).
 * `tzOffsetSecs`를 `taskPatterns`처럼 자동 계산하지 않고 인자로 받는다 — 화면이 이미
 * 같은 값을 다른 인사이트 호출에도 넘기고 있어, 호출부가 그 값을 재사용할 수 있게 한다.
 */
export const insightCards = (range: InsightsRange, tzOffsetSecs: number) =>
  invoke<SignalCard[]>("insight_cards", { range, tzOffsetSecs });

// ── 발견 카드 행동 (설계 §8, 슬라이스 T4) ──

/** 메모리에 남길 대상 — 저장소 없는 신호는 `USER.md`, 저장소에 묶인 신호는 그 저장소의
 *  `MEMORY.md`(§8 O2). 저장 다이얼로그가 카드에 repo가 있을 때만 전환 옵션을 보여준다. */
export type MemoryTarget = { kind: "User" } | { kind: "Repo"; repo: string };

/** `insightMemoryAppend`의 결과. 상한을 넘기면 쓰지 않고 `Full`을 돌려준다 — 화면은 그
 *  경로를 `memoryFileOpen`으로 열어 사람이 합치게 한다. */
export type AppendOutcome = { kind: "Appended" } | { kind: "Full"; path: string };

/** 카드 문장을 메모리 한 줄로 남긴다(§8 "메모리에 남기기"). 경로는 백엔드가 `target`만으로
 *  조립한다 — 이 커맨드는 경로 인자를 받지 않는다. */
export const insightMemoryAppend = (target: MemoryTarget, text: string) =>
  invoke<AppendOutcome>("insight_memory_append", { target, text });

/** `(signal, repo)`를 7일 동안 무시한다(§6.4) — 저장소 없는 카드는 `repo: null`로 보낸다. */
export const insightCardDismiss = (signal: SignalId, repo: string | null) =>
  invoke<void>("insight_card_dismiss", { signal, repo });

// ── 교훈 주제 (설계 2026-09-28 §6, 슬라이스 T3) ──

/** subject 하나의 빈도. `lessons`는 항목 수가 아니라 이 구간 교훈 줄 수의 합이다. */
export interface SubjectCount {
  subject: string;
  lessons: number;
  /** 직전 동일 길이 구간의 같은 지표. "all"은 직전 구간이 없어 항상 null. */
  prev_lessons: number | null;
  /** 이 합을 재현할 수 있는 원장 번호(교훈이 있는 항목만). */
  numbers: number[];
}

/** 버린 길 한 줄 — `docs/lessons.json`의 `abandoned` 배열 원소 하나가 곧 한 항목이다. */
export interface AbandonedItem {
  number: number;
  date: string;
  subjects: string[];
  text: string;
}

/** 저장소 하나의 교훈 주제. 투영은 있지만 이 구간에 낼 것이 없으면 목록에 나타나지 않는다. */
export interface RepoLessons {
  repo: string;
  /** 이 구간에 속한 원장 항목 수(교훈 유무와 무관). */
  entries_in_window: number;
  subjects: SubjectCount[];
  abandoned: AbandonedItem[];
}

/**
 * 저장소별 교훈 주제 조회 — `docs/lessons.json` 투영이 있는 저장소만 결과에 나타난다(§6.3).
 * 투영이 없거나 호출이 실패하면 빈 배열이 오며, 화면은 섹션 자체를 감춘다(ledger #554 선례).
 */
export const lessonThemes = (range: InsightsRange, tzOffsetSecs: number) =>
  invoke<RepoLessons[]>("lesson_themes", { range, tzOffsetSecs });

/** 에이전트 하나가 스킬 안에서 차지한 몫. */
export interface AgentSlice {
  agent: string;
  calls: number;
  messages: number;
  tokens: number;
}

/** 스킬 하나가 에이전트 안에서 차지한 몫. */
export interface SkillSlice {
  skill: string;
  calls: number;
  messages: number;
  tokens: number;
}

export interface AgentSkillStat {
  /** "main" 또는 서브에이전트 agentType. */
  agent: string;
  /** 스폰 횟수 — 서브에이전트는 트랜스크립트 수, main은 세션 수. */
  runs: number;
  calls: number;
  messages: number;
  tokens: number;
  unattributed_messages: number;
  /** 토큰 내림차순 */
  skills: SkillSlice[];
}

export interface SkillUsageStat {
  skill: string;
  calls: number;
  messages: number;
  tokens: number;
  sessions: number;
  /** 토큰 내림차순 */
  agents: AgentSlice[];
}

export interface AgentSkillUsage {
  /** 토큰 내림차순 */
  agents: AgentSkillStat[];
  /** 토큰 내림차순 */
  skills: SkillUsageStat[];
  attributed_messages: number;
  unattributed_messages: number;
  subagent_runs: number;
}

/**
 * 에이전트 × 스킬 사용 통계. 메인 세션과 서브에이전트 트랜스크립트를 함께 스캔하므로
 * 개요(`insightsCompute`)보다 파일이 많다 — 화면에서 따로 로드한다.
 */
export const insightsAgentSkills = (range: InsightsRange) =>
  invoke<AgentSkillUsage>("insights_agent_skills", {
    range,
    tzOffsetSecs: -new Date().getTimezoneOffset() * 60,
  });

/** 에이전트별 설정 모델 (키=벤더, 값=모델 문자열, 미설정="") */
export const agentModelsGet = () => invoke<Record<string, string>>("agent_models_get");
/** 에이전트 모델 저장. 빈 문자열=해제(CLI 기본값 사용). */
export const agentModelSet = (agent: string, model: string) =>
  invoke("agent_model_set", { agent, model });

export const codexSpeedModels = () => invoke<string[]>("codex_speed_models");
export const taskServiceTierSet = (ref: TaskRef, serviceTier: ServiceTier) => {
  if (ref.host !== LOCAL_HOST) return Promise.reject(new Error("실행 속도 선택은 로컬 Codex 대화에서 지원합니다"));
  return invoke<TaskRow>("task_service_tier_set", { id: ref.id, serviceTier }).then((task): Task => ({ ...task, host: LOCAL_HOST }));
};

/** 이 세션의 모델 오버라이드 교체 — 다음 턴부터 적용된다. 빈 문자열=해제(벤더 기본으로 복귀).
 *
 *  **`TaskRef`를 받는다** — 형제 task 변경들과 같은 이유다. task.id는 각 호스트 DB의 정수라
 *  호스트 없이는 유일하지 않고, 로컬 invoke로 직행하면 원격 3번이 무관한 로컬 3번을 고친다. */
export const taskModelSet = (ref: TaskRef, model: string) =>
  getTransport(ref.host).taskModelSet(ref.id, model);

/** 대화 세션의 다음 턴을 다른 에이전트·모델로 넘긴다.
 *
 *  **로컬 전용이다.** 전환은 벤더 세션을 버리고 핸드오프를 재조립하는 경로이고, Runner에는
 *  대응 엔드포인트가 없다. 모델 교체(`taskModelSet`)와 달리 DB 한 칸을 고치는 일이 아니다. */
export const taskAgentSet = (ref: TaskRef, agent: string, model: string) => {
  if (getTransport(ref.host).kind !== "local") {
    return Promise.reject(new Error("원격 세션에서는 에이전트를 바꿀 수 없습니다"));
  }
  return invoke<Task>("task_agent_set", { id: ref.id, agent, model });
};

export const mcpList = () => invoke<McpServer[]>("mcp_list");
export const mcpAdd = (name: string, command: string, args: string) =>
  invoke<number>("mcp_add", { name, command, args });
export const mcpRemove = (id: number) => invoke("mcp_remove", { id });
export const mcpSetEnabled = (id: number, enabled: boolean) =>
  invoke("mcp_set_enabled", { id, enabled });

/** 데스크톱이 직접 서빙하는 모바일 표면(PWA) — 설계 2026-09-13. */
export interface MobileSurfaceStatus {
  running: boolean;
  port: number;
  prevent_sleep: boolean;
  /** 잠자기 방지를 켰는데 caffeinate가 뜨지 않은 경우 false. */
  sleep_prevented: boolean;
}

export const mobileSurfaceStatus = () => invoke<MobileSurfaceStatus>("mobile_surface_status");
export const mobileSurfaceSetEnabled = (enabled: boolean) =>
  invoke<MobileSurfaceStatus>("mobile_surface_set_enabled", { enabled });
export const mobileSurfaceSetPort = (port: number) =>
  invoke<MobileSurfaceStatus>("mobile_surface_set_port", { port });
export const mobileSurfaceSetPreventSleep = (enabled: boolean) =>
  invoke<MobileSurfaceStatus>("mobile_surface_set_prevent_sleep", { enabled });

/** 일회용 페어링 코드. 원문은 이 응답에만 있다 — 다시 볼 수 없다. */
export interface MobilePairing {
  code: string;
  expires_at: number;
}

export const mobilePairingCreate = () => invoke<MobilePairing>("mobile_pairing_create");

/** 페어링된 기기의 세션. 토큰 원문은 서버에도 없다(해시만). */
export interface MobileSession {
  id: number;
  label: string;
  scope: string;
  created_at: number;
  last_seen_at: number;
  expires_at: number;
}

export const mobileSessionList = () => invoke<MobileSession[]>("mobile_session_list");
export const mobileSessionRevoke = (id: number) =>
  invoke<boolean>("mobile_session_revoke", { id });

/** 폰에서의 승인·되돌리기 허용 토글(기본 OFF). OFF여도 읽기·대화는 된다. */
export const remoteReviewCommandsGet = () => invoke<boolean>("remote_review_commands_get");
export const remoteReviewCommandsSet = (enabled: boolean) =>
  invoke("remote_review_commands_set", { enabled });

/** 크론 예약 — kind="task"(payload={repo,instruction,agent}) | "reminder"(payload={text}). */
export interface Schedule {
  id: number;
  label: string;
  cron: string;
  kind: string;
  payload: string;
  enabled: boolean;
  last_run_at: number | null;
  created_at: number;
  /** 1회성 리마인더면 실행 예정 epoch초, 반복(cron) 스케줄이면 null. */
  run_at: number | null;
  /** Cron 달력 기준 timezone offset (초). 기본값 32400 = KST (+09:00). */
  tz_offset_secs: number;
}

export const scheduleList = (host: HostId) => getTransport(host).scheduleList();
export const scheduleAdd = (
  host: HostId,
  label: string,
  cron: string,
  kind: string,
  payload: string,
  tz_offset_secs: number,
) =>
  getTransport(host).scheduleAdd({
    label,
    cron,
    kind,
    payload,
    tzOffsetSecs: tz_offset_secs,
  });
export const scheduleRemove = (host: HostId, id: number) => getTransport(host).scheduleRemove(id);
export const scheduleSetEnabled = (host: HostId, id: number, enabled: boolean) =>
  getTransport(host).scheduleSetEnabled(id, enabled);

/** Cron 표현식의 다음 N개 실행 시각 미리보기 (timezone 적용). */
export const cronNextRuns = (cron: string, tz_offset_secs: number, count?: number) =>
  invoke<string[]>("cron_next_runs", { cron, tz_offset_secs, count: count ?? 5 });

/** 1회성 상대시각 리마인더 — now+delayMinutes에 발화하는 스케줄 생성. 생성된 id 반환. */
export const reminderAdd = (host: HostId, text: string, delayMinutes: number) =>
  getTransport(host).reminderAdd(text, delayMinutes);

/** 멀티벤더 리뷰 — 벤더별 결과 한 건. text=리뷰 본문 또는(ok=false) 에러 메시지. */
export interface MultiReviewItem {
  vendor: string;
  ok: boolean;
  text: string;
}

export interface MultiReviewResult {
  items: MultiReviewItem[];
  synthesis: string | null;
  /** 파이프라인 리뷰만 채운다: 벤더별 구조화 지적(`{vendor, ok, findings, raw, error}`). */
  vendor_reviews?: unknown;
  /** 파이프라인 리뷰만 채운다: 구조화된 종합(`Synthesis`). */
  synthesis_structured?: unknown;
  /** 파이프라인 리뷰만 채운다: 종합(없으면 벤더 리뷰) 기준 blocking 지적 존재 여부. */
  blocking?: boolean;
}

/** 리뷰 실행 시 사용한 모델 정보. */
export interface ModelInfo {
  vendor: string;
  model: string;
  cmd: string;
}

/** 리뷰 상세 정보 — 콘텐츠, 프롬프트, 모델 정보. */
export interface ReviewDetail {
  content: string;
  prompt_review: string;
  prompt_synthesis: string | null;
  model_info: ModelInfo[];
  synthesis_model: ModelInfo | null;
}

/** 리뷰 실행 결과 — 리뷰 결과 + 상세 정보. */
export interface MultiReviewRun {
  result: MultiReviewResult;
  detail: ReviewDetail;
}

/** 계획 문서/작업 diff/원문 텍스트를 여러 벤더로 병렬 리뷰 + 선택적 종합 판정.
 *  sourceKind="plan" → sourceRef=repo 상대 md 경로, "diff" → sourceRef=task id 문자열, "text" → sourceRef=원문.
 *  벤더별 최대 120s 병렬 실행 — 오래 걸릴 수 있어 호출부에서 로딩 상태 필수.
 *  반환: MultiReviewRun { result, detail } — detail에 콘텐츠/프롬프트/모델정보 포함. */
export const multiReview = (
  repo: string,
  sourceKind: "plan" | "diff" | "text",
  sourceRef: string,
  focus: string,
  vendors: string[],
  synthesize: boolean,
) =>
  invoke<MultiReviewRun>("multi_review", {
    repo,
    sourceKind,
    sourceRef,
    focus,
    vendors,
    synthesize,
  });

/** 저장된 멀티벤더 리뷰 실행 한 건의 메타(결과 본문 제외, 목록용). */
export interface ReviewMeta {
  id: number;
  created_at: number;
  repo: string;
  source_kind: string;
  source_ref: string;
  focus: string;
  ok_count: number;
  total: number;
}

/** 메타 + 벤더별 결과 본문 + 상세 정보를 합친 리뷰 이력 한 건. */
export interface ReviewRecord {
  meta: ReviewMeta;
  result: MultiReviewResult;
  detail: ReviewDetail;
}

/** 리뷰 이력 목록(최신순, 메타만). */
export const reviewHistoryList = () => invoke<ReviewMeta[]>("review_history_list");
/** 리뷰 이력 한 건 전체(메타 + 결과 본문) 조회. */
export const reviewGet = (id: number) => invoke<ReviewRecord>("review_get", { id });
/** 리뷰 이력 삭제. */
export const reviewDelete = (id: number) => invoke<void>("review_delete", { id });

// GitHub 이슈 → 태스크 (C-2)

/** 이슈 라벨(`github::GhLabel` 미러). */
export interface GhLabel {
  name: string;
}

/** GitHub 이슈 한 건(`github::GhIssue` 미러). */
export interface GhIssue {
  number: number;
  title: string;
  labels: GhLabel[];
  updated_at: string;
}

/** 이슈를 볼 수 있는 레포 하나(`github::GhRepo` 미러) — 로컬 경로와 그 `owner/repo`. */
export interface GhRepo {
  path: string;
  owner_repo: string;
}

/** gh 미설치/미인증·비 GitHub 레포를 프론트가 구분해 처리하도록 태그된 응답(PRD F-07
 *  "조용한 비활성"). `unavailable`이면 안내 카드, `not_github_repo`면 섹션 자체를 숨긴다. */
export type GithubIssuesResult =
  | { status: "ready"; owner_repo: string; issues: GhIssue[] }
  | { status: "unavailable" }
  | { status: "not_github_repo" };

/** GitHub 이슈 목록(최근 갱신 30건) — HomeView 섹션. */
export const githubIssuesList = (host: HostId, repo: string) => getTransport(host).githubIssuesList(repo);

/** 후보 경로 중 이슈를 볼 수 있는 레포만 — 홈의 레포 전환 버튼. */
export const githubReposList = (host: HostId, repos: string[]) => getTransport(host).githubReposList(repos);

/** 이슈 번호로 태스크 생성 — 지시문은 제목+본문+`#N` 참조, origin=External(승인 대기). */
export const githubCreateTaskFromIssue = (
  host: HostId,
  repo: string,
  issueNumber: number,
  agent: string,
) => getTransport(host).githubCreateTaskFromIssue(repo, issueNumber, agent);

/** 이슈 삭제 — 닫는 것이 아니라 GitHub 원본을 지운다. 되돌릴 수 없고 레포 admin 권한이 필요하다. */
export const githubIssueDelete = (host: HostId, repo: string, issueNumber: number) =>
  getTransport(host).githubIssueDelete(repo, issueNumber);

// Design Mode 프리뷰 탭 (D-1) — 로컬 전용, transport 파사드를 거치지 않는다(runner 미노출,
// `shellOpen`과 동일한 관행). 자식 웹뷰 좌표는 논리 픽셀(컨테이너 `getBoundingClientRect()`).
export type { DesignBoundingRect, DesignCaptureRecord } from "./designmode/types";

/** 프리뷰 웹뷰가 사는 곳 — 창은 사이드패널 폭에 갇히지 않고, 탭을 떠나도 남는다. */
export type PreviewMode = "inline" | "window";

export interface NativePreviewState {
  taskId: number;
  url: string;
  mode: PreviewMode;
  generation: number;
}

export const designmodeState = (id: number) =>
  invoke<NativePreviewState | null>("designmode_state", { id });

/** 프리뷰 진입 — 독립 창(기본) 또는 자식 웹뷰 생성(이미 있으면 navigate만). */
export const designmodeOpen = (
  id: number,
  url: string,
  bounds: DesignBoundingRect,
  mode: PreviewMode = "window",
) => invoke<void>("designmode_open", { id, url, bounds, mode });

/** 탭 컨테이너 리사이즈 시 위치·크기 동기화. */
export const designmodeSetBounds = (id: number, bounds: DesignBoundingRect) =>
  invoke<void>("designmode_set_bounds", { id, bounds });

/** 열린 프리뷰 탭 재활성화 — 현재 페이지를 다시 탐색하지 않고 위치와 표시 상태만 복원. */
export const designmodeShow = (id: number, bounds: DesignBoundingRect) =>
  invoke<boolean>("designmode_show", { id, bounds });

/** URL 바 이동/새로고침. */
export const designmodeNavigate = (id: number, url: string) =>
  invoke<void>("designmode_navigate", { id, url });

/** 요소 선택 모드 토글. */
export const designmodeSetSelectionMode = (id: number, enabled: boolean) =>
  invoke<void>("designmode_set_selection_mode", { id, enabled });

/** 다른 중앙 탭으로 전환 — 웹뷰를 파괴하지 않고 숨긴다. */
export const designmodeHide = (id: number) => invoke<void>("designmode_hide", { id });

/** 프리뷰 탭을 완전히 닫을 때 웹뷰 해제. */
export const designmodeClose = (id: number) => invoke<void>("designmode_close", { id });

/** Composer 칩 × — 저장된 캡처 파일 삭제. */
export const designmodeRemoveCapture = (id: number, captureId: string) =>
  invoke<void>("designmode_remove_capture", { id, captureId });

/** 중앙 에디터 영역을 캡처해 Composer 첨부 레코드로 저장. */
export const designmodeCaptureEditor = (id: number, capture: EditorCaptureTarget) =>
  invoke<DesignCaptureRecord>("designmode_capture_editor", { id, capture });

/** 사용자가 프리뷰를 되찾는다 — 대기 중인 에이전트 명령은 끊긴다. */
export const previewTakeOver = (id: number) => invoke<void>("preview_take_over", { id });

/** 회수 해제 — 다시 에이전트가 몰 수 있다. */
export const previewRelease = (id: number) => invoke<void>("preview_release", { id });

export interface PreviewWorkbenchRemoteState {
  taskId: number;
  appEpoch: string;
  busy: "busy" | "idle";
  url: string | null;
  convoActive: boolean;
  takenOver: boolean;
  supported: boolean;
  unsupportedReason: string | null;
}

export interface PreviewWorkbenchReceipt {
  requestId: string;
  status: "prepared" | "accepted" | "finished" | "rejected" | "retired" | "invalidated";
  accepted: boolean;
  running: boolean;
  retryable: boolean;
  resultId?: string;
}

export const previewWorkbenchState = (taskId: number) =>
  invoke<PreviewWorkbenchRemoteState>("preview_workbench_state", { taskId });

export const previewWorkbenchPrepare = (
  taskId: number,
  correlationId: string,
  message: string,
  url: string,
  source: "manual" | "preview_queue",
) => invoke<PreviewWorkbenchReceipt>("preview_workbench_prepare", { taskId, correlationId, message, url, source });

export const previewWorkbenchSend = (
  taskId: number,
  requestId: string,
  message: string,
  url: string,
  source: "manual" | "preview_queue",
) => invoke<PreviewWorkbenchReceipt>("preview_workbench_send", { taskId, requestId, message, url, source });

export const previewWorkbenchReceipt = (taskId: number, requestId: string) =>
  invoke<PreviewWorkbenchReceipt>("preview_workbench_receipt", { taskId, requestId });

/** 작업 컴포저 클립보드 이미지 → 캡처 레코드 저장 — 로컬 전용(designmode와 같은 관행).
 *  반환 레코드를 store에 push하면 칩 표시·전송 시 프롬프트 주입·`image_paths` 전달·종결
 *  정리까지 기존 캡처 파이프라인을 그대로 탄다. */
export const pasteCaptureSave = (id: number, dataBase64: string, mime: string) =>
  invoke<DesignCaptureRecord>("paste_capture_save", { id, dataBase64, mime });

/** 홈 컴포저(작업 생성 전) 클립보드 이미지 저장 — repo의 `.praxis/pasted/`에 남기고
 *  절대경로를 돌려준다. 호출부가 그 경로를 지시문 텍스트에 삽입한다. */
export const pasteImageSave = (repo: string, dataBase64: string, mime: string) =>
  invoke<string>("paste_image_save", { repo, dataBase64, mime });

// ── 지식 그래프 (설계 0020) ──
//
// `transport`를 거치지 않고 `invoke`를 직접 부른다. Tauri 전용 커맨드의 기존 관례이면서
// (`convoInterrupt` 등), **transport에 넣지 않는 것 자체가 Runner 격리를 강제**한다 —
// runner.ts에 구현이 없으므로 원격 프로필에서는 애초에 도달할 수 없다 (DR-6).

export interface KnowledgeHit {
  chunk_id: number;
  node_id: number;
  source: string;
  title: string;
  heading: string | null;
  url: string | null;
  snippet: string;
  score: number;
}

export interface KnowledgeVaultEntry {
  root: string;
  /** 색인조차 하지 않을 경로. */
  exclude: string[];
  /** 색인은 하되 임베딩만 건너뛸 경로 — 어휘 검색에는 계속 걸린다. */
  embed_exclude: string[];
}

export interface KnowledgeSyncResult {
  indexed: number;
  skipped: number;
  deleted: number;
  edges: number;
  embedded: number;
}

export const knowledgeSearch = (query: string, limit?: number) =>
  invoke<KnowledgeHit[]>("knowledge_search", { query, limit });

export const knowledgeVaultsGet = () =>
  invoke<{ vaults: KnowledgeVaultEntry[] }>("knowledge_vaults_get");

export const knowledgeVaultsSet = (vaults: KnowledgeVaultEntry[]) =>
  invoke<void>("knowledge_vaults_set", { config: { vaults } });

export const knowledgeSync = () => invoke<KnowledgeSyncResult>("knowledge_sync");

// ── Gmail 소스 (설계 0020 Phase 4) ──

export interface GmailStatus {
  connected: boolean;
  client_id: string;
  /** secret 값은 절대 돌려주지 않는다 — 있는지 여부만 온다. */
  client_secret_set: boolean;
  query: string;
  /** "미연결" | "백필 대기" | "백필 중" | "최신" — 커서에서 파생된다. */
  stage: string;
  indexed: number;
  last_error: string | null;
  /** 빌드에 client가 박혀 있는가 (ADR 0147). 없으면 화면은 BYO 입력만 보여준다. */
  bundled_available: boolean;
  /** 지금 연결에 실제로 쓰일 credential이 번들인가. 사용자가 자기 값을 넣으면 false다. */
  using_bundled: boolean;
}

export interface KnowledgeGmailSyncResult {
  indexed: number;
  skipped: number;
  deleted: number;
  embedded: number;
  /** 참이면 아직 남았다. 화면이 진행률을 갱신하며 다시 부른다. */
  has_more: boolean;
}

export const knowledgeGmailStatus = () =>
  invoke<GmailStatus>("knowledge_gmail_status");

/** `clientSecret`을 비우면 기존 값을 유지한다 — 필터만 고칠 때 연결이 끊기지 않도록. */
export const knowledgeGmailConfigSet = (
  clientId: string,
  query: string,
  clientSecret?: string,
) =>
  invoke<void>("knowledge_gmail_config_set", {
    clientId,
    query,
    clientSecret: clientSecret ?? null,
  });

/** 브라우저로 동의 화면을 열고 콜백을 기다린다. 성공하면 연결된 주소를 준다. */
export const knowledgeGmailConnect = () =>
  invoke<string>("knowledge_gmail_connect");

/** 파괴적 — 흡수한 메일 노드를 전부 지운다. 지워진 개수를 돌려준다. */
export const knowledgeGmailDisconnect = () =>
  invoke<number>("knowledge_gmail_disconnect");

/** 백필 전 규모 어림수 (DR-12: 확인 후 시작). */
export const knowledgeGmailEstimate = () =>
  invoke<number>("knowledge_gmail_estimate");

export const knowledgeGmailSync = (maxBatches?: number) =>
  invoke<KnowledgeGmailSyncResult>("knowledge_gmail_sync", { maxBatches });

// ── 금일 할 일 (설계 0021 · 플랜 0026) ─────────────────────────────────────
// 로컬 전용 — Runner transport를 태우지 않고 invoke로 직결한다 (설계 §3 Scope).

export type {
  DayItem,
  DayItemStatus,
  DaySuggestion,
} from "../components/ide/today-items";
import type {
  DayItem as TodayItem,
  DayItemStatus as TodayItemStatus,
  DaySuggestion as TodaySuggestion,
} from "../components/ide/today-items";

export interface DayClosing {
  day: string;
  closed_at: number;
  done: number;
  open: number;
  dropped: number;
  /** `docs/memory.md` 원장 항목 초안. 원장에 자동으로 쓰지 않는다 (설계 DR-6). */
  draft: string;
}

export const todayList = (day?: string) => invoke<TodayItem[]>("today_list", { day });

/** 날짜 범위 조회(경계 포함). 인사이트 계획 캘린더가 달 단위로 부른다 (설계 0023). */
export const todayRange = (from: string, to: string) =>
  invoke<TodayItem[]>("today_range", { from, to });

export const todayAdd = (title: string, day?: string, repo?: string) =>
  invoke<TodayItem>("today_add", { title, day, repo });

export const todaySetStatus = (id: number, status: TodayItemStatus) =>
  invoke<TodayItem>("today_set_status", { id, status });

export const todayReorder = (day: string, orderedIds: number[]) =>
  invoke<void>("today_reorder", { day, orderedIds });

export const todayRemove = (id: number) => invoke<void>("today_remove", { id });

/** 항목을 다른 레인으로 옮긴다. `to`를 비우면 오늘, `"backlog"`면 백로그. */
export const todayMove = (id: number, to?: string) =>
  invoke<TodayItem>("today_move", { id, to });

export const todayStart = (
  id: number,
  agent: string,
  mode: string,
  model?: string,
  reasoningEffort?: string,
) => invoke<Task>("today_start", { id, agent, mode, model, reasoningEffort });

export const todaySuggest = (day?: string, repo?: string) =>
  invoke<TodaySuggestion[]>("today_suggest", { day, repo });

/** 이미 담은 제안이면 `null`을 돌려준다 (에러가 아니다). */
export const todayTake = (suggestion: TodaySuggestion, day?: string) =>
  invoke<TodayItem | null>("today_take", {
    day,
    title: suggestion.title,
    source: suggestion.source,
    sourceRef: suggestion.source_ref,
    repo: suggestion.repo,
  });

export const todayClose = (day?: string) => invoke<DayClosing>("today_close", { day });
