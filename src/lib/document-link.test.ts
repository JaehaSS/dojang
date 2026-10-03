import { describe, expect, it } from "vitest";
import { resolveDocumentLink } from "./document-link";

describe("resolveDocumentLink", () => {
  const source = "docs/discovery/example.md";

  it("resolves a document-relative target inside the workspace", () => {
    expect(resolveDocumentLink("../../DESIGN%20%ED%95%9C%EA%B8%80.md:4#intro", source)).toEqual({
      kind: "file",
      path: "DESIGN 한글.md",
    });
  });

  it("keeps HTTP(S) links as browser targets", () => {
    expect(resolveDocumentLink("https://example.com/docs#part", source)).toEqual({
      kind: "url",
      url: "https://example.com/docs#part",
    });
  });

  it("does not make anchors, unsupported schemes, decoded controls, tilde paths, or workspace escapes into file targets", () => {
    for (const link of ["#part", "javascript:alert(1)", "file:///Users/test/private.md", "file%00.md", "~/private.md", "../../../private.md"]) {
      expect(resolveDocumentLink(link, source)).toBeNull();
    }
  });

  it("resolves an absolute link that sits inside the root", () => {
    expect(resolveDocumentLink("/work/root/docs/a.md:3#x", source, "/work/root")).toEqual({
      kind: "file",
      path: "docs/a.md",
    });
    expect(resolveDocumentLink("/work/root/docs/a.md", source, "/work/root/")).toEqual({
      kind: "file",
      path: "docs/a.md",
    });
    expect(resolveDocumentLink("/work/root/docs/%ED%95%9C%EA%B8%80.md", source, "/work/root")).toEqual({
      kind: "file",
      path: "docs/한글.md",
    });
  });

  it("does not open absolute links outside the root, or any absolute link without one", () => {
    for (const link of ["/etc/passwd", "/work/root/../etc/passwd", "/work/root2/a.md"]) {
      expect(resolveDocumentLink(link, source, "/work/root")).toBeNull();
    }
    expect(resolveDocumentLink("/work/root/docs/a.md", source)).toBeNull();
  });

  it("rejects a workspace-relative source path that is not normalized", () => {
    expect(resolveDocumentLink("notes.md", "docs/../example.md")).toBeNull();
    expect(resolveDocumentLink("notes.md", "docs//example.md")).toBeNull();
  });

  describe("source document opened from outside the root (absolute tab path)", () => {
    const external = "/Users/test/notes/README.md";

    it("resolves a relative link next to that document as an absolute path", () => {
      expect(resolveDocumentLink("GLOSSARY.md", external, "/work/root")).toEqual({
        kind: "file",
        path: "/Users/test/notes/GLOSSARY.md",
      });
      expect(resolveDocumentLink("./sub/%ED%95%9C%EA%B8%80.md:3#x", external)).toEqual({
        kind: "file",
        path: "/Users/test/notes/sub/한글.md",
      });
      expect(resolveDocumentLink("../shared/x.md", external, "/work/root")).toEqual({
        kind: "file",
        path: "/Users/test/shared/x.md",
      });
    });

    it("turns a target that lands inside the root back into a root-relative path", () => {
      expect(resolveDocumentLink("GLOSSARY.md", "/work/root/docs/README.md", "/work/root")).toEqual({
        kind: "file",
        path: "docs/GLOSSARY.md",
      });
      expect(resolveDocumentLink("../../work/root/a.md", "/Users/test/notes/README.md", "/work/root")).toEqual({
        kind: "file",
        path: "/Users/work/root/a.md",
      });
      expect(resolveDocumentLink("../../../work/root/a.md", "/Users/test/notes/README.md", "/work/root")).toEqual({
        kind: "file",
        path: "a.md",
      });
    });

    it("handles Windows drive paths with either separator", () => {
      expect(resolveDocumentLink("GLOSSARY.md", "C:\\Users\\t\\README.md")).toEqual({
        kind: "file",
        path: "C:/Users/t/GLOSSARY.md",
      });
    });

    it("does not climb above the filesystem root or resolve to a directory", () => {
      expect(resolveDocumentLink("../../../../x.md", "/a/b/c.md")).toBeNull();
      expect(resolveDocumentLink("..", "/a/b.md")).toBeNull();
      expect(resolveDocumentLink("#part", external)).toBeNull();
      expect(resolveDocumentLink("~/private.md", external)).toBeNull();
    });
  });
});
