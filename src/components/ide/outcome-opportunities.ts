import type { OutcomeInsights } from "../../lib/ipc";

export type OutcomeOpportunityId = "verification" | "no_reexplanation";

export type OutcomeOpportunityAction = null;

export interface OutcomeOpportunity {
  id: OutcomeOpportunityId;
  title: string;
  reason: string;
  action: OutcomeOpportunityAction;
  actionLabel: string | null;
}

export function deriveOutcomeOpportunities(data: OutcomeInsights): OutcomeOpportunity[] {
  const opportunities: OutcomeOpportunity[] = [];
  if (data.ready_accepted_task_count < data.accepted_task_count) {
    opportunities.push({
      id: "verification",
      title: "승인 전 검증",
      reason: `검증 evidence가 있는 승인은 ${data.ready_accepted_task_count} / ${data.accepted_task_count}건입니다.`,
      action: null,
      actionLabel: null,
    });
  }
  if (data.task_count > 0 && data.no_reexplanation_completion_rate == null) {
    const reason =
      data.no_reexplanation_target_task_count === 0
        ? "versioned memory 주입 후 완료된 측정 대상이 0건입니다."
        : `관측 ${data.no_reexplanation_observed_task_count} / ${data.no_reexplanation_target_task_count} · 미계측 ${data.no_reexplanation_unmeasured_task_count}건입니다.`;
    opportunities.push({
      id: "no_reexplanation",
      title: "무재설명 완료율 계측",
      reason,
      action: null,
      actionLabel: null,
    });
  }
  return opportunities;
}
