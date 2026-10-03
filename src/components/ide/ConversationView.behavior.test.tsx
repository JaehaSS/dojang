// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ConversationView, type ConvoItem } from "./ConversationView";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement | null = null;
let root: Root | null = null;

async function renderConversation(conversationId: number, text?: string, hidden = false): Promise<void> {
  const items: ConvoItem[] = text == null ? [] : [{ role: "text", text }];
  await act(async () => {
    root?.render(
      <ConversationView conversationId={conversationId} items={items} busy={false} hidden={hidden} />,
    );
  });
}

function setScrollMetrics(scrollContainer: HTMLDivElement, scrollHeight: number): void {
  Object.defineProperties(scrollContainer, {
    clientHeight: { configurable: true, value: 500 },
    scrollHeight: { configurable: true, value: scrollHeight },
  });
}

function scrollAwayFromBottom(): HTMLDivElement {
  const scrollContainer = container?.firstElementChild?.firstElementChild as HTMLDivElement;
  setScrollMetrics(scrollContainer, 1_000);
  scrollContainer.scrollTop = 100;
  scrollContainer.dispatchEvent(new Event("scroll"));
  return scrollContainer;
}

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root?.unmount());
  container?.remove();
  root = null;
  container = null;
});

describe("ConversationView scroll lifecycle", () => {
  it("다른 세션을 본 뒤 돌아오면 이전 세션의 스크롤 위치를 복원한다", async () => {
    await renderConversation(1, "첫 번째 세션");
    const scrollContainer = scrollAwayFromBottom();

    await renderConversation(2, "두 번째 세션");

    expect(scrollContainer.scrollTop).toBe(1_000);

    await renderConversation(1, "첫 번째 세션");

    expect(scrollContainer.scrollTop).toBe(100);
  });

  it("저장 히스토리를 불러오는 중의 빈 상태가 복원 위치를 덮어쓰지 않는다", async () => {
    await renderConversation(1, "첫 번째 세션");
    const scrollContainer = scrollAwayFromBottom();
    await renderConversation(2, "두 번째 세션");

    setScrollMetrics(scrollContainer, 0);
    await renderConversation(1);
    scrollContainer.scrollTop = 0;
    scrollContainer.dispatchEvent(new Event("scroll"));

    setScrollMetrics(scrollContainer, 1_000);
    await renderConversation(1, "복원된 첫 번째 세션");

    expect(scrollContainer.scrollTop).toBe(100);
  });

  it("같은 세션에서는 사용자가 이전 대화를 읽는 위치를 유지한다", async () => {
    await renderConversation(1, "첫 번째 응답");
    const scrollContainer = scrollAwayFromBottom();

    await renderConversation(1, "이어지는 응답");

    expect(scrollContainer.scrollTop).toBe(100);
  });

  it("Diff 동안 스트림이 와도 처음 복귀한 위치를 보존한다", async () => {
    await renderConversation(1, "첫 번째 응답");
    const scrollContainer = container?.firstElementChild?.firstElementChild as HTMLDivElement;
    setScrollMetrics(scrollContainer, 1_000);
    scrollContainer.scrollTop = 500;

    await renderConversation(1, "스트리밍 응답", true);
    setScrollMetrics(scrollContainer, 1_400);
    scrollContainer.scrollTop = 300;
    scrollContainer.dispatchEvent(new Event("scroll"));
    await renderConversation(1, "스트리밍 응답");

    expect(scrollContainer.scrollTop).toBe(500);
  });
});

describe("ConversationView 폭 변화", () => {
  /** 코드 열이 열리고 닫히면 대화 열의 폭이 바뀐다 — 대화 영역을 지켜보는 관찰자를 손으로 부른다. */
  let observers: { callback: () => void; targets: Element[] }[] = [];
  let scrollContainer: HTMLDivElement;
  const resize = () => {
    for (const o of observers) if (o.targets.includes(scrollContainer)) o.callback();
  };
  beforeEach(() => {
    vi.stubGlobal(
      "ResizeObserver",
      class {
        private entry: { callback: () => void; targets: Element[] };
        constructor(callback: () => void) {
          this.entry = { callback, targets: [] };
          observers.push(this.entry);
        }
        observe(target: Element) {
          this.entry.targets.push(target);
        }
        disconnect() {
          this.entry.targets = [];
        }
      },
    );
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    observers = [];
  });

  /**
   * 첫 항목이 문서에서 차지하는 위치를 줄바꿈 상태로 흉내 낸다. 넓으면 400px 지점, 좁아져 줄이
   * 늘면 위 항목이 길어져 700px 지점으로 밀린다. 뷰포트 윗변은 0이다.
   */
  function layout(scrollContainer: HTMLDivElement, width: () => number): HTMLElement {
    const item = scrollContainer.lastElementChild as HTMLElement;
    Object.defineProperty(scrollContainer, "clientWidth", { configurable: true, get: width });
    scrollContainer.getBoundingClientRect = () => ({ top: 0, bottom: 500 }) as DOMRect;
    item.getBoundingClientRect = () => {
      const top = (width() >= 600 ? 400 : 700) - scrollContainer.scrollTop;
      return { top, bottom: top + 300 } as DOMRect;
    };
    return item;
  }

  it("위로 읽던 중 폭이 바뀌면 보던 항목을 같은 자리에 다시 놓는다", async () => {
    await renderConversation(1, "읽던 답변");
    scrollContainer = container?.firstElementChild?.firstElementChild as HTMLDivElement;
    let width = 800;
    const item = layout(scrollContainer, () => width);
    setScrollMetrics(scrollContainer, 1_000);
    act(() => resize());
    scrollContainer.scrollTop = 300;
    scrollContainer.dispatchEvent(new Event("scroll"));
    expect(item.getBoundingClientRect().top).toBe(100);

    width = 500;
    // 줄어든 길이 때문에 브라우저가 낸 스크롤이 관찰자보다 먼저 와도 기준을 덮어쓰지 않는다.
    scrollContainer.dispatchEvent(new Event("scroll"));
    act(() => resize());

    expect(item.getBoundingClientRect().top).toBe(100);
    expect(scrollContainer.scrollTop).toBe(600);
  });

  it("하단을 따라가던 중이면 폭이 바뀌어도 하단에 붙는다", async () => {
    await renderConversation(1, "마지막 답변");
    scrollContainer = container?.firstElementChild?.firstElementChild as HTMLDivElement;
    let width = 800;
    layout(scrollContainer, () => width);
    setScrollMetrics(scrollContainer, 1_000);
    act(() => resize());

    width = 500;
    setScrollMetrics(scrollContainer, 1_600);
    act(() => resize());

    expect(scrollContainer.scrollTop).toBe(1_600);
  });
});

describe("ConversationView 렌더 윈도우", () => {
  /** 아이템 n개를 `대화 0` … `대화 n-1`로 만든다 — 어디까지 올라왔는지 텍스트로 판정하려고. */
  const numbered = (count: number): ConvoItem[] =>
    Array.from({ length: count }, (_, i) => ({ role: "text", text: `대화 ${i}` }) as ConvoItem);

  async function renderItems(items: ConvoItem[]): Promise<void> {
    await act(async () => {
      root?.render(<ConversationView conversationId={1} items={items} busy={false} />);
    });
  }

  function moreButton(): HTMLButtonElement | null {
    const buttons = [...(container?.querySelectorAll("button") ?? [])] as HTMLButtonElement[];
    return buttons.find((b) => b.textContent?.includes("더 보기")) ?? null;
  }

  it("상한 이하면 전부 올리고 더 보기를 내지 않는다", async () => {
    await renderItems(numbered(30));

    expect(container?.textContent).toContain("대화 0");
    expect(container?.textContent).toContain("대화 29");
    expect(moreButton()).toBeNull();
  });

  it("상한을 넘으면 최신 쪽만 올리고 남은 개수를 알린다", async () => {
    await renderItems(numbered(200));

    // 오래된 80건(200 - 120)은 아직 DOM에 없다 — 세션 진입에서 수천 노드를 만들지 않는 지점.
    expect(container?.textContent).not.toContain("대화 0");
    expect(container?.textContent).toContain("대화 199");
    expect(moreButton()?.textContent).toContain("80개");
  });

  it("더 보기를 누르면 이전 대화가 올라오고 버튼이 사라진다", async () => {
    await renderItems(numbered(200));

    const button = moreButton();
    await act(async () => {
      button?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(container?.textContent).toContain("대화 0");
    expect(moreButton()).toBeNull();
  });

  it("스트리밍으로 아이템이 늘어도 윈도우는 최신을 따라간다", async () => {
    await renderItems(numbered(200));
    await renderItems([...numbered(200), { role: "text", text: "새 응답" }]);

    expect(container?.textContent).toContain("새 응답");
    expect(moreButton()?.textContent).toContain("81개");
  });
});

describe("ConversationView — 컨텍스트 절단 구분선", () => {
  it("divider를 위아래 대화 사이에 separator로 그린다", async () => {
    // 이 경계가 화면에 없으면 위아래가 하나의 대화로 읽히는데 에이전트는 위를 기억하지 못한다.
    const items: ConvoItem[] = [
      { role: "text", text: "문서화 결과" },
      { role: "divider", text: "컨텍스트를 비웠습니다. CLAUDE.md" },
      { role: "text", text: "구현 시작" },
    ];
    await act(async () => {
      root?.render(<ConversationView conversationId={1} items={items} busy={false} />);
    });

    const separator = container?.querySelector('[role="separator"]');
    expect(separator?.textContent).toContain("컨텍스트를 비웠습니다");

    const text = container?.textContent ?? "";
    expect(text.indexOf("문서화 결과")).toBeLessThan(text.indexOf("컨텍스트를 비웠습니다"));
    expect(text.indexOf("컨텍스트를 비웠습니다")).toBeLessThan(text.indexOf("구현 시작"));
  });
});


it("sends only the selection within its message to a separate question", async () => {
  const onAsk = vi.fn();
  await act(async () => root?.render(<ConversationView conversationId="local:7" items={[{ role: "text", text: "first selected last" }]} busy={false} onAskSeparately={onAsk} />));
  const paragraph = [...container!.querySelectorAll("p")].find((node) => node.textContent === "first selected last")!;
  const range = document.createRange();
  range.setStart(paragraph.firstChild!, 6); range.setEnd(paragraph.firstChild!, 14);
  window.getSelection()!.removeAllRanges(); window.getSelection()!.addRange(range);
  const action = [...container!.querySelectorAll("button")].find((node) => node.textContent === "따로 질문")!;
  await act(async () => action.click());
  expect(onAsk).toHaveBeenLastCalledWith("selected");
  window.getSelection()!.removeAllRanges();
  await act(async () => action.click());
  expect(onAsk).toHaveBeenLastCalledWith("first selected last");
});

describe("ConversationView — 누적 지출 게이지는 승계분을 빼고 센다", () => {
  /** 헤더 게이지 — 렌더될 때만 바깥 컨테이너의 첫 자식이다. 아이템별 meta 줄도 같은 금액 문자열을
   *  그리므로 textContent 전체로 보면 둘을 구분할 수 없다. */
  function spendHeader(): HTMLElement | null {
    const header = container?.firstElementChild?.firstElementChild as HTMLElement | undefined;
    return header && header.textContent?.startsWith("누적") ? header : null;
  }

  it("승계한 턴의 비용·토큰·턴 수는 누적에서 제외한다", async () => {
    // 승계분을 더하면 원본이 이미 계상한 지출이 이어받을 때마다 한 번씩 더 불어난다.
    const items: ConvoItem[] = [
      { role: "meta", cost: 0.5, turns: 3, tokensIn: 10_000, tokensOut: 20_000, inherited: true },
      { role: "meta", cost: 0.02, turns: 1, tokensIn: 1_000, tokensOut: 2_000 },
      { role: "meta", cost: 0.01, turns: 1, tokensIn: 500, tokensOut: 1_500 },
    ];
    await act(async () => {
      root?.render(<ConversationView conversationId={1} items={items} busy={false} />);
    });

    const header = spendHeader();
    expect(header?.textContent).toContain("$0.030");
    expect(header?.textContent).toContain("1.5k → 3.5k tok");
    expect(header?.textContent).toContain("2턴");
    // 승계분이 섞였다면 $0.530 · 11.5k → 23.5k · 3턴이 됐을 것이다.
    expect(header?.textContent).not.toContain("0.530");
    expect(header?.textContent).not.toContain("3턴");
  });

  it("승계분만 있으면 게이지 자체를 띄우지 않는다 — 이 작업이 쓴 것은 아직 없다", async () => {
    const items: ConvoItem[] = [
      { role: "meta", cost: 0.5, turns: 3, tokensIn: 10_000, tokensOut: 20_000, inherited: true },
    ];
    await act(async () => {
      root?.render(<ConversationView conversationId={1} items={items} busy={false} />);
    });

    expect(spendHeader()).toBeNull();
    // 아이템이 버려진 것이 아니라 누적에서만 빠진 것이다.
    expect(container?.textContent).toContain("$0.500");
  });

  it("비용 없이 토큰만 주는 벤더도 자기 턴은 누적에 센다", async () => {
    const items: ConvoItem[] = [
      { role: "meta", cost: 0.4, turns: 2, tokensIn: 8_000, tokensOut: 9_000, inherited: true },
      { role: "meta", cost: 0, turns: 1, tokensIn: 1_200, tokensOut: 3_400 },
    ];
    await act(async () => {
      root?.render(<ConversationView conversationId={1} items={items} busy={false} />);
    });

    const header = spendHeader();
    expect(header?.textContent).toContain("1.2k → 3.4k tok");
    expect(header?.textContent).toContain("1턴");
    expect(header?.textContent).not.toContain("$");
  });
});

describe("ConversationView — 공백 없는 긴 줄은 가로로 넘치지 않는다", () => {
  /** 스택트레이스 한 프레임 — 공백이 없어 pre-wrap이 끊을 자리를 찾지 못한다. */
  const FRAME =
    "at com.example.app.service.document.processor.OcrDocumentSaveProcessor.process(OcrDocumentSaveProcessor.java:104)";

  /** pre-wrap으로 그려진 블록들 — 말풍선·에러·툴 결과가 모두 같은 형식을 쓴다. */
  function preWrapBlocks(text: string): HTMLElement[] {
    return [...container!.querySelectorAll<HTMLElement>("div")].filter(
      (node) => node.textContent === text && node.className.includes("whitespace-pre-wrap"),
    );
  }

  it("사용자 말풍선·에러·툴 결과가 긴 토큰을 끊어 접는다", async () => {
    // 접지 않으면 박스를 넘쳐 스크롤 컨테이너에 가로 스크롤 영역이 생기고,
    // 한 번 옆으로 밀리면 대화 전체가 화면 밖으로 잘려 나간다.
    const items: ConvoItem[] = [
      { role: "user", text: FRAME },
      { role: "error", text: FRAME },
      { role: "tool_result", summary: FRAME, is_error: false },
    ];
    await act(async () => {
      root?.render(<ConversationView conversationId={1} items={items} busy={false} />);
    });

    const blocks = preWrapBlocks(FRAME);
    expect(blocks).toHaveLength(3);
    for (const block of blocks) expect(block.className).toContain("break-words");
  });

  it("대화 스크롤 컨테이너는 가로로 스크롤되지 않는다", async () => {
    await act(async () => {
      root?.render(<ConversationView conversationId={1} items={[{ role: "user", text: FRAME }]} busy={false} />);
    });
    const scroller = container?.firstElementChild?.firstElementChild as HTMLDivElement;
    expect(scroller.className).toContain("overflow-x-hidden");
    expect(scroller.className).not.toMatch(/\boverflow-auto\b/);
  });
});

describe("ConversationView — 구조화된 도구 미디어", () => {
  it("allowlist된 data 이미지·음성을 표시하고 리소스는 사용자 클릭으로만 연다", async () => {
    const onOpenLink = vi.fn();
    const image = "data:image/png;base64,aGVsbG8=";
    const audio = "data:audio/mpeg;base64,aGVsbG8=";
    const items: ConvoItem[] = [{
      role: "tool_output",
      toolUseId: "tool-1",
      contents: [
        { type: "text", text: "connector output" },
        { type: "image", url: image },
        { type: "audio", url: audio },
        { type: "resource", uri: "https://example.com/report", title: "보고서 열기" },
      ],
    }];
    await act(async () => root?.render(<ConversationView conversationId={1} items={items} busy={false} onOpenLink={onOpenLink} />));

    expect(container?.textContent).toContain("connector output");
    expect(container?.querySelector("img")?.getAttribute("src")).toBe(image);
    const player = container?.querySelector("audio") as HTMLAudioElement;
    expect(player?.getAttribute("src")).toBe(audio);
    expect(player?.autoplay).toBe(false);
    expect(onOpenLink).not.toHaveBeenCalled();
    await act(async () => [...container!.querySelectorAll("button")].find((node) => node.textContent === "보고서 열기")?.click());
    expect(onOpenLink).toHaveBeenCalledWith("https://example.com/report");
  });

  it("refuses remote media, invalid MIME, and executable resource URLs", async () => {
    const onOpenLink = vi.fn();
    const items: ConvoItem[] = [{
      role: "tool_output",
      toolUseId: "tool-1",
      contents: [
        { type: "image", url: "https://evil.example/track.png" },
        { type: "audio", url: "data:text/html;base64,aGVsbG8=" },
        { type: "resource", uri: "javascript:alert(1)", title: "열기" },
      ],
    }];
    await act(async () => root?.render(<ConversationView conversationId={1} items={items} busy={false} onOpenLink={onOpenLink} />));

    expect(container?.querySelector("img")).toBeNull();
    expect(container?.querySelector("audio")).toBeNull();
    expect([...container!.querySelectorAll("button")].some((node) => node.textContent === "열기")).toBe(false);
    expect(onOpenLink).not.toHaveBeenCalled();
  });
});
