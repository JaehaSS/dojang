// @vitest-environment jsdom

import { act, useRef, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ openUrl: vi.fn(async () => undefined) }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: h.openUrl }));

// Monaco 본체는 jsdom에서 로드되지 않는다 — 이 테스트가 보는 것은 배치와 단축키뿐이라
// 에디터 자리는 표식 하나로 대신한다.
vi.mock("@monaco-editor/react", async () => {
  const React = await import("react");
  return {
    default: ({ path }: { path: string }) =>
      React.createElement("div", { "data-testid": "monaco", "data-path": path }),
  };
});
vi.mock("../../lib/monaco", () => ({ langFromPath: () => "plaintext" }));
vi.mock("../../lib/designmode/editor-capture-target", () => ({
  registerEditorCaptureTarget: () => () => undefined,
}));

import { EditorSplitView } from "./EditorSplitView";
import { FILE_DRAG_MIME, TAB_DRAG_MIME } from "./editor-drag";
import type { OpenFile } from "./EditorPane";
import { fileTabKey, type TabKey } from "../../lib/tab-key";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** 문서마다 어떤 링크를 담을지 — 클릭 경로가 링크 종류마다 갈린다. */
const DOC_LINK: Record<string, string> = {
  "docs/url.md": "https://example.com/문서",
  "docs/bad.md": "/etc/passwd",
  "docs/abs.md": "/work/root/docs/target.md",
  "docs/fragment.md": "#소제목",
  "/Users/test/notes/README.md": "GLOSSARY.md",
  "docs/left.md": "../x.ts",
  "docs/right.md": "../y.ts",
  "docs/same-left.md": "../shared.ts",
  "docs/same-right.md": "../shared.ts",
};
const DEFAULT_LINK = "../../DESIGN%20%ED%95%9C%EA%B8%80.md:4#intro";

const file = (path: string, preview = false): OpenFile => {
  const body = path.endsWith(".md")
    ? `[경로 **링크**](${DOC_LINK[path] ?? DEFAULT_LINK})`
    : "본문";
  return {
    key: fileTabKey(path),
    path,
    kind: "text",
    content: body,
    baseContent: body,
    mtime: 0,
    dirty: false,
    preview,
  };
};

const closedGlobally: string[] = [];
/** 배치가 "이건 고정해라"라고 파일 층에 알린 키 — 손동작으로 붙든 탭이 여기 쌓인다. */
const pinnedGlobally: string[] = [];
/** 테스트가 워크스페이스 쪽에서 파일을 여는 손잡이 — `useWorkspaceFiles.openFile`에 해당. */
let openFromTree: (path: string, preview?: boolean) => Promise<boolean> = async () => false;
/** 작업 전환처럼 열린 파일이 통째로 사라지는 상황 — 분할이 접히는 경로를 태운다. */
let closeAll: () => void = () => undefined;
/** 파일 하나가 밖에서 사라지는 상황 — 떠 있던 메뉴가 유령을 가리키게 된다. */
let closeOne: (path: string) => void = () => undefined;
const navigationError = vi.fn();
const copyAbsolutePath = vi.fn();
const revealPath = vi.fn();
const writeClipboard = vi.fn(async () => undefined);

/**
 * `useWorkspaceFiles`의 계약만 흉내 낸 부모. 열린 목록과 활성 경로를 소유하고, 분할 뷰가
 * 돌려주는 선택·닫기를 그대로 반영한다 — 실제 앱에서 sync가 도는 경로를 그대로 태운다.
 */
function Harness({
  initial,
  shortcutsActive = true,
  ownsWindow = false,
  supportsExternalPath,
  onCloseExhausted,
  onOpenFile,
  rootPath,
}: {
  initial: Array<string | OpenFile>;
  shortcutsActive?: boolean;
  ownsWindow?: boolean;
  supportsExternalPath?: boolean;
  onCloseExhausted?: () => void;
  onOpenFile?: (path: string, complete: () => void) => Promise<boolean>;
  rootPath?: string | null;
}) {
  const [files, setFiles] = useState<OpenFile[]>(() =>
    initial.map((item) => (typeof item === "string" ? file(item) : item)),
  );
  const [activeKey, setActiveKey] = useState<TabKey | null>(
    files.length > 0 ? files[files.length - 1].key : null,
  );
  const [treeOpen, setTreeOpen] = useState<{ key: TabKey; request: number } | null>(null);
  const treeRequest = useRef(0);
  openFromTree = (path: string, preview = false) => {
    setFiles((current) =>
      current.some((f) => f.path === path) ? current : [...current, file(path, preview)],
    );
    const key = fileTabKey(path);
    setActiveKey(key);
    setTreeOpen({ key, request: ++treeRequest.current });
    return Promise.resolve(true);
  };
  const openFile = (path: string) => {
    const complete = () => {
      setFiles((current) => current.some((f) => f.path === path) ? current : [...current, file(path)]);
      setActiveKey(fileTabKey(path));
    };
    if (onOpenFile != null) return onOpenFile(path, complete);
    complete();
    return Promise.resolve(true);
  };
  closeAll = () => {
    setFiles([]);
    setActiveKey(null);
  };
  closeOne = (path: string) => {
    setFiles((current) => current.filter((f) => f.path !== path));
    setActiveKey((current) => (current === fileTabKey(path) ? null : current));
  };
  return (
    <EditorSplitView
      taskId={1}
      host="local"
      windowId="test"
      rootPath={rootPath}
      files={files}
      activeKey={activeKey}
      treeOpen={treeOpen}
      onTreeOpenHandled={(request) =>
        setTreeOpen((current) => current?.request === request ? null : current)
      }
      dark={false}
      shortcutsActive={shortcutsActive}
      ownsWindow={ownsWindow}
      onCloseExhausted={onCloseExhausted}
      onSelect={setActiveKey}
      onOpenFile={openFile}
      onNavigationError={navigationError}
      onPinTab={(key) => {
        pinnedGlobally.push(key);
        setFiles((current) => current.map((f) => (f.key === key ? { ...f, preview: false } : f)));
      }}
      onClose={(key) => {
        closedGlobally.push(key);
        setFiles((current) => current.filter((f) => f.key !== key));
        setActiveKey((current) => (current === key ? null : current));
      }}
      onChange={() => undefined}
      onSave={() => undefined}
      onReload={() => undefined}
      onOpenPath={() => undefined}
      onRevealPath={revealPath}
      onCopyAbsPath={copyAbsolutePath}
      supportsExternalPath={supportsExternalPath}
    />
  );
}

let container: HTMLDivElement;
let root: Root;

const render = async (props: Parameters<typeof Harness>[0]) => {
  await act(async () => {
    root.render(<Harness {...props} />);
  });
};

/** 칸 = 포커스 표식을 단 컨테이너. 순서가 곧 화면 순서다. */
const panes = () => Array.from(container.querySelectorAll<HTMLElement>("[data-focused]"));
const focusedPane = () => panes().find((p) => p.dataset.focused === "true") ?? null;
/** 탭 title의 첫 줄 = 경로. 둘째 줄에는 ⌘번호 힌트가 붙는다. */
const tabPath = (el: Element) => (el.getAttribute("title") ?? "").split("\n")[0];
/** 한 칸의 탭 제목 — 탭 div가 전체 경로를 title 첫 줄로 달고 있다. */
const tabsOf = (pane: HTMLElement) =>
  Array.from(pane.querySelectorAll<HTMLElement>("[title]"))
    .map(tabPath)
    .filter((t) => t.includes("."));

const click = async (el: Element | null | undefined) => {
  expect(el, "클릭할 요소를 찾지 못했다").toBeTruthy();
  await act(async () => {
    el?.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    el?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
};

const splitButton = (pane: HTMLElement, label: string) =>
  pane.querySelector(`button[aria-label="${label}"]`);

const press = async (init: KeyboardEventInit): Promise<KeyboardEvent> => {
  const event = new KeyboardEvent("keydown", { cancelable: true, bubbles: true, ...init });
  await act(async () => {
    document.body.dispatchEvent(event);
  });
  return event;
};

let animationFrame = 0;
let animationFrames = new Map<number, FrameRequestCallback>();

const flushAnimationFrame = async () => {
  const callbacks = Array.from(animationFrames.values());
  animationFrames.clear();
  await act(async () => callbacks.forEach((callback) => callback(0)));
};

beforeEach(() => {
  closedGlobally.length = 0;
  pinnedGlobally.length = 0;
  copyAbsolutePath.mockClear();
  navigationError.mockClear();
  h.openUrl.mockClear();
  revealPath.mockClear();
  writeClipboard.mockClear();
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText: writeClipboard },
  });
  animationFrame = 0;
  animationFrames = new Map();
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    const id = ++animationFrame;
    animationFrames.set(id, callback);
    return id;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => animationFrames.delete(id));
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

describe("분할", () => {
  it("새 칸이 보던 파일을 들고 포커스를 가져간다", async () => {
    await render({ initial: ["a.ts", "b.ts"] });
    await click(splitButton(panes()[0], "오른쪽으로 분할"));

    expect(panes()).toHaveLength(2);
    expect(tabsOf(panes()[1])).toEqual(["b.ts"]);
    expect(focusedPane()).toBe(panes()[1]);
  });

  it("분할 뒤 트리에서 연 파일은 포커스된 칸으로 들어간다", async () => {
    await render({ initial: ["a.ts"] });
    await click(splitButton(panes()[0], "오른쪽으로 분할"));
    await act(async () => openFromTree("c.ts"));

    expect(tabsOf(panes()[0])).toEqual(["a.ts"]);
    expect(tabsOf(panes()[1])).toEqual(["a.ts", "c.ts"]);
  });

  it("⌘\\는 오른쪽, ⇧⌘\\는 아래로 나눈다", async () => {
    await render({ initial: ["a.ts"] });
    await press({ code: "Backslash", metaKey: true });
    expect(container.querySelector('[role="separator"]')?.getAttribute("aria-orientation")).toBe(
      "vertical",
    );

    await press({ code: "Backslash", metaKey: true, shiftKey: true });
    expect(container.querySelector('[role="separator"]')?.getAttribute("aria-orientation")).toBe(
      "horizontal",
    );
  });

  it("칸을 접으면 들고 있던 탭이 옆으로 넘어간다 — 파일은 닫히지 않는다", async () => {
    await render({ initial: ["a.ts"] });
    await click(splitButton(panes()[0], "오른쪽으로 분할"));
    await act(async () => openFromTree("c.ts"));
    await click(panes()[1].querySelector('button[aria-label="편집 칸 접기"]'));

    expect(panes()).toHaveLength(1);
    expect(tabsOf(panes()[0])).toEqual(["a.ts", "c.ts"]);
    expect(closedGlobally).toEqual([]);
  });

  it("칸이 하나면 접기 버튼을 두지 않는다", async () => {
    await render({ initial: ["a.ts"] });
    expect(panes()[0].querySelector('button[aria-label="편집 칸 접기"]')).toBeNull();
  });
});

describe("⌘W", () => {
  it.each([{ altKey: true, key: "∑" }, { metaKey: true }])("HTML iframe에 포커스한 뒤에도 파일을 닫는다: %j", async (modifiers) => {
    await render({ initial: ["a.ts", "page.html"] });
    const frame = container.querySelector("iframe")!;
    expect(frame.getAttribute("sandbox")).toBe("");
    await act(async () => {
      frame.focus();
      window.dispatchEvent(new Event("blur"));
      await new Promise((resolve) => setTimeout(resolve, 10));
    });
    expect(document.activeElement).toBe(frame.parentElement);
    const event = await press({ code: "KeyW", ...modifiers });
    expect(event.defaultPrevented).toBe(true);
    expect(closedGlobally).toEqual(["page.html"]);
    expect(tabsOf(panes()[0])).toEqual(["a.ts"]);
  });

  it("비활성 화면에서는 Option+W를 가로채지 않는다", async () => {
    await render({ initial: ["page.html"], shortcutsActive: false });
    const event = await press({ code: "KeyW", altKey: true });
    expect(event.defaultPrevented).toBe(false);
    expect(closedGlobally).toEqual([]);
  });

  it("포커스된 칸의 활성 파일만 닫는다", async () => {
    await render({ initial: ["a.ts", "b.ts"] });
    const event = await press({ code: "KeyW", metaKey: true });

    expect(event.defaultPrevented).toBe(true);
    expect(closedGlobally).toEqual(["b.ts"]);
    expect(tabsOf(panes()[0])).toEqual(["a.ts"]);
  });

  it("다른 칸이 같은 파일을 띄우고 있으면 전역에서는 닫지 않는다", async () => {
    await render({ initial: ["a.ts", "b.ts"] });
    await click(splitButton(panes()[0], "오른쪽으로 분할"));
    await press({ code: "KeyW", metaKey: true });

    // 오른쪽 칸은 b.ts 하나뿐이었으므로 칸째 사라지고, 파일은 왼쪽에 남는다.
    expect(panes()).toHaveLength(1);
    expect(closedGlobally).toEqual([]);
    expect(tabsOf(panes()[0])).toEqual(["a.ts", "b.ts"]);
  });

  it("닫을 파일이 없으면 이벤트를 흘려보낸다", async () => {
    await render({ initial: [] });
    const event = await press({ code: "KeyW", metaKey: true });
    expect(event.defaultPrevented).toBe(false);
  });

  it("닫을 파일이 없고 폴백이 있으면 그쪽을 부른다", async () => {
    const onCloseExhausted = vi.fn();
    await render({ initial: [], onCloseExhausted });
    const event = await press({ code: "KeyW", metaKey: true });

    expect(onCloseExhausted).toHaveBeenCalledOnce();
    expect(event.defaultPrevented).toBe(true);
  });

  it("이 화면이 단축키를 듣지 않으면 가로채지 않는다", async () => {
    await render({ initial: ["a.ts"], shortcutsActive: false });
    const event = await press({ code: "KeyW", metaKey: true });

    expect(event.defaultPrevented).toBe(false);
    expect(closedGlobally).toEqual([]);
  });
});

/**
 * jsdom에는 DataTransfer도 레이아웃도 없다. 드래그와 크기 조절은 둘 다 그 둘을 쓰므로
 * 여기서 최소한만 흉내 낸다 — 값을 나르는 통과 사각형 하나.
 */
class FakeDataTransfer {
  private store = new Map<string, string>();
  dropEffect = "none";
  effectAllowed = "uninitialized";
  get types(): string[] {
    return Array.from(this.store.keys());
  }
  setData(type: string, value: string): void {
    this.store.set(type, value);
  }
  getData(type: string): string {
    return this.store.get(type) ?? "";
  }
  /**
   * 받는 쪽이 정한 dropEffect가 끄는 쪽의 effectAllowed 안에 드는가 — HTML 드래그 협상표다.
   *
   * 어긋나면 브라우저는 드래그 동작을 none으로 확정하고 **drop 이벤트를 보내지 않는다**.
   * jsdom은 이 판정을 하지 않으므로, 흉내도 하지 않으면 실제로는 죽어 있는 드롭이 테스트에서는
   * 멀쩡히 통과한다 — 트리에서 끌어 놓는 분할이 그 형태로 처음부터 동작하지 않았다.
   */
  get negotiated(): boolean {
    const allowed: Record<string, readonly string[]> = {
      uninitialized: ["copy", "link", "move"],
      all: ["copy", "link", "move"],
      none: [],
      copy: ["copy"],
      link: ["link"],
      move: ["move"],
      copyLink: ["copy", "link"],
      copyMove: ["copy", "move"],
      linkMove: ["link", "move"],
    };
    return (allowed[this.effectAllowed] ?? []).includes(this.dropEffect);
  }
}

const PANE = { width: 1000, height: 600 };

const dragEvent = (type: string, data: FakeDataTransfer, x = 0, y = 0): Event => {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y });
  Object.defineProperty(event, "dataTransfer", { value: data });
  return event;
};

let restoreRect: (() => void) | null = null;

/** 탭 하나의 폭 — 실제로는 파일명에 따라 다르지만 여기서는 자리 판정을 결정적으로 만든다. */
const TAB_W = 100;

const rectOf = (left: number, width: number, height: number): DOMRect =>
  ({
    left,
    top: 0,
    width,
    height,
    right: left + width,
    bottom: height,
    x: left,
    y: 0,
    toJSON: () => ({}),
  }) as DOMRect;

/** 모든 요소가 같은 사각형을 갖는다 — 가장자리 판정과 픽셀 환산이 결정적이 된다.
 *  탭만은 예외로 폭 TAB_W씩 늘어선다. 전부 같은 사각형이면 "몇 번째 탭 사이"를 물을 수 없다. */
const stubLayout = () => {
  const original = HTMLElement.prototype.getBoundingClientRect;
  HTMLElement.prototype.getBoundingClientRect = function rect(this: HTMLElement): DOMRect {
    if (this.dataset?.tabPath != null) {
      const siblings = Array.from(
        this.parentElement?.querySelectorAll<HTMLElement>("[data-tab-path]") ?? [],
      );
      return rectOf(Math.max(0, siblings.indexOf(this)) * TAB_W, TAB_W, 32);
    }
    return {
      left: 0,
      top: 0,
      width: PANE.width,
      height: PANE.height,
      right: PANE.width,
      bottom: PANE.height,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    } as DOMRect;
  };
  restoreRect = () => {
    HTMLElement.prototype.getBoundingClientRect = original;
  };
};

afterEach(() => {
  restoreRect?.();
  restoreRect = null;
});

const dropSurface = (pane: HTMLElement) => pane.querySelector<HTMLElement>("[data-drop-surface]");

const grows = () => panes().map((p) => p.style.flexGrow);

describe("⌘1‥⌘9", () => {
  /** 그 칸이 지금 띄우고 있는 파일 — Monaco 자리 표식이 경로를 들고 있다. */
  const shownIn = (pane: HTMLElement) =>
    pane.querySelector('[data-testid="monaco"]')?.getAttribute("data-path") ?? null;

  it("포커스가 에디터 안이면 그 칸의 N번째 탭으로 간다", async () => {
    await render({ initial: ["a.ts", "b.ts", "c.ts"] });
    // 탭 자체는 포커스를 받지 않는다 — 칸 안의 아무 버튼이나 잡으면 조건은 같다.
    act(() => panes()[0].querySelector("button")?.focus());

    const first = await press({ code: "Digit1", metaKey: true });
    expect(first.defaultPrevented).toBe(true);
    expect(shownIn(panes()[0])).toBe("a.ts");

    await press({ code: "Digit2", metaKey: true });
    expect(shownIn(panes()[0])).toBe("b.ts");
  });

  it("포커스가 에디터 밖이면 건드리지 않는다 — 세션 이동의 몫이다", async () => {
    await render({ initial: ["a.ts", "b.ts"] });
    const outside = document.createElement("button");
    document.body.appendChild(outside);
    act(() => outside.focus());

    const event = await press({ code: "Digit1", metaKey: true });

    expect(event.defaultPrevented).toBe(false);
    expect(shownIn(panes()[0])).toBe("b.ts");
    outside.remove();
  });

  it("창이 통째로 에디터면 포커스가 없어도 받는다", async () => {
    await render({ initial: ["a.ts", "b.ts"], ownsWindow: true });
    expect(document.activeElement).toBe(document.body);

    const event = await press({ code: "Digit1", metaKey: true });

    expect(event.defaultPrevented).toBe(true);
    expect(shownIn(panes()[0])).toBe("a.ts");
  });

  it("없는 번호는 삼키지 않는다", async () => {
    await render({ initial: ["a.ts", "b.ts"], ownsWindow: true });
    const event = await press({ code: "Digit5", metaKey: true });

    expect(event.defaultPrevented).toBe(false);
    expect(shownIn(panes()[0])).toBe("b.ts");
  });

  it("번호는 칸마다 따로 센다 — 포커스된 칸의 탭 순서다", async () => {
    await render({ initial: ["a.ts", "b.ts"], ownsWindow: true });
    await click(splitButton(panes()[0], "오른쪽으로 분할"));
    await act(async () => openFromTree("c.ts"));
    // 오른쪽 칸은 [b.ts, c.ts]를 들고 있다 — 전역 목록(a·b·c)의 2번은 b.ts지만
    // 이 칸의 2번은 c.ts다. 번호를 어디서 세는지가 여기서 갈린다.
    expect(tabsOf(panes()[1])).toEqual(["b.ts", "c.ts"]);

    await press({ code: "Digit2", metaKey: true });

    expect(shownIn(panes()[1])).toBe("c.ts");
    // 옆 칸은 건드리지 않는다 — 분할의 의미가 각 칸이 자기 파일을 지키는 것이다.
    expect(shownIn(panes()[0])).toBe("b.ts");
  });

  it("파일 탭을 보고 있지 않으면 듣지 않는다", async () => {
    await render({ initial: ["a.ts", "b.ts"], ownsWindow: true, shortcutsActive: false });
    const event = await press({ code: "Digit1", metaKey: true });

    expect(event.defaultPrevented).toBe(false);
    expect(shownIn(panes()[0])).toBe("b.ts");
  });
});

describe("탭 순서 바꾸기", () => {
  /** 같은 칸의 탭 바에 떨군다 — 지정한 x가 곧 삽입 자리다. */
  const dropOnStrip = async (pane: HTMLElement, data: FakeDataTransfer, x: number) => {
    const strip = pane.querySelector<HTMLElement>("[data-drop-strip]");
    expect(strip, "탭 바 드롭 띠가 없다").toBeTruthy();
    await act(async () => {
      strip?.dispatchEvent(dragEvent("dragover", data, x, 10));
    });
    // 협상이 깨졌으면 브라우저는 여기서 멈춘다 — 흉내도 같아야 회귀가 테스트에 잡힌다.
    if (!data.negotiated) return;
    await act(async () => {
      strip?.dispatchEvent(dragEvent("drop", data, x, 10));
    });
  };

  const grabTab = async (pane: HTMLElement, title: string) => {
    const tab = Array.from(pane.querySelectorAll<HTMLElement>("[data-tab-path]")).find(
      (el) => el.dataset.tabPath === title,
    );
    expect(tab, `탭 ${title}을 찾지 못했다`).toBeTruthy();
    const data = new FakeDataTransfer();
    await act(async () => {
      tab?.dispatchEvent(dragEvent("dragstart", data));
    });
    await flushAnimationFrame();
    return data;
  };

  it("탭을 앞쪽으로 끌면 그 자리에 꽂힌다 — 칸은 그대로 하나다", async () => {
    await render({ initial: ["a.ts", "b.ts", "c.ts"] });
    stubLayout();
    const data = await grabTab(panes()[0], "c.ts");

    await dropOnStrip(panes()[0], data, 10); // a.ts의 왼쪽 절반 = 맨 앞

    expect(panes()).toHaveLength(1);
    expect(tabsOf(panes()[0])).toEqual(["c.ts", "a.ts", "b.ts"]);
  });

  it("드래그 중에는 어디에 꽂힐지 표시한다", async () => {
    await render({ initial: ["a.ts", "b.ts"] });
    stubLayout();
    const data = await grabTab(panes()[0], "b.ts");
    const strip = panes()[0].querySelector<HTMLElement>("[data-drop-strip]");
    await act(async () => {
      strip?.dispatchEvent(dragEvent("dragover", data, 10, 10));
    });

    expect(strip?.querySelector("[data-drop-caret]")?.getAttribute("data-drop-caret")).toBe("0");
  });

  it("순서를 바꿔도 파일은 닫히지 않는다", async () => {
    await render({ initial: ["a.ts", "b.ts"] });
    stubLayout();
    const data = await grabTab(panes()[0], "b.ts");
    await dropOnStrip(panes()[0], data, 10);

    expect(closedGlobally).toEqual([]);
  });
});

describe("⌘⇧[ · ⌘⇧]", () => {
  const shownIn = (pane: HTMLElement) =>
    pane.querySelector('[data-testid="monaco"]')?.getAttribute("data-path") ?? null;

  it("다음·이전 탭으로 순서대로 간다", async () => {
    await render({ initial: ["a.ts", "b.ts", "c.ts"] });
    expect(shownIn(panes()[0])).toBe("c.ts");

    await press({ code: "BracketLeft", metaKey: true, shiftKey: true });
    expect(shownIn(panes()[0])).toBe("b.ts");

    await press({ code: "BracketRight", metaKey: true, shiftKey: true });
    expect(shownIn(panes()[0])).toBe("c.ts");
  });

  it("끝에서 한 바퀴 돈다", async () => {
    await render({ initial: ["a.ts", "b.ts"] });
    // b.ts(마지막)에서 다음으로 가면 처음으로 돌아온다.
    const event = await press({ code: "BracketRight", metaKey: true, shiftKey: true });
    expect(event.defaultPrevented).toBe(true);
    expect(shownIn(panes()[0])).toBe("a.ts");
  });

  it("탭이 하나뿐이면 아무 일도 없다", async () => {
    await render({ initial: ["a.ts"] });
    const event = await press({ code: "BracketRight", metaKey: true, shiftKey: true });
    expect(event.defaultPrevented).toBe(false);
  });

  it("Shift 없이는 듣지 않는다 — ⌘[는 다른 손짓이다", async () => {
    await render({ initial: ["a.ts", "b.ts"] });
    const event = await press({ code: "BracketLeft", metaKey: true });
    expect(event.defaultPrevented).toBe(false);
    expect(shownIn(panes()[0])).toBe("b.ts");
  });
});

describe("탭 우클릭 메뉴", () => {
  const openMenu = async (pane: HTMLElement, path: string) => {
    const tab = Array.from(pane.querySelectorAll<HTMLElement>("[data-tab-path]")).find(
      (el) => el.dataset.tabPath === path,
    );
    expect(tab, `탭 ${path}을 찾지 못했다`).toBeTruthy();
    await act(async () => {
      tab?.dispatchEvent(
        new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 20, clientY: 20 }),
      );
    });
    return container.ownerDocument.querySelector('[role="menu"]');
  };

  const itemNamed = (menu: Element | null, label: string) =>
    Array.from(menu?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []).find(
      (el) => el.textContent === label,
    );

  it("다른 탭 모두 닫기 — 가리킨 탭만 남는다", async () => {
    await render({ initial: ["a.ts", "b.ts", "c.ts"] });
    const menu = await openMenu(panes()[0], "a.ts");
    await click(itemNamed(menu, "다른 탭 모두 닫기"));

    expect(tabsOf(panes()[0])).toEqual(["a.ts"]);
    expect(closedGlobally.sort()).toEqual(["b.ts", "c.ts"]);
  });

  it("오른쪽 탭 모두 닫기 — 왼쪽은 건드리지 않는다", async () => {
    await render({ initial: ["a.ts", "b.ts", "c.ts"] });
    const menu = await openMenu(panes()[0], "b.ts");
    await click(itemNamed(menu, "오른쪽 탭 모두 닫기"));

    expect(tabsOf(panes()[0])).toEqual(["a.ts", "b.ts"]);
    expect(closedGlobally).toEqual(["c.ts"]);
  });

  it("맨 오른쪽 탭에서는 오른쪽 닫기가 비활성이다", async () => {
    await render({ initial: ["a.ts", "b.ts"] });
    const menu = await openMenu(panes()[0], "b.ts");
    expect(itemNamed(menu, "오른쪽 탭 모두 닫기")?.disabled).toBe(true);
  });

  it("활성이 아닌 탭에서 분할하면 **그 탭**이 새 칸에 열린다", async () => {
    await render({ initial: ["a.ts", "b.ts"] });
    // 활성은 b.ts인데 메뉴는 a.ts를 가리킨다 — 새 칸에 떠야 하는 것은 a.ts다.
    const menu = await openMenu(panes()[0], "a.ts");
    await click(itemNamed(menu, "오른쪽으로 분할"));

    expect(panes()).toHaveLength(2);
    expect(tabsOf(panes()[1])).toEqual(["a.ts"]);
  });

  it("메뉴에서 분할하면 칸이 늘어난다", async () => {
    await render({ initial: ["a.ts"] });
    const menu = await openMenu(panes()[0], "a.ts");
    await click(itemNamed(menu, "아래로 분할"));

    expect(panes()).toHaveLength(2);
    expect(container.querySelector('[role="separator"]')?.getAttribute("aria-orientation")).toBe(
      "horizontal",
    );
  });

  it("메뉴가 떠 있는 사이 그 탭이 사라지면 아무것도 닫지 않는다", async () => {
    await render({ initial: ["a.ts", "b.ts", "c.ts"] });
    const menu = await openMenu(panes()[0], "a.ts");
    // 그 탭만 밖에서 사라진다 — 남은 탭들은 그대로다.
    await act(async () => closeOne("a.ts"));
    expect(tabsOf(panes()[0])).toEqual(["b.ts", "c.ts"]);

    // "오른쪽 닫기"는 비활성으로 막히지만("hasRight"), "다른 탭 닫기"에는 그 판정이 없다 —
    // 가리키던 탭이 목록에 없으면 filter가 **남은 전부**를 지운다.
    await click(itemNamed(menu, "다른 탭 모두 닫기"));

    expect(closedGlobally).toEqual([]);
    expect(tabsOf(panes()[0])).toEqual(["b.ts", "c.ts"]);
  });

  it("원격 워크트리에서는 Finder 항목을 두지 않는다", async () => {
    await render({ initial: ["a.ts"] });
    const menu = await openMenu(panes()[0], "a.ts");
    expect(itemNamed(menu, "Finder에서 보기")).toBeUndefined();
  });
});

describe("경계 끌기", () => {
  it("마우스로 끌면 양쪽 비중이 함께 움직인다", async () => {
    await render({ initial: ["a.ts", "b.ts"] });
    await click(splitButton(panes()[0], "오른쪽으로 분할"));
    stubLayout();
    expect(grows()).toEqual(["1", "1"]);

    const separator = container.querySelector('[role="separator"]') as HTMLElement;
    await act(async () => {
      separator.dispatchEvent(
        new MouseEvent("pointerdown", { bubbles: true, cancelable: true, clientX: 500, clientY: 300 }),
      );
    });
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 750, clientY: 300 }));
    });

    // 1000px에 1+1 → 1비중당 500px. 250px 밀었으므로 0.5비중이 옮겨 간다.
    const [left, right] = grows().map(Number);
    expect(left).toBeCloseTo(1.5);
    expect(right).toBeCloseTo(0.5);
    // 총합은 그대로 — 남는 여백이 생기지 않는다.
    expect(left + right).toBeCloseTo(2);

    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointerup", {}));
    });
    // 놓으면 문서에 걸어 둔 커서·선택 잠금을 되돌린다.
    expect(document.body.style.cursor).toBe("");
    expect(document.body.style.userSelect).toBe("");
  });

  it("더블클릭하면 균등으로 되돌아간다", async () => {
    await render({ initial: ["a.ts", "b.ts"] });
    await click(splitButton(panes()[0], "오른쪽으로 분할"));
    stubLayout();
    const separator = container.querySelector('[role="separator"]') as HTMLElement;
    await act(async () => {
      separator.dispatchEvent(
        new MouseEvent("pointerdown", { bubbles: true, cancelable: true, clientX: 500, clientY: 300 }),
      );
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 750, clientY: 300 }));
      window.dispatchEvent(new MouseEvent("pointerup", {}));
    });
    expect(grows()).not.toEqual(["1", "1"]);

    await act(async () => {
      separator.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    });
    expect(grows()).toEqual(["1", "1"]);
  });
});

describe("칸 크기 승계", () => {
  it("파일을 전부 닫았다 다시 열면 칸이 화면을 온전히 채운다", async () => {
    await render({ initial: ["a.ts"] });
    await click(splitButton(panes()[0], "오른쪽으로 분할"));
    await click(splitButton(panes()[1], "오른쪽으로 분할"));
    expect(panes()).toHaveLength(3);

    await act(async () => closeAll());
    expect(panes()).toHaveLength(1);

    await act(async () => openFromTree("새.ts"));
    // 옛 분할의 1/4을 물려받으면 여기가 "0.25"가 된다 — 화면의 4분의 1만 쓰는 에디터.
    expect(grows()).toEqual(["1"]);
  });
});

describe("끌어다 놓아 분할", () => {
  /** 탭을 집어 든다 — 실제 브라우저처럼 dragstart가 창까지 버블한다. */
  const startTabDrag = async (pane: HTMLElement, title: string) => {
    const tab = Array.from(pane.querySelectorAll<HTMLElement>("[title]")).find(
      (el) => tabPath(el) === title,
    );
    expect(tab, `탭 ${title}을 찾지 못했다`).toBeTruthy();
    const data = new FakeDataTransfer();
    await act(async () => {
      tab?.dispatchEvent(dragEvent("dragstart", data));
    });
    await flushAnimationFrame();
    return data;
  };

  const dropOn = async (pane: HTMLElement, data: FakeDataTransfer, x: number, y: number) => {
    const surface = dropSurface(pane);
    expect(surface, "드롭 면이 깔리지 않았다").toBeTruthy();
    await act(async () => {
      surface?.dispatchEvent(dragEvent("dragover", data, x, y));
    });
    if (!data.negotiated) return;
    await act(async () => {
      surface?.dispatchEvent(dragEvent("drop", data, x, y));
    });
  };

  it("탭을 오른쪽 가장자리에 놓으면 그 자리에 칸이 생긴다", async () => {
    await render({ initial: ["a.ts", "b.ts"] });
    stubLayout();
    const data = await startTabDrag(panes()[0], "a.ts");
    expect(data.getData(TAB_DRAG_MIME)).toContain("a.ts");

    await dropOn(panes()[0], data, PANE.width - 10, PANE.height / 2);

    expect(panes()).toHaveLength(2);
    // 복제가 아니라 이동이다 — 원본 칸에 a.ts가 남으면 "옮겼는데 그대로"가 된다.
    expect(tabsOf(panes()[0])).toEqual(["b.ts"]);
    expect(tabsOf(panes()[1])).toEqual(["a.ts"]);
    expect(focusedPane()).toBe(panes()[1]);
  });

  it("아래 가장자리에 놓으면 배치가 세로로 돈다", async () => {
    await render({ initial: ["a.ts", "b.ts"] });
    stubLayout();
    const data = await startTabDrag(panes()[0], "a.ts");
    await dropOn(panes()[0], data, PANE.width / 2, PANE.height - 10);

    expect(container.querySelector('[role="separator"]')?.getAttribute("aria-orientation")).toBe(
      "horizontal",
    );
    expect(tabsOf(panes()[1])).toEqual(["a.ts"]);
  });

  it("가운데에 놓으면 나누지 않고 그 칸으로 옮긴다", async () => {
    await render({ initial: ["a.ts", "b.ts"] });
    await click(splitButton(panes()[0], "오른쪽으로 분할"));
    stubLayout();
    const data = await startTabDrag(panes()[0], "a.ts");
    await dropOn(panes()[1], data, PANE.width / 2, PANE.height / 2);

    expect(panes()).toHaveLength(2);
    expect(tabsOf(panes()[0])).toEqual(["b.ts"]);
    expect(tabsOf(panes()[1])).toEqual(["b.ts", "a.ts"]);
  });

  it("다른 칸의 탭 바에 놓으면 나누지 않고 그 칸에 꽂힌다", async () => {
    await render({ initial: ["a.ts", "b.ts"] });
    await click(splitButton(panes()[0], "오른쪽으로 분할"));
    stubLayout();
    const data = await startTabDrag(panes()[0], "a.ts");

    // 탭 바는 32px 띠다 — 좌표로 재면 "위쪽 가장자리"가 되어 새 칸이 생겨 버린다.
    const strip = panes()[1].querySelector<HTMLElement>("[data-drop-strip]");
    expect(strip, "탭 바 드롭 띠가 없다").toBeTruthy();
    // b.ts 탭의 오른쪽 절반 — 그 뒤에 꽂힌다.
    await act(async () => {
      strip?.dispatchEvent(dragEvent("dragover", data, 90, 10));
    });
    await act(async () => {
      strip?.dispatchEvent(dragEvent("drop", data, 90, 10));
    });

    expect(panes()).toHaveLength(2);
    expect(tabsOf(panes()[1])).toEqual(["b.ts", "a.ts"]);
  });

  it("끌어 옮긴 프리뷰 탭은 고정된다 — 옮겼다는 것 자체가 붙들겠다는 뜻이다", async () => {
    await render({ initial: ["a.ts", file("b.ts", true)] });
    stubLayout();
    const data = await startTabDrag(panes()[0], "b.ts");

    await dropOn(panes()[0], data, PANE.width - 10, PANE.height / 2);

    expect(pinnedGlobally).toEqual([fileTabKey("b.ts")]);
    expect(tabsOf(panes()[1])).toEqual(["b.ts"]);
  });

  it("같은 칸 안의 순서 바꾸기는 고정이 아니다", async () => {
    await render({ initial: ["a.ts", file("b.ts", true)] });
    stubLayout();
    const data = await startTabDrag(panes()[0], "b.ts");

    const strip = panes()[0].querySelector<HTMLElement>("[data-drop-strip]");
    await act(async () => {
      strip?.dispatchEvent(dragEvent("dragover", data, 10, 10));
    });
    await act(async () => {
      strip?.dispatchEvent(dragEvent("drop", data, 10, 10));
    });

    expect(tabsOf(panes()[0])).toEqual(["b.ts", "a.ts"]);
    expect(pinnedGlobally).toEqual([]);
  });

  it("드래그 중이 아니면 드롭 면을 깔지 않는다 — Monaco의 드래그를 가리지 않게", async () => {
    await render({ initial: ["a.ts"] });
    expect(dropSurface(panes()[0])).toBeNull();
  });

  it("받는 면은 드래그 시작 다음 프레임에 열린다", async () => {
    await render({ initial: ["a.ts", "b.ts"] });
    const tab = panes()[0].querySelector<HTMLElement>('[data-tab-path="a.ts"]');
    await act(async () => {
      tab?.dispatchEvent(dragEvent("dragstart", new FakeDataTransfer()));
    });

    expect(dropSurface(panes()[0])).toBeNull();
    expect(animationFrames.size).toBe(1);

    await flushAnimationFrame();

    expect(dropSurface(panes()[0])).toBeTruthy();
  });

  it("다음 프레임 전 취소하면 받는 면을 열지 않는다", async () => {
    await render({ initial: ["a.ts", "b.ts"] });
    const tab = panes()[0].querySelector<HTMLElement>('[data-tab-path="a.ts"]');
    const data = new FakeDataTransfer();
    await act(async () => {
      tab?.dispatchEvent(dragEvent("dragstart", data));
    });
    expect(animationFrames.size).toBe(1);
    await act(async () => {
      tab?.dispatchEvent(dragEvent("dragend", data));
    });

    await flushAnimationFrame();

    expect(dropSurface(panes()[0])).toBeNull();
  });

  it("언마운트는 대기 중인 드래그 프레임을 취소한다", async () => {
    await render({ initial: ["a.ts"] });
    const tab = panes()[0].querySelector<HTMLElement>('[data-tab-path="a.ts"]');
    await act(async () => {
      tab?.dispatchEvent(dragEvent("dragstart", new FakeDataTransfer()));
    });
    expect(animationFrames.size).toBe(1);
    await act(async () => {
      root.unmount();
    });

    expect(animationFrames.size).toBe(0);
  });

  it("파일 트리에서 끌어 온 파일은 열린 뒤 그 자리로 들어간다", async () => {
    await render({ initial: ["a.ts"] });
    stubLayout();
    const data = new FakeDataTransfer();
    data.setData(FILE_DRAG_MIME, "트리.ts");
    // 트리 행은 끌어도 제자리에 남으므로 copy다(`FileTree`가 정하는 값 그대로).
    data.effectAllowed = "copy";
    await act(async () => {
      container.dispatchEvent(dragEvent("dragstart", data));
    });
    await dropOn(panes()[0], data, PANE.width - 10, PANE.height / 2);

    expect(panes()).toHaveLength(2);
    expect(tabsOf(panes()[0])).toEqual(["a.ts"]);
    expect(tabsOf(panes()[1])).toEqual(["트리.ts"]);
  });
});

describe("문서 링크 우클릭 메뉴", () => {
  const openMenu = async () => {
    const nested = container.querySelector("a strong");
    expect(nested, "문서 링크를 찾지 못했다").toBeTruthy();
    const event = new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: 20,
      clientY: 30,
    });
    await act(async () => nested?.dispatchEvent(event));
    expect(event.defaultPrevented).toBe(true);
    return container.querySelector('[role="menu"]');
  };

  const item = (menu: Element | null, label: string) =>
    Array.from(menu?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []).find(
      (button) => button.textContent === label,
    );

  it("우클릭한 문서를 기준으로 경로·절대 경로·Finder 동작을 보낸다", async () => {
    await render({ initial: ["docs/discovery/example.md"], ownsWindow: true, supportsExternalPath: true });

    await click(item(await openMenu(), "경로 복사"));
    expect(writeClipboard).toHaveBeenCalledWith("DESIGN 한글.md");

    await click(item(await openMenu(), "절대 경로 복사"));
    expect(copyAbsolutePath).toHaveBeenCalledWith("DESIGN 한글.md");

    await click(item(await openMenu(), "Finder에서 보기"));
    expect(revealPath).toHaveBeenCalledWith("DESIGN 한글.md");
  });

  it("원격에는 OS 파일 동작을 내지 않는다", async () => {
    await render({ initial: ["docs/discovery/example.md"], supportsExternalPath: false });

    const labels = Array.from((await openMenu())?.querySelectorAll('[role="menuitem"]') ?? []).map(
      (button) => button.textContent,
    );
    expect(labels).toContain("경로 복사");
    expect(labels).not.toContain("절대 경로 복사");
    expect(labels).not.toContain("Finder에서 보기");
  });

  it("원본 문서 탭이 닫히면 메뉴도 닫는다", async () => {
    await render({ initial: ["docs/discovery/example.md"] });
    await openMenu();

    await act(async () => closeOne("docs/discovery/example.md"));

    expect(container.querySelector('[role="menu"]')).toBeNull();
  });

  it("다른 문서로 바꾸면 이전 문서의 메뉴를 닫는다", async () => {
    await render({ initial: ["docs/other.md", "docs/discovery/example.md"] });
    await openMenu();

    await click(container.querySelector('[data-tab-path="docs/other.md"]'));

    expect(container.querySelector('[role="menu"]')).toBeNull();
  });
});

describe("문서 링크 클릭", () => {
  /** 렌더된 마크다운의 링크를 누른다 — 포인터를 짚지 않는다. 포커스가 옮겨 가면
   *  "문서가 있는 칸"과 "포커스된 칸"이 구분되지 않기 때문이다. */
  const clickLink = async (pane: HTMLElement) => {
    const nested = pane.querySelector("a strong");
    expect(nested, "문서 링크를 찾지 못했다").toBeTruthy();
    const event = new MouseEvent("click", { bubbles: true, cancelable: true });
    await act(async () => nested?.dispatchEvent(event));
    return event;
  };

  const activeTabOf = (pane: HTMLElement) =>
    Array.from(pane.querySelectorAll<HTMLElement>("[data-tab-path]")).find((el) =>
      el.className.includes("bg-bg"),
    )?.dataset.tabPath ?? null;

  it("링크가 가리키는 파일은 문서와 같은 칸에 열린다 — 포커스된 칸이 아니라", async () => {
    await render({ initial: ["a.ts", "docs/discovery/example.md"] });
    await click(splitButton(panes()[0], "오른쪽으로 분할"));
    expect(focusedPane()).toBe(panes()[1]);

    await clickLink(panes()[0]);

    expect(panes()).toHaveLength(2);
    expect(tabsOf(panes()[0])).toEqual(["a.ts", "docs/discovery/example.md", "DESIGN 한글.md"]);
    expect(activeTabOf(panes()[0])).toBe("DESIGN 한글.md");
    // 옆 칸은 링크와 무관하다.
    expect(tabsOf(panes()[1])).toEqual(["docs/discovery/example.md"]);
  });

  it("이미 열린 파일이면 그 칸에서 활성으로 올린다", async () => {
    await render({ initial: ["DESIGN 한글.md", "docs/discovery/example.md"] });

    await clickLink(panes()[0]);

    expect(tabsOf(panes()[0])).toEqual(["DESIGN 한글.md", "docs/discovery/example.md"]);
    expect(activeTabOf(panes()[0])).toBe("DESIGN 한글.md");
    expect(closedGlobally).toEqual([]);
  });

  it("http 링크는 기본 브라우저로 넘기고 앱은 그대로 둔다", async () => {
    await render({ initial: ["docs/url.md"] });

    await clickLink(panes()[0]);

    expect(h.openUrl).toHaveBeenCalledWith("https://example.com/%EB%AC%B8%EC%84%9C");
    expect(tabsOf(panes()[0])).toEqual(["docs/url.md"]);
  });

  it("루트 밖에서 열린 문서의 상대 링크는 그 문서 옆의 파일을 같은 칸에 연다", async () => {
    await render({ initial: ["a.ts", "/Users/test/notes/README.md"], rootPath: "/work/root" });

    const event = await clickLink(panes()[0]);

    expect(event.defaultPrevented).toBe(true);
    expect(navigationError).not.toHaveBeenCalled();
    expect(tabsOf(panes()[0])).toEqual(["a.ts", "/Users/test/notes/README.md", "/Users/test/notes/GLOSSARY.md"]);
    expect(activeTabOf(panes()[0])).toBe("/Users/test/notes/GLOSSARY.md");
  });

  it("열 수 없는 링크는 알리기만 한다", async () => {
    await render({ initial: ["docs/bad.md"] });

    const event = await clickLink(panes()[0]);

    // 막지 않으면 새 창이 뜬다 — 열지 못한 링크일수록 더 그렇다.
    expect(event.defaultPrevented).toBe(true);
    expect(navigationError).toHaveBeenCalledWith("에디터에서 열 수 없는 링크: /etc/passwd");
    expect(tabsOf(panes()[0])).toEqual(["docs/bad.md"]);
  });

  it("루트 안 절대 경로 링크는 루트 기준 상대 경로로 같은 칸에 연다", async () => {
    await render({ initial: ["docs/abs.md"], rootPath: "/work/root" });

    await clickLink(panes()[0]);

    expect(tabsOf(panes()[0])).toEqual(["docs/abs.md", "docs/target.md"]);
    expect(navigationError).not.toHaveBeenCalled();
  });

  it("조각만 있는 링크는 조용히 넘긴다 — 같은 문서 안의 이동이다", async () => {
    await render({ initial: ["docs/fragment.md"] });

    await clickLink(panes()[0]);

    expect(navigationError).not.toHaveBeenCalled();
    expect(tabsOf(panes()[0])).toEqual(["docs/fragment.md"]);
  });

  it("우클릭 메뉴의 \"열기\"도 같은 칸에 연다", async () => {
    await render({ initial: ["docs/discovery/example.md"] });
    const nested = container.querySelector("a strong");
    await act(async () =>
      nested?.dispatchEvent(
        new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 20, clientY: 30 }),
      ),
    );
    const open = Array.from(
      container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'),
    ).find((button) => button.textContent === "열기");

    await click(open);

    expect(tabsOf(panes()[0])).toEqual(["docs/discovery/example.md", "DESIGN 한글.md"]);
  });

  it("서로 다른 링크 읽기가 뒤집혀 끝나도 각각 문서가 있던 칸에 앉힌다", async () => {
    const pending = new Map<string, { complete: () => void; resolve: (opened: boolean) => void }>();
    await render({
      initial: ["docs/left.md", "docs/right.md"],
      onOpenFile: (path, complete) => new Promise((resolve) => pending.set(path, { complete, resolve })),
    });
    await click(splitButton(panes()[0], "오른쪽으로 분할"));
    await click(panes()[0].querySelector('[data-tab-path="docs/left.md"]'));
    await clickLink(panes()[0]);
    await clickLink(panes()[1]);

    await act(async () => {
      const opened = pending.get("y.ts");
      opened?.complete();
      opened?.resolve(true);
    });
    await act(async () => {
      const opened = pending.get("x.ts");
      opened?.complete();
      opened?.resolve(true);
    });

    expect(tabsOf(panes()[0])).toContain("x.ts");
    expect(tabsOf(panes()[1])).toContain("y.ts");
  });

  it("실패한 링크 열기는 같은 경로의 뒤 이은 트리 열기를 가로채지 않는다", async () => {
    let fail: ((opened: boolean) => void) | undefined;
    await render({
      initial: ["docs/left.md", "docs/right.md"],
      onOpenFile: (_path, _complete) => new Promise((resolve) => { fail = resolve; }),
    });
    await click(splitButton(panes()[0], "오른쪽으로 분할"));
    await click(panes()[0].querySelector('[data-tab-path="docs/left.md"]'));
    await clickLink(panes()[0]);
    await act(async () => { fail?.(false); });
    await click(panes()[1].querySelector('[data-tab-path="docs/right.md"]'));

    await act(async () => { await openFromTree("x.ts", true); });

    expect(tabsOf(panes()[0])).not.toContain("x.ts");
    expect(tabsOf(panes()[1])).toContain("x.ts");
  });

  it("같은 미개방 파일로 난 두 링크도 각각의 문서 칸에 복제한다", async () => {
    const pending: Array<{ complete: () => void; resolve: (opened: boolean) => void }> = [];
    await render({
      initial: ["docs/same-left.md", "docs/same-right.md"],
      onOpenFile: (_path, complete) => new Promise((resolve) => pending.push({ complete, resolve })),
    });
    await click(splitButton(panes()[0], "오른쪽으로 분할"));
    await click(panes()[0].querySelector('[data-tab-path="docs/same-left.md"]'));
    await clickLink(panes()[0]);
    await click(panes()[1].querySelector('[data-tab-path="docs/same-right.md"]'));
    await clickLink(panes()[1]);

    await act(async () => {
      pending[0]?.complete();
      pending[0]?.resolve(true);
    });

    expect(tabsOf(panes()[0])).toContain("shared.ts");
    expect(tabsOf(panes()[1])).toContain("shared.ts");
  });

  it("같은 파일의 두 번째 링크 실패가 첫 번째 요청의 착지를 지우지 않는다", async () => {
    const pending: Array<{ complete: () => void; resolve: (opened: boolean) => void }> = [];
    await render({
      initial: ["docs/same-left.md", "docs/same-right.md"],
      onOpenFile: (_path, complete) => new Promise((resolve) => pending.push({ complete, resolve })),
    });
    await click(splitButton(panes()[0], "오른쪽으로 분할"));
    await click(panes()[0].querySelector('[data-tab-path="docs/same-left.md"]'));
    await clickLink(panes()[0]);
    await click(panes()[1].querySelector('[data-tab-path="docs/same-right.md"]'));
    await clickLink(panes()[1]);
    await act(async () => { pending[1]?.resolve(false); });
    await act(async () => {
      pending[0]?.complete();
      pending[0]?.resolve(true);
    });

    expect(tabsOf(panes()[0])).toContain("shared.ts");
    expect(tabsOf(panes()[1])).not.toContain("shared.ts");
  });
});

describe("프리뷰 자리는 칸마다 하나다", () => {
  const closeTabIn = async (pane: HTMLElement, path: string) => {
    const tab = pane.querySelector<HTMLElement>(`[data-tab-path="${path}"]`);
    await click(tab?.querySelector('[aria-label="닫기"]'));
  };

  /** 오른쪽 칸이 프리뷰 p.ts 하나만 들고 있는 두 칸 배치 — 결함이 났던 그 모양이다. */
  const twoPanesWithPreview = async () => {
    await render({ initial: ["a.ts"] });
    await click(splitButton(panes()[0], "오른쪽으로 분할"));
    await act(async () => openFromTree("p.ts", true));
    await closeTabIn(panes()[1], "a.ts");
    expect(tabsOf(panes()[1])).toEqual(["p.ts"]);
  };

  it("포커스 칸의 프리뷰만 바뀌고 칸은 그대로 남는다", async () => {
    await twoPanesWithPreview();

    await act(async () => openFromTree("y.ts", true));

    expect(panes()).toHaveLength(2);
    expect(tabsOf(panes()[0])).toEqual(["a.ts"]);
    expect(tabsOf(panes()[1])).toEqual(["y.ts"]);
    expect(closedGlobally).toEqual(["p.ts"]);
  });

  it("다른 칸의 프리뷰는 남는다", async () => {
    await twoPanesWithPreview();
    // 왼쪽 칸을 포커스하고 거기서도 훑어본다.
    await click(panes()[0].querySelector('[data-tab-path="a.ts"]'));
    await act(async () => openFromTree("q.ts", true));
    expect(tabsOf(panes()[0])).toEqual(["a.ts", "q.ts"]);

    await act(async () => openFromTree("r.ts", true));

    expect(tabsOf(panes()[0])).toEqual(["a.ts", "r.ts"]);
    expect(tabsOf(panes()[1])).toEqual(["p.ts"]);
    expect(closedGlobally).toEqual(["q.ts"]);
  });

  it("프리뷰를 끄면(표시 없는 열기) 탭이 늘고 분할은 유지된다", async () => {
    await twoPanesWithPreview();

    await act(async () => openFromTree("y.ts"));

    expect(panes()).toHaveLength(2);
    expect(tabsOf(panes()[1])).toEqual(["p.ts", "y.ts"]);
    expect(closedGlobally).toEqual([]);
  });

  it("다른 칸에 있던 고정 탭도 트리로 고르면 포커스된 칸에 더한다", async () => {
    await render({ initial: ["a.ts", "b.ts"] });
    await click(splitButton(panes()[0], "오른쪽으로 분할"));
    await closeTabIn(panes()[0], "b.ts");
    await click(panes()[0].querySelector('[data-tab-path="a.ts"]'));

    await act(async () => { await openFromTree("b.ts", true); });

    expect(panes()).toHaveLength(2);
    expect(focusedPane()).toBe(panes()[0]);
    expect(tabsOf(panes()[0])).toEqual(["a.ts", "b.ts"]);
  });

  it("다른 칸의 프리뷰를 트리로 고르면 포커스된 칸의 프리뷰 자리만 물려받는다", async () => {
    await render({ initial: [file("a.ts"), file("p.ts", true)] });
    await click(panes()[0].querySelector('[data-tab-path="a.ts"]'));
    await click(splitButton(panes()[0], "오른쪽으로 분할"));
    await act(async () => { await openFromTree("x.ts", true); });
    await closeTabIn(panes()[1], "a.ts");
    await click(panes()[0].querySelector('[data-tab-path="p.ts"]'));

    await act(async () => { await openFromTree("x.ts", true); });

    expect(panes()).toHaveLength(2);
    expect(focusedPane()).toBe(panes()[0]);
    expect(tabsOf(panes()[0])).toEqual(["a.ts", "x.ts"]);
    expect(closedGlobally).toEqual(["p.ts"]);
  });
});
