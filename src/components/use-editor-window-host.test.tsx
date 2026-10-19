// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Event, EventCallback } from "@tauri-apps/api/event";

const mocks = vi.hoisted(() => {
  const listeners = new Map<string, EventCallback<unknown>>();
  return {
    listeners,
    emitTo: vi.fn(async (): Promise<void> => undefined),
    listen: vi.fn(async <T,>(event: string, handler: EventCallback<T>): Promise<() => void> => {
      listeners.set(event, handler as EventCallback<unknown>);
      return () => {
        listeners.delete(event);
      };
    }),
    editorWindowOpen: vi.fn(async (): Promise<void> => undefined),
    editorWindowHide: vi.fn(async (): Promise<void> => undefined),
    editorWindowFocus: vi.fn(async (): Promise<void> => undefined),
    editorWindowAlive: vi.fn(async (): Promise<boolean> => true),
    editorWindowFilesLoad: vi.fn(async () => ({
      open_paths: [] as string[],
      active_path: null as string | null,
    })),
    editorProjectPrepare: vi.fn(async (root: string) => ({ root, label: "editor", launch: false })),
  };
});

vi.mock("@tauri-apps/api/event", () => ({ emitTo: mocks.emitTo, listen: mocks.listen }));
vi.mock("../lib/project-editor-ipc", () => ({ editorProjectPrepare: mocks.editorProjectPrepare }));
vi.mock("../lib/ipc", () => ({
  editorWindowOpen: mocks.editorWindowOpen,
  editorWindowHide: mocks.editorWindowHide,
  editorWindowFocus: mocks.editorWindowFocus,
  editorWindowAlive: mocks.editorWindowAlive,
  editorWindowFilesLoad: mocks.editorWindowFilesLoad,
}));

import {
  EDITOR_CLOSED_EVENT,
  EDITOR_GONE_EVENT,
  EDITOR_READY_EVENT,
  EDITOR_REVEAL_EVENT,
  EDITOR_SESSION_EVENT,
  EDITOR_STATUS_EVENT,
  EDITOR_AUTOSAVED_EVENT,
  EDITOR_AUTOSAVE_BLOCKED_EVENT,
  EDITOR_ASK_EVENT,
  EDITOR_CAPTURE_EVENT,
  EDITOR_WINDOW_LABEL,
  type EditorAskPayload,
  type EditorAutosavedPayload,
  type EditorCapturePayload,
  type EditorClosedPayload,
  type EditorNotificationPayload,
  type EditorStatus,
} from "../lib/editor-window-events";
import { useEditorWindowHost, type EditorWindowHost } from "./use-editor-window-host";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: EditorWindowHost | null = null;

interface HarnessProps {
  taskId?: number | null;
  host?: string;
  openPaths?: string[];
  /** 주지 않으면 첫 파일이 활성이다 — 활성이 diff 탭인 상황은 `null`을 명시해 만든다(F-12). */
  activePath?: string | null;
  status?: EditorStatus;
  onPopIn?: (payload: EditorClosedPayload) => void;
  onAutosaved?: (payload: EditorAutosavedPayload) => void;
  onAsk?: (payload: EditorAskPayload) => void;
  onCapture?: (payload: EditorCapturePayload) => void;
  onNotification?: (payload: EditorNotificationPayload) => void;
  onError?: (message: string) => void;
}

function Harness({
  taskId = 42,
  host: taskHost = "local",
  openPaths = ["src/a.ts"],
  activePath,
  status = "idle",
  onPopIn = () => {},
  onAutosaved = () => {},
  onAsk = () => {},
  onCapture = () => {},
  onNotification = () => {},
  onError = () => {},
}: HarnessProps) {
  host = useEditorWindowHost({
    taskId,
    host: taskHost,
    branch: "feature/x",
    worktreePath: "/work/task",
    supportsLsp: true,
    openPaths,
    activePath: activePath === undefined ? (openPaths[0] ?? null) : activePath,
    status,
    onPopIn,
    onAutosaved,
    onAsk,
    onCapture,
    onNotification,
    onError,
  });
  return null;
}

function HarnessWithError({ onError }: { onError: (message: string) => void }) {
  host = useEditorWindowHost({
    taskId: 42, host: "local",
    branch: null,
    worktreePath: null,
    supportsLsp: true,
    openPaths: [],
    activePath: null,
    status: "idle",
    onPopIn: () => {},
    onAutosaved: () => {},
    onAsk: () => {},
    onCapture: () => {},
    onNotification: () => {},
    onError,
  });
  return null;
}

let container: HTMLDivElement | null = null;
let root: Root | null = null;

const render = async (props: HarnessProps = {}) => {
  await act(async () => {
    root?.render(<Harness {...props} />);
    await Promise.resolve();
  });
};

const fire = async <T,>(name: string, payload: T) => {
  await act(async () => {
    mocks.listeners.get(name)?.({ event: name, id: 1, payload } as Event<T>);
    await Promise.resolve();
  });
};

beforeEach(() => {
  localStorage.clear();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root?.unmount());
  container?.remove();
  container = null;
  root = null;
  host = null;
  mocks.listeners.clear();
  vi.clearAllMocks();
});

describe("useEditorWindowHost", () => {
  it("홈 프로젝트도 공용 창에 열고 다음 세션 선택을 따라간다", async () => {
    await render({ taskId: null });
    await act(async () => { await host?.openProject("/repo/first"); });
    expect(mocks.editorWindowOpen).toHaveBeenCalledOnce();
    expect(host?.poppedOut).toBe(true);
    expect(mocks.emitTo).toHaveBeenCalledWith(EDITOR_WINDOW_LABEL, EDITOR_SESSION_EVENT, {
      kind: "project", root: "/repo/first", open_paths: [], active_path: null,
    });
    mocks.emitTo.mockClear();
    await render({ taskId: 43 });
    expect(mocks.emitTo).toHaveBeenCalledWith(EDITOR_WINDOW_LABEL, EDITOR_SESSION_EVENT,
      expect.objectContaining({ task_id: 43 }));
  });

  it("홈 프로젝트를 다시 고르면 같은 창에 새 루트를 보낸다", async () => {
    await render({ taskId: null });
    await act(async () => { await host?.openProject("/repo/first"); });
    await act(async () => { await host?.openProject("/repo/second"); });
    mocks.emitTo.mockClear();
    await fire(EDITOR_READY_EVENT, null);
    expect(mocks.emitTo).toHaveBeenCalledWith(EDITOR_WINDOW_LABEL, EDITOR_SESSION_EVENT,
      expect.objectContaining({ kind: "project", root: "/repo/second" }));
  });

  it("프로젝트마다 저장한 탭을 다시 열고 닫을 때 세션 탭으로 섞지 않는다", async () => {
    localStorage.setItem("praxis-project-editor:/repo/first", JSON.stringify({ paths: ["notes.md"], active: "notes.md" }));
    const onPopIn = vi.fn();
    await render({ taskId: null, onPopIn });
    await act(async () => { await host?.openProject("/repo/first"); });
    expect(mocks.emitTo).toHaveBeenCalledWith(EDITOR_WINDOW_LABEL, EDITOR_SESSION_EVENT, {
      kind: "project", root: "/repo/first", open_paths: ["notes.md"], active_path: "notes.md",
    });
    await fire(EDITOR_CLOSED_EVENT, { kind: "project", root: "/repo/first" });
    expect(host?.poppedOut).toBe(false);
    expect(mocks.editorWindowHide).toHaveBeenCalledOnce();
    expect(onPopIn).not.toHaveBeenCalled();
  });

  it("홈 프로젝트 준비 중 세션을 고르면 늦은 프로젝트가 선택을 덮지 않는다", async () => {
    let finish!: (info: { root: string; label: string; launch: boolean }) => void;
    mocks.editorProjectPrepare.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    await render({ taskId: null });
    let opening!: Promise<string>;
    act(() => { opening = host!.openProject("/repo/first"); });
    await render({ taskId: 77 });
    await act(async () => { finish({ root: "/repo/first", label: "editor", launch: false }); await opening; });
    expect(mocks.editorWindowOpen).not.toHaveBeenCalled();
    expect(mocks.emitTo).not.toHaveBeenCalledWith(EDITOR_WINDOW_LABEL, EDITOR_SESSION_EVENT,
      expect.objectContaining({ kind: "project" }));
  });

  it("프로젝트 열기가 실패하면 팝아웃으로 바꾸지 않는다", async () => {
    await render({ taskId: null });
    mocks.editorWindowOpen.mockRejectedValueOnce(new Error("open failed"));
    await act(async () => { await expect(host!.openProject("/repo/first")).rejects.toThrow("open failed"); });
    expect(host?.poppedOut).toBe(false);
  });

  it("같은 세션 번호여도 호스트를 바꾸면 공용 창이 새 호스트를 따른다", async () => {
    await render({ taskId: 42, host: "local" });
    await act(async () => { await host?.popOut(); });
    mocks.emitTo.mockClear();
    await render({ taskId: 42, host: "remote" });
    expect(mocks.emitTo).toHaveBeenCalledWith(EDITOR_WINDOW_LABEL, EDITOR_SESSION_EVENT,
      expect.objectContaining({ task_id: 42, host: "remote" }));
  });

  it("창을 보인 다음에 팝아웃 상태가 된다", async () => {
    // 탭을 먼저 지우고 창 열기에 실패하면 에디터가 어디에도 없게 된다.
    await render();
    expect(host?.poppedOut).toBe(false);

    await act(async () => {
      await host?.popOut();
    });

    expect(mocks.editorWindowOpen).toHaveBeenCalledOnce();
    expect(host?.poppedOut).toBe(true);
    expect(mocks.emitTo).toHaveBeenCalledWith(
      EDITOR_WINDOW_LABEL,
      EDITOR_SESSION_EVENT,
      expect.objectContaining({ task_id: 42, open_paths: ["src/a.ts"], supports_lsp: true }),
    );
  });

  it("창 열기가 실패하면 팝아웃 상태가 되지 않는다", async () => {
    mocks.editorWindowOpen.mockRejectedValueOnce(new Error("no window"));
    await render();

    await act(async () => {
      await host?.popOut();
    });

    expect(host?.poppedOut).toBe(false);
  });

  it("세션이 없으면 뺄 것이 없다", async () => {
    await render({ taskId: null });

    await act(async () => {
      await host?.popOut();
    });

    expect(mocks.editorWindowOpen).not.toHaveBeenCalled();
  });

  it("창이 준비를 알리면 세션을 다시 보낸다", async () => {
    await render();
    await act(async () => {
      await host?.popOut();
    });
    mocks.emitTo.mockClear();

    await fire(EDITOR_READY_EVENT, null);

    expect(mocks.emitTo).toHaveBeenCalledWith(
      EDITOR_WINDOW_LABEL,
      EDITOR_SESSION_EVENT,
      expect.objectContaining({ task_id: 42 }),
    );
  });

  it("팝아웃 전에는 준비 알림에 반응하지 않는다", async () => {
    await render();
    await fire(EDITOR_READY_EVENT, null);

    expect(mocks.emitTo).not.toHaveBeenCalled();
  });

  it("메인 창 목록이 비었으면 저장된 목록을 이어받는다", async () => {
    // 창을 강제 종료하면 editor://closed가 오지 않아 메인 창의 목록이 비어 있다.
    mocks.editorWindowFilesLoad.mockResolvedValueOnce({
      open_paths: ["src/crashed.ts"],
      active_path: "src/crashed.ts",
    });
    await render({ openPaths: [] });

    await act(async () => {
      await host?.popOut();
    });

    expect(mocks.editorWindowFilesLoad).toHaveBeenCalledWith(42);
    expect(mocks.emitTo).toHaveBeenCalledWith(
      EDITOR_WINDOW_LABEL,
      EDITOR_SESSION_EVENT,
      expect.objectContaining({ open_paths: ["src/crashed.ts"] }),
    );
  });

  it("원격 세션은 로컬 팝아웃 목록을 읽지 않는다", async () => {
    await render({ host: "remote", openPaths: [] });

    await act(async () => {
      await host?.popOut();
    });

    expect(mocks.editorWindowFilesLoad).not.toHaveBeenCalled();
  });

  // AC-10 · F-12 — 활성이 diff 탭이면 App이 `activePath`를 null로 준다. 여기서 목록의 첫
  // 항목으로 메워 버리면 저쪽 창이 사용자가 보고 있지 않던 파일을 앞으로 세운다.
  it("활성이 diff면 active_path를 목록에서 메우지 않는다 (AC-10)", async () => {
    await render({ openPaths: ["src/a.ts", "src/b.ts"], activePath: null });

    await act(async () => {
      await host?.popOut();
    });

    // 목록이 비지 않았으므로 저장값 복원 분기로도 새지 않는다.
    expect(mocks.editorWindowFilesLoad).not.toHaveBeenCalled();
    expect(mocks.emitTo).toHaveBeenCalledWith(
      EDITOR_WINDOW_LABEL,
      EDITOR_SESSION_EVENT,
      expect.objectContaining({ open_paths: ["src/a.ts", "src/b.ts"], active_path: null }),
    );
  });

  it("창이 닫히면 파일 목록과 함께 팝인한다", async () => {
    const onPopIn = vi.fn();
    await render({ onPopIn });
    await act(async () => {
      await host?.popOut();
    });

    const payload: EditorClosedPayload = {
      task_id: 42,
      host: "local",
      open_paths: ["src/a.ts", "src/b.ts"],
      active_path: "src/b.ts",
    };
    await fire(EDITOR_CLOSED_EVENT, payload);

    expect(host?.poppedOut).toBe(false);
    expect(onPopIn).toHaveBeenCalledWith(payload);
    expect(mocks.editorWindowHide).toHaveBeenCalledOnce();
  });

  it("이전 세션의 닫힘도 창을 숨기되 현재 세션 파일은 복원하지 않는다", async () => {
    const onPopIn = vi.fn();
    await render({ onPopIn });
    await act(async () => {
      await host?.popOut();
    });
    await render({ taskId: 43, host: "remote" });

    await fire(EDITOR_CLOSED_EVENT, {
      task_id: 42,
      host: "local",
      open_paths: ["src/old.ts"],
      active_path: "src/old.ts",
    });

    expect(mocks.editorWindowHide).toHaveBeenCalledOnce();
    expect(host?.poppedOut).toBe(false);
    expect(onPopIn).not.toHaveBeenCalled();
  });

  it("알림 팝인은 숨김 뒤에만 전달하고 파일 복원은 건너뛴다", async () => {
    const onPopIn = vi.fn();
    const onNotification = vi.fn();
    await render({ onPopIn, onNotification });
    await act(async () => { await host?.popOut(); });
    const item = { host: "local", source_id: "source", task_id: 42, sequence: 3, kind: "result", title: "done", repo: "/tmp/x", ts: 1 } as const;
    await fire(EDITOR_CLOSED_EVENT, {
      task_id: 42, host: "local", open_paths: ["src/a.ts"], active_path: "src/a.ts",
      notification: { action: "result", item, request_id: "request-1" },
    });
    expect(onPopIn).not.toHaveBeenCalled();
    expect(onNotification).toHaveBeenCalledWith(expect.objectContaining({ request_id: "request-1" }));
    expect(mocks.editorWindowHide.mock.invocationCallOrder[0]).toBeLessThan(onNotification.mock.invocationCallOrder[0]);
  });

  it("숨기기에 실패하면 알림 이동을 전달하지 않는다", async () => {
    const onNotification = vi.fn();
    mocks.editorWindowHide.mockRejectedValueOnce(new Error("hide failed"));
    await render({ onNotification });
    await fire(EDITOR_CLOSED_EVENT, {
      task_id: 42, host: "local", open_paths: [], active_path: null,
      notification: { action: "changes", item: { host: "local", source_id: "source", task_id: 42, sequence: 3, kind: "result", title: "done", repo: "/tmp/x", ts: 1 }, request_id: "request-2" },
    });
    expect(onNotification).not.toHaveBeenCalled();
  });

  it("숨기기에 실패해도 창이 이미 죽었으면 팝인을 끝낸다", async () => {
    // 죽은 창은 hide가 "찾을 수 없습니다"로 실패한다. 그때 멈추면 poppedOut이 true로 굳는다.
    const onPopIn = vi.fn();
    const errors: string[] = [];
    await render({ onPopIn, onError: (m) => errors.push(m) });
    await act(async () => { await host?.popOut(); });
    mocks.editorWindowHide.mockRejectedValueOnce(new Error("에디터 창을 찾을 수 없습니다"));
    mocks.editorWindowAlive.mockResolvedValueOnce(false);

    const payload: EditorClosedPayload = { task_id: 42, host: "local", open_paths: ["src/a.ts"], active_path: "src/a.ts" };
    await fire(EDITOR_CLOSED_EVENT, payload);

    expect(host?.poppedOut).toBe(false);
    expect(onPopIn).toHaveBeenCalledWith(payload);
    expect(errors).toEqual([]);
  });

  it("창이 사라지면 팝아웃을 풀고 저장된 목록으로 복원한다", async () => {
    // destroy된 창은 editor://closed를 보내지 못한다. Rust가 대신 보내는 gone에는 목록이 없으므로
    // 편집 때마다 DB에 남긴 목록을 읽어 되살린다.
    const onPopIn = vi.fn();
    await render({ onPopIn, openPaths: ["src/a.ts"] });
    await act(async () => { await host?.popOut(); });
    mocks.editorWindowHide.mockRejectedValueOnce(new Error("에디터 창을 찾을 수 없습니다"));
    mocks.editorWindowFilesLoad.mockResolvedValueOnce({ open_paths: ["src/a.ts", "src/b.ts"], active_path: "src/b.ts" });

    await fire(EDITOR_GONE_EVENT, undefined);

    expect(host?.poppedOut).toBe(false);
    expect(mocks.editorWindowFilesLoad).toHaveBeenCalledWith(42);
    expect(onPopIn).toHaveBeenCalledWith({ task_id: 42, host: "local", open_paths: ["src/a.ts", "src/b.ts"], active_path: "src/b.ts" });
  });

  it("원격 세션의 창이 사라지면 목록 없이 팝아웃만 푼다", async () => {
    const onPopIn = vi.fn();
    await render({ onPopIn, host: "remote", openPaths: ["src/a.ts"] });
    await act(async () => { await host?.popOut(); });
    mocks.editorWindowFilesLoad.mockClear();

    await fire(EDITOR_GONE_EVENT, undefined);

    expect(host?.poppedOut).toBe(false);
    expect(mocks.editorWindowFilesLoad).not.toHaveBeenCalled();
    expect(onPopIn).toHaveBeenCalledWith({ task_id: 42, host: "remote", open_paths: [], active_path: null });
  });

  it("팝인 상태에서 온 사라짐 알림은 아무것도 복원하지 않는다", async () => {
    // 앱 종료 때도 Destroyed가 오고, 정상 팝인 뒤에 늦게 올 수도 있다. 이미 접혀 있으면 할 일이 없다.
    const onPopIn = vi.fn();
    await render({ onPopIn });

    await fire(EDITOR_GONE_EVENT, undefined);

    expect(host?.poppedOut).toBe(false);
    expect(onPopIn).not.toHaveBeenCalled();
  });

  it("자동 저장 결과를 되돌릴 지점으로 넘긴다", async () => {
    const onAutosaved = vi.fn();
    await render({ onAutosaved });

    const payload: EditorAutosavedPayload = {
      task_id: 42,
      entries: [{ path: "src/a.ts", content: "agent-made" }],
    };
    await fire(EDITOR_AUTOSAVED_EVENT, payload);

    expect(onAutosaved).toHaveBeenCalledWith(payload);
  });

  it("저장이 막히면 왜 갈아타지 않았는지 메인 창에도 알린다", async () => {
    // 창은 스스로 앞으로 나오지만, 메인 창만 보고 있으면 아무 일도 없었던 것처럼 보인다.
    const errors: string[] = [];
    await act(async () => {
      root?.render(
        <HarnessWithError
          onError={(m) => {
            errors.push(m);
          }}
        />,
      );
      await Promise.resolve();
    });

    await fire(EDITOR_AUTOSAVE_BLOCKED_EVENT, {
      task_id: 42,
      path: "src/a.ts",
      reason: "conflict",
      detail: null,
    });

    expect(errors[0]).toContain("src/a.ts");
    expect(errors[0]).toContain("디스크에서 바뀌어");
  });

  it("버블 질문을 그대로 넘긴다", async () => {
    const onAsk = vi.fn();
    await render({ onAsk });

    const payload: EditorAskPayload = {
      task_id: 42,
      file_path: "src/a.ts",
      start_line: 3,
      end_line: 5,
      selection_text: "const x = 1;",
      question: "이게 왜 필요해?",
    };
    await fire(EDITOR_ASK_EVENT, payload);

    expect(onAsk).toHaveBeenCalledWith(payload);
  });

  it("팝아웃 중이면 링크를 창으로 배달하고 창을 앞으로 가져온다", async () => {
    // 이 창에는 파일 탭이 없다(CodeColumnTabs가 팝아웃 중 지운다) — 배달하지 않으면 죽은 링크다.
    await render();
    await act(async () => {
      await host?.popOut();
    });
    mocks.emitTo.mockClear();

    let delivered: boolean | undefined;
    await act(async () => {
      delivered = await host?.revealFile({
        task_id: 42,
        host: "local",
        path: "docs/plans/0001.md",
        line: 12,
        column: null,
      });
    });

    expect(delivered).toBe(true);
    expect(mocks.editorWindowFocus).toHaveBeenCalledOnce();
    expect(mocks.emitTo).toHaveBeenCalledWith(EDITOR_WINDOW_LABEL, EDITOR_REVEAL_EVENT, {
      task_id: 42,
      host: "local",
      path: "docs/plans/0001.md",
      line: 12,
      column: null,
    });
  });

  it("팝인 상태에서는 배달하지 않고 호출자에게 맡긴다", async () => {
    await render();

    let delivered: boolean | undefined;
    await act(async () => {
      delivered = await host?.revealFile({ task_id: 42, host: "local", path: "src/a.ts", line: null, column: null });
    });

    expect(delivered).toBe(false);
    expect(mocks.emitTo).not.toHaveBeenCalled();
    expect(mocks.editorWindowFocus).not.toHaveBeenCalled();
  });

  it("다른 host의 같은 task id 링크는 메인 창 fallback으로 돌린다", async () => {
    await render();
    await act(async () => {
      await host?.popOut();
    });
    mocks.emitTo.mockClear();

    await expect(host?.revealFile({
      task_id: 42,
      host: "remote",
      path: "src/a.ts",
      line: null,
      column: null,
    })).resolves.toBe(false);

    expect(mocks.editorWindowFocus).not.toHaveBeenCalled();
    expect(mocks.emitTo).not.toHaveBeenCalled();
  });

  it("창을 앞으로 못 가져오면 보내지 않고 호출자에게 돌려준다", async () => {
    // false를 돌려주지 않으면 링크가 조용히 사라진다 — 이 창은 이미 열지 않기로 했으므로.
    const errors: string[] = [];
    await render({ onError: (m) => errors.push(m) });
    await act(async () => {
      await host?.popOut();
    });
    mocks.editorWindowFocus.mockRejectedValueOnce(new Error("no window"));
    mocks.emitTo.mockClear();

    let delivered: boolean | undefined;
    await act(async () => {
      delivered = await host?.revealFile({ task_id: 42, host: "local", path: "src/a.ts", line: null, column: null });
    });

    expect(delivered).toBe(false);
    expect(mocks.emitTo).not.toHaveBeenCalled();
    expect(errors.join()).toContain("no window");
    // 팝아웃 상태는 건드리지 않는다 — Rust는 show() 다음에 set_focus()를 부르므로 창이
    // 화면에 뜬 채 focus만 실패할 수 있고, 그때 뒤집으면 에디터가 두 곳에 산다.
    expect(host?.poppedOut).toBe(true);
  });

  it("창이 죽어 앞으로 못 가져오면 팝인으로 되돌리고 호출자에게 맡긴다", async () => {
    // 위 케이스와 반대다: 창이 없으면 focus는 언제나 실패하므로 두면 클릭마다 같은 오류만 난다.
    const errors: string[] = [];
    await render({ onError: (m) => errors.push(m) });
    await act(async () => { await host?.popOut(); });
    mocks.editorWindowFocus.mockRejectedValueOnce(new Error("에디터 창을 찾을 수 없습니다"));
    mocks.editorWindowAlive.mockResolvedValueOnce(false);
    mocks.emitTo.mockClear();

    let delivered: boolean | undefined;
    await act(async () => {
      delivered = await host?.revealFile({ task_id: 42, host: "local", path: "src/a.ts", line: null, column: null });
    });

    expect(delivered).toBe(false);
    expect(mocks.emitTo).not.toHaveBeenCalled();
    expect(host?.poppedOut).toBe(false);
    // 호출자가 이 창에서 파일을 여니 오류 배너는 필요 없다.
    expect(errors).toEqual([]);
  });

  it("보내지 못하면 삼키지 않고 호출자에게 돌려준다", async () => {
    // 세션·상태 이벤트와 달리 링크는 재생되지 않는다 — 삼키면 클릭 한 번이 통째로 사라진다.
    const errors: string[] = [];
    await render({ onError: (m) => errors.push(m) });
    await act(async () => {
      await host?.popOut();
    });
    mocks.emitTo.mockRejectedValueOnce(new Error("no listener"));

    let delivered: boolean | undefined;
    await act(async () => {
      delivered = await host?.revealFile({ task_id: 42, host: "local", path: "src/a.ts", line: null, column: null });
    });

    expect(delivered).toBe(false);
    expect(errors.join()).toContain("no listener");
  });

  it("앞으로 가져오는 사이 창이 닫혔으면 보내지 않는다", async () => {
    // focus는 숨긴 창도 다시 보이게 한다(commands.rs의 show()). 팝인된 창을 되살리면 안 된다.
    await render();
    await act(async () => {
      await host?.popOut();
    });
    mocks.emitTo.mockClear();
    mocks.editorWindowFocus.mockImplementationOnce(async () => {
      // focus를 기다리는 사이 창이 닫힌 상황.
      mocks.listeners.get(EDITOR_CLOSED_EVENT)?.({
        event: EDITOR_CLOSED_EVENT,
        id: 1,
        payload: { task_id: 42, host: "local", open_paths: [], active_path: null },
      } as Event<EditorClosedPayload>);
    });

    let delivered: boolean | undefined;
    await act(async () => {
      delivered = await host?.revealFile({ task_id: 42, host: "local", path: "src/a.ts", line: null, column: null });
    });

    expect(delivered).toBe(false);
    expect(mocks.emitTo).not.toHaveBeenCalled();
  });

  it("링크 배달 함수는 팝아웃 전후로 같은 참조다", async () => {
    // 이 참조가 흔들리면 `openAgentLink`가 다시 만들어지고 대화의 Markdown memo가 전부 깨진다.
    await render();
    const before = host?.revealFile;
    await act(async () => {
      await host?.popOut();
    });

    expect(host?.revealFile).toBe(before);
  });

  it("팝아웃의 선택 첨부를 레코드째 위로 올린다", async () => {
    // 캡처 store는 창마다 따로다 — 레코드를 배달받지 않으면 ⌘L이 메인 컴포저에 닿지 않는다.
    // 훅은 store를 모른다. 어디에 넣을지는 창을 아는 App이 정한다.
    const onCapture = vi.fn();
    await render({ onCapture });

    // 목적지는 하니스의 taskId(42)가 아니라 레코드 안의 task_id다 — 창이 옛 세션에 머물러도
    // 캡처는 자기 주소로 간다. 두 값을 다르게 두어 그 계약을 못으로 박는다.
    const payload: EditorCapturePayload = {
      id: "local-w77-0",
      task_id: 77,
      source: "editor",
      outer_html: "",
      computed_css: {},
      bounding_rect: { x: 0, y: 0, width: 0, height: 0 },
      captured_at: 1,
      image_path: null,
      file_path: "src/a.ts",
      selection_text: "const x = 1;",
      selection_start_line: 3,
      selection_end_line: 3,
    };
    await fire(EDITOR_CAPTURE_EVENT, payload);

    expect(onCapture).toHaveBeenCalledWith(payload);
  });

  it("팝아웃 중에만 상태를 흘려보낸다", async () => {
    await render({ status: "idle" });
    await render({ status: "busy" });
    expect(mocks.emitTo).not.toHaveBeenCalledWith(
      EDITOR_WINDOW_LABEL,
      EDITOR_STATUS_EVENT,
      expect.anything(),
    );

    await act(async () => {
      await host?.popOut();
    });
    await render({ status: "busy" });

    expect(mocks.emitTo).toHaveBeenCalledWith(EDITOR_WINDOW_LABEL, EDITOR_STATUS_EVENT, "busy");
  });
});
