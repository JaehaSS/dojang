import type { DesignCaptureRecord } from "./types";

// 에디터에서 드래그한 코드를 세션 컴포저에 첨부하기 위한 프론트 전용 캡처 레코드.
// 백엔드 `save_editor_capture`를 쓰지 않는 이유: 그 경로는 스크린샷 실패 시 전체가 Err이고
// (designmode/editor.rs), ⌘L마다 `screencapture` shellout이 도는 것은 비용·권한 양쪽에서 부적합하다.
// 선택 코드는 프롬프트에 텍스트로 인라인되므로 디스크에 남길 산출물 자체가 없다.

/** `designmode/editor.rs`의 `MAX_SELECTION_TEXT`와 같은 값 — 두 경로의 프롬프트 상한을 맞춘다. */
export const MAX_SELECTION_TEXT = 20_000;

/** 백엔드에 대응 파일이 없는 캡처 — 제거 시 IPC 왕복을 건너뛰는 판별에 쓴다. */
export const LOCAL_CAPTURE_PREFIX = "local-";

export function isLocalCapture(id: string): boolean {
  return id.startsWith(LOCAL_CAPTURE_PREFIX);
}

/**
 * 상한을 넘으면 잘라내고 원본 길이를 덧붙인다. 빈 선택은 null(첨부할 것이 없음).
 * 길이 기준은 백엔드의 `chars().count()`(유니코드 스칼라)와 맞추기 위해 코드포인트 단위로 센다 —
 * `String.length`(UTF-16 코드유닛)를 쓰면 이모지·CJK 조합에서 두 경로의 상한이 어긋난다.
 */
export function truncateSelection(text: string): string | null {
  if (!text) return null;
  const chars = [...text];
  if (chars.length <= MAX_SELECTION_TEXT) return text;
  return `${chars.slice(0, MAX_SELECTION_TEXT).join("")}\n…truncated (원본 ${chars.length}자)`;
}

export interface SelectionCaptureInput {
  taskId: number;
  filePath: string;
  text: string;
  startLine: number;
  endLine: number;
}

let seq = 0;

/** 선택이 비어 있으면 null — 호출부가 "첨부할 선택 없음"을 분기한다. */
export function buildSelectionCapture(input: SelectionCaptureInput): DesignCaptureRecord | null {
  const selection = truncateSelection(input.text);
  if (selection === null) return null;
  return {
    id: `${LOCAL_CAPTURE_PREFIX}${input.taskId}-${seq++}`,
    task_id: input.taskId,
    source: "editor",
    outer_html: "",
    computed_css: {},
    // 스크린샷을 찍지 않으므로 캡처 영역이 없다. 칩·프롬프트 어느 쪽도 이 값을 읽지 않는다.
    bounding_rect: { x: 0, y: 0, width: 0, height: 0 },
    captured_at: Date.now(),
    image_path: null,
    file_path: input.filePath,
    selection_text: selection,
    selection_start_line: input.startLine,
    selection_end_line: input.endLine,
  };
}

/**
 * 팝아웃 창이 만든 로컬 캡처 id에 창 스코프(`w`)를 섞는다.
 *
 * `seq`는 모듈 인스턴스의 것이고 창이 갈리면 모듈도 갈린다 — 메인·팝아웃 양쪽이 0부터 세므로
 * 스코프가 없으면 두 창의 첫 캡처가 `local-42-0`으로 겹친다. 겹치면 칩 하나를 지울 때
 * `removeCapture`가 id로 걸러 남의 칩까지 지운다.
 *
 * 이미 스코프가 붙었으면 그대로 둔다(멱등). 로컬 캡처가 아니면 백엔드가 발급한 id이므로 손대지 않는다.
 */
export function scopeLocalCaptureId(id: string): string {
  const scoped = `${LOCAL_CAPTURE_PREFIX}w`;
  if (id.startsWith(scoped)) return id;
  if (!isLocalCapture(id)) return id;
  return `${scoped}${id.slice(LOCAL_CAPTURE_PREFIX.length)}`;
}
