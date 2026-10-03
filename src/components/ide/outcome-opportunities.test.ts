import { describe, expect, it } from "vitest";
import type { OutcomeInsights } from "../../lib/ipc";
import { deriveOutcomeOpportunities } from "./outcome-opportunities";

const baseline: OutcomeInsights = {
  task_count: 92,
  accepted_task_count: 12,
  goal_contract_task_count: 0,
  ready_accepted_task_count: 0,
  ensemble_count: 5,
  selected_ensemble_count: 2,
  ambiguous_ensemble_count: 0,
  no_selection_ensemble_count: 3,
  average_accept_seconds: 23_844.2,
  no_reexplanation_target_task_count: 0,
  no_reexplanation_observed_task_count: 0,
  no_reexplanation_success_task_count: 0,
  no_reexplanation_unmeasured_task_count: 0,
  no_reexplanation_completion_rate: null,
};

describe("deriveOutcomeOpportunities", () => {
  it("derives every observed adoption gap in causal order", () => {
    const opportunities = deriveOutcomeOpportunities(baseline);

    expect(opportunities.map(({ id }) => id)).toEqual(["verification", "no_reexplanation"]);
    expect(opportunities.map(({ action }) => action)).toEqual([null, null]);
    expect(opportunities[0]?.reason).toContain("0 / 12");
    expect(opportunities[1]?.reason).toContain("측정 대상이 0건");
  });

  it("reports incomplete observation coverage as the measurement goal", () => {
    const incomplete: OutcomeInsights = {
      ...baseline,
      no_reexplanation_target_task_count: 2,
      no_reexplanation_observed_task_count: 1,
      no_reexplanation_unmeasured_task_count: 1,
    };

    const opportunity = deriveOutcomeOpportunities(incomplete).find(
      ({ id }) => id === "no_reexplanation",
    );

    expect(opportunity?.reason).toContain("관측 1 / 2");
    expect(opportunity?.reason).toContain("미계측 1");
  });

  it("returns no goal when every measured gap is closed", () => {
    const closed: OutcomeInsights = {
      ...baseline,
      goal_contract_task_count: baseline.task_count,
      ready_accepted_task_count: baseline.accepted_task_count,
      no_reexplanation_completion_rate: 0.8,
    };

    expect(deriveOutcomeOpportunities(closed)).toEqual([]);
  });

  it("does not recommend verification or measurement without eligible tasks", () => {
    const empty: OutcomeInsights = {
      ...baseline,
      task_count: 0,
      accepted_task_count: 0,
      goal_contract_task_count: 0,
      ready_accepted_task_count: 0,
      no_reexplanation_completion_rate: null,
    };

    expect(deriveOutcomeOpportunities(empty)).toEqual([]);
  });
});
