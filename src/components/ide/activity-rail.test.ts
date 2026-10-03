import { describe, expect, it } from "vitest";
import {
  FLOATING_CHANNEL_HEADER_INSET,
  FLOATING_CHANNEL_MIN_CENTER,
  FLOATING_CHANNEL_RESERVED,
  FLOATING_CHANNEL_WIDTH,
  channelHandleVisible,
  channelPlacement,
  floatingChannelMinCenter,
} from "./activity-rail";
import { debateSessionWidth, MIN_DEBATE_SESSION_WIDTH } from "./workspace-split-width";

const at = (input: Partial<Parameters<typeof channelPlacement>[0]>) =>
  channelPlacement({ centerWidth: 1072, pinned: true, codeOpen: false, ...input });

describe("channelPlacement", () => {
  it("코드 열이 닫혀 있고 자리가 있으면 세션 위에 띄운다", () => {
    expect(at({})).toBe("floating");
    expect(at({ centerWidth: FLOATING_CHANNEL_MIN_CENTER })).toBe("floating");
  });

  it("코드 열이 열리면 그 안의 탭으로 옮긴다 — 파일·Diff를 가리지 않는다", () => {
    expect(at({ codeOpen: true })).toBe("code");
    // 자리가 없는 좁은 중앙에서도 마찬가지다. 거기서는 이 탭이 유일한 진입로다.
    expect(at({ codeOpen: true, centerWidth: 600 })).toBe("code");
  });

  it("코드 열이 열려 있으면 접어 둔 상태여도 탭은 남는다 — 고정 탭은 닫을 수 없다", () => {
    expect(at({ codeOpen: true, pinned: false })).toBe("code");
  });

  it("사용자가 접으면 폭과 무관하게 숨긴다 — 게이지로 다시 펼친다", () => {
    expect(at({ pinned: false })).toBe("hidden");
    expect(at({ pinned: false, centerWidth: 900 })).toBe("hidden");
  });

  it("예약하고 나면 세션이 최소 폭을 못 지키는 좁은 중앙에서는 띄우지 않는다", () => {
    expect(at({ centerWidth: FLOATING_CHANNEL_MIN_CENTER - 1 })).toBe("hidden");
    expect(at({ centerWidth: 600 })).toBe("hidden");
  });

  it("토론이면 면이 둘이라 임계가 972로 오른다 — 971은 자리가 없다", () => {
    expect(floatingChannelMinCenter(MIN_DEBATE_SESSION_WIDTH)).toBe(972);
    expect(at({ centerWidth: 972, sessionMin: MIN_DEBATE_SESSION_WIDTH })).toBe("floating");
    expect(at({ centerWidth: 971, sessionMin: MIN_DEBATE_SESSION_WIDTH })).toBe("hidden");
    // 같은 폭이 단일 세션에서는 자리가 있다 — 임계는 세션 열의 최소 폭에서 나온다.
    expect(at({ centerWidth: 971 })).toBe("floating");
  });

  it("3자 토론이면 면이 셋이라 임계가 1292로 오른다", () => {
    expect(floatingChannelMinCenter(debateSessionWidth(3))).toBe(1292);
    expect(at({ centerWidth: 1291, sessionMin: debateSessionWidth(3) })).toBe("hidden");
  });

  it("판정은 창 폭이 아니라 중앙 잔여 폭이다 — 사이드바·트리를 켜면 같은 창도 자리가 없다", () => {
    // 창 1280: 사이드바 축소(56) + 트리 없음 -> 중앙 약 1224. 자리가 있다.
    expect(at({ centerWidth: 1224 })).toBe("floating");
    // 같은 창에 사이드바 확장(240) + 트리(208) -> 중앙 832. 예약하면 세션이 500 남는다.
    expect(at({ centerWidth: 832 })).toBe("floating");
    // 창 1000에 같은 크롬 -> 중앙 552. 예약하면 세션이 220뿐이라 띄우지 않는다.
    expect(at({ centerWidth: 552 })).toBe("hidden");
  });
});

describe("channelHandleVisible", () => {
  const handle = (input: Partial<Parameters<typeof channelHandleVisible>[0]>) =>
    channelHandleVisible({ centerWidth: 1000, pinned: false, codeOpen: false, ...input });

  it("접어 둔 채널이 다시 뜰 자리가 있으면 손잡이를 낸다", () => {
    expect(handle({})).toBe(true);
    expect(handle({ centerWidth: FLOATING_CHANNEL_MIN_CENTER })).toBe(true);
  });

  it("이미 떠 있거나 뜰 예정이면 손잡이가 없다 — 같은 문을 두 번 만들지 않는다", () => {
    expect(handle({ pinned: true })).toBe(false);
    expect(handle({ pinned: true, centerWidth: 2000 })).toBe(false);
    // 핀이 켜져 있는데 자리가 없어 hidden인 경우도 손잡이의 몫이 아니다 — 게이지가 맡는다.
    expect(handle({ pinned: true, centerWidth: 600 })).toBe(false);
  });

  it("코드 열이 열려 있으면 거처가 그 안의 탭이라 손잡이를 내지 않는다", () => {
    expect(handle({ codeOpen: true })).toBe(false);
    expect(handle({ codeOpen: true, centerWidth: 2000 })).toBe(false);
  });

  it("눌러도 뜨지 못하는 좁은 중앙에서는 문을 만들지 않는다", () => {
    expect(handle({ centerWidth: FLOATING_CHANNEL_MIN_CENTER - 1 })).toBe(false);
    expect(handle({ centerWidth: 600 })).toBe(false);
  });

  it("손잡이가 보이는 조건은 핀을 켰을 때 실제로 뜨는 조건과 같다", () => {
    for (const centerWidth of [400, 600, 691, 692, 900, 971, 972, 1400]) {
      for (const codeOpen of [true, false]) {
        // 토론 임계를 넣어도 두 함수가 같은 식을 쓴다 — 어긋나면 눌러도 안 뜨는 손잡이가 남는다.
        for (const sessionMin of [undefined, MIN_DEBATE_SESSION_WIDTH]) {
          expect(channelHandleVisible({ centerWidth, pinned: false, codeOpen, sessionMin })).toBe(
            channelPlacement({ centerWidth, pinned: true, codeOpen, sessionMin }) === "floating",
          );
        }
      }
    }
  });
});

describe("FLOATING_CHANNEL_RESERVED", () => {
  it("채널 폭에 바깥 여백과 콘텐츠 간격을 더한 값이다 — 예약이 모자라면 대화가 카드 아래로 깔린다", () => {
    expect(FLOATING_CHANNEL_RESERVED).toBe(FLOATING_CHANNEL_WIDTH + 32);
  });

  it("헤더 인셋은 같은 예약에서 헤더가 이미 가진 패딩만 뺀 값이다 — 손잡이 줄이 세션 끝에 선다", () => {
    expect(FLOATING_CHANNEL_HEADER_INSET).toBe(FLOATING_CHANNEL_RESERVED - 12);
  });
});
