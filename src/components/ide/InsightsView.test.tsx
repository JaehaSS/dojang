// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { InsightsView } from "./InsightsView";
import { resetExperimentalFeaturesForTest } from "../../lib/experimental-features";
import type { SignalCard } from "../../lib/ipc";

vi.mock("../../lib/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/ipc")>();
  return { ...actual, insightCards: vi.fn(), lessonThemes: vi.fn() };
});
import { insightCards, lessonThemes } from "../../lib/ipc";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// jsdom에는 IntersectionObserver가 없다 — 스크롤 스파이(§ 섹션 관찰)가 이를 필요로 하므로
// 렌더가 깨지지 않도록 아무 것도 하지 않는 스텁을 심어 둔다.
class NoopIntersectionObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as { IntersectionObserver?: unknown }).IntersectionObserver = NoopIntersectionObserver;

// 근거 자료를 펼친 뒤 스크롤하는 effect가 requestAnimationFrame으로 한 프레임 늦춘다(§4).
// 실제 프레임을 기다리지 않도록 동기로 즉시 실행하는 스텁을 심는다.
(globalThis as { requestAnimationFrame?: (cb: FrameRequestCallback) => number }).requestAnimationFrame = (
  cb,
) => {
  cb(0);
  return 0;
};

const card = (overrides: Partial<SignalCard> = {}): SignalCard => ({
  key: "S1:7d:2026-09-21",
  signal: "S1",
  sentence: "입력 1회 이하로 머지 없이 닫은 작업이 8건(80%)이다. 직전 구간 2건",
  value: 8,
  baseline: 2,
  sample: 10,
  evidence: { kind: "Tasks", ids: [1, 2, 3, 4, 5, 6, 7, 8] },
  repo: null,
  ...overrides,
});

beforeEach(() => {
  resetExperimentalFeaturesForTest(false);
  vi.mocked(insightCards).mockReset();
  vi.mocked(insightCards).mockResolvedValue([]);
  vi.mocked(lessonThemes).mockReset();
  vi.mocked(lessonThemes).mockResolvedValue([]);
  localStorage.clear();
});

describe("InsightsView — 섹션 구성", () => {
  it("점프 칩을 발견 → 교훈 주제 → 근거 자료 순서로 싣는다", () => {
    const html = renderToStaticMarkup(<InsightsView />);

    // 탭이 아니라 단일 스크롤 리포트의 섹션이다(ADR 0033) — 점프 칩과 섹션이 함께 있어야 한다.
    const order = ["발견", "교훈 주제", "근거 자료"].map((label) => html.indexOf(`>${label}<`));
    expect(order.every((i) => i >= 0), "세 라벨이 모두 있어야 한다").toBe(true);
    expect(order).toEqual([...order].sort((a, b) => a - b));
    expect(html).toContain('id="discovery"');
    expect(html).toContain('id="lessons"');
    expect(html).toContain('id="evidence"');
  });

  it("근거 자료는 기본 접혀 있어 지출 · 작업 · 방식 패널을 그리지 않는다", () => {
    const html = renderToStaticMarkup(<InsightsView />);

    expect(html).toContain('aria-expanded="false"');
    expect(html).not.toContain('id="cost"');
    expect(html).not.toContain('id="tasks"');
    expect(html).not.toContain('id="how"');
  });

  it("SummaryLanes(요약 질문 레인)는 더 이상 렌더되지 않는다 — 발견 섹션이 대체한다(O4)", () => {
    const html = renderToStaticMarkup(<InsightsView />);

    for (const question of ["얼마나 썼나?", "무엇을 했나?", "어떻게 일했나?"]) {
      expect(html).not.toContain(question);
    }
    expect(html).not.toContain('id="summary"');
  });
});

describe("InsightsView — 근거 자료 펼침·접힘", () => {
  let el: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    el = document.createElement("div");
    document.body.append(el);
    root = createRoot(el);
  });

  afterEach(() => {
    act(() => root.unmount());
    el.remove();
  });

  it("토글을 누르면 지출 · 작업 · 방식 패널이 펼쳐진다", async () => {
    await act(async () => {
      root.render(<InsightsView />);
    });
    await act(async () => {
      await Promise.resolve();
    });

    const toggle = el.querySelector<HTMLButtonElement>('[data-testid="evidence-toggle"]');
    expect(toggle?.getAttribute("aria-expanded")).toBe("false");
    expect(el.querySelector("#cost")).toBeNull();

    await act(async () => {
      toggle?.click();
    });

    expect(toggle?.getAttribute("aria-expanded")).toBe("true");
    expect(el.querySelector("#cost")).not.toBeNull();
    expect(el.querySelector("#tasks")).not.toBeNull();
    expect(el.querySelector("#how")).not.toBeNull();
  });

  it("근거 자료가 접혀 있으면 고급 기능이 켜져 있어도 AX 결과를 그리지 않는다", async () => {
    resetExperimentalFeaturesForTest(true);
    await act(async () => {
      root.render(<InsightsView />);
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(el.textContent).not.toContain("AX 결과");
  });

  it("근거 자료를 펼치고 고급 기능이 켜져 있으면 AX 결과를 접어서 보여준다", async () => {
    resetExperimentalFeaturesForTest(true);
    await act(async () => {
      root.render(<InsightsView />);
    });
    await act(async () => {
      await Promise.resolve();
    });
    const toggle = el.querySelector<HTMLButtonElement>('[data-testid="evidence-toggle"]');

    await act(async () => {
      toggle?.click();
    });

    expect(el.querySelector("details")).not.toBeNull();
    expect(el.textContent).toContain("AX 결과");
  });

  it("발견 카드의 지출 근거 열기는 근거 자료를 펼치고 지출 섹션으로 스크롤한다", async () => {
    const scrollIntoView = vi.fn();
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: scrollIntoView,
    });

    vi.mocked(insightCards).mockResolvedValue([
      card({ signal: "S6", key: "S6:7d:2026-09-21", evidence: { kind: "Spend", project: "repo-a" } }),
    ]);
    await act(async () => {
      root.render(<InsightsView />);
    });
    await act(async () => {
      await Promise.resolve();
    });

    const openEvidence = [...el.querySelectorAll("button")].find((b) => b.textContent === "근거 열기");
    expect(openEvidence, "발견 카드에 근거 열기 버튼이 있어야 한다").toBeTruthy();

    await act(async () => {
      openEvidence?.click();
    });

    const toggle = el.querySelector('[data-testid="evidence-toggle"]');
    expect(toggle?.getAttribute("aria-expanded")).toBe("true");
    expect(scrollIntoView).toHaveBeenCalled();

    delete (HTMLElement.prototype as { scrollIntoView?: unknown }).scrollIntoView;
  });
});

describe("InsightsView — 발견 공간 신호 카드", () => {
  let el: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    el = document.createElement("div");
    document.body.append(el);
    root = createRoot(el);
  });

  afterEach(() => {
    act(() => root.unmount());
    el.remove();
  });

  it("카드가 오면 발견 섹션에 문장을 보여준다", async () => {
    vi.mocked(insightCards).mockResolvedValue([card()]);
    await act(async () => {
      root.render(<InsightsView />);
    });
    // 비동기 useEffect의 then이 마이크로태스크 큐를 한 바퀴 돌아야 반영된다.
    await act(async () => {
      await Promise.resolve();
    });

    expect(el.querySelector('[data-testid="discovery-cards"]')).not.toBeNull();
    expect(el.textContent).toContain("입력 1회 이하로 머지 없이 닫은 작업이 8건");
  });

  it("카드가 없으면 발견 섹션을 아예 그리지 않는다", async () => {
    vi.mocked(insightCards).mockResolvedValue([]);
    await act(async () => {
      root.render(<InsightsView />);
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(el.querySelector('[data-testid="discovery-cards"]')).toBeNull();
  });

  it("호출이 실패해도 발견 섹션은 조용히 사라질 뿐 다른 패널을 막지 않는다", async () => {
    vi.mocked(insightCards).mockRejectedValue(new Error("boom"));
    await act(async () => {
      root.render(<InsightsView />);
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(el.querySelector('[data-testid="discovery-cards"]')).toBeNull();
    expect(el.textContent).toContain("근거 자료"); // 다른 섹션 헤더는 여전히 살아 있다
  });
});

describe("InsightsView — 교훈 주제", () => {
  let el: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    el = document.createElement("div");
    document.body.append(el);
    root = createRoot(el);
  });

  afterEach(() => {
    act(() => root.unmount());
    el.remove();
  });

  it("투영이 있는 저장소가 오면 교훈 주제 섹션을 보여준다", async () => {
    vi.mocked(lessonThemes).mockResolvedValue([
      {
        repo: "/Users/x/work/praxis",
        entries_in_window: 12,
        subjects: [
          { subject: "runner/db", lessons: 5, prev_lessons: 2, numbers: [1, 2, 3] },
        ],
        abandoned: [],
      },
    ]);
    await act(async () => {
      root.render(<InsightsView />);
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(el.querySelector('[data-testid="lesson-themes"]')).not.toBeNull();
    expect(el.textContent).toContain("runner/db");
  });

  it("투영이 있는 저장소가 없으면 교훈 주제 섹션을 아예 그리지 않는다", async () => {
    vi.mocked(lessonThemes).mockResolvedValue([]);
    await act(async () => {
      root.render(<InsightsView />);
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(el.querySelector('[data-testid="lesson-themes"]')).toBeNull();
  });

  it("호출이 실패해도 교훈 주제 섹션은 조용히 사라질 뿐 다른 패널을 막지 않는다", async () => {
    vi.mocked(lessonThemes).mockRejectedValue(new Error("boom"));
    await act(async () => {
      root.render(<InsightsView />);
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(el.querySelector('[data-testid="lesson-themes"]')).toBeNull();
    expect(el.textContent).toContain("근거 자료");
  });
});
