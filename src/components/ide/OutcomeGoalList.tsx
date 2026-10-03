import type { ReactElement } from "react";
import type { OutcomeInsights } from "../../lib/ipc";
import { deriveOutcomeOpportunities, type OutcomeOpportunity } from "./outcome-opportunities";

interface Props {
  data: OutcomeInsights;
}

export function OutcomeGoalList({ data }: Props): ReactElement {
  const opportunities = deriveOutcomeOpportunities(data);
  if (opportunities.length === 0) {
    return <p className="text-xs text-text-muted">현재 집계에서 즉시 드러난 gap이 없습니다.</p>;
  }
  return (
    <section aria-label="추천 goal" className="rounded-md border border-border bg-raised/30 p-3">
      <h3 className="text-xs font-medium uppercase tracking-wide text-text-secondary">추천 goal</h3>
      <div className="mt-2 grid gap-2 sm:grid-cols-2">
        {opportunities.map((opportunity) => (
          <OutcomeGoal key={opportunity.id} opportunity={opportunity} />
        ))}
      </div>
    </section>
  );
}

function OutcomeGoal({ opportunity }: { opportunity: OutcomeOpportunity }): ReactElement {
  return (
    <article className="rounded border border-border bg-surface p-2.5">
      <div className="text-xs font-medium text-text">{opportunity.title}</div>
      <p className="mt-1 text-xs leading-relaxed text-text-muted">{opportunity.reason}</p>
    </article>
  );
}
