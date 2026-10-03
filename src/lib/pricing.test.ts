import { describe, expect, it } from "vitest";
import type { ModelStat } from "./ipc";
import { modelCost, totalCost } from "./pricing";

const claude = (overrides: Partial<ModelStat> = {}): ModelStat => ({
  model: "claude-sonnet-4-6",
  provider: "claude",
  messages: 1,
  sessions: 1,
  input_tokens: 100,
  output_tokens: 10,
  cache_creation_tokens: 0,
  cache_read_tokens: 50,
  total_tokens: 160,
  hours: Array(24).fill(0),
  ...overrides,
});

describe("pricing cache coverage", () => {
  it("does not price Codex token records with the Claude heuristic", () => {
    expect(modelCost(claude({ provider: "codex", model: "gpt-5.6" }))).toBeNull();
  });

  it("marks a Claude model with unknown cache fields as partial", () => {
    const model = claude({ cache_unknown_messages: 1 });
    expect(modelCost(model)).toBeNull();
    expect(totalCost([model])).toEqual({ usd: 0, partial: true });
  });

  it("keeps explicit zero-cache observations priceable", () => {
    expect(modelCost(claude({ cache_observed_messages: 1, cache_read_tokens: 0 }))).not.toBeNull();
  });
});
