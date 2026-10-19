// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => {
  const listeners = new Map<string, (event: { payload: unknown }) => void>();
  return {
    listeners,
    listen: vi.fn(async (event: string, callback: (event: { payload: unknown }) => void) => {
      listeners.set(event, callback);
      return () => { listeners.delete(event); };
    }),
    open: vi.fn(async () => ({ session: 7, existed: false })),
    snapshot: vi.fn(async () => ({ session: 7, sequence: 0, data: "", exited: false, exit_code: null })),
    close: vi.fn(async () => undefined),
    write: vi.fn(),
    writeln: vi.fn(),
    shellWrite: vi.fn(async () => undefined),
    input: undefined as ((data: string) => void) | undefined,
  };
});
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("@xterm/xterm", () => ({ Terminal: class { cols = 80; rows = 24; options = {}; loadAddon() {} open() {} write = mocks.write; writeln = mocks.writeln; onData(callback: (data: string) => void) { mocks.input = callback; return { dispose() {} }; } } }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));
vi.mock("../TerminalView", () => ({ useXtermTheme: () => ({}) }));
vi.mock("../../lib/xterm-webgl", () => ({ tryLoadWebgl: () => null, disposeWebgl() {}, disposeTerminal() {} }));
vi.mock("../../lib/project-editor-ipc", () => ({ projectShellOpen: mocks.open, projectShellSnapshot: mocks.snapshot, projectShellWrite: mocks.shellWrite, projectShellResize: vi.fn(), projectShellClose: mocks.close }));

import { ProjectTerminal, restartProjectShell } from "./ProjectTerminal";

(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root;
let host: HTMLDivElement;
beforeEach(() => {
  mocks.listeners.clear();
  mocks.input = undefined;
  class Observer { observe() {} disconnect() {} }
  vi.stubGlobal("ResizeObserver", Observer);
  host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); vi.clearAllMocks(); });

describe("ProjectTerminal", () => {
  it("subscribes before opening the root-bound shell", async () => {
    await act(async () => {
      root.render(<ProjectTerminal hidden={false} restart={0} onSession={() => {}} onExit={() => {}} onError={() => {}} />);
      await Promise.resolve(); await Promise.resolve();
    });

    expect(mocks.listen).toHaveBeenCalledWith("project-shell://output", expect.any(Function));
    expect(mocks.listen).toHaveBeenCalledWith("project-shell://exit", expect.any(Function));
    expect(mocks.listen.mock.invocationCallOrder[1]).toBeLessThan(mocks.open.mock.invocationCallOrder[0]);
  });

  it("listener failure releases partial subscriptions and exposes recovery without opening a shell", async () => {
    const stop = vi.fn();
    mocks.listen.mockResolvedValueOnce(stop).mockRejectedValueOnce(new Error("listener unavailable"));
    const onExit = vi.fn();
    const onError = vi.fn();
    await act(async () => { root.render(<ProjectTerminal hidden={false} restart={0} onSession={() => {}} onExit={onExit} onError={onError} />); });
    expect(stop).toHaveBeenCalledOnce();
    expect(mocks.open).not.toHaveBeenCalled();
    expect(onExit).toHaveBeenLastCalledWith(true);
    expect(onError).toHaveBeenCalledWith("Error: listener unavailable");
  });

  it("snapshot failure preserves the session for explicit retry and removes subscriptions", async () => {
    mocks.snapshot.mockRejectedValueOnce(new Error("snapshot unavailable"));
    const onSession = vi.fn();
    const onExit = vi.fn();
    await act(async () => { root.render(<ProjectTerminal hidden={false} restart={0} onSession={onSession} onExit={onExit} onError={() => {}} />); });
    expect(onSession).toHaveBeenLastCalledWith(7);
    expect(onExit).toHaveBeenLastCalledWith(true);
    expect(mocks.listeners.size).toBe(0);
    expect(mocks.close).not.toHaveBeenCalled();
    act(() => mocks.input?.("must not execute\r"));
    expect(mocks.shellWrite).not.toHaveBeenCalled();
    await restartProjectShell(7);
    expect(mocks.close).toHaveBeenCalledWith(7, undefined);
    await act(async () => { root.render(<ProjectTerminal hidden={false} restart={1} onSession={onSession} onExit={onExit} onError={() => {}} />); });
    expect(mocks.open).toHaveBeenCalledTimes(2);
    expect(onExit).toHaveBeenLastCalledWith(false);
    act(() => mocks.input?.("pwd\r"));
    expect(mocks.shellWrite).toHaveBeenCalledWith(7, "pwd\r", undefined);
  });

  it("unmount during listener registration cleans up the late subscription without spawning a shell", async () => {
    let finishListen!: (stop: () => void) => void;
    mocks.listen.mockImplementationOnce(() => new Promise((resolve) => { finishListen = resolve; }));
    await act(async () => { root.render(<ProjectTerminal hidden={false} restart={0} onSession={() => {}} onExit={() => {}} onError={() => {}} />); });
    await act(async () => { root.render(<div />); });
    const stop = vi.fn();
    await act(async () => { finishListen(stop); });
    expect(stop).toHaveBeenCalledOnce();
    expect(mocks.open).not.toHaveBeenCalled();
  });

  it("shared editor shell requests keep the root that opened the terminal", async () => {
    await act(async () => { root.render(<ProjectTerminal root="/repo/first" hidden={false} restart={0} onSession={() => {}} onExit={() => {}} onError={() => {}} />); });
    expect(mocks.open).toHaveBeenCalledWith(80, 24, "/repo/first");
    expect(mocks.snapshot).toHaveBeenCalledWith(7, "/repo/first");
    act(() => mocks.input?.("pwd\r"));
    expect(mocks.shellWrite).toHaveBeenCalledWith(7, "pwd\r", "/repo/first");
    await restartProjectShell(7, "/repo/first");
    expect(mocks.close).toHaveBeenCalledWith(7, "/repo/first");
  });

  it("reconciles queued output and exit with the snapshot before accepting live output", async () => {
    let finishSnapshot!: (value: Awaited<ReturnType<typeof mocks.snapshot>>) => void;
    mocks.snapshot.mockImplementationOnce(() => new Promise((resolve) => { finishSnapshot = resolve; }));
    const onExit = vi.fn();
    await act(async () => { root.render(<ProjectTerminal hidden={false} restart={0} onSession={() => {}} onExit={onExit} onError={() => {}} />); });
    act(() => {
      mocks.listeners.get("project-shell://output")!({ payload: { session: 7, sequence: 2, data: btoa("duplicate") } });
      mocks.listeners.get("project-shell://output")!({ payload: { session: 7, sequence: 3, data: btoa("queued") } });
      mocks.listeners.get("project-shell://exit")!({ payload: { session: 7, code: 0 } });
    });
    await act(async () => { finishSnapshot({ session: 7, sequence: 2, data: btoa("snapshot"), exited: false, exit_code: null }); });
    act(() => { mocks.listeners.get("project-shell://output")!({ payload: { session: 7, sequence: 4, data: btoa("live") } }); });
    expect(mocks.write.mock.calls.map(([bytes]) => new TextDecoder().decode(bytes))).toEqual(["snapshot", "queued", "live"]);
    expect(onExit).toHaveBeenLastCalledWith(true);
  });
});
