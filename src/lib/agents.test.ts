import { describe, expect, it } from "vitest";
import {
  AGENT_PRESETS,
  badgeLabelFor,
  labelFor,
  normalizeAgentSelection,
  singleAgentSelection,
} from "./agents";

describe("AGENT_PRESETS", () => {
  it("지원 종료된 Gemini CLI를 선택지에 노출하지 않는다", () => {
    expect(AGENT_PRESETS.some((preset) => preset.key === "gemini")).toBe(false);
    expect(AGENT_PRESETS.some((preset) => preset.key === "agy")).toBe(true);
  });

  it("제거된 opencode를 선택지에 노출하지 않는다", () => {
    expect(AGENT_PRESETS.some((preset) => preset.key === "opencode")).toBe(false);
  });
});

describe("normalizeAgentSelection", () => {
  it("저장된 Gemini 선택을 Antigravity로 마이그레이션하고 중복을 제거한다", () => {
    expect(normalizeAgentSelection(["gemini", "agy", "codex"])).toEqual(["agy", "codex"]);
  });

  it("저장된 opencode 선택을 Claude로 치환한다", () => {
    expect(normalizeAgentSelection(["opencode", "codex"])).toEqual(["claude", "codex"]);
  });

  it("유효한 선택이 없으면 Claude를 사용한다", () => {
    expect(normalizeAgentSelection([" ", ""])).toEqual(["claude"]);
  });
});

describe("labelFor", () => {
  it("프리셋 키를 전체 라벨로 매핑한다", () => {
    expect(labelFor("claude")).toBe("Claude Code");
    expect(labelFor("codex")).toBe("Codex");
  });
  it("비프리셋은 커스텀 접두 라벨로 만든다", () => {
    expect(labelFor("crush")).toBe("커스텀: crush");
  });
});

describe("badgeLabelFor", () => {
  it("프리셋 키를 짧은 배지 라벨로 매핑한다", () => {
    expect(badgeLabelFor("claude")).toBe("Claude");
    expect(badgeLabelFor("codex")).toBe("Codex");
    expect(badgeLabelFor("gemini")).toBe("Gemini");
    expect(badgeLabelFor("agy")).toBe("Agy");
  });
  it("제거된 프리셋이 남은 과거 행은 원문 그대로 — 마이그레이션하지 않는다", () => {
    expect(badgeLabelFor("opencode")).toBe("opencode");
  });
  it("커스텀 문자열은 원문 그대로 반환한다", () => {
    expect(badgeLabelFor("crush")).toBe("crush");
  });
  it("NULL/undefined/공백(구버전 행)은 null — 배지 생략", () => {
    expect(badgeLabelFor(null)).toBeNull();
    expect(badgeLabelFor(undefined)).toBeNull();
    expect(badgeLabelFor("  ")).toBeNull();
  });
  it("모든 프리셋이 비어있지 않은 badge를 갖는다 (백엔드 PRESETS 동기 시 누락 방지)", () => {
    for (const p of AGENT_PRESETS) {
      expect(p.badge.length).toBeGreaterThan(0);
      expect(badgeLabelFor(p.key)).toBe(p.badge);
    }
  });
});

describe("singleAgentSelection", () => {
  it("앙상블 시절 저장된 다중 선택은 첫 에이전트만 남긴다", () => {
    expect(singleAgentSelection(["codex", "claude"])).toEqual(["codex"]);
    expect(singleAgentSelection(["gemini", "codex"])).toEqual(["agy"]);
    expect(singleAgentSelection([])).toEqual(["claude"]);
  });
});
