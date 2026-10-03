// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useQuestionThreads } from "./use-question-threads";
import type { SideQuestionSnapshot } from "./side-question";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root;
let host: HTMLDivElement;
let state: ReturnType<typeof useQuestionThreads> | null = null;

function Harness({ sessionKey, conversation = true, api }: { sessionKey: string | null; conversation?: boolean; api: { read: () => Promise<SideQuestionSnapshot> } | null }) {
  state = useQuestionThreads(sessionKey, conversation, api);
  return null;
}

const snapshot = (turns: number) => ({ generation: 0, turns: Array.from({ length: turns }, () => ({})) }) as unknown as SideQuestionSnapshot;
const render = async (props: Parameters<typeof Harness>[0]) => {
  await act(async () => { root.render(<Harness {...props} />); await Promise.resolve(); await Promise.resolve(); });
};

beforeEach(() => {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  state = null;
});

it("기록이 없는 작업에서는 숨기고, 기록이 있으면 보인다", async () => {
  await render({ sessionKey: "local:1", api: { read: vi.fn().mockResolvedValue(snapshot(0)) } });
  expect(state!.has).toBe(false);

  await render({ sessionKey: "local:2", api: { read: vi.fn().mockResolvedValue(snapshot(2)) } });
  expect(state!.has).toBe(true);
});

it("처음 질문하는 순간 보이고, 다른 작업을 다녀와도 남는다", async () => {
  const read = vi.fn().mockResolvedValue(snapshot(0));
  const api = { read };
  await render({ sessionKey: "local:1", api });
  act(() => state!.mark("local:1"));
  expect(state!.has).toBe(true);

  await render({ sessionKey: "local:2", api });
  expect(state!.has).toBe(false);
  await render({ sessionKey: "local:1", api });
  expect(state!.has).toBe(true);
});

it("대화 작업이 아니면 조회하지 않고, 조회 실패는 없음으로 둔다", async () => {
  const read = vi.fn().mockResolvedValue(snapshot(1));
  await render({ sessionKey: "local:1", conversation: false, api: { read } });
  expect(read).not.toHaveBeenCalled();
  expect(state!.has).toBe(false);

  await render({ sessionKey: "local:3", api: { read: () => { throw new Error("no transport"); } } });
  expect(state!.has).toBe(false);
});
