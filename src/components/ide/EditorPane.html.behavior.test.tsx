// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
  pushCapture: vi.fn(),
  requestComposerFocus: vi.fn(),
  /** 언마운트된 Monaco는 dispose된다 — 그 뒤의 `getValue()`는 빈 문자열이다. */
  disposed: false,
}));

/** `registerLanguageFeatures`가 마운트에서 만지는 것만 흉내 낸다. */
const monacoStub = {
  KeyMod: { CtrlCmd: 2048, Alt: 512, Shift: 1024 },
  KeyCode: { KeyS: 49, KeyB: 32, KeyL: 42, F12: 71 },
  languages: {
    registerDefinitionProvider: () => ({ dispose: () => undefined }),
    registerImplementationProvider: () => ({ dispose: () => undefined }),
  },
  // 모델 정리 이펙트가 만지는 것 — Monaco가 마운트된 뒤 files가 바뀌면 실행된다.
  Uri: { parse: (p: string) => ({ toString: () => `file://${p}` }) },
  editor: { getModels: () => [] },
};

// 프리뷰 화면에는 Monaco가 없다 — 소스 모드로 바꿔야 나타난다. 마운트·언마운트가
// 에디터 인스턴스의 수명과 같아야 프리뷰에서의 저장이 무엇을 읽는지 볼 수 있다.
vi.mock("@monaco-editor/react", async () => {
  const React = await import("react");
  return {
    default: ({ onMount }: { onMount: (ed: unknown, m: unknown) => void }) => {
      React.useEffect(() => {
        h.disposed = false;
        onMount(
          {
            addCommand: () => undefined,
            addAction: () => ({ dispose: () => undefined }),
            onMouseDown: () => ({ dispose: () => undefined }),
            onMouseUp: () => ({ dispose: () => undefined }),
            createContextKey: () => ({ set: () => undefined }),
            onDidChangeCursorSelection: () => undefined,
            onDidScrollChange: () => undefined,
            getValue: () => (h.disposed ? "" : DOC),
          },
          monacoStub,
        );
        return () => {
          h.disposed = true;
        };
      }, [onMount]);
      return React.createElement("div", { "data-testid": "monaco" });
    },
  };
});
vi.mock("../../lib/monaco", () => ({ langFromPath: () => "html" }));
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

const DOC = "<!doctype html><html><body><h1>제목</h1><p>본문이다.</p></body></html>";

const FILE: OpenFile = {
  key: fileTabKey("docs/page.html"),
  path: "docs/page.html",
  kind: "text",
  content: DOC,
  baseContent: DOC,
  mtime: 0,
  dirty: false,
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

/** 툴바의 프리뷰⇄소스 토글. 문구가 곧 "누르면 갈 곳"이다. */
const clickToggle = async (label: string) => {
  const button = [...container.querySelectorAll("button")].find((b) => b.textContent === label);
  expect(button, `"${label}" 버튼을 찾지 못했다`).toBeTruthy();
  await act(async () => {
    button?.click();
  });
};

it("renders an SSH document in the sandboxed HTML preview", async () => {
  const file: OpenFile = { ...FILE, path: "/srv/reports/page.html", key: fileTabKey("/srv/reports/page.html"), readOnly: true };
  await render({ files: [file], activeKey: file.key, retainedPaths: [file.path] });
  expect(container.querySelector("iframe")?.getAttribute("srcdoc")).toBe(DOC);
  expect(container.querySelector("iframe")?.getAttribute("sandbox")).toBe("");
  expect(container.textContent).toContain("읽기 전용");
});

const clickSave = async () => {
  const button = container.querySelector<HTMLButtonElement>('button[aria-label="저장"]');
  expect(button, "저장 버튼을 찾지 못했다").toBeTruthy();
  await act(async () => {
    button?.click();
  });
};

beforeEach(() => {
  h.disposed = false;
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

describe("HTML 프리뷰", () => {
  it("연 직후엔 샌드박스 iframe으로 원문을 렌더한다", async () => {
    await render();

    const frame = container.querySelector("iframe");
    expect(frame).toBeTruthy();
    // 빈 sandbox — 스크립트·동일 출처를 절대 열어주지 않는다.
    expect(frame?.getAttribute("sandbox")).toBe("");
    expect(frame?.getAttribute("srcdoc")).toBe(DOC);
    expect(frame?.getAttribute("title")).toBe("page.html");
    expect(container.querySelector('[data-testid="monaco"]')).toBeNull();
  });

  it("소스 버튼으로 Monaco 편집과 프리뷰를 오간다", async () => {
    await render();

    await clickToggle("소스");
    expect(container.querySelector('[data-testid="monaco"]')).toBeTruthy();
    expect(container.querySelector("iframe")).toBeNull();

    await clickToggle("프리뷰");
    expect(container.querySelector("iframe")).toBeTruthy();
    expect(container.querySelector('[data-testid="monaco"]')).toBeNull();
  });

  it("프리뷰로 돌아와 저장하면 dispose된 에디터가 아니라 탭 내용을 쓴다", async () => {
    const onSave = vi.fn();
    await render({ onSave });

    await clickToggle("소스"); // Monaco 마운트 — editorRef가 채워진다.
    await clickToggle("프리뷰"); // 언마운트로 dispose되지만 ref는 옛 인스턴스를 계속 문다.
    await clickSave();

    expect(onSave).toHaveBeenCalledWith(FILE.path, DOC);
  });

  it(".htm도 프리뷰로 열리고 소스로 전환할 수 있다", async () => {
    const file: OpenFile = { ...FILE, key: fileTabKey("docs/page.htm"), path: "docs/page.htm" };
    await render({ files: [file], activeKey: file.key, retainedPaths: [file.path] });

    expect(container.querySelector("iframe")).toBeTruthy();

    await clickToggle("소스");
    expect(container.querySelector('[data-testid="monaco"]')).toBeTruthy();
  });

  it("content가 바뀌면 srcdoc이 따라간다 — 외부 변경으로 다시 읽은 경우", async () => {
    await render();

    const next = "<!doctype html><html><body><p>디스크에서 다시 읽은 본문.</p></body></html>";
    await render({ files: [{ ...FILE, content: next, baseContent: next, mtime: 1 }] });

    expect(container.querySelector("iframe")?.getAttribute("srcdoc")).toBe(next);
  });
});

describe("확장자가 아니라 내용으로 HTML을 알아보는 경우", () => {
  const TABLE = "<table><tr><td>셀</td></tr></table>";

  it(".md인데 내용이 통째로 HTML이면 iframe 프리뷰다", async () => {
    const file: OpenFile = {
      ...FILE,
      key: fileTabKey("docs/report.md"),
      path: "docs/report.md",
      content: TABLE,
      baseContent: TABLE,
    };
    await render({ files: [file], activeKey: file.key, retainedPaths: [file.path] });

    const frame = container.querySelector("iframe");
    expect(frame?.getAttribute("srcdoc")).toBe(TABLE);
    expect(frame?.getAttribute("sandbox")).toBe("");

    // 소스 버튼은 그대로 살아 있어야 한다 — HTML로 판정돼도 편집 경로는 막지 않는다.
    await clickToggle("소스");
    expect(container.querySelector('[data-testid="monaco"]')).toBeTruthy();
  });
});

describe("편집 중 판정이 뒤집히지 않는다", () => {
  it(".txt에 HTML을 다 입력해도 Monaco가 자리를 지킨다", async () => {
    const file: OpenFile = {
      ...FILE,
      key: fileTabKey("scratch.txt"),
      path: "scratch.txt",
      content: "",
      baseContent: "",
    };
    await render({ files: [file], activeKey: file.key, retainedPaths: [file.path] });
    expect(container.querySelector('[data-testid="monaco"]')).toBeTruthy();

    // 마지막 `>`까지 친 순간 판정이 뒤집히면 Monaco가 언마운트되고 포커스가 사라진다.
    const typed = { ...file, content: "<p>hi</p>", dirty: true };
    await render({ files: [typed], activeKey: typed.key, retainedPaths: [typed.path] });

    expect(container.querySelector('[data-testid="monaco"]')).toBeTruthy();
    expect(container.querySelector("iframe")).toBeNull();
  });
});
