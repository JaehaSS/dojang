import { describe, expect, it } from "vitest";
import { captureImagePaths, formatCaptureBlock, formatCapturesPrompt } from "./prompt";
import type { DesignCaptureRecord } from "./types";

function record(overrides: Partial<DesignCaptureRecord> = {}): DesignCaptureRecord {
  return {
    id: "1-0",
    task_id: 1,
    source: "preview",
    outer_html: "<button>Click</button>",
    computed_css: { display: "flex", color: "rgb(0, 0, 0)" },
    bounding_rect: { x: 1, y: 2, width: 3, height: 4 },
    captured_at: 1000,
    image_path: null,
    file_path: null,
    selection_text: null,
    selection_start_line: null,
    selection_end_line: null,
    ...overrides,
  };
}

describe("formatCaptureBlock", () => {
  it("HTML/CSS 블록과 인덱스 라벨을 포함한다", () => {
    const block = formatCaptureBlock(record(), 2);
    expect(block).toContain("[Design Mode 캡처 2]");
    expect(block).toContain("<button>Click</button>");
    expect(block).toContain("display: flex;");
    expect(block).toContain("```html");
    expect(block).toContain("```css");
  });

  it("computed_css가 비어 있으면 안내 주석을 남긴다", () => {
    const block = formatCaptureBlock(record({ computed_css: {} }), 1);
    expect(block).toContain("스타일 없음");
  });

  it("image_path가 있으면(있으면) 경로를 덧붙인다", () => {
    const block = formatCaptureBlock(record({ image_path: ".praxis/captures/1/1.png" }), 1);
    expect(block).toContain("이미지: .praxis/captures/1/1.png");
  });

  it("image_path가 없으면 이미지 줄을 넣지 않는다", () => {
    const block = formatCaptureBlock(record(), 1);
    expect(block).not.toContain("이미지:");
  });

  it("에디터 캡처는 파일·선택 줄·선택 코드·이미지 확인 지시를 포함한다", () => {
    const block = formatCaptureBlock(record({
      source: "editor",
      file_path: "src/App.tsx",
      selection_text: "const answer = 42;",
      selection_start_line: 10,
      selection_end_line: 12,
      image_path: "/tmp/editor.png",
    }), 1);
    expect(block).toContain("[에디터 캡처 1]");
    expect(block).toContain("파일: src/App.tsx");
    expect(block).toContain("선택 줄: L10–L12");
    expect(block).toContain("const answer = 42;");
    expect(block).toContain("이미지를 열어");
  });

  it("선택 코드 펜스에 파일 확장자 기반 언어를 붙인다", () => {
    const block = formatCaptureBlock(
      record({
        source: "editor",
        file_path: "src/main.rs",
        selection_text: "let answer = 42;",
        selection_start_line: 3,
        selection_end_line: 3,
      }),
      1,
    );
    expect(block).toContain("```rust");
  });

  it("스크린샷 없는 선택 첨부(⌘L)는 이미지 줄 없이 코드만 전달한다", () => {
    const block = formatCaptureBlock(
      record({
        source: "editor",
        file_path: "src/App.tsx",
        selection_text: "const answer = 42;",
        selection_start_line: 10,
        selection_end_line: 12,
        image_path: null,
      }),
      1,
    );
    expect(block).toContain("```typescript");
    expect(block).toContain("const answer = 42;");
    expect(block).not.toContain("이미지:");
  });

  it("붙여넣기 캡처는 HTML/CSS 없이 이미지 경로만 전달한다", () => {
    const block = formatCaptureBlock(
      record({ source: "paste", image_path: "/wt/.praxis/captures/1/1-0.png" }),
      3,
    );
    expect(block).toContain("[붙여넣은 이미지 3]");
    expect(block).toContain("이미지: /wt/.praxis/captures/1/1-0.png");
    expect(block).not.toContain("```html");
    expect(block).not.toContain("```css");
  });
});

describe("formatCapturesPrompt", () => {
  it("빈 배열은 빈 문자열을 반환한다", () => {
    expect(formatCapturesPrompt([])).toBe("");
  });

  it("여러 캡처를 순서대로 이어붙인다", () => {
    const out = formatCapturesPrompt([record({ id: "a" }), record({ id: "b" })]);
    expect(out).toContain("[Design Mode 캡처 1]");
    expect(out).toContain("[Design Mode 캡처 2]");
    expect(out.indexOf("캡처 1")).toBeLessThan(out.indexOf("캡처 2"));
  });
});

describe("captureImagePaths", () => {
  it("실제 이미지가 있는 캡처 경로만 순서대로 반환한다", () => {
    expect(captureImagePaths([
      record({ image_path: "/tmp/a.png" }),
      record({ id: "b", image_path: null }),
      record({ id: "c", image_path: "/tmp/c.png" }),
    ])).toEqual(["/tmp/a.png", "/tmp/c.png"]);
  });
});
