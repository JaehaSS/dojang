// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  taskActivity: vi.fn(),
  quizAvailability: vi.fn(),
  insightAvailability: vi.fn(),
  insightNext: vi.fn(),
}));
vi.mock("../../lib/ipc", () => mocks);
vi.mock("../../lib/ipc.ts", () => mocks);

import { QUIZ_GATE_POLL_MS } from "../../lib/use-quiz-gate";
import { ConversationView, type ConvoItem } from "./ConversationView";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const CARD = {
  key: "wiki:업무-일지/2026-09-14.md#그 외 작업",
  deck: "업무-일지",
  title: "그 외 작업",
  body: "Praxis(업무 도구): Tauri 앱 빌드.",
  source: "업무-일지/2026-09-14.md › 그 외 작업",
  tags: ["업무-일지"],
};

/** 임계값을 한참 넘긴 이 작업의 대기. `started_at`은 초 단위다. */
const longWait = (taskId: number) => [{ task_id: taskId, last_operation: null, last_event_at: 0, started_at: 0 }];

const items: ConvoItem[] = [{ role: "user", text: "빌드해줘" }];

let container: HTMLDivElement | null = null;
let root: Root | null = null;

const scroller = () => container?.firstElementChild?.firstElementChild as HTMLDivElement;
const card = () => container?.querySelector('[aria-label="대기 인사이트"]');
const busyStrip = () => [...(container?.querySelectorAll("span") ?? [])].find((el) => el.textContent === "에이전트 작업 중…");

async function render(props: { busy: boolean; waitTaskId?: number | null }) {
  await act(async () => {
    root?.render(<ConversationView conversationId={1} items={items} {...props} />);
  });
}

/** 폴링 한 바퀴 — 타이머를 돌리고 그 안의 await까지 흘려보낸다. */
async function tick() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(QUIZ_GATE_POLL_MS);
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  mocks.taskActivity.mockResolvedValue(longWait(1));
  mocks.quizAvailability.mockResolvedValue({ askable: 0, pending_review: 0 });
  mocks.insightAvailability.mockResolvedValue({
    cards: 1, decks: 0, enabled: true, warnings: [], deck_cards: 0, wiki_notes: 1, wiki_cards: 1, wiki_root: "/v",
  });
  mocks.insightNext.mockResolvedValue(CARD);
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root?.unmount());
  container?.remove();
  vi.useRealTimers();
  vi.clearAllMocks();
});

describe("ConversationView — 대기 카드는 대화 흐름 안에 놓인다", () => {
  it("이 대화의 턴이 길어지면 '작업 중' 띠 아래에 카드가 붙는다", async () => {
    await render({ busy: true, waitTaskId: 1 });
    await tick();

    const strip = busyStrip();
    const insight = card();
    expect(strip).toBeTruthy();
    expect(insight).toBeTruthy();
    // 띠 → 카드 순서. 카드는 스크롤 영역 안의 마지막 항목이다.
    expect(strip!.compareDocumentPosition(insight!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(scroller().contains(insight!)).toBe(true);
    expect(container?.textContent).toContain("그 외 작업");
  });

  it("응답이 도착해도 카드는 남는다 — 닫는 것은 사용자다(DR-3)", async () => {
    await render({ busy: true, waitTaskId: 1 });
    await tick();
    expect(card()).toBeTruthy();

    await render({ busy: false, waitTaskId: 1 });
    expect(busyStrip()).toBeUndefined();
    expect(card()).toBeTruthy();

    await act(async () => {
      (container?.querySelector('[aria-label="인사이트 닫기"]') as HTMLButtonElement).click();
    });
    expect(card()).toBeNull();
  });

  it("작업 id가 없으면 기다려도 카드를 띄우지 않고 백엔드도 묻지 않는다", async () => {
    await render({ busy: true });
    await tick();
    expect(card()).toBeNull();
    expect(mocks.taskActivity).not.toHaveBeenCalled();
  });

  it("다른 세션의 긴 대기는 이 대화에 카드를 띄우지 않는다", async () => {
    mocks.taskActivity.mockResolvedValue(longWait(2));
    await render({ busy: true, waitTaskId: 1 });
    await tick();
    expect(card()).toBeNull();
    expect(mocks.quizAvailability).not.toHaveBeenCalled();
  });
});

/** jsdom은 레이아웃이 없다 — 스크롤 영역의 치수를 손으로 쥔다. */
function fakeScrollMetrics(el: HTMLElement, metrics: { scrollHeight: number; clientHeight: number }) {
  let top = 0;
  Object.defineProperty(el, "scrollHeight", { configurable: true, get: () => metrics.scrollHeight });
  Object.defineProperty(el, "clientHeight", { configurable: true, get: () => metrics.clientHeight });
  Object.defineProperty(el, "scrollTop", {
    configurable: true,
    get: () => top,
    set: (v: number) => {
      top = Math.max(0, Math.min(v, metrics.scrollHeight - metrics.clientHeight));
    },
  });
}

describe("ConversationView — 카드 높이가 바뀌면 하단을 따라간다", () => {
  /** 관찰 대상과 콜백을 붙잡아 두는 ResizeObserver 스텁. */
  const observers: { target: Element; fire: () => void }[] = [];
  class ResizeObserverStub {
    constructor(private readonly cb: ResizeObserverCallback) {}
    observe(target: Element) {
      observers.push({ target, fire: () => this.cb([], this as unknown as ResizeObserver) });
    }
    disconnect() {}
    unobserve() {}
  }

  beforeEach(() => {
    observers.length = 0;
    vi.stubGlobal("ResizeObserver", ResizeObserverStub);
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  async function openCard() {
    await render({ busy: true, waitTaskId: 1 });
    await tick();
    expect(card()).toBeTruthy();
    // 대화 영역 자신도 관찰되고 그 안에 카드가 있다 — 카드 쪽 관찰자만 고른다.
    const observer = observers.find((o) => o.target !== scroller() && o.target.contains(card()!));
    expect(observer).toBeTruthy();
    return observer!;
  }

  it("'다음'으로 더 긴 카드가 오면 늘어난 만큼 내려가 새 카드가 다 보인다", async () => {
    const metrics = { scrollHeight: 1000, clientHeight: 400 };
    const observer = await openCard();
    const el = scroller();
    fakeScrollMetrics(el, metrics);
    el.scrollTop = 600; // 하단에 붙어 있음
    el.dispatchEvent(new Event("scroll"));

    mocks.insightNext.mockResolvedValue({ ...CARD, key: "wiki:b#1", title: "더 긴 카드", body: "본문 ".repeat(200) });
    await act(async () => {
      [...el.querySelectorAll("button")].find((b) => b.textContent?.includes("다음"))!.click();
    });
    expect(container?.textContent).toContain("더 긴 카드");

    metrics.scrollHeight = 1400; // 카드가 400px 자랐다
    observer.fire();
    expect(el.scrollTop).toBe(1000);
  });

  it("사용자가 위로 올라가 읽는 중이면 카드가 커져도 끌어내리지 않는다", async () => {
    const metrics = { scrollHeight: 1000, clientHeight: 400 };
    const observer = await openCard();
    const el = scroller();
    fakeScrollMetrics(el, metrics);
    el.scrollTop = 100; // 한참 위
    el.dispatchEvent(new Event("scroll"));

    metrics.scrollHeight = 1400;
    observer.fire();
    expect(el.scrollTop).toBe(100);
  });

  const scrollerObserver = () => observers.find((o) => o.target === scroller())!;

  it("아래 영역이 자라 대화 영역이 줄어도 하단을 보던 중이면 마지막 답변이 가려지지 않는다", async () => {
    const metrics = { scrollHeight: 1000, clientHeight: 400 };
    await render({ busy: false });
    const el = scroller();
    fakeScrollMetrics(el, metrics);
    el.scrollTop = 600;
    el.dispatchEvent(new Event("scroll"));

    metrics.clientHeight = 200; // 컴포저·대기열이 200px 자랐다 — 스크롤 이벤트는 나지 않는다
    scrollerObserver().fire();
    expect(el.scrollTop).toBe(800);
  });

  it("위로 올라가 읽는 중이면 대화 영역이 줄어도 읽던 자리를 옮기지 않는다", async () => {
    const metrics = { scrollHeight: 1000, clientHeight: 400 };
    await render({ busy: false });
    const el = scroller();
    fakeScrollMetrics(el, metrics);
    el.scrollTop = 100;
    el.dispatchEvent(new Event("scroll"));

    metrics.clientHeight = 200;
    scrollerObserver().fire();
    expect(el.scrollTop).toBe(100);
  });
});
