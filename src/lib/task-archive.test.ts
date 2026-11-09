import { describe, expect, it, vi } from "vitest";
import type { Task } from "./ipc";
import { canArchiveTask, isTaskArchived, taskArchive, taskRestore, type TaskArchive } from "./task-archive";

const transport = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: transport.invoke }));

const archive: TaskArchive = {
  archived_at: 1_700_000_000,
  branch: "feature/archived",
  head_revision: "a1b2c3d4",
  worktree_path: "/workspace/praxis/.dojang/worktrees/task-1",
  worktree_exists: true,
  result_path: null,
  pr_url: null,
};

const task = (overrides: Partial<Task> = {}): Task => ({
  id: 1,
  host: "local",
  repo: "/workspace/praxis",
  branch: "feature/one",
  base: "main",
  worktree_path: "/workspace/praxis/.dojang/worktrees/task-1",
  instruction: "작업 하나",
  state: "AwaitingReview",
  created_at: 1,
  updated_at: 1,
  mode: "conversation",
  ...overrides,
});

describe("task archive", () => {
  it("보관과 복원은 로컬 Tauri 명령으로 id만 전달한다", () => {
    taskArchive(7);
    taskRestore(8);

    expect(transport.invoke).toHaveBeenNthCalledWith(1, "task_archive", { id: 7 });
    expect(transport.invoke).toHaveBeenNthCalledWith(2, "task_restore", { id: 8 });
  });

  it("로컬의 종료 가능한 작업만 보관할 수 있다", () => {
    expect(canArchiveTask(task())).toBe(true);
    expect(canArchiveTask(task({ state: "Done" }))).toBe(true);
    expect(canArchiveTask(task({ state: "Failed" }))).toBe(true);
    expect(canArchiveTask(task({ state: "Discarded" }))).toBe(true);
    expect(canArchiveTask(task({ state: "Running" }))).toBe(false);
  });

  it("원격 작업, 살아 있는 대화 프로세스, 이미 보관한 작업은 보관할 수 없다", () => {
    expect(canArchiveTask(task({ host: "runner-1" }))).toBe(false);
    expect(canArchiveTask(task({ stale: true }))).toBe(false);
    expect(canArchiveTask(task({ convo_pgid: 1234 }))).toBe(false);
    expect(canArchiveTask(task({ archive }))).toBe(false);
  });

  it("보관 정보가 있는 작업만 보관 목록의 작업으로 좁힌다", () => {
    const candidate = task({ archive });
    expect(isTaskArchived(candidate)).toBe(true);
    if (isTaskArchived(candidate)) expect(candidate.archive.branch).toBe("feature/archived");
    expect(isTaskArchived(task())).toBe(false);
  });
});
