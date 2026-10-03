// @vitest-environment jsdom

/** 앙상블 "실험 지표"(EnsembleMetrics) — "고급·실험 기능 표시"가 꺼져 있으면(기본값) 숨는다. */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CandidateBenchmarkMetrics, EnsembleFeedbackHistory } from "../../lib/ipc";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const oneMetric: CandidateBenchmarkMetrics = {
  task_id: 1,
  agent: "claude",
  model: null,
  resolved_model: "claude-sonnet",
  state: "Done",
  active_seconds: 12,
  user_turns: 1,
  completed_turns: 1,
  failed_turns: 0,
  tool_calls: 0,
  tool_errors: 0,
  tokens_in: 100,
  tokens_out: 50,
  cost_usd: 0.01,
};

const emptyFeedback: EnsembleFeedbackHistory = {
  entries: [],
  selected_count: 0,
  pending_count: 0,
  ambiguous_count: 0,
};

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string) => {
    if (cmd === "ensemble_list") return [];
    if (cmd === "ensemble_metrics") return [oneMetric];
    if (cmd === "ensemble_feedback_history") return emptyFeedback;
    return [];
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => undefined) }));

import { EnsembleView } from "./EnsembleView";
import { resetExperimentalFeaturesForTest } from "../../lib/experimental-features";

let container: HTMLDivElement;
let root: Root;

const mount = async () => {
  await act(async () => {
    root.render(
      <EnsembleView ensemble="e1" onOpenTask={() => {}} onApprove={() => {}} onHome={() => {}} />,
    );
  });
  await act(async () => {
    await Promise.resolve();
  });
};

beforeEach(() => {
  resetExperimentalFeaturesForTest(false);
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("앙상블 실험 지표 게이트", () => {
  it("고급·실험 기능이 꺼져 있으면(기본값) 실험 지표를 렌더하지 않는다", async () => {
    await mount();

    expect(container.textContent).not.toContain("실험 지표");
  });

  it("고급·실험 기능을 켜면 실험 지표가 나타난다", async () => {
    resetExperimentalFeaturesForTest(true);
    await mount();

    expect(container.textContent).toContain("실험 지표");
  });
});
