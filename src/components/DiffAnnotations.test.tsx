// @vitest-environment jsdom

import { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";
import { setDiffTokenizer } from "../lib/diff-highlight";
import { getActiveTheme } from "../lib/themes";
import type { DiffHunk, RematchedAnnotation } from "../lib/ipc";
import { AnnotatedUnifiedDiff } from "./DiffAnnotations";

const hunk: DiffHunk = {
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
};

function annotation(overrides: Partial<RematchedAnnotation> = {}): RematchedAnnotation {
  return {
    id: "ann-1",
    task_id: 1,
    hunk_id: "h1",
    path: "src/auth/login.ts",
    line: 10,
    side: "new",
    body_md: "에러 코드는 상수로 빼줘",
    status: "draft",
    created_at: 1000,
    matched_hunk_id: "h1",
    orphaned: false,
    ...overrides,
  };
}

describe("AnnotatedUnifiedDiff", () => {
  it("renders hunk lines with the added text and an existing comment thread", () => {
    const html = renderToStaticMarkup(
      <AnnotatedUnifiedDiff
        hunks={[hunk]}
        annotations={[annotation()]}
        onCreate={async () => {}}
        onUpdateBody={async () => {}}
      />,
    );

    expect(html).toContain("if (!token) throw new AuthError()");
    expect(html).toContain("에러 코드는 상수로 빼줘");
    expect(html).toContain("초안");
    expect(html).toContain("💬");
  });

  it("labels an agent note with its author instead of a status and offers no edit button", () => {
    const html = renderToStaticMarkup(
      <AnnotatedUnifiedDiff
        hunks={[hunk]}
        annotations={[
          annotation({ id: "ann-agent", status: "sent", author: "claude", body_md: "null 체크가 빠졌다" }),
          annotation({ id: "ann-human", status: "draft", author: "human" }),
        ]}
        onCreate={async () => {}}
        onUpdateBody={async () => {}}
      />,
    );

    expect(html).toContain("에이전트 · claude");
    expect(html).toContain("null 체크가 빠졌다");
    expect(html).not.toContain("전송됨");
    expect(html).toContain("초안");
    expect(html.match(/>수정</g)).toHaveLength(1);
  });

  it("shows an orphaned banner without dropping the annotation", () => {
    const html = renderToStaticMarkup(
      <AnnotatedUnifiedDiff
        hunks={[hunk]}
        annotations={[annotation({ id: "ann-2", orphaned: true, matched_hunk_id: null, status: "sent" })]}
        onCreate={async () => {}}
        onUpdateBody={async () => {}}
      />,
    );

    expect(html).toContain("고아 주석 1건");
    expect(html).toContain("src/auth/login.ts:10");
  });

  it("renders a hunk checkbox that is checked when selected, and disabled with a reason when protected", () => {
    const protectedHunk: DiffHunk = { ...hunk, id: "h2", protected: true };
    const html = renderToStaticMarkup(
      <AnnotatedUnifiedDiff
        hunks={[hunk, protectedHunk]}
        annotations={[]}
        onCreate={async () => {}}
        onUpdateBody={async () => {}}
        selectedHunkIds={new Set(["h1"])}
        onToggleHunk={() => {}}
      />,
    );

    expect(html).toContain('type="checkbox"');
    expect(html).toContain("checked=\"\"");
    expect(html).toContain("disabled=\"\"");
    expect(html).toContain("부분 적용으로 유지할 수 없습니다");
  });

  it("omits checkboxes entirely when selectedHunkIds is not provided", () => {
    const html = renderToStaticMarkup(
      <AnnotatedUnifiedDiff hunks={[hunk]} annotations={[]} onCreate={async () => {}} onUpdateBody={async () => {}} />,
    );

    expect(html).not.toContain('type="checkbox"');
  });

  it("shows the omitted line count between distant hunks", () => {
    const later: DiffHunk = {
      ...hunk,
      id: "h-later",
      old_range: [30, 2],
      new_range: [31, 2],
    };
    const html = renderToStaticMarkup(
      <AnnotatedUnifiedDiff
        hunks={[hunk, later]}
        annotations={[]}
        onCreate={async () => {}}
        onUpdateBody={async () => {}}
      />,
    );

    expect(html).toContain("19개 변경되지 않은 줄");
  });

  describe("syntax highlighting", () => {
    afterEach(() => setDiffTokenizer(null));

    it("colors keywords once a tokenizer is registered, using the stream each line belongs to", async () => {
      (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
      const calls: { lines: string[]; lang: string }[] = [];
      // 줄 첫 단어를 키워드로 친다 — 스트림 구성과 줄 매핑만 확인하면 되므로 실제 문법은 필요 없다.
      setDiffTokenizer((lines, lang) => {
        calls.push({ lines, lang });
        return lines.map((line) => {
          const end = line.search(/\W|$/);
          return end > 0 ? [{ start: 0, end, type: `keyword.${lang}` }] : [];
        });
      });
      const container = document.createElement("div");
      const root = createRoot(container);
      await act(async () => {
        root.render(
          <AnnotatedUnifiedDiff
            hunks={[hunk]}
            annotations={[]}
            onCreate={async () => {}}
            onUpdateBody={async () => {}}
            lang="typescript"
          />,
        );
      });
      expect(calls).toEqual([
        { lines: ["function login() {", "}"], lang: "typescript" },
        { lines: ["function login() {", "if (!token) throw new AuthError()", "}"], lang: "typescript" },
      ]);
      const colored = [...container.querySelectorAll<HTMLSpanElement>("span[style]")];
      expect(colored.map((span) => span.textContent)).toEqual(["function", "if"]);
      expect(colored[0].style.color).not.toBe("");
      expect(getActiveTheme()).toBeDefined();
      await act(async () => root.unmount());
    });

    it("renders plain text when no tokenizer is registered", () => {
      const html = renderToStaticMarkup(
        <AnnotatedUnifiedDiff
          hunks={[hunk]}
          annotations={[]}
          onCreate={async () => {}}
          onUpdateBody={async () => {}}
          lang="typescript"
        />,
      );
      expect(html).not.toContain("style=");
    });
  });
});
