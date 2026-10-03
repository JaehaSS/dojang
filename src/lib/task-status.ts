import type { Task } from "./ipc";

// 작업 상태 → 표시 라벨·색의 단일 소스. (모바일은 자체 Tone 체계를 쓴다 — src/mobile/status.ts)
//
// 이전에는 dotColor 맵이 SessionTaskProject/HomeView/EnsembleView에 각각 복제돼 있어,
// 상태가 하나 늘 때마다 세 곳이 서로 어긋났다.

/** `AwaitingReview`가 왜 대기 중인지 — DB `tasks.awaiting_kind` 값. */
export const AWAITING_QUESTION = "question";

/** `blocked_reason`의 인증 차단 접두사 — DB `db::blocked::AUTH_PREFIX`와 같은 문자열. */
export const BLOCKED_AUTH_PREFIX = "auth:";

/** 인증이 풀려 시작하지 못하는 작업인가. 큐에 남아 있을 뿐 실패가 아니다. */
export function isAuthBlocked(task: Pick<Task, "blocked_reason">): boolean {
  return (task.blocked_reason ?? "").startsWith(BLOCKED_AUTH_PREFIX);
}

/** 차단을 일으킨 벤더 — 로그인 버튼이 어느 CLI를 열지 결정한다. */
export function blockedVendor(task: Pick<Task, "blocked_reason">): string | null {
  if (!isAuthBlocked(task)) return null;
  return (task.blocked_reason as string).slice(BLOCKED_AUTH_PREFIX.length) || null;
}

export type StatusTone = "running" | "awaiting" | "question" | "done" | "failed" | "muted";

/**
 * 검토 대기는 두 갈래다. 에이전트가 **답을 기다리는 것**과 **결과 승인을 기다리는 것**은
 * 사용자가 취할 행동이 다르다(대답 vs 검토) — 색과 라벨을 갈라 한눈에 구분한다.
 * 상태 머신에서는 둘 다 AwaitingReview로, 승인·폐기·재개 경로는 동일하게 열려 있다.
 */
export function statusTone(
  state: string,
  awaitingKind?: string | null,
  blockedReason?: string | null,
  pendingQuestion = false,
): StatusTone {
  // 차단된 큐 작업은 실패가 아니라 **사용자 조치 대기**다 — 빨강으로 칠하면 이미 죽은 줄 안다.
  if (blockedReason?.startsWith(BLOCKED_AUTH_PREFIX)) return "awaiting";
  if (awaitingAnswer(state, pendingQuestion)) return "question";
  if (state === "Running" || state === "Starting") return "running";
  if (state === "AwaitingReview") {
    return awaitingKind === AWAITING_QUESTION ? "question" : "awaiting";
  }
  if (state === "PendingApproval" || state === "Finalizing") return "awaiting";
  if (state === "Done") return "done";
  if (state === "Failed" || state === "Discarded") return "failed";
  return "muted";
}

/**
 * 구조화 질문(MCP `ask_user`)은 실행이 `Running`인 채로 열린다 — 상태만 보면 실행 중이지만 사용자가
 * 답해야 진행된다. 그 조건은 `tasks` 행이 아니라 attention 투영(`reason: "question"`)이 관측하므로
 * 호출부가 관측 결과를 `pendingQuestion`으로 넘긴다. 실행 중이 아닌 상태의 잔상은 무시한다 — 턴이
 * 끝났으면 `awaiting_kind`가 같은 뜻을 이미 싣고 있다.
 */
const awaitingAnswer = (state: string, pendingQuestion: boolean): boolean =>
  pendingQuestion && (state === "Running" || state === "Starting");

/** 톤 → CSS 변수. 인라인 style(점 배경)용 — Tailwind 클래스가 필요하면 `toneTextClass`. */
const TONE_VAR: Record<StatusTone, string> = {
  running: "var(--c-running)",
  awaiting: "var(--c-awaiting)",
  question: "var(--c-question)",
  done: "var(--c-done)",
  failed: "var(--c-failed)",
  muted: "var(--c-text-muted)",
};

const TONE_TEXT_CLASS: Record<StatusTone, string> = {
  running: "text-status-running",
  awaiting: "text-status-awaiting",
  question: "text-status-question",
  done: "text-status-done",
  failed: "text-status-failed",
  muted: "text-text-muted",
};

const LABEL: Record<string, string> = {
  Created: "생성됨",
  Queued: "대기",
  Starting: "시작 중",
  Running: "실행 중…",
  Finalizing: "마무리 중",
  PendingApproval: "실행 승인 대기",
  Done: "완료",
  Failed: "실패",
  Discarded: "버림",
};

export function taskTone(
  task: Pick<Task, "state" | "awaiting_kind" | "blocked_reason">,
  pendingQuestion = false,
): StatusTone {
  return statusTone(task.state, task.awaiting_kind, task.blocked_reason, pendingQuestion);
}

export function taskDotColor(
  task: Pick<Task, "state" | "awaiting_kind" | "blocked_reason">,
  pendingQuestion = false,
): string {
  return TONE_VAR[taskTone(task, pendingQuestion)];
}

export function taskTextClass(
  task: Pick<Task, "state" | "awaiting_kind" | "blocked_reason">,
  pendingQuestion = false,
): string {
  return TONE_TEXT_CLASS[taskTone(task, pendingQuestion)];
}

/** 알 수 없는 상태는 원문을 그대로 보여준다 — 추측한 라벨보다 낫다. */
export function taskStatusLabel(
  task: Pick<Task, "state" | "awaiting_kind" | "blocked_reason">,
  pendingQuestion = false,
): string {
  // 색만으로 구분하지 않는다(DESIGN.md Do #2) — 차단은 라벨로도 드러나야 한다.
  if (isAuthBlocked(task)) return "로그인 필요";
  if (awaitingAnswer(task.state, pendingQuestion)) return "답변 대기";
  if (task.state === "AwaitingReview") {
    return task.awaiting_kind === AWAITING_QUESTION ? "답변 대기" : "검토 대기";
  }
  return LABEL[task.state] ?? task.state;
}

/**
 * 좁은 목록(사이드바 카드)용 짧은 라벨. 대화 턴이 끝나면 대부분 검토 대기로 수렴해 같은 문구가
 * 줄줄이 반복되므로 "대기"를 뗀다 — 문구를 아예 지우면 앰버 점을 완료로 오해하는 문제가 재발하고
 * 색 단독 인코딩 금지(DESIGN.md Do #2)에도 어긋나지만, 줄이는 것은 그 계약을 깨지 않는다.
 */
export function taskStatusLabelShort(
  task: Pick<Task, "state" | "awaiting_kind" | "blocked_reason">,
  pendingQuestion = false,
): string {
  if (isAuthBlocked(task)) return "로그인";
  if (awaitingAnswer(task.state, pendingQuestion)) return "답변";
  if (task.state === "AwaitingReview") {
    return task.awaiting_kind === AWAITING_QUESTION ? "답변" : "검토";
  }
  return LABEL[task.state] ?? task.state;
}

/** 아직 한 번도 실행된 적 없는 상태 — `updated_at`을 "끝난 시각"으로 읽으면 거짓이 된다. */
const NEVER_RAN = new Set(["Created", "Queued", "PendingApproval"]);
/** 실행이 아직 끝나지 않은 상태 — `updated_at`은 이번 실행이 **시작된** 시각이다. */
const IN_FLIGHT = new Set(["Running", "Starting", "Finalizing"]);

/**
 * `updated_at`이 가리키는 시점의 이름. 같은 필드라도 상태에 따라 뜻이 달라진다 —
 * 실행 중이면 시작 시각, 턴이 끝났으면 멈춘 시각. null이면 실행 기록이 없어 보여줄 시각이 없다.
 */
export function lastRunLabel(task: Pick<Task, "state">): string | null {
  if (NEVER_RAN.has(task.state)) return null;
  return IN_FLIGHT.has(task.state) ? "실행 시작" : "마지막 종료";
}

/**
 * 에이전트가 지금 붙어 있는 작업인가 — 대기열(Created/Queued)과 승인 대기는 아니다.
 * `lastRunLabel`과 같은 집합을 본다. 판정을 복제하지 않으려고 여기서 함께 내보낸다.
 */
export function isInFlight(task: Pick<Task, "state">): boolean {
  return IN_FLIGHT.has(task.state);
}

/** 종결 상태 — 이 셋에서는 대화가 더 진행되지 않는다. `src-tauri`의 `tstate::is_terminal`과 같은 집합
 *  (프런트에는 그 함수가 없어 별도 유지 — 상태 이름 문자열 자체가 계약이라 어긋나면 즉시 드러난다). */
const TERMINAL_STATES = new Set(["Done", "Failed", "Discarded"]);
export function isTerminalState(state: string): boolean {
  return TERMINAL_STATES.has(state);
}

/** 사용자의 응답·결정을 기다리는 작업인가 — 배지 카운트와 정렬 우선순위의 기준. */
export function needsUser(
  task: Pick<Task, "state" | "awaiting_kind" | "blocked_reason">,
): boolean {
  // 차단된 작업도 사용자가 움직여야 풀린다 — 배지에 안 잡히면 큐에서 조용히 잊힌다.
  if (isAuthBlocked(task)) return true;
  return task.state === "AwaitingReview" || task.state === "PendingApproval";
}
