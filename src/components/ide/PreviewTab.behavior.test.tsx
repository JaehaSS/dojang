// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  close: vi.fn(async () => undefined),
  hide: vi.fn(async () => undefined),
  captureEditor: vi.fn(),
  listen: vi.fn(
    async (_event: string, _handler: (e: { payload: unknown }) => void) => () => undefined,
  ),
  navigate: vi.fn(async () => undefined),
  open: vi.fn(async () => undefined),
  setBounds: vi.fn(async () => undefined),
  setSelectionMode: vi.fn(async () => undefined),
  show: vi.fn(async () => true),
  state: vi.fn(),
  readEditorTarget: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("../../lib/ipc", () => ({
  designmodeCaptureEditor: mocks.captureEditor,
  designmodeClose: mocks.close,
  designmodeHide: mocks.hide,
  designmodeNavigate: mocks.navigate,
  designmodeOpen: mocks.open,
  designmodeSetBounds: mocks.setBounds,
  designmodeSetSelectionMode: mocks.setSelectionMode,
  designmodeShow: mocks.show,
  designmodeState: mocks.state,
}));
vi.mock("../../lib/designmode/editor-capture-target", () => ({
  readEditorCaptureTarget: mocks.readEditorTarget,
}));

import { PreviewTab, formatLastAction } from "./PreviewTab";
import { clearCaptures, getCaptures } from "../../lib/designmode/store";
import { enqueueDesignTransition, waitForDesignTransition } from "../../lib/design-canvas/lifecycle";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

class TestResizeObserver implements ResizeObserver {
  static latest: TestResizeObserver | null = null;

  constructor(private readonly callback: ResizeObserverCallback) {
    TestResizeObserver.latest = this;
  }

  disconnect(): void {}
  observe(): void {}
  unobserve(): void {}

  notify(): void {
    this.callback([], this);
  }
}

let container: HTMLDivElement | null = null;
let root: Root | null = null;
let rectWidth = 800;
let rectX = 12;

// 컴포넌트가 마지막으로 고른 모드를 task별 모듈 Map에 기억하므로, 테스트마다 새 id를 써야
// 앞 테스트의 인라인 전환이 다음 테스트로 새지 않는다.
let nextTaskId = 7;
let taskId = nextTaskId;

async function render(
  active: boolean,
  overlayOpen = false,
): Promise<void> {
  await act(async () => {
    root?.render(
      <PreviewTab
        taskId={taskId}
        active={active}
        overlayOpen={overlayOpen}
        editorAvailable
      />,
    );
    await Promise.resolve();
  });
}

/** 기본은 창 모드다. 사이드패널 안 웹뷰 수명주기를 검증하려면 사용자와 같은 경로로 전환하고,
 *  전환이 기억된 상태에서 다시 마운트한다 — 아래 테스트들은 "인라인으로 마운트"를 전제한다. */
async function switchToInline(): Promise<void> {
  await render(true);
  const toggle = container?.querySelector<HTMLButtonElement>('button[aria-label="별도 창"]');
  await act(async () => toggle?.click());
  await act(async () => root?.unmount());
  root = createRoot(container!);
  vi.clearAllMocks();
  mocks.show.mockResolvedValue(true);
  mocks.state.mockResolvedValue({ taskId, url: "http://localhost:3000", mode: "inline", generation: 1 });
}

/** `listen`에 등록된 특정 이벤트의 핸들러를 꺼낸다 — 캡처와 창 닫힘 둘을 구독한다. */
function handlerFor(event: string): ((e: { payload: unknown }) => void) | undefined {
  const call = mocks.listen.mock.calls.find((c) => c[0] === event);
  return call?.[1];
}

beforeEach(() => {
  vi.useFakeTimers();
  taskId = ++nextTaskId;
  rectWidth = 800;
  rectX = 12;
  TestResizeObserver.latest = null;
  mocks.show.mockResolvedValue(true);
  mocks.state.mockResolvedValue({ taskId, url: "http://localhost:3000", mode: "window", generation: 1 });
  mocks.readEditorTarget.mockReturnValue({
    bounds: { x: 20, y: 30, width: 500, height: 400 },
    file_path: "src/App.tsx",
    selection_text: "selected",
    selection_start_line: 10,
    selection_end_line: 12,
  });
  mocks.captureEditor.mockResolvedValue({
    id: "1-0",
    task_id: taskId,
    source: "editor",
    outer_html: "",
    computed_css: {},
    bounding_rect: { x: 20, y: 30, width: 500, height: 400 },
    captured_at: 1,
    image_path: "/tmp/editor.png",
    file_path: "src/App.tsx",
    selection_text: "selected",
    selection_start_line: 10,
    selection_end_line: 12,
  });
  clearCaptures(taskId);
  vi.stubGlobal("ResizeObserver", TestResizeObserver);
  // 모드 전환은 페이지 상태를 잃으므로 확인을 받는다. jsdom의 confirm은 falsy라 stub이 없으면
  // 전환이 조용히 취소된다.
  vi.stubGlobal("confirm", () => true);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    get x() {
      return rectX;
    },
    y: 40,
    get width() {
      return rectWidth;
    },
    height: 600,
    top: 40,
    get right() {
      return rectX + rectWidth;
    },
    bottom: 640,
    get left() {
      return rectX;
    },
    toJSON: () => ({}),
  });
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root?.unmount());
  container?.remove();
  root = null;
  container = null;
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  vi.clearAllMocks();
  vi.useRealTimers();
});

describe("PreviewTab lifecycle", () => {
  it("finishes a delayed Preview hide before Design can open the reused surface", async () => {
    await switchToInline();
    await render(true);
    await waitForDesignTransition(taskId);
    let release!: () => void;
    const hidden = new Promise<undefined>((resolve) => { release = () => resolve(undefined); });
    mocks.hide.mockImplementationOnce(() => hidden);
    await act(async () => root?.unmount());
    root = null;
    const openDesign = vi.fn(async () => {});
    const transition = enqueueDesignTransition(taskId, openDesign);
    try {
      await Promise.resolve();
      expect(openDesign).not.toHaveBeenCalled();
      release();
      await transition;
      expect(openDesign).toHaveBeenCalledOnce();
    } finally { release(); await transition; }
  });

  it("cancels a queued Enter when Preview becomes inactive during Design cleanup", async () => {
    let release!: () => void;
    const cleanup = enqueueDesignTransition(taskId, () => new Promise<void>((resolve) => { release = resolve; }));
    await render(true);
    const input = container!.querySelector<HTMLInputElement>('[aria-label="프리뷰 URL"]')!;
    await act(async () => { input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    await render(false);
    try {
      await act(async () => { release(); await cleanup; });
      await waitForDesignTransition(taskId);
      expect(mocks.open).not.toHaveBeenCalled();
      expect(mocks.navigate).not.toHaveBeenCalled();
    } finally { release(); await cleanup; }
  });

  it("waits for Design cleanup before a fast Enter can open the shared native surface", async () => {
    let release!: () => void;
    const cleanup = enqueueDesignTransition(taskId, () => new Promise<void>((resolve) => { release = resolve; }));
    await Promise.resolve();
    try {
      mocks.state.mockResolvedValue(null);
      await render(true);
      const input = container!.querySelector<HTMLInputElement>('[aria-label="프리뷰 URL"]')!;
      await act(async () => { input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
      expect(mocks.open).not.toHaveBeenCalled();
      expect(mocks.state).not.toHaveBeenCalled();
      await act(async () => { release(); await cleanup; });
      expect(mocks.open).toHaveBeenCalledOnce();
    } finally {
      release();
      await cleanup;
    }
  });
  it("shows the persistent webview on tab activation without navigating again", async () => {
    await switchToInline();
    await render(true);
    expect(mocks.show).toHaveBeenCalledOnce();
    expect(mocks.show).toHaveBeenCalledWith(taskId, {
      x: 12,
      y: 40,
      width: 800,
      height: 600,
    });

    await render(false);
    expect(mocks.hide).toHaveBeenCalled();

    await render(true);
    expect(mocks.show).toHaveBeenCalledTimes(2);
    expect(mocks.open).not.toHaveBeenCalled();
    expect(mocks.navigate).not.toHaveBeenCalled();
    expect(container?.querySelector('button[aria-label="새로고침"]')).not.toBeNull();
  });

  it("hides the native webview while a side panel overlay is open, keeping the URL bar", async () => {
    await switchToInline();
    await render(true);
    mocks.hide.mockClear();

    // 네이티브 웹뷰는 DOM 위에 그려지므로 도구 피커가 열린 동안은 숨겨야 메뉴가 잘리지 않는다.
    await render(true, true);
    expect(mocks.hide).toHaveBeenCalledWith(taskId);
    expect(container?.querySelector('input[aria-label="프리뷰 URL"]')).not.toBeNull();

    await render(true, false);
    expect(mocks.show).toHaveBeenCalledTimes(2);
  });

  it("stops syncing bounds while an overlay hides the webview", async () => {
    await switchToInline();
    await render(true);
    await render(true, true);
    mocks.setBounds.mockClear();

    rectWidth = 900;
    window.dispatchEvent(new Event("resize"));
    await act(async () => vi.advanceTimersByTime(16));

    expect(mocks.setBounds).not.toHaveBeenCalled();
  });

  it("coalesces resize bursts and skips unchanged native bounds", async () => {
    await switchToInline();
    await render(true);
    TestResizeObserver.latest?.notify();
    TestResizeObserver.latest?.notify();
    await act(async () => vi.advanceTimersByTime(16));
    expect(mocks.setBounds).not.toHaveBeenCalled();

    rectWidth = 801;
    TestResizeObserver.latest?.notify();
    TestResizeObserver.latest?.notify();
    await act(async () => vi.advanceTimersByTime(16));
    expect(mocks.setBounds).toHaveBeenCalledOnce();
    expect(mocks.setBounds).toHaveBeenCalledWith(taskId, {
      x: 12,
      y: 40,
      width: 801,
      height: 600,
    });
  });

  it("syncs bounds on window resize even when container size is unchanged (fixed-width side panel shift)", async () => {
    await switchToInline();
    await render(true);
    mocks.setBounds.mockClear();

    // 사이드패널은 고정 폭이라 메인 창 리사이즈로 위치만 이동해도 크기는 그대로 -> ResizeObserver 미발화.
    rectX = 60;
    window.dispatchEvent(new Event("resize"));
    await act(async () => vi.advanceTimersByTime(16));

    expect(mocks.setBounds).toHaveBeenCalledOnce();
    expect(mocks.setBounds).toHaveBeenCalledWith(taskId, {
      x: 60,
      y: 40,
      width: 800,
      height: 600,
    });
  });

  it("keeps the separate window alive when the tab goes away", async () => {
    // 창 모드는 메인 창과 독립이다 — 탭을 떠났다고 창을 숨기면 "따로 띄워 크게 본다"가 무너진다.
    await render(true);
    expect(mocks.hide).not.toHaveBeenCalled();

    await render(false);
    await render(true, true); // 도구 피커 오버레이도 창을 건드리지 않는다.

    expect(mocks.hide).not.toHaveBeenCalled();
    expect(mocks.setBounds).not.toHaveBeenCalled();
  });

  it("returns to the empty state when the user closes the preview window", async () => {
    await render(true);
    expect(container?.textContent).toContain("프리뷰 창이 열려 있습니다");

    await act(async () => handlerFor("designmode://closed")?.({ payload: taskId }));

    expect(container?.textContent).toContain("dev 서버 URL을 입력하고 Enter");
  });

  it("ignores a close event for another task", async () => {
    await render(true);
    await act(async () => handlerFor("designmode://closed")?.({ payload: taskId + 1 }));
    expect(container?.textContent).toContain("프리뷰 창이 열려 있습니다");
  });

  it("captures the visible editor and adds it to the task composer attachments", async () => {
    await render(true);
    const button = container?.querySelector<HTMLButtonElement>('button[aria-label="에디터 캡처"]');
    expect(button?.disabled).toBe(false);

    await act(async () => button?.click());

    expect(mocks.captureEditor).toHaveBeenCalledWith(taskId, expect.objectContaining({
      file_path: "src/App.tsx",
      selection_start_line: 10,
    }));
    expect(getCaptures(taskId)).toEqual([
      expect.objectContaining({ source: "editor", image_path: "/tmp/editor.png" }),
    ]);
  });

  it("keeps the tab available when the preview opens as a separate window", async () => {
    mocks.show.mockResolvedValue(false);
    mocks.state.mockResolvedValue(null);
    await render(true);

    const refresh = container?.querySelector<HTMLButtonElement>('button[aria-label="새로고침"]');
    await act(async () => refresh?.click());

    expect(mocks.open).toHaveBeenCalledWith(
      taskId,
      "http://localhost:3000",
      { x: 12, y: 40, width: 800, height: 600 },
      "window",
    );
    expect(container?.textContent).toContain("프리뷰 창이 열려 있습니다");
    await act(async () => refresh?.click());
    expect(mocks.navigate).toHaveBeenCalledOnce();
  });

  it("opens inline previews without changing tab ownership", async () => {
    await switchToInline();
    mocks.show.mockResolvedValue(false);
    mocks.state.mockResolvedValue(null);
    await render(true);

    const refresh = container?.querySelector<HTMLButtonElement>('button[aria-label="새로고침"]');
    await act(async () => refresh?.click());

    expect(mocks.open).toHaveBeenCalledWith(
      taskId,
      "http://localhost:3000",
      expect.anything(),
      "inline",
    );
  });

  it("clears a leftover preview before switching modes, even when the tab thinks nothing is open", async () => {
    // 회귀: 창 모드를 떠날 때 웹뷰만 닫혀 빈 "Praxis Preview" 창이 남았다. 프론트가 "열려 있지
    // 않다"고 믿는 동안에도 네이티브에는 그 유령이 있고, 그것이 조작면 위에 눌러앉으면
    // URL 입력줄도 선택 버튼도 누를 수 없게 된다 — 전환은 loaded와 무관하게 정리부터 한다.
    mocks.show.mockResolvedValue(false);
    mocks.state.mockResolvedValue(null);
    await render(true);
    expect(container?.textContent).toContain("dev 서버 URL을 입력하고 Enter");

    const toggle = container?.querySelector<HTMLButtonElement>('button[aria-label="별도 창"]');
    await act(async () => toggle?.click());

    expect(mocks.close).toHaveBeenCalledWith(taskId);
    expect(toggle?.getAttribute("aria-pressed")).toBe("false");
  });

  it("keeps the current mode when the leftover preview cannot be closed", async () => {
    // 못 없앴는데 새로 열면 유령과 새 프리뷰가 겹친다 — 실패는 그 자리에 드러낸다.
    await render(true);
    mocks.close.mockRejectedValueOnce("프리뷰를 닫지 못했습니다");

    const toggle = container?.querySelector<HTMLButtonElement>('button[aria-label="별도 창"]');
    await act(async () => toggle?.click());

    expect(toggle?.getAttribute("aria-pressed")).toBe("true");
    expect(container?.textContent).toContain("프리뷰를 닫지 못했습니다");
  });

  it("waits for the old preview to go away before opening the new one", async () => {
    await render(true);
    mocks.show.mockResolvedValue(false);
    mocks.state.mockResolvedValue(null);

    const toggle = container?.querySelector<HTMLButtonElement>('button[aria-label="별도 창"]');
    await act(async () => toggle?.click());
    const refresh = container?.querySelector<HTMLButtonElement>('button[aria-label="새로고침"]');
    await act(async () => refresh?.click());

    expect(mocks.close.mock.invocationCallOrder[0]).toBeLessThan(
      mocks.open.mock.invocationCallOrder[0],
    );
    expect(mocks.open).toHaveBeenCalledWith(
      taskId,
      "http://localhost:3000",
      expect.anything(),
      "inline",
    );
  });
});

/** `designmode://control` 페이로드 — 필요한 필드만 바꿔 쓴다. */
function controlEvent(over: Partial<Parameters<typeof formatLastAction>[0]> = {}) {
  return {
    payload: {
      task_id: taskId,
      active: true,
      op: "click",
      target: "s1e3",
      changed: null,
      url: "http://localhost:3000",
      controllable: true,
      ...over,
    },
  };
}

describe("PreviewTab agent control strip", () => {
  it("turns the badge on while the agent drives the page and off when it goes idle", async () => {
    await render(true);
    expect(container?.querySelector('[role="status"]')).toBeNull();

    await act(async () => handlerFor("designmode://control")?.(controlEvent()));
    expect(container?.querySelector('[role="status"]')?.textContent).toContain("에이전트 제어 중");

    await act(async () =>
      handlerFor("designmode://control")?.(controlEvent({ active: false, changed: true })),
    );
    expect(container?.querySelector('[role="status"]')).toBeNull();
  });

  it("leaves the last action line from the finish event", async () => {
    await render(true);
    await act(async () =>
      handlerFor("designmode://control")?.(
        controlEvent({ active: false, target: 'button "로그인"', changed: true }),
      ),
    );

    const line = container?.querySelector('[data-testid="preview-last-action"]');
    expect(line?.textContent).toBe('click button "로그인" → changed');
  });

  it("marks a page the agent cannot drive", async () => {
    await render(true);
    await act(async () =>
      handlerFor("designmode://control")?.(
        controlEvent({ active: false, op: "snapshot", target: null, controllable: false }),
      ),
    );

    expect(container?.querySelector('[data-testid="preview-last-action"]')?.textContent).toBe(
      "snapshot · 제어 불가 origin",
    );
  });

  it("ignores control events for another task", async () => {
    await render(true);
    await act(async () =>
      handlerFor("designmode://control")?.(controlEvent({ task_id: taskId + 1 })),
    );

    expect(container?.querySelector('[role="status"]')).toBeNull();
    expect(container?.querySelector('[data-testid="preview-last-action"]')).toBeNull();
  });

  it("clears the badge and the last action when the preview closes", async () => {
    await render(true);
    await act(async () => handlerFor("designmode://control")?.(controlEvent()));

    await act(async () => handlerFor("designmode://closed")?.({ payload: taskId }));

    expect(container?.querySelector('[role="status"]')).toBeNull();
    expect(container?.querySelector('[data-testid="preview-last-action"]')).toBeNull();
  });
});

describe("formatLastAction", () => {
  it.each([
    [
      { op: "navigate", target: "http://localhost:3000/login" },
      "navigate http://localhost:3000/login",
    ],
    [{ op: "snapshot", target: null }, "snapshot"],
    [{ op: "click", target: 'button "로그인"', changed: true }, 'click button "로그인" → changed'],
    [{ op: "fill", target: "s1e3", changed: false }, "fill s1e3 → unchanged"],
    [{ op: "press_key", target: "Enter", changed: true }, "press_key Enter → changed"],
    [{ op: "wait_for", target: "로딩 완료" }, 'wait_for "로딩 완료"'],
    [{ op: "console", target: null }, "console"],
  ])("formats %o", (over, expected) => {
    expect(formatLastAction(controlEvent(over).payload)).toBe(expected);
  });
});

describe("PreviewTab native activation", () => {
  it("restores a window opened before the tab mounted, including its URL", async () => {
    mocks.state.mockResolvedValue({ taskId, url: "http://localhost:5173/settings", mode: "window", generation: 4 });
    await render(true);
    expect(container?.querySelector<HTMLInputElement>("input")?.value).toBe("http://localhost:5173/settings");
    expect(container?.textContent).toContain("프리뷰 창이 열려 있습니다");
    expect(mocks.open).not.toHaveBeenCalled();
    expect(mocks.navigate).not.toHaveBeenCalled();
  });

  it("updates an empty tab after agent activation and ignores other tasks", async () => {
    mocks.state.mockResolvedValue(null);
    await render(true);
    expect(container?.textContent).toContain("dev 서버 URL을 입력하고 Enter");
    mocks.state.mockResolvedValue({ taskId, url: "http://127.0.0.1:4173/demo", mode: "window", generation: 5 });
    const count = mocks.state.mock.calls.length;
    await act(async () => handlerFor("designmode://activated")?.({ payload: { taskId: taskId + 1 } }));
    expect(mocks.state).toHaveBeenCalledTimes(count);
    await act(async () => handlerFor("designmode://activated")?.({ payload: { taskId } }));
    expect(container?.querySelector<HTMLInputElement>("input")?.value).toBe("http://127.0.0.1:4173/demo");
    expect(container?.textContent).toContain("프리뷰 창이 열려 있습니다");
  });

  it("does not let a late state response undo activation or a close", async () => {
    let resolveOld!: (value: unknown) => void;
    mocks.state.mockReturnValueOnce(new Promise((resolve) => { resolveOld = resolve; }));
    await render(true);
    mocks.state.mockResolvedValue({ taskId, url: "http://localhost:8080/new", mode: "window", generation: 7 });
    await act(async () => handlerFor("designmode://activated")?.({ payload: { taskId } }));
    await act(async () => resolveOld(null));
    expect(container?.querySelector<HTMLInputElement>("input")?.value).toBe("http://localhost:8080/new");
    expect(container?.textContent).toContain("프리뷰 창이 열려 있습니다");
    mocks.state.mockReturnValueOnce(new Promise((resolve) => { resolveOld = resolve; }));
    await act(async () => handlerFor("designmode://changed")?.({ payload: taskId }));
    await act(async () => handlerFor("designmode://closed")?.({ payload: taskId }));
    await act(async () => resolveOld({ taskId, url: "http://localhost:8080/old", mode: "window", generation: 6 }));
    expect(container?.textContent).toContain("dev 서버 URL을 입력하고 Enter");
  });
});
