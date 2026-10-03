/** Rust `designmode::BoundingRect`와 필드가 일치한다(스크린 좌표, 논리 픽셀). */
export interface DesignBoundingRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Rust `designmode::CaptureRecord`와 필드가 일치한다 — 기존 IPC 타입(Task 등)과 같은 snake_case 컨벤션. */
export interface DesignCaptureRecord {
  id: string;
  task_id: number;
  /** Rust `CaptureSource` enum과 같다. */
  source: "preview" | "editor" | "paste";
  outer_html: string;
  computed_css: Record<string, string>;
  bounding_rect: DesignBoundingRect;
  captured_at: number;
  /** 스크린샷 절대경로 — macOS에서 `screencapture` shellout 성공 시 채워지고, TCC 미허용·비-macOS 등 실패 시 null. */
  image_path: string | null;
  /** 에디터 캡처일 때 열린 worktree 상대 경로. */
  file_path: string | null;
  /** 에디터에서 명시적으로 선택한 코드(상한 적용). */
  selection_text: string | null;
  selection_start_line: number | null;
  selection_end_line: number | null;
}
