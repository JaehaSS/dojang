// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  fsRead: vi.fn(async () => ({
    kind: "text" as const,
    content: "agent-made",
    mtime: 1,
  })),
  fsWrite: vi.fn(async () => 2),
  fsTree: vi.fn(async () => [
    { name: "src", path: "src", is_dir: true, children: [] },
  ]),
  readLocalFile: vi.fn(async () => ({
    kind: "text" as const,
    content: "outside-worktree",
    mtime: 3,
  })),
}));

vi.mock("../../lib/ipc", () => ({
  fsRead: mocks.fsRead,
  fsWrite: mocks.fsWrite,
  fsTree: mocks.fsTree,
  readLocalFile: mocks.readLocalFile,
}));

import { useWorkspaceFiles, type WorkspaceFiles, type WorkspaceFileSource } from "./useWorkspaceFiles";
import { diffTabKey, fileTabKey } from "../../lib/tab-key";
import { resolveAgentLink } from "../../lib/agent-link";

(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

let api: WorkspaceFiles | null = null;

function Harness({
  task,
  onError = () => {},
  onFileOpened,
  onSourceChange,
  source,
}: {
  task: { host: string; id: number } | null;
  onError?: (message: string) => void;
  onFileOpened?: () => void;
  onSourceChange?: (path: string) => void;
  source?: WorkspaceFileSource | null;
}) {
  api = useWorkspaceFiles({ task, source, onError, onFileOpened, onSourceChange });
  return null;
}

const LOCAL_42 = { host: "local", id: 42 };

let container: HTMLDivElement | null = null;
let root: Root | null = null;

const render = async (props: Parameters<typeof Harness>[0]) => {
  await act(async () => {
    root?.render(<Harness {...props} />);
    await Promise.resolve();
  });
};

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root?.unmount());
  container?.remove();
  container = null;
  root = null;
  api = null;
  vi.clearAllMocks();
});

describe("useWorkspaceFiles", () => {
  it("duplicate showFile preserves the dirty buffer's external-conflict baseline", async () => {
    let disk = "base";
    const source: WorkspaceFileSource = {
      key: "project:duplicate", tree: async () => [],
      read: async () => ({ kind: "text", content: disk, mtime: 1 }),
      write: vi.fn(async () => 2),
    };
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    try {
      await render({ task: null, source });
      await act(async () => { await api!.openFile("a.ts"); });
      act(() => api!.changeFile("a.ts", "my edit"));
      disk = "external edit";
      act(() => api!.showFile("a.ts", { kind: "text", content: disk, mtime: 2 }));
      await act(async () => { await api!.saveFile("a.ts", "my edit"); });
      expect(confirm).toHaveBeenCalledOnce();
      expect(source.write).not.toHaveBeenCalled();
      expect(api!.openFiles[0]).toMatchObject({ content: "my edit", baseContent: "base", dirty: true });
    } finally { confirm.mockRestore(); }
  });

  it("an old save completion cannot alter a reopened same-path buffer", async () => {
    let finishWrite!: (mtime: number) => void;
    let disk = "base";
    const source: WorkspaceFileSource = {
      key: "project:reopen", tree: async () => [],
      read: async () => ({ kind: "text", content: disk, mtime: 1 }),
      write: () => new Promise((resolve) => { finishWrite = resolve; }),
    };
    await render({ task: null, source });
    await act(async () => { await api!.openFile("a.ts"); });
    act(() => api!.changeFile("a.ts", "old save"));
    let saving!: Promise<void>;
    await act(async () => { saving = api!.saveFile("a.ts", "old save"); await Promise.resolve(); });
    act(() => api!.closeTab(fileTabKey("a.ts")));
    disk = "reopened disk";
    await act(async () => { await api!.openFile("a.ts"); });
    act(() => api!.changeFile("a.ts", "new buffer edit"));
    await act(async () => { finishWrite(2); await saving; });
    expect(api!.openFiles[0]).toMatchObject({ content: "new buffer edit", baseContent: "reopened disk", dirty: true });
  });

  it("closing a diff during save cannot reset the live file's revision", async () => {
    let finishWrite!: (mtime: number) => void;
    const source: WorkspaceFileSource = {
      key: "project:diff-close", tree: async () => [],
      read: async () => ({ kind: "text", content: "base", mtime: 1 }),
      write: () => new Promise((resolve) => { finishWrite = resolve; }),
    };
    await render({ task: null, source });
    await act(async () => { await api!.openFile("a.ts"); });
    act(() => { api!.openDiff("a.ts"); api!.changeFile("a.ts", "old save"); });
    let saving!: Promise<void>;
    await act(async () => { saving = api!.saveFile("a.ts", "old save"); await Promise.resolve(); });
    act(() => { api!.closeTab(diffTabKey("a.ts")); api!.changeFile("a.ts", "newer edit"); });
    await act(async () => { finishWrite(2); await saving; });
    expect(api!.openFiles[0]).toMatchObject({ content: "newer edit", baseContent: "old save", dirty: true });
  });

  it("a pending reload cannot replace a reopened same-path buffer", async () => {
    let finishRead!: (file: { kind: "text"; content: string; mtime: number }) => void;
    const read = vi.fn(async () => ({ kind: "text" as const, content: "base", mtime: 1 }));
    const source: WorkspaceFileSource = { key: "project:reload", tree: async () => [], read, write: async () => 2 };
    await render({ task: null, source });
    await act(async () => { await api!.openFile("a.ts"); });
    read.mockImplementationOnce(() => new Promise((resolve) => { finishRead = resolve; }));
    let reloading!: Promise<void>;
    act(() => { reloading = api!.reloadFile("a.ts"); });
    act(() => api!.closeTab(fileTabKey("a.ts")));
    await act(async () => { await api!.openFile("a.ts"); });
    await act(async () => { finishRead({ kind: "text", content: "stale read", mtime: 0 }); await reloading; });
    expect(api!.openFiles[0]).toMatchObject({ content: "base", baseContent: "base", dirty: false });
  });

  it("a live Monaco save value becomes clean even before its change callback", async () => {
    const source: WorkspaceFileSource = {
      key: "project:live-save", tree: async () => [],
      read: async () => ({ kind: "text", content: "base", mtime: 1 }), write: async () => 2,
    };
    await render({ task: null, source });
    await act(async () => { await api!.openFile("a.ts"); await api!.saveFile("a.ts", "live value"); });
    expect(api!.openFiles[0]).toMatchObject({ content: "live value", baseContent: "live value", dirty: false });
  });

  it("source write keeps newer typing dirty while the older save completes", async () => {
    let resolveWrite: ((mtime: number) => void) | undefined;
    const source: WorkspaceFileSource = {
      key: "project:/repo",
      tree: async () => [],
      read: vi.fn(async () => ({ kind: "text" as const, content: "base", mtime: 1 })),
      write: vi.fn(() => new Promise<number>((resolve) => { resolveWrite = resolve; })),
    };
    await render({ task: null, source });
    await act(async () => { await api?.openFile("src/a.ts"); });
    act(() => api?.changeFile("src/a.ts", "first"));
    let saving: Promise<void> | undefined;
    act(() => { saving = api?.saveFile("src/a.ts", "first"); });
    await act(async () => { await Promise.resolve(); });
    act(() => api?.changeFile("src/a.ts", "newer"));
    await act(async () => { resolveWrite?.(2); await saving; });

    expect(source.write).toHaveBeenCalledWith("src/a.ts", "first", "base");
    expect(api?.openFiles[0]).toMatchObject({ content: "newer", baseContent: "first", dirty: true });
  });

  it("flush reports a newer edit so a closing window stays open", async () => {
    let resolveWrite: ((mtime: number) => void) | undefined;
    const source: WorkspaceFileSource = {
      key: "project:/repo",
      tree: async () => [],
      read: vi.fn(async () => ({ kind: "text" as const, content: "base", mtime: 1 })),
      write: vi.fn(() => new Promise<number>((resolve) => { resolveWrite = resolve; })),
    };
    await render({ task: null, source });
    await act(async () => { await api?.openFile("src/a.ts"); });
    act(() => api?.changeFile("src/a.ts", "first"));
    let flushing: ReturnType<WorkspaceFiles["flushDirty"]> | undefined;
    act(() => { flushing = api?.flushDirty(); });
    await act(async () => { await Promise.resolve(); });
    act(() => api?.changeFile("src/a.ts", "newer"));
    let result: Awaited<ReturnType<WorkspaceFiles["flushDirty"]>> | undefined;
    await act(async () => { resolveWrite?.(2); result = await flushing; });

    expect(result).toMatchObject({ ok: false, path: "src/a.ts", reason: "failed" });
    expect(api?.openFiles[0]).toMatchObject({ content: "newer", dirty: true });
  });

  it("flush rechecks an earlier file when a later file is still saving", async () => {
    let releaseB: (() => void) | undefined;
    const source: WorkspaceFileSource = {
      key: "project:/repo",
      tree: async () => [],
      read: vi.fn((path: string) => path === "b.ts"
        ? new Promise<{ kind: "text"; content: string; mtime: number }>((resolve) => { releaseB = () => resolve({ kind: "text", content: "base-b", mtime: 1 }); })
        : Promise.resolve({ kind: "text" as const, content: "base-a", mtime: 1 })),
      write: vi.fn(async () => 2),
    };
    await render({ task: null, source });
    await act(async () => { await api?.openFile("a.ts"); });
    // Opening b must not wait for its later flush read.
    source.read = vi.fn(async (path) => ({ kind: "text" as const, content: path === "b.ts" ? "base-b" : "base-a", mtime: 1 }));
    await act(async () => { await api?.openFile("b.ts"); });
    act(() => { api?.changeFile("a.ts", "first-a"); api?.changeFile("b.ts", "first-b"); });
    source.read = vi.fn((path: string) => path === "b.ts"
      ? new Promise<{ kind: "text"; content: string; mtime: number }>((resolve) => { releaseB = () => resolve({ kind: "text", content: "base-b", mtime: 1 }); })
      : Promise.resolve({ kind: "text" as const, content: "base-a", mtime: 1 }));
    let flushing: ReturnType<WorkspaceFiles["flushDirty"]> | undefined;
    act(() => { flushing = api?.flushDirty(); });
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    act(() => api?.changeFile("a.ts", "newer-a"));
    let result: Awaited<ReturnType<WorkspaceFiles["flushDirty"]>> | undefined;
    await act(async () => { releaseB?.(); result = await flushing; });

    expect(result).toMatchObject({ ok: false, path: "a.ts" });
  });

  it("파일을 열면 baseContent가 디스크 내용으로 채워진다", async () => {
    await render({ task: LOCAL_42 });
    await act(async () => {
      await api?.openFile("src/a.ts");
    });

    const file = api?.openFiles[0];
    expect(file?.content).toBe("agent-made");
    expect(file?.baseContent).toBe("agent-made");
    expect(file?.dirty).toBe(false);
    expect(api?.activeKey).toBe("src/a.ts");
  });

  it("opens an encoded external session link in a read-only editor tab", async () => {
    const target = resolveAgentLink(
      "file:///Users/test/notes%20with%20space.md:12",
      "/Users/test/.praxis/worktrees/task-42",
      { externalPaths: true },
    );
    expect(target).toEqual({
      kind: "os-path",
      path: "/Users/test/notes with space.md",
      line: 12,
      column: 1,
    });
    if (target?.kind !== "os-path") throw new Error("expected an external file target");
    await render({ task: LOCAL_42 });
    await act(async () => {
      await api?.openFile(target.path);
    });

    expect(mocks.readLocalFile).toHaveBeenCalledWith(LOCAL_42, "/Users/test/notes with space.md");
    expect(mocks.fsRead).not.toHaveBeenCalled();
    expect(api?.openFiles[0]).toMatchObject({
      path: "/Users/test/notes with space.md",
      content: "outside-worktree",
      readOnly: true,
    });
    act(() => api?.changeFile(target.path, "attempted write"));
    await act(async () => {
      await api?.saveFile(target.path, "attempted write");
      await expect(api?.flushDirty()).resolves.toEqual({ ok: true, entries: [] });
    });
    expect(mocks.fsWrite).not.toHaveBeenCalled();
    expect(api?.openFiles[0]?.dirty).toBe(false);
  });

  it("does not add an old-host read to the replacement task with the same id", async () => {
    let resolveRead:
      | ((file: { kind: "text"; content: string; mtime: number }) => void)
      | undefined;
    const onError = vi.fn();
    const onFileOpened = vi.fn();
    const onSourceChange = vi.fn();
    mocks.fsRead.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveRead = resolve;
        }),
    );
    await render({ task: LOCAL_42, onError, onFileOpened, onSourceChange });
    let pending: Promise<boolean> | undefined;
    act(() => {
      pending = api?.openFile("src/a.ts");
    });
    await render({ task: { host: "remote", id: 42 }, onError, onFileOpened, onSourceChange });
    await act(async () => {
      resolveRead?.({ kind: "text", content: "old host", mtime: 1 });
      await pending;
    });

    expect(await pending).toBe(false);
    expect(api?.openFiles).toEqual([]);
    expect(onFileOpened).not.toHaveBeenCalled();
    expect(onSourceChange).not.toHaveBeenCalled();
    expect(onError).not.toHaveBeenCalled();
  });

  it.each(["/srv/reports/note.md", "/srv/reports/page.html"])("opens and reloads %s from the SSH host as read-only", async (path) => {
    const task = { host: "ssh-host", id: 42 };
    await render({ task });
    await act(async () => { expect(await api!.openFile(path)).toBe(true); });
    expect(mocks.fsRead).toHaveBeenCalledWith(task, path);
    expect(mocks.readLocalFile).not.toHaveBeenCalled();
    expect(api!.activeFile).toMatchObject({ path, readOnly: true, content: "agent-made", dirty: false });

    act(() => api!.changeFile(path, "attempted edit"));
    await act(async () => {
      await api!.saveFile(path, "attempted edit");
      await expect(api!.flushDirty()).resolves.toEqual({ ok: true, entries: [] });
    });
    expect(mocks.fsWrite).not.toHaveBeenCalled();

    mocks.fsRead.mockResolvedValueOnce({ kind: "text", content: "updated remotely", mtime: 4 });
    await act(async () => { await api!.reloadFile(path); });
    expect(api!.activeFile).toMatchObject({ readOnly: true, content: "updated remotely" });
    expect(mocks.readLocalFile).not.toHaveBeenCalled();
  });

  it("reports a denied SSH file without adding a tab or reading the client filesystem", async () => {
    const onError = vi.fn();
    const onFileOpened = vi.fn();
    mocks.fsRead.mockRejectedValueOnce(new Error("허용되지 않는 repository 경로입니다"));
    await render({ task: { host: "ssh-host", id: 42 }, onError, onFileOpened });
    await act(async () => { expect(await api!.openFile("/etc/private.md")).toBe(false); });
    expect(api!.openFiles).toEqual([]);
    expect(onError).toHaveBeenCalledWith(expect.stringContaining("허용되지 않는 repository 경로입니다"));
    expect(onFileOpened).not.toHaveBeenCalled();
    expect(mocks.readLocalFile).not.toHaveBeenCalled();
  });

  it("drops a delayed explicit-source read after the project root changes", async () => {
    let resolveRead: ((file: { kind: "text"; content: string; mtime: number }) => void) | undefined;
    const first: WorkspaceFileSource = {
      key: "project:/one", tree: async () => [],
      read: () => new Promise((resolve) => { resolveRead = resolve; }), write: async () => 1,
    };
    const second: WorkspaceFileSource = {
      key: "project:/two", tree: async () => [],
      read: async () => ({ kind: "text", content: "two", mtime: 1 }), write: async () => 1,
    };
    await render({ task: null, source: first });
    let pending: Promise<boolean> | undefined;
    act(() => { pending = api?.openFile("a.ts"); });
    await render({ task: null, source: second });
    await act(async () => { resolveRead?.({ kind: "text", content: "one", mtime: 1 }); await pending; });

    expect(await pending).toBe(false);
    expect(api?.openFiles).toEqual([]);
  });

  it("load and edit notify the source-version adapter", async () => {
    const onSourceChange = vi.fn();
    await render({ task: LOCAL_42, onSourceChange });
    await act(async () => {
      await api?.openFile("src/a.ts");
    });
    await act(async () => api?.changeFile("src/a.ts", "my-edit"));
    expect(onSourceChange).toHaveBeenNthCalledWith(1, "src/a.ts");
    expect(onSourceChange).toHaveBeenNthCalledWith(2, "src/a.ts");
  });

  it("closing an open source notifies query invalidation", async () => {
    const onSourceChange = vi.fn();
    await render({ task: LOCAL_42, onSourceChange });
    await act(async () => {
      await api?.openFile("src/a.ts");
    });
    act(() => api?.closeTab(fileTabKey("src/a.ts")));

    expect(onSourceChange).toHaveBeenNthCalledWith(2, "src/a.ts");
  });

  it("편집해도 baseContent는 그대로다", async () => {
    // 이게 이 훅의 요점이다. content만 두면 편집 순간 디스크 원본이 사라져
    // "에이전트가 만든 마지막 상태"를 가리킬 기준점이 없어진다.
    await render({ task: LOCAL_42 });
    await act(async () => {
      await api?.openFile("src/a.ts");
    });
    await act(async () => api?.changeFile("src/a.ts", "my-edit"));

    const file = api?.openFiles[0];
    expect(file?.content).toBe("my-edit");
    expect(file?.baseContent).toBe("agent-made");
    expect(file?.dirty).toBe(true);
  });

  it("저장하면 baseContent가 방금 쓴 내용으로 옮겨간다", async () => {
    await render({ task: LOCAL_42 });
    await act(async () => {
      await api?.openFile("src/a.ts");
    });
    await act(async () => api?.changeFile("src/a.ts", "my-edit"));
    await act(async () => {
      await api?.saveFile("src/a.ts", "my-edit");
    });

    const file = api?.openFiles[0];
    expect(file?.baseContent).toBe("my-edit"); // 디스크가 이제 이것이다
    expect(file?.dirty).toBe(false);
    expect(file?.mtime).toBe(2);
  });

  it("외부 변경이 없으면 덮어쓰기를 묻지 않는다", async () => {
    // 가드가 편집값(content)이 아니라 baseContent를 기준으로 판정한다. 편집값과 비교하면
    // dirty 파일은 정의상 항상 달라서 저장할 때마다 모달이 떴다.
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    await render({ task: LOCAL_42 });
    await act(async () => {
      await api?.openFile("src/a.ts");
    });
    await act(async () => api?.changeFile("src/a.ts", "my-edit"));
    await act(async () => {
      await api?.saveFile("src/a.ts", "my-edit");
    });

    expect(confirm).not.toHaveBeenCalled();
    expect(mocks.fsWrite).toHaveBeenCalledWith(
      { host: "local", id: 42 },
      "src/a.ts",
      "my-edit",
    );
    confirm.mockRestore();
  });

  it("디스크가 그사이 바뀌었으면 저장 전에 묻는다", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    await render({ task: LOCAL_42 });
    await act(async () => {
      await api?.openFile("src/a.ts");
    });
    await act(async () => api?.changeFile("src/a.ts", "my-edit"));
    mocks.fsRead.mockResolvedValueOnce({
      kind: "text",
      content: "agent-changed",
      mtime: 9,
    });
    await act(async () => {
      await api?.saveFile("src/a.ts", "my-edit");
    });

    expect(confirm).toHaveBeenCalledOnce();
    expect(mocks.fsWrite).not.toHaveBeenCalled();
    confirm.mockRestore();
  });

  describe("flushDirty", () => {
    it("dirty가 없으면 쓸 것도 되돌릴 것도 없다", async () => {
      await render({ task: LOCAL_42 });
      await act(async () => {
        await api?.openFile("src/a.ts");
      });

      let result:
        Awaited<ReturnType<NonNullable<typeof api>["flushDirty"]>> | undefined;
      await act(async () => {
        result = await api?.flushDirty();
      });

      expect(result).toEqual({ ok: true, entries: [] });
      expect(mocks.fsWrite).not.toHaveBeenCalled();
    });

    it("저장하고 디스크에 있던 내용을 되돌릴 지점으로 넘긴다", async () => {
      await render({ task: LOCAL_42 });
      await act(async () => {
        await api?.openFile("src/a.ts");
      });
      await act(async () => api?.changeFile("src/a.ts", "my-edit"));

      let result:
        Awaited<ReturnType<NonNullable<typeof api>["flushDirty"]>> | undefined;
      await act(async () => {
        result = await api?.flushDirty();
      });

      expect(result).toEqual({
        ok: true,
        entries: [{ path: "src/a.ts", content: "agent-made" }],
      });
      expect(mocks.fsWrite).toHaveBeenCalledWith(
        { host: "local", id: 42 },
        "src/a.ts",
        "my-edit",
      );
      expect(api?.openFiles[0]?.dirty).toBe(false);
    });

    it("충돌이면 덮어쓰지 않고 멈춘다", async () => {
      // 에이전트가 같은 파일을 고쳤다는 뜻이다. 여기서 쓰면 그 작업이 사라진다.
      await render({ task: LOCAL_42 });
      await act(async () => {
        await api?.openFile("src/a.ts");
      });
      await act(async () => api?.changeFile("src/a.ts", "my-edit"));
      mocks.fsRead.mockResolvedValueOnce({
        kind: "text",
        content: "agent-changed",
        mtime: 9,
      });

      let result:
        Awaited<ReturnType<NonNullable<typeof api>["flushDirty"]>> | undefined;
      await act(async () => {
        result = await api?.flushDirty();
      });

      expect(result).toEqual({
        ok: false,
        path: "src/a.ts",
        reason: "conflict",
        detail: null,
      });
      expect(mocks.fsWrite).not.toHaveBeenCalled();
      expect(api?.openFiles[0]?.dirty).toBe(true);
    });

    it("쓰기가 실패하면 그대로 알린다", async () => {
      await render({ task: LOCAL_42 });
      await act(async () => {
        await api?.openFile("src/a.ts");
      });
      await act(async () => api?.changeFile("src/a.ts", "my-edit"));
      mocks.fsWrite.mockRejectedValueOnce(new Error("read-only"));

      let result:
        Awaited<ReturnType<NonNullable<typeof api>["flushDirty"]>> | undefined;
      await act(async () => {
        result = await api?.flushDirty();
      });

      expect(result?.ok).toBe(false);
      if (result && !result.ok) {
        expect(result.reason).toBe("failed");
        expect(result.detail).toContain("read-only");
      }
    });
  });

  it("다시 읽으면 baseContent도 디스크를 따라간다", async () => {
    await render({ task: LOCAL_42 });
    await act(async () => {
      await api?.openFile("src/a.ts");
    });
    await act(async () => api?.changeFile("src/a.ts", "my-edit"));
    mocks.fsRead.mockResolvedValueOnce({
      kind: "text",
      content: "agent-changed",
      mtime: 3,
    });
    await act(async () => {
      await api?.reloadFile("src/a.ts");
    });

    const file = api?.openFiles[0];
    expect(file?.content).toBe("agent-changed");
    expect(file?.baseContent).toBe("agent-changed");
    expect(file?.dirty).toBe(false);
  });

  it("does not invalidate the replacement task when an old-host reload finishes", async () => {
    const onSourceChange = vi.fn();
    await render({ task: LOCAL_42, onSourceChange });
    await act(async () => {
      await api?.openFile("src/a.ts");
    });
    onSourceChange.mockClear();
    let resolveRead:
      | ((file: { kind: "text"; content: string; mtime: number }) => void)
      | undefined;
    mocks.fsRead.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveRead = resolve;
        }),
    );
    let pending: Promise<void> | undefined;
    act(() => {
      pending = api?.reloadFile("src/a.ts");
    });
    await render({ task: { host: "remote", id: 42 }, onSourceChange });
    await act(async () => {
      resolveRead?.({ kind: "text", content: "old host", mtime: 2 });
      await pending;
    });

    expect(onSourceChange).not.toHaveBeenCalled();
  });

  it("읽기에 실패하면 열림을 알리지 않고 오류만 전한다", async () => {
    // 기존 동작 보존 — showFile이 성공 경로에만 있어 실패 시 탭이 전환되지 않았다.
    const onError = vi.fn();
    const onFileOpened = vi.fn();
    await render({ task: LOCAL_42, onError, onFileOpened });
    mocks.fsRead.mockRejectedValueOnce(new Error("no such file"));
    await act(async () => {
      await api?.openFile("src/missing.ts");
    });

    expect(onFileOpened).not.toHaveBeenCalled();
    expect(onError).toHaveBeenCalledOnce();
    expect(api?.openFiles).toHaveLength(0);
  });

  it("이미 열린 파일을 다시 열면 다시 읽지 않고 활성만 옮긴다", async () => {
    const onFileOpened = vi.fn();
    await render({ task: LOCAL_42, onFileOpened });
    await act(async () => {
      await api?.openFile("src/a.ts");
    });
    await act(async () => {
      await api?.openFile("src/b.ts");
    });
    mocks.fsRead.mockClear();
    await act(async () => {
      await api?.openFile("src/a.ts");
    });

    expect(mocks.fsRead).not.toHaveBeenCalled();
    expect(api?.activeKey).toBe("src/a.ts");
    expect(onFileOpened).toHaveBeenCalledTimes(3);
  });

  it("트리에서 이미 열린 파일을 고르면 분할 배치에 보낼 새 요청을 남긴다", async () => {
    await render({ task: LOCAL_42 });
    await act(async () => { await api!.openFile("src/a.ts"); });
    await act(async () => { await api!.openFile("src/a.ts", { preview: true, tree: true }); });

    expect(api!.treeOpen).toEqual({ key: fileTabKey("src/a.ts"), request: 1 });
  });

  it("분할 뷰가 소비한 트리 열기 요청은 다시 전달하지 않는다", async () => {
    await render({ task: LOCAL_42 });
    await act(async () => { await api!.openFile("src/a.ts", { preview: true, tree: true }); });
    act(() => api!.consumeTreeOpen(1));

    expect(api!.treeOpen).toBeNull();
  });

  it("새 활성 탭이 생기면 소비되지 않은 트리 열기 요청을 버린다", async () => {
    await render({ task: LOCAL_42 });
    await act(async () => { await api!.openFile("src/a.ts", { preview: true, tree: true }); });
    await act(async () => { await api!.openFile("src/b.ts"); });

    expect(api!.treeOpen).toBeNull();
  });

  it("탭을 닫으면 마지막 남은 파일이 활성이 된다", async () => {
    await render({ task: LOCAL_42 });
    await act(async () => {
      await api?.openFile("src/a.ts");
    });
    await act(async () => {
      await api?.openFile("src/b.ts");
    });
    await act(async () => api?.closeTab(fileTabKey("src/b.ts")));

    expect(api?.openFiles).toHaveLength(1);
    expect(api?.activeKey).toBe("src/a.ts");
  });

  it("작업이 바뀌면 파일 목록과 활성 경로를 비운다", async () => {
    await render({ task: LOCAL_42 });
    await act(async () => {
      await api?.openFile("src/a.ts");
    });
    await render({ task: { host: "local", id: 43 } });

    expect(api?.openFiles).toHaveLength(0);
    expect(api?.activeKey).toBeNull();
  });

  it("작업이 없으면 트리를 비우고 조회하지 않는다", async () => {
    await render({ task: null });
    await act(async () => api?.refreshTree());

    expect(mocks.fsTree).not.toHaveBeenCalled();
    expect(api?.tree).toEqual([]);
  });
});

describe("프리뷰 탭", () => {
  const paths = () => api?.openFiles.map((f) => f.path) ?? [];
  /** 훑어보기 표시가 붙은 파일들 — 자리를 물려주는 일 자체는 배치가 한다(ADR 0189). */
  const previews = () => api?.openFiles.filter((f) => f.preview).map((f) => f.path) ?? [];

  beforeEach(() => localStorage.clear());

  it("훑어보기로 연 파일에는 표시만 붙는다 — 밀어내지 않는다", async () => {
    await render({ task: LOCAL_42 });
    await act(async () => {
      await api?.openFile("src/a.ts", { preview: true });
    });
    await act(async () => {
      await api?.openFile("src/b.ts", { preview: true });
    });

    expect(paths()).toEqual(["src/a.ts", "src/b.ts"]);
    expect(previews()).toEqual(["src/a.ts", "src/b.ts"]);
  });

  it("does not seat a delayed preview after the task scope changes", async () => {
    let resolveRead:
      | ((file: { kind: "text"; content: string; mtime: number }) => void)
      | undefined;
    mocks.fsRead.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveRead = resolve;
        }),
    );
    await render({ task: LOCAL_42 });
    let pending: Promise<boolean> | undefined;
    act(() => {
      pending = api?.openFile("src/a.ts", { preview: true });
    });
    await render({ task: { host: "remote", id: 42 } });
    await act(async () => {
      resolveRead?.({ kind: "text", content: "old host", mtime: 1 });
      await pending;
    });

    expect(await pending).toBe(false);
    expect(paths()).toEqual([]);
  });

  it("고정하고 나면 다음 파일이 밀어내지 못한다", async () => {
    await render({ task: LOCAL_42 });
    await act(async () => {
      await api?.openFile("src/a.ts", { preview: true });
    });
    act(() => api?.pinTab(fileTabKey("src/a.ts")));
    await act(async () => {
      await api?.openFile("src/b.ts", { preview: true });
    });

    expect(paths()).toEqual(["src/a.ts", "src/b.ts"]);
    expect(previews()).toEqual(["src/b.ts"]);
  });

  it("편집하면 스스로 고정된다 — 저장 안 된 것이 밀려나면 안 된다", async () => {
    await render({ task: LOCAL_42 });
    await act(async () => {
      await api?.openFile("src/a.ts", { preview: true });
    });
    act(() => api?.changeFile("src/a.ts", "고친 내용"));
    expect(previews()).toEqual([]);

    await act(async () => {
      await api?.openFile("src/b.ts", { preview: true });
    });
    expect(paths()).toEqual(["src/a.ts", "src/b.ts"]);
    expect(previews()).toEqual(["src/b.ts"]);
  });

  it("목적이 분명한 열기는 훑어보기가 아니다 — 정의 이동·복원 경로", async () => {
    await render({ task: LOCAL_42 });
    await act(async () => {
      await api?.openFile("src/a.ts", { preview: true });
    });
    await act(async () => {
      await api?.openFile("src/b.ts");
    });

    expect(paths()).toEqual(["src/a.ts", "src/b.ts"]);
    // 표시는 그대로 a.ts만 쥐고 있다 — 다음 훑어보기가 그 자리를 물려받는다.
    expect(previews()).toEqual(["src/a.ts"]);
  });

  it("꺼 두면 훑어보기 요청도 그냥 탭을 연다", async () => {
    localStorage.setItem("praxis:preview-tabs", "off");
    await render({ task: LOCAL_42 });
    await act(async () => {
      await api?.openFile("src/a.ts", { preview: true });
    });
    await act(async () => {
      await api?.openFile("src/b.ts", { preview: true });
    });

    expect(paths()).toEqual(["src/a.ts", "src/b.ts"]);
    expect(previews()).toEqual([]);
  });

  it("기능을 끄면 남아 있던 표시를 한 번에 걷는다", async () => {
    await render({ task: LOCAL_42 });
    await act(async () => {
      await api?.openFile("src/a.ts", { preview: true });
    });
    await act(async () => {
      await api?.openFile("src/b.ts", { preview: true });
    });
    act(() => api?.pinAll());

    expect(paths()).toEqual(["src/a.ts", "src/b.ts"]);
    expect(previews()).toEqual([]);
  });
});

describe("diff 탭", () => {
  it("같은 파일의 파일 탭과 나란히 열린다 — 키가 다르기 때문이다", async () => {
    await render({ task: LOCAL_42 });
    await act(async () => {
      await api?.openFile("src/a.ts");
    });
    act(() => api?.openDiff("src/a.ts"));

    expect(api?.openFiles.map((f) => f.key)).toEqual([
      fileTabKey("src/a.ts"),
      diffTabKey("src/a.ts"),
    ]);
    expect(api?.activeKey).toBe(diffTabKey("src/a.ts"));
  });

  // F-10 — 반대 순서. diff 탭을 먼저 열어 두면 `openFile`이 그것을 "이미 열림"으로 오인해
  // 트리 클릭이 아무 일도 하지 않던 것이 이 재설계의 출발점이다(DR-3).
  it("diff 탭이 있어도 트리에서 연 파일은 따로 열린다 (AC-4 · F-10)", async () => {
    await render({ task: LOCAL_42 });
    act(() => api?.openDiff("src/a.ts"));
    await act(async () => {
      await api?.openFile("src/a.ts");
    });

    expect(api?.openFiles.map((f) => f.key)).toEqual([
      diffTabKey("src/a.ts"),
      fileTabKey("src/a.ts"),
    ]);
    expect(api?.activeKey).toBe(fileTabKey("src/a.ts"));
  });

  it("한쪽을 닫아도 다른 쪽은 남는다 (AC-4)", async () => {
    await render({ task: LOCAL_42 });
    await act(async () => {
      await api?.openFile("src/a.ts");
    });
    act(() => api?.openDiff("src/a.ts"));
    act(() => api?.closeTab(diffTabKey("src/a.ts")));

    expect(api?.openFiles.map((f) => f.key)).toEqual([fileTabKey("src/a.ts")]);
  });

  it("디스크를 읽지 않는다 — 본문은 변경분이지 파일이 아니다", async () => {
    await render({ task: LOCAL_42 });
    mocks.fsRead.mockClear();
    act(() => api?.openDiff("src/a.ts"));

    expect(mocks.fsRead).not.toHaveBeenCalled();
    const tab = api?.openFiles[0];
    expect(tab?.kind).toBe("diff");
    expect(tab?.dirty).toBe(false);
  });

  it("파일을 편집해도 diff 탭이 dirty가 되지 않는다", async () => {
    await render({ task: LOCAL_42 });
    await act(async () => {
      await api?.openFile("src/a.ts");
    });
    act(() => api?.openDiff("src/a.ts"));
    act(() => api?.changeFile("src/a.ts", "고친 내용"));

    expect(api?.openFiles.map((f) => f.dirty)).toEqual([true, false]);
  });

  it("한 번 클릭은 프리뷰 자리를 돌려 쓰고 코드 열을 부른다 (AC-3)", async () => {
    const onFileOpened = vi.fn();
    await render({ task: LOCAL_42, onFileOpened });
    act(() => api?.openDiff("src/a.ts", { preview: true }));
    act(() => api?.openDiff("src/b.ts", { preview: true }));

    expect(api?.openFiles.map((f) => f.key)).toEqual([
      diffTabKey("src/a.ts"),
      diffTabKey("src/b.ts"),
    ]);
    expect(api?.openFiles.map((f) => f.preview)).toEqual([true, true]);
    expect(onFileOpened).toHaveBeenCalledTimes(2);
  });

  it("고정한 diff 탭은 다음 클릭에 밀려나지 않는다 (AC-3)", async () => {
    await render({ task: LOCAL_42 });
    act(() => api?.openDiff("src/a.ts", { preview: true }));
    act(() => api?.pinTab(diffTabKey("src/a.ts")));
    act(() => api?.openDiff("src/b.ts", { preview: true }));

    expect(api?.openFiles.map((f) => f.key)).toEqual([
      diffTabKey("src/a.ts"),
      diffTabKey("src/b.ts"),
    ]);
  });

  it("파일이 사라지면 두 탭을 함께 닫는다", async () => {
    await render({ task: LOCAL_42 });
    await act(async () => {
      await api?.openFile("src/a.ts");
    });
    act(() => api?.openDiff("src/a.ts"));
    await act(async () => {
      await api?.openFile("src/b.ts");
    });
    act(() => api?.closeTabsForPath("src/a.ts"));

    expect(api?.openFiles.map((f) => f.key)).toEqual([fileTabKey("src/b.ts")]);
    expect(api?.activeKey).toBe(fileTabKey("src/b.ts"));
  });
});
