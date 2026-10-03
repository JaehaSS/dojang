import type { ReactElement } from "react";
import type { CandidateBenchmarkMetrics } from "../../lib/ipc";

interface Props {
  metrics: CandidateBenchmarkMetrics[];
  unavailable?: boolean;
}

export function EnsembleMetrics({ metrics, unavailable = false }: Props): ReactElement | null {
  if (metrics.length === 0) {
    return unavailable ? <MetricsUnavailable /> : null;
  }
  return (
    <section className="m-3 mb-0 rounded-lg border border-border bg-bg p-3">
      <div className="flex items-baseline gap-2">
        <h2 className="text-sm font-medium text-text">실험 지표</h2>
        <span className="text-[11px] text-text-muted">
          공급자별 토큰 산정 방식이 달라 절대 효율 점수로 해석하지 않습니다.
        </span>
      </div>
      <div className="mt-2 grid gap-2" style={{ gridTemplateColumns: columns(metrics.length) }}>
        {metrics.map((candidate) => (
          <CandidateMetricCard key={candidate.task_id} candidate={candidate} />
        ))}
      </div>
    </section>
  );
}

function MetricsUnavailable(): ReactElement {
  return (
    <section className="m-3 mb-0 rounded-lg border border-border bg-bg p-3 text-xs">
      <div className="font-medium text-text">실험 지표</div>
      <div role="alert" className="mt-1 text-status-failed">
        실험 지표를 불러오지 못했습니다. 대화·Diff 비교는 계속 사용할 수 있습니다.
      </div>
    </section>
  );
}

function CandidateMetricCard({
  candidate,
}: {
  candidate: CandidateBenchmarkMetrics;
}): ReactElement {
  const settledTurns = candidate.completed_turns + candidate.failed_turns;
  const identity = modelIdentity(candidate);
  return (
    <div className="rounded border border-border px-2.5 py-2 text-xs">
      <div className="flex items-center gap-2">
        <span className="font-code font-medium text-text-secondary">{candidate.agent}</span>
        <span className="min-w-0 truncate text-text-muted" title={identity.title}>
          {identity.label}
        </span>
        <span className="ml-auto text-text-muted">{candidate.state}</span>
      </div>
      {identity.requested && (
        <div className="mt-0.5 truncate pl-0.5 text-[10px] text-text-muted">
          {identity.requested}
        </div>
      )}
      <div className="mt-1.5 grid grid-cols-2 gap-x-3 gap-y-1 text-text-secondary">
        <span>활동 {formatActiveDuration(candidate.active_seconds)}</span>
        <span>
          턴 {settledTurns}/{candidate.user_turns} · 실패 {candidate.failed_turns}
        </span>
        <span>
          입력 {candidate.tokens_in.toLocaleString()} · 출력 {candidate.tokens_out.toLocaleString()}
        </span>
        <span>
          도구 {candidate.tool_calls} · 오류 {candidate.tool_errors}
        </span>
        {candidate.cost_usd > 0 && <span>비용 ${candidate.cost_usd.toFixed(2)}</span>}
      </div>
    </div>
  );
}

interface ModelIdentity {
  label: string;
  requested: string | null;
  title: string;
}

function modelIdentity(candidate: CandidateBenchmarkMetrics): ModelIdentity {
  const requested = candidate.model?.trim() || null;
  const resolved = candidate.resolved_model?.trim() || null;
  if (resolved) {
    const requestLabel = requested && requested !== resolved ? `요청 ${requested}` : null;
    return {
      label: `${resolved} · 관측됨`,
      requested: requestLabel,
      title: requestLabel ? `${resolved} (${requestLabel})` : resolved,
    };
  }
  if (requested) {
    return {
      label: `${requested} · 지정값`,
      requested: null,
      title: `${requested} — 공급자 관측값 없음`,
    };
  }
  return {
    label: "CLI 기본값 · 미확정",
    requested: null,
    title: "공급자에서 실제 모델을 관측하지 못했습니다.",
  };
}

export function formatActiveDuration(seconds: number): string {
  const safe = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(safe / 3600);
  const minutes = Math.floor((safe % 3600) / 60);
  const remainder = safe % 60;
  if (hours > 0) return `${hours}h ${pad(minutes)}m ${pad(remainder)}s`;
  if (minutes > 0) return `${minutes}m ${pad(remainder)}s`;
  return `${remainder}s`;
}

function columns(count: number): string {
  return `repeat(${Math.min(Math.max(count, 1), 3)}, minmax(0, 1fr))`;
}

function pad(value: number): string {
  return value.toString().padStart(2, "0");
}
