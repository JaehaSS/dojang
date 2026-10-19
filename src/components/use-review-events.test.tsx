// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type Handler = (event: { payload: unknown }) => void;
const mocks = vi.hoisted(() => ({
  handlers: new Map<string, Handler>(),
  unlisten: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, handler: Handler) => {
    mocks.handlers.set(name, handler);
    return Promise.resolve(mocks.unlisten);
  },
}));

import {
  REVIEW_ANNOTATIONS_EVENT,
  REVIEW_NAVIGATE_EVENT,
  navigationFrom,
  resolveNavigationHunk,
  targetsTask,
  useReviewEvents,
} from "./use-review-events";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root | null = null;
let container: HTMLDivElement | null = null;
const onAnnotationsChanged = vi.fn();
const onNavigate = vi.fn();

async function mount(task: { host: string; id: number }) {
  const Probe = () => {
    useReviewEvents(task, { onAnnotationsChanged, onNavigate });
    return null;
  };
  await act(async () => {
    root?.render(createElement(Probe));
    await Promise.resolve();
  });
}

const fire = async (name: string, payload: unknown) => {
  await act(async () => mocks.handlers.get(name)?.({ payload }));
};

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root?.unmount());
  container?.remove();
  root = null;
  container = null;
  mocks.handlers.clear();
  vi.clearAllMocks();
});

describe("useReviewEvents", () => {
  it("refreshes annotations and forwards navigation only for this local task", async () => {
    await mount({ host: "local", id: 9 });

    await fire(REVIEW_ANNOTATIONS_EVENT, { task_id: 9, path: "src/a.ts" });
    await fire(REVIEW_ANNOTATIONS_EVENT, { task_id: 10, path: "src/a.ts" });
    expect(onAnnotationsChanged).toHaveBeenCalledTimes(1);

    const payload = { task_id: 9, path: "src/a.ts", hunk_id: "h2", line: 12, side: "new" };
    await fire(REVIEW_NAVIGATE_EVENT, payload);
    await fire(REVIEW_NAVIGATE_EVENT, { ...payload, task_id: 10 });
    expect(onNavigate).toHaveBeenCalledTimes(1);
    expect(onNavigate).toHaveBeenCalledWith(payload);
  });

  it("ignores events for a remote task with the same id and unlistens on unmount", async () => {
    await mount({ host: "runner-1", id: 9 });
    await fire(REVIEW_ANNOTATIONS_EVENT, { task_id: 9, path: "src/a.ts" });
    expect(onAnnotationsChanged).not.toHaveBeenCalled();
    expect(targetsTask({ host: "runner-1", id: 9 }, 9)).toBe(false);

    await act(async () => root?.unmount());
    await act(async () => Promise.resolve());
    expect(mocks.unlisten).toHaveBeenCalledTimes(2);
  });

  it("maps the Rust payload to the tab-facing navigation target", () => {
    expect(
      navigationFrom({ task_id: 9, path: "src/a.ts", hunk_id: "h2", line: 41, side: "new" }, 3),
    ).toEqual({ path: "src/a.ts", hunkId: "h2", line: 41, side: "new", nonce: 3 });
  });

  it("ignores queued callbacks after the subscription is disposed", async () => {
    await mount({ host: "local", id: 9 });
    await act(async () => root?.unmount());
    await fire(REVIEW_ANNOTATIONS_EVENT, { task_id: 9, path: "src/a.ts" });
    await fire(REVIEW_NAVIGATE_EVENT, { task_id: 9, path: "src/a.ts", hunk_id: "h1", line: 1, side: "new" });
    expect(onAnnotationsChanged).not.toHaveBeenCalled();
    expect(onNavigate).not.toHaveBeenCalled();
  });

  it("falls back from a stale hunk id to the hunk that contains the line", () => {
    const hunks = [
      { id: "h1", path: "src/a.ts", old_range: [9, 2] as [number, number], new_range: [9, 3] as [number, number] },
      { id: "h2", path: "src/a.ts", old_range: [40, 0] as [number, number], new_range: [41, 1] as [number, number] },
      { id: "h3", path: "src/b.ts", old_range: [1, 1] as [number, number], new_range: [1, 1] as [number, number] },
    ];
    const target = { path: "src/a.ts", hunkId: "gone", line: 41, side: "new", nonce: 1 };
    expect(resolveNavigationHunk({ ...target, hunkId: "h1" }, hunks)).toBe("h1");
    expect(resolveNavigationHunk(target, hunks)).toBe("h2");
    expect(resolveNavigationHunk({ ...target, line: 40, side: "old" }, hunks)).toBe("h2");
    expect(resolveNavigationHunk({ ...target, line: 1 }, hunks)).toBeNull();
    expect(resolveNavigationHunk({ ...target, line: 20 }, hunks)).toBeNull();
  });
});
