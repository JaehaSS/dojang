// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  annotationsList: vi.fn(async () => []),
  diffHunks: vi.fn(async () => []),
  taskDiff: vi.fn(),
  /** 캡처가 읽어 가는 자리. 등록된 판독기를 그대로 붙잡아 무엇을 돌려주는지 본다. */
  captureReader: { current: null as (() => unknown) | null },
}));

// Monaco는 diff 탭에서 마운트되면 안 된다 — 그것이 이 파일이 보는 것이다.
vi.mock("@monaco-editor/react", async () => {
  const React = await import("react");
  return { default: () => React.createElement("div", { "data-testid": "monaco" }) };
});
vi.mock("../../lib/monaco", () => ({ langFromPath: () => "typescript" }));
vi.mock("./Mermaid", () => ({ Mermaid: () => null }));
vi.mock("../../lib/designmode/editor-capture-target", () => ({
  registerEditorCaptureTarget: (_taskId: number, reader: () => unknown) => {
    mocks.captureReader.current = reader;
    return () => {
      mocks.captureReader.current = null;
    };
  },
}));
vi.mock("../../lib/ipc", async (original) => ({
  ...(await original<Record<string, unknown>>()),
  annotationSave: vi.fn(),
  annotationsList: mocks.annotationsList,
  annotationsResend: vi.fn(),
  diffHunks: mocks.diffHunks,
  partialApply: vi.fn(),
  partialRollback: vi.fn(),
  taskDiff: mocks.taskDiff,
}));

import { DiffSessionProvider } from "../DiffSessionContext";
import { EditorPane, type OpenFile } from "./EditorPane";
import { DiffTab } from "./DiffTab";
import { diffTabKey } from "../../lib/tab-key";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

class ResizeObserverStub {
  observe() {}
  disconnect() {}
}

const PATH = "src/a.ts";
const DIFF_FILE: OpenFile = {
  key: diffTabKey(PATH),
  path: PATH,
  kind: "diff",
  content: "",
  baseContent: "",
  mtime: 0,
  dirty: false,
};

let container: HTMLDivElement;
let root: Root;

const render = async () => {
  await act(async () => {
    root.render(
      <DiffSessionProvider task={{ host: "local", id: 3 }} openDiff={() => {}}>
        <EditorPane
          taskId={3}
          files={[DIFF_FILE]}
          activeKey={DIFF_FILE.key}
          retainedPaths={[]}
          dark={false}
          onSelect={() => undefined}
          onClose={() => undefined}
          onChange={() => undefined}
          onSave={() => undefined}
          onReload={() => undefined}
          onOpenPath={() => undefined}
          onRevealPath={() => undefined}
        />
      </DiffSessionProvider>,
    );
    await Promise.resolve();
  });
};

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", ResizeObserverStub);
  mocks.taskDiff.mockResolvedValue({
    files: [{ path: PATH, status: "M", patch: "@@ -1 +1 @@\n+const b = 2;" }],
    baseline: { kind: "pinned" as const },
  });
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

describe("EditorPane diff 탭 (AC-13)", () => {
  it("저장·다시 불러오기 크롬 없이 diff 본문을 그리고 Monaco를 만들지 않는다", async () => {
    await render();

    expect(container.querySelector(`[data-diff-tab="${PATH}"]`)).toBeTruthy();
    expect(container.querySelector("[data-testid='monaco']")).toBeNull();
    expect(container.querySelector("[aria-label='저장']")).toBeNull();
    expect(container.querySelector("[aria-label='다시 불러오기']")).toBeNull();
    // 실경로에 대한 동작은 남는다 — 진짜 파일을 다루므로 여전히 유효하다.
    expect(container.querySelector("[aria-label='Finder에서 보기']")).toBeTruthy();
  });

  it("캡처 대상 자리를 비운다 — diff 화면을 그 파일의 에디터로 귀속시키지 않는다", async () => {
    await render();

    expect(mocks.captureReader.current).toBeTruthy();
    expect(mocks.captureReader.current?.()).toBeNull();
  });

  it("비활성 diff 탭은 활성 diff 탭의 파일 단축키를 받지 않는다", async () => {
    const hiddenOpen = vi.fn();
    const centralOpen = vi.fn();
    mocks.taskDiff.mockResolvedValue({
      files: [
        { path: PATH, status: "M", patch: "@@ -1 +1 @@\n+const b = 2;" },
        { path: "src/b.ts", status: "M", patch: "@@ -1 +1 @@\n+const c = 3;" },
      ],
      baseline: { kind: "pinned" as const },
    });
    await act(async () => {
      root.render(
        <DiffSessionProvider task={{ host: "local", id: 3 }} openDiff={hiddenOpen}>
          <EditorPane
            taskId={3}
            files={[DIFF_FILE]}
            activeKey={DIFF_FILE.key}
            retainedPaths={[]}
            dark={false}
            onSelect={() => undefined}
            onClose={() => undefined}
            onChange={() => undefined}
            onSave={() => undefined}
            onReload={() => undefined}
            onOpenPath={() => undefined}
            onRevealPath={() => undefined}
            shortcutsActive={false}
          />
          <DiffTab path={PATH} active onOpenPath={centralOpen} />
        </DiffSessionProvider>,
      );
      await Promise.resolve();
    });

    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "]" })));

    expect(centralOpen).toHaveBeenCalledWith("src/b.ts");
    expect(hiddenOpen).not.toHaveBeenCalled();
  });
});
