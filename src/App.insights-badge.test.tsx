// @vitest-environment jsdom

/**
 * 인사이트 사이드바 점 — 아직 보지 않은 발견 카드 수(설계 2026-09-28 §4, 슬라이스 T5).
 *
 * `insightCards("all", tz)`는 앱 시작 때, 그리고 인사이트 화면을 나갈 때 다시 불린다
 * (App.tsx `refreshInsightsUnread`). 인사이트 화면을 열어 카드를 보면 `markSeen`이
 * (signal, repo) 쌍을 localStorage에 남기고, 그 뒤 재조회에서는 같은 쌍이 더 이상
 * "보지 않은" 것으로 세어지지 않는다.
 */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
vi.stubGlobal("matchMedia", (query: string) => ({
  matches: false, media: query, onchange: null,
  addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {},
  dispatchEvent: () => false,
}));
vi.stubGlobal("IntersectionObserver", class { observe() {} unobserve() {} disconnect() {} });

const SIGNAL_CARDS = vi.hoisted(() => [
  {
    key: "S2:7d:2026-09-21",
    signal: "S2",
    sentence: "이 구간 작업의 70%가 한 저장소에 몰렸다",
    value: 7,
    baseline: 3,
    sample: 10,
    evidence: { kind: "Tasks", ids: [1, 2, 3] },
    repo: "/repo-a",
  },
  {
    key: "L1:7d:2026-09-21",
    signal: "L1",
    sentence: "runner/db 주제 교훈이 이 구간에 5건 늘었다",
    value: 5,
    baseline: 2,
    sample: 5,
    evidence: { kind: "Ledger", repo: "/repo-b", numbers: [1, 2] },
    repo: "/repo-b",
  },
]);

// 로컬 Tauri command는 대부분 "아무것도 없다"로 답한다 — insight_cards만 위 고정 카드를 준다.
const tauri = vi.hoisted(() => {
  const empty: Record<string, unknown> = {};
  return {
    invoke: vi.fn(async (cmd: string) => {
      if (cmd === "insight_cards") return SIGNAL_CARDS;
      // 사용량·작업 패턴 집계는 "아직 없음"이 빈 배열이 아니라 null이다 — 각 패널이
      // null을 "집계 중…" 상태로 다루므로, 빈 배열을 주면 `.models`·`.funnel` 접근이 깨진다.
      if (cmd === "insights_compute" || cmd === "task_patterns") return null;
      if (!(cmd in empty)) {
        empty[cmd] = cmd === "notification_source_page"
          ? { source_id: "test", after: null, cursor: 0, watermark: 0, results: [] }
          : cmd.startsWith("notification_")
            ? { items: [], sources: [], enabled: false, delivery_error: null }
            : cmd === "usage_snapshot" ? { vendors: [] } : [];
      }
      return empty[cmd];
    }),
  };
});
vi.mock("@tauri-apps/api/core", () => ({ invoke: tauri.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => undefined) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn(), openPath: vi.fn() }));
vi.mock("./lib/monaco", () => ({ langFromPath: () => "typescript" }));
vi.mock("@monaco-editor/react", async () => {
  const React = await import("react");
  return { default: () => React.createElement("div", { "data-testid": "monaco" }) };
});
vi.mock("@xterm/xterm", () => ({
  Terminal: class { cols = 80; rows = 24; options = {}; loadAddon() {} open() {} write() {} writeln() {} dispose() {} onData() { return { dispose() {} }; } focus() {} onResize() { return { dispose() {} }; } },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} dispose() {} } }));
vi.mock("@xterm/addon-webgl", () => ({ WebglAddon: class { dispose() {} } }));

import App from "./App";
import { ErrorBoundary } from "./components/ErrorBoundary";

let container: HTMLDivElement;
let root: Root;

const dot = () => container.querySelector<HTMLElement>('[aria-label^="읽지 않은 발견 카드"]');
const insightsLink = () =>
  [...container.querySelectorAll("button")].find((b) => b.textContent?.includes("인사이트"));
const newTaskButton = () =>
  [...container.querySelectorAll("button")].find((b) => b.textContent?.includes("새 작업"));

beforeEach(async () => {
  localStorage.clear();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => root.render(<ErrorBoundary><App /></ErrorBoundary>));
  await act(async () => { await Promise.resolve(); });
  await act(async () => { await Promise.resolve(); });
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

describe("인사이트 사이드바 점", () => {
  it("앱 시작 시 보지 않은 카드 수만큼 점을 켠다", () => {
    expect(dot()?.getAttribute("aria-label")).toBe("읽지 않은 발견 카드 2건");
  });

  it("인사이트 화면을 열어 카드를 본 뒤 나가면 점이 꺼진다", async () => {
    const link = insightsLink();
    expect(link, "사이드바에 인사이트 링크가 있어야 한다").toBeTruthy();
    await act(async () => { link?.click(); });
    await act(async () => { await Promise.resolve(); });
    await act(async () => { await Promise.resolve(); });

    expect(container.textContent).not.toContain("화면을 그리지 못했습니다");
    // 인사이트 화면이 열려 있는 동안 markSeen이 이미 두 쌍을 기록했다.
    expect(localStorage.getItem("praxis:insight-seen")).toContain("S2");

    const home = newTaskButton();
    expect(home, "나가는 데 쓸 새 작업 버튼이 있어야 한다").toBeTruthy();
    await act(async () => { home?.click(); });
    await act(async () => { await Promise.resolve(); });
    await act(async () => { await Promise.resolve(); });

    expect(container.textContent).not.toContain("화면을 그리지 못했습니다");
    expect(dot()).toBeNull();
  });
});
