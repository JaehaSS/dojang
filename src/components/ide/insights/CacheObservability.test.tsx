import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { CacheObservability } from "./CacheObservability";
import { cacheReadRate } from "./cache";
import type { PromptInjectionSummary } from "../../../lib/ipc";

const prompt: PromptInjectionSummary = {
  accepted_turns: 3, observed_turns: 2, unknown_turns: 1,
  user_bytes: 10, sent_user_bytes: 300, instruction_bytes: 200,
  skill_added_bytes: 20, goal_added_bytes: 30, capsule_added_bytes: 40,
  vault_added_bytes: 50, instruction_changes: 1,
};

describe("cache observations", () => {
  it("distinguishes missing, zero-cache and zero-input observations", () => {
    expect(cacheReadRate({})).toBe("미관측");
    expect(cacheReadRate({ cache_observed_messages: 1 })).toBe("미관측");
    expect(cacheReadRate({ cache_observed_messages: 1, cache_observed_input_tokens: 100, cache_observed_read_tokens: 0 })).toBe("0%");
    expect(cacheReadRate({ cache_observed_messages: 1, cache_observed_input_tokens: 0, cache_observed_read_tokens: 0 })).toBe("입력 없음");
    expect(cacheReadRate({ cache_observed_messages: 1, cache_observed_input_tokens: 10, cache_observed_read_tokens: 11 })).toBe("미관측");
  });

  it("renders observed-cohort ratio, daily coverage, and app injection components", () => {
    const html = renderToStaticMarkup(<CacheObservability data={{
      cache_observed_messages: 2, cache_unknown_messages: 5,
      cache_observed_input_tokens: 100, cache_observed_read_tokens: 80,
      cache_days: [{ date: "2026-09-26", input_tokens: 100, cache_read_tokens: 80,
        cache_write_tokens: 10, observed_messages: 2, unknown_messages: 5 }],
      prompt_injection: prompt,
    }} />);
    for (const text of ["80%", "2026-09-26", "스킬 +20 B", "목표 +30 B", "인계 +40 B", "위키 근거 +50 B", "지침 변경 1회", "미수집 1", "토큰·청구량이 아닙니다"]) {
      expect(html).toContain(text);
    }
  });

  it("does not turn unavailable admission metrics into a zero count", () => {
    const missing = renderToStaticMarkup(<CacheObservability data={{ prompt_injection: null }} />);
    expect(missing).toContain("조회 불가");
    expect(missing).not.toContain("접수 0");
    const old = renderToStaticMarkup(<CacheObservability data={{ prompt_injection: { ...prompt, observed_turns: 0 } }} />);
    expect(old).toContain("미수집");
    expect(old).not.toContain("스킬 +");
  });
});
