// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DesignSelection } from "../lib/design-canvas/selection";

const mocks = vi.hoisted(() => ({ bind: vi.fn(), send: vi.fn(), setEnabled: vi.fn(), ancestor: vi.fn(), dispose: vi.fn(), reselect: vi.fn() }));
vi.mock("./selection-client", () => ({ bindCanvasDocument: mocks.bind, sendSelectionToApp: mocks.send }));
import { DesignCanvas } from "./DesignCanvas";
import { collectScreens } from "./registry";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement;
let root: Root;
const screens = collectScreens({ "./screens/sample.screen.tsx": { meta: { title: "표본", viewport: { width: 960, height: 640 }, states: { 정상: {}, "빈 상태": {} } }, default: () => null } });
const snapshot: DesignSelection = { version: 1, source: "src/design/screens/sample.screen.tsx:4:0", classKind: "dynamic", tagName: "div", className: "p-4", computedCss: { "padding-top": "16px", "font-weight": "500" }, boundingRect: { x: 20, y: 30, width: 184, height: 96 }, suggestedAncestor: null };
const pick = async (selection = snapshot) => act(async () => { mocks.bind.mock.calls[mocks.bind.mock.calls.length - 1][1](selection); });
beforeEach(async () => {
  vi.clearAllMocks(); vi.useFakeTimers(); sessionStorage.clear();
  mocks.bind.mockReturnValue({ setEnabled: mocks.setEnabled, selectSuggestedAncestor: mocks.ancestor, dispose: mocks.dispose, reselectSource: mocks.reselect });
  mocks.reselect.mockReturnValue(false);
  mocks.send.mockReturnValue(false);
  host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host);
  await act(async () => root.render(<DesignCanvas screens={screens} />));
  // jsdom does not fetch the dev-server page. Use its loaded blank document to
  // model a completed iframe load while keeping the selector boundary intact.
  host.querySelector("iframe")!.src = "about:blank";
  await act(async () => { host.querySelector("iframe")!.dispatchEvent(new Event("load")); });
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); });
describe("DesignCanvas selection", () => {
  it("binds the iframe selector and displays readonly metadata and sizes", async () => {
    await act(async () => { [...host.querySelectorAll("button")].find((button) => button.textContent === "요소 선택")!.click(); });
    expect(mocks.setEnabled).toHaveBeenCalledWith(true);
    await pick();
    expect(host.querySelector('[aria-label="소스 위치"]')?.textContent).toBe(snapshot.source);
    expect(host.textContent).toContain("동적 className · 읽기 전용");
    expect(host.querySelector('[aria-label="요소 크기"]')?.textContent).toContain("184.0");
    expect(host.querySelector('[aria-label="padding 박스"]')?.textContent).toContain("16px");
  });
  it("requires an explicit ancestor choice instead of replacing untagged targets", async () => {
    await pick({ ...snapshot, source: null, classKind: "unknown", suggestedAncestor: { source: snapshot.source!, classKind: "static", className: "p-4", tagName: "section" } });
    expect(host.querySelector('[aria-label="소스 위치"]')?.textContent).toContain("태그가 없습니다");
    await act(async () => { [...host.querySelectorAll("button")].find((button) => button.textContent?.includes("가까운 조상 선택"))!.click(); });
    expect(mocks.ancestor).toHaveBeenCalledOnce();
  });
  it("waits for matching native ACK and ignores a late ACK after state change", async () => {
    mocks.send.mockReturnValue(true);
    await pick();
    const requestId = mocks.send.mock.calls[0][0].requestId;
    expect(host.querySelector('[aria-label="소스 위치"]')).toBeNull();
    await act(async () => { window.dispatchEvent(new CustomEvent("dojang:design-selection", { detail: { ...snapshot, requestId: "wrong" } })); });
    expect(host.querySelector('[aria-label="소스 위치"]')).toBeNull();
    await act(async () => { window.dispatchEvent(new CustomEvent("dojang:design-selection", { detail: { ...snapshot, requestId } })); });
    expect(host.querySelector('[aria-label="소스 위치"]')?.textContent).toBe(snapshot.source);
    await pick();
    const late = mocks.send.mock.calls[1][0].requestId;
    expect(host.querySelector('[aria-label="소스 위치"]')).toBeNull();
    await act(async () => { [...host.querySelectorAll("button")].find((button) => button.textContent === "빈 상태")!.click(); });
    await act(async () => { window.dispatchEvent(new CustomEvent("dojang:design-selection", { detail: { ...snapshot, requestId: late } })); });
    expect(host.querySelector('[aria-label="소스 위치"]')).toBeNull();
  });
  it("clears stale geometry on viewport change and still permits selection", async () => {
    await pick();
    const viewport = host.querySelector<HTMLSelectElement>('[aria-label="뷰포트"]')!;
    await act(async () => { viewport.value = "390x844"; viewport.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(host.querySelector('[aria-label="요소 크기"]')).toBeNull();
    expect([...host.querySelectorAll("button")].find((button) => button.textContent === "요소 선택")?.disabled).toBe(false);
    await pick({ ...snapshot, boundingRect: { ...snapshot.boundingRect, width: 300 } });
    expect(host.querySelector('[aria-label="요소 크기"]')?.textContent).toContain("300.0");
  });
  it("invalidates a delayed native ACK on HMR and reselects the locally picked source", async () => {
    mocks.send.mockReturnValue(true);
    await pick(); const beforeHmr = mocks.send.mock.calls[0][0].requestId;
    await act(async () => { window.dispatchEvent(new Event("dojang:design-hmr")); });
    await act(async () => { window.dispatchEvent(new CustomEvent("dojang:design-selection", { detail: { ...snapshot, requestId: beforeHmr } })); });
    expect(host.querySelector('[aria-label="소스 위치"]')).toBeNull();
    mocks.reselect.mockImplementation((source) => { mocks.bind.mock.calls[mocks.bind.mock.calls.length - 1][1]({ ...snapshot, source, computedCss: { "padding-top": "32px" } }); return true; });
    await act(async () => { await vi.advanceTimersByTimeAsync(100); });
    expect(mocks.reselect).toHaveBeenCalledWith(snapshot.source, undefined);
    const refreshed = mocks.send.mock.calls[mocks.send.mock.calls.length - 1][0];
    await act(async () => { window.dispatchEvent(new CustomEvent("dojang:design-selection", { detail: refreshed })); });
    expect(host.querySelector('[aria-label="padding 박스"]')?.textContent).toContain("32px");
    mocks.reselect.mockReturnValue(false); mocks.reselect.mockImplementation(() => false);
  });
  it("shows failure when the app does not acknowledge and removes the controller on unmount", async () => {
    mocks.send.mockReturnValue(true);
    await pick();
    await act(async () => { await vi.advanceTimersByTimeAsync(3000); });
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("선택 정보를 확인하지 못했습니다");
    await act(async () => root.render(null));
    expect(mocks.dispose).toHaveBeenCalled();
  });
});
