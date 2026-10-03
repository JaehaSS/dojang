import { describe, expect, it } from "vitest";
import { editorChipLabel } from "./CaptureAttachmentChips";
import type { DesignCaptureRecord } from "../../lib/designmode/types";

const record = (patch: Partial<DesignCaptureRecord>): DesignCaptureRecord => ({
  id: "local-7-0",
  task_id: 7,
  source: "editor",
  outer_html: "",
  computed_css: {},
  bounding_rect: { x: 0, y: 0, width: 0, height: 0 },
  captured_at: 1,
  image_path: null,
  file_path: "src/components/ide/App.tsx",
  selection_text: "const answer = 42;",
  selection_start_line: 12,
  selection_end_line: 30,
  ...patch,
});

describe("editorChipLabel", () => {
  it("여러 줄 선택은 범위로 보인다", () => {
    expect(editorChipLabel(record({}))).toBe("App.tsx L12–30");
  });

  it("한 줄 선택은 줄 하나만 보인다", () => {
    expect(editorChipLabel(record({ selection_start_line: 12, selection_end_line: 12 }))).toBe(
      "App.tsx L12",
    );
  });

  it("선택 정보가 없는 화면 캡처는 기존 라벨을 유지한다", () => {
    expect(
      editorChipLabel(record({ selection_start_line: null, selection_end_line: null })),
    ).toBe("에디터 · App.tsx");
  });

  it("경로가 없어도 라벨을 만든다", () => {
    expect(editorChipLabel(record({ file_path: null }))).toBe("파일 L12–30");
  });
});
