// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ edit: vi.fn(async () => {}), editHandler: undefined as undefined | ((event: { payload: unknown }) => void), start: vi.fn(), status: vi.fn(), stop: vi.fn(async () => {}), close: vi.fn(async () => {}), hide: vi.fn(async () => {}), open: vi.fn(async () => {}), bounds: vi.fn(async () => {}), off: vi.fn(), handler: undefined as undefined | ((event: { payload: unknown }) => void) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async (event: string, handler: (event: { payload: unknown }) => void) => { if (event === "design-canvas://edit-request") mocks.editHandler = handler; else mocks.handler = handler; return mocks.off; } }));
vi.mock("../../lib/ipc", () => ({ designCanvasEdit: mocks.edit, designCanvasStart: mocks.start, designCanvasStatus: mocks.status, designCanvasStop: mocks.stop, designmodeClose: mocks.close, designmodeHide: mocks.hide, designmodeOpen: mocks.open, designmodeSetBounds: mocks.bounds }));
import { DesignTab } from "./DesignTab";
import { waitForDesignTransition } from "../../lib/design-canvas/lifecycle";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const ready = { status: "ready", url: "http://127.0.0.1:51873/design.html", port: 51873, error: null, logPath: "/tmp/dojang-design.log" };
let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  vi.useFakeTimers();
  Object.values(mocks).forEach((mock) => { if (typeof mock === "function" && "mockClear" in mock) mock.mockClear(); });
  mocks.handler = undefined; mocks.editHandler = undefined;
  mocks.start.mockResolvedValue(ready); mocks.status.mockResolvedValue(ready);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ x: 10, y: 20, width: 1000, height: 700 } as DOMRect);
  host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => { root.unmount(); await waitForDesignTransition(7); });
  host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers();
});
describe("DesignTab lifecycle", () => {
  it("routes only this task's bounded edit request through the main Rust command", async () => {
    await act(async () => { root.render(<DesignTab taskId={7} />); });
    const request = { requestId: "edit-1", operation: "apply", source: "src/view.tsx:3:0", scope: "instance", property: "padding-top", value: "13px", expectedHash: "a".repeat(64) };
    await act(async () => { mocks.editHandler?.({ payload: null }); mocks.editHandler?.({ payload: { taskId: 8, surface: "surface", request } }); mocks.editHandler?.({ payload: { taskId: 7, surface: "surface", request: { ...request, expectedHash: "bad" } } }); });
    expect(mocks.edit).not.toHaveBeenCalled();
    await act(async () => { mocks.editHandler?.({ payload: { taskId: 7, surface: "surface", request } }); });
    expect(mocks.edit).toHaveBeenCalledWith(7, "surface", request);
    await act(async () => root.render(null));
    await act(async () => { mocks.editHandler?.({ payload: { taskId: 7, surface: "surface", request } }); });
    expect(mocks.edit).toHaveBeenCalledOnce();
  });
  it("retains only validated selections for the current task and unsubscribes", async () => {
    await act(async () => { root.render(<DesignTab taskId={7} />); });
    await act(async () => { await waitForDesignTransition(7); });
    const selection = { version: 1, source: "src/design/screens/weekly-summary.screen.tsx:4:2", classKind: "dynamic", className: "p-4", tagName: "div", computedCss: { padding: "16px" }, boundingRect: { x: 0, y: 0, width: 100, height: 50 }, suggestedAncestor: null };
    await act(async () => { mocks.handler?.({ payload: null }); });
    expect(host.querySelector('[data-design-source]')).toBeNull();
    await act(async () => { mocks.handler?.({ payload: { taskId: 8, selection } }); });
    expect(host.querySelector('[data-design-source]')).toBeNull();
    await act(async () => { mocks.handler?.({ payload: { taskId: 7, selection: { ...selection, boundingRect: { width: NaN } } } }); });
    expect(host.querySelector('[data-design-source]')).toBeNull();
    await act(async () => { mocks.handler?.({ payload: { taskId: 7, selection } }); });
    expect(host.querySelector('[data-design-source]')?.getAttribute("data-design-class-kind")).toBe("dynamic");
    await act(async () => { root.render(null); });
    await waitForDesignTransition(7);
    expect(mocks.off).toHaveBeenCalledTimes(2);
  });
  it("starts on mount, opens the existing native surface inline, and reaps on close", async () => {
    await act(async () => { root.render(<DesignTab taskId={7} />); });
    await act(async () => { await waitForDesignTransition(7); });
    expect(mocks.open).toHaveBeenCalledWith(7, ready.url, { x: 10, y: 20, width: 1000, height: 700 }, "inline");
    const owner = mocks.start.mock.calls[0][1];
    await act(async () => { root.render(null); });
    await act(async () => { await waitForDesignTransition(7); });
    expect(mocks.stop).toHaveBeenCalledWith(7, owner);
    expect(mocks.close.mock.invocationCallOrder[mocks.close.mock.invocationCallOrder.length - 1]).toBeLessThan(mocks.stop.mock.invocationCallOrder[0]);
  });
  it("shows starting state and cancels an in-flight start without opening a stale view", async () => {
    mocks.start.mockResolvedValue({ ...ready, status: "server-starting" });
    await act(async () => { root.render(<DesignTab taskId={7} />); });
    expect(host.textContent).toContain("개발 서버 시작 중 · 포트 51873");
    await act(async () => { root.render(null); });
    await act(async () => { await waitForDesignTransition(7); });
    expect(mocks.open).not.toHaveBeenCalled();
    expect(mocks.stop).toHaveBeenCalledTimes(1);
  });
  it("shows the failure reason and log, then retries with a new owner after cleanup", async () => {
    mocks.start.mockResolvedValueOnce({ ...ready, status: "server-failed", error: "Vite 실행 실패" });
    await act(async () => { root.render(<DesignTab taskId={7} />); });
    await act(async () => { await waitForDesignTransition(7); });
    expect(host.textContent).toContain("Vite 실행 실패");
    expect(host.textContent).toContain(ready.logPath);
    await act(async () => { host.querySelector("button")!.click(); });
    await act(async () => { await waitForDesignTransition(7); });
    expect(mocks.start).toHaveBeenCalledTimes(2);
    expect(mocks.start.mock.calls[0][1]).not.toBe(mocks.start.mock.calls[1][1]);
    expect(mocks.stop.mock.invocationCallOrder[0]).toBeLessThan(mocks.start.mock.invocationCallOrder[1]);
    expect(mocks.open).toHaveBeenCalledTimes(1);
  });
  it("removes the native overlay when a running server dies so its error is readable", async () => {
    await act(async () => { root.render(<DesignTab taskId={7} />); });
    await act(async () => { await waitForDesignTransition(7); });
    mocks.status.mockResolvedValue({ ...ready, status: "server-failed", error: "서버 종료" });
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(mocks.close).toHaveBeenCalledTimes(2);
    expect(host.querySelector('[role="alert"]')?.textContent).toBe("서버 종료");
  });
});
