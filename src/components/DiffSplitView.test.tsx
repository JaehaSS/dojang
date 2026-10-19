import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { DiffHunk, RematchedAnnotation } from "../lib/ipc";
import { AnnotatedSplitDiff } from "./DiffSplitView";

const hunk: DiffHunk = {
  id: "h1",
  path: "a.ts",
  old_range: [10, 3],
  new_range: [10, 3],
  protected: false,
    committed: false,
  risk: "low",
  lines: [
    { kind: "context", text: "const a = 1;" },
    { kind: "del", text: "const b = 2;" },
    { kind: "add", text: "const b = 3;" },
  ],
};

const annotation = (over: Partial<RematchedAnnotation> = {}): RematchedAnnotation =>
  ({
    id: "a1",
    path: "a.ts",
    line: 11,
    side: "old",
    body_md: "여기 왜 바뀌었나요",
    status: "draft",
    orphaned: false,
    matched_hunk_id: "h1",
    ...over,
  }) as RematchedAnnotation;

function render(props: Partial<Parameters<typeof AnnotatedSplitDiff>[0]> = {}) {
  return renderToStaticMarkup(
    <AnnotatedSplitDiff
      hunks={[hunk]}
      annotations={[]}
      onCreate={vi.fn()}
      onUpdateBody={vi.fn()}
      {...props}
    />,
  );
}

describe("AnnotatedSplitDiff", () => {
  it("offers an annotation gutter on every side cell", () => {
    const html = render();
    expect(html).toContain("주석 남기기");
  });

  it("renders the partial-apply checkbox on the hunk header", () => {
    const html = render({ selectedHunkIds: new Set(["h1"]), onToggleHunk: vi.fn() });
    expect(html).toContain('type="checkbox"');
    expect(html).toContain('checked=""');
  });

  it("disables the checkbox for protected hunks and explains why", () => {
    const html = render({
      hunks: [{ ...hunk, protected: true }],
      selectedHunkIds: new Set<string>(),
      onToggleHunk: vi.fn(),
    });
    expect(html).toContain("disabled");
    expect(html).toContain("protected 경로 변경");
  });

  it("omits checkboxes entirely when partial apply is not active", () => {
    expect(render()).not.toContain('type="checkbox"');
  });

  it("renders an existing thread anchored to the deleted side", () => {
    const html = render({ annotations: [annotation()] });
    expect(html).toContain("여기 왜 바뀌었나요");
    expect(html).toContain("초안");
  });

  it("renders a context-row thread once even though both sides share the cell", () => {
    const html = render({
      annotations: [annotation({ id: "a2", line: 10, side: "new", body_md: "컨텍스트 주석" })],
    });
    expect(html.split("컨텍스트 주석")).toHaveLength(2);
  });

  it("keeps the hunk header addressable for keyboard jumps", () => {
    expect(render()).toContain('data-hunk-id="h1"');
  });

  it("numbers the left column from old_range and the right from new_range", () => {
    // 앞선 hunk에서 줄이 늘거나 줄면 old/new 시작점이 갈라진다. context 행은 좌우가 같은
    // cell 객체를 공유하므로, side를 모르면 양쪽 모두 old 번호를 찍게 된다.
    const html = render({
      hunks: [{ ...hunk, old_range: [10, 3], new_range: [40, 3] }],
    });
    const numbers = [...html.matchAll(/select-none">(\d+)</g)].map((m) => m[1]);
    // context(10|40), del(11|—), add(—|41)
    expect(numbers).toEqual(["10", "40", "11", "41"]);
  });

  it("tints only the changed span when a paired line differs by one token", () => {
    const html = render();
    expect(html).toContain("bg-delbg-strong");
    expect(html).toContain("bg-addbg-strong");
  });

  it("leaves a wholesale rewrite without span tints", () => {
    const html = render({
      hunks: [
        {
          ...hunk,
          lines: [
            { kind: "del", text: "alpha beta" },
            { kind: "add", text: "totally different text" },
          ],
        },
      ],
    });
    expect(html).not.toContain("bg-addbg-strong");
  });

  it("wraps long lines inside each half instead of clipping them", () => {
    const html = render();
    expect(html).toContain("whitespace-pre-wrap break-all");
    expect(html).not.toContain("whitespace-pre ");
    expect(html).not.toContain("overflow-hidden");
  });
});
