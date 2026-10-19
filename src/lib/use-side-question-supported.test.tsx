// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { SideQuestionSnapshot } from "./side-question";
import { useSideQuestionSupported } from "./use-side-question-supported";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

type Api = { read: () => Promise<SideQuestionSnapshot> };
let root: Root;
let host: HTMLDivElement;
let supported: boolean | null = null;

function Harness({ sessionKey, revision = "Running", api }: { sessionKey: string | null; revision?: string; api: Api | null }) {
  supported = useSideQuestionSupported(sessionKey, revision, api);
  return null;
}

const snapshot = (value: boolean) => ({ supported: value, turns: [] }) as unknown as SideQuestionSnapshot;
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
  supported = null;
});

it("백엔드가 지원한다고 답한 뒤에야 쓸 수 있다", async () => {
  let resolve: (value: SideQuestionSnapshot) => void = () => {};
  const api = { read: vi.fn(() => new Promise<SideQuestionSnapshot>((r) => { resolve = r; })) };
  await render({ sessionKey: "local:1", api });
  expect(supported).toBe(false);

  await act(async () => resolve(snapshot(true)));
  expect(supported).toBe(true);
});

it("지원하지 않는다는 답과 조회 실패는 못 쓴다로 둔다", async () => {
  await render({ sessionKey: "local:1", api: { read: vi.fn().mockResolvedValue(snapshot(false)) } });
  expect(supported).toBe(false);

  await render({ sessionKey: "local:2", api: { read: vi.fn().mockRejectedValue(new Error("no task")) } });
  expect(supported).toBe(false);
});

it("작업 상태가 바뀌면 다시 읽고, 답이 올 때까지 앞 값을 유지한다", async () => {
  const read = vi.fn().mockResolvedValueOnce(snapshot(true));
  const api = { read };
  await render({ sessionKey: "local:1", revision: "Running", api });
  expect(supported).toBe(true);

  let resolve: (value: SideQuestionSnapshot) => void = () => {};
  read.mockReturnValueOnce(new Promise<SideQuestionSnapshot>((r) => { resolve = r; }));
  await render({ sessionKey: "local:1", revision: "Done", api });
  expect(read).toHaveBeenCalledTimes(2);
  expect(supported).toBe(true);

  await act(async () => resolve(snapshot(false)));
  expect(supported).toBe(false);
});

it("다른 세션으로 옮기면 앞 세션의 값을 쓰지 않는다", async () => {
  const read = vi.fn().mockResolvedValueOnce(snapshot(true)).mockReturnValueOnce(new Promise(() => {}));
  const api = { read };
  await render({ sessionKey: "local:1", api });
  expect(supported).toBe(true);

  await render({ sessionKey: "local:2", api });
  expect(supported).toBe(false);
});

it("조회할 API가 없으면 읽지 않고 못 쓴다로 둔다", async () => {
  await render({ sessionKey: "local:1", api: null });
  expect(supported).toBe(false);
});
