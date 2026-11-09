import { describe, expect, it } from "vitest";
import type { Task } from "./ipc";
import { RECENT_SESSION_WINDOW_SEC, recentSessions } from "./recent-sessions";
import { taskKey } from "./transport";

const NOW = 1_700_000_000;

const task = (overrides: Partial<Task> = {}): Task => ({
  id: 1,
  host: "local",
  repo: "/workspace/praxis",
  branch: "feature/one",
  base: "main",
  worktree_path: "/workspace/praxis/.praxis/worktrees/task-1",
  instruction: "작업 하나",
  state: "AwaitingReview",
  created_at: NOW - 10_000,
  updated_at: NOW,
  mode: "conversation",
  ...overrides,
});

describe("recentSessions", () => {
  it("빈 입력에는 빈 배열", () => {
    expect(recentSessions([], NOW)).toEqual([]);
  });

  // 경계는 포함이다 — "1시간 이내"라고 말하면서 정확히 1시간을 떨어뜨리면 목록이 깜박인다.
  it("창 경계에서 정확히 1시간은 포함하고 1초 더 지난 것은 뺀다", () => {
    const onEdge = task({ id: 1, updated_at: NOW - RECENT_SESSION_WINDOW_SEC });
    const pastEdge = task({ id: 2, updated_at: NOW - RECENT_SESSION_WINDOW_SEC - 1 });

    expect(recentSessions([onEdge, pastEdge], NOW).map((t) => t.id)).toEqual([1]);
  });

  it("창을 인자로 좁힐 수 있다", () => {
    const tasks = [task({ id: 1, updated_at: NOW - 30 }), task({ id: 2, updated_at: NOW - 90 })];

    expect(recentSessions(tasks, NOW, 60).map((t) => t.id)).toEqual([1]);
  });

  // 종료된 작업은 "방금까지 대화하던 세션"이 아니다 — 트리와 같은 집합을 본다.
  it("Done·Discarded는 창 안이라도 제외한다", () => {
    const tasks = [
      task({ id: 1, state: "Done" }),
      task({ id: 2, state: "Discarded" }),
      task({ id: 3, state: "Running" }),
    ];

    expect(recentSessions(tasks, NOW).map((t) => t.id)).toEqual([3]);
  });

  it("보관된 작업은 같은 id의 원격 작업과 무관하게 최근 목록에서 제외한다", () => {
    const tasks = [
      task({ id: 3, host: "local", archive: {
        archived_at: NOW,
        branch: "feature/local",
        head_revision: null,
        worktree_path: "/workspace/praxis/.dojang/worktrees/task-3",
        worktree_exists: true,
        result_path: null,
        pr_url: null,
      } }),
      task({ id: 3, host: "runner-1" }),
    ];

    expect(recentSessions(tasks, NOW).map(taskKey)).toEqual(["runner-1:3"]);
  });

  it("updated_at 내림차순, 같으면 id 내림차순으로 정렬한다", () => {
    const tasks = [
      task({ id: 1, updated_at: NOW - 100 }),
      task({ id: 5, updated_at: NOW - 10 }),
      task({ id: 9, updated_at: NOW - 100 }),
      task({ id: 3, updated_at: NOW - 10 }),
    ];

    expect(recentSessions(tasks, NOW).map((t) => t.id)).toEqual([5, 3, 9, 1]);
  });

  it("입력 배열을 변형하지 않는다", () => {
    const tasks = [task({ id: 1, updated_at: NOW - 100 }), task({ id: 2, updated_at: NOW - 10 })];
    const before = tasks.map((t) => t.id);

    recentSessions(tasks, NOW);

    expect(tasks.map((t) => t.id)).toEqual(before);
  });

  // 로컬 3번과 원격 3번은 서로 다른 작업이다(ADR 0133) — id만 보고 합치거나 걸러내면 안 된다.
  it("host가 다르면 같은 id도 각각 남긴다", () => {
    const tasks = [
      task({ id: 3, host: "local", updated_at: NOW - 20 }),
      task({ id: 3, host: "workstation", updated_at: NOW - 10 }),
    ];

    expect(recentSessions(tasks, NOW).map(taskKey)).toEqual(["workstation:3", "local:3"]);
  });
});
