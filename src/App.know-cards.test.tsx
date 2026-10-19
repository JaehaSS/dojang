// @vitest-environment jsdom

/**
 * 알아둘 것 카드가 App에 이어지는 자리 — 접힌 채널 핸들의 배지, `자세히`가 사이드 질문으로
 * 넘어가는 흐름, 코드 열로 옮겨 간 채널(설계 5절 A7·A8·A10).
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
  const task = (id: number, mode: string) => ({
    id, repo: "/r", branch: `b${id}`, base: "dev", worktree_path: `/r/w${id}`, instruction: `작업 ${id}`,
    state: "Working", created_at: now, updated_at: now, agent: "claude", ensemble: "", mode,
  });
  // 5: 대화 세션 — 카드가 붙는다 · 6: 터미널 세션 — 판정이 돌지 않으므로 조회도 없다.
  const TASKS = [task(5, "conversation"), task(6, "terminal")];
  const card = (id: number) => ({
    id, task_id: 5, kind: "heads_up", topic: `주제 ${id}`, title: `카드 ${id}`, body: `본문 ${id}`,
    evidence: `Edit src/f${id}.ts`, turn_label: "로그인 오류 고쳐줘", created_at: now, seen_at: null, asked_at: null,
  });
  // supported: 백엔드가 이 세션의 사이드 질문을 받는지 — 제공자·정책·작업 상태를 본 답이다.
  const state = { cards: [] as ReturnType<typeof card>[], supported: true };
  const handlers = new Map<string, (event: { payload: unknown }) => void>();
  const empty: Record<string, unknown> = {};
  return {
    card,
    state,
    handlers,
    invoke: vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "know_cards_list") return args?.taskId === 5 ? state.cards : [];
      if (cmd === "side_question_read") {
        const reason = state.supported ? null : "이 세션의 에이전트는 사이드 질문을 지원하지 않습니다.";
        return { task_id: 5, thread_id: 1, generation: 0, model: "haiku", supported: state.supported, reason, turns: [] };
      }
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
            : cmd.includes("tool_cost") ? null : [];
      }
      return empty[cmd];
    }),
  };
});
vi.mock("@tauri-apps/api/core", () => ({ invoke: tauri.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, handler: (event: { payload: unknown }) => void) => {
    tauri.handlers.set(event, handler);
    return Promise.resolve(() => undefined);
  },
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
let innerWidth: PropertyDescriptor | undefined;

const settle = async () => {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
};
async function openTask(id: number) {
  const row = container.querySelector<HTMLElement>(`[data-task-key="local:${id}"]`);
  expect(row, `작업 ${id} 행이 사이드바에 있어야 한다`).not.toBeNull();
  await act(async () => row!.dispatchEvent(new MouseEvent("click", { bubbles: true })));
  await settle();
}
const byLabel = (label: string) => container.querySelector<HTMLElement>(`[aria-label="${label}"]`);
const click = async (el: HTMLElement | null | undefined) => {
  expect(el).toBeTruthy();
  await act(async () => el!.dispatchEvent(new MouseEvent("click", { bubbles: true })));
  await settle();
};
const handle = () => container.querySelector<HTMLButtonElement>('button[aria-label^="작업정보 열기"]');
const sections = () => [...container.querySelectorAll<HTMLElement>('aside[aria-label="알아둘 것"]')];
const cardsChanged = async () => {
  await act(async () => tauri.handlers.get("know-cards://changed")?.({ payload: { taskId: 5 } }));
  await settle();
};
const calls = (cmd: string) => tauri.invoke.mock.calls.filter(([c]) => c === cmd).map(([, args]) => args);

beforeEach(async () => {
  // 채널이 뜰 만큼 넓은 창 — 중앙 폭의 첫 추정은 창 폭에서 시작하고, 관측은 이 환경에서 오지 않는다.
  innerWidth = Object.getOwnPropertyDescriptor(window, "innerWidth");
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 1600 });
  tauri.state.cards = [];
  tauri.state.supported = true;
  tauri.handlers.clear();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => root.render(<App />));
  await settle();
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  if (innerWidth) Object.defineProperty(window, "innerWidth", innerWidth);
  vi.clearAllMocks();
});

describe("알아둘 것 카드 — App", () => {
  it("떠 있는 채널 맨 아래에 카드가 붙고, 보인 카드는 읽음으로 기록한다", async () => {
    tauri.state.cards = [tauri.card(2), tauri.card(1)];
    await openTask(5);

    expect(sections()).toHaveLength(1);
    expect(sections()[0].textContent).toContain("카드 2");
    expect(calls("know_cards_list")).toContainEqual({ taskId: 5 });
    expect(calls("know_card_seen")).toEqual([{ cardId: 2 }]);
  });

  it("접힌 동안 온 카드는 핸들의 숫자로만 알린다", async () => {
    await openTask(5);
    await click(byLabel("작업정보 접기"));
    expect(handle()!.getAttribute("aria-label")).toBe("작업정보 열기");
    expect(handle()!.textContent).toBe("");

    tauri.state.cards = [tauri.card(2), tauri.card(1)];
    await cardsChanged();
    expect(handle()!.getAttribute("aria-label")).toBe("작업정보 열기 · 안 읽은 알아둘 것 2장");
    expect(handle()!.textContent).toBe("2");
    // 접힌 채널의 카드는 아무도 보지 않았다.
    expect(calls("know_card_seen")).toEqual([]);

    // 다시 열면 1페이지의 카드만 읽힌다 — 넘겨 보지 않은 카드는 다음에 접었을 때 여전히 센다.
    await click(handle());
    expect(calls("know_card_seen")).toEqual([{ cardId: 2 }]);
    await click(byLabel("작업정보 접기"));
    expect(handle()!.getAttribute("aria-label")).toBe("작업정보 열기 · 안 읽은 알아둘 것 1장");
  });

  it("자세히는 카드를 사이드 질문의 자료로 넘기고 asked를 기록한다 — 카드는 남는다", async () => {
    tauri.state.cards = [tauri.card(2), tauri.card(1)];
    await openTask(5);

    const ask = [...sections()[0].querySelectorAll("button")].find((b) => b.textContent === "자세히");
    await click(ask);

    expect(calls("know_card_act")).toEqual([{ cardId: 2, action: "asked" }]);
    const material = container.querySelector('[role="list"][aria-label="질의 자료"] [role="listitem"]');
    expect(material?.textContent).toContain("카드 2");
    expect(material?.getAttribute("title")).toBe("카드 2\n\n본문 2\n\n근거: Edit src/f2.ts");

    // 코드 열이 열려 채널은 그 안의 탭으로 옮겨 갔다 — 떠 있는 카드와 탭의 카드가 둘 다 있으면 안 된다.
    expect(sections()).toHaveLength(1);
    expect(sections()[0].textContent).toContain("카드 2");
  });

  it("백엔드가 사이드 질문을 받지 않는 세션에는 자세히가 없다 — 눌러도 질문은 못 하고 asked만 남는다", async () => {
    tauri.state.supported = false;
    tauri.state.cards = [tauri.card(2)];
    await openTask(5);

    expect(calls("side_question_read")).not.toEqual([]);
    const labels = [...sections()[0].querySelectorAll("button")].map((b) => b.textContent);
    expect(labels).toContain("이미 앎");
    expect(labels).not.toContain("자세히");
  });

  it("터미널 세션은 카드를 조회하지 않는다", async () => {
    await openTask(6);
    expect(calls("know_cards_list")).toEqual([]);
    expect(sections()).toHaveLength(0);
  });
});
