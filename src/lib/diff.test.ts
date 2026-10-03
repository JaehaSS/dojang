import { describe, it, expect } from "vitest";
import type { DiffHunk } from "./ipc";
import {
  collapsedLineCount,
  describePatch,
  intraLineMap,
  intraLineSegments,
  lineClass,
  splitFilePath,
  summarizePatch,
  toSplitRows,
  toSplitRowsFromHunks,
} from "./diff";

describe("lineClass", () => {
  it("colors add/del/hunk/context", () => {
    expect(lineClass("+added")).toContain("status-done");
    expect(lineClass("-removed")).toContain("status-failed");
    expect(lineClass("@@ -1 +1 @@")).toContain("primary-bright");
    expect(lineClass(" context")).toContain("text-secondary");
  });
});

describe("toSplitRows", () => {
  it("pairs del/add and aligns context", () => {
    const patch = ["diff --git a/x b/x", "@@ -1,2 +1,2 @@", " ctx", "-old", "+new"].join("\n");
    const rows = toSplitRows(patch);
    expect(rows[0].left.kind).toBe("hunk");
    const ctx = rows.find((r) => r.left.kind === "ctx")!;
    expect(ctx.left.text).toBe("ctx");
    expect(ctx.right.text).toBe("ctx");
    const change = rows.find((r) => r.left.kind === "del")!;
    expect(change.left.text).toBe("old");
    expect(change.right.text).toBe("new");
    expect(change.right.kind).toBe("add");
  });

  it("pads unequal del/add runs with empty", () => {
    const patch = ["@@ -1,2 +1,1 @@", "-a", "-b", "+c"].join("\n");
    const rows = toSplitRows(patch).filter((r) => r.left.kind !== "hunk");
    expect(rows.length).toBe(2);
    expect(rows[0].left.text).toBe("a");
    expect(rows[0].right.text).toBe("c");
    expect(rows[1].left.text).toBe("b");
    expect(rows[1].right.kind).toBe("empty");
  });

  it("skips meta lines and tracks new-file additions", () => {
    const patch = [
      "diff --git a/x b/x",
      "index 111..222 100644",
      "--- /dev/null",
      "+++ b/x",
      "@@ -0,0 +1 @@",
      "+only",
    ].join("\n");
    const adds = toSplitRows(patch).filter((r) => r.right.kind === "add");
    expect(adds.length).toBe(1);
    expect(adds[0].right.text).toBe("only");
    expect(adds[0].left.kind).toBe("empty");
  });

  it("marks omitted context between hunks as a collapsed row", () => {
    const patch = [
      "@@ -1,2 +1,2 @@",
      " one",
      "-two",
      "+TWO",
      "@@ -21,2 +21,2 @@",
      " twenty-one",
      "-twenty-two",
      "+TWENTY-TWO",
    ].join("\n");
    const gap = toSplitRows(patch).find((row) => row.left.kind === "gap");
    expect(gap?.left.text).toBe("18개 변경되지 않은 줄");
  });
});

describe("diff summaries", () => {
  it("counts content lines without counting file headers", () => {
    const patch = ["--- a/file.ts", "+++ b/file.ts", "@@ -1 +1,2 @@", "-old", "+new", "+next"].join("\n");
    expect(summarizePatch(patch)).toEqual({ additions: 2, deletions: 1 });
  });

  it("describes non-text patches instead of rendering an empty pane", () => {
    expect(describePatch("Binary files a/logo.png and b/logo.png differ")).toContain("바이너리");
    expect(describePatch("Praxis diff omitted: file exceeds 1 MiB")).toContain("1 MiB");
    expect(describePatch("similarity index 100%\nrename from old.ts\nrename to new.ts")).toContain(
      "old.ts → new.ts",
    );
  });

  it("calculates omitted lines from consecutive hunk ranges", () => {
    expect(collapsedLineCount([1, 2], [21, 2])).toBe(18);
  });
});

const hunk = (over: Partial<DiffHunk> = {}): DiffHunk => ({
  id: "h1",
  committed: false,
  path: "a.ts",
  old_range: [10, 3],
  new_range: [10, 3],
  protected: false,
  risk: "low",
  lines: [
    { kind: "context", text: "const a = 1;" },
    { kind: "del", text: "const b = 2;" },
    { kind: "add", text: "const b = 3;" },
  ],
  ...over,
});

describe("splitFilePath", () => {
  it("separates the file name from its directory", () => {
    expect(splitFilePath("src/components/ide/Composer.tsx")).toEqual({
      name: "Composer.tsx",
      dir: "src/components/ide",
    });
  });

  it("handles a root-level file", () => {
    expect(splitFilePath("README.md")).toEqual({ name: "README.md", dir: "" });
  });

  it("keeps a trailing-slash path from losing its name", () => {
    expect(splitFilePath("src/")).toEqual({ name: "", dir: "src" });
  });
});

describe("intraLineSegments", () => {
  it("marks only the changed middle span", () => {
    expect(intraLineSegments("const a = 1;", "const a = 2;")?.after).toEqual([
      { text: "const a = ", changed: false },
      { text: "2", changed: true },
      { text: ";", changed: false },
    ]);
  });

  it("reports the removed span on the before side", () => {
    expect(intraLineSegments("const a = 1;", "const a = 2;")?.before).toEqual([
      { text: "const a = ", changed: false },
      { text: "1", changed: true },
      { text: ";", changed: false },
    ]);
  });

  it("expands to the word start so suffix edits do not flicker", () => {
    expect(intraLineSegments("useState", "useStates")?.after).toEqual([
      { text: "useStates", changed: true },
    ]);
  });

  it("keeps a pure insertion anchored to the shared prefix", () => {
    const result = intraLineSegments("a.b(1)", "a.b(1, 2)");
    expect(result?.after.map((segment) => segment.text).join("")).toBe("a.b(1, 2)");
    expect(result?.before.some((segment) => segment.changed)).toBe(false);
  });

  it("returns null when the line is rewritten wholesale", () => {
    expect(intraLineSegments("alpha beta", "totally different text")).toBeNull();
  });

  it("returns null for identical text", () => {
    expect(intraLineSegments("same", "same")).toBeNull();
  });

  it("stops the word expansion at a non-word boundary instead of eating the line", () => {
    const changed = intraLineSegments("x.foobar", "x.foobaz")?.after.find((s) => s.changed);
    expect(changed?.text).toBe("foobaz");
  });

  it("gives up rather than cutting a surrogate pair in half", () => {
    // 🙂(U+1F642)와 🙃(U+1F643)은 상위 서로게이트가 같아 경계가 글자 한가운데 떨어진다.
    expect(intraLineSegments("🙂 ok", "🙃 ok")).toBeNull();
  });

  it("still highlights when emoji sit outside the changed span", () => {
    const result = intraLineSegments("🙂 count = 1", "🙂 count = 2");
    expect(result?.after.find((s) => s.changed)?.text).toBe("2");
  });

  it("never overlaps the prefix and suffix scans on repeated text", () => {
    const result = intraLineSegments("aaa", "aaaa");
    expect(result?.after.map((segment) => segment.text).join("")).toBe("aaaa");
    expect(result?.before.map((segment) => segment.text).join("")).toBe("aaa");
  });
});

describe("intraLineMap", () => {
  const line = (kind: "context" | "add" | "del", text: string) =>
    ({ kind, text, oldLine: null, newLine: null, side: "new" }) as const;

  it("pairs the k-th deletion with the k-th addition", () => {
    const map = intraLineMap([
      line("del", "const a = 1;"),
      line("del", "const b = 1;"),
      line("add", "const a = 2;"),
      line("add", "const b = 2;"),
    ]);
    expect(map.get(0)?.find((segment) => segment.changed)?.text).toBe("1");
    expect(map.get(2)?.find((segment) => segment.changed)?.text).toBe("2");
    expect(map.get(3)?.find((segment) => segment.changed)?.text).toBe("2");
  });

  it("leaves unpaired lines out of the map", () => {
    const map = intraLineMap([line("del", "const a = 1;"), line("add", "const a = 2;"), line("add", "extra")]);
    expect(map.has(2)).toBe(false);
  });

  it("skips wholesale rewrites so the line keeps only its row tint", () => {
    const map = intraLineMap([line("del", "alpha beta"), line("add", "totally different text")]);
    expect(map.size).toBe(0);
  });

  it("ignores addition-only blocks", () => {
    expect(intraLineMap([line("context", "x"), line("add", "y")]).size).toBe(0);
  });
});

describe("toSplitRowsFromHunks", () => {
  it("pairs del/add lines and carries the hunk id for annotations", () => {
    const pairs = toSplitRowsFromHunks([hunk()]).filter((row) => row.kind === "pair");
    expect(pairs).toHaveLength(2);

    expect(pairs[0].left).toMatchObject({
      hunkId: "h1",
      line: { kind: "context", oldLine: 10, newLine: 10, side: "new" },
    });
    expect(pairs[1].left).toMatchObject({ hunkId: "h1", line: { kind: "del", oldLine: 11, side: "old" } });
    expect(pairs[1].right).toMatchObject({ hunkId: "h1", line: { kind: "add", newLine: 11, side: "new" } });
  });

  it("shares one cell object for context rows so threads render once", () => {
    const pairs = toSplitRowsFromHunks([hunk()]).filter((row) => row.kind === "pair");
    expect(pairs[0].left).toBe(pairs[0].right);
    expect(pairs[1].left).not.toBe(pairs[1].right);
  });

  it("leaves the opposite cell null when one side is shorter", () => {
    const pairs = toSplitRowsFromHunks([
      hunk({
        lines: [
          { kind: "del", text: "x" },
          { kind: "del", text: "y" },
          { kind: "add", text: "z" },
        ],
      }),
    ]).filter((row) => row.kind === "pair");
    expect(pairs[1].left).toMatchObject({ line: { text: "y" } });
    expect(pairs[1].right).toBeNull();
  });

  it("emits a gap row between non-adjacent hunks", () => {
    const rows = toSplitRowsFromHunks([
      hunk({ id: "h1", old_range: [1, 2] }),
      hunk({ id: "h2", old_range: [40, 2] }),
    ]);
    const gaps = rows.filter((row) => row.kind === "gap");
    expect(gaps).toHaveLength(1);
    expect(gaps[0]).toMatchObject({ text: "37개 변경되지 않은 줄" });
  });

  it("emits no gap row for adjacent hunks", () => {
    const rows = toSplitRowsFromHunks([
      hunk({ id: "h1", old_range: [1, 3] }),
      hunk({ id: "h2", old_range: [4, 3] }),
    ]);
    expect(rows.filter((row) => row.kind === "gap")).toHaveLength(0);
  });

  it("handles an addition-only block with empty left cells", () => {
    const pairs = toSplitRowsFromHunks([
      hunk({
        lines: [
          { kind: "add", text: "first" },
          { kind: "add", text: "second" },
        ],
      }),
    ]).filter((row) => row.kind === "pair");
    expect(pairs).toHaveLength(2);
    expect(pairs.every((row) => row.left === null)).toBe(true);
    expect(pairs.map((row) => row.right?.line.text)).toEqual(["first", "second"]);
  });

  it("keeps a hunk header row that exposes the hunk for checkbox binding", () => {
    const headers = toSplitRowsFromHunks([hunk()]).filter((row) => row.kind === "hunk");
    expect(headers).toHaveLength(1);
    expect(headers[0]).toMatchObject({ text: "@@ -10,3 +10,3 @@" });
    expect(headers[0].kind === "hunk" && headers[0].hunk.id).toBe("h1");
  });
});
