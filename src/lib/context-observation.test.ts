import { describe, expect, it } from "vitest";
import {
  EMPTY_CONTEXT_OBSERVATION,
  foldContextObservation,
  MODEL_CHANGE_INVALIDATION,
} from "./context-observation";

const codex = (over: Record<string, unknown> = {}) => ({
  kind: "context_usage",
  context_tokens: 147_043,
  context_window: 258_400,
  observed_at: 1_725_000_000,
  source: "codex_session",
  valid: true,
  ...over,
});

describe("foldContextObservation", () => {
  it("accepts Codex App Server observations with the same seconds contract", () => {
    const out = foldContextObservation(EMPTY_CONTEXT_OBSERVATION, [codex({ source: "app_server" })], "codex");
    expect(out.observation).toEqual({ contextTokens: 147_043, contextWindow: 258_400, observedAt: 1_725_000_000, source: "app_server" });
    expect(foldContextObservation(EMPTY_CONTEXT_OBSERVATION, [codex({ source: "app_server", observed_at: 1_725_000_000_000 })], "codex").observation).toBeNull();
  });

  it("accepts Antigravity step observations without a window", () => {
    const out = foldContextObservation(
      EMPTY_CONTEXT_OBSERVATION,
      [codex({ source: "agy_step", context_tokens: 16_886, context_window: undefined })],
      "agy",
    );
    expect(out.observation).toEqual({ contextTokens: 16_886, contextWindow: null, observedAt: 1_725_000_000, source: "agy_step" });
  });

  it("does not replace main context with a peer debate participant", () => {
    const parent = foldContextObservation(EMPTY_CONTEXT_OBSERVATION, [codex()], "codex");
    expect(foldContextObservation(parent, [codex({ context_tokens: 42, speaker: "right" }), { kind: "context_cleared", speaker: "right" }], "codex")).toEqual(parent);
  });
  it("child metadata and result do not replace main context usage", () => {
    const parent = foldContextObservation(EMPTY_CONTEXT_OBSERVATION, [codex()], "codex");
    const child = codex({ kind: "subagent_update", context_tokens: 1, context_window: 2 });
    expect(foldContextObservation(parent, [child, { kind: "subagent_message" }], "codex")).toEqual(parent);
  });
  it("last call and its actual Codex window replace the legacy cumulative total", () => {
    const out = foldContextObservation(
      EMPTY_CONTEXT_OBSERVATION,
      [{ kind: "context_usage", context_tokens: 7_451_604 }, codex()],
      "codex",
    );
    expect(out.observation).toEqual({
      contextTokens: 147_043,
      contextWindow: 258_400,
      observedAt: 1_725_000_000,
      source: "codex_session",
    });
  });

  it("rejects malformed or old Codex observations", () => {
    for (const event of [
      codex({ context_window: -1 }),
      codex({ context_tokens: Infinity }),
      codex({ observed_at: Number.MAX_SAFE_INTEGER }),
      { kind: "context_usage", context_tokens: 7_451_604 },
    ])
      expect(
        foldContextObservation(EMPTY_CONTEXT_OBSERVATION, [event], "codex").observation,
      ).toBeNull();
  });

  it("clears on reset and accepts a later valid observation", () => {
    const out = foldContextObservation(
      EMPTY_CONTEXT_OBSERVATION,
      [codex(), { kind: "context_cleared" }, codex({ context_tokens: 2_000 })],
      "codex",
    );
    expect(out.observation?.contextTokens).toBe(2_000);
  });

  it("uses the same last observation for full history and remote continuation batches", () => {
    const events = [codex(), { kind: "text" }, codex({ context_tokens: 2_000 })];
    const history = foldContextObservation(EMPTY_CONTEXT_OBSERVATION, events, "codex");
    const continuation = foldContextObservation(
      foldContextObservation(EMPTY_CONTEXT_OBSERVATION, events.slice(0, 2), "codex"),
      events.slice(2),
      "codex",
    );
    expect(continuation).toEqual(history);
  });

  it("keeps a model-change invalidation until the next invocation", () => {
    const out = foldContextObservation(EMPTY_CONTEXT_OBSERVATION, [
      codex(),
      { kind: "context_usage", context_tokens: 0, source: "model_change", valid: false },
      codex({ context_tokens: 100_000 }),
      { kind: "context_usage", context_tokens: 0, source: "codex_session", valid: false },
    ], "codex");
    expect(out).toEqual(MODEL_CHANGE_INVALIDATION);
    const restored = foldContextObservation(
      out,
      [{ kind: "model_snapshot", source: "invocation" }, codex({ context_tokens: 2_000 })],
      "codex",
    );
    expect(restored.observation?.contextTokens).toBe(2_000);
  });

  it("reconstructs Claude legacy values with the model-derived window", () => {
    const out = foldContextObservation(
      EMPTY_CONTEXT_OBSERVATION,
      [{ kind: "context_usage", context_tokens: 200_001 }],
      "claude",
    );
    expect(out.observation?.source).toBe("claude_legacy");
  });
});
