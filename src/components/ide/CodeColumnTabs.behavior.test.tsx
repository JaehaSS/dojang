// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CodeColumnTabs } from "./CodeColumnTabs";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const onActivate = vi.fn();
const onClose = vi.fn();

type Overrides = Partial<Parameters<typeof CodeColumnTabs>[0]>;

let host: HTMLDivElement;
let root: Root | null = null;

const render = async (props: Overrides = {}) => {
  await act(async () => {
    root?.render(
      <CodeColumnTabs
        active="file"
        onActivate={onActivate}
        previewAvailable
        editorPoppedOut={false}
        activity={<div>작업정보 본문</div>}
        file={<div>파일 본문</div>}
        preview={<div>프리뷰 본문</div>}
        {...props}
      />,
    );
  });
};

const labelled = (label: string): HTMLButtonElement | undefined =>
  [...host.querySelectorAll("button")].find(
    (b) => b.getAttribute("aria-label") === label,
  ) as HTMLButtonElement | undefined;

const tab = (text: string): HTMLButtonElement | undefined =>
  [...host.querySelectorAll('button[role="tab"]')].find(
    (b) => b.textContent === text,
  ) as HTMLButtonElement | undefined;

const clickOn = async (button: HTMLElement | undefined) => {
  await act(async () => {
    button?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
};

beforeEach(() => {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  onActivate.mockClear();
  onClose.mockClear();
});

afterEach(async () => {
  await act(async () => root?.unmount());
  host.remove();
  root = null;
});

describe("CodeColumnTabs", () => {
  it("소환된 면은 자기 위에 닫기를 갖는다 — 탭줄에 코드 열 닫기 버튼을 낸다", async () => {
    await render({ onClose });

    const close = labelled("코드 열 닫기");
    expect(close).toBeTruthy();

    await clickOn(close);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("닫기를 받지 않으면 닫는 시늉을 하지 않는다 — 버튼 자체가 없다", async () => {
    await render();

    expect(labelled("코드 열 닫기")).toBeUndefined();
  });

  it("탭 버튼을 누르면 그 탭으로 옮긴다", async () => {
    await render({ active: "file", onClose });

    await clickOn(tab("프리뷰"));
    expect(onActivate).toHaveBeenCalledWith("preview");
    // 옆에 닫기가 생겨도 탭은 여전히 탭이다 — 눌러도 열이 닫히지 않는다.
    expect(onClose).not.toHaveBeenCalled();
  });

  it("닫기는 탭이 아니다 — 탭 목록에 섞여 선택 상태를 갖지 않는다", async () => {
    await render({ onClose });

    const close = labelled("코드 열 닫기");
    expect(close?.getAttribute("role")).not.toBe("tab");
    expect(close?.hasAttribute("aria-selected")).toBe(false);
  });

  it("보고 있는 탭만 선택으로 표시한다", async () => {
    await render({ active: "preview", onClose });

    expect(tab("프리뷰")?.getAttribute("aria-selected")).toBe("true");
    expect(tab("파일")?.getAttribute("aria-selected")).toBe("false");
    expect(tab("작업정보")?.getAttribute("aria-selected")).toBe("false");
  });

  it("좁은 열에서는 탭을 가로로 스크롤하고 닫기를 고정한다", async () => {
    await render({ onClose });

    const tabList = host.querySelector('[role="tablist"]') as HTMLDivElement | null;
    expect(tabList?.className).toContain("overflow-x-auto");
    expect([...host.querySelectorAll('[role="tab"]')].every((item) =>
      item.className.includes("whitespace-nowrap") && item.className.includes("shrink-0"),
    )).toBe(true);
    expect(labelled("코드 열 닫기")?.parentElement?.className).toContain("shrink-0");
  });

  it("외부에서 고른 탭도 가로 목록 안으로 가져온다", async () => {
    const descriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollIntoView");
    const scrollIntoView = vi.fn();
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", { configurable: true, value: scrollIntoView });

    try {
      await render({ active: "preview", onClose, preview: <div>프리뷰</div> });

      expect(scrollIntoView).toHaveBeenCalledWith({ block: "nearest", inline: "nearest" });
    } finally {
      if (descriptor) Object.defineProperty(HTMLElement.prototype, "scrollIntoView", descriptor);
      else delete (HTMLElement.prototype as { scrollIntoView?: unknown }).scrollIntoView;
    }
  });
});
