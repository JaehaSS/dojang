// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DesignSelection } from "../lib/design-canvas/selection";
import type { DesignEditInfo, DesignEditRequest, DesignEditResult } from "../lib/design-canvas/edit";
const mock = vi.hoisted(() => ({ request: vi.fn(), enabled: vi.fn(() => true), written: vi.fn() }));
vi.mock("./edit-client", () => ({ canEditCanvas: mock.enabled, requestDesignEdit: mock.request }));
import { useDesignEdits } from "./useDesignEdits";
import { DesignInspector } from "./DesignInspector";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const selection: DesignSelection = { version: 1, source: "src/design/screens/example.screen.tsx:3:0", tagName: "div", classKind: "static", className: "p-4", computedCss: { "padding-top": "16px" }, boundingRect: { x: 0, y: 0, width: 100, height: 100 }, suggestedAncestor: null };
const info: DesignEditInfo = { requestId: "inspect", ok: true, source: selection.source!, selectionSource: selection.source!, file: "src/design/screens/example.screen.tsx", line: 3, column: 0, hash: "a".repeat(64), editable: true, shared: false, className: "p-4", kind: "static" };
let controls: ReturnType<typeof useDesignEdits>;
function Harness({ target = selection }: { target?: DesignSelection | null }) { controls = useDesignEdits(target, mock.written); return <DesignInspector selection={target} matches={{}} onSelectAncestor={() => {}} edit={controls} />; }
let host: HTMLDivElement; let root: Root;
beforeEach(async () => {
  vi.clearAllMocks(); vi.useFakeTimers(); mock.enabled.mockReturnValue(true);
  mock.request.mockImplementation(async (request: DesignEditRequest) => ({ ...info, requestId: request.requestId, hash: request.operation === "apply" ? "b".repeat(64) : info.hash }));
  host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host);
  await act(async () => root.render(<Harness />));
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); sessionStorage.clear(); vi.restoreAllMocks(); vi.useRealTimers(); });
const applies = () => mock.request.mock.calls.filter(([request]) => request.operation === "apply");
describe("direct design edits", () => {
  it("coalesces valid input until 200ms after the latest value", async () => {
    await act(async () => { controls.change("padding-top", "13px"); await vi.advanceTimersByTimeAsync(100); controls.change("padding-top", "17px"); await vi.advanceTimersByTimeAsync(199); });
    expect(applies()).toHaveLength(0);
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    expect(applies()).toHaveLength(1); expect(applies()[0][0].value).toBe("17px"); expect(mock.written).toHaveBeenCalledOnce();
  });
  it("flushes immediately and removes a cleared pending input", async () => {
    await act(async () => { controls.change("padding-top", "13px"); controls.flush(); });
    expect(applies()).toHaveLength(1);
    await act(async () => { controls.change("padding-top", "15px"); controls.change("padding-top", ""); await vi.advanceTimersByTimeAsync(200); });
    expect(applies()).toHaveLength(1);
  });
  it("flushes through the inspector Enter and blur controls", async () => {
    await act(async () => { controls.change("padding-top", "13px"); });
    await act(async () => host.querySelector('[aria-label="편집 padding-top"]')!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(applies()).toHaveLength(1);
    await act(async () => { controls.change("padding-left", "19px"); });
    await act(async () => host.querySelector('[aria-label="편집 padding-left"]')!.dispatchEvent(new FocusEvent("focusout", { bubbles: true })));
    expect(applies()).toHaveLength(2);
  });
  it("keeps invalid CSS input local instead of writing an invalid class", async () => {
    await act(async () => { controls.change("padding-top", "invalid"); await vi.advanceTimersByTimeAsync(200); });
    expect(applies()).toHaveLength(0); expect(host.textContent).toContain("CSS 값과 단위");
  });
  it("normalizes a zero pixel value before proposing the class", async () => {
    await act(async () => { controls.change("padding-top", "0"); controls.flush(); });
    expect(applies()[0][0].value).toBe("0px");
  });
  it("serializes edits with the hash returned by the preceding write", async () => {
    let release!: (value: DesignEditResult) => void;
    mock.request.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
    await act(async () => { controls.change("padding-top", "13px"); controls.flush(); controls.change("padding-left", "19px"); controls.flush(); });
    expect(applies()).toHaveLength(1);
    await act(async () => release({ ...info, hash: "c".repeat(64) }));
    expect(applies()).toHaveLength(2); expect(applies()[1][0].expectedHash).toBe("c".repeat(64));
  });
  it("still waits for the next value's debounce when an earlier write completes", async () => {
    let release!: (value: DesignEditResult) => void;
    mock.request.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
    await act(async () => { controls.change("padding-top", "13px"); controls.flush(); await vi.advanceTimersByTimeAsync(100); controls.change("padding-top", "17px"); await vi.advanceTimersByTimeAsync(100); });
    await act(async () => release({ ...info, hash: "c".repeat(64) }));
    expect(applies()).toHaveLength(1);
    await act(async () => { await vi.advanceTimersByTimeAsync(99); }); expect(applies()).toHaveLength(1);
    await act(async () => { await vi.advanceTimersByTimeAsync(1); }); expect(applies()).toHaveLength(2);
    expect(applies()[1][0]).toMatchObject({ value: "17px", expectedHash: "c".repeat(64) });
  });
  it("blocks subsequent writes after stale-file and requires reselection", async () => {
    mock.request.mockResolvedValueOnce({ requestId: "write", ok: false, reason: "stale_file", message: "changed" });
    await act(async () => { controls.change("padding-top", "13px"); controls.flush(); });
    expect(host.textContent).toContain("stale-file"); expect(controls.info).toBeNull();
    await act(async () => { controls.change("padding-top", "19px"); controls.flush(); });
    expect(applies()).toHaveLength(1);
  });
  it("asks before component scope and carries confirmation only after acceptance", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    await act(async () => controls.chooseScope("component")); expect(controls.scope).toBe("instance");
    confirm.mockReturnValue(true); await act(async () => controls.chooseScope("component"));
    await act(async () => { controls.change("padding-top", "18px"); controls.flush(); });
    expect(applies()[0][0]).toMatchObject({ scope: "component", componentConfirmed: true });
  });
  it("shows disabled dynamic fields and the agent request path", async () => {
    mock.request.mockResolvedValueOnce({ ...info, editable: false, kind: "dynamic", reason: "dynamic_classname" });
    await act(async () => root.render(<Harness target={{ ...selection, source: "src/dynamic.tsx:1:0", classKind: "dynamic" }} />));
    expect(host.querySelector<HTMLInputElement>('input')?.disabled).toBe(true); expect(host.textContent).toContain("에이전트에게");
    await act(async () => { [...host.querySelectorAll("button")].find((button) => button.textContent === "에이전트에게 요청")!.click(); });
    expect(host.querySelector<HTMLTextAreaElement>('[aria-label="에이전트 요청문"]')?.value).toContain("src/dynamic.tsx");
  });
  it("offers confirmed definition scope when the callsite cannot be established", async () => {
    mock.request.mockResolvedValueOnce({ ...info, editable: false, shared: true, reason: "instance_source_missing" });
    await act(async () => root.render(<Harness target={{ ...selection, source: "src/Shared.tsx:1:0" }} />));
    expect(host.querySelector('[aria-label="수정 범위"]')).not.toBeNull();
    expect(host.textContent).toContain("사용처 위치를 확인할 수 없어");
    expect(host.querySelector<HTMLInputElement>('input')?.disabled).toBe(true);
  });
  it("cancels the pending debounce when a different element is selected", async () => {
    await act(async () => { controls.change("padding-top", "13px"); root.render(<Harness target={{ ...selection, source: "src/other.tsx:1:0" }} />); });
    await act(async () => { await vi.advanceTimersByTimeAsync(200); }); expect(applies()).toHaveLength(0);
  });
  it("aborts request listeners and drops pending writes on unmount", async () => {
    await act(async () => { controls.change("padding-top", "13px"); root.render(null); });
    expect(mock.request.mock.calls[0][1].aborted).toBe(true);
    await act(async () => { await vi.advanceTimersByTimeAsync(200); }); expect(applies()).toHaveLength(0);
  });
});
