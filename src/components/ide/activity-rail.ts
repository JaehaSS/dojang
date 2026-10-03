import { MIN_SESSION_WIDTH } from "./workspace-split-width";

/** 플로팅 작업정보 채널의 고정 폭 — 조절 핸들은 두지 않는다(설계 0018 D5). */
export const FLOATING_CHANNEL_WIDTH = 300;

/**
 * 채널이 떠 있는 동안 세션 열이 오른쪽에 비워 두는 폭 — 채널 300 + 바깥 여백 16 + 콘텐츠와의 간격 16.
 *
 * 겹치게 두면 대화의 오른쪽 끝이 카드 아래로 깔린다. 세션과 채널은 **다른 영역**이라는 것이
 * 이 값의 뜻이고, 예약은 채널이 실제로 떠 있을 때만 건다.
 */
export const FLOATING_CHANNEL_RESERVED = FLOATING_CHANNEL_WIDTH + 32;

/**
 * 채널이 뜬 뒤에도 세션이 자기 최소 폭을 지킬 수 있는 중앙 잔여 폭.
 *
 * 창 폭으로 재면 안 된다 — 사이드바(240)와 파일 트리(208)를 빼고 나면 같은 1000px 창이라도
 * 세션에 남는 폭이 배 넘게 차이 난다. 겹쳐 띄우던 시절에는 세션이 줄지 않아 창 폭 판정으로도
 * 무해했지만, 자리를 예약하는 순간 그 차이가 그대로 세션 폭에서 빠진다.
 */
export const FLOATING_CHANNEL_MIN_CENTER = MIN_SESSION_WIDTH + FLOATING_CHANNEL_RESERVED;

/**
 * 같은 판정의 임계를 세션 열의 최소 폭에서 계산한다 — 토론이면 면이 둘이라 하한이 640이고,
 * 임계도 `640 + 332 = 972`로 올라간다(설계 0020 §5).
 *
 * 소비자가 셋(placement · 핸들 · 자동 코드 열)이라 식이 한 곳에서 나와야 한다. 하나라도
 * 다른 수를 쓰면 "핸들은 보이는데 눌러도 안 뜨는" 상태가 재발한다(ADR 0115).
 */
export const floatingChannelMinCenter = (sessionMin: number = MIN_SESSION_WIDTH): number =>
  sessionMin + FLOATING_CHANNEL_RESERVED;

/**
 * 작업정보가 사는 자리.
 *
 * - `floating` — 세션 열 우상단에 떠 있고, 세션은 그만큼 오른쪽을 비운다.
 * - `code` — 코드 열의 고정 `작업정보` 탭. 파일·Diff를 부른 동안의 거처다.
 * - `hidden` — 사용자가 접었다. 게이지로 다시 부른다.
 */
export type ChannelPlacement = "floating" | "code" | "hidden";

/**
 * 작업정보의 거처를 정한다. **어디에 있든 한 곳뿐이다** — 같은 내용을 두 자리에 두지 않는다(ADR 0066).
 *
 * 자리를 가르는 것은 코드 열이다. 코드 열이 열려 있으면 세션은 이미 폭을 반으로 나눠 쓰는
 * 중이라 채널이 뜰 여백이 없고, 그때 겹쳐 띄우면 정작 부른 파일·Diff를 가린다. 그래서
 * 코드 열이 열리는 순간 채널은 그 안의 탭으로 들어간다.
 *
 * 자리가 모자라 `hidden`인 것도 같은 이유다. 여기서는 코드 열 탭이 유일한 진입로가 된다 —
 * 게이지가 그리로 보낸다.
 */
export function channelPlacement(input: {
  /** 중앙(세션 + 코드)에 남은 폭. 사이드바·파일 트리를 이미 뺀 값이다. */
  centerWidth: number;
  pinned: boolean;
  codeOpen: boolean;
  /** 세션 열이 지켜야 하는 최소 폭. 토론이면 `debateSessionWidth(자리 수)`가 온다. */
  sessionMin?: number;
}): ChannelPlacement {
  if (input.codeOpen) return "code";
  if (!input.pinned) return "hidden";
  return input.centerWidth >= floatingChannelMinCenter(input.sessionMin) ? "floating" : "hidden";
}

/** 세션 헤더의 좌우 패딩(`px-3`). 액션 줄이 물러설 때 이미 비어 있는 만큼은 빼고 센다. */
const HEADER_PADDING_X = 12;

/**
 * 채널이 떠 있는 동안 헤더 액션 줄이 오른쪽에서 물러서는 폭.
 *
 * 헤더는 세션 열이 아니라 전폭이라 예약이 걸리지 않는다. 그대로 두면 파일 트리·파일·Diff·
 * 프리뷰·터미널 손잡이가 창 맨 끝, **바로 아래 채널 카드의 폭 안에** 붙어 선다. 겹치지는
 * 않지만 채널의 머리처럼 읽혀 세션의 손잡이라는 것이 보이지 않는다.
 *
 * 그래서 액션 줄의 오른쪽 끝을 세션 콘텐츠 박스의 오른쪽 끝에 맞춘다 — 채널이 자리를
 * 예약하면 그 위의 손잡이도 같은 선까지만 온다. 예약(332)에서 헤더가 이미 가진 패딩(12)을
 * 뺀 값이고, 채널이 뜨지 않으면 물러서지 않는다.
 */
export const FLOATING_CHANNEL_HEADER_INSET = FLOATING_CHANNEL_RESERVED - HEADER_PADDING_X;

/**
 * 접힌 채널의 재열기 핸들이 보이는 조건 — **다시 뜰 자리가 있을 때만**이다.
 *
 * 사용자가 접었어도(`!pinned`) 코드 열이 열려 있거나 중앙이 좁으면 핸들을 눌러 봐야
 * 채널이 뜨지 못한다(placement가 그대로 hidden/code). 눌러도 못 여는 문은 문이 아니라
 * 버그처럼 보이므로, "핀을 켜면 실제로 플로팅이 되는" 조건과 정확히 같은 식을 쓴다.
 */
export function channelHandleVisible(input: {
  centerWidth: number;
  pinned: boolean;
  codeOpen: boolean;
  sessionMin?: number;
}): boolean {
  return (
    !input.pinned &&
    channelPlacement({ ...input, pinned: true }) === "floating"
  );
}

/**
 * 헤더 소환 칩(파일·Diff·프리뷰·터미널)이 라벨을 보여 주는 중앙 잔여 하한.
 *
 * 라벨 4개는 아이콘 온리 대비 행을 약 190px 늘린다. 채널이 떠서 액션 줄이 320px
 * 물러선 상태에서도 제목·배지와 겹치지 않는 폭이 이 값이다. 임계 아래에서는
 * 아이콘으로 접는다 — 레이아웃 모드가 아니라 행 폭만 바뀌므로, 분할 폴백(설계 0044)과
 * 달리 히스테리시스는 두지 않는다.
 */
export const PANE_LABELS_MIN_CENTER = 1080;
