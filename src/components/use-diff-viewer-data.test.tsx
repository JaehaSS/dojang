// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  annotationsList: vi.fn(),
  diffHunks: vi.fn(),
  taskDiff: vi.fn(),
}));

vi.mock("../lib/ipc", () => ({
  annotationsList: mocks.annotationsList,
  diffHunks: mocks.diffHunks,
  taskDiff: mocks.taskDiff,
}));

import { useDiffViewerData, type DiffViewerData } from "./use-diff-viewer-data";
import type { DiffHunk, FileDiff } from "../lib/ipc";
import type { TaskRef } from "../lib/transport";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const hunk: DiffHunk = {
  id: "h1",
  path: "a.ts",
  old_range: [1, 1],
  new_range: [1, 1],
  protected: false,
  committed: false,
  risk: "low",
  lines: [{ kind: "add", text: "const value = 1;" }],
};

const file = (path: string): FileDiff => ({ path, status: "M", patch: "@@ -1 +1 @@\n+value" });

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

let latest: DiffViewerData | null = null;
let container: HTMLDivElement | null = null;
let root: Root | null = null;

function Probe({ task }: { task: TaskRef }) {
  latest = useDiffViewerData(task);
  return null;
}

async function render(task: TaskRef = { host: "local", id: 7 }) {
  await act(async () => {
    root?.render(<Probe task={task} />);
    await Promise.resolve();
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  latest = null;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root?.unmount());
  container?.remove();
  root = null;
  container = null;
  vi.clearAllMocks();
  vi.useRealTimers();
});

describe("useDiffViewerData", () => {
  it("uses the review bundled with taskDiff without separate hunk or annotation requests", async () => {
    mocks.taskDiff.mockResolvedValue({
      files: [file("atomic.ts")],
      baseline: { kind: "pinned" },
      review: { hunks: [hunk], annotations: [], warning: "주석 일부를 불러오지 못했습니다." },
    });

    await render();

    expect(mocks.taskDiff).toHaveBeenCalledWith({ host: "local", id: 7 }, "session", true);
    expect(mocks.diffHunks).not.toHaveBeenCalled();
    expect(mocks.annotationsList).not.toHaveBeenCalled();
    expect(latest?.hunks).toEqual([hunk]);
    expect(latest?.warning).toContain("주석 일부");
  });

  it("falls back to hunk and annotation endpoints for an older Runner", async () => {
    mocks.taskDiff.mockResolvedValue({ files: [file("legacy.ts")], baseline: { kind: "legacy" } });
    mocks.diffHunks.mockResolvedValue([hunk]);
    mocks.annotationsList.mockResolvedValue([]);

    await render();

    expect(mocks.diffHunks).toHaveBeenCalledWith({ host: "local", id: 7 }, "session");
    expect(mocks.annotationsList).toHaveBeenCalledWith({ host: "local", id: 7 }, "session");
    expect(latest?.files).toEqual([file("legacy.ts")]);
  });

  it("coalesces focus, timer, and manual refresh while one task/range request is pending", async () => {
    const pending = deferred<{
      files: FileDiff[];
      baseline: { kind: "pinned" };
      review: { hunks: DiffHunk[]; annotations: [] };
    }>();
    mocks.taskDiff.mockReturnValue(pending.promise);

    await render();
    await act(async () => {
      window.dispatchEvent(new Event("focus"));
      vi.advanceTimersByTime(5_000);
      latest?.refresh();
      await Promise.resolve();
    });

    expect(mocks.taskDiff).toHaveBeenCalledOnce();

    await act(async () => {
      pending.resolve({ files: [file("once.ts")], baseline: { kind: "pinned" }, review: { hunks: [], annotations: [] } });
      await pending.promise;
    });
    expect(latest?.files).toEqual([file("once.ts")]);
  });

  it("allows a new range to load while the old range is pending and ignores its stale result", async () => {
    const session = deferred<{ files: FileDiff[]; baseline: { kind: "pinned" }; review: { hunks: []; annotations: [] } }>();
    const uncommitted = deferred<{ files: FileDiff[]; baseline: { kind: "pinned" }; review: { hunks: []; annotations: [] } }>();
    mocks.taskDiff.mockImplementation((_task: TaskRef, range: string) =>
      range === "uncommitted" ? uncommitted.promise : session.promise,
    );

    await render();
    await act(async () => {
      latest?.setRange("uncommitted");
      await Promise.resolve();
    });
    expect(mocks.taskDiff).toHaveBeenCalledTimes(2);

    await act(async () => {
      uncommitted.resolve({ files: [file("new-range.ts")], baseline: { kind: "pinned" }, review: { hunks: [], annotations: [] } });
      await uncommitted.promise;
    });
    await act(async () => {
      session.resolve({ files: [file("old-range.ts")], baseline: { kind: "pinned" }, review: { hunks: [], annotations: [] } });
      await session.promise;
    });

    expect(latest?.files).toEqual([file("new-range.ts")]);
  });
  it("refetches after A→B→A without publishing or overlapping the obsolete A request", async () => {
    const old = deferred<{ files: FileDiff[]; baseline: { kind: "pinned" }; review: { hunks: []; annotations: [] } }>();
    const snapshot = (path: string) => ({ files: [file(path)], baseline: { kind: "pinned" }, review: { hunks: [], annotations: [] } });
    mocks.taskDiff.mockReturnValueOnce(old.promise)
      .mockResolvedValueOnce(snapshot("B.ts"))
      .mockResolvedValueOnce(snapshot("fresh-A.ts"));
    await render({ host: "local", id: 7 });
    await render({ host: "local", id: 8 });
    await render({ host: "local", id: 7 });
    expect(mocks.taskDiff).toHaveBeenCalledTimes(2);
    expect(latest?.files).toBeNull();
    await act(async () => {
      old.resolve({ files: [file("stale-A.ts")], baseline: { kind: "pinned" }, review: { hunks: [], annotations: [] } });
      await old.promise;
    });
    expect(mocks.taskDiff).toHaveBeenCalledTimes(3);
    expect(latest?.files).toEqual([file("fresh-A.ts")]);
  });

});
