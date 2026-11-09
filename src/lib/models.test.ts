import { describe, it, expect } from "vitest";
import {
  AGENT_MODEL_CATALOG,
  modelsForAgent,
  modelsForAgentWithObserved,
  normalizeReasoningEffort,
  reasoningEffortsForModel,
  type ObservedModel,
} from "./models";

const observed = (agent: string, model: string, last_used_at: number): ObservedModel => ({
  agent,
  model,
  last_used_at,
});

describe("modelsForAgentWithObserved", () => {
  it("조회 목록을 우선하고 사용 이력과 합치면서 중복을 제거한다", () => {
    const available = [{ id: "gemini-4-argon-high", label: "Gemini 4 Argon (High)" }];
    const merged = modelsForAgentWithObserved("agy", [
      observed("agy", "gemini-4-argon-high", 200),
      observed("agy", "custom-model", 100),
      observed("codex", "gpt-5.6-sol", 300),
    ], available);
    expect(merged.map((m) => m.id)).toEqual(["gemini-4-argon-high", "custom-model"]);
    expect(merged[0].label).toBe("Gemini 4 Argon (High)");
  });

  it("조회 목록이 없거나 비어 있으면 내장 목록을 유지한다", () => {
    expect(modelsForAgentWithObserved("agy", [], null)).toEqual(modelsForAgent("agy"));
    expect(modelsForAgentWithObserved("agy", [], [])).toEqual(modelsForAgent("agy"));
  });

  it("카탈로그에 없는 관측 모델을 뒤에 덧붙인다", () => {
    const merged = modelsForAgentWithObserved("claude", [
      observed("claude", "claude-mythos-1", 100),
    ]);

    expect(merged.map((m) => m.id)).toContain("claude-mythos-1");
    expect(merged[merged.length - 1].id).toBe("claude-mythos-1");
    expect(merged[merged.length - 1].label).toContain("사용 기록");
  });

  it("카탈로그에 이미 있는 모델은 중복시키지 않고 사람이 붙인 라벨을 남긴다", () => {
    const merged = modelsForAgentWithObserved("claude", [observed("claude", "claude-opus-5", 100)]);

    expect(merged.filter((m) => m.id === "claude-opus-5")).toHaveLength(1);
    expect(merged.find((m) => m.id === "claude-opus-5")?.label).toBe("Opus 5");
  });

  it("다른 벤더의 관측은 섞지 않는다", () => {
    const merged = modelsForAgentWithObserved("claude", [observed("codex", "gpt-5.6-sol", 100)]);

    expect(merged.map((m) => m.id)).not.toContain("gpt-5.6-sol");
    expect(merged).toEqual(modelsForAgent("claude"));
  });

  it("최근 사용순으로 정렬하고 같은 모델은 한 번만 넣는다", () => {
    const merged = modelsForAgentWithObserved("claude", [
      observed("claude", "old-model", 10),
      observed("claude", "new-model", 900),
      observed("claude", "new-model", 500),
    ]);
    const extra = merged.slice(modelsForAgent("claude").length).map((m) => m.id);

    expect(extra).toEqual(["new-model", "old-model"]);
  });

  it("관측이 없으면 카탈로그를 그대로 돌려준다", () => {
    expect(modelsForAgentWithObserved("claude", [])).toEqual(modelsForAgent("claude"));
    // 미등록 벤더는 관측만으로 목록이 생긴다 — 자유입력 폴백을 대체하지는 않는다.
    expect(modelsForAgentWithObserved("mystery", [observed("mystery", "x-1", 1)])).toHaveLength(1);
  });
});

describe("modelsForAgent", () => {
  it("알려진 벤더는 비어있지 않은 후보 목록을 반환한다", () => {
    for (const key of Object.keys(AGENT_MODEL_CATALOG)) {
      expect(modelsForAgent(key).length).toBeGreaterThan(0);
    }
  });

  it("claude 후보에는 opus 계열 ID가 포함된다", () => {
    const ids = modelsForAgent("claude").map((m) => m.id);
    expect(ids.some((id) => id.includes("opus"))).toBe(true);
  });

  it("codex 후보에는 현재 세대의 Sol, Terra, Luna 모델이 포함된다", () => {
    const ids = modelsForAgent("codex").map((m) => m.id);
    expect(ids).toEqual(expect.arrayContaining(["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"]));
  });

  it("Antigravity 후보에는 설치된 CLI의 Gemini 3.6 모델이 포함된다", () => {
    const ids = modelsForAgent("agy").map((model) => model.id);

    expect(ids).toEqual(
      expect.arrayContaining([
        "gemini-3.6-flash-high",
        "gemini-3.6-flash-medium",
        "gemini-3.6-flash-low",
      ]),
    );
  });

  it("Codex 모델별 지원 effort를 반환한다", () => {
    expect(reasoningEffortsForModel("codex", "gpt-5.6-sol")).toEqual([
      "low",
      "medium",
      "high",
      "xhigh",
      "max",
      "ultra",
    ]);
    expect(reasoningEffortsForModel("codex", "gpt-5.6-luna")).not.toContain("ultra");
    expect(reasoningEffortsForModel("codex", "gpt-5.4")).toEqual([
      "low",
      "medium",
      "high",
      "xhigh",
    ]);
    expect(reasoningEffortsForModel("codex", "")).toEqual([
      "low",
      "medium",
      "high",
      "xhigh",
    ]);
    expect(reasoningEffortsForModel("codex", "custom-model")).toEqual([
      "low",
      "medium",
      "high",
      "xhigh",
    ]);
  });

  it("claude는 CLI가 모델 무관 전역 검증하는 5종 effort를 노출한다", () => {
    expect(reasoningEffortsForModel("claude", "opus")).toEqual([
      "low",
      "medium",
      "high",
      "xhigh",
      "max",
    ]);
    expect(reasoningEffortsForModel("claude", "my-custom-claude-model")).toEqual([
      "low",
      "medium",
      "high",
      "xhigh",
      "max",
    ]);
  });

  it("Antigravity와 실행 별칭에는 low, medium, high만 노출한다", () => {
    for (const agent of ["agy", "gemini", "antigravity"]) {
      expect(reasoningEffortsForModel(agent, "gemini-3-pro")).toEqual(["low", "medium", "high"]);
    }
    expect(reasoningEffortsForModel("crush", "google/gemini-3-pro")).toEqual([]);
  });

  it("모델 변경 시 지원하지 않는 effort만 기본값으로 되돌린다", () => {
    expect(normalizeReasoningEffort("codex", "gpt-5.6-sol", "ultra")).toBe("ultra");
    expect(normalizeReasoningEffort("codex", "gpt-5.6-luna", "ultra")).toBe("");
    expect(normalizeReasoningEffort("claude", "opus", "high")).toBe("high");
    expect(normalizeReasoningEffort("agy", "gemini-3-pro", "high")).toBe("high");
    expect(normalizeReasoningEffort("gemini", "gemini-3-pro", "xhigh")).toBe("");
  });

  it("알 수 없는 벤더는 빈 배열", () => {
    expect(modelsForAgent("no-such-vendor")).toEqual([]);
    expect(modelsForAgent("")).toEqual([]);
  });

  it("모든 후보는 비어있지 않은 id를 가진다", () => {
    for (const key of Object.keys(AGENT_MODEL_CATALOG)) {
      for (const m of modelsForAgent(key)) {
        expect(m.id.trim().length).toBeGreaterThan(0);
      }
    }
  });
});
