import { invoke } from "@tauri-apps/api/core";

/**
 * 멀티벤더 파이프라인 — 백엔드 `src-tauri/src/pipeline/`의 IPC 래퍼와 순수 헬퍼.
 * 계약: docs/plans/2026-10-01-multi-vendor-pipeline.md 12절 "IPC 계약". 행 필드는 snake_case, 인자는 Tauri camelCase.
 */

export const PIPELINE_CHANGED_EVENT = "pipeline://changed";

export type RunState =
  | "drafting"
  | "plan_review"
  | "awaiting_plan_approval"
  | "executing"
  | "final_review"
  | "awaiting_merge_approval"
  | "done"
  | "paused"
  | "cancelled"
  | "failed";

export type TicketState =
  | "pending"
  | "running"
  | "verifying"
  | "reviewing"
  | "fixing"
  | "ready"
  | "integrating"
  | "integrated"
  | "escalated"
  | "cancelled";

export interface RunRow {
  id: number;
  repo: string;
  base_branch: string;
  goal: string;
  state: string;
  paused_from: string | null;
  paused_reason: string | null;
  integration_task_id: number | null;
  integration_branch: string | null;
  integration_path: string | null;
  spec_json: string | null;
  spec_revision: number;
  plan_review_id: number | null;
  final_review_id: number | null;
  auto_fix_used: number;
  plan_fix_used: number;
  last_error: string | null;
  created_at: number;
  updated_at: number;
  /** 분할 심판이 clarify·decline을 내고 사용자 답변을 기다리는 동안의 `Clarification` JSON. */
  clarification_json: string | null;
}

/** `pipeline::model::Clarification` — 분할 전에 사용자에게 되돌아온 질문(clarify) 또는 수행 불가 사유(decline). */
export interface Clarification {
  decision: "clarify" | "decline";
  questions: string[];
  reason: string;
}

export interface TicketRow {
  id: number;
  run_id: number;
  key: string;
  title: string;
  body: string;
  vendor: string;
  reviewer_vendor: string | null;
  acceptance_json: string;
  allowed_paths_json: string;
  deps_json: string;
  covers_json: string;
  state: string;
  task_id: number | null;
  attempt: number;
  reassigned: number;
  last_error: string | null;
  updated_at: number;
  /** 에이전트를 새로 띄울 때마다 오르는 세대(재시도·재배정). 백엔드 `TicketRow.gen`. */
  gen?: number;
}

export interface StepRow {
  id: number;
  run_id: number;
  ticket_id: number | null;
  kind: string;
  vendor: string | null;
  step_key: string;
  status: string;
  prompt: string | null;
  output: string | null;
  started_at: number;
  finished_at: number | null;
}

/** `multireview::PipelineReviewMeta` — 리뷰 메타 + 단계. */
export interface PipelineReviewMeta {
  id: number;
  created_at: number;
  repo: string;
  source_kind: string;
  source_ref: string;
  focus: string;
  ok_count: number;
  total: number;
  pipeline_step: string;
}

export interface PipelineDetail {
  run: RunRow;
  tickets: TicketRow[];
  steps: StepRow[];
  reviews: PipelineReviewMeta[];
}

export interface VendorAvailability {
  vendor: string;
  available: boolean;
  /** 사용 불가 사유(예: "PATH에 없음", "할당량 소진: ..."). 가용이면 null. */
  reason?: string | null;
}

/** `pipeline_list` 항목 — 실행 행에 티켓 집계와 진행 중 단계가 붙는다. */
export interface RunListItem extends RunRow {
  tickets_total: number;
  tickets_integrated: number;
  tickets_escalated: number;
  active_step_kind: string | null;
}

/** `pipeline::model::TicketDraft`. */
export interface TicketDraft {
  key: string;
  title: string;
  body: string;
  vendor: string;
  vendor_reason: string;
  acceptance_commands: string[];
  allowed_paths: string[];
  deps: string[];
  covers: string[];
}

export interface Requirement {
  id: string;
  text: string;
  acceptance: string;
}

export interface Spec {
  goal: string;
  requirements: Requirement[];
  out_of_scope: string[];
}

export type Severity = "blocking" | "advisory";

export interface Finding {
  severity: Severity;
  requirement?: string | null;
  file?: string | null;
  line?: number | null;
  summary: string;
  evidence?: string;
}

/** `multireview::verdict::SynthFinding` — 지적 + 출처 벤더. */
export interface SynthFinding extends Finding {
  sources?: string[];
}

export interface Synthesis {
  decision: "pass" | "fix";
  findings: SynthFinding[];
  summary: string;
}

// --- IPC 래퍼 ---

export const pipelineVendors = () => invoke<VendorAvailability[]>("pipeline_vendors");
export const pipelineStart = (repo: string, baseBranch: string, goal: string) =>
  invoke<RunRow>("pipeline_start", { repo, baseBranch, goal });
export const pipelineList = (repo?: string) => invoke<RunListItem[]>("pipeline_list", { repo });
export const pipelineGet = (runId: number) => invoke<PipelineDetail>("pipeline_get", { runId });
export const pipelineUpdateTickets = (runId: number, expectedRevision: number, tickets: TicketDraft[]) =>
  invoke<RunRow>("pipeline_update_tickets", { runId, expectedRevision, tickets });
export const pipelineApprovePlan = (runId: number, expectedRevision: number) =>
  invoke<RunRow>("pipeline_approve_plan", { runId, expectedRevision });
export const pipelineRejectPlan = (runId: number, comment: string) =>
  invoke<RunRow>("pipeline_reject_plan", { runId, comment });
export const pipelineRetryTicket = (runId: number, ticketId: number, vendor?: string) =>
  invoke<RunRow>("pipeline_retry_ticket", { runId, ticketId, vendor });
export const pipelineSkipTicket = (runId: number, ticketId: number) =>
  invoke<RunRow>("pipeline_skip_ticket", { runId, ticketId });
export const pipelineResume = (runId: number) => invoke<RunRow>("pipeline_resume", { runId });
export const pipelineAnswerClarification = (runId: number, answer: string) =>
  invoke<RunRow>("pipeline_answer_clarification", { runId, answer });
export const pipelineCancel = (runId: number) => invoke<RunRow>("pipeline_cancel", { runId });

// --- 라벨 ---

export type Tone = "neutral" | "running" | "attention" | "success" | "failed";

export const RUN_STATE_LABEL = {
  drafting: { label: "분할 중", tone: "running" },
  plan_review: { label: "계획 리뷰 중", tone: "running" },
  awaiting_plan_approval: { label: "계획 승인 대기", tone: "attention" },
  executing: { label: "티켓 실행 중", tone: "running" },
  final_review: { label: "최종 리뷰 중", tone: "running" },
  awaiting_merge_approval: { label: "머지 승인 대기", tone: "attention" },
  done: { label: "완료", tone: "success" },
  paused: { label: "일시정지", tone: "attention" },
  cancelled: { label: "취소됨", tone: "neutral" },
  failed: { label: "실패", tone: "failed" },
} as const satisfies Record<RunState, { label: string; tone: Tone }>;

export const TICKET_STATE_LABEL = {
  pending: { label: "대기", tone: "neutral" },
  running: { label: "구현 중", tone: "running" },
  verifying: { label: "검증 중", tone: "running" },
  reviewing: { label: "리뷰 중", tone: "running" },
  fixing: { label: "수정 중", tone: "running" },
  ready: { label: "통합 대기", tone: "attention" },
  integrating: { label: "통합 중", tone: "running" },
  integrated: { label: "통합됨", tone: "success" },
  escalated: { label: "에스컬레이션", tone: "failed" },
  cancelled: { label: "제외됨", tone: "neutral" },
} as const satisfies Record<TicketState, { label: string; tone: Tone }>;

export const VENDOR_LABEL: Record<string, string> = {
  claude: "Claude",
  codex: "GPT(codex)",
  agy: "Gemini(agy)",
};
export const PIPELINE_VENDORS = ["claude", "codex", "agy"] as const;

export const vendorLabel = (vendor: string | null | undefined): string =>
  vendor ? (VENDOR_LABEL[vendor] ?? vendor) : "-";

export const STEP_KIND_LABEL: Record<string, string> = {
  baseline: "기준선 검증",
  split: "분할",
  plan_review: "계획 리뷰",
  plan_fix: "계획 수정",
  ticket: "티켓 실행",
  verify: "검증",
  ticket_review: "티켓 리뷰",
  ticket_fix: "티켓 수정",
  integrate: "통합",
  conflict: "충돌 해소",
  final_review: "최종 리뷰",
  final_fix: "최종 수정",
};
export const stepKindLabel = (kind: string) => STEP_KIND_LABEL[kind] ?? kind;

export const STEP_STATUS_LABEL: Record<string, string> = {
  running: "진행 중",
  succeeded: "성공",
  failed: "실패",
};
export const stepStatusLabel = (status: string) => STEP_STATUS_LABEL[status] ?? status;

export function runStateInfo(state: string): { label: string; tone: Tone } {
  return (RUN_STATE_LABEL as Record<string, { label: string; tone: Tone }>)[state] ?? { label: state, tone: "neutral" };
}
export function ticketStateInfo(state: string): { label: string; tone: Tone } {
  return (TICKET_STATE_LABEL as Record<string, { label: string; tone: Tone }>)[state] ?? { label: state, tone: "neutral" };
}

// --- 판정 ---

const TERMINAL_RUN_STATES: readonly string[] = ["done", "cancelled", "failed"];
export const isTerminalRun = (run: Pick<RunRow, "state">) => TERMINAL_RUN_STATES.includes(run.state);

export const canApprovePlan = (run: Pick<RunRow, "state"> | null | undefined): boolean =>
  !!run && run.state === "awaiting_plan_approval";

export const hasEscalation = (tickets: Pick<TicketRow, "state">[]): boolean =>
  tickets.some((t) => t.state === "escalated");

export const canResume = (run: Pick<RunRow, "state">) => run.state === "paused";

/** 분할 심판의 질문·거절에 답할 수 있는가 — Drafting에서 멈췄고 질문이 남아 있을 때. */
export const canAnswerClarification = (run: Pick<RunRow, "state" | "paused_from" | "clarification_json">): boolean =>
  run.state === "paused" && run.paused_from === "drafting" && !!run.clarification_json;

/** 질문에 답하지 않고 그대로 분할시킬 때 보내는 고정 답변. 백엔드 `model::PROCEED_ANYWAY_ANSWER`와 같은 문구다. */
export const PROCEED_ANYWAY_ANSWER = "질문에 답하지 않고 그대로 진행한다. 목표를 가장 자연스러운 구현 요청으로 해석해 분할하라.";

/** 목표가 슬래시 명령으로 시작하면 파이프라인은 그 명령을 전개하지 못한다 — 시작 전에 알려 준다. */
export const isSlashCommandGoal = (goal: string): boolean => /^\s*\/[A-Za-z0-9_-]+/.test(goal);

/** 취소는 비종결 상태에서만 허용된다(백엔드 계약). */
export const canCancel = (run: Pick<RunRow, "state">) => !isTerminalRun(run);

export const availableVendors = (vendors: VendorAvailability[]): string[] =>
  vendors.filter((v) => v.available).map((v) => v.vendor);

/** 파이프라인은 서로 다른 벤더 둘 이상이 있어야 교차 리뷰가 성립한다. */
export const MIN_VENDORS = 2;
export const canStartPipeline = (vendors: VendorAvailability[]) => availableVendors(vendors).length >= MIN_VENDORS;

// --- JSON 파싱 ---

function parseJson<T>(text: string | null | undefined, fallback: T): T {
  if (!text) return fallback;
  try {
    return JSON.parse(text) as T;
  } catch {
    return fallback;
  }
}

const stringList = (text: string | null | undefined): string[] => {
  const v = parseJson<unknown>(text, []);
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
};

export const ticketDeps = (t: Pick<TicketRow, "deps_json">) => stringList(t.deps_json);
export const ticketAcceptance = (t: Pick<TicketRow, "acceptance_json">) => stringList(t.acceptance_json);
export const ticketAllowedPaths = (t: Pick<TicketRow, "allowed_paths_json">) => stringList(t.allowed_paths_json);
export const ticketCovers = (t: Pick<TicketRow, "covers_json">) => stringList(t.covers_json);

export function parseSpec(specJson: string | null | undefined): Spec | null {
  const raw = parseJson<Partial<Spec> | null>(specJson, null);
  if (!raw || typeof raw !== "object") return null;
  return {
    goal: raw.goal ?? "",
    requirements: Array.isArray(raw.requirements) ? raw.requirements : [],
    out_of_scope: Array.isArray(raw.out_of_scope) ? raw.out_of_scope : [],
  };
}

export function parseClarification(json: string | null | undefined): Clarification | null {
  const raw = parseJson<Partial<Clarification> | null>(json, null);
  if (!raw || typeof raw !== "object") return null;
  return {
    decision: raw.decision === "decline" ? "decline" : "clarify",
    questions: Array.isArray(raw.questions) ? raw.questions.filter((q): q is string => typeof q === "string" && q.trim() !== "") : [],
    reason: typeof raw.reason === "string" ? raw.reason : "",
  };
}

export function ticketToDraft(t: TicketRow): TicketDraft {
  return {
    key: t.key,
    title: t.title,
    body: t.body,
    vendor: t.vendor,
    vendor_reason: "",
    acceptance_commands: ticketAcceptance(t),
    allowed_paths: ticketAllowedPaths(t),
    deps: ticketDeps(t),
    covers: ticketCovers(t),
  };
}

// --- 리뷰 종합 ---

export function parseSynthesis(text: string | null | undefined): Synthesis | null {
  const raw = parseJson<Partial<Synthesis> | null>(text, null);
  if (!raw || typeof raw !== "object" || !Array.isArray(raw.findings)) return null;
  return {
    decision: raw.decision === "pass" ? "pass" : "fix",
    findings: raw.findings,
    summary: raw.summary ?? "",
  };
}

/** 구조화 필드(`synthesis_structured`)를 우선하고, 없으면 기존 문자열 파싱으로 되돌아간다. */
export function synthesisOf(result: {
  synthesis: string | null;
  synthesis_structured?: unknown;
}): Synthesis | null {
  const s = result.synthesis_structured;
  if (s && typeof s === "object" && Array.isArray((s as Partial<Synthesis>).findings)) {
    const raw = s as Partial<Synthesis>;
    return {
      decision: raw.decision === "pass" ? "pass" : "fix",
      findings: raw.findings as SynthFinding[],
      summary: raw.summary ?? "",
    };
  }
  return parseSynthesis(result.synthesis);
}

/** 구조화 필드(`vendor_reviews`)의 해당 벤더 지적을 우선하고, 없으면 본문 텍스트 파싱으로 되돌아간다. */
export function vendorFindingsOf(
  result: { vendor_reviews?: unknown },
  vendor: string,
  text: string | null | undefined,
): Finding[] | null {
  const list = result.vendor_reviews;
  if (Array.isArray(list)) {
    const hit = list.find((r) => r && typeof r === "object" && (r as { vendor?: unknown }).vendor === vendor) as
      | { ok?: boolean; findings?: unknown }
      | undefined;
    if (hit && hit.ok !== false && Array.isArray(hit.findings)) return hit.findings as Finding[];
  }
  return parseVendorFindings(text);
}

/** 벤더별 원 지적. 벤더 출력 본문(JSON)이 아니면 null — 호출측이 원문 텍스트로 대체한다. */
export function parseVendorFindings(text: string | null | undefined): Finding[] | null {
  const raw = parseJson<{ findings?: unknown } | unknown[] | null>(text, null);
  const list = Array.isArray(raw) ? raw : raw && Array.isArray((raw as { findings?: unknown }).findings) ? (raw as { findings: unknown[] }).findings : null;
  return list ? (list as Finding[]) : null;
}

export function groupFindings<T extends Pick<Finding, "severity">>(
  synthesis: { findings: T[] } | null | undefined,
): { blocking: T[]; advisory: T[] } {
  const blocking: T[] = [];
  const advisory: T[] = [];
  for (const f of synthesis?.findings ?? []) (f.severity === "blocking" ? blocking : advisory).push(f);
  return { blocking, advisory };
}

// --- 티켓 편집 검증(백엔드 기본 검사 미러) ---

export interface TicketErrors {
  title?: string;
  allowed_paths?: string;
  acceptance_commands?: string;
  deps?: string;
}

const isBadPath = (p: string) => p.startsWith("/") || /^[A-Za-z]:[\\/]/.test(p) || p.split(/[\\/]/).includes("..");

export function validateTickets(tickets: TicketDraft[]): Record<string, TicketErrors> {
  const keys = new Set(tickets.map((t) => t.key));
  const out: Record<string, TicketErrors> = {};
  for (const t of tickets) {
    const e: TicketErrors = {};
    if (!t.title.trim()) e.title = "제목을 입력하세요.";
    const paths = t.allowed_paths.map((p) => p.trim()).filter(Boolean);
    if (paths.length === 0) e.allowed_paths = "허용 경로가 하나 이상 필요합니다.";
    else if (paths.some(isBadPath)) e.allowed_paths = "절대 경로와 `..`는 쓸 수 없습니다.";
    if (t.acceptance_commands.map((c) => c.trim()).filter(Boolean).length === 0) {
      e.acceptance_commands = "수용 명령이 하나 이상 필요합니다.";
    }
    if (t.deps.includes(t.key)) e.deps = "자기 자신을 선행으로 둘 수 없습니다.";
    else if (t.deps.some((d) => !keys.has(d))) e.deps = "존재하지 않는 티켓을 선행으로 지정했습니다.";
    if (Object.keys(e).length) out[t.key] = e;
  }
  return out;
}

export const hasTicketErrors = (errors: Record<string, TicketErrors>) => Object.keys(errors).length > 0;

/** 편집 전후가 같은지 — 같으면 update 호출 없이 바로 승인한다. */
export const ticketsEqual = (a: TicketDraft[], b: TicketDraft[]) => JSON.stringify(a) === JSON.stringify(b);

/** 줄 단위 텍스트 ↔ 목록. 빈 줄은 버린다. */
export const linesToList = (text: string): string[] =>
  text.split("\n").map((l) => l.trim()).filter(Boolean);
export const listToLines = (list: string[]) => list.join("\n");

/** 개정 충돌 오류인지 — 백엔드 메시지에 revision/개정이 들어간다고 가정한다. */
export const isRevisionConflict = (e: unknown): boolean => /revision|개정|리비전/i.test(String(e));

/** 단계 소요 시간(초). 시각 단위는 epoch 초이며, 밀리초 값이 오면 보정한다. */
export function stepDurationSecs(step: Pick<StepRow, "started_at" | "finished_at">): number | null {
  if (step.finished_at == null) return null;
  let d = step.finished_at - step.started_at;
  if (step.started_at > 1e12) d /= 1000;
  return Math.max(0, Math.round(d));
}

/** 단계 종류별 시간 예산(초). 드라이버 상수와 같다 — 진행 카드가 "경과/예산"을 보일 때 쓴다. */
export const STEP_BUDGET_SECS: Record<string, number> = {
  baseline: 600,
  split: 600,
  plan_review: 420,
  ticket_review: 300,
  final_review: 600,
};

/** 아직 끝나지 않은 가장 최근 단계. 없으면 null. */
export function runningStep(steps: StepRow[]): StepRow | null {
  const running = steps.filter((s) => s.status === "running");
  return running.length === 0 ? null : running.reduce((a, b) => (b.started_at > a.started_at ? b : a));
}

/** 단계 시작 뒤 경과 시간(초). 시각 단위는 epoch 초이며, 밀리초 값이 오면 보정한다. */
export function elapsedSecs(step: Pick<StepRow, "started_at">, nowMs = Date.now()): number {
  const started = step.started_at > 1e12 ? step.started_at : step.started_at * 1000;
  return Math.max(0, Math.round((nowMs - started) / 1000));
}

export function formatDuration(secs: number | null): string {
  if (secs == null) return "-";
  if (secs < 60) return `${secs}초`;
  const m = Math.floor(secs / 60);
  return m < 60 ? `${m}분 ${secs % 60}초` : `${Math.floor(m / 60)}시간 ${m % 60}분`;
}

export const STEP_OUTPUT_LIMIT = 4000;
export const boundOutput = (text: string | null | undefined, limit = STEP_OUTPUT_LIMIT): string => {
  if (!text) return "";
  return text.length > limit ? `${text.slice(0, limit)}\n… (${text.length - limit}자 생략)` : text;
};

// --- 목록 요약·주의 필요 판정 ---

const ATTENTION_RUN_STATES: readonly string[] = ["awaiting_plan_approval", "awaiting_merge_approval", "paused"];

/** 사용자가 손대야 진행되는 실행인가 — 계획·머지 승인 대기, 일시정지(질문·에스컬레이션 포함), 에스컬레이션 티켓. */
export const needsAttention = (run: Pick<RunListItem, "state"> & Partial<Pick<RunListItem, "tickets_escalated">>): boolean =>
  ATTENTION_RUN_STATES.includes(run.state) || (!TERMINAL_RUN_STATES.includes(run.state) && (run.tickets_escalated ?? 0) > 0);

export const attentionCount = (runs: Parameters<typeof needsAttention>[0][]): number => runs.filter(needsAttention).length;

/** 종결되지 않았고 사용자 조치도 필요 없는, 자동으로 도는 실행. */
export const isAutoRunning = (run: Parameters<typeof needsAttention>[0]): boolean =>
  !TERMINAL_RUN_STATES.includes(run.state) && !needsAttention(run);

export interface RunGroups<T> {
  attention: T[];
  active: T[];
  finished: T[];
}

/** 조치 필요 / 진행 중 / 종료로 가른다. 각 묶음은 최근 갱신순. */
export function groupRuns<T extends Parameters<typeof needsAttention>[0] & Pick<RunRow, "updated_at">>(runs: T[]): RunGroups<T> {
  const out: RunGroups<T> = { attention: [], active: [], finished: [] };
  for (const r of [...runs].sort((a, b) => b.updated_at - a.updated_at)) {
    if (TERMINAL_RUN_STATES.includes(r.state)) out.finished.push(r);
    else if (needsAttention(r)) out.attention.push(r);
    else out.active.push(r);
  }
  return out;
}

/** 경로의 마지막 구성요소. */
export const repoBasename = (repo: string): string => repo.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || repo;

/** 통합 진행률(0–100). 티켓이 없으면 0. */
export const integratedPercent = (total: number, integrated: number): number =>
  total <= 0 ? 0 : Math.max(0, Math.min(100, Math.round((integrated / total) * 100)));

/** 상대 시각. 시각 단위는 epoch 초이며 밀리초 값이 오면 보정한다. */
export function relativeTime(at: number, nowMs = Date.now()): string {
  const ms = at > 1e12 ? at : at * 1000;
  const secs = Math.max(0, Math.round((nowMs - ms) / 1000));
  if (secs < 60) return "방금";
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m}분 전`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}시간 전`;
  return `${Math.floor(h / 24)}일 전`;
}

/** 지금 사용자가 할 일 한 줄. */
export function nextActionHint(run: Pick<RunRow, "state" | "paused_from" | "clarification_json">, escalated: number): string {
  if (run.state === "awaiting_plan_approval") return "계획 승인 필요";
  if (run.state === "awaiting_merge_approval") return "머지 승인 필요";
  if (run.state === "paused") {
    if (canAnswerClarification(run)) return "분할 전 질문에 답변 필요";
    if (escalated > 0) return `티켓 ${escalated}건 에스컬레이션 확인 필요`;
    return "일시정지 — 사유 확인 후 해제 필요";
  }
  if (escalated > 0 && !TERMINAL_RUN_STATES.includes(run.state)) return `티켓 ${escalated}건 에스컬레이션 확인 필요`;
  if (run.state === "done") return "완료 — 조치 불필요";
  if (run.state === "cancelled") return "취소됨 — 조치 불필요";
  if (run.state === "failed") return "실패 — 오류를 확인하세요";
  return "자동 진행 중 — 조치 불필요";
}

/** `baseline` 단계 출력 — 기준선(base 브랜치)에서 이미 실패하는 검사. */
export interface BaselineResult {
  failed_commands: string[];
  summary: string;
}

export function parseBaseline(output: string | null | undefined): BaselineResult | null {
  const raw = parseJson<Partial<BaselineResult> | null>(output, null);
  if (!raw || typeof raw !== "object") return null;
  return {
    failed_commands: Array.isArray(raw.failed_commands) ? raw.failed_commands.filter((c): c is string => typeof c === "string") : [],
    summary: typeof raw.summary === "string" ? raw.summary : "",
  };
}

/** 가장 최근 baseline 단계의 실패 명령 결과. 없거나 실패 명령이 비어 있으면 null. */
export function baselineFailure(steps: StepRow[]): BaselineResult | null {
  const last = steps.filter((s) => s.kind === "baseline" && s.output).reduce<StepRow | null>((a, b) => (!a || b.started_at >= a.started_at ? b : a), null);
  const parsed = last ? parseBaseline(last.output) : null;
  return parsed && parsed.failed_commands.length > 0 ? parsed : null;
}
