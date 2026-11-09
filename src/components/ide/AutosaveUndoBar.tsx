import { useEffect, useState } from "react";
import type { AutosaveUndoEntry } from "../../lib/editor-window-events";

export interface AutosaveNotice {
  taskId: number;
  entries: AutosaveUndoEntry[];
  /** 유예 마감 시각(ms). 지나면 되돌릴 지점이 사라진다. */
  deadline: number;
}

interface Props {
  notice: AutosaveNotice | null;
  onUndo: () => void;
  onDismiss: () => void;
}

/** 남은 시간은 초 단위로 올림 — 1초가 채 안 남았어도 "0초"라고 적어 이미 끝난 것처럼 보이지 않게. */
const secondsLeft = (deadline: number): number => Math.max(0, Math.ceil((deadline - Date.now()) / 1000));

/**
 * 에디터 창이 조용히 디스크에 쓴 것을 알리고 되돌릴 자리를 준다.
 *
 * 자동 저장은 확인을 묻지 않는다(시야 밖 창에서 뜨는 모달은 아무도 못 본다). 대신 쓴 뒤에
 * 여기서 고지한다 — 워크트리에 조용히 들어간 내용이 에이전트 diff에 섞이면 리뷰 단위가
 * 오염되므로, 되돌릴 지점만은 남겨 둔다.
 */
export function AutosaveUndoBar({ notice, onUndo, onDismiss }: Props) {
  const [, tick] = useState(0);

  useEffect(() => {
    if (!notice) return;
    const timer = window.setInterval(() => tick((n) => n + 1), 250);
    return () => window.clearInterval(timer);
  }, [notice]);

  useEffect(() => {
    if (!notice) return;
    const left = notice.deadline - Date.now();
    const timer = window.setTimeout(onDismiss, Math.max(0, left));
    return () => window.clearTimeout(timer);
  }, [notice, onDismiss]);

  if (!notice) return null;

  const undoable = notice.entries.filter((e) => e.content !== null);
  const tooBig = notice.entries.length - undoable.length;

  return (
    <div
      role="status"
      aria-live="polite"
      className="fixed bottom-6 left-1/2 z-50 flex max-w-[min(560px,calc(100vw-3rem))] -translate-x-1/2 items-center gap-3 rounded-lg border border-border-strong bg-raised px-3 py-2 text-xs shadow-e2"
    >
      <span className="truncate">
        코드 창에서 파일 {notice.entries.length}개를 자동 저장했습니다
        {tooBig > 0 && <span className="text-text-muted"> · {tooBig}개는 되돌릴 수 없음</span>}
      </span>
      {undoable.length > 0 && (
        <button className="shrink-0 underline hover:text-accent" onClick={onUndo}>
          되돌리기
        </button>
      )}
      <span className="shrink-0 tabular-nums text-text-muted">{secondsLeft(notice.deadline)}초</span>
    </div>
  );
}
