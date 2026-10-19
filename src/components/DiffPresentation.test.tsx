import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { getTheme } from "../lib/themes";
import { DiffFileHeader, DiffLineText, DiffModeToggle, SplitDiff } from "./DiffPresentation";

describe("Diff presentation", () => {
  it("renders Korean unified and split controls with explicit pressed state", () => {
    const html = renderToStaticMarkup(<DiffModeToggle mode="split" onChange={() => {}} />);
    expect(html).toContain("통합");
    expect(html).toContain("분할");
    expect(html).toContain('aria-pressed="true"');
  });

  it("shows per-file addition and deletion totals", () => {
    const html = renderToStaticMarkup(
      <DiffFileHeader
        file={{
          path: "src/app.ts",
          status: "M",
          patch: "@@ -1 +1,2 @@\n-old\n+new\n+next",
        }}
      />,
    );
    expect(html).toContain("+2");
    expect(html).toContain("−1");
  });

  it("explains rename-only changes instead of leaving split view blank", () => {
    const html = renderToStaticMarkup(
      <SplitDiff patch={"similarity index 100%\nrename from old.ts\nrename to new.ts"} />,
    );
    expect(html).toContain("파일 이름이 변경되었습니다");
    expect(html).toContain("old.ts → new.ts");
  });

  it("paints token colors as foreground on top of the changed-span background", () => {
    const theme = getTheme("catppuccin-mocha");
    const html = renderToStaticMarkup(
      <DiffLineText
        text="const b = 3;"
        kind="add"
        theme={theme}
        tokens={[
          { start: 0, end: 5, type: "keyword.ts" },
          { start: 5, end: 10, type: "" },
          { start: 10, end: 11, type: "number.ts" },
          { start: 11, end: 12, type: "delimiter.ts" },
        ]}
        segments={[
          { text: "const b = ", changed: false },
          { text: "3", changed: true },
          { text: ";", changed: false },
        ]}
      />,
    );
    expect(html).toContain(`<span style="color:${theme.syntax!.keyword}">const</span>`);
    expect(html).toContain(`<span class="bg-addbg-strong" style="color:${theme.syntax!.number}">3</span>`);
  });

  it("keeps the plain span when there are no tokens and no segments", () => {
    const html = renderToStaticMarkup(
      <DiffLineText text="x" kind="context" tokens={[]} theme={getTheme("praxis-dark")} />,
    );
    expect(html).toBe("<span>x</span>");
  });
});
