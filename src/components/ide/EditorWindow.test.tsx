// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Event, EventCallback } from "@tauri-apps/api/event";
import type { DesignCaptureRecord } from "../../lib/designmode/types";

/** onCloseRequested에 넘기는 핸들러 — 테스트가 붙잡아 직접 호출한다. */
type CloseRequestedHandler = (e: {
  preventDefault: () => void;
}) => void | Promise<void>;

const mocks = vi.hoisted(() => {
  const listeners = new Map<string, EventCallback<unknown>>();
  return {
    listeners,
    emit: vi.fn(async (): Promise<void> => undefined),
    listen: vi.fn(async <T,>(event: string, handler: EventCallback<T>): Promise<() => void> => {
      listeners.set(event, handler as EventCallback<unknown>);
      return () => {
        listeners.delete(event);
      };
    }),
    fsTree: vi.fn(async () => [{ name: "src", path: "src", is_dir: true, children: [] }]),
    fsRead: vi.fn(async () => ({ kind: "text" as const, content: "fn main() {}", mtime: 1 })),
    readLocalFile: vi.fn(async () => ({ kind: "text" as const, content: "external", mtime: 1 })),
    fsWrite: vi.fn(async () => 2),
    fontSettingsGet: vi.fn(async () => ({
      ui_family: "",
      ui_size: 13,
      code_family: "",
      code_size: 13,
    })),
    editorSettingsGet: vi.fn(async () => ({
      tree_font_size: 16,
      minimap: true,
      word_wrap: false,
      tab_size: 2,
    })),
    filesSave: vi.fn(async (): Promise<void> => undefined),
    geometrySave: vi.fn(async (): Promise<void> => undefined),
    replClose: vi.fn(async (): Promise<void> => undefined),
    replRun: vi.fn(async (): Promise<void> => undefined),
    onCloseRequested: vi.fn(
      async (_handler: CloseRequestedHandler) => () => {},
    ),
    onMoved: vi.fn(async () => () => {}),
    onResized: vi.fn(async () => () => {}),
    setFocus: vi.fn(async (): Promise<void> => undefined),
    /** 목킹한 EditorPane이 넘겨받은 onChange — 편집 상태를 만들 때 쓴다. */
    changeSpy: null as null | ((path: string, content: string) => void),
    /** 같은 이유로 붙잡아 두는 onAttachCapture — ⌘L 첨부를 흉내 낼 때 쓴다. */
    attachSpy: null as null | ((record: DesignCaptureRecord) => void),
  };
});

vi.mock("@tauri-apps/api/event", () => ({ emit: mocks.emit, listen: mocks.listen }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    onCloseRequested: mocks.onCloseRequested,
    onMoved: mocks.onMoved,
    onResized: mocks.onResized,
    setFocus: mocks.setFocus,
  }),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openPath: vi.fn(), revealItemInDir: vi.fn() }));
vi.mock("../../lib/ipc", () => ({
  fsTree: mocks.fsTree,
  fsRead: mocks.fsRead,
  readLocalFile: mocks.readLocalFile,
  fsWrite: mocks.fsWrite,
  fontSettingsGet: mocks.fontSettingsGet,
  editorSettingsGet: mocks.editorSettingsGet,
  flattenFiles: (nodes: { is_dir: boolean; path: string; children: unknown[] }[]): string[] =>
    nodes.flatMap((n) =>
      n.is_dir
        ? (n.children as { is_dir: boolean; path: string; children: unknown[] }[]).flatMap((c) =>
            c.is_dir ? [] : [c.path],
          )
        : [n.path],
    ),
  editorWindowFilesSave: mocks.filesSave,
  editorWindowGeometrySave: mocks.geometrySave,
  // 팝아웃이 Python 콘솔을 붙들고 있다 — 세션 전환·팝인에서 프로세스를 닫는다.
  replClose: mocks.replClose,
  replRun: mocks.replRun,
  lspGoto: vi.fn(),
  lspStatus: vi.fn(),
  resolveAbsPath: vi.fn(),
}));
vi.mock("../../lib/transport", () => ({
  LOCAL_HOST: "local",
  getTransport: () => ({ kind: "local" }),
}));
vi.mock("../../lib/use-theme", () => ({ useTheme: () => ({ id: "praxis-dark", kind: "dark" }) }));

// Monaco는 jsdom에서 뜨지 않는다. 창 배선을 보는 테스트이므로 받은 props만 드러낸다.
vi.mock("../QuickOpen", () => ({
  QuickOpen: (props: { open: boolean; scopes?: string[]; initialEditorTab?: string; editorSearch?: { scopeLabel: string; contentAvailable: boolean } }) =>
    props.open ? (
      <div
        data-testid="quick-open"
        data-initial-tab={props.initialEditorTab ?? "all"}
        data-scopes={(props.scopes ?? []).join(",")}
        data-search-scope={props.editorSearch?.scopeLabel ?? ""}
        data-content-search={String(props.editorSearch?.contentAvailable ?? false)}
      />
    ) : null,
}));

vi.mock("./EditorPane", () => ({
  EditorPane: (props: {
    files: { path: string; readOnly?: boolean }[];
    onGoto?: unknown;
    onChange: (path: string, content: string) => void;
    onAttachCapture?: (record: DesignCaptureRecord) => void;
    reveal?: { path: string; line: number; column: number } | null;
    activeKey?: string | null;
  }) => {
    mocks.changeSpy = props.onChange;
    mocks.attachSpy = props.onAttachCapture ?? null;
    return (
      <div
        data-testid="editor-pane"
        data-files={props.files.length}
        data-paths={props.files.map((file) => file.path).join(",")}
        data-read-only={String(props.files.every((file) => file.readOnly === true))}
        data-lsp={props.onGoto ? "on" : "off"}
        data-reveal={
          props.reveal ? `${props.reveal.path}:${props.reveal.line}:${props.reveal.column}` : ""
        }
        data-active={props.activeKey ?? ""}
      />
    );
  },
}));

import {
  EDITOR_AUTOSAVED_EVENT,
  EDITOR_AUTOSAVE_BLOCKED_EVENT,
  EDITOR_CAPTURE_EVENT,
  EDITOR_CLOSED_EVENT,
  EDITOR_GONE_EVENT,
  EDITOR_READY_EVENT,
  EDITOR_REVEAL_EVENT,
  EDITOR_SESSION_EVENT,
  EDITOR_STATUS_EVENT,
  type EditorSessionPayload,
} from "../../lib/editor-window-events";
import { EditorWindow } from "./EditorWindow";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const session = (over: Partial<EditorSessionPayload> = {}): EditorSessionPayload => ({
  task_id: 42,
  host: "local",
  branch: "feature/JH2-63-editor-prep",
  worktree_path: "/work/task",
  open_paths: [],
  active_path: null,
  supports_lsp: true,
  ...over,
});

const event = <T,>(name: string, payload: T): Event<T> => ({ event: name, id: 1, payload });

const capture = (): DesignCaptureRecord => ({
  id: "local-42-0",
  task_id: 42,
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
});

let container: HTMLDivElement | null = null;
let root: Root | null = null;

const mount = async () => {
  await act(async () => {
    root?.render(<EditorWindow />);
    await Promise.resolve();
  });
};

const send = async (name: string, payload: unknown) => {
  await act(async () => {
    mocks.listeners.get(name)?.(event(name, payload));
    await Promise.resolve();
  });
};

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  // 이 둘은 mock이 아니라 평범한 참조다 — `vi.clearAllMocks()`가 비워 주지 않으므로,
  // 안 비우면 앞 테스트가 붙잡아 둔 콜백을 다음 테스트가 제 것인 양 부른다.
  mocks.changeSpy = null;
  mocks.attachSpy = null;
});

afterEach(async () => {
  await act(async () => root?.unmount());
  container?.remove();
  container = null;
  root = null;
  mocks.listeners.clear();
  vi.clearAllMocks();
});

describe("EditorWindow", () => {
  it("세션이 배정되기 전에는 기다린다", async () => {
    await mount();

    expect(container?.textContent).toContain("세션을 기다리는 중");
    expect(container?.querySelector("[data-testid=editor-pane]")).toBeNull();
  });

  it("준비됐음을 알려 메인 창이 세션을 보내게 한다", async () => {
    // 창은 먼저 뜨고 세션은 나중에 온다 — 알리지 않으면 아무도 보내 주지 않는다.
    await mount();

    expect(mocks.emit).toHaveBeenCalledWith(EDITOR_READY_EVENT);
  });

  it("세션을 받으면 에디터와 트리를 그리고 세션을 표시한다", async () => {
    await mount();
    await send(EDITOR_SESSION_EVENT, session());

    expect(container?.querySelector("[data-testid=editor-pane]")).not.toBeNull();
    expect(container?.textContent).toContain("세션 #42");
    expect(container?.textContent).toContain("feature/JH2-63-editor-prep");
    expect(mocks.fsTree).toHaveBeenCalledWith({ host: "local", id: 42 });
  });

  it("에디터 래퍼는 flex 컨테이너로 확정 높이를 내려준다", async () => {
    // Monaco는 퍼센트 높이 체인이다. 래퍼가 flex가 아니면 EditorPane의 flex-1이 무효가 돼
    // 0px로 붕괴하고, 마크다운 프리뷰(콘텐츠 높이)만 보이는 창이 된다. jsdom은 레이아웃을
    // 못 하므로 클래스로 계약을 고정한다.
    await mount();
    await send(EDITOR_SESSION_EVENT, session());

    const wrapper = container?.querySelector("[data-testid=editor-pane]")?.parentElement;
    expect(wrapper?.className).toContain("flex");
    expect(wrapper?.className).toContain("min-h-0");
  });

  it("원격 워크트리면 심볼 이동을 붙이지 않는다", async () => {
    // 언어 서버가 없는 곳에서 ⌘B를 살려 두면 눌러서 실패하는 손잡이가 된다.
    await mount();
    await send(EDITOR_SESSION_EVENT, session({ supports_lsp: false }));

    expect(container?.querySelector("[data-testid=editor-pane]")?.getAttribute("data-lsp")).toBe(
      "off",
    );
  });

  it("세션이 갈아타면 새 워크트리의 트리를 다시 읽는다", async () => {
    await mount();
    await send(EDITOR_SESSION_EVENT, session());
    mocks.fsTree.mockClear();
    await send(EDITOR_SESSION_EVENT, session({ task_id: 77,
      host: "local", branch: "feature/other" }));

    expect(mocks.fsTree).toHaveBeenCalledWith({ host: "local", id: 77 });
    expect(container?.textContent).toContain("세션 #77");
  });

  it("갈아타기 전에 dirty를 저장하고 되돌릴 지점을 넘긴다", async () => {
    await mount();
    await send(EDITOR_SESSION_EVENT, session({ open_paths: ["src/a.ts"], active_path: "src/a.ts" }));
    await act(async () => {
      container?.dispatchEvent(new Event("noop"));
    });
    // 편집 상태를 만든다 — EditorPane을 목킹했으므로 훅을 통해 직접 바꾼다.
    await act(async () => {
      mocks.changeSpy?.("src/a.ts", "my-edit");
      await Promise.resolve();
    });

    await send(EDITOR_SESSION_EVENT, session({ task_id: 77 }));

    expect(mocks.fsWrite).toHaveBeenCalledWith({ host: "local", id: 42 }, "src/a.ts", "my-edit");
    expect(mocks.emit).toHaveBeenCalledWith(
      EDITOR_AUTOSAVED_EVENT,
      expect.objectContaining({
        task_id: 42,
        entries: [{ path: "src/a.ts", content: "fn main() {}" }],
      }),
    );
    expect(container?.textContent).toContain("세션 #77");
  });

  it("충돌이면 갈아타지 않고 창을 앞으로 가져온다", async () => {
    // 시야 밖에서 막혀 있으면 알 길이 없다.
    await mount();
    await send(EDITOR_SESSION_EVENT, session({ open_paths: ["src/a.ts"], active_path: "src/a.ts" }));
    await act(async () => {
      mocks.changeSpy?.("src/a.ts", "my-edit");
      await Promise.resolve();
    });
    mocks.fsRead.mockResolvedValueOnce({ kind: "text", content: "agent-changed", mtime: 9 });

    await send(EDITOR_SESSION_EVENT, session({ task_id: 77 }));

    expect(mocks.fsWrite).not.toHaveBeenCalled();
    expect(mocks.setFocus).toHaveBeenCalled();
    expect(container?.textContent).toContain("세션 #42"); // 갈아타지 않았다
    expect(container?.textContent).toContain("저장하지 못했습니다");
    expect(mocks.emit).toHaveBeenCalledWith(
      EDITOR_AUTOSAVE_BLOCKED_EVENT,
      expect.objectContaining({ path: "src/a.ts", reason: "conflict" }),
    );
  });

  it("메인 창이 열어 두었던 파일을 이어받는다", async () => {
    await mount();
    await send(EDITOR_SESSION_EVENT, session({ open_paths: ["src/a.ts"], active_path: "src/a.ts" }));

    expect(mocks.fsRead).toHaveBeenCalledWith({ host: "local", id: 42 }, "src/a.ts");
    expect(container?.querySelector("[data-testid=editor-pane]")?.getAttribute("data-files")).toBe(
      "1",
    );
  });

  // AC-10 · F-12 — 메인 창의 활성이 diff 탭이면 `active_path`가 비어 온다. diff 키를 그대로
  // 활성으로 세우면 이 창에 없는 탭을 가리키게 되므로, 파일만 열고 활성은 스스로 정한다.
  it("active_path가 비어 와도 파일을 열고 활성 탭을 남긴다 (AC-10)", async () => {
    await mount();
    await send(
      EDITOR_SESSION_EVENT,
      session({ open_paths: ["src/a.ts", "src/b.ts"], active_path: null }),
    );

    const pane = container?.querySelector("[data-testid=editor-pane]");
    expect(pane?.getAttribute("data-files")).toBe("2");
    // 활성이 비면 빈 창처럼 보인다 — 열린 파일 중 하나가 반드시 앞에 선다.
    expect(["src/a.ts", "src/b.ts"]).toContain(pane?.getAttribute("data-active"));
  });

  it("메인 창에서 온 링크를 열고 그 줄에 착지한다", async () => {
    // 팝아웃 중 대화의 파일 링크는 이 창이 받는다 — 메인 창에는 파일 탭이 없다.
    await mount();
    await send(EDITOR_SESSION_EVENT, session());

    await send(EDITOR_REVEAL_EVENT, {
      task_id: 42,
      host: "local",
      path: "docs/plans/0001.md",
      line: 12,
      column: null,
    });

    const pane = container?.querySelector("[data-testid=editor-pane]");
    expect(mocks.fsRead).toHaveBeenCalledWith({ host: "local", id: 42 }, "docs/plans/0001.md");
    expect(pane?.getAttribute("data-files")).toBe("1");
    // column이 없으면 1열 — `path:line` 표기로 온 링크의 기본값이다.
    expect(pane?.getAttribute("data-reveal")).toBe("docs/plans/0001.md:12:1");
  });

  it("팝아웃 창의 외부 파일 링크는 읽기 전용 탭으로 연다", async () => {
    const path = "/Users/test/notes with space.md";
    await mount();
    await send(EDITOR_SESSION_EVENT, session());
    mocks.fsRead.mockClear();

    await send(EDITOR_REVEAL_EVENT, {
      task_id: 42,
      host: "local",
      path,
      line: 12,
      column: null,
    });

    const pane = container?.querySelector("[data-testid=editor-pane]");
    expect(mocks.readLocalFile).toHaveBeenCalledWith({ host: "local", id: 42 }, path);
    expect(mocks.fsRead).not.toHaveBeenCalled();
    expect(pane?.getAttribute("data-paths")).toBe(path);
    expect(pane?.getAttribute("data-read-only")).toBe("true");
    expect(pane?.getAttribute("data-reveal")).toBe(`${path}:12:1`);
  });

  it("줄 없는 링크는 파일만 열고 커서를 옮기지 않는다", async () => {
    await mount();
    await send(EDITOR_SESSION_EVENT, session());

    await send(EDITOR_REVEAL_EVENT, { task_id: 42, host: "local", path: "src/a.ts", line: null, column: null });

    const pane = container?.querySelector("[data-testid=editor-pane]");
    expect(pane?.getAttribute("data-files")).toBe("1");
    expect(pane?.getAttribute("data-reveal")).toBe("");
  });

  it("opens an SSH absolute link in the detached editor without local IPC", async () => {
    const path = "/srv/reports/notes.md";
    await mount();
    await send(EDITOR_SESSION_EVENT, { ...session(), host: "remote" });
    mocks.fsRead.mockClear();
    await send(EDITOR_REVEAL_EVENT, { task_id: 42, host: "remote", path, line: 12, column: null });

    const pane = container?.querySelector("[data-testid=editor-pane]");
    expect(mocks.fsRead).toHaveBeenCalledWith({ host: "remote", id: 42 }, path);
    expect(mocks.readLocalFile).not.toHaveBeenCalled();
    expect(pane?.getAttribute("data-paths")).toBe(path);
    expect(pane?.getAttribute("data-read-only")).toBe("true");
    expect(pane?.getAttribute("data-reveal")).toBe(`${path}:12:1`);
  });

  it("아직 오지 않은 세션의 링크는 맡아 두었다가 그 세션이 오면 연다", async () => {
    // 저장이 막혀 창이 옛 세션에 머무는 구간이 있다. 그때 같은 상대 경로는 다른 워크트리를
    // 가리키므로 지금 열 수 없지만, 메인 창은 이미 배달됐다고 보고 폴백하지 않는다 —
    // 여기서 버리면 클릭이 통째로 사라진다.
    await mount();
    await send(EDITOR_SESSION_EVENT, session());
    mocks.fsRead.mockClear();

    await send(EDITOR_REVEAL_EVENT, { task_id: 77, host: "local", path: "src/a.ts", line: 3, column: null });
    expect(mocks.fsRead).not.toHaveBeenCalled();

    await send(EDITOR_SESSION_EVENT, session({ task_id: 77 }));

    expect(mocks.fsRead).toHaveBeenCalledWith({ host: "local", id: 77 }, "src/a.ts");
    expect(
      container?.querySelector("[data-testid=editor-pane]")?.getAttribute("data-reveal"),
    ).toBe("src/a.ts:3:1");
  });

  it("같은 task id라도 host가 다르면 링크를 새 세션까지 보류한다", async () => {
    await mount();
    await send(EDITOR_SESSION_EVENT, session());
    mocks.fsRead.mockClear();

    await send(EDITOR_REVEAL_EVENT, { task_id: 42, host: "remote", path: "src/a.ts", line: 3, column: null });
    expect(mocks.fsRead).not.toHaveBeenCalled();

    await send(EDITOR_SESSION_EVENT, session({ host: "remote" }));

    expect(mocks.fsRead).toHaveBeenCalledWith({ host: "remote", id: 42 }, "src/a.ts");
  });

  it("세션이 배정되기 전에 온 링크도 맡아 둔다", async () => {
    // 팝아웃 직후 창이 부팅 중일 때 클릭하면 세션보다 링크가 먼저 도착한다.
    await mount();

    await send(EDITOR_REVEAL_EVENT, { task_id: 42, host: "local", path: "src/a.ts", line: 7, column: null });
    expect(mocks.fsRead).not.toHaveBeenCalled();

    await send(EDITOR_SESSION_EVENT, session());

    expect(mocks.fsRead).toHaveBeenCalledWith({ host: "local", id: 42 }, "src/a.ts");
    expect(
      container?.querySelector("[data-testid=editor-pane]")?.getAttribute("data-reveal"),
    ).toBe("src/a.ts:7:1");
  });

  it("열지 못한 파일에는 착지 지점을 걸지 않는다", async () => {
    // 남은 착지 지점은 소비되지 않고 대기하다가, 나중에 그 경로를 열 때 커서를 튀게 한다.
    await mount();
    await send(EDITOR_SESSION_EVENT, session());
    mocks.fsRead.mockRejectedValueOnce(new Error("없는 파일"));

    await send(EDITOR_REVEAL_EVENT, { task_id: 42, host: "local", path: "docs/gone.md", line: 5, column: null });

    const pane = container?.querySelector("[data-testid=editor-pane]");
    expect(pane?.getAttribute("data-files")).toBe("0");
    expect(pane?.getAttribute("data-reveal")).toBe("");
    expect(container?.querySelector("[role=alert]")?.textContent).toContain("없는 파일");
  });

  it("이미 열린 파일로 온 링크는 그 탭을 활성으로 돌리고 착지시킨다", async () => {
    // 창은 메인 창의 열린 목록을 이어받으므로, 링크 대상이 이미 열려 있는 쪽이 오히려 흔하다.
    await mount();
    await send(EDITOR_SESSION_EVENT, session({ open_paths: ["src/a.ts", "src/b.ts"], active_path: "src/b.ts" }));
    mocks.fsRead.mockClear();

    await send(EDITOR_REVEAL_EVENT, { task_id: 42, host: "local", path: "src/a.ts", line: 9, column: 4 });

    const pane = container?.querySelector("[data-testid=editor-pane]");
    expect(mocks.fsRead).not.toHaveBeenCalled();
    expect(pane?.getAttribute("data-files")).toBe("2");
    expect(pane?.getAttribute("data-active")).toBe("src/a.ts");
    expect(pane?.getAttribute("data-reveal")).toBe("src/a.ts:9:4");
  });

  it("0줄 링크는 파일만 연다", async () => {
    // `docs/a.md:0`도 링크 문법을 통과한다. 0줄은 없다.
    await mount();
    await send(EDITOR_SESSION_EVENT, session());

    await send(EDITOR_REVEAL_EVENT, { task_id: 42, host: "local", path: "src/a.ts", line: 0, column: null });

    const pane = container?.querySelector("[data-testid=editor-pane]");
    expect(pane?.getAttribute("data-files")).toBe("1");
    expect(pane?.getAttribute("data-reveal")).toBe("");
  });

  it("열린 목록을 변경 시마다 남긴다", async () => {
    // editor://closed는 정상 닫기에서만 온다. 크래시·강제 종료에서 목록을 지키는 것은 이 저장뿐이다.
    await mount();
    await send(EDITOR_SESSION_EVENT, session({ open_paths: ["src/a.ts"], active_path: "src/a.ts" }));

    expect(mocks.filesSave).toHaveBeenCalledWith(42, ["src/a.ts"], "src/a.ts");
  });

  it("원격 세션의 열린 목록을 로컬 저장소에 남기지 않는다", async () => {
    await mount();
    mocks.filesSave.mockClear();

    await send(EDITOR_SESSION_EVENT, session({ host: "remote", open_paths: ["src/a.ts"] }));

    expect(mocks.filesSave).not.toHaveBeenCalled();
  });

  it("창 닫기·이동·리사이즈를 붙잡는다", async () => {
    await mount();

    expect(mocks.onCloseRequested).toHaveBeenCalledOnce();
    expect(mocks.onMoved).toHaveBeenCalledOnce();
    expect(mocks.onResized).toHaveBeenCalledOnce();
  });

  const closeRequested = async () => {
    const handler = mocks.onCloseRequested.mock.calls[0]?.[0];
    const e = { preventDefault: vi.fn() };
    await act(async () => {
      await handler?.(e);
      await Promise.resolve();
    });
    return e;
  };

  it("세션이 없는 채 닫으면 destroy 대신 사라짐을 알린다", async () => {
    // preventDefault 없이 돌아가면 Tauri가 창을 destroy한다. 창은 부팅 때 한 번만 만들어지므로
    // 그 뒤 메인 창의 모든 호출이 "에디터 창을 찾을 수 없습니다"로 죽고 팝아웃 상태가 굳는다.
    await mount();

    const e = await closeRequested();

    expect(e.preventDefault).toHaveBeenCalledOnce();
    expect(mocks.emit).toHaveBeenCalledWith(EDITOR_GONE_EVENT);
    expect(mocks.emit).not.toHaveBeenCalledWith(EDITOR_CLOSED_EVENT, expect.anything());
  });

  it("세션이 있는 채 닫으면 파일 목록과 함께 팝인한다", async () => {
    await mount();
    await send(EDITOR_SESSION_EVENT, session({ open_paths: ["src/a.ts"], active_path: "src/a.ts" }));
    mocks.emit.mockClear();

    const e = await closeRequested();

    expect(e.preventDefault).toHaveBeenCalledOnce();
    expect(mocks.emit).toHaveBeenCalledWith(
      EDITOR_CLOSED_EVENT,
      expect.objectContaining({ task_id: 42, host: "local", open_paths: ["src/a.ts"], active_path: "src/a.ts" }),
    );
    expect(mocks.emit).not.toHaveBeenCalledWith(EDITOR_GONE_EVENT);
  });

  it("선택 첨부를 메인 창으로 배달하고 상태바에 알린다", async () => {
    // 팝아웃의 캡처 store는 메인 창과 별개다. 배달하지 않으면 ⌘L이 아무 데도 닿지 않는다.
    await mount();
    await send(EDITOR_SESSION_EVENT, session());

    await act(async () => {
      mocks.attachSpy?.(capture());
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(mocks.emit).toHaveBeenCalledWith(
      EDITOR_CAPTURE_EVENT,
      // 메인·팝아웃의 seq가 둘 다 0부터라 스코프 접두사 없이는 id가 겹친다.
      expect.objectContaining({ id: "local-w42-0", task_id: 42, selection_text: "const x = 1;" }),
    );
    // 목적지 세션이 문구에 있어야 한다 — 칩은 이 창에 나타나지 않는다.
    expect(container?.textContent).toContain("세션 #42에 첨부됨 ✓");
  });

  it("배달이 실패하면 ✓ 대신 사유를 남긴다", async () => {
    // 조용히 실패하면 사용자는 첨부된 줄 알고 질문을 보낸다 — 칩이 없는 것은 나중에야 보인다.
    await mount();
    await send(EDITOR_SESSION_EVENT, session());
    mocks.emit.mockRejectedValueOnce(new Error("창이 없다"));

    await act(async () => {
      mocks.attachSpy?.(capture());
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(container?.querySelector('[role="alert"]')?.textContent).toContain("창이 없다");
    expect(container?.textContent).not.toContain("첨부됨 ✓");
  });

  it("알림은 2초 뒤 스스로 사라지고, 이어 첨부하면 시계가 다시 돈다", async () => {
    vi.useFakeTimers();
    try {
      await mount();
      await send(EDITOR_SESSION_EVENT, session());

      const attach = async () => {
        await act(async () => {
          mocks.attachSpy?.(capture());
          await Promise.resolve();
          await Promise.resolve();
        });
      };

      await attach();
      expect(container?.textContent).toContain("첨부됨 ✓");

      // 앞 첨부의 타이머가 살아 있으면 뒤 알림이 뜬 지 1초 만에 꺼진다.
      await act(async () => {
        vi.advanceTimersByTime(1500);
      });
      await attach();
      await act(async () => {
        vi.advanceTimersByTime(1500);
      });
      expect(container?.textContent).toContain("첨부됨 ✓");

      await act(async () => {
        vi.advanceTimersByTime(500);
      });
      expect(container?.textContent).not.toContain("첨부됨 ✓");
    } finally {
      vi.useRealTimers();
    }
  });

  it("세션이 갈리면 앞 세션 앞으로 온 알림을 지운다", async () => {
    // "세션 #42에 첨부됨"이 #77 상태바에 남아 있으면 방금 것이 저기로 갔다고 읽힌다.
    await mount();
    await send(EDITOR_SESSION_EVENT, session());
    await act(async () => {
      mocks.attachSpy?.(capture());
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(container?.textContent).toContain("세션 #42에 첨부됨 ✓");

    await send(EDITOR_SESSION_EVENT, session({ task_id: 77 }));

    expect(container?.textContent).not.toContain("첨부됨 ✓");
  });

  it("숨김 토글로 도트 항목을 낸다", async () => {
    // 메인 창에만 있던 토글 — 팝아웃에서 `.github`를 보려면 창을 되돌려야 했다.
    mocks.fsTree.mockResolvedValueOnce([
      { name: ".github", path: ".github", is_dir: true, children: [] },
      { name: "src", path: "src", is_dir: true, children: [] },
    ]);
    await mount();
    await send(EDITOR_SESSION_EVENT, session());
    expect(container?.textContent).not.toContain(".github");

    const toggle = container?.querySelector('[aria-label="숨김 항목"]') as HTMLElement;
    await act(async () => {
      toggle.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    expect(container?.textContent).toContain(".github");
    expect(toggle.getAttribute("aria-pressed")).toBe("true");

    // 끄는 방향도 본다 — 켜기만 검증하면 토글이 한 방향 스위치여도 통과한다.
    await act(async () => {
      toggle.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });
    expect(container?.textContent).not.toContain(".github");
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
  });

  it("새로고침은 트리를 다시 읽는다", async () => {
    // 팝아웃에서 브랜치를 갈아타면 트리가 낡는다 — 창을 되돌리지 않고 다시 읽을 수 있어야 한다.
    await mount();
    await send(EDITOR_SESSION_EVENT, session());
    const before = mocks.fsTree.mock.calls.length;

    const refresh = container?.querySelector('[aria-label="파일 트리 새로고침"]') as HTMLElement;
    await act(async () => {
      refresh.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    expect(mocks.fsTree.mock.calls.length).toBeGreaterThan(before);
    expect(mocks.fsTree).toHaveBeenLastCalledWith({ host: "local", id: 42 });
  });

  it("상태를 받으면 배지에 반영한다", async () => {
    await mount();
    await send(EDITOR_SESSION_EVENT, session());
    expect(container?.textContent).toContain("대기");

    await send(EDITOR_STATUS_EVENT, "busy");
    expect(container?.textContent).toContain("응답 중…");

    await send(EDITOR_STATUS_EVENT, "done");
    expect(container?.textContent).toContain("완료");
  });

  /**
   * 팝아웃 창은 별개의 웹뷰다 — 메인 창에 건 리스너가 여기 키를 듣지 못한다.
   * 배선이 빠지면 에디터에서 Shift를 두 번 눌러도 아무 일이 일어나지 않는다.
   */
  const shiftTap = async () => {
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Shift" }));
      window.dispatchEvent(new KeyboardEvent("keyup", { key: "Shift" }));
      await Promise.resolve();
    });
  };

  it("Shift 더블탭이 Quick Open을 연다", async () => {
    await mount();
    await send(EDITOR_SESSION_EVENT, session());
    expect(container?.querySelector("[data-testid=quick-open]")).toBeNull();

    await shiftTap();
    await shiftTap();

    expect(container?.querySelector("[data-testid=quick-open]")).not.toBeNull();
  });

  it("에디터 자식이 버블을 소비해도 Shift 더블탭을 잡는다", async () => {
    await mount();
    await send(EDITOR_SESSION_EVENT, session());
    const editor = document.createElement("textarea");
    editor.addEventListener("keydown", (event) => event.stopPropagation());
    editor.addEventListener("keyup", (event) => event.stopPropagation());
    container?.append(editor);
    editor.focus();

    await act(async () => {
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Shift", bubbles: true }));
      editor.dispatchEvent(new KeyboardEvent("keyup", { key: "Shift", bubbles: true }));
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Shift", bubbles: true }));
      editor.dispatchEvent(new KeyboardEvent("keyup", { key: "Shift", bubbles: true }));
      await Promise.resolve();
    });

    expect(container?.querySelector("[data-testid=quick-open]")).not.toBeNull();
  });

  it("한 번만 누르면 열지 않는다", async () => {
    await mount();
    await send(EDITOR_SESSION_EVENT, session());

    await shiftTap();

    expect(container?.querySelector("[data-testid=quick-open]")).toBeNull();
  });

  it("Shift 사이에 다른 키가 끼면 열지 않는다", async () => {
    // `Shift+A`를 입력하는 동안의 Shift 두 번은 의도가 아니다.
    await mount();
    await send(EDITOR_SESSION_EVENT, session());

    await shiftTap();
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "a" }));
      await Promise.resolve();
    });
    await shiftTap();

    expect(container?.querySelector("[data-testid=quick-open]")).toBeNull();
  });

  it("Shift 더블탭은 전체 범위로 연다", async () => {
    await mount();
    await send(EDITOR_SESSION_EVENT, session());
    await shiftTap();
    await shiftTap();

    expect(container?.querySelector("[data-testid=quick-open]")?.getAttribute("data-initial-tab")).toBe("all");
  });

  /** macOS에서 ⌥F는 `ƒ`를 입력한다 — 에디터가 키를 받기 전에 캡처 단계에서 가로채야 한다. */
  const altF = () =>
    new KeyboardEvent("keydown", { key: "ƒ", code: "KeyF", altKey: true, bubbles: true, cancelable: true });

  it("⌥F가 Quick Open을 파일 범위로 열고, 에디터에는 글자를 넘기지 않는다", async () => {
    await mount();
    await send(EDITOR_SESSION_EVENT, session());
    const editor = document.createElement("textarea");
    const reached = vi.fn();
    editor.addEventListener("keydown", reached);
    container?.append(editor);
    editor.focus();

    const event = altF();
    await act(async () => {
      editor.dispatchEvent(event);
      await Promise.resolve();
    });

    const quick = container?.querySelector("[data-testid=quick-open]");
    expect(quick).not.toBeNull();
    expect(quick?.getAttribute("data-initial-tab")).toBe("file");
    expect(event.defaultPrevented).toBe(true);
    expect(reached).not.toHaveBeenCalled();
  });

  it("다른 수식키가 섞인 F는 파일 찾기가 아니다", async () => {
    await mount();
    await send(EDITOR_SESSION_EVENT, session());

    for (const extra of [{ metaKey: true }, { ctrlKey: true }, { shiftKey: true }]) {
      await act(async () => {
        window.dispatchEvent(new KeyboardEvent("keydown", { code: "KeyF", altKey: true, ...extra }));
        await Promise.resolve();
      });
    }
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "f", code: "KeyF" }));
      await Promise.resolve();
    });

    expect(container?.querySelector("[data-testid=quick-open]")).toBeNull();
  });

  it("이 창이 열 수 있는 스코프만 보여 준다", async () => {
    // 작업·세션·스킬·커맨드는 팝아웃 창에 열 자리가 없다 — 보여 주면 고를 수 있는데
    // 아무 일도 안 하는 항목이 된다.
    await mount();
    await send(EDITOR_SESSION_EVENT, session());
    await shiftTap();
    await shiftTap();

    expect(
      container?.querySelector("[data-testid=quick-open]")?.getAttribute("data-scopes"),
    ).toBe("file,code");
    expect(container?.querySelector("[data-testid=quick-open]")?.getAttribute("data-search-scope")).toBe(
      "feature/JH2-63-editor-prep · local · 세션 #42",
    );
  });

  it("원격 창은 로컬 내용 검색을 사용할 수 없게 표시한다", async () => {
    await mount();
    await send(EDITOR_SESSION_EVENT, session({ host: "remote", task_id: 42 }));
    await shiftTap();
    await shiftTap();

    expect(container?.querySelector("[data-testid=quick-open]")?.getAttribute("data-content-search")).toBe("false");
  });
});
