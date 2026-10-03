import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { OutcomeInsights } from "../../lib/ipc";
import { OutcomeInsightsPanelState } from "./OutcomeInsightsPanel";

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

describe("OutcomeInsightsPanelState", () => {
  it("renders adoption and accepted-outcome denominators without inventing missing measures", () => {
    const html = renderToStaticMarkup(
      <OutcomeInsightsPanelState data={baseline} loading={false} error={null} />,
    );

    expect(html).toContain("AX 결과");
    expect(html).toContain("Goal Contract");
    expect(html).toContain("0 / 92");
    expect(html).toContain("승인 검증");
    expect(html).toContain("0 / 12");
    expect(html).toContain("ensemble 선택");
    expect(html).toContain("2 / 5");
    expect(html).toContain("선택 없음 3");
    expect(html).toContain("무재설명 완료율");
    expect(html).toContain("계측 전 · N/A");
    expect(html).toContain("Done은 사용자 승인");
    expect(html).not.toContain("무재설명 완료율 0%");
  });

  it("keeps an outcome-only failure visible without claiming there is no data", () => {
    const html = renderToStaticMarkup(
      <OutcomeInsightsPanelState data={null} loading={false} error="database unavailable" />,
    );

    expect(html).toContain("AX 결과를 불러오지 못했습니다");
    expect(html).not.toContain("이 기간에 결과가 없습니다");
  });

  it("shows a conservative zero-followup rate when observation is complete", () => {
    const measured: OutcomeInsights = {
      ...baseline,
      no_reexplanation_target_task_count: 2,
      no_reexplanation_observed_task_count: 2,
      no_reexplanation_success_task_count: 1,
      no_reexplanation_completion_rate: 0.5,
    };

    const html = renderToStaticMarkup(
      <OutcomeInsightsPanelState data={measured} loading={false} error={null} />,
    );

    expect(html).toContain("50% · 1 / 2");
    expect(html).toContain("관측 2 / 2");
    expect(html).toContain("초기 지시 후 추가 입력 0회의 보수적 proxy");
  });

  it("keeps incomplete observation at N/A and exposes coverage", () => {
    const incomplete: OutcomeInsights = {
      ...baseline,
      no_reexplanation_target_task_count: 2,
      no_reexplanation_observed_task_count: 1,
      no_reexplanation_success_task_count: 1,
      no_reexplanation_unmeasured_task_count: 1,
    };

    const html = renderToStaticMarkup(
      <OutcomeInsightsPanelState data={incomplete} loading={false} error={null} />,
    );

    expect(html).toContain("N/A");
    expect(html).toContain("관측 1 / 2");
    expect(html).toContain("미계측 1");
    expect(html).not.toContain("50%");
  });

  it("shows zero as a measured baseline", () => {
    const empty = Object.fromEntries(
      Object.keys(baseline).map((key) => [key, key.includes("rate") ? null : 0]),
    ) as unknown as OutcomeInsights;
    const html = renderToStaticMarkup(
      <OutcomeInsightsPanelState data={empty} loading={false} error={null} />,
    );

    expect(html).toContain("0 / 0");
    expect(html).toContain("계측 전 · N/A");
    expect(html).not.toContain("이 기간에 결과가 없습니다");
  });

  it("turns observed gaps into explicit next actions without forcing them", () => {
    const html = renderToStaticMarkup(
      <OutcomeInsightsPanelState data={baseline} loading={false} error={null} />,
    );

    expect(html).toContain("추천 goal");
    expect(html).toContain("승인 전 검증");
    expect(html).toContain("측정 대상이 0건");
  });
});
