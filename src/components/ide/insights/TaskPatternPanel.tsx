import type { ReactElement } from "react";

import type { TaskPatterns } from "../../../lib/ipc";
import { Metric, RankBar } from "./parts";
import { fmtDuration, fmtInt } from "./format";

interface Props {
  /** 상위가 로드한 집계. 아직 없으면 null — 같은 조회를 두 번 하지 않는다. */
  data: TaskPatterns | null;
}

const pct = (part: number, whole: number) => (whole > 0 ? Math.round((part / whole) * 100) : 0);

/**
 * 작업 패턴 — "작업이 어떻게 굴러갔는가" (설계 0054 §6.2).
 *
 * 사용량 리포트와 데이터 원천이 다르다(작업 DB) — 위 리포트가 비어 있어도 이 패널은
 * 채워진다. 집계 자체는 상위가 한 번 로드해 내려준다.
 */
export function TaskPatternPanel({ data }: Props): ReactElement {
  if (!data) return <div className="text-text-muted text-sm py-8">집계 중…</div>;
  if (data.funnel.total === 0) {
    return (
      <div className="text-text-muted text-sm border border-border rounded-lg p-8 text-center">
        이 기간에 작업 기록이 없습니다.
      </div>
    );
  }

  const { funnel, followup, discard_trend: trend, role_outcomes: roles } = data;
  const maxMonth = Math.max(1, ...trend.map((m) => m.total));

  const stages = [
    { label: "생성", value: funnel.total, drop: null as string | null },
    {
      label: "실행",
      value: funnel.started,
      drop: funnel.total > funnel.started ? `${funnel.total - funnel.started} 큐에서 멈춤` : null,
    },
    {
      label: "검토",
      value: funnel.reviewed,
      drop: funnel.failed > 0 ? `${funnel.failed} 실패` : null,
    },
    {
      label: "승인",
      value: funnel.done,
      drop: funnel.discarded > 0 ? `${funnel.discarded} 폐기` : null,
    },
  ];

  return (
    <>
      {/* ── 흐름 ── */}
      <div className="bg-surface border border-border rounded-lg p-4 mb-2.5">
        <div className="text-xs text-text-secondary mb-3">흐름 — 상태별 도달</div>
        {stages.map((stage) => (
          <div key={stage.label} className="mb-2.5 last:mb-0">
            <div className="flex items-baseline justify-between gap-3 mb-1">
              <span className="text-sm">{stage.label}</span>
              <span className="text-xs font-code text-text-secondary shrink-0">
                {fmtInt(stage.value)}
                {stage.drop && <span className="text-text-muted"> · ▼{stage.drop}</span>}
              </span>
            </div>
            <RankBar value={stage.value} max={funnel.total} />
          </div>
        ))}
      </div>

      {/* ── 폐기 추세 ──
          범위 칩을 따르지 않는다. 7d로 자르면 상승 추세 자체가 사라져 이 레인의 존재
          이유가 없어진다(설계 0054 §6.2). */}
      <div className="bg-surface border border-border rounded-lg p-4 mb-2.5">
        <div className="flex items-baseline justify-between mb-3">
          <span className="text-xs text-text-secondary">폐기 추세 — 월별</span>
          <span className="text-xs text-text-muted">범위 칩과 무관 · 전체 기간</span>
        </div>
        {trend.length === 0 ? (
          <div className="text-text-muted text-sm">기록이 없습니다.</div>
        ) : (
          trend.map((month) => (
            <div key={month.month} className="mb-2 last:mb-0">
              <div className="flex items-baseline justify-between gap-3 mb-1">
                <span className="text-sm font-code">{month.month}</span>
                <span className="text-xs font-code text-text-secondary shrink-0">
                  {pct(month.discarded, month.total)}% · {fmtInt(month.discarded)}/
                  {fmtInt(month.total)}
                </span>
              </div>
              <RankBar value={month.total} max={maxMonth} />
            </div>
          ))
        )}
      </div>

      {/* ── 한 번에 끝났나 ── */}
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-2.5 mb-2.5">
        <Metric
          label="후속 입력 없음"
          value={`${fmtInt(followup.total - followup.with_followup)}건`}
        />
        <Metric
          label="후속 입력 있음"
          value={`${pct(followup.with_followup, followup.total)}%`}
        />
        <Metric label="소요 중앙값" value={fmtDuration(data.duration_p50)} />
        <Metric label="소요 p90" value={fmtDuration(data.duration_p90)} />
      </div>

      {/* ── 역할별 결말 ── */}
      <div className="bg-surface border border-border rounded-lg overflow-x-auto">
        <table className="w-full text-sm min-w-[420px]">
          <thead>
            <tr className="text-xs text-text-secondary border-b border-border">
              <th className="text-left font-medium px-3 py-2">역할</th>
              <th className="text-right font-medium px-3 py-2">건수</th>
              <th className="text-right font-medium px-3 py-2">승인</th>
              <th className="text-right font-medium px-3 py-2">폐기</th>
            </tr>
          </thead>
          <tbody>
            {roles.map((role) => (
              <tr key={role.role} className="border-b border-border last:border-0">
                <td className="px-3 py-2 truncate max-w-[180px]" title={role.role}>
                  {role.role}
                </td>
                <td className="px-3 py-2 text-right font-code">{fmtInt(role.count)}</td>
                <td className="px-3 py-2 text-right font-code">
                  {pct(role.done, role.count)}%
                </td>
                <td className="px-3 py-2 text-right font-code text-text-secondary">
                  {pct(role.discarded, role.count)}%
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </>
  );
}
