import { summarizePartialSelection } from "../../lib/partial";
import { viewedCount } from "../../lib/diff-viewed";
import type { DiffSessionValue } from "../DiffSessionContext";
import { DiffRangeToggle } from "../DiffPresentation";
import { BaselineNotice } from "../DiffViewerLayout";
import { Icon } from "./icons";
import { IconButton } from "../ui";

/** 두 줄은 늘 있고, 재전송·부분 적용은 필요할 때만 자리를 쓴다(설계 DR-10). */
export function ChangesHeader({ session }: { session: DiffSessionValue }) {
  const { data, viewed, actions } = session;
  const files = data.files ?? [];
  const notice = actions.resendError ?? data.error;
  return (
    <div className="shrink-0 border-b border-border">
      <div className="flex h-8 items-center gap-1 px-3 text-xs text-text-muted">
        <span className="truncate">
          변경 {files.length} · 확인 {viewedCount(viewed, files)}/{files.length}
        </span>
        <IconButton label="Diff 새로고침" className="ml-auto" onClick={data.refresh}>
          <Icon name="refresh" size={14} />
        </IconButton>
      </div>
      <div className="flex items-center px-3 pb-1.5">
        <DiffRangeToggle range={data.range} onChange={data.setRange} />
      </div>
      <BaselineNotice baseline={data.baseline} />
      {actions.draftIds.length > 0 && (
        <div className="px-3 pb-1.5">
          <button
            className="h-6 w-full rounded border border-border bg-raised px-2 text-xs text-text disabled:opacity-50"
            disabled={actions.resending}
            onClick={() => void actions.resend()}
          >
            {actions.resending ? "재전송 중…" : `주석 ${actions.draftIds.length}건 재전송`}
          </button>
        </div>
      )}
      <PartialApplyLine session={session} />
      {notice && (
        <p className="border-t border-border px-3 py-1.5 text-xs text-status-failed">{notice}</p>
      )}
    </div>
  );
}

/**
 * 부분 적용 — 선택이 기본값과 다를 때만 나타난다.
 *
 * 확인 단계가 hunk 수가 아니라 **파일 이름**을 나열하는 이유는, 되돌릴 대상이 지금 보고 있는
 * 탭 밖에도 있기 때문이다. 숫자만 보이면 무엇을 버리는지 모른 채 누르게 된다(설계 F-8).
 */
function PartialApplyLine({ session }: { session: DiffSessionValue }) {
  const { data, partial } = session;
  const { discardedIds } = summarizePartialSelection(data.hunks, partial.selection);
  const discarded = new Set(discardedIds);
  const paths = [...new Set(data.hunks.filter((h) => discarded.has(h.id)).map((h) => h.path))];
  if (discardedIds.length === 0 && !partial.result && !partial.error) return null;

  if (partial.confirming) {
    return (
      <div className="border-t border-border bg-raised px-3 py-1.5 text-xs">
        <p className="text-text-secondary">
          hunk {discardedIds.length}개를 버립니다 — 아래 파일이 바뀝니다.
        </p>
        <ul className="mt-1 font-code text-[11px] text-text-muted">
          {paths.map((path) => (
            <li key={path} className="truncate" title={path}>
              {path}
            </li>
          ))}
        </ul>
        <div className="mt-1.5 flex gap-1">
          <button
            className="h-6 flex-1 rounded bg-primary text-bg disabled:opacity-50"
            disabled={partial.busy}
            onClick={() => void partial.apply()}
          >
            {partial.busy ? "적용 중…" : "적용"}
          </button>
          <button
            className="h-6 px-2 rounded text-text-secondary hover:bg-border"
            disabled={partial.busy}
            onClick={partial.cancelConfirm}
          >
            취소
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="border-t border-border px-3 py-1.5 text-xs">
      {discardedIds.length > 0 && (
        <>
          <p className="text-text-secondary">
            hunk {discardedIds.length}개 버림 · 파일 {paths.length}개
          </p>
          <button
            className="mt-1 h-6 w-full rounded border border-border bg-raised text-text hover:border-primary-bright disabled:opacity-50"
            disabled={partial.busy}
            onClick={partial.requestConfirm}
          >
            선택 적용 ({partial.keptCount} hunks)
          </button>
        </>
      )}
      {partial.error && <p className="text-status-failed">{partial.error}</p>}
      {partial.result && (
        <button
          className="mt-1 h-6 w-full rounded border border-dangerborder bg-status-failed/15 text-status-failed disabled:opacity-50"
          disabled={partial.busy}
          onClick={() => void partial.rollback()}
        >
          되돌림 (유지 {partial.result.kept_hunk_ids.length})
        </button>
      )}
    </div>
  );
}
