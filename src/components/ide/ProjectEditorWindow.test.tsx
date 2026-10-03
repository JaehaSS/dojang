// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  info: vi.fn(async () => ({ root: "/repo", label: "project-editor-7", launch: false })),
  refreshTree: vi.fn(),
  emit: vi.fn(async () => undefined),
  close: vi.fn(async () => undefined),
  closeListener: undefined as ((event: { preventDefault: () => void }) => void) | undefined,
  flushDirty: vi.fn(),
}));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ onCloseRequested: async (callback: typeof mocks.closeListener) => { mocks.closeListener = callback; return () => {}; }, close: mocks.close }) }));
vi.mock("@tauri-apps/api/event", () => ({ emit: mocks.emit }));
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: vi.fn() }));
vi.mock("../../lib/project-editor-ipc", () => ({ projectEditorInfo: mocks.info, projectEditorTree: vi.fn(), projectEditorRead: vi.fn(), projectEditorWrite: vi.fn(), projectEditorResolvePath: vi.fn(), projectEditorOpenPath: vi.fn() }));
vi.mock("../../lib/ipc", () => ({ fontSettingsGet: vi.fn(async () => ({ ui_family: "system", code_family: "mono", ui_size: 13, code_size: 13 })), editorSettingsGet: vi.fn(async () => ({ tree_font_size: 13, minimap: true, word_wrap: false, tab_size: 2 })) }));
vi.mock("./useWorkspaceFiles", () => ({ useWorkspaceFiles: () => ({ tree: [], openFiles: [], activeKey: null, activeFile: null, refreshTree: mocks.refreshTree, openFile: vi.fn(), pinTab: vi.fn(), setActiveKey: vi.fn(), closeTab: vi.fn(), changeFile: vi.fn(), saveFile: vi.fn(), reloadFile: vi.fn(), flushDirty: mocks.flushDirty }) }));
vi.mock("./EditorSplitView", () => ({ EditorSplitView: () => <div data-editor-split /> }));
vi.mock("./FileTree", () => ({ FileTree: () => <div data-file-tree /> }));
vi.mock("./icons", () => ({ Icon: () => null }));
vi.mock("./ProjectTerminal", () => ({ ProjectTerminal: () => <div />, restartProjectShell: vi.fn() }));
vi.mock("../../lib/use-theme", () => ({ useTheme: () => ({ kind: "dark" }) }));

import { ProjectEditorWindow } from "./ProjectEditorWindow";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root;
let host: HTMLDivElement;
beforeEach(() => { localStorage.clear(); mocks.closeListener = undefined; mocks.flushDirty.mockResolvedValue({ ok: true, entries: [] }); host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.clearAllMocks(); });

describe("ProjectEditorWindow", () => {
  it("loads caller-bound project info and exposes the editor surface", async () => {
    await act(async () => { root.render(<ProjectEditorWindow />); await Promise.resolve(); await Promise.resolve(); });

    expect(mocks.info).toHaveBeenCalledOnce();
    expect(mocks.refreshTree).toHaveBeenCalledOnce();
    expect(host.textContent).toContain("/repo");
    expect(host.querySelector("[data-editor-split]")).not.toBeNull();
  });

  it("toggles the project terminal from its header", async () => {
    await act(async () => { root.render(<ProjectEditorWindow />); await Promise.resolve(); await Promise.resolve(); });
    const button = host.querySelector<HTMLButtonElement>('[aria-label="터미널 열기"]');
    expect(button?.title).toBe("터미널 열기 (⌃`)");

    await act(async () => button?.click());

    expect(host.querySelector('[aria-label="터미널 닫기"]')?.getAttribute("aria-pressed")).toBe("true");
  });

  it("opens the terminal on load when the window launches an agent", async () => {
    mocks.info.mockResolvedValueOnce({ root: "/vault", label: "project-editor-9", launch: true });
    await act(async () => { root.render(<ProjectEditorWindow />); await Promise.resolve(); await Promise.resolve(); });

    expect(host.textContent).toContain("터미널");
    expect(host.querySelector('[aria-label="터미널 닫기"]')?.getAttribute("aria-pressed")).toBe("true");
  });

  it("keeps the window open when dirty flush fails", async () => {
    mocks.flushDirty.mockResolvedValueOnce({ ok: false, path: "a.ts", reason: "failed", detail: "read-only" });
    await act(async () => { root.render(<ProjectEditorWindow />); await Promise.resolve(); await Promise.resolve(); });
    const preventDefault = vi.fn();
    await act(async () => { mocks.closeListener?.({ preventDefault }); await Promise.resolve(); });

    expect(preventDefault).toHaveBeenCalledOnce();
    expect(mocks.close).not.toHaveBeenCalled();
    expect(host.textContent).toContain("a.ts를 저장하지 못했습니다");
  });

  it("reports malformed saved tabs and resets their metadata to an empty record", async () => {
    localStorage.setItem("praxis-project-editor:/repo", "{");
    await act(async () => { root.render(<ProjectEditorWindow />); await Promise.resolve(); await Promise.resolve(); });

    expect(host.textContent).toContain("저장된 탭 정보를 읽지 못했습니다");
    expect(JSON.parse(localStorage.getItem("praxis-project-editor:/repo")!)).toEqual({ paths: [], active: null });
  });

  it("prevents repeated native close requests while a flush is pending", async () => {
    let finishFlush!: (result: { ok: true; entries: [] }) => void;
    mocks.flushDirty.mockImplementationOnce(() => new Promise((resolve) => { finishFlush = resolve; }));
    await act(async () => { root.render(<ProjectEditorWindow />); });
    const first = vi.fn();
    const second = vi.fn();
    act(() => { mocks.closeListener?.({ preventDefault: first }); mocks.closeListener?.({ preventDefault: second }); });
    expect(first).toHaveBeenCalledOnce();
    expect(second).toHaveBeenCalledOnce();
    expect(mocks.flushDirty).toHaveBeenCalledOnce();
    await act(async () => { finishFlush({ ok: true, entries: [] }); });
    expect(mocks.close).toHaveBeenCalledOnce();
  });
});
