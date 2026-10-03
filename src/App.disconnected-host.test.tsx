// @vitest-environment jsdom

/**
 * 끊긴 원격 호스트의 세션을 고르면 창이 통째로 빈다 — 그 회귀를 막는 테스트다.
 *
 * `taskListAll`은 응답 없는 원격을 레지스트리에서 내리고(`unregisterTransport`) 캐시에 남은
 * 행을 `stale: true`로 되돌려 준다. 그 행을 고르면 `refreshDebateSide`가 `getTransport`를
 * 부르는데, 내려간 호스트라 그 자리에서 **throw**한다. 이 콜백은 effect라 예외가 커밋 단계로
 * 올라가고 루트 ErrorBoundary까지 가서, 세션을 클릭한 것뿐인데 화면 전체가 폴백으로 바뀐다.
 * 그래서 여기서 보는 것은 특정 위젯이 아니라 **아무 오류도 루트에 닿지 않는다**는 사실이다.
 */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// 캐시는 모듈 로드 시점에 localStorage를 한 번 읽는다(`new TaskListCache()`) — import보다
// 먼저 심어야 한다. vi.hoisted가 그 유일한 자리다.
const DEAD_HOST = vi.hoisted(() => {
  const host = "dead-remote";
  const now = Math.floor(Date.now() / 1000);
  localStorage.setItem(
    "praxis-task-list-cache-v1",
    JSON.stringify({
      [host]: [
        { id: 7, repo: "/srv/acme", title: "원격 리팩터", state: "Working", updated_at: now },
        { id: 8, repo: "/srv/acme", title: "원격 점검", state: "AwaitingReview", updated_at: now },
      ],
    }),
  );
  return host;
});

// jsdom에 없는 브라우저 API — App 아래 레이아웃·테마 코드가 마운트에서 곧바로 만진다.
vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
vi.stubGlobal("matchMedia", (query: string) => ({
  matches: false, media: query, onchange: null,
  addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {},
  dispatchEvent: () => false,
}));

// 로컬 Tauri command는 모두 "아무것도 없다"로 답한다 — 이 테스트가 보는 것은 원격 호스트
// 하나뿐이고, 나머지 화면은 마운트만 되면 된다. 명령마다 **같은 인스턴스**를 돌려주는 것이
// 중요하다: 매번 새 객체를 주면 그것을 의존성으로 삼은 effect가 서로를 깨워 루프가 된다.
const tauri = vi.hoisted(() => {
  const empty: Record<string, unknown> = {};
  return {
    invoke: vi.fn(async (cmd: string) => {
      if (!(cmd in empty)) {
        // 알림 수집기는 커서를 따라 페이지를 이어 읽는다 — 모양이 어긋난 응답을 주면
        // 그 while 루프가 끝나지 않는다. 이미 다 읽은 원천 하나를 흉내낸다.
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
// monaco·xterm 본체는 import만으로 브라우저 API를 만진다 — jsdom에서 로드되지 않는다.
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
import { registerTransport, unregisterTransport, type PraxisTransport } from "./lib/transport";

let container: HTMLDivElement;
let root: Root;
let rootErrors: unknown[];

const row = (key: string) => container.querySelector<HTMLElement>(`[data-task-key="${key}"]`);

beforeEach(async () => {
  rootErrors = [];
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container, {
    onCaughtError: (error) => rootErrors.push(error),
    onUncaughtError: (error) => rootErrors.push(error),
  });
  // 응답하지 않는 원격 — 첫 목록 조회에서 거절해 레지스트리에서 내려간다.
  registerTransport({
    kind: "remote",
    hostId: DEAD_HOST,
    taskList: async () => { throw new Error("연결이 끊겼습니다"); },
  } as unknown as PraxisTransport);
  await act(async () => root.render(<ErrorBoundary><App /></ErrorBoundary>));
  await act(async () => { await Promise.resolve(); });
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  unregisterTransport(DEAD_HOST);
  vi.clearAllMocks();
});

describe("끊긴 호스트의 세션 선택", () => {
  it("캐시에 남은 원격 행을 골라도 루트로 오류가 올라가지 않는다", async () => {
    const target = row(`${DEAD_HOST}:7`);
    expect(target, "끊긴 호스트의 캐시 행이 사이드바에 있어야 한다").not.toBeNull();

    await act(async () => target?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    await act(async () => { await Promise.resolve(); });

    expect(rootErrors.map(String)).toEqual([]);
    expect(container.textContent).not.toContain("화면을 그리지 못했습니다");
    // 선택이 실제로 먹혔는지 — 창이 살아 있는 채로 "끊겼다"고 말해야 한다.
    expect(container.querySelector('[role="status"]')?.textContent)
      .toContain(`${DEAD_HOST} 연결이 끊겼습니다`);
  });
});
