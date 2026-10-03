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
export const pipelineList = (repo?: string) => invoke<RunRow[]>("pipeline_list", { repo });
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
