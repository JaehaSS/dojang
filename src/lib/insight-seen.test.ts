// @vitest-environment jsdom

import { beforeEach, describe, expect, it } from "vitest";
import { countUnseen, isSeen, markSeen } from "./insight-seen";

describe("insight-seen", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("본 적 없는 쌍은 보지 않은 것이다", () => {
    expect(isSeen("S1", null)).toBe(false);
  });

  it("markSeen 후에는 같은 (signal, repo) 쌍이 보인 것으로 남는다", () => {
    markSeen([{ signal: "S2", repo: "repo-a" }]);
    expect(isSeen("S2", "repo-a")).toBe(true);
  });

  it("같은 신호라도 저장소가 다르면 별개 쌍이다", () => {
    markSeen([{ signal: "S2", repo: "repo-a" }]);
    expect(isSeen("S2", "repo-b")).toBe(false);
  });

  it("저장소에 묶이지 않은 신호(null)와 빈 문자열 저장소는 같은 쌍이다", () => {
    // 백엔드 insight_dismissals(§6.4)와 같은 관례 — repo 없는 카드는 ''로 접는다.
    markSeen([{ signal: "S1", repo: null }]);
    expect(isSeen("S1", "")).toBe(true);
  });

  it("TTL 7일이 지나면 다시 보지 않은 것이 된다", () => {
    const now = Date.now();
    markSeen([{ signal: "S1", repo: null }], now);

    const sixDaysLater = now + 6 * 24 * 60 * 60 * 1000;
    const eightDaysLater = now + 8 * 24 * 60 * 60 * 1000;
    expect(isSeen("S1", null, sixDaysLater)).toBe(true);
    expect(isSeen("S1", null, eightDaysLater)).toBe(false);
  });

  it("countUnseen은 본 적 없거나 TTL이 지난 쌍만 센다", () => {
    markSeen([{ signal: "S1", repo: null }]);
    const cards = [
      { signal: "S1", repo: null },
      { signal: "S2", repo: "repo-a" },
      { signal: "L1", repo: "repo-b" },
    ];

    expect(countUnseen(cards)).toBe(2);
  });

  it("전부 본 카드면 0을 센다", () => {
    const cards = [
      { signal: "S1", repo: null },
      { signal: "S2", repo: "repo-a" },
    ];
    markSeen(cards);

    expect(countUnseen(cards)).toBe(0);
  });

  it("markSeen(빈 배열)은 기존 기록을 지우지 않는다", () => {
    markSeen([{ signal: "S1", repo: null }]);
    markSeen([]);

    expect(isSeen("S1", null)).toBe(true);
  });

  it("깨진 저장값은 없는 것으로 읽는다", () => {
    localStorage.setItem("praxis:insight-seen", "not-json");

    expect(isSeen("S1", null)).toBe(false);
    expect(countUnseen([{ signal: "S1", repo: null }])).toBe(1);
  });
});
