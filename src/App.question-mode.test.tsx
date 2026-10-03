// @vitest-environment jsdom

/**
 * 질문 응답은 질문형 세션의 정식 기능이다 — "고급·실험 기능 표시"(설정 › 모양새)와 무관하게
 * 홈 화면 컴포저에 보이고, 토글을 껐다 켜도 사용자가 고른 값이 그대로 남아야 한다.
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

describe("질문 응답 선택 — 고급 토글과 무관", () => {
  it("기본값(고급 꺼짐)에서도 '(실험)' 없이 입력창 아래 옵션 줄에 보인다", () => {
    const select = interactionSelect();
    expect(select).not.toBeNull();
    expect(select!.value).toBe("off");
    expect(select!.textContent).toContain("질문 응답");
    expect(select!.textContent).not.toContain("실험");
    // 도구줄(저장소·에이전트·모델)과 갈라, 도구줄이 두 줄로 넘어가지 않게 한다.
    const toolbar = container.querySelector(".composer-toolbar");
    expect(toolbar!.contains(select)).toBe(false);
  });

  it("고급 토글을 켜고 꺼도 사용자가 고른 값이 유지된다", async () => {
    await act(async () => {
      const select = interactionSelect()!;
      select.value = "questions";
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    expect(interactionSelect()?.value).toBe("questions");

    await act(async () => setExperimentalFeaturesEnabled(true));
    await act(async () => setExperimentalFeaturesEnabled(false));
    expect(interactionSelect()?.value).toBe("questions");
  });
});
