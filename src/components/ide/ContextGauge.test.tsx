// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Task } from "../../lib/ipc";
import { ContextGauge, gaugeTone } from "./ContextGauge";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const task: Task = {
  id: 42,
  host: "local",
  repo: "/workspace/praxis",
  branch: "feature/JH2-75-session-header-ia-consolidation",
  base: "main",
  worktree_path: "/workspace/praxis/.praxis/worktrees/task-42",
  instruction: "컨텍스트 잔량을 컴포저 곁에서 보여준다",
  state: "Running",
  created_at: 1,
  updated_at: 2,
  agent: "codex",
  mode: "conversation",
};

const observation = (contextTokens: number, over: Record<string, unknown> = {}) => ({
  contextTokens,
  contextWindow: 258_400,
  observedAt: 1_725_000_000,
  source: "codex_session",
  ...over,
});

const renderGauge = (contextTokens: number | null = null, busy = false) =>
  renderToStaticMarkup(
    <ContextGauge
      task={task}
      observation={contextTokens == null ? null : observation(contextTokens)}
      busy={busy}
      onOpenDetails={() => {}}
    />,
  );

/** claude 세션 — 200K 표준과 `[1m]` 1M 윈도가 갈리는 유일한 벤더라 별도 픽스처를 쓴다. */
const renderClaude = (contextTokens: number, model?: string | null) =>
  renderToStaticMarkup(
    <ContextGauge
      task={{ ...task, agent: "claude" }}
      observation={observation(contextTokens, { contextWindow: null, source: "claude_message" })}
      model={model}
      onOpenDetails={() => {}}
    />,
  );

describe("gaugeTone (잔량 임계)", () => {
  it("여유가 넉넉하면 조용한 톤", () => {
    expect(gaugeTone(93)).toBe("text-text-muted");
  });

  it("50% 미만이면 주의 톤", () => {
    expect(gaugeTone(49)).toBe("text-status-awaiting");
  });

  it("20% 미만이면 경고 톤", () => {
    expect(gaugeTone(19)).toBe("text-status-failed");
  });

  // 경계는 "미만"이라 임계값 자체는 아직 이전 단계다 — 부등호 방향이 뒤집히면 여기서 잡힌다.
  it("경계값 50은 아직 조용하다", () => {
    expect(gaugeTone(50)).toBe("text-text-muted");
  });

  it("경계값 20은 아직 주의다", () => {
    expect(gaugeTone(20)).toBe("text-status-awaiting");
  });
});

describe("ContextGauge", () => {
  it("관측값이 없으면 확인 불가를 표시한다", () => {
    expect(renderGauge(null)).toContain("컨텍스트 확인 불가");
  });

  it("사용률이 아니라 잔량을 표시한다", () => {
    // 실제 Codex 윈도 258,400, 129,200 토큰 = 50% 사용 → 50% 남음.
    const html = renderGauge(129_200);
    expect(html).toContain("컨텍스트 약 50% 남음");
    expect(html).not.toContain("CTX");
  });

  it("잔량이 적을수록 강한 톤을 입힌다", () => {
    expect(renderGauge(258_400 * 0.75)).toContain("text-status-awaiting"); // 25% 남음
    expect(renderGauge(258_400 * 0.95)).toContain("text-status-failed"); // 5% 남음
  });

  it("`[1m]` 모델은 1M 윈도 기준으로 계산한다", () => {
    const html = renderClaude(300_000, "opus[1m]");
    expect(html).toContain("70% 남음");
    expect(html).toContain("1,000,000 토큰"); // 툴팁 분모도 같은 윈도를 쓴다
  });

  it("모델 표기가 없으면 Claude 관측으로 티어를 승격하지 않는다", () => {
    expect(renderClaude(200_001)).toContain("컨텍스트 약 0% 남음");
  });

  it("실행 중에는 마지막 관측임을 밝히고 시각과 토큰을 툴팁에 보인다", () => {
    const html = renderGauge(147_043, true);
    expect(html).toContain("이전 관측");
    expect(html).toContain("2024-08-30T06:40:00.000Z");
    expect(html).toContain("147,043/258,400 토큰");
  });

  describe("클릭 상호작용", () => {
    let container: HTMLDivElement | null = null;
    let root: Root | null = null;

    beforeEach(() => {
      container = document.createElement("div");
      document.body.appendChild(container);
      root = createRoot(container);
    });

    afterEach(async () => {
      await act(async () => root?.unmount());
      container?.remove();
      container = null;
      root = null;
    });

    it("클릭하면 채널 진입 콜백을 호출하고 팝업을 만들지 않는다", async () => {
      const onOpenDetails = vi.fn();
      await act(async () => root?.render(<ContextGauge task={task} observation={observation(129_200)} onOpenDetails={onOpenDetails} />));
      await act(async () => container?.querySelector("button")?.click());
      expect(onOpenDetails).toHaveBeenCalledOnce();
      expect(document.querySelector('[role="dialog"]')).toBeNull();
      expect(container?.querySelector("button")?.getAttribute("aria-label")).toContain("컨텍스트 구성 보기");
    });

    it("관측이 없어도 채널에 진입할 수 있다", async () => {
      const onOpenDetails = vi.fn();
      await act(async () => root?.render(<ContextGauge task={task} observation={null} onOpenDetails={onOpenDetails} />));
      await act(async () => container?.querySelector("button")?.click());
      expect(onOpenDetails).toHaveBeenCalledOnce();
    });
  });
});
