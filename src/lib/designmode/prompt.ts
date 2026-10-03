import { fenceLangFromPath } from "../lang-from-path";
import type { DesignCaptureRecord } from "./types";

const CSS_INDENT = "  ";

function cssBlock(css: Record<string, string>): string {
  const lines = Object.entries(css).map(([key, value]) => `${CSS_INDENT}${key}: ${value};`);
  return lines.length > 0 ? lines.join("\n") : `${CSS_INDENT}/* (스타일 없음) */`;
}

function editorLineLabel(record: DesignCaptureRecord): string | null {
  if (record.selection_start_line == null || record.selection_end_line == null) return null;
  return record.selection_start_line === record.selection_end_line
    ? `선택 줄: L${record.selection_start_line}`
    : `선택 줄: L${record.selection_start_line}–L${record.selection_end_line}`;
}

function formatEditorCaptureBlock(record: DesignCaptureRecord, index: number): string {
  const parts = [
    `[에디터 캡처 ${index}]`,
    `파일: ${record.file_path ?? "(경로 없음)"}`,
  ];
  const lineLabel = editorLineLabel(record);
  if (lineLabel) parts.push(lineLabel);
  if (record.selection_text) {
    // 코드펜스 언어를 파일 확장자에서 뽑는다 — 에이전트가 문법을 언어로 읽게 하려는 것.
    const lang = record.file_path ? fenceLangFromPath(record.file_path) : "text";
    parts.push("선택 코드:", `\`\`\`${lang}`, record.selection_text, "```");
  }
  if (record.image_path) {
    parts.push(`이미지: ${record.image_path}`, "이미지를 열어 화면 상태와 코드를 함께 확인하세요.");
  }
  return parts.join("\n");
}

/** 클립보드 붙여넣기 캡처 — HTML/CSS 없이 이미지 경로만 전달한다. */
function formatPastedCaptureBlock(record: DesignCaptureRecord, index: number): string {
  return [
    `[붙여넣은 이미지 ${index}]`,
    `이미지: ${record.image_path ?? "(경로 없음)"}`,
    "이미지를 열어 내용을 확인하세요.",
  ].join("\n");
}

/** 캡처 1건 → HTML/CSS 블록(+있으면 이미지 경로). Composer 전송 시 프롬프트에 그대로 삽입한다. */
export function formatCaptureBlock(record: DesignCaptureRecord, index: number): string {
  if (record.source === "editor") return formatEditorCaptureBlock(record, index);
  if (record.source === "paste") return formatPastedCaptureBlock(record, index);
  const parts = [
    `[Design Mode 캡처 ${index}]`,
    "```html",
    record.outer_html,
    "```",
    "```css",
    cssBlock(record.computed_css),
    "```",
  ];
  if (record.image_path) parts.push(`이미지: ${record.image_path}`);
  return parts.join("\n");
}

/** 여러 캡처를 순서대로 이어붙인다 — 빈 배열이면 빈 문자열(호출부가 조건 분기 없이 사용 가능). */
export function formatCapturesPrompt(records: DesignCaptureRecord[]): string {
  if (records.length === 0) return "";
  return records.map((record, i) => formatCaptureBlock(record, i + 1)).join("\n\n");
}

/** 실제 멀티모달 입력을 지원하는 벤더에 전달할 이미지 절대경로. */
export function captureImagePaths(records: DesignCaptureRecord[]): string[] {
  return records.flatMap((record) => (record.image_path ? [record.image_path] : []));
}
