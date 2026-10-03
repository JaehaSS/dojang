import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { EnsembleFeedbackHistory } from "../../lib/ipc";
import { EnsembleFeedback } from "./EnsembleFeedback";

const history: EnsembleFeedbackHistory = {
  selected_count: 1,
  pending_count: 1,
  ambiguous_count: 1,
  entries: [
    {
      ensemble: "current",
      updated_at: 30,
      candidate_count: 2,
      selection_status: "selected",
      selected_task_id: 1,
      selected_agent: "claude",
      requested_model: "opus",
      resolved_model: "claude-opus-4-8",
    },
    {
      ensemble: "pending",
      updated_at: 20,
      candidate_count: 2,
      selection_status: "pending",
      selected_task_id: null,
      selected_agent: null,
      requested_model: null,
      resolved_model: null,
    },
    {
      ensemble: "ambiguous",
      updated_at: 10,
      candidate_count: 3,
      selection_status: "ambiguous",
      selected_task_id: null,
      selected_agent: null,
      requested_model: null,
      resolved_model: null,
    },
  ],
};

describe("EnsembleFeedback", () => {
  it("renders actual selections, model evidence, and uncertain states", () => {
    const html = renderToStaticMarkup(
      <EnsembleFeedback history={history} currentEnsemble="current" />,
    );

    expect(html).toContain("선택 피드백");
    expect(html).toContain("선정 완료 1");
    expect(html).toContain("claude-opus-4-8 · 관측됨");
    expect(html).toContain("현재");
    expect(html).toContain("선택 대기");
    expect(html).toContain("복수 승인 · 승자 미추정");
  });

  it("keeps a feedback-only load failure visible", () => {
    const html = renderToStaticMarkup(
      <EnsembleFeedback history={null} currentEnsemble="current" unavailable />,
    );

    expect(html).toContain("피드백 이력을 불러오지 못했습니다");
  });
});
