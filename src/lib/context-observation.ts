export interface ContextObservation {
  contextTokens: number;
  contextWindow: number | null;
  observedAt: number | null;
  source: string;
}

export interface ContextObservationState {
  observation: ContextObservation | null;
  awaitingInvocation: boolean;
}

export const EMPTY_CONTEXT_OBSERVATION: ContextObservationState = {
  observation: null,
  awaitingInvocation: false,
};

export const MODEL_CHANGE_INVALIDATION: ContextObservationState = {
  observation: null,
  awaitingInvocation: true,
};

interface ContextObservationEvent {
  kind: string;
  context_tokens?: unknown;
  context_window?: unknown;
  observed_at?: unknown;
  source?: unknown;
  valid?: unknown;
  speaker?: unknown;
  parent_id?: unknown;
}

const isSafeNonNegative = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;

const isSafePositive = (value: unknown): value is number =>
  isSafeNonNegative(value) && value > 0;

const isSafeUnixSeconds = (value: unknown): value is number =>
  isSafeNonNegative(value) && value <= 8_640_000_000_000;

const isClaude = (agent: string | null | undefined): boolean => agent?.trim() === "claude";

const normalizeObservation = (
  event: ContextObservationEvent,
  agent: string | null | undefined,
): ContextObservation | null => {
  if (!isSafePositive(event.context_tokens)) return null;
  if ((event.source === "codex_session" || event.source === "app_server") && event.valid === true) {
    if (!isSafePositive(event.context_window) || !isSafeUnixSeconds(event.observed_at)) return null;
    // App Server formerly emitted milliseconds; those observations were never accepted.
    // Keep old values unknown rather than display a timestamp thousands of years ahead.
    if (event.source === "app_server" && event.observed_at > 253_402_300_799) return null;
    return {
      contextTokens: event.context_tokens,
      contextWindow: event.context_window,
      observedAt: event.observed_at,
      source: event.source,
    };
  }
  // agy는 응답 step마다 그 호출의 입력 토큰을 준다. 모델별 윈도 크기는 싣지 않는다.
  if ((event.source === "claude_message" || event.source === "agy_step") && event.valid === true) {
    if (!isSafeUnixSeconds(event.observed_at)) return null;
    return {
      contextTokens: event.context_tokens,
      contextWindow: null,
      observedAt: event.observed_at,
      source: event.source,
    };
  }
  if (event.source == null && event.valid == null && isClaude(agent))
    return {
      contextTokens: event.context_tokens,
      contextWindow: null,
      observedAt: null,
      source: "claude_legacy",
    };
  return null;
};

/** 이벤트 순서대로 마지막 관측 하나만 복원한다. 다른 이벤트의 필드는 섞지 않는다. */
export function foldContextObservation(
  prev: ContextObservationState,
  events: readonly ContextObservationEvent[],
  agent: string | null | undefined,
): ContextObservationState {
  return events.reduce<ContextObservationState>((state, event) => {
    if (event.parent_id != null || (event.speaker != null && event.speaker !== "left")) return state;
    if (event.kind === "model_snapshot" && event.source === "invocation")
      return { observation: state.observation, awaitingInvocation: false };
    if (event.kind === "context_cleared") return EMPTY_CONTEXT_OBSERVATION;
    if (event.kind !== "context_usage") return state;
    if (state.awaitingInvocation) return state;
    const legacyClaude = event.source == null && event.valid == null && isClaude(agent);
    if (event.valid !== true && !legacyClaude)
      return event.source === "model_change" ? MODEL_CHANGE_INVALIDATION : EMPTY_CONTEXT_OBSERVATION;
    return { observation: normalizeObservation(event, agent), awaitingInvocation: false };
  }, prev);
}
