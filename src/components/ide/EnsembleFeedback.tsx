import type { ReactElement } from "react";
import type { EnsembleFeedbackEntry, EnsembleFeedbackHistory } from "../../lib/ipc";

interface Props {
  history: EnsembleFeedbackHistory | null;
  currentEnsemble: string;
  unavailable?: boolean;
}

export function EnsembleFeedback({
  history,
  currentEnsemble,
  unavailable = false,
}: Props): ReactElement | null {
  if (unavailable) return <FeedbackUnavailable />;
  if (!history) return null;
  return (
    <section className="m-3 mb-0 rounded-lg border border-border bg-bg p-3">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <h2 className="text-sm font-medium text-text">선택 피드백</h2>
        <span className="text-[11px] text-text-muted">최근 비교 실행의 선정 결과입니다.</span>
      </div>
      <div className="mt-2 flex flex-wrap gap-2 text-xs text-text-secondary">
        <Summary label="선정 완료" value={history.selected_count} />
        {history.pending_count > 0 && <Summary label="선택 대기" value={history.pending_count} />}
        {history.ambiguous_count > 0 && (
          <Summary label="복수 승인" value={history.ambiguous_count} tone="warning" />
        )}
      </div>
      {history.entries.length === 0 ? (
        <div className="mt-2 text-xs text-text-muted">비교 실행 이력이 아직 없습니다.</div>
      ) : (
        <div className="mt-2 grid gap-1.5">
          {history.entries.slice(0, 6).map((entry) => (
            <FeedbackRow
              key={entry.ensemble}
              entry={entry}
              current={entry.ensemble === currentEnsemble}
            />
          ))}
        </div>
      )}
    </section>
  );
}

function Summary({
  label,
  value,
  tone = "normal",
}: {
  label: string;
  value: number;
  tone?: "normal" | "warning";
}): ReactElement {
  const toneClass = tone === "warning" ? "text-status-failed" : "text-text-secondary";
  return (
    <span className={`rounded border border-border px-2 py-1 ${toneClass}`}>
      {label} {value}
    </span>
  );
}

function FeedbackRow({
  entry,
  current,
}: {
  entry: EnsembleFeedbackEntry;
  current: boolean;
}): ReactElement {
  return (
    <div className="flex min-w-0 items-center gap-2 rounded border border-border px-2.5 py-1.5 text-xs">
      {current && (
        <span className="shrink-0 rounded bg-primary/15 px-1.5 text-[10px] text-primary-bright">
          현재
        </span>
      )}
      <span className="shrink-0 text-text-muted">{entry.candidate_count} 후보</span>
      {entry.selection_status === "selected" ? (
        <SelectedFeedback entry={entry} />
      ) : (
        <UncertainFeedback status={entry.selection_status} />
      )}
    </div>
  );
}

function SelectedFeedback({ entry }: { entry: EnsembleFeedbackEntry }): ReactElement {
  return (
    <>
      <span className="shrink-0 font-code font-medium text-text-secondary">
        {entry.selected_agent ?? "알 수 없는 에이전트"}
      </span>
      <span className="min-w-0 truncate text-text-muted">{modelLabel(entry)}</span>
    </>
  );
}

function UncertainFeedback({
  status,
}: {
  status: EnsembleFeedbackEntry["selection_status"];
}): ReactElement {
  const ambiguous = status === "ambiguous";
  return (
    <span className={ambiguous ? "text-status-failed" : "text-text-muted"}>
      {ambiguous ? "복수 승인 · 승자 미추정" : "선택 대기"}
    </span>
  );
}

function modelLabel(entry: EnsembleFeedbackEntry): string {
  const resolved = entry.resolved_model?.trim();
  if (resolved) return `${resolved} · 관측됨`;
  const requested = entry.requested_model?.trim();
  if (requested) return `${requested} · 지정값`;
  return "CLI 기본값 · 미확정";
}

function FeedbackUnavailable(): ReactElement {
  return (
    <section className="m-3 mb-0 rounded-lg border border-border bg-bg p-3 text-xs">
      <div className="font-medium text-text">선택 피드백</div>
      <div role="alert" className="mt-1 text-status-failed">
        피드백 이력을 불러오지 못했습니다. 기존 비교 기능은 계속 사용할 수 있습니다.
      </div>
    </section>
  );
}
