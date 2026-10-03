// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
  pushCapture: vi.fn(),
  requestComposerFocus: vi.fn(),
}));

// 이 화면에는 Monaco가 없다 — 마운트되지 않지만 import는 되므로 자리만 채운다.
vi.mock("@monaco-editor/react", async () => {
  const React = await import("react");
  return {
    default: (props: { options?: { readOnly?: boolean } }) => React.createElement("div", {
      "data-testid": "monaco",
      "data-readonly": String(props.options?.readOnly === true),
    }),
  };
});
vi.mock("../../lib/monaco", () => ({ langFromPath: () => "markdown" }));
vi.mock("./Mermaid", () => ({ Mermaid: () => null }));
vi.mock("../../lib/designmode/editor-capture-target", () => ({
  registerEditorCaptureTarget: () => () => undefined,
}));
vi.mock("../../lib/designmode/store", async (original) => ({
  ...(await original<Record<string, unknown>>()),
  pushCapture: h.pushCapture,
}));
vi.mock("../../lib/composer-focus", async (original) => ({
  ...(await original<Record<string, unknown>>()),
  requestComposerFocus: h.requestComposerFocus,
}));

import { EditorPane, type OpenFile } from "./EditorPane";
import { fileTabKey } from "../../lib/tab-key";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** 줄 번호를 세기 쉬운 문서 — 1줄 제목, 3줄 첫 문단, 5~6줄 목록. */
const DOC = ["# 제목", "", "첫 문단이다.", "", "- 하나", "- 둘"].join("\n");

const FILE: OpenFile = {
  key: fileTabKey("docs/note.md"),
  path: "docs/note.md",
  kind: "text",
  content: DOC,
  baseContent: DOC,
  mtime: 0,
  dirty: false,
};

const READ_ONLY_FILE: OpenFile = {
  ...FILE,
  key: fileTabKey("/Users/test/external.ts"),
  path: "/Users/test/external.ts",
  readOnly: true,
};

let container: HTMLDivElement;
let root: Root;

const render = async (over: Partial<Parameters<typeof EditorPane>[0]> = {}) => {
  await act(async () => {
    root.render(
      <EditorPane
        taskId={7}
        files={[FILE]}
        activeKey={FILE.key}
        retainedPaths={[FILE.path]}
        dark={false}
        onSelect={() => undefined}
        onClose={() => undefined}
        onChange={() => undefined}
        onSave={() => undefined}
        onReload={() => undefined}
        onOpenPath={() => undefined}
        onRevealPath={() => undefined}
        {...over}
      />,
    );
  });
};

/** 렌더된 문서에서 이 선택자의 텍스트를 통째로 고른다. */
const selectIn = (selector: string) => {
  const el = container.querySelector(selector);
  expect(el, `${selector}를 찾지 못했다`).toBeTruthy();
  const range = document.createRange();
  range.selectNodeContents(el as Node);
  const selection = window.getSelection();
  selection?.removeAllRanges();
  selection?.addRange(range);
};

const pressCmdL = async () => {
  const event = new KeyboardEvent("keydown", {
    code: "KeyL",
    metaKey: true,
    bubbles: true,
    cancelable: true,
  });
  await act(async () => {
    window.dispatchEvent(event);
  });
  return event;
};

beforeEach(() => {
  h.pushCapture.mockClear();
  h.requestComposerFocus.mockClear();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  window.getSelection()?.removeAllRanges();
  act(() => root.unmount());
  container.remove();
});

describe("마크다운 프리뷰", () => {
  it("renders an SSH Markdown document and keeps its source read-only", async () => {
    const file: OpenFile = { ...FILE, path: "/srv/reports/note.md", key: fileTabKey("/srv/reports/note.md"), readOnly: true };
    await render({ files: [file], activeKey: file.key, retainedPaths: [file.path] });
    expect(container.querySelector("h1")?.textContent).toBe("제목");
    expect(container.querySelectorAll("li")).toHaveLength(2);
    expect(container.textContent).toContain("읽기 전용");
    const source = [...container.querySelectorAll("button")].find((button) => button.textContent === "소스");
    expect(source).toBeTruthy();
    await act(async () => source!.click());
    expect(container.querySelector('[data-testid="monaco"]')?.getAttribute("data-readonly")).toBe("true");
  });
  it("외부 파일의 코드 탭은 읽기 전용임을 표시한다", async () => {
    await render({ files: [READ_ONLY_FILE], activeKey: READ_ONLY_FILE.key, retainedPaths: [READ_ONLY_FILE.path] });

    expect(container.querySelector("[data-testid=monaco]")?.getAttribute("data-readonly")).toBe("true");
    expect(container.textContent).toContain("읽기 전용");
  });

  it("링크가 아닌 본문은 기본 우클릭을 그대로 둔다", async () => {
    await render();

    const paragraph = container.querySelector("p");
    expect(paragraph).toBeTruthy();
    const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    await act(async () => paragraph?.dispatchEvent(event));

    expect(event.defaultPrevented).toBe(false);
  });

  it("링크 안쪽을 우클릭하면 브라우저 메뉴를 막고 링크 메뉴를 연다", async () => {
    const onLinkMenu = vi.fn();
    const linked = {
      ...FILE,
      content: "[바깥 **안쪽**](../../DESIGN%20%ED%95%9C%EA%B8%80.md:4#intro)",
    };
    await render({ files: [linked], onLinkMenu });

    const nested = container.querySelector("a strong");
    expect(nested).toBeTruthy();
    const event = new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: 30,
      clientY: 40,
    });
    await act(async () => nested?.dispatchEvent(event));

    expect(event.defaultPrevented).toBe(true);
    expect(onLinkMenu).toHaveBeenCalledWith(
      FILE.key,
      "docs/note.md",
      "../../DESIGN%20%ED%95%9C%EA%B8%80.md:4#intro",
      { x: 30, y: 40 },
    );
  });

  it("링크를 클릭하면 브라우저 이동을 막고 여는 쪽에 넘긴다", async () => {
    const onOpenLink = vi.fn();
    const linked = {
      ...FILE,
      content: "[바깥 **안쪽**](../../DESIGN%20%ED%95%9C%EA%B8%80.md:4#intro)",
    };
    await render({ files: [linked], onOpenLink });

    const nested = container.querySelector("a strong");
    const event = new MouseEvent("click", { bubbles: true, cancelable: true });
    await act(async () => nested?.dispatchEvent(event));

    // 막지 않으면 target="_blank"가 문서를 새 창으로 띄운다 — 에디터 밖으로 나가 버린다.
    expect(event.defaultPrevented).toBe(true);
    expect(onOpenLink).toHaveBeenCalledWith(
      FILE.key,
      "docs/note.md",
      "../../DESIGN%20%ED%95%9C%EA%B8%80.md:4#intro",
    );
  });

  it("렌더된 블록이 원본 줄 번호를 들고 있다", async () => {
    await render();
    expect(container.querySelector("h1")?.getAttribute("data-md-line")).toBe("1");
    expect(container.querySelector("p")?.getAttribute("data-md-line")).toBe("3");
    expect(container.querySelector("ul")?.getAttribute("data-md-line")).toBe("5");
    expect(container.querySelector("ul")?.getAttribute("data-md-line-end")).toBe("6");
  });

  it("⌘L이 고른 문단을 그 줄 범위로 첨부한다", async () => {
    await render();
    selectIn("p");

    const event = await pressCmdL();

    expect(event.defaultPrevented).toBe(true);
    expect(h.pushCapture).toHaveBeenCalledTimes(1);
    const [taskId, record] = h.pushCapture.mock.calls[0];
    expect(taskId).toBe(7);
    expect(record).toMatchObject({
      file_path: "docs/note.md",
      selection_text: "첫 문단이다.",
      selection_start_line: 3,
      selection_end_line: 3,
    });
    expect(h.requestComposerFocus).toHaveBeenCalledWith(7);
  });

  it("여러 블록에 걸친 선택은 앞 블록의 시작부터 뒤 블록의 끝까지", async () => {
    await render();
    const range = document.createRange();
    range.setStart(container.querySelector("p") as Node, 0);
    range.setEnd(container.querySelector("ul") as Node, 1);
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);

    await pressCmdL();

    expect(h.pushCapture.mock.calls[0][1]).toMatchObject({
      selection_start_line: 3,
      selection_end_line: 6,
    });
  });

  it("고른 것이 없으면 아무 일도 하지 않는다 — 파일 전체 참조는 @멘션의 몫이다", async () => {
    await render();
    const event = await pressCmdL();

    expect(event.defaultPrevented).toBe(false);
    expect(h.pushCapture).not.toHaveBeenCalled();
  });

  it("팝아웃 창에서는 store를 건드리지 않고 메인 창으로 넘긴다", async () => {
    const onAttachCapture = vi.fn();
    await render({ onAttachCapture });
    selectIn("h1");

    await pressCmdL();

    expect(onAttachCapture).toHaveBeenCalledTimes(1);
    expect(onAttachCapture.mock.calls[0][0]).toMatchObject({ selection_start_line: 1 });
    expect(h.pushCapture).not.toHaveBeenCalled();
    // 남의 창으로 포커스를 뺏지 않는다.
    expect(h.requestComposerFocus).not.toHaveBeenCalled();
  });
});
