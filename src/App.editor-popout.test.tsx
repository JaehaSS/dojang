// @vitest-environment jsdom

/**
 * 에디터 팝아웃 — 파일 탭을 보던 코드 열은 창과 함께 닫힌다. 남겨 두면 열이 첫 탭(작업정보)으로
 * 물러나, 떠 있던 채널이 부르지도 않은 옆 열로 옮겨 가고 대화 열 폭까지 바뀐다.
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
  const TASKS = [task(2, "conversation"), task(3, "conversation")];
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
            : cmd === "interaction_snapshot" ? { enabled: false, phase: "idle", items: [] }
            : cmd.includes("diff_stat") ? ""
            // 작업 상세 패널이 여는 보고서들 — 없음(null)이 정상 응답이다.
            : cmd.includes("tool_cost") ? null : [];
      }
      return empty[cmd];
    }),
  };
});
vi.mock("@tauri-apps/api/core", () => ({ invoke: tauri.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => undefined),
  emitTo: () => Promise.resolve(),
}));
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

let container: HTMLDivElement;
let root: Root;

const button = (label: string) => container.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
/** 좁은 창(탭 모드)에서는 닫힌 코드 열도 마운트된 채 inert로 가려진다 — 보이는지로 판정한다. */
const codeColumnShown = () => {
  const close = button("코드 열 닫기");
  return close != null && close.closest("[inert]") == null;
};
const click = async (label: string) => {
  const target = button(label);
  expect(target, `${label} 버튼이 있어야 한다`).not.toBeNull();
  await act(async () => target!.dispatchEvent(new MouseEvent("click", { bubbles: true })));
  // 팝아웃은 창을 연 IPC가 돌아온 뒤에야 상태를 바꾼다.
  await act(async () => { await new Promise((done) => setTimeout(done, 0)); });
};

async function openTask(id: number) {
  const row = container.querySelector<HTMLElement>(`[data-task-key="local:${id}"]`);
  expect(row, `작업 ${id} 행이 사이드바에 있어야 한다`).not.toBeNull();
  await act(async () => row!.dispatchEvent(new MouseEvent("click", { bubbles: true })));
  await act(async () => { await Promise.resolve(); });
}

beforeEach(async () => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => root.render(<App />));
  await act(async () => { await Promise.resolve(); });
  await openTask(2);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

describe("에디터 팝아웃과 코드 열", () => {
  it("파일 탭을 보던 코드 열은 팝아웃과 함께 닫힌다", async () => {
    await click("파일 보기");
    expect(codeColumnShown()).toBe(true);

    await click("에디터를 새 창으로");

    expect(tauri.invoke).toHaveBeenCalledWith("editor_window_open");
    expect(button("코드 창 앞으로")).not.toBeNull();
    expect(codeColumnShown()).toBe(false);
  });

  it("다른 탭을 보던 코드 열은 팝아웃해도 그대로 둔다", async () => {
    await click("Diff 보기");
    expect(codeColumnShown()).toBe(true);

    await click("에디터를 새 창으로");

    expect(codeColumnShown()).toBe(true);
  });

  it("나가 있는 동안 파일 탭을 열어 두었던 세션으로 옮기면 그 세션의 코드 열도 닫힌다", async () => {
    await openTask(3);
    await click("파일 보기");
    await openTask(2);
    await click("에디터를 새 창으로");

    await openTask(3);

    expect(codeColumnShown()).toBe(false);
  });

  it("나가 있는 동안 코드 열을 다시 열면 닫지 않는다", async () => {
    await click("파일 보기");
    await click("에디터를 새 창으로");
    expect(codeColumnShown()).toBe(false);

    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { code: "KeyS", metaKey: true, altKey: true }));
    });

    expect(codeColumnShown()).toBe(true);
  });
});
