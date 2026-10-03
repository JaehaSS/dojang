// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { usePendingQuestions } from "./use-pending-questions";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const invoke = vi.fn<(cmd: string) => Promise<number[]>>();
const handlers = new Map<string, (event: { payload: unknown }) => void>();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (cmd: string) => invoke(cmd) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, handler: (event: { payload: unknown }) => void) => {
    handlers.set(event, handler);
    return Promise.resolve(() => handlers.delete(event));
  },
}));

function Probe() {
  return <output>{usePendingQuestions().join(",")}</output>;
}

let root: Root;
let host: HTMLDivElement;
const text = () => host.querySelector("output")?.textContent ?? null;
const flush = async () => { await act(async () => { await Promise.resolve(); await Promise.resolve(); }); };
const fire = async (event: string, payload: unknown) => {
  await act(async () => { handlers.get(event)?.({ payload }); await Promise.resolve(); await Promise.resolve(); });
};

beforeEach(() => {
  invoke.mockReset();
  handlers.clear();
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});

describe("usePendingQuestions", () => {
  it("첫 조회 결과를 내고, 질문 원장과 작업 상태 이벤트마다 다시 읽는다", async () => {
    invoke.mockResolvedValueOnce([3]);
    await act(async () => root.render(<Probe />));
    await flush();
    expect(text()).toBe("3");
    expect(invoke).toHaveBeenCalledWith("interaction_pending_tasks");
    expect([...handlers.keys()].sort()).toEqual(["convo-interaction://changed", "task://state"]);

    invoke.mockResolvedValueOnce([3, 7]);
    await fire("convo-interaction://changed", { taskId: 7 });
    expect(text()).toBe("3,7");

    // 턴이 끝나면 질문은 원장 이벤트 없이 닫힌다 — 작업 상태 전이로도 다시 읽는다.
    invoke.mockResolvedValueOnce([]);
    await fire("task://state", { id: 3, state: "Completed" });
    expect(text()).toBe("");
  });

  it("조회가 실패해도 마지막 목록을 지우지 않는다", async () => {
    invoke.mockResolvedValueOnce([5]);
    await act(async () => root.render(<Probe />));
    await flush();
    expect(text()).toBe("5");
    invoke.mockRejectedValueOnce(new Error("offline"));
    await fire("task://state", { id: 5, state: "Running" });
    expect(text()).toBe("5");
  });

  it("언마운트 뒤에는 구독을 풀고 늦은 응답을 버린다", async () => {
    let resolve: (ids: number[]) => void = () => {};
    invoke.mockReturnValueOnce(new Promise<number[]>((r) => { resolve = r; }));
    await act(async () => root.render(<Probe />));
    await flush();
    await act(async () => root.unmount());
    expect(handlers.size).toBe(0);
    resolve([9]);
    await flush();
    root = createRoot(host);
  });
});
