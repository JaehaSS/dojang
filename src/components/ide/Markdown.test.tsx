import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { Markdown } from "./Markdown";
import { MarkdownDoc } from "./MarkdownDoc";

describe("Markdown agent links", () => {
  it("preserves file URIs when the host provides a safe link handler", () => {
    const html = renderToStaticMarkup(
      <Markdown
        text="[Excel 열기](file:///Users/test/worktree/report.xlsx)"
        onOpenLink={() => {}}
      />,
    );

    expect(html).toContain('href="file:///Users/test/worktree/report.xlsx"');
    expect(html).toContain("Excel 열기");
  });

  it("keeps the default sanitizer when no host link handler exists", () => {
    const html = renderToStaticMarkup(
      <Markdown text="[unsafe](javascript:alert(1))" />,
    );

    expect(html).toContain('href=""');
  });
});

describe("Markdown plain file paths", () => {
  it("links a bare path in an agent answer", () => {
    const html = renderToStaticMarkup(
      <Markdown
        text="플랜 저장 완료: docs/plans/0014.2026-08-04-vlm-ocr-date-partition.md"
        onOpenLink={() => {}}
      />,
    );

    expect(html).toContain('href="docs/plans/0014.2026-08-04-vlm-ocr-date-partition.md"');
    expect(html).toContain(">docs/plans/0014.2026-08-04-vlm-ocr-date-partition.md</a>");
    expect(html).toContain("플랜 저장 완료: ");
  });

  it("keeps the line suffix in the href", () => {
    const html = renderToStaticMarkup(
      <Markdown text="resolveAgentLink는 src/App.tsx:1187 이다" onOpenLink={() => {}} />,
    );

    expect(html).toContain('href="src/App.tsx:1187"');
  });

  it("links inline code that is nothing but a path", () => {
    const html = renderToStaticMarkup(
      <Markdown text="`docs/memory.md` 맨 위에 추가한다" onOpenLink={() => {}} />,
    );

    expect(html).toMatch(/<a[^>]*href="docs\/memory\.md"[^>]*><code/);
  });

  it("leaves a command that merely contains a path alone", () => {
    const html = renderToStaticMarkup(
      <Markdown text="`cat docs/memory.md` 로 본다" onOpenLink={() => {}} />,
    );

    expect(html).not.toContain("<a");
  });

  it("leaves fenced code and existing links alone", () => {
    const html = renderToStaticMarkup(
      <Markdown
        text={"```\ncat docs/memory.md\n```\n\n[원장](docs/memory.md)"}
        onOpenLink={() => {}}
      />,
    );

    expect(html.match(/<a /g)).toHaveLength(1);
    expect(html).toContain(">원장</a>");
  });

  it("links bare file names in a list under a directory lead-in", () => {
    const html = renderToStaticMarkup(
      <Markdown
        text={[
          "**만든 문서** (`~/Documents/째하 지식창고/개인/웹어셈블리/`)",
          "- `README.md`: 목차",
          "- `웹어셈블리-구조.md:12`: 구조",
        ].join("\n")}
        onOpenLink={() => {}}
      />,
    );

    const hrefs = [...html.matchAll(/href="([^"]*)"/g)].map((m) => decodeURIComponent(m[1]));
    expect(hrefs).toEqual([
      "~/Documents/째하 지식창고/개인/웹어셈블리/README.md",
      "~/Documents/째하 지식창고/개인/웹어셈블리/웹어셈블리-구조.md:12",
    ]);
  });

  it("does not carry a directory past the list that follows it", () => {
    const html = renderToStaticMarkup(
      <Markdown
        text={"`docs/plans/` 아래:\n\n- `a.md`\n\n다른 문단의 `package.json`"}
        onOpenLink={() => {}}
      />,
    );

    expect(html).toContain('href="docs/plans/a.md"');
    expect(html.match(/<a /g)).toHaveLength(1);
  });

  it("leaves bare file names alone without a directory", () => {
    const html = renderToStaticMarkup(
      <Markdown text={"- `README.md`: 목차"} onOpenLink={() => {}} />,
    );

    expect(html).not.toContain("<a");
  });

  it("makes no link when the host cannot open one", () => {
    const html = renderToStaticMarkup(<Markdown text="docs/plans/0014.md 를 보라" />);

    expect(html).not.toContain("<a");
  });
});

describe("Markdown fenced code layout", () => {
  it("keeps a horizontal scrollbar inside the code frame and clear of the following text", () => {
    const html = renderToStaticMarkup(
      <Markdown text={"```\nvery-long-code-line\n```\n\n아래 문단"} />,
    );

    expect(html).toContain("overflow-hidden");
    expect(html).toContain("overflow-x-auto pb-3");
    expect(html).not.toMatch(/<pre[^>]*overflow-auto/);
    expect(html.indexOf("</pre>")).toBeLessThan(html.indexOf("아래 문단"));
  });

  it("uses the same protected code frame in the document preview", () => {
    const html = renderToStaticMarkup(
      <MarkdownDoc text={"```\nvery-long-code-line\n```\n\n아래 문단"} dark />,
    );

    expect(html).toContain("overflow-hidden");
    expect(html).toContain("overflow-x-auto pb-3");
    expect(html).not.toMatch(/<pre[^>]*overflow-auto/);
    expect(html.indexOf("</pre>")).toBeLessThan(html.indexOf("아래 문단"));
  });

  it("can leave document-preview images unloaded", () => {
    const html = renderToStaticMarkup(
      <MarkdownDoc text="![원격 그림](https://example.test/image.png)" dark loadImages={false} />,
    );

    expect(html).not.toContain("<img");
    expect(html).not.toContain("https://example.test/image.png");
    expect(html).toContain("원격 그림");
  });

  it("does not hand raw HTML to the sandbox when images are blocked", () => {
    // sandbox=""는 스크립트를 막을 뿐 하위 리소스 로드는 막지 않는다 — srcdoc에 넣으면 이미지가
    // 실제로 나간다. 외부 이미지가 금지된 문서에서는 마크다운 경로로 떨어뜨린다.
    const text = '<div><img src="https://tracker.example/beacon.png"></div>';

    expect(renderToStaticMarkup(<MarkdownDoc text={text} dark loadImages={false} />)).not.toContain(
      "<iframe",
    );
    expect(renderToStaticMarkup(<MarkdownDoc text={text} dark />)).toContain("<iframe");
  });

  it("keeps Mermaid source readable without executing diagrams when images are blocked", () => {
    const text = '```mermaid\nflowchart LR\nA@{ img: "https://example.test/diagram.png" }\n```';
    const html = renderToStaticMarkup(<MarkdownDoc text={text} dark loadImages={false} />);

    expect(html).toContain("flowchart LR");
    expect(html).toContain("<code");
    expect(html).not.toContain("<img");
    expect(html).not.toContain("<svg");
    // 기본 파일 미리보기는 기존 Mermaid 컴포넌트 경로를 유지한다(SSR에서는 빈 출력).
    expect(renderToStaticMarkup(<MarkdownDoc text={text} dark />)).not.toContain("flowchart LR");
  });
});
