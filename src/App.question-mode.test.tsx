// @vitest-environment jsdom

/**
 * 홈 컴포저의 세션 방식 선택은 제거한다. 고급 기능 토글과 무관하게 이어받기는 유지한다.
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

const tauri = vi.hoisted(() => {
  const empty: Record<string, unknown> = {};
  return {
    invoke: vi.fn(async (cmd: string) => {
      if (!(cmd in empty)) {
        empty[cmd] = cmd === "notification_source_page"
          ? { source_id: "test", after: null, cursor: 0, watermark: 0, results: [] }
          : cmd.startsWith("notification_")
            ? { items: [], sources: [], enabled: false, delivery_error: null }
            : cmd === "attention_snapshot" ? { source_id: "test", observed_at: 1, items: [] }
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
import {
  setExperimentalFeaturesEnabled,
  resetExperimentalFeaturesForTest,
} from "./lib/experimental-features";

let container: HTMLDivElement;
let root: Root;

const interactionSelect = (): HTMLSelectElement | null =>
  container.querySelector<HTMLSelectElement>('select[aria-label="세션 방식"]');

beforeEach(async () => {
  resetExperimentalFeaturesForTest(false);
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => root.render(<App />));
  await act(async () => { await Promise.resolve(); });
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  resetExperimentalFeaturesForTest(false);
  vi.clearAllMocks();
});

describe("홈 컴포저의 세션 방식 선택 제거", () => {
  it("세션 방식 선택 없이 상단 도구줄에 세션 이어받기를 둔다", () => {
    expect(interactionSelect()).toBeNull();
    expect(container.textContent).not.toContain("질문 응답");
    const resume = [...container.querySelectorAll("button")].find((button) => button.textContent?.trim() === "세션 이어받기");
    expect(resume).toBeDefined();
    // 질문·승인 선택이 사라진 뒤 이어받기는 입력창 아래가 아니라 에이전트 선택 옆(상단 도구줄)에 있다.
    const toolbar = container.querySelector(".composer-toolbar")!;
    expect(toolbar.contains(resume!)).toBe(true);
    expect(container.querySelectorAll(".composer-toolbar").length).toBe(1);
  });

  it("고급 토글을 켜고 꺼도 세션 방식 선택이 나타나지 않는다", async () => {
    await act(async () => setExperimentalFeaturesEnabled(true));
    expect(interactionSelect()).toBeNull();
    await act(async () => setExperimentalFeaturesEnabled(false));
    expect(interactionSelect()).toBeNull();
    expect(container.textContent).toContain("세션 이어받기");
  });
});
