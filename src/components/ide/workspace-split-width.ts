/** 코드 열 최소 가독 폭 — JetBrains Mono 14px ≈ 8.4px/자 → 약 51칼럼 + 줄번호 거터. */
export const MIN_CODE_WIDTH = 480;
/** 세션 열 최소 폭 — 마크다운 본문과 코드블록이 접히지 않는 하한. */
export const MIN_SESSION_WIDTH = 360;
/** 토론 한 면의 최소 폭 — 면이 여럿이므로 세션 열의 하한이 달라진다(설계 0020 §5). */
export const MIN_DEBATE_PANE_WIDTH = 320;
/** 토론 중 세션 열의 최소 폭 — 면을 접지 않는다. 그 아래에서는 가로 스크롤한다. */
export const debateSessionWidth = (seats: number): number => MIN_DEBATE_PANE_WIDTH * seats;
/** 2자 토론의 세션 열 최소 폭. */
export const MIN_DEBATE_SESSION_WIDTH = debateSessionWidth(2);

/** 2열을 포기하는 임계. */
export const SPLIT_EXIT = MIN_CODE_WIDTH + MIN_SESSION_WIDTH;
/**
 * 2열로 돌아오는 임계. EXIT보다 높게 잡는다 — 같은 값이면 경계에서 창을 조절할 때
 * 모드가 깜빡인다(설계 0044 §5).
 */
export const SPLIT_ENTER = 900;

const DEFAULT_SESSION_WIDTH = 480;
const KEY = "praxis-workspace-split";

export type SplitMode = "split" | "tabs";

export interface SplitState {
  width: number;
}

/**
 * 폭 변화에 따른 모드 전이. 두 임계 사이 구간에서는 직전 모드를 그대로 둔다 —
 * 이 갭이 히스테리시스이고, 없으면 경계에서 레이아웃이 진동한다.
 */
export function nextSplitMode(current: SplitMode, available: number): SplitMode {
  if (current === "split") return available < SPLIT_EXIT ? "tabs" : "split";
  return available >= SPLIT_ENTER ? "split" : "tabs";
}

/** 세션 열 폭을 두 최소치 사이로 가둔다. */
export function clampSplit(sessionWidth: number, available: number): number {
  return Math.min(Math.max(sessionWidth, MIN_SESSION_WIDTH), available - MIN_CODE_WIDTH);
}

export function readSplit(storage: Pick<Storage, "getItem">): SplitState {
  const raw = storage.getItem(KEY);
  if (raw == null) return { width: DEFAULT_SESSION_WIDTH };
  try {
    const parsed = JSON.parse(raw) as Partial<SplitState>;
    return {
      width: typeof parsed.width === "number" ? parsed.width : DEFAULT_SESSION_WIDTH,
    };
  } catch {
    // 저장 값이 깨져도 워크스페이스는 열려야 한다 — 조용히 기본값으로 돌아간다.
    return { width: DEFAULT_SESSION_WIDTH };
  }
}

export function writeSplit(storage: Pick<Storage, "setItem">, value: SplitState): void {
  storage.setItem(KEY, JSON.stringify(value));
}
