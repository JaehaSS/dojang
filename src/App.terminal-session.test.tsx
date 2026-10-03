// @vitest-environment jsdom

/**
 * 세션 방식은 설정 하나가 정한다 — 터미널 작업의 화면에는 앱 컴포저를 두지 않아 입력창이
 * 둘로 보이지 않는다. 컴포저 표시는 설정이 아니라 각 작업에 기록된 mode를 따른다.
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
  const now = Math.floor(Date.now() / 1000);
  const task = (id: number, mode: string, ensemble = "") => ({
    id, repo: "/r", branch: `b${id}`, base: "dev", worktree_path: `/r/w${id}`, instruction: `작업 ${id}`,
    state: "Working", created_at: now, updated_at: now, agent: "claude", ensemble, mode,
  });
  // 2: 터미널(단일, xterm이 입력을 받는다) · 3: 앙상블 PTY(읽기 전용이라 컴포저가 유일한 입력).
  const TASKS = [task(2, "terminal"), task(3, "terminal", "claude,codex")];
  const empty: Record<string, unknown> = {};
  return {
    invoke: vi.fn(async (cmd: string) => {
      if (!(cmd in empty)) {
        empty[cmd] = cmd === "notification_source_page"
          ? { source_id: "test", after: null, cursor: 0, watermark: 0, results: [] }
          : cmd.startsWith("notification_")
            ? { items: [], sources: [], enabled: false, delivery_error: null }
            : cmd === "attention_snapshot" ? { source_id: "test", observed_at: 1, items: [] }
            : cmd === "usage_snapshot" ? { vendors: [] }
            : cmd === "task_list" ? TASKS
            : cmd.includes("diff_stat") ? ""
            // 작업 상세 패널이 여는 보고서들 — 없음(null)이 정상 응답이다.
            : cmd.includes("tool_cost") ? null : [];
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
  Terminal: class { cols = 80; rows = 24; options = {}; loadAddon() {} open() {} write() {} writeln() {} dispose() {} onData() { return { dispose() {} }; } focus() {} registerLinkProvider() { return { dispose() {} }; } onResize() { return { dispose() {} }; } },
}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} dispose() {} } }));
vi.mock("@xterm/addon-webgl", () => ({ WebglAddon: class { dispose() {} } }));

import App from "./App";
import { resetSessionStyleForTest, setSessionStyle } from "./lib/session-style";

let container: HTMLDivElement;
let root: Root;

const interactionSelect = (): HTMLSelectElement | null =>
  container.querySelector<HTMLSelectElement>('select[aria-label="세션 방식"]');
const agentComposer = () => container.querySelector('textarea[placeholder^="에이전트에게 질의"]');

async function openTask(id: number) {
  const row = container.querySelector<HTMLElement>(`[data-task-key="local:${id}"]`);
  expect(row, `작업 ${id} 행이 사이드바에 있어야 한다`).not.toBeNull();
  await act(async () => row!.dispatchEvent(new MouseEvent("click", { bubbles: true })));
  await act(async () => { await Promise.resolve(); });
}

beforeEach(async () => {
  resetSessionStyleForTest();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => root.render(<App />));
  await act(async () => { await Promise.resolve(); });
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  resetSessionStyleForTest();
  vi.clearAllMocks();
});

describe("세션 방식 — 설정 단위", () => {
  it("홈 컴포저에는 작업별 터미널 선택이 없고, 설정이 터미널이면 대화 선택 대신 표시만 둔다", async () => {
    expect([...interactionSelect()!.options].map((o) => o.value)).toEqual(["off", "questions", "approvals"]);
    await act(async () => setSessionStyle("terminal"));
    expect(interactionSelect()).toBeNull();
    expect(container.textContent).toContain("터미널 세션");
    await act(async () => setSessionStyle("conversation"));
    expect(interactionSelect()).not.toBeNull();
  });

  it("단일 터미널 작업에는 앱 컴포저가 없고, 읽기 전용 앙상블 PTY에는 있다", async () => {
    await openTask(2);
    expect(agentComposer()).toBeNull();
    await openTask(3);
    expect(agentComposer()).not.toBeNull();
  });

  it("컴포저 표시는 설정이 아니라 작업의 mode를 따른다", async () => {
    await act(async () => setSessionStyle("conversation"));
    await openTask(2);
    expect(agentComposer()).toBeNull();
    await act(async () => setSessionStyle("terminal"));
    await openTask(3);
    expect(agentComposer()).not.toBeNull();
  });
});
