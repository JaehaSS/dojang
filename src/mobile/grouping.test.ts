import { describe, expect, it } from "vitest";
import type { Task } from "../lib/ipc";
import { finishedTasks, groupByRepo, isActive, repoName } from "./grouping";

function task(id: number, repo: string, state: string, updated_at = 0): Task {
  return {
    id,
    host: "local",
    repo,
    branch: "b",
    base: "main",
    worktree_path: "/wt",
    instruction: `작업 ${id}`,
    state,
    created_at: 0,
    updated_at,
    mode: "terminal",
  };
}

describe("repoName", () => {
  it("경로의 마지막 세그먼트를 쓴다", () => {
    expect(repoName("/home/dev/praxis-build")).toBe("praxis-build");
    expect(repoName("C:\\Users\\me\\praxis")).toBe("praxis");
    expect(repoName("/trailing/slash/")).toBe("slash");
    expect(repoName("bare")).toBe("bare");
  });
});

describe("isActive", () => {
  it("종료 상태만 제외한다", () => {
    for (const state of ["Created", "Queued", "Running", "AwaitingReview", "Finalizing"]) {
      expect(isActive(state)).toBe(true);
    }
    for (const state of ["Done", "Discarded", "Failed"]) {
      expect(isActive(state)).toBe(false);
    }
  });
});

describe("groupByRepo", () => {
  it("프로젝트별로 묶고 종료 작업은 뺀다", () => {
    const groups = groupByRepo([
      task(1, "/a", "Running", 10),
      task(2, "/a", "Queued", 20),
      task(3, "/b", "Running", 30),
      task(4, "/a", "Done", 99),
    ]);
    expect(groups.map((g) => [g.name, g.tasks.length])).toEqual([
      ["b", 1],
      ["a", 2],
    ]);
  });

  it("행동이 필요한 프로젝트를 위로 올린다", () => {
    // 최근 활동이 더 늦어도, 내 결정을 기다리는 쪽이 먼저다.
    const groups = groupByRepo([
      task(1, "/recent", "Running", 100),
      task(2, "/waiting", "AwaitingReview", 10),
    ]);
    expect(groups.map((g) => g.name)).toEqual(["waiting", "recent"]);
    expect(groups[0].actionable).toBe(1);
    expect(groups[1].actionable).toBe(0);
  });

  it("행동 필요 여부가 같으면 최근 활동 순", () => {
    const groups = groupByRepo([
      task(1, "/old", "AwaitingReview", 10),
      task(2, "/new", "AwaitingReview", 50),
    ]);
    expect(groups.map((g) => g.name)).toEqual(["new", "old"]);
  });

  it("개수가 많다고 최신을 밀어내지 않는다", () => {
    // 오래된 검토 대기 3건이 방금 온 1건을 계속 위에 두면 새 알림이 묻힌다.
    const groups = groupByRepo([
      task(1, "/stale", "AwaitingReview", 1),
      task(2, "/stale", "AwaitingReview", 2),
      task(3, "/stale", "AwaitingReview", 3),
      task(4, "/fresh", "AwaitingReview", 100),
    ]);
    expect(groups[0].name).toBe("fresh");
  });

  it("동률이면 이름순으로 고정한다", () => {
    // 새로고침마다 순서가 흔들리면 눈이 길을 잃는다.
    const groups = groupByRepo([task(1, "/zeta", "Running", 5), task(2, "/alpha", "Running", 5)]);
    expect(groups.map((g) => g.name)).toEqual(["alpha", "zeta"]);
  });

  it("경로가 다르면 이름이 같아도 합치지 않는다", () => {
    const groups = groupByRepo([
      task(1, "/one/praxis", "Running", 10),
      task(2, "/two/praxis", "Running", 20),
    ]);
    expect(groups).toHaveLength(2);
    expect(groups.map((g) => g.repo)).toEqual(["/two/praxis", "/one/praxis"]);
  });

  it("섹션 안은 행동 필요 순으로 정렬한다", () => {
    const groups = groupByRepo([
      task(1, "/a", "Running", 100),
      task(2, "/a", "AwaitingReview", 1),
    ]);
    expect(groups[0].tasks.map((t) => t.id)).toEqual([2, 1]);
  });

  it("빈 입력은 빈 배열", () => {
    expect(groupByRepo([])).toEqual([]);
    expect(groupByRepo([task(1, "/a", "Done")])).toEqual([]);
  });
});

describe("finishedTasks", () => {
  it("종료 작업만 최근 순으로 모은다", () => {
    const list = finishedTasks([
      task(1, "/a", "Done", 10),
      task(2, "/b", "Running", 99),
      task(3, "/a", "Failed", 30),
      task(4, "/b", "Discarded", 20),
    ]);
    expect(list.map((t) => t.id)).toEqual([3, 4, 1]);
  });

  it("상한을 넘기지 않는다", () => {
    const many = Array.from({ length: 30 }, (_, i) => task(i, "/a", "Done", i));
    expect(finishedTasks(many, 5)).toHaveLength(5);
    expect(finishedTasks(many, 5)[0].id).toBe(29);
  });
});
