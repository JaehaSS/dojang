import { describe, expect, it } from "vitest";
import { applyStateOverrides, archivedStateRaces, noteStateOverride, type StateOverrides } from "./task-state-overlay";
import type { Task } from "./ipc";
import { LOCAL_HOST } from "./transport";

const task = (host: string, id: number, state: string, awaiting_kind: string | null = null): Task => ({
  host,
  id,
  repo: "/repo",
  branch: `task-${id}`,
  base: "main",
  worktree_path: `/wt/${id}`,
  instruction: `작업 ${id}`,
  state,
  awaiting_kind,
  created_at: 0,
  updated_at: 0,
  mode: "conversation",
});

describe("applyStateOverrides", () => {
  it("재조회 시작 이후 도착한 전이는 스냅샷의 낡은 상태를 덮는다", () => {
    const overrides: StateOverrides = new Map();
    // t=100 재조회 시작(로컬 스냅샷: 실행 중) → t=150 턴 종료(검토 대기) → t=10_000 느린 호스트 타임아웃 뒤 결과 도착
    noteStateOverride(overrides, 1, "AwaitingReview", null, 150);
    const snapshot = [task(LOCAL_HOST, 1, "Running")];

    const merged = applyStateOverrides(snapshot, overrides, 100);

    expect(merged[0].state).toBe("AwaitingReview");
    expect(snapshot[0].state).toBe("Running");
  });

  it("재조회 시작 전에 온 전이는 스냅샷이 이미 담고 있으므로 버린다", () => {
    const overrides: StateOverrides = new Map();
    noteStateOverride(overrides, 1, "AwaitingReview", "question", 50);
    // 스냅샷은 그 뒤에 읽혔고, 그 사이 다시 실행 중이 됐다(후속 메시지) — 전이 기록이 더 낡다.
    const snapshot = [task(LOCAL_HOST, 1, "Running")];

    const merged = applyStateOverrides(snapshot, overrides, 100);

    expect(merged[0].state).toBe("Running");
    expect(overrides.size).toBe(0);
  });

  it("답변 대기 ↔ 검토 대기처럼 awaiting_kind만 바뀐 전이도 덧씌운다", () => {
    const overrides: StateOverrides = new Map();
    noteStateOverride(overrides, 1, "AwaitingReview", "question", 200);
    const snapshot = [task(LOCAL_HOST, 1, "AwaitingReview", null)];

    const merged = applyStateOverrides(snapshot, overrides, 100);

    expect(merged[0].awaiting_kind).toBe("question");
  });

  it("같은 작업의 늦은 전이가 이른 전이를 대체한다", () => {
    const overrides: StateOverrides = new Map();
    noteStateOverride(overrides, 1, "AwaitingReview", null, 150);
    noteStateOverride(overrides, 1, "Running", null, 160);
    const snapshot = [task(LOCAL_HOST, 1, "AwaitingReview")];

    expect(applyStateOverrides(snapshot, overrides, 100)[0].state).toBe("Running");
  });

  it("원격 작업은 id가 같아도 건드리지 않는다 — 전이 이벤트는 로컬 것이다", () => {
    const overrides: StateOverrides = new Map();
    noteStateOverride(overrides, 1, "AwaitingReview", null, 150);
    const snapshot = [task("mini1", 1, "Running"), task(LOCAL_HOST, 2, "Running")];

    const merged = applyStateOverrides(snapshot, overrides, 100);

    expect(merged.map((t) => t.state)).toEqual(["Running", "Running"]);
  });

  it("재조회 시작과 같은 시각의 전이는 덧씌운다 — 같은 틱이면 전이 쪽이 스냅샷보다 늦을 수 있다", () => {
    const overrides: StateOverrides = new Map();
    noteStateOverride(overrides, 1, "AwaitingReview", null, 100);
    const snapshot = [task(LOCAL_HOST, 1, "Running")];

    expect(applyStateOverrides(snapshot, overrides, 100)[0].state).toBe("AwaitingReview");
  });

  it("낡은 기록은 지우고 새 기록만 남기며, 같은 호출에서 둘을 함께 다룬다", () => {
    const overrides: StateOverrides = new Map();
    noteStateOverride(overrides, 1, "AwaitingReview", null, 50);
    noteStateOverride(overrides, 2, "Running", null, 150);
    const snapshot = [task(LOCAL_HOST, 1, "Running"), task(LOCAL_HOST, 2, "AwaitingReview")];

    const merged = applyStateOverrides(snapshot, overrides, 100);

    expect(merged.map((t) => t.state)).toEqual(["Running", "Running"]);
    expect([...overrides.keys()]).toEqual([2]);
  });

  it("낡은 기록만 있었으면 지운 뒤 같은 배열을 그대로 돌려준다", () => {
    const overrides: StateOverrides = new Map();
    noteStateOverride(overrides, 1, "AwaitingReview", null, 50);
    const snapshot = [task(LOCAL_HOST, 1, "Running")];

    expect(applyStateOverrides(snapshot, overrides, 100)).toBe(snapshot);
    expect(overrides.size).toBe(0);
  });

  it("stale 행(로컬 조회 실패 → 캐시 대체)은 시각과 무관하게 덧씌우고 기록도 남긴다", () => {
    const overrides: StateOverrides = new Map();
    noteStateOverride(overrides, 1, "AwaitingReview", null, 50);
    const snapshot = [{ ...task(LOCAL_HOST, 1, "Running"), stale: true }];

    const merged = applyStateOverrides(snapshot, overrides, 100);

    expect(merged[0].state).toBe("AwaitingReview");
    expect(overrides.size).toBe(1);
  });

  it("덧씌울 것이 없으면 같은 배열을 그대로 돌려준다", () => {
    const snapshot = [task(LOCAL_HOST, 1, "Running")];
    expect(applyStateOverrides(snapshot, new Map(), 100)).toBe(snapshot);
  });

  it("스냅샷이 이미 같은 상태면 객체를 새로 만들지 않는다", () => {
    const overrides: StateOverrides = new Map();
    noteStateOverride(overrides, 1, "Running", null, 150);
    const snapshot = [task(LOCAL_HOST, 1, "Running")];

    expect(applyStateOverrides(snapshot, overrides, 100)[0]).toBe(snapshot[0]);
  });
  it("보관 스냅샷과 상태 이벤트가 경합하면 표지를 임의로 지우지 않고 재조회 대상을 반환한다", () => {
    const archive = { archived_at: 1, branch: "task-1", head_revision: null, worktree_path: "/wt/1", worktree_exists: true, result_path: null, pr_url: null };
    const overrides: StateOverrides = new Map();
    noteStateOverride(overrides, 1, "Running", null, 150);
    noteStateOverride(overrides, 1, "AwaitingReview", null, 160);
    const local = { ...task(LOCAL_HOST, 1, "AwaitingReview"), archive };
    const remote = { ...task("mini1", 1, "AwaitingReview"), archive };
    const merged = applyStateOverrides([local, remote], overrides, 100);
    expect(merged[0].archive).toBe(archive);
    expect(archivedStateRaces([local, remote], overrides, 100)).toEqual([1]);
    expect(merged[0].state).toBe("AwaitingReview");
    expect(merged[1]).toBe(remote);
    expect(local.archive).toBe(archive);
  });

});
