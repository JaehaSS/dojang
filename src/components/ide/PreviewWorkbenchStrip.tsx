import { useRef } from "react";
import { Icon } from "./icons";
import type { PreviewWorkbenchState } from "../../lib/preview-workbench/types";
import { IconButton } from "../ui";

interface Props {
  state: PreviewWorkbenchState;
  compact?: boolean;
  onDraftChange: (draft: string) => void;
  onSubmit: (message: string) => void;
  onCancelPending: () => void;
  onTakeOver: () => void;
  onRelease: () => void;
  onRefresh: () => void;
}

function unavailableReason(state: PreviewWorkbenchState): string | null {
  if (!state.supported) return state.unsupportedReason ?? "이 작업 모드에서는 프리뷰 질문을 지원하지 않습니다.";
  if (!state.url) return "열린 프리뷰가 없습니다.";
  if (!/^http:\/\/(localhost|127\.0\.0\.1)(?::|\/|$)/.test(state.url)) return "이 페이지는 제어 가능한 로컬 프리뷰가 아닙니다.";
  return null;
}

export function PreviewWorkbenchStrip({
  state,
  compact = false,
  onDraftChange,
  onSubmit,
  onCancelPending,
  onTakeOver,
  onRelease,
  onRefresh,
}: Props) {
  const composing = useRef(false);
  const reason = unavailableReason(state);
  const canSubmit = !reason && !!state.draft.trim();
  const submit = () => {
    if (canSubmit) onSubmit(state.draft);
  };

  return (
    <section className="shrink-0 border-b border-border bg-surface" aria-label="프리뷰 워크벤치">
      <div className={`flex items-center gap-2 px-3 py-2 text-xs ${compact ? "flex-nowrap" : "flex-wrap"}`}>
        <span className={`min-w-0 truncate font-mono text-text-secondary ${compact ? "flex-1" : ""}`} title={state.displayUrl ?? state.url ?? undefined}>
          {state.displayUrl ?? state.url ?? "프리뷰 없음"}
        </span>
        <span className="rounded border border-border px-1.5 py-0.5 text-text-secondary" role="status" aria-live="polite">
          {state.takenOver ? "사용자 제어" : reason ? "제어 불가" : "제어 가능"}
        </span>
        {state.lastAction && <span className="min-w-0 truncate text-text-muted">마지막 동작: {state.lastAction}</span>}
        <IconButton size="sm" className="ml-auto" label="프리뷰 상태 다시 확인" onClick={onRefresh}>
          <Icon name="refresh" size={14} />
        </IconButton>
      </div>
      <div className={`flex gap-2 px-3 pb-2 ${compact ? "flex-row" : "flex-col sm:flex-row"}`}>
        <textarea
          className={`min-h-9 min-w-0 flex-1 rounded border border-border bg-surface px-2 py-1 text-sm text-text outline-none focus:border-primary ${compact ? "h-9 resize-none" : "resize-y"}`}
          value={state.draft}
          onChange={(event) => onDraftChange(event.target.value)}
          onCompositionStart={() => { composing.current = true; }}
          onCompositionEnd={() => { composing.current = false; }}
          onKeyDown={(event) => {
            if (event.key !== "Enter" || event.shiftKey || composing.current) return;
            event.preventDefault();
            submit();
          }}
          aria-label="프리뷰에 요청"
          aria-describedby={reason || state.error ? "preview-workbench-message" : undefined}
          placeholder="프리뷰에 요청…"
        />
        <div className="flex shrink-0 gap-2">
          <button className="h-9 rounded bg-primary px-3 text-sm font-medium text-bg disabled:bg-neutral-800 disabled:text-text-muted" type="button" onClick={submit} disabled={!canSubmit}>
            {state.pending ? "대기 질문 교체" : state.busy === "idle" ? "보내기" : "대기 등록"}
          </button>
          <button className="h-9 rounded border border-primary px-3 text-sm text-primary-bright disabled:border-border disabled:text-text-muted" type="button" onClick={state.takenOver ? onRelease : onTakeOver} disabled={!!reason}>
            {state.takenOver ? "돌려주기" : "직접 제어"}
          </button>
        </div>
      </div>
      {(state.pending || reason || state.error) && (
        <div id="preview-workbench-message" className="flex max-h-16 flex-wrap items-center gap-2 overflow-y-auto border-t border-border px-3 py-1.5 text-xs" aria-live="polite">
          {state.pending && <span className="min-w-0 truncate text-text-secondary">대기: {state.pending.message}</span>}
          {state.pending && !state.inFlight && <button className="text-text-muted hover:text-text" type="button" onClick={onCancelPending}>대기 취소</button>}
          {(reason || state.error) && <span className="text-status-failed">{state.error ?? reason}</span>}
          {state.error && <button className="text-primary-bright hover:text-text" type="button" onClick={onRefresh}>재시도</button>}
        </div>
      )}
      {state.busy === "unknown" && !reason && !state.error && <div className="border-t border-border px-3 py-1.5 text-xs text-text-muted" aria-live="polite">실행 상태 확인 중</div>}
    </section>
  );
}
