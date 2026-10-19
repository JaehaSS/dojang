// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { requestDesignEdit } from "./edit-client";
const request = { requestId: "edit-1", operation: "inspect" as const, source: "src/card.tsx:2:0", scope: "instance" as const };
let host: DocumentFragment & { location: { href: string }; __dojangCanvasNative: boolean };
beforeEach(() => { vi.useFakeTimers(); host = Object.assign(document.createDocumentFragment(), { location: { href: "" }, __dojangCanvasNative: true }); vi.stubGlobal("window", host); });
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });
it("offers no write transport to a standalone browser", async () => { host.__dojangCanvasNative = false; expect(await requestDesignEdit(request)).toMatchObject({ ok: false, reason: "app_required" }); expect(host.location.href).toBe(""); });
it("waits for the matching bounded response and removes its timeout", async () => {
  const promise = requestDesignEdit(request); const done = vi.fn(); void promise.then(done);
  expect(host.location.href).toContain("praxis-designmode:edit?data=");
  host.dispatchEvent(new CustomEvent("dojang:design-edit-result", { detail: { requestId: "other", ok: false, reason: "write_failed", message: "bad" } }));
  host.dispatchEvent(new CustomEvent("dojang:design-edit-result", { detail: { requestId: request.requestId, ok: true } }));
  await Promise.resolve(); expect(done).not.toHaveBeenCalled();
  host.dispatchEvent(new CustomEvent("dojang:design-edit-result", { detail: { requestId: request.requestId, ok: false, reason: "stale_file", message: "changed" } }));
  expect(await promise).toMatchObject({ reason: "stale_file" }); expect(vi.getTimerCount()).toBe(0);
});
it("times out a missing native reply", async () => { const promise = requestDesignEdit(request); await vi.advanceTimersByTimeAsync(15000); expect(await promise).toMatchObject({ reason: "write_failed" }); expect(vi.getTimerCount()).toBe(0); });
it("cancels a pending request on selection teardown", async () => { const abort = new AbortController(); const promise = requestDesignEdit(request, abort.signal); abort.abort(); expect(await promise).toMatchObject({ reason: "cancelled" }); expect(vi.getTimerCount()).toBe(0); });
it("does not navigate for an already aborted request", async () => { const abort = new AbortController(); abort.abort(); expect(await requestDesignEdit(request, abort.signal)).toMatchObject({ reason: "cancelled" }); expect(host.location.href).toBe(""); });
