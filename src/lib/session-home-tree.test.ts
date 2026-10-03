import { describe, expect, it } from "vitest";
import type { SessionHomeSession } from "./transport";
import {
  buildSessionHomeTree,
  initiallyCollapsedProjects,
  mergeSessionHomeResults,
  projectOf,
  sessionHomeRows,
  UNKNOWN_PROJECT_ID,
} from "./session-home-tree";

function session(id: string, overrides: Partial<SessionHomeSession> = {}): SessionHomeSession {
  return {
    session_id: id,
    cwd: "/work/app",
    last_cwd: null,
    git_branch: null,
    title: `세션 ${id}`,
    first_message: null,
    last_active: 100,
    messages: 1,
    vendor_version: null,
    host: "local",
    ...overrides,
  };
}

describe("projectOf", () => {
  it("등록 프로젝트의 최장 접두를 고른다 — 워크트리도 프로젝트에 속한다", () => {
    const projects = ["/work", "/work/app"];
    expect(projectOf(session("a", { cwd: "/work/app/.praxis/worktrees/x" }), projects)).toBe("/work/app");
    expect(projectOf(session("b", { cwd: "/work/other" }), projects)).toBe("/work");
  });

  it("등록되지 않은 cwd는 그 자체가 프로젝트이고, 끝 슬래시는 무시한다", () => {
    expect(projectOf(session("a", { cwd: "/tmp/probe/" }), ["/work"])).toBe("/tmp/probe");
    expect(projectOf(session("b", { cwd: "/workspace" }), ["/work"])).toBe("/workspace");
  });

  it("cwd가 없으면 last_cwd, 둘 다 없으면 null", () => {
    expect(projectOf(session("a", { cwd: null, last_cwd: "/x" }), [])).toBe("/x");
    expect(projectOf(session("b", { cwd: null, last_cwd: null }), [])).toBeNull();
  });
});

describe("mergeSessionHomeResults", () => {
  it("session_id로 합치고 첫 등장 순서를 지킨다", () => {
    const merged = mergeSessionHomeResults(
      [session("1"), session("2")],
      [session("3"), session("1", { title: "중복" })],
    );
    expect(merged.map((s) => s.session_id)).toEqual(["1", "2", "3"]);
    expect(merged[0].title).toBe("세션 1");
  });

  it("다른 공급자의 같은 session_id는 별도 세션으로 보존한다", () => {
    const merged = mergeSessionHomeResults(
      [session("same", { vendor: "claude" })],
      [session("same", { vendor: "codex" })],
    );
    expect(merged).toHaveLength(2);
    expect(merged.map((item) => item.vendor)).toEqual(["claude", "codex"]);
  });
});

describe("buildSessionHomeTree", () => {
  it("현재 저장소가 맨 앞, 나머지는 최근 활동순, 경로 없음이 맨 뒤", () => {
    const tree = buildSessionHomeTree(
      [
        session("old", { cwd: "/work/lib", last_active: 10 }),
        session("new", { cwd: "/work/tool", last_active: 900 }),
        session("cur", { cwd: "/work/app/.praxis/worktrees/t", last_active: 1 }),
        session("lost", { cwd: null, last_cwd: null }),
      ],
      "/work/app",
      ["/work/lib", "/work/tool"],
    );
    expect(tree.map((node) => node.id)).toEqual([
      "project:/work/app",
      "project:/work/tool",
      "project:/work/lib",
      UNKNOWN_PROJECT_ID,
    ]);
    expect(tree[0].label).toBe("app");
    expect(tree[0].detail).toBe("/work/app");
    expect(tree[0].children[0].detail).toBe(".praxis/worktrees/t");
    expect(tree[3].label).toBe("(경로 없음)");
  });

  it("프로젝트 안의 세션 순서는 입력 순서(서버의 최근순)를 그대로 둔다", () => {
    const tree = buildSessionHomeTree(
      [session("b", { last_active: 5 }), session("a", { last_active: 50 })],
      "/work/app",
      [],
    );
    expect(tree[0].children.map((c) => c.id)).toEqual(["session:local:claude:b", "session:local:claude:a"]);
    expect(tree[0].children[0].detail).toBeNull();
  });

  it("제목이 없으면 첫 메시지, 그것도 없으면 자리표시 라벨", () => {
    const tree = buildSessionHomeTree(
      [session("a", { title: null, first_message: " 첫 줄 " }), session("b", { title: null })],
      "/work/app",
      [],
    );
    expect(tree[0].children.map((c) => c.label)).toEqual(["첫 줄", "(제목 없음)"]);
  });
});

describe("sessionHomeRows / initiallyCollapsedProjects", () => {
  const tree = buildSessionHomeTree(
    [session("a", { cwd: "/work/app" }), session("b", { cwd: "/work/other" })],
    "/work/app",
    [],
  );

  it("처음에는 현재 저장소만 펼친다", () => {
    const collapsed = initiallyCollapsedProjects(tree, "/work/app/");
    expect([...collapsed]).toEqual(["project:/work/other"]);
    const rows = sessionHomeRows(tree, collapsed, false);
    expect(rows.map((r) => `${r.depth}:${r.id}`)).toEqual([
      "0:project:/work/app",
      "1:session:local:claude:a",
      "0:project:/work/other",
    ]);
  });

  it("검색 중에는 접힘을 무시하고 전부 편다", () => {
    const rows = sessionHomeRows(tree, new Set(["project:/work/app", "project:/work/other"]), true);
    expect(rows.filter((r) => r.kind === "session")).toHaveLength(2);
  });
});
