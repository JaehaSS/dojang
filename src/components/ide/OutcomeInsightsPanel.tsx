import { useEffect, useState, type ReactElement } from "react";
import {
  outcomeInsights,
  type InsightsRange,
  type OutcomeInsights,
} from "../../lib/ipc";
import { describeNoReexplanation } from "./no-reexplanation-metric";
import { OutcomeGoalList } from "./OutcomeGoalList";

interface PanelProps {
  range: InsightsRange;
}

interface StateProps {
  data: OutcomeInsights | null;
  loading: boolean;
  error: string | null;
}

export function OutcomeInsightsPanel({ range }: PanelProps): ReactElement {
  const [data, setData] = useState<OutcomeInsights | null>(null);
  const [loading, setLoading] = useState<boolean>(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    setLoading(true);
    setError(null);
    outcomeInsights(range)
      .then((result) => active && setData(result))
      .catch((reason: unknown) => active && setError(String(reason)))
      .finally(() => active && setLoading(false));
    return () => {
      active = false;
    };
  }, [range]);

  return (
    <OutcomeInsightsPanelState
      data={data}
      loading={loading}
      error={error}
    />
  );
}

export function OutcomeInsightsPanelState({
  data,
  loading,
  error,
}: StateProps): ReactElement {
  if (loading) return <PanelMessage>AX 결과 집계 중…</PanelMessage>;
  if (error || !data) {
    return (
      <PanelMessage alert>
        AX 결과를 불러오지 못했습니다. 사용량 인사이트는 계속 사용할 수 있습니다.
      </PanelMessage>
    );
  }
  return (
    <section aria-label="AX 결과" className="space-y-3">
      <div>
        <h2 className="text-sm font-medium text-text">AX 결과</h2>
        <p className="mt-1 text-xs text-text-muted">
          Done은 사용자 승인 상태이며 검증된 Goal 완료와 동일하지 않습니다.
        </p>
      </div>
      <OutcomeMetricGrid data={data} />
      <OutcomeGoalList data={data} />
      <div className="text-xs text-text-muted">
        ensemble 복수 승인 {data.ambiguous_ensemble_count} · 선택 없음 {data.no_selection_ensemble_count}
      </div>
    </section>
  );
}

function OutcomeMetricGrid({ data }: { data: OutcomeInsights }): ReactElement {
  const noReexplanation = describeNoReexplanation(data);
  const ratios: Array<{ label: string; value: number; total: number }> = [
    { label: "Goal Contract", value: data.goal_contract_task_count, total: data.task_count },
    { label: "사용자 승인", value: data.accepted_task_count, total: data.task_count },
    {
      label: "승인 검증",
      value: data.ready_accepted_task_count,
      total: data.accepted_task_count,
    },
    { label: "ensemble 선택", value: data.selected_ensemble_count, total: data.ensemble_count },
  ];
  return (
    <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-4">
      {ratios.map((metric) => (
        <RatioMetric key={metric.label} {...metric} />
      ))}
      <TextMetric label="평균 승인 시간" value={formatDuration(data.average_accept_seconds)} />
      <Metric
        label="무재설명 완료율"
        value={noReexplanation.value}
        detail={noReexplanation.detail}
      />
    </div>
  );
}

function RatioMetric({
  label,
  value,
  total,
}: {
  label: string;
  value: number;
  total: number;
}): ReactElement {
  const percent = total > 0 ? `${Math.round((value / total) * 100)}%` : "—";
  return <Metric label={label} value={`${value} / ${total}`} detail={percent} />;
}

function TextMetric({ label, value }: { label: string; value: string }): ReactElement {
  return <Metric label={label} value={value} />;
}

function Metric({
  label,
  value,
  detail,
}: {
  label: string;
  value: string;
  detail?: string;
}): ReactElement {
  return (
    <div className="rounded-md border border-border bg-surface p-3">
      <div className="text-xs text-text-secondary">{label}</div>
      <div className="mt-0.5 text-lg font-medium text-text">{value}</div>
      {detail && <div className="text-xs text-text-muted">{detail}</div>}
    </div>
  );
}

function PanelMessage({
  children,
  alert = false,
}: {
  children: string;
  alert?: boolean;
}): ReactElement {
  return (
    <div
      role={alert ? "alert" : "status"}
      className={
        alert ? "text-sm text-status-failed" : "py-10 text-center text-sm text-text-muted"
      }
    >
      {children}
    </div>
  );
}

function formatDuration(seconds: number | null): string {
  if (seconds == null) return "계측 전 · N/A";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}분`;
  return `${Math.floor(minutes / 60)}시간 ${minutes % 60}분`;
}
