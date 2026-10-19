import { describe, it, expect } from "vitest";
import {
  draftAnnotationIds,
  groupAnnotationsByPosition,
  hunkLines,
  isAgentAnnotation,
  orphanedAnnotations,
  positionKey,
} from "./annotations";
import type { DiffHunk, RematchedAnnotation } from "./ipc";

function hunk(overrides: Partial<DiffHunk> = {}): DiffHunk {
  return {
    id: "h1",
    path: "src/auth/login.ts",
    old_range: [9, 2],
    new_range: [9, 3],
    lines: [
      { kind: "context", text: "function login() {" },
      { kind: "add", text: "if (!token) throw new AuthError()" },
      { kind: "context", text: "}" },
    ],
    protected: false,
    committed: false,
    risk: "low",
    ...overrides,
  };
}

function annotation(overrides: Partial<RematchedAnnotation> = {}): RematchedAnnotation {
  return {
    id: "ann-1",
    task_id: 1,
    hunk_id: "h1",
    path: "src/auth/login.ts",
    line: 10,
    side: "new",
    body_md: "코멘트",
    status: "draft",
    created_at: 1000,
    matched_hunk_id: "h1",
    orphaned: false,
    ...overrides,
  };
}

describe("hunkLines", () => {
  it("assigns old/new line numbers matching the server's quote_line counting", () => {
    const lines = hunkLines(hunk());
    expect(lines).toEqual([
      { kind: "context", text: "function login() {", oldLine: 9, newLine: 9, side: "new" },
      { kind: "add", text: "if (!token) throw new AuthError()", oldLine: null, newLine: 10, side: "new" },
      { kind: "context", text: "}", oldLine: 10, newLine: 11, side: "new" },
    ]);
  });

  it("assigns old-only line numbers for del lines", () => {
    const del = hunk({
      old_range: [5, 1],
      new_range: [5, 0],
      lines: [{ kind: "del", text: "legacy()" }],
    });
    expect(hunkLines(del)).toEqual([
      { kind: "del", text: "legacy()", oldLine: 5, newLine: null, side: "old" },
    ]);
  });
});

describe("groupAnnotationsByPosition", () => {
  it("groups matched annotations by hunk/line/side and excludes orphaned ones", () => {
    const matched = annotation({ id: "a" });
    const sameLine = annotation({ id: "b" });
    const orphan = annotation({ id: "c", orphaned: true, matched_hunk_id: null });
    const groups = groupAnnotationsByPosition([matched, sameLine, orphan]);

    const key = positionKey("h1", 10, "new");
    expect(groups.get(key)?.map((a) => a.id)).toEqual(["a", "b"]);
    expect(groups.size).toBe(1);
  });
});

describe("orphanedAnnotations / draftAnnotationIds", () => {
  it("filters orphaned and draft annotations independently", () => {
    const draft = annotation({ id: "a", status: "draft" });
    const sent = annotation({ id: "b", status: "sent" });
    const orphan = annotation({ id: "c", orphaned: true, matched_hunk_id: null, status: "draft" });

    expect(orphanedAnnotations([draft, sent, orphan]).map((a) => a.id)).toEqual(["c"]);
    expect(draftAnnotationIds([draft, sent, orphan])).toEqual(["a", "c"]);
  });
});

describe("agent notes", () => {
  it("treats a missing author as human and any other author as an agent", () => {
    expect(isAgentAnnotation(annotation())).toBe(false);
    expect(isAgentAnnotation(annotation({ author: "human" }))).toBe(false);
    expect(isAgentAnnotation(annotation({ author: "claude" }))).toBe(true);
  });

  it("never resends an agent note, even one stored as draft", () => {
    const human = annotation({ id: "a", status: "draft", author: "human" });
    const legacy = annotation({ id: "b", status: "draft" });
    const agentSent = annotation({ id: "c", status: "sent", author: "codex" });
    const agentDraft = annotation({ id: "d", status: "draft", author: "claude" });

    expect(draftAnnotationIds([human, legacy, agentSent, agentDraft])).toEqual(["a", "b"]);
  });
});
