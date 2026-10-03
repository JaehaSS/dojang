import type { DesignCaptureRecord } from "../../lib/ipc";
import { Icon } from "./icons";

/** 에디터 캡처 칩 라벨 — 선택 범위가 있으면 줄 번호를 보여 무엇을 붙였는지 눈으로 확인하게 한다. */
export function editorChipLabel(capture: DesignCaptureRecord): string {
  const name = capture.file_path?.split("/").pop() ?? "파일";
  const start = capture.selection_start_line;
  const end = capture.selection_end_line;
  if (start == null || end == null) return `에디터 · ${name}`;
  return start === end ? `${name} L${start}` : `${name} L${start}–${end}`;
}

function chipLabel(capture: DesignCaptureRecord): string {
  if (capture.source === "editor") return editorChipLabel(capture);
  if (capture.source === "paste") return `이미지 · ${capture.image_path?.split("/").pop() ?? "클립보드"}`;
  const { width, height } = capture.bounding_rect;
  return `${Math.round(width)}×${Math.round(height)}${capture.image_path ? "" : " (HTML/CSS)"}`;
}

/** 칩에 다 안 들어가는 것을 툴팁이 받는다 — 어느 파일인지가 제목보다 먼저 궁금해진다. */
function chipTitle(capture: DesignCaptureRecord): string {
  if (capture.source === "editor") return capture.file_path ?? "에디터 캡처";
  if (capture.source === "paste") return capture.image_path ?? "붙여넣은 이미지";
  return capture.outer_html.slice(0, 120);
}

interface Props {
  captures: DesignCaptureRecord[];
  onRemove: (id: string) => void;
}

export function CaptureAttachmentChips({ captures, onRemove }: Props) {
  if (captures.length === 0) return null;
  return (
    <div className="flex gap-1.5 flex-wrap mb-1.5" role="list" aria-label="세션 캡처 첨부">
      {captures.map((capture) => (
        <span
          key={capture.id}
          role="listitem"
          className="flex items-center gap-1 text-xs text-text-secondary border border-border rounded-md px-2 py-0.5"
          title={chipTitle(capture)}
        >
          <Icon
            name={capture.source === "preview" ? "code" : "desktop"}
            size={11}
          />
          {chipLabel(capture)}
          <button
            className="text-text-muted hover:text-status-failed"
            onClick={() => onRemove(capture.id)}
            aria-label="캡처 제거"
          >
            <Icon name="x" size={11} />
          </button>
        </span>
      ))}
    </div>
  );
}
