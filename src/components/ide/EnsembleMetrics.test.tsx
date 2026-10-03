import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { CandidateBenchmarkMetrics } from "../../lib/ipc";
import { EnsembleMetrics, formatActiveDuration } from "./EnsembleMetrics";

const claude: CandidateBenchmarkMetrics = {
  task_id: 1,
  agent: "claude",
  model: "opus",
  resolved_model: "claude-opus-4-8",
  state: "AwaitingReview",
  active_seconds: 65,
  user_turns: 2,
  completed_turns: 2,
  failed_turns: 0,
  tool_calls: 8,
  tool_errors: 1,
  tokens_in: 1234,
  tokens_out: 56,
  cost_usd: 0.42,
};

const codex: CandidateBenchmarkMetrics = {
  ...claude,
  task_id: 2,
  agent: "codex",
  model: null,
  resolved_model: null,
  active_seconds: 9,
  completed_turns: 1,
  failed_turns: 1,
  cost_usd: 0,
};

describe("EnsembleMetrics", () => {
  it("renders comparable activity, reliability, token, tool, and available cost data", () => {
    const html = renderToStaticMarkup(<EnsembleMetrics metrics={[claude, codex]} />);

    expect(html).toContain("실험 지표");
    expect(html).toContain("1m 05s");
    expect(html).toContain("입력 1,234 · 출력 56");
    expect(html).toContain("도구 8 · 오류 1");
    expect(html).toContain("$0.42");
    expect(html).toContain("claude-opus-4-8 · 관측됨");
    expect(html).toContain("요청 opus");
    expect(html).toContain("CLI 기본값 · 미확정");
    expect(html).toContain("공급자별 토큰 산정 방식이 달라");
  });

  it("labels a requested model without provider observation as a specified value", () => {
    const html = renderToStaticMarkup(
      <EnsembleMetrics
        metrics={[
          {
            ...codex,
            model: "gpt-5.6-sol",
          },
        ]}
      />,
    );

    expect(html).toContain("gpt-5.6-sol · 지정값");
  });

  it("formats zero and hour-scale durations without rounding away seconds", () => {
    expect(formatActiveDuration(0)).toBe("0s");
    expect(formatActiveDuration(3661)).toBe("1h 01m 01s");
  });

  it("makes a metrics-only load failure visible without replacing the comparison view", () => {
    const html = renderToStaticMarkup(<EnsembleMetrics metrics={[]} unavailable />);

    expect(html).toContain("실험 지표를 불러오지 못했습니다");
  });
});
