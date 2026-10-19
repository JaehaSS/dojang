// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CodeColumnTabs, type CodeTab } from "./CodeColumnTabs";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

const onActivate = vi.fn();

type TabsProps = Parameters<typeof CodeColumnTabs>[0];

/** 세 슬롯에 서로 구분되는 더미를 넣는다 — 어느 칸이 보이는지 텍스트로 판별하기 위해서다. */
const render = (extra: Partial<TabsProps> = {}) => {
  const props: TabsProps = {
    active: "activity",
    onActivate,
    previewAvailable: true,
    editorPoppedOut: false,
    activity: <div data-testid="slot-activity">작업정보 내용</div>,
    file: <div data-testid="slot-file">파일 내용</div>,
    preview: <div data-testid="slot-preview">프리뷰 내용</div>,
    ...extra,
  };
  act(() => {
    root.render(<CodeColumnTabs {...props} />);
  });
};

const tabs = () => [...container.querySelectorAll('[role="tab"]')] as HTMLButtonElement[];
const tabLabels = () => tabs().map((el) => el.textContent?.trim());
const selectedLabel = () =>
  tabs()
    .find((el) => el.getAttribute("aria-selected") === "true")
    ?.textContent?.trim();
/** 슬롯을 감싼 div가 보이는지 — 자식은 언마운트하지 않고 hidden으로 감춘다. */
const slotVisible = (name: string) => {
  const wrap = container.querySelector(`[data-testid="slot-${name}"]`)?.parentElement;
  return wrap !== null && wrap !== undefined && !wrap.className.includes("hidden");
};

beforeEach(() => {
  onActivate.mockClear();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("CodeColumnTabs — 탭 구성", () => {
  it("Design 슬롯을 제공하면 로컬 프리뷰 다음에 탭을 추가한다", () => {
    render({ design: <div>캔버스</div>, active: "design" });
    expect(tabLabels()).toEqual(["작업정보", "파일", "프리뷰", "Design"]);
    expect(selectedLabel()).toBe("Design");
    render({ design: <div>캔버스</div>, previewAvailable: false });
    expect(tabLabels()).not.toContain("Design");
  });
  it("작업정보가 항상 맨 앞에 상주한다", () => {
    render();
    expect(tabLabels()).toEqual(["작업정보", "파일", "프리뷰"]);
  });

  it("프리뷰를 쓸 수 없으면 탭 자체를 만들지 않는다", () => {
    render({ previewAvailable: false });
    expect(tabLabels()).not.toContain("프리뷰");
  });

  it("탭을 누르면 활성 전환을 위로 올린다", () => {
    render();
    act(() => {
      tabs()[2].dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onActivate).toHaveBeenCalledWith<[CodeTab]>("preview");
  });
});

describe("CodeColumnTabs — 팝아웃 손잡이 제거(회귀 방지)", () => {
  it("팝아웃 버튼을 더 이상 그리지 않는다", () => {
    // 손잡이는 세션 헤더로 옮겼다 — 여기 남아 있으면 코드 열을 열어야만 보이는 자리로 되돌아간다.
    render();
    expect(container.querySelector('[aria-label="에디터 팝아웃"]')).toBeNull();
    expect(container.textContent).not.toContain("⧉");
  });

  it("탭 바에는 탭 말고 다른 버튼이 없다", () => {
    // aria-label이나 글리프를 바꿔 되살리는 것까지 막는다.
    render();
    const buttons = [...container.querySelectorAll("button")];
    expect(buttons.filter((el) => el.getAttribute("role") !== "tab")).toEqual([]);
  });
});

describe("CodeColumnTabs — 에디터가 나가 있을 때", () => {
  it("파일 탭이 사라진다", () => {
    // 같은 코드를 두 자리에 두지 않는다.
    render({ editorPoppedOut: true });
    expect(tabLabels()).toEqual(["작업정보", "프리뷰"]);
  });

  it("파일이 활성이던 채로 나가면 첫 탭으로 되돌린다", () => {
    render({ editorPoppedOut: true, active: "file" });
    expect(selectedLabel()).toBe("작업정보");
  });

  it("되돌린 첫 탭의 내용이 실제로 보인다", () => {
    // 활성만 옮기고 슬롯을 안 열면 탭은 선택돼 보이는데 화면은 비어 있다.
    render({ editorPoppedOut: true, active: "file" });
    expect(slotVisible("activity")).toBe(true);
    expect(slotVisible("file")).toBe(false);
  });

  it("돌아오면 파일 탭이 다시 선다", () => {
    render({ editorPoppedOut: true, active: "file" });
    render({ editorPoppedOut: false, active: "file" });
    expect(tabLabels()).toContain("파일");
    expect(slotVisible("file")).toBe(true);
  });
});


it("keeps the separate question tab inert until selected and supports keyboard navigation", () => {
  const question = <textarea data-testid="slot-question" defaultValue="preserved" />;
  render({ question });
  expect(tabLabels()).toContain("따로 질문");
  expect(container.querySelector('[data-testid="slot-question"]')?.parentElement?.hasAttribute("inert")).toBe(true);
  act(() => tabs()[0].dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true })));
  expect(onActivate).toHaveBeenCalledWith("question");
  render({ question, active: "question" });
  expect(selectedLabel()).toBe("따로 질문");
  expect(container.querySelector('[data-testid="slot-question"]')?.parentElement?.hasAttribute("inert")).toBe(false);
  expect((container.querySelector('[data-testid="slot-question"]') as HTMLTextAreaElement).value).toBe("preserved");
});
