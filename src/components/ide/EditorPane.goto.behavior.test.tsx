// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LspTarget } from "../../lib/ipc";

const CTRL_CMD = 2048;
const ALT = 512;
const SHIFT = 1024;
const KEY_B = 32;
const KEY_L = 42;
const KEY_F7 = 60;
const KEY_F12 = 59;

/** 드래그 범위 스텁 — Monaco Selection에서 EditorPane이 읽는 것만 흉내 낸다. */
const selectionStub = (
  over: Partial<{ startLineNumber: number; endLineNumber: number }> = {},
) => ({
  startLineNumber: 3,
  startColumn: 1,
  endLineNumber: 5,
  endColumn: 12,
  isEmpty: () => false,
  ...over,
});

/** Monaco가 등록해 간 언어 기능 — ⌘클릭·⌘hover·F12가 실제로 부르는 것들. */
interface StubProvider {
  provideDefinition?: (model: unknown, position: unknown) => Promise<unknown[]>;
  provideImplementation?: (
    model: unknown,
    position: unknown,
  ) => Promise<unknown[]>;
  provideReferences?: (model: unknown, position: unknown) => Promise<unknown[]>;
}

const h = vi.hoisted(() => ({
  commands: new Map<number, () => void>(),
  // 우클릭 메뉴에 걸리는 동작 — 단축키는 commands에도 함께 꽂는다.
  actions: new Map<string, { label: string; run: () => void }>(),
  mouseDown: [] as ((e: unknown) => void)[],
  mouseUp: [] as ((e: unknown) => void)[],
  providers: {} as Record<string, StubProvider>,
  // 분할에서는 칸마다 프로바이더가 하나씩 걸린다 — 몇 개가 걸렸고 누가 답하는지를 봐야 한다.
  definitionProviders: [] as StubProvider[],
  opener: null as {
    openCodeEditor: (src: unknown, uri: unknown) => boolean;
  } | null,
  // ⌘L이 창을 모르고 동작하는지 보려면 "쓰지 않는 것"도 감시해야 한다.
  pushCapture: vi.fn(),
  requestComposerFocus: vi.fn(),
  editor: {
    addCommand(key: number, handler: () => void) {
      h.commands.set(key, handler);
    },
    addAction(action: {
      id: string;
      label: string;
      keybindings?: number[];
      run: () => void;
    }) {
      h.actions.set(action.id, action);
      for (const key of action.keybindings ?? []) h.commands.set(key, action.run);
      return { dispose: () => undefined };
    },
    onMouseDown(handler: (e: unknown) => void) {
      h.mouseDown.push(handler);
      return { dispose: () => undefined };
    },
    onMouseUp(handler: (e: unknown) => void) {
      h.mouseUp.push(handler);
      return { dispose: () => undefined };
    },
    createContextKey: () => ({ set: () => undefined }),
    getPosition: () => ({ lineNumber: 10, column: 5 }),
    getValue: () => "저장 안 한 편집",
    // 기본은 선택 없음 — ⌘L 테스트만 자기 범위를 꽂아 넣는다.
    getSelection: vi.fn((): unknown => null),
    getModel: vi.fn((): unknown => null),
    revealLineInCenter: vi.fn(),
    setPosition: vi.fn(),
    setScrollPosition: vi.fn(),
    focus: vi.fn(),
    // 질문 버블이 선택·스크롤을 따라다닌다. 이 테스트는 버블을 보지 않지만
    // 구독 자체는 마운트에서 일어나므로 받아 둔다.
    onDidChangeCursorSelection: vi.fn(),
    onDidScrollChange: vi.fn(),
    getScrolledVisiblePosition: () => null,
    getLayoutInfo: () => ({ height: 600 }),
  },
}));

vi.mock("@monaco-editor/react", async () => {
  const React = await import("react");
  return {
    default: ({ onMount }: { onMount: (ed: unknown, m: unknown) => void }) => {
      React.useEffect(() => {
        onMount(h.editor, {
          KeyMod: { CtrlCmd: CTRL_CMD, Alt: ALT, Shift: SHIFT },
          KeyCode: { KeyS: 49, KeyB: KEY_B, KeyL: KEY_L, F7: KEY_F7, F12: KEY_F12 },
          Uri: {
            parse: (p: string) => ({ toString: () => `file://${p}` }),
            file: (p: string) => ({ toString: () => `abs://${p}` }),
          },
          Range: class {
            constructor(
              public startLineNumber: number,
              public startColumn: number,
              public endLineNumber: number,
              public endColumn: number,
            ) {}
          },
          languages: {
            registerDefinitionProvider: (_lang: string, p: StubProvider) => {
              h.providers.definition = p;
              h.definitionProviders.push(p);
              return { dispose: () => undefined };
            },
            registerImplementationProvider: (
              _lang: string,
              p: StubProvider,
            ) => {
              h.providers.implementation = p;
              return { dispose: () => undefined };
            },
            registerReferenceProvider: (_lang: string, p: StubProvider) => {
              h.providers.references = p;
              return { dispose: () => undefined };
            },
          },
          editor: {
            MouseTargetType: { CONTENT_TEXT: 6 },
            getModels: () => [],
            registerEditorOpener: (o: {
              openCodeEditor: (s: unknown, u: unknown) => boolean;
            }) => {
              h.opener = o;
              return { dispose: () => undefined };
            },
          },
        });
      }, [onMount]);
      return React.createElement("div", { "data-testid": "monaco" });
    },
  };
});

vi.mock("../../lib/designmode/editor-capture-target", () => ({
  registerEditorCaptureTarget: () => () => undefined,
}));

// 이 둘은 EditorPane이 **부르지 않아야** 하는 것들이다. 첨부의 목적지를 아는 것은 창을 아는
// 쪽(App / EditorWindow)뿐이므로, 여기서 store를 직접 건드리면 팝아웃 창의 ⌘L이 제 창에 쌓인다.
vi.mock("../../lib/designmode/store", async (original) => ({
  ...(await original<Record<string, unknown>>()),
  pushCapture: h.pushCapture,
}));
vi.mock("../../lib/composer-focus", async (original) => ({
  ...(await original<Record<string, unknown>>()),
  requestComposerFocus: h.requestComposerFocus,
}));

// lib/monaco는 monaco-editor 번들 전체(클립보드 기여 등 DOM API 의존)를 끌어온다 —
// jsdom에서는 로드 자체가 실패하므로 여기서 필요한 것만 대신 내놓는다.
vi.mock("../../lib/monaco", () => ({
  langFromPath: (path: string) => (path.endsWith(".rs") ? "rust" : "plaintext"),
}));

import { EditorPane, type OpenFile } from "./EditorPane";
import { fileTabKey } from "../../lib/tab-key";

(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

const FILE: OpenFile = {
  key: fileTabKey("src/main.rs"),
  path: "src/main.rs",
  kind: "text",
  content: "fn main() {}",
  baseContent: "fn main() {}",
  mtime: 0,
  dirty: false,
};

const READ_ONLY_FILE: OpenFile = {
  ...FILE,
  key: fileTabKey("/Users/test/external.rs"),
  path: "/Users/test/external.rs",
  readOnly: true,
};

const target = (over: Partial<LspTarget> = {}): LspTarget => ({
  path: "src/lib.rs",
  abs_path: "/w/src/lib.rs",
  line: 42,
  column: 8,
  external: false,
  ...over,
});

let container: HTMLDivElement;
let root: Root;

type PaneProps = Parameters<typeof EditorPane>[0];

function render(extra: Partial<PaneProps> = {}) {
  const props: PaneProps = {
    taskId: 1,
    files: [FILE],
    activeKey: FILE.key,
    retainedPaths: [FILE.path],
    dark: false,
    onSelect: () => undefined,
    onClose: () => undefined,
    onChange: () => undefined,
    onSave: () => undefined,
    onReload: () => undefined,
    onOpenPath: () => undefined,
    onRevealPath: () => undefined,
    ...extra,
  };
  act(() => {
    root.render(<EditorPane {...props} />);
  });
}

/** 등록된 Monaco 커맨드를 눌러보고, 뒤따르는 비동기 조회까지 흘려보낸다. */
async function press(keybinding: number) {
  await act(async () => {
    h.commands.get(keybinding)?.();
  });
  await act(async () => {}); // definition → references 폴백처럼 이어지는 라운드 대기
}

/** 에디터 본문을 눌렀다 뗀다 — 플랫폼 판정과 무관하게 ⌘·Ctrl을 함께 싣는다. */
function click(
  position: { lineNumber: number; column: number },
  { modifier = true, releaseAt = position } = {},
) {
  const event = (at: typeof position) => ({
    event: { metaKey: modifier, ctrlKey: modifier, leftButton: true },
    target: { type: 6, position: at },
  });
  for (const handler of h.mouseDown) handler(event(position));
  for (const handler of h.mouseUp) handler(event(releaseAt));
}

/** Monaco가 ⌘클릭·⌘hover에서 넘겨주는 모델 스텁. */
const model = (path = FILE.path, text = "편집 중") => ({
  uri: { toString: () => `file://${path}` },
  getValue: () => text,
});

beforeEach(() => {
  h.commands.clear();
  h.actions.clear();
  h.mouseDown = [];
  h.mouseUp = [];
  h.providers = {};
  h.definitionProviders = [];
  h.opener = null;
  h.editor.revealLineInCenter.mockClear();
  h.editor.setPosition.mockClear();
  h.editor.setScrollPosition.mockClear();
  h.editor.focus.mockClear();
  h.editor.getSelection.mockReturnValue(null);
  h.editor.getModel.mockReturnValue(null);
  h.pushCapture.mockClear();
  h.requestComposerFocus.mockClear();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("EditorPane ⌘B", () => {
  it("커서 심볼의 정의를 묻고, 결과가 하나면 곧장 그 위치를 연다", async () => {
    const onGoto = vi.fn().mockResolvedValue([target()]);
    const onOpenTarget = vi.fn();
    render({ onGoto, onOpenTarget });

    await press(CTRL_CMD | KEY_B);

    expect(onGoto).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "definition",
        path: "src/main.rs",
        line: 10,
        column: 5,
      }),
    );
    expect(onOpenTarget).toHaveBeenCalledWith(target());
    // 결과가 하나면 고르라고 묻지 않는다.
    expect(container.querySelector("[role=dialog]")).toBeNull();
  });

  it("디스크가 아니라 편집 중인 버퍼를 보낸다", async () => {
    const onGoto = vi.fn().mockResolvedValue([]);
    render({ onGoto, onOpenTarget: vi.fn() });

    await press(CTRL_CMD | KEY_B);

    expect(onGoto.mock.calls[0][0].text).toBe("저장 안 한 편집");
  });

  it("선언 위에서 누르면 제자리 대신 사용처를 보여준다", async () => {
    // 정의 결과가 커서가 선 그 줄뿐 = 이미 선언 위에 있다.
    const onGoto = vi
      .fn()
      .mockResolvedValueOnce([target({ path: "src/main.rs", line: 10 })])
      .mockResolvedValueOnce([target({ line: 3 }), target({ line: 77 })]);
    render({ onGoto, onOpenTarget: vi.fn() });

    await press(CTRL_CMD | KEY_B);

    expect(onGoto).toHaveBeenCalledTimes(2);
    expect(onGoto.mock.calls[1][0].kind).toBe("references");
    expect(container.textContent).toContain("사용처");
  });

  it("후보가 여럿이면 목록에서 고르게 하고, 고른 곳을 연다", async () => {
    const onGoto = vi
      .fn()
      .mockResolvedValue([target({ line: 3 }), target({ line: 9 })]);
    const onOpenTarget = vi.fn();
    render({ onGoto, onOpenTarget });

    await press(CTRL_CMD | KEY_B);
    expect(onOpenTarget).not.toHaveBeenCalled();

    const items = container.querySelectorAll("[role=listitem]");
    expect(items).toHaveLength(2);
    act(() => {
      (items[1] as HTMLButtonElement).click();
    });

    expect(onOpenTarget).toHaveBeenCalledWith(target({ line: 9 }));
  });

  it("결과가 없으면 조용히 알린다", async () => {
    render({ onGoto: vi.fn().mockResolvedValue([]), onOpenTarget: vi.fn() });

    await press(CTRL_CMD | KEY_B);

    expect(container.textContent).toContain("결과가 없습니다");
  });

  it("서버가 없거나 죽었으면 그 사유를 그대로 보여준다", async () => {
    const onGoto = vi
      .fn()
      .mockRejectedValue("rust-analyzer이(가) PATH에 없습니다");
    render({ onGoto, onOpenTarget: vi.fn() });

    await press(CTRL_CMD | KEY_B);

    expect(container.textContent).toContain("PATH에 없습니다");
  });

  it("⌥⌘B는 구현을 묻는다", async () => {
    const onGoto = vi.fn().mockResolvedValue([target()]);
    render({ onGoto, onOpenTarget: vi.fn() });

    await press(CTRL_CMD | ALT | KEY_B);

    expect(onGoto.mock.calls[0][0].kind).toBe("implementation");
  });

  it("조회 수단이 없으면(원격 작업) 아무 일도 하지 않는다", async () => {
    const onOpenTarget = vi.fn();
    render({ onGoto: undefined, onOpenTarget });

    await press(CTRL_CMD | KEY_B);

    expect(onOpenTarget).not.toHaveBeenCalled();
    expect(container.querySelector("[role=dialog]")).toBeNull();
  });

  it("읽기 전용 외부 파일에서는 키보드 정의 이동을 요청하지 않는다", async () => {
    const onGoto = vi.fn().mockResolvedValue([target()]);
    render({
      files: [READ_ONLY_FILE],
      activeKey: READ_ONLY_FILE.key,
      retainedPaths: [READ_ONLY_FILE.path],
      onGoto,
      onOpenTarget: vi.fn(),
    });

    await press(CTRL_CMD | KEY_B);

    expect(onGoto).not.toHaveBeenCalled();
  });
});

describe("EditorPane references", () => {
  it("⌥B는 저장하지 않은 버퍼의 사용처를 열고 선택한 결과로 이동한다", async () => {
    const onGoto = vi.fn().mockResolvedValue([target()]);
    const onOpenTarget = vi.fn();
    render({ onGoto, onOpenTarget });

    expect(h.commands.get(ALT | KEY_B)).toBeTypeOf("function");
    expect(h.commands.get(SHIFT | KEY_F12)).toBeTypeOf("function");
    expect(h.commands.get(ALT | KEY_F7)).toBeTypeOf("function");

    await press(ALT | KEY_B);

    expect(onGoto).toHaveBeenCalledWith({
      kind: "references",
      path: "src/main.rs",
      text: "저장 안 한 편집",
      line: 10,
      column: 5,
    });
    expect(container.querySelector('[role="dialog"][aria-label="사용처"]')).not.toBeNull();

    const result = container.querySelector('[role="option"]') as HTMLButtonElement;
    expect(result).not.toBeNull();
    expect(result.getAttribute("aria-selected")).toBe("true");
    act(() => result.click());

    expect(onOpenTarget).toHaveBeenCalledWith(target());
  });

  it("drops a late references response after its source version changes", async () => {
    let resolve: ((targets: LspTarget[]) => void) | undefined;
    let version = 0;
    const onReferences = vi.fn();
    render({
      sourceVersion: () => version,
      beginReferences: () => 1,
      onReferences,
      onGoto: () =>
        new Promise((done) => {
          resolve = done;
        }),
    });

    await act(async () => h.commands.get(SHIFT | KEY_F12)?.());
    version += 1;
    await act(async () => resolve?.([target()]));

    expect(onReferences).not.toHaveBeenCalled();
  });

  it("drops a delayed references response after the task scope changes", async () => {
    let resolve: ((targets: LspTarget[]) => void) | undefined;
    const onReferences = vi.fn();
    render({
      taskId: 1,
      navigationScope: "local:1:main",
      beginReferences: () => 1,
      onReferences,
      onGoto: () => new Promise((done) => { resolve = done; }),
    });

    await act(async () => h.commands.get(SHIFT | KEY_F12)?.());
    render({ taskId: 2, navigationScope: "local:2:main", onReferences, onGoto: vi.fn() });
    await act(async () => resolve?.([target()]));

    expect(onReferences).not.toHaveBeenCalled();
  });
});

describe("EditorPane — Monaco 언어 기능(⌘클릭·⌘hover·F12)", () => {
  it("정의/구현 provider만 등록하고 사용처는 앱의 메뉴 항목으로 연다", () => {
    render({ onGoto: vi.fn().mockResolvedValue([]), onOpenTarget: vi.fn() });
    expect(h.providers.definition?.provideDefinition).toBeTypeOf("function");
    expect(h.providers.implementation?.provideImplementation).toBeTypeOf(
      "function",
    );
    // Monaco 자체 사용처 UI는 1건이면 바로 이동하고 열지 않은 파일은 미리보기를 못 만든다.
    expect(h.providers.references).toBeUndefined();
    expect(h.actions.get("praxis.findReferences")?.label).toBe("사용처 보기");
  });

  it("선언 위 ⌘클릭은 제자리 이동 대신 사용처 목록을 연다", async () => {
    const onGoto = vi
      .fn()
      .mockResolvedValueOnce([target({ path: "src/main.rs", line: 2 })])
      .mockResolvedValueOnce([target({ line: 3 })]);
    const onReferences = vi.fn();
    render({ onGoto, onReferences, beginReferences: () => 1, onOpenTarget: vi.fn() });

    await act(async () => click({ lineNumber: 2, column: 4 }));
    await act(async () => {});

    expect(onGoto.mock.calls.map((call) => call[0])).toEqual([
      expect.objectContaining({ kind: "definition", line: 2, column: 4 }),
      expect.objectContaining({ kind: "references", line: 2, column: 4 }),
    ]);
    expect(onReferences).toHaveBeenCalledTimes(1);
  });

  it("선언이 아닌 곳의 ⌘클릭은 Monaco의 정의 이동에 맡긴다", async () => {
    const onGoto = vi.fn().mockResolvedValue([target()]);
    const onOpenTarget = vi.fn();
    render({ onGoto, onOpenTarget });

    await act(async () => click({ lineNumber: 2, column: 4 }));
    await act(async () => {});
    await act(async () => click({ lineNumber: 2, column: 4 }, { modifier: false }));
    // ⌘-드래그는 선택이다.
    await act(async () =>
      click({ lineNumber: 2, column: 4 }, { releaseAt: { lineNumber: 2, column: 9 } }),
    );

    expect(onGoto).toHaveBeenCalledTimes(1);
    // 이동은 provider 결과로 Monaco가 한다 — 여기서 한 번 더 열면 두 번 착지한다.
    expect(onOpenTarget).not.toHaveBeenCalled();
    expect(container.querySelector("[role=dialog]")).toBeNull();
  });

  it("⌘클릭 보조 조회가 진행 중인 ⌘B를 버리게 하지 않는다", async () => {
    let resolveB: ((targets: LspTarget[]) => void) | undefined;
    const onGoto = vi
      .fn()
      .mockImplementationOnce(() => new Promise((done) => { resolveB = done; }))
      .mockResolvedValueOnce([target()]);
    const onOpenTarget = vi.fn();
    render({ onGoto, onOpenTarget });

    await act(async () => h.commands.get(CTRL_CMD | KEY_B)?.());
    await act(async () => click({ lineNumber: 2, column: 4 }));
    await act(async () => resolveB?.([target({ line: 5 })]));

    expect(onOpenTarget).toHaveBeenCalledWith(target({ line: 5 }));
    expect(container.textContent).not.toContain("찾는 중");
  });

  it("우클릭 메뉴의 사용처 보기는 앱의 사용처 목록을 연다", async () => {
    const onGoto = vi.fn().mockResolvedValue([target()]);
    const onReferences = vi.fn();
    render({ onGoto, onReferences, beginReferences: () => 1, onOpenTarget: vi.fn() });

    await act(async () => h.actions.get("praxis.findReferences")?.run());

    expect(onGoto.mock.calls.map((call) => call[0].kind)).toEqual(["references"]);
    expect(onReferences).toHaveBeenCalledTimes(1);
  });

  it("읽기 전용 외부 파일의 provider 요청은 보내지 않는다", async () => {
    const onGoto = vi.fn().mockResolvedValue([target()]);
    render({
      files: [READ_ONLY_FILE],
      activeKey: READ_ONLY_FILE.key,
      retainedPaths: [READ_ONLY_FILE.path],
      onGoto,
      onOpenTarget: vi.fn(),
    });

    await expect(
      h.providers.definition!.provideDefinition!(model(READ_ONLY_FILE.path), {
        lineNumber: 7,
        column: 3,
      }),
    ).resolves.toEqual([]);

    expect(onGoto).not.toHaveBeenCalled();
  });

  it("source edit after a query discards its late response", async () => {
    let resolve: ((targets: LspTarget[]) => void) | undefined;
    let version = 0;
    render({
      sourceVersion: () => version,
      onGoto: () =>
        new Promise((done) => {
          resolve = done;
        }),
    });

    const pending = h.providers.definition!.provideDefinition!(model(), {
      lineNumber: 7,
      column: 3,
    });
    version += 1;
    await act(async () => resolve?.([target()]));

    await expect(pending).resolves.toEqual([]);
  });

  it("does not return a delayed provider result to a replacement task with the same path", async () => {
    let resolve: ((targets: LspTarget[]) => void) | undefined;
    render({
      taskId: 1,
      navigationScope: "local:1:main",
      onGoto: () => new Promise((done) => { resolve = done; }),
    });

    const pending = h.providers.definition!.provideDefinition!(model(), {
      lineNumber: 7,
      column: 3,
    });
    render({ taskId: 2, navigationScope: "local:2:main", onGoto: vi.fn() });
    await act(async () => resolve?.([target()]));

    await expect(pending).resolves.toEqual([]);
  });

  it("정의 요청을 편집 중 버퍼와 함께 백엔드로 넘기고 위치로 돌려준다", async () => {
    const onGoto = vi.fn().mockResolvedValue([target()]);
    render({ onGoto, onOpenTarget: vi.fn() });

    const result = await h.providers.definition!.provideDefinition!(model(), {
      lineNumber: 7,
      column: 3,
    });

    expect(onGoto).toHaveBeenCalledWith({
      kind: "definition",
      path: "src/main.rs",
      text: "편집 중",
      line: 7,
      column: 3,
    });
    // 워크트리 안이면 탭 모델과 같은 URI 규약을 써야 "이미 열린 파일"로 인식된다.
    expect(result).toHaveLength(1);
    expect((result[0] as { uri: { toString(): string } }).uri.toString()).toBe(
      "file://src/lib.rs",
    );
  });

  it("조회가 실패해도 hover가 깨지지 않게 빈 결과로 접는다", async () => {
    const onGoto = vi.fn().mockRejectedValue(new Error("서버 없음"));
    render({ onGoto, onOpenTarget: vi.fn() });

    await expect(
      h.providers.definition!.provideDefinition!(model(), {
        lineNumber: 1,
        column: 1,
      }),
    ).resolves.toEqual([]);
  });

  it("열려 있지 않은 모델의 요청은 묻지 않는다", async () => {
    const onGoto = vi.fn().mockResolvedValue([target()]);
    render({ onGoto, onOpenTarget: vi.fn() });

    const result = await h.providers.definition!.provideDefinition!(
      model("src/남의파일.rs"),
      {
        lineNumber: 1,
        column: 1,
      },
    );

    expect(result).toEqual([]);
    expect(onGoto).not.toHaveBeenCalled();
  });

  it("다른 파일로의 착지는 App의 탭 열기로 넘긴다", async () => {
    const onOpenTarget = vi.fn();
    render({ onGoto: vi.fn().mockResolvedValue([target()]), onOpenTarget });
    await h.providers.definition!.provideDefinition!(model(), {
      lineNumber: 1,
      column: 1,
    });

    const handled = h.opener!.openCodeEditor(h.editor, {
      toString: () => "file://src/lib.rs",
    });

    expect(handled).toBe(true);
    expect(onOpenTarget).toHaveBeenCalledWith(target());
  });

  it("우리가 낸 결과가 아닌 URI는 Monaco에 되돌려준다", () => {
    const onOpenTarget = vi.fn();
    render({ onGoto: vi.fn().mockResolvedValue([]), onOpenTarget });

    const handled = h.opener!.openCodeEditor(h.editor, {
      toString: () => "file://모르는곳",
    });

    expect(handled).toBe(false);
    expect(onOpenTarget).not.toHaveBeenCalled();
  });
});

describe("EditorPane reveal", () => {
  it("지시받은 줄로 커서를 옮기고 소비를 알린다", () => {
    const onRevealed = vi.fn();
    render({ reveal: { path: FILE.path, line: 42, column: 8 }, onRevealed });

    expect(h.editor.revealLineInCenter).toHaveBeenCalledWith(42);
    expect(h.editor.setPosition).toHaveBeenCalledWith({
      lineNumber: 42,
      column: 8,
    });
    expect(onRevealed).toHaveBeenCalled();
  });

  it("saved navigation restores the editor scroll position", () => {
    render({
      reveal: {
        path: FILE.path,
        line: 42,
        column: 8,
        scrollTop: 120,
        scrollLeft: 24,
        restore: true,
      },
    });

    expect(h.editor.setScrollPosition).toHaveBeenCalledWith({
      scrollTop: 120,
      scrollLeft: 24,
    });
    expect(h.editor.revealLineInCenter).not.toHaveBeenCalled();
  });

  it("다른 파일을 가리키면 그 탭이 활성화될 때까지 기다린다", () => {
    const onRevealed = vi.fn();
    render({
      reveal: { path: "src/other.rs", line: 3, column: 1 },
      onRevealed,
    });

    expect(h.editor.setPosition).not.toHaveBeenCalled();
    expect(onRevealed).not.toHaveBeenCalled();
  });
});

describe("EditorPane ⌘L", () => {
  /** 드래그해 둔 상태를 만든다 — 범위와, 그 범위에서 읽히는 텍스트를 함께 꽂는다. */
  const drag = (text: string, over = {}) => {
    h.editor.getSelection.mockReturnValue(selectionStub(over));
    h.editor.getModel.mockReturnValue({ getValueInRange: () => text });
  };

  it("목적지가 없으면 이 창의 컴포저에 붙이고 입력창으로 넘어간다", async () => {
    // 메인 창 경로 — store가 같은 webview에 있으므로 곧장 넣고 포커스를 옮긴다.
    drag("const x = 1;");
    render();

    await press(CTRL_CMD | KEY_L);

    expect(h.pushCapture).toHaveBeenCalledWith(
      1,
      expect.objectContaining({
        task_id: 1,
        file_path: "src/main.rs",
        selection_text: "const x = 1;",
        selection_start_line: 3,
        selection_end_line: 5,
      }),
    );
    expect(h.requestComposerFocus).toHaveBeenCalledWith(1);
  });

  it("목적지가 주어지면 그쪽에만 넘기고 store도 포커스도 건드리지 않는다", async () => {
    // 팝아웃 창 경로 — 여기서 store에 넣으면 칩이 남의 창에 쌓이고, 포커스를 옮기면
    // 코드를 읽던 사용자의 창을 메인이 뺏는다.
    const onAttachCapture = vi.fn();
    drag("const x = 1;");
    render({ onAttachCapture });

    await press(CTRL_CMD | KEY_L);

    expect(onAttachCapture).toHaveBeenCalledWith(
      expect.objectContaining({ task_id: 1, selection_text: "const x = 1;" }),
    );
    expect(h.pushCapture).not.toHaveBeenCalled();
    expect(h.requestComposerFocus).not.toHaveBeenCalled();
  });

  it("선택이 없으면 아무 일도 하지 않는다", async () => {
    // 파일 전체를 참조하는 것은 @멘션의 몫이다 — 빈 ⌘L이 빈 칩을 만들면 안 된다.
    const onAttachCapture = vi.fn();
    h.editor.getSelection.mockReturnValue({
      ...selectionStub(),
      isEmpty: () => true,
    });
    render({ onAttachCapture });

    await press(CTRL_CMD | KEY_L);

    expect(onAttachCapture).not.toHaveBeenCalled();
    expect(h.pushCapture).not.toHaveBeenCalled();
  });

  it("범위는 있는데 담긴 글자가 없으면 첨부하지 않는다", async () => {
    // `isEmpty()`는 false여도 읽어 온 텍스트가 빈 문자열일 수 있다(모델이 아직 안 붙은 순간 등).
    // `buildSelectionCapture`가 null을 돌려주므로 여기서 끊긴다.
    const onAttachCapture = vi.fn();
    drag("");
    render({ onAttachCapture });

    await press(CTRL_CMD | KEY_L);

    expect(onAttachCapture).not.toHaveBeenCalled();
    expect(h.pushCapture).not.toHaveBeenCalled();
  });
});

describe("EditorPane 분할 — 언어 기능 소유권", () => {
  /** 같은 파일을 두 칸이 띄운 상태. 두 칸 모두 Monaco에 프로바이더를 건다. */
  function renderPair(onGoto: PaneProps["onGoto"]) {
    const props: PaneProps = {
      taskId: 1,
      files: [FILE],
      activeKey: FILE.key,
      retainedPaths: [FILE.path],
      dark: false,
      onGoto,
      onSelect: () => undefined,
      onClose: () => undefined,
      onChange: () => undefined,
      onSave: () => undefined,
      onReload: () => undefined,
      onOpenPath: () => undefined,
      onRevealPath: () => undefined,
    };
    act(() => {
      root.render(
        <>
          <EditorPane {...props} />
          <EditorPane {...props} focused={false} />
        </>,
      );
    });
  }

  it("두 칸이 같은 파일을 띄워도 답하는 칸은 하나다", async () => {
    // Monaco는 등록된 프로바이더의 결과를 이어 붙인다 — 둘 다 답하면 유일한 정의도
    // 두 벌이 되어 Peek 목록으로 열린다.
    const onGoto = vi.fn().mockResolvedValue([target()]);
    renderPair(onGoto);

    // 등록은 (칸 × 지원 언어)만큼 일어난다 — 앞 절반이 먼저 마운트된 칸의 몫이다.
    const half = h.definitionProviders.length / 2;
    expect(half).toBeGreaterThan(0);
    const ask = (p: StubProvider) =>
      p.provideDefinition?.(model(), { lineNumber: 1, column: 1 });

    expect(await ask(h.definitionProviders[0])).toHaveLength(1);
    expect(await ask(h.definitionProviders[half])).toEqual([]);
    expect(onGoto).toHaveBeenCalledOnce();
  });

  it("아무 칸도 안 들고 있는 파일은 누구도 답하지 않는다", async () => {
    const onGoto = vi.fn().mockResolvedValue([target()]);
    renderPair(onGoto);

    const answers = await Promise.all(
      h.definitionProviders.map((p) =>
        p.provideDefinition?.(model("src/없는파일.rs"), {
          lineNumber: 1,
          column: 1,
        }),
      ),
    );

    expect(answers.every((r) => (r?.length ?? 0) === 0)).toBe(true);
    expect(onGoto).not.toHaveBeenCalled();
  });
});
