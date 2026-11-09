import { useEffect, useState } from "react";
import { removalDetail, removalHeadline } from "../../lib/task-removal";
import type { PendingRemoval } from "./useDeferredTaskRemoval";

export interface RemovalUndoBarProps {
  pending: PendingRemoval[];
  onUndo: () => void;
}

/** 남은 시간은 초 단위로 올림 — 1초가 채 안 남았어도 "0초"라고 적어 이미 끝난 것처럼 보이지 않게. */
const secondsLeft = (deadline: number): number => Math.max(0, Math.ceil((deadline - Date.now()) / 1000));

/**
 * 유예 중인 삭제를 보여 주고 되돌릴 자리를 주는 배너.
 *
 * 카운트다운은 여기서만 흐른다 — 남은 시간을 상위 상태로 올리면 App 전체가 매 틱 다시 그려진다.
 */
export function RemovalUndoBar({ pending, onUndo }: RemovalUndoBarProps) {
  const [, tick] = useState(0);
  const earliest = pending.reduce<number | null>(
    (soonest, entry) => (soonest === null || entry.deadline < soonest ? entry.deadline : soonest),
    null,
  );

  useEffect(() => {
    if (earliest === null) return;
    const timer = window.setInterval(() => tick((n) => n + 1), 250);
    return () => window.clearInterval(timer);
  }, [earliest]);

  if (earliest === null) return null;

  const last = pending[pending.length - 1];
  const single = pending.length === 1 && last;

  return (
    <div
      role="status"
      aria-live="polite"
      className="fixed bottom-6 left-1/2 z-50 flex max-w-[min(560px,calc(100vw-3rem))] -translate-x-1/2 items-center gap-3 rounded-lg border border-border-strong bg-raised px-3 py-2 text-xs shadow-e2"
    >
      <span className="min-w-0 flex-1">
        <span className="block truncate text-text">
          {single ? removalHeadline(last.task) : `세션 ${pending.length}개 정리`}
        </span>
        <span className="block truncate text-text-muted">
          {single ? removalDetail(last.task) : "되돌리면 가장 최근 것부터 하나씩 살아납니다."}
        </span>
      </span>
      <span className="shrink-0 tabular-nums text-text-secondary">{secondsLeft(earliest)}초</span>
      <button
        className="shrink-0 rounded-md border border-border px-2 py-1 text-text-secondary hover:border-border-strong hover:text-text"
        onClick={onUndo}
      >
        실행 취소 <span className="text-text-muted">⌘Z</span>
      </button>
    </div>
  );
}
