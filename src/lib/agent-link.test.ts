import { describe, expect, it } from "vitest";
import { resolveAgentLink } from "./agent-link";

const worktree = "/Users/test/.praxis/worktrees/task-42";

describe("resolveAgentLink", () => {
  it("maps an encoded file URI inside the task worktree to a relative file", () => {
    expect(
      resolveAgentLink(
        "file:///Users/test/.praxis/worktrees/task-42/reports/Top-5%20%ED%83%9C%EA%B9%85.xlsx",
        worktree,
      ),
    ).toEqual({ kind: "task-file", path: "reports/Top-5 태깅.xlsx" });
  });

  it("accepts task-relative links from Markdown answers", () => {
    expect(resolveAgentLink("./reports/result.csv", worktree)).toEqual({
      kind: "task-file",
      path: "reports/result.csv",
    });
  });

  it("splits a line suffix off the path so the editor can land on it", () => {
    expect(resolveAgentLink("src/App.tsx:1187", worktree)).toEqual({
      kind: "task-file",
      path: "src/App.tsx",
      line: 1187,
      column: 1,
    });
    expect(resolveAgentLink("src/lib.rs:20:5", worktree)).toEqual({
      kind: "task-file",
      path: "src/lib.rs",
      line: 20,
      column: 5,
    });
  });

  it("keeps a web link's port out of the line suffix", () => {
    expect(resolveAgentLink("https://example.com:8080/report", worktree)).toEqual({
      kind: "external-url",
      url: "https://example.com:8080/report",
    });
  });

  it("keeps web links external", () => {
    expect(resolveAgentLink("https://example.com/report", worktree)).toEqual({
      kind: "external-url",
      url: "https://example.com/report",
    });
  });

  it("rejects files outside the task worktree and unsafe schemes", () => {
    expect(resolveAgentLink("file:///Users/test/secrets.xlsx", worktree)).toBeNull();
    expect(resolveAgentLink("../secrets.xlsx", worktree)).toBeNull();
    expect(resolveAgentLink("javascript:alert(1)", worktree)).toBeNull();
  });

  const local = { homePath: "/Users/test", externalPaths: true };

  it("routes SSH links to the remote filesystem and preserves line coordinates", () => {
    const remote = { remotePaths: true, homePath: "/Users/client" };
    expect(resolveAgentLink("/srv/work/docs/a.md:9:2", "/srv/work", remote)).toEqual({
      kind: "task-file", path: "docs/a.md", line: 9, column: 2,
    });
    expect(resolveAgentLink("file:///srv/reports/%ED%95%9C%EA%B8%80%20report.html:12", "/srv/work", remote)).toEqual({
      kind: "remote-file", path: "/srv/reports/한글 report.html", line: 12, column: 1,
    });
    expect(resolveAgentLink("/srv/reports/note.md", "/srv/work", remote)).toEqual({
      kind: "remote-file", path: "/srv/reports/note.md",
    });
    for (const path of ["~/note.md", "../note.md", "/srv/../etc/passwd", "/srv/%00.md", "file://other/srv/note.md", "//other/share/note.md"]) {
      expect(resolveAgentLink(path, "/srv/work", remote), path).toBeNull();
    }
  });

  it("expands a tilde path outside the worktree into an OS path", () => {
    expect(resolveAgentLink("~/work/docs/a.md", worktree, local)).toEqual({
      kind: "os-path",
      path: "/Users/test/work/docs/a.md",
    });
  });

  it("keeps a tilde path closed without a home directory", () => {
    expect(resolveAgentLink("~/work/docs/a.md", worktree, { externalPaths: true })).toBeNull();
  });

  it("keeps OS paths closed unless the client has real paths", () => {
    expect(resolveAgentLink("~/work/docs/a.md", worktree, { homePath: "/Users/test" })).toBeNull();
    expect(resolveAgentLink("~/work/docs/a.md", worktree)).toBeNull();
  });

  it("prefers the editor tab when a tilde path lands inside the worktree", () => {
    expect(resolveAgentLink("~/.praxis/worktrees/task-42/src/App.tsx:10", worktree, local)).toEqual({
      kind: "task-file",
      path: "src/App.tsx",
      line: 10,
      column: 1,
    });
  });

  it("carries the line suffix onto an OS path", () => {
    expect(resolveAgentLink("~/notes.md:12", worktree, local)).toEqual({
      kind: "os-path",
      path: "/Users/test/notes.md",
      line: 12,
      column: 1,
    });
  });

  it("rejects another user's home and escapes through the home directory", () => {
    expect(resolveAgentLink("~root/x.md", worktree, local)).toBeNull();
    expect(resolveAgentLink("~/../etc/passwd", worktree, local)).toBeNull();
  });

  it("opens absolute and file-URI paths outside the worktree as OS paths", () => {
    expect(resolveAgentLink("/Users/other/x.md", worktree, { externalPaths: true })).toEqual({
      kind: "os-path",
      path: "/Users/other/x.md",
    });
    expect(
      resolveAgentLink("file:///Users/test/secrets.xlsx", worktree, { externalPaths: true }),
    ).toEqual({ kind: "os-path", path: "/Users/test/secrets.xlsx" });
  });

  it("still refuses relative paths that escape the worktree", () => {
    expect(resolveAgentLink("../secrets.xlsx", worktree, local)).toBeNull();
  });

  it("decodes a percent-encoded tilde href into a vault file", () => {
    // 렌더러가 href의 공백·한글을 퍼센트 인코딩해 넘긴다(`~/Documents/째하 지식창고/…`).
    const href = `~/${encodeURI("Documents/째하 지식창고/웹어셈블리/README.md")}`;
    expect(resolveAgentLink(href, "/Users/test/Documents/째하 지식창고", local)).toEqual({
      kind: "task-file",
      path: "웹어셈블리/README.md",
    });
  });
});
