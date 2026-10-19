import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { LOCAL_HOST } from "../../lib/transport";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { copyText } from "../../lib/clipboard";
import {
  EDITOR_ASK_EVENT,
  EDITOR_AUTOSAVED_EVENT,
  EDITOR_AUTOSAVE_BLOCKED_EVENT,
  EDITOR_CAPTURE_EVENT,
  EDITOR_CLOSED_EVENT,
  EDITOR_GONE_EVENT,
  EDITOR_READY_EVENT,
  EDITOR_REVEAL_EVENT,
  EDITOR_REVERTED_EVENT,
  EDITOR_SESSION_EVENT,
  EDITOR_STATUS_EVENT,
  PROJECT_EDITOR_CLOSED_EVENT,
  isProjectWorkspace,
  type EditorRevealPayload,
  type EditorNotificationPayload,
  type EditorSessionPayload,
  type EditorStatus,
  type EditorWorkspacePayload,
} from "../../lib/editor-window-events";
import { editorProjectActivate, projectEditorTree, projectEditorRead, projectEditorWrite, projectEditorResolvePath, projectEditorOpenPath } from "../../lib/project-editor-ipc";
import { saveProjectTabs } from "../../lib/project-editor-tabs";
import {
  editorWindowFilesSave,
  editorWindowGeometrySave,
  fontSettingsGet,
  editorSettingsGet,
  flattenFiles,
  replClose,
  replRun,
  type FontSettings,
  type EditorSettings,
} from "../../lib/ipc";
import { codeFontStack, uiFontStack } from "../../lib/fonts";
import {
  DEFAULT_EDITOR_SETTINGS,
  applyTreeMetrics,
  normalizeEditorSettings,
} from "../../lib/editor-settings";
import { scopeLocalCaptureId } from "../../lib/designmode/selection-capture";
import { buildAskPayload } from "../../lib/selection-ask";
import { useTheme } from "../../lib/use-theme";
import { EditorSplitView } from "./EditorSplitView";
import { FileTree } from "./FileTree";
import { useEditorActions } from "./useEditorActions";
import { useWorkspaceFiles, type WorkspaceFileSource } from "./useWorkspaceFiles";
import { asTabKey, fileTabKey, isDiffKey, pathFromTabKey } from "../../lib/tab-key";
import { DiffSessionScope } from "../DiffSessionContext";
import { ChangesList } from "./ChangesList";
import type { TreeLens } from "./useSessionPanels";
import { useTreeFileOps } from "./useTreeFileOps";
import { ContextMenuShell } from "./ContextMenuShell";
import { FilePromptDialog } from "./FilePromptDialog";
import { targetDir } from "./tree-file-ops";
import { QuickOpen, type EditorSearchTab } from "../QuickOpen";
import { parseCodeItemId, type RankedQuickOpenItem } from "../../lib/quickopen";
import { onKeyDown as shiftDown, onKeyUp as shiftUp, type ShiftTapState } from "../../lib/shift-double-tap";
import { Icon } from "./icons";
import { ReplDock } from "./ReplDock";
import { pandasOpenSnippet } from "../../lib/repl-snippets";
import { ProjectTerminal, restartProjectShell } from "./ProjectTerminal";

const STATUS_LABEL: Record<EditorStatus, string> = {
  idle: "대기",
  busy: "응답 중…",
  done: "완료",
  error: "오류",
};

/** 첨부 알림이 남아 있는 시간(ms). 결과는 메인 창에 나타나므로 여기서는 흔적을 남기지 않는다. */
const NOTICE_MS = 2000;

function StatusBar({
  session,
  status,
  error,
  notice,
}: {
  session: EditorSessionPayload;
  status: EditorStatus;
  error: string | null;
  notice?: string | null;
}) {
  return (
    <div className="h-7 shrink-0 border-t border-border bg-raised px-3 flex items-center gap-2 text-xs text-text-muted">
      <span className="text-accent">●</span>
      <span>세션 #{session.task_id}</span>
      {session.branch && <span className="truncate">· {session.branch}</span>}
      {notice && (
        <span className="shrink-0 text-primary-bright" role="status">
          · {notice}
        </span>
      )}
      {error && (
        <span className="truncate text-danger" role="alert">
          · {error}
        </span>
      )}
      <span className="ml-auto" aria-live="polite">
        {STATUS_LABEL[status]}
        {status === "busy" && " ⏳"}
        {status === "done" && " ✓"}
      </span>
    </div>
  );
}

/**
 * 팝아웃된 에디터 창의 루트.
 *
 * 상태를 스스로 소유한다 — 파일 읽기·쓰기·LSP를 메인 창에 묻지 않고 직접 IPC로 부른다.
 * 메인 창을 경유하면 키 입력마다 창 사이를 왕복해 Monaco의 체감이 무너진다.
 *
 * 창은 항상 떠 있고(`visible: false`로 만들어 둔다) 세션은 나중에 배정된다. 그래서 마운트
 * 직후에는 그릴 것이 없고, `editor://ready`로 준비를 알린 뒤 메인 창이 보내 주는 세션을 기다린다.
 */
export function EditorWindow() {
  const [workspace, setWorkspace] = useState<EditorWorkspacePayload | null>(null);
  const workspaceRef = useRef<EditorWorkspacePayload | null>(null);
  workspaceRef.current = workspace;
  const project = workspace && isProjectWorkspace(workspace) ? workspace : null;
  const session = workspace && !isProjectWorkspace(workspace) ? workspace : null;
  const workspaceKey = project ? `project:${project.root}` : session ? `task:${session.host}:${session.task_id}` : null;
  const [switching, setSwitching] = useState(false);
  const [restoredProject, setRestoredProject] = useState<string | null>(null);
  const [status, setStatus] = useState<EditorStatus>("idle");
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [fontSettings, setFontSettings] = useState<FontSettings | null>(null);
  const sessionRef = useRef<EditorSessionPayload | null>(null);
  sessionRef.current = session;
  const [editorSettings, setEditorSettings] = useState<EditorSettings>(DEFAULT_EDITOR_SETTINGS);
  /** 트리에 도트 파일을 낼지. 기본은 감춤 — 매일 여는 디렉터리가 위쪽을 차지해야 한다. */
  const [showHidden, setShowHidden] = useState(false);
  /** 트리를 접을 수 있게 둔다 — 이 창은 통째로 에디터라 폭이 곧 코드 폭이다.
   *  접힌 자리에는 다시 펼 손잡이만 남는다(메인 창은 상단 툴바의 폴더 칩이 그 역할). */
  const [treeOpen, setTreeOpen] = useState(true);
  /** 트리 열 렌즈 — 파일 트리인가 변경 목록인가(ADR 2026-10-08 결정 1). 세션이 바뀌면 파일로 돌아간다. */
  const [treeLens, setTreeLens] = useState<TreeLens>("files");
  /** 열린 Quick Open의 시작 범위. null이면 닫혀 있다 — Shift 더블탭은 전체, ⌥F는 파일로 연다. */
  const [quickOpen, setQuickOpen] = useState<EditorSearchTab | null>(null);
  /** 하단 Python 콘솔(IPython) 도크. ⌃`로 여닫는다 — 메인 창의 터미널 도크와 같은 손동작. */
  const [replDock, setReplDock] = useState(false);
  const [terminalStarted, setTerminalStarted] = useState(false);
  const [terminalOpen, setTerminalOpen] = useState(false);
  const [terminalExited, setTerminalExited] = useState(false);
  const [shellSession, setShellSession] = useState<number | null>(null);
  const [terminalRestart, setTerminalRestart] = useState(0);
  const restartingRef = useRef(false);
  const theme = useTheme();

  // 알림은 잠깐 떴다 사라진다. 타이머를 ref로 잡아 두어야 연속 첨부에서 앞 타이머가
  // 뒤 알림을 지우지 않고, 언마운트 뒤에 setState를 부르지도 않는다.
  const noticeTimerRef = useRef<number | undefined>(undefined);
  const showNotice = useCallback((message: string) => {
    globalThis.clearTimeout(noticeTimerRef.current);
    setNotice(message);
    noticeTimerRef.current = globalThis.setTimeout(() => setNotice(null), NOTICE_MS);
  }, []);
  useEffect(() => () => globalThis.clearTimeout(noticeTimerRef.current), []);

  // 창은 작업 목록을 갖지 않는다 — 좌표는 메인 창이 세션 페이로드로 실어 보낸 것을 쓴다.
  const task = session ? { host: session.host, id: session.task_id } : null;
  const workspaceSourceChangeRef = useRef<(path: string) => void>(() => undefined);
  const source = useMemo<WorkspaceFileSource | null>(() => project == null ? null : {
    key: `project:${project.root}`,
    tree: () => projectEditorTree(project.root),
    read: (path) => projectEditorRead(path, project.root),
    write: (path, content, expected) => projectEditorWrite(path, content, expected, project.root),
  }, [project?.root]);
  const files = useWorkspaceFiles({
    task,
    source,
    onError: setError,
    onSourceChange: (path) => workspaceSourceChangeRef.current(path),
  });
  const actions = useEditorActions({
    taskId: session?.task_id ?? null,
    projectRoot: project?.root,
    host: task?.host ?? LOCAL_HOST,
    onError: setError,
    openFile: files.openFile,
  });
  const openPathExternal = useCallback((path: string) => project
    ? projectEditorOpenPath(path, project.root).catch((cause) => setError(String(cause)))
    : actions.openPathExternal(path), [project?.root, actions.openPathExternal]);
  const revealPathInFinder = useCallback((path: string) => project
    ? projectEditorResolvePath(path, project.root).then(revealItemInDir).catch((cause) => setError(String(cause)))
    : actions.revealPathInFinder(path), [project?.root, actions.revealPathInFinder]);
  const copyAbsPath = useCallback((path: string) => project
    ? projectEditorResolvePath(path, project.root).then(copyText).catch((cause) => setError(String(cause)))
    : actions.copyAbsPath(path), [project?.root, actions.copyAbsPath]);

  /** 활성 **파일 탭**의 실경로. diff 탭이 활성이면 `null`이다 — 트리 선택·프로젝트 탭 저장이 본다.
   *  세션 저장·팝인 페이로드는 이것을 쓰지 않는다: 탭 키를 그대로 넘긴다(ADR 2026-10-08 결정 5). */
  const activeFilePath =
    files.activeFile != null && files.activeFile.kind !== "diff" ? files.activeFile.path : null;

  /** Quick Open이 보는 파일 목록 — 렌더마다 새 배열이면 닫혀 있어도 전체 정렬이 매번 돈다(원장 #448). */
  const workspaceFiles = useMemo(() => flattenFiles(files.tree), [files.tree]);

  /** 트리 우클릭 메뉴와 새 파일·폴더 — 메인 창과 같은 훅이라 "어디에 만드는가"의 규칙이 하나다. */
  const treeOps = useTreeFileOps({
    taskId: session?.task_id ?? null,
    tree: files.tree,
    refreshTree: files.refreshTree,
    openFile: files.openFile,
    // 생성 IPC는 로컬 고정이다. 이 창은 원격 워크트리도 띄우므로 같은 신호로 가른다.
    canMutate: session?.supports_lsp === true,
    openFiles: files.openFiles,
    activePath: activeFilePath,
    closeTabsForPath: files.closeTabsForPath,
    onError: setError,
    onRevealPath: (path) => void revealPathInFinder(path),
    onCopyAbsPath: (path) => void copyAbsPath(path),
  });

  /**
   * Shift 더블탭 — IntelliJ의 "Search Everywhere". 메인 창과 같은 손동작을 이 창에도 둔다.
   *
   * 팝아웃 창은 **별개의 웹뷰**라 메인 창의 리스너가 여기 키를 듣지 못한다. 그래서 같은
   * 오버레이를 이 창에도 띄운다 — 없으면 에디터에서 눌렀을 때 아무 일도 일어나지 않는다.
   *
   * 스코프는 파일과 코드뿐이다. 작업·세션·스킬·커맨드는 이 창에 열 자리가 없어서, 보여 주면
   * 고를 수 있는데 아무 일도 안 하는 항목이 된다.
   */
  useEffect(() => {
    let tap: ShiftTapState = { lastUp: 0 };
    const down = (e: KeyboardEvent) => {
      tap = shiftDown(tap, e);
    };
    const up = (e: KeyboardEvent) => {
      const { next, fire } = shiftUp(tap, e, performance.now());
      tap = next;
      if (fire) setQuickOpen("all");
    };
    window.addEventListener("keydown", down, true);
    window.addEventListener("keyup", up, true);
    return () => {
      window.removeEventListener("keydown", down, true);
      window.removeEventListener("keyup", up, true);
    };
  }, []);

  /**
   * ⌥F — 파일 찾기. Quick Open을 파일 범위로 연다.
   *
   * 캡처 단계에서 잡는다: macOS에서 ⌥F는 `ƒ`를 입력하는 키라, Monaco에 먼저 닿으면 코드에
   * 글자가 들어간다. `e.key`가 `ƒ`로 오므로 물리 키(`e.code`)로 본다.
   */
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.altKey || e.metaKey || e.ctrlKey || e.shiftKey || e.code !== "KeyF") return;
      e.preventDefault();
      e.stopPropagation();
      if (e.repeat) return;
      setQuickOpen((current) => current ?? "file");
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  useEffect(() => {
    setTerminalStarted(false);
    setTerminalOpen(false);
    setTerminalExited(false);
    setShellSession(null);
  }, [project?.root]);

  /** 고른 것을 이 창에서 연다. 코드 히트는 LSP 점프와 같은 착지 경로를 쓴다. */
  const openFromQuickOpen = (item: RankedQuickOpenItem) => {
    setQuickOpen(null);
    if (item.scope === "file") {
      void files.openFile(item.id);
      return;
    }
    if (item.scope === "code") {
      const at = parseCodeItemId(item.id);
      if (!at) return;
      void files.openFile(at.path).then((opened) => {
        if (opened) actions.setRevealTarget({ path: at.path, line: at.line, column: at.column });
      });
    }
  };

  /** ⌘N — 새 파일, ⇧⌘N — 새 폴더. 자리는 지금 보고 있는 파일의 폴더(없으면 루트). */
  const newItemRef = useRef({ dir: "", canMutate: false, busy: false, startNew: treeOps.startNew });
  newItemRef.current = {
    // 키가 아니라 실경로다 — diff 탭이 활성이어도 새 파일은 그 파일의 폴더에 만든다.
    dir: targetDir(files.activeFile == null ? null : { path: files.activeFile.path, is_dir: false }),
    canMutate: session?.supports_lsp === true,
    busy: treeOps.promptProps.open,
    startNew: treeOps.startNew,
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.repeat) return;
      if (!(e.metaKey || e.ctrlKey) || e.altKey || e.code !== "KeyN") return;
      const now = newItemRef.current;
      if (!now.canMutate || now.busy) return;
      e.preventDefault();
      now.startNew(e.shiftKey ? "dir" : "file", now.dir);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  /**
   * Python 콘솔 실행 경로. 도크가 아직 열리지 않았으면(첫 ⇧⏎·"IPython으로 열기") 코드를 맡아
   * 두었다가 세션이 붙은 뒤 흘려보낸다 — 백엔드 큐는 슬롯이 있어야 받으므로, 슬롯이 생기기
   * 전의 입력은 이 창이 들고 있어야 한다.
   */
  const replOpenedRef = useRef(false);
  const pendingRunRef = useRef<string[]>([]);
  const runInRepl = useCallback((code: string) => {
    const current = sessionRef.current;
    if (!current || current.host !== LOCAL_HOST) return;
    setReplDock(true);
    if (replOpenedRef.current) {
      void replRun(current.task_id, code).catch((e) => setError(String(e)));
      return;
    }
    pendingRunRef.current.push(code);
  }, []);
  const onReplOpened = useCallback(() => {
    const current = sessionRef.current;
    if (!current) return;
    replOpenedRef.current = true;
    const queued = pendingRunRef.current.splice(0);
    for (const code of queued) void replRun(current.task_id, code).catch((e) => setError(String(e)));
  }, []);
  const closeReplDock = useCallback(() => {
    // 화면만 닫는다 — 콘솔 프로세스는 남아 다음 ⌃`에 스크롤백째 돌아온다(워크스페이스 셸과 같다).
    setReplDock(false);
    replOpenedRef.current = false;
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.ctrlKey || e.metaKey || e.altKey || e.shiftKey || e.code !== "Backquote") return;
      e.preventDefault();
      if (workspaceRef.current && isProjectWorkspace(workspaceRef.current)) {
        setTerminalStarted(true);
        setTerminalOpen((open) => !open);
        return;
      }
      setReplDock((open) => {
        if (open) replOpenedRef.current = false;
        return !open;
      });
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  // 세션이 바뀌면 이전 작업의 콘솔은 갈 곳이 없다 — 프로세스까지 닫는다.
  const replTaskRef = useRef<number | null>(null);
  useEffect(() => {
    const prev = replTaskRef.current;
    const next = session?.host === LOCAL_HOST ? session.task_id : null;
    if (prev != null && prev !== next) void replClose(prev).catch(() => {});
    if (prev !== next) {
      setReplDock(false);
      replOpenedRef.current = false;
      pendingRunRef.current = [];
    }
    replTaskRef.current = next;
  }, [session?.host, session?.task_id]);

  useEffect(() => {
    fontSettingsGet()
      .then((s) => {
        document.documentElement.style.setProperty("--font-ui", uiFontStack(s.ui_family));
        document.documentElement.style.setProperty("--font-code", codeFontStack(s.code_family));
        document.documentElement.style.setProperty("--font-ui-size", `${s.ui_size}px`);
        setFontSettings(s);
      })
      .catch(() => {});
  }, []);

  // 에디터 설정 — 메인 창과 별개의 웹뷰라 CSS 변수를 여기서 따로 걸어야 한다.
  // 창을 띄운 뒤 메인에서 값을 바꾸면 다음 팝아웃부터 반영된다.
  useEffect(() => {
    editorSettingsGet()
      .then((s) => {
        const next = normalizeEditorSettings(s);
        applyTreeMetrics(document.documentElement, next.tree_font_size);
        setEditorSettings(next);
      })
      .catch(() => {});
  }, []);

  // 세션 전환·창 닫기 직전에 dirty를 비운다. 최신 값을 ref로 잡아 두는 이유는 리스너를
  // 한 번만 등록하기 위해서다 — 매 렌더 재등록하면 이벤트를 놓치는 창이 생긴다.
  const flushRef = useRef(files.flushDirty);
  flushRef.current = files.flushDirty;
  const sessionRequestRef = useRef(0);
  const transitionRef = useRef(Promise.resolve());
  const closingRef = useRef<Promise<boolean> | null>(null);
  // 리스너는 한 번만 등록하므로(재등록하면 이벤트를 놓친다) 그 안에서 쓰는 콜백은 ref로
  // 최신을 잡아 둔다 — `flushRef`와 같은 이유. `reloadFile`이 여기 있는 것도 그래서다:
  // 직접 캡처하면 첫 렌더(세션 없음 → taskId null) 버전에 묶여 영영 아무것도 하지 않는다.
  const liveRef = useRef({
    openFile: files.openFile,
    openDiff: files.openDiff,
    reloadFile: files.reloadFile,
    revealAt: actions.revealAt,
  });
  liveRef.current = {
    openFile: files.openFile,
    openDiff: files.openDiff,
    reloadFile: files.reloadFile,
    revealAt: actions.revealAt,
  };

  /** 세션 전환을 기다리는 링크 한 건. 저장이 막혀 창이 옛 세션에 머무는 동안 온 것으로,
   *  전환이 성사되면 그때 연다. 최신 하나만 남긴다 — 밀린 링크가 우르르 열릴 자리는 없다. */
  const pendingRevealRef = useRef<EditorRevealPayload | null>(null);
  /** 링크가 활성 탭을 가져갔다. 뒤늦게 끝나는 세션 복원 루프가 그것을 되돌리지 않게 한다. */
  const revealClaimedRef = useRef(false);

  /** 링크 한 건을 연다 — 탭을 띄우고, 열렸을 때만 그 줄에 착지시킨다. */
  const applyReveal = useCallback(async (payload: EditorRevealPayload) => {
    const current = sessionRef.current;
    if (!current || current.task_id !== payload.task_id || current.host !== payload.host) return;
    revealClaimedRef.current = true;
    // 변경 목록의 행이 보낸 링크 — diff 탭은 읽을 파일이 없으니 바로 만들고 끝낸다.
    if (payload.kind === "diff") {
      liveRef.current.openDiff(payload.path, { preview: true });
      return;
    }
    // 열지 못한 파일(에이전트가 지운 경로 등)에 착지 지점을 걸면 소비되지 않고 남아,
    // 나중에 같은 경로를 열 때 커서가 난데없이 뛴다.
    if (!(await liveRef.current.openFile(payload.path))) return;
    const latest = sessionRef.current;
    if (!latest || latest.task_id !== payload.task_id || latest.host !== payload.host) return;
    liveRef.current.revealAt(payload.path, payload.line, payload.column);
  }, []);

  /** 남은 dirty를 저장한다. 저장하지 못하면 false — 세션을 갈아타지도, 창을 닫지도 않는다. */
  const flushBeforeLeaving = useCallback(async (taskId: number | null): Promise<boolean> => {
    const result = await flushRef.current();
    if (!result.ok) {
      void emit(EDITOR_AUTOSAVE_BLOCKED_EVENT, {
        task_id: taskId,
        path: result.path,
        reason: result.reason,
        detail: result.detail,
      }).catch(() => {});
      setError(
        result.reason === "conflict"
          ? `${result.path} 가 디스크에서 바뀌어 저장하지 못했습니다. 확인 후 다시 시도하세요.`
          : `${result.path} 를 저장하지 못했습니다: ${result.detail ?? "알 수 없는 오류"}`,
      );
      // 시야 밖에서 막혀 있으면 알 길이 없다. 창을 앞으로 가져와 사용자가 보게 한다.
      void getCurrentWindow().setFocus().catch(() => {});
      return false;
    }
    if (taskId != null && result.entries.length > 0) {
      void emit(EDITOR_AUTOSAVED_EVENT, { task_id: taskId, entries: result.entries }).catch(
        () => {},
      );
    }
    return true;
  }, []);

  useEffect(() => {
    const unSession = listen<EditorWorkspacePayload>(EDITOR_SESSION_EVENT, ({ payload }) => {
      const request = ++sessionRequestRef.current;
      if (
        pendingRevealRef.current &&
        (isProjectWorkspace(payload) || pendingRevealRef.current.task_id !== payload.task_id || pendingRevealRef.current.host !== payload.host)
      ) pendingRevealRef.current = null;
      // 저장과 루트 배정을 직렬화한다. 전환 도중 온 오래된 요청은 다음 루트를 덮지 않는다.
      transitionRef.current = transitionRef.current.then(async () => {
        if (request !== sessionRequestRef.current) return;
        const current = workspaceRef.current;
        const same = current && (isProjectWorkspace(current)
          ? isProjectWorkspace(payload) && current.root === payload.root
          : !isProjectWorkspace(payload) && current.task_id === payload.task_id && current.host === payload.host);
        setSwitching(true);
        try {
          if (current && !same && !(await flushBeforeLeaving(isProjectWorkspace(current) ? null : current.task_id))) return;
          if (request !== sessionRequestRef.current) return;
          await editorProjectActivate(isProjectWorkspace(payload) ? payload.root : null);
          if (request !== sessionRequestRef.current) return;
          workspaceRef.current = payload;
          setWorkspace(payload);
          setError(null);
          setNotice(null);
        } catch (cause) {
          setError(String(cause));
        } finally {
          if (request === sessionRequestRef.current) setSwitching(false);
        }
      });
    });
    const unStatus = listen<EditorStatus>(EDITOR_STATUS_EVENT, ({ payload }) => setStatus(payload));
    // 메인 창 대화에서 클릭한 파일 링크. 창이 나가 있는 동안 링크는 전부 이리로 온다.
    const unReveal = listen<EditorRevealPayload>(EDITOR_REVEAL_EVENT, ({ payload }) => {
      const current = sessionRef.current;
      // 아직 세션이 없거나(창이 부팅 중) 옛 세션에 머무는 중이면(저장이 막혔거나 flush 진행 중)
      // 같은 상대 경로가 다른 워크트리를 가리킨다. 버리지 않고 맡아 둔다 — 메인 창은 이미
      // 배달됐다고 보고 폴백하지 않으므로, 여기서 버리면 클릭이 통째로 사라진다.
      if (!current || current.task_id !== payload.task_id || current.host !== payload.host) {
        pendingRevealRef.current = payload;
        return;
      }
      void applyReveal(payload);
    });
    // 메인 창이 되돌렸으면 디스크가 바뀌었다 — 열려 있는 탭을 다시 읽는다.
    const unReverted = listen<string[]>(EDITOR_REVERTED_EVENT, ({ payload }) => {
      for (const path of payload) void liveRef.current.reloadFile(path);
    });
    // 준비됐음을 알리면 메인 창이 현재 세션을 보내 준다. 창이 먼저 뜨고 세션이 나중에 온다.
    void emit(EDITOR_READY_EVENT).catch(() => {});
    return () => {
      void unSession.then((f) => f()).catch(() => {});
      void unStatus.then((f) => f()).catch(() => {});
      void unReveal.then((f) => f()).catch(() => {});
      void unReverted.then((f) => f()).catch(() => {});
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [flushBeforeLeaving, applyReveal]);

  // 세션이 바뀌면 트리를 다시 읽고, 메인 창이 열어 두었던 파일을 이어받는다.
  // 워크트리가 달라지므로 이전 목록은 의미가 없다.
  useEffect(() => {
    if (!workspace) return;
    let cancelled = false;
    setRestoredProject(null);
    setTreeLens("files");
    revealClaimedRef.current = false;
    files.refreshTree();
    void (async () => {
      // 목록은 탭 키다 — `diff:<path>`는 읽을 파일 없이 바로 만들고, 파일 탭은 읽어서 연다.
      for (const raw of workspace.open_paths) {
        if (cancelled) return;
        const key = asTabKey(raw);
        if (isDiffKey(key)) files.openDiff(pathFromTabKey(key), { preview: true });
        else await files.openFile(key);
        if (cancelled) return;
      }
      if (cancelled) return;
      // 복원은 느리다(파일마다 읽기). 그 사이 링크가 활성 탭을 가져갔으면 되돌리지 않는다 —
      // 되돌리면 링크로 연 파일이 배경 탭으로만 남아 클릭이 안 먹은 것처럼 보인다.
      if (workspace.active_path && !revealClaimedRef.current)
        files.setActiveKey(asTabKey(workspace.active_path));
      if (isProjectWorkspace(workspace)) setRestoredProject(workspace.root);
      // 이 세션을 기다리던 링크가 있으면 이제 연다.
      const pending = pendingRevealRef.current;
      if (session && pending && pending.task_id === session.task_id && pending.host === session.host) {
        pendingRevealRef.current = null;
        await applyReveal(pending);
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceKey]);

  // 열린 목록을 변경 시마다 남긴다. editor://closed는 정상 닫기에서만 오므로,
  // 크래시·강제 종료에서 목록을 지키는 것은 이 저장뿐이다.
  // 탭 키를 남긴다 — diff 탭도 `diff:<path>`로 돌아온다. 프로젝트 탭 저장만 실경로다(diff 없음).
  const openPaths = files.openFiles.map((f) => f.key).join("\n");
  const activeTabKey = files.activeKey ?? null;
  useEffect(() => {
    if (project) {
      if (restoredProject === project.root) saveProjectTabs(project.root,
        files.openFiles.filter((file) => file.kind !== "diff").map((file) => file.path), activeFilePath);
      return;
    }
    if (!session || session.host !== LOCAL_HOST) return;
    void editorWindowFilesSave(
      session.task_id,
      openPaths ? openPaths.split("\n") : [],
      activeTabKey,
    ).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceKey, restoredProject, openPaths, activeFilePath, activeTabKey, files.openFiles]);

  // 창을 닫으면 파일 목록과 함께 메인 창으로 돌아간다. 실제로 닫지 않고 숨기므로
  // (메인 창이 editor_window_hide를 부른다) 다음 팝아웃이 즉시 뜬다.
  const closeStateRef = useRef({ workspace, session, terminalStarted, openFiles: files.openFiles, activeKey: activeTabKey });
  closeStateRef.current = { workspace, session, terminalStarted, openFiles: files.openFiles, activeKey: activeTabKey };
  const requestPopIn = useCallback(async (notification?: EditorNotificationPayload): Promise<boolean> => {
    if (closingRef.current) return closingRef.current;
    const request = ++sessionRequestRef.current;
    const closing = transitionRef.current.then(async (): Promise<boolean> => {
      if (request !== sessionRequestRef.current) return false;
      setSwitching(true);
      const current = closeStateRef.current;
      if (!current.workspace) {
        try {
          await editorProjectActivate(null);
          if (request !== sessionRequestRef.current) return false;
          await emit(EDITOR_GONE_EVENT);
          return true;
        } catch (cause) { setError(String(cause)); return false; }
      }
      if (current.workspace && isProjectWorkspace(current.workspace)) {
        if (!(await flushBeforeLeaving(null))) return false;
        if (request !== sessionRequestRef.current) return false;
        if (current.terminalStarted && !window.confirm("실행 중인 프로젝트 셸을 종료하고 창을 닫을까요?")) return false;
        try {
          await editorProjectActivate(null);
          if (request !== sessionRequestRef.current) return false;
          await emit(EDITOR_CLOSED_EVENT, { kind: "project", root: current.workspace.root });
          void emit(PROJECT_EDITOR_CLOSED_EVENT, current.workspace.root).catch(() => {});
          workspaceRef.current = null;
          setWorkspace(null);
          return true;
        } catch (cause) {
          setError(`코드 창을 접지 못했습니다: ${String(cause)}`);
          return false;
        }
      }
      if (!current.session) return false;
      if (!(await flushBeforeLeaving(current.session.task_id))) return false;
      if (request !== sessionRequestRef.current) return false;
      // 팝인하면 이 창의 콘솔을 볼 자리가 없다 — 프로세스도 함께 닫는다.
      if (current.session.host === LOCAL_HOST) {
        void replClose(current.session.task_id).catch(() => {});
        replTaskRef.current = null;
        replOpenedRef.current = false;
        pendingRunRef.current = [];
        setReplDock(false);
      }
      try {
        await emit(EDITOR_CLOSED_EVENT, {
          task_id: current.session.task_id,
          host: current.session.host,
          // 탭 키 — 메인 창이 diff 탭까지 되살린다(ADR 2026-10-08 결정 5).
          open_paths: current.openFiles.map((file) => file.key),
          active_path: current.activeKey,
          ...(notification ? { notification } : {}),
        });
        return true;
      } catch (reason) {
        setError(`코드 창을 접지 못했습니다: ${String(reason)}`);
        return false;
      }
    });
    closingRef.current = closing;
    transitionRef.current = closing.then(() => {});
    try { return await closing; }
    finally {
      closingRef.current = null;
      if (request === sessionRequestRef.current) setSwitching(false);
    }
  }, [flushBeforeLeaving]);
  useEffect(() => {
    const window = getCurrentWindow();
    const unClose = window.onCloseRequested((e) => {
      // 어느 경우든 실제로 닫지 않는다. preventDefault 없이 돌아가면 Tauri가 창을 destroy하고,
      // 부팅 때 한 번 만든 창이라 그 뒤 메인 창의 모든 호출이 "에디터 창을 찾을 수 없습니다"로 죽는다.
      e.preventDefault();
      // 닫지 않고 팝인으로 돌린다. 저장하지 못하면 그대로 남는다 — 창이 사라진 뒤에는
      // 저장 못 한 변경이 있다는 것을 알릴 자리가 없다.
      void requestPopIn();
    });
    // 자리는 이동·리사이즈가 멎은 뒤에 남긴다 — 드래그 중 매 프레임 쓰면 DB가 갈린다.
    let timer: number | undefined;
    const remember = () => {
      globalThis.clearTimeout(timer);
      timer = globalThis.setTimeout(() => void editorWindowGeometrySave().catch(() => {}), 400);
    };
    const unMoved = window.onMoved(remember);
    const unResized = window.onResized(remember);
    return () => {
      globalThis.clearTimeout(timer);
      void unClose.then((f) => f()).catch(() => {});
      void unMoved.then((f) => f()).catch(() => {});
      void unResized.then((f) => f()).catch(() => {});
    };
  }, [requestPopIn]);

  const restartShell = async () => {
    if (!project || restartingRef.current) return;
    restartingRef.current = true;
    try {
      await restartProjectShell(shellSession, project.root);
      if (workspaceRef.current !== workspace) return;
      setError(null);
      setTerminalExited(false);
      setTerminalRestart((value) => value + 1);
    } catch (cause) { setError(String(cause)); }
    finally { restartingRef.current = false; }
  };

  if (!workspace) {
    return (
      <div className="h-screen grid place-items-center text-sm text-text-muted">
        {error ?? "세션을 기다리는 중…"}
      </div>
    );
  }

  const lsp = project != null || session?.supports_lsp === true;

  return (
    <div className="h-screen flex flex-col bg-base text-text">
      {project && <header className="h-9 shrink-0 flex items-center gap-2 px-3 border-b border-border text-xs text-text-muted">
        <Icon name="folder" size={14} /><span className="truncate">{project.root}</span>
        <button className="ml-auto p-1 hover:text-text" aria-label={terminalOpen ? "터미널 닫기" : "터미널 열기"} aria-pressed={terminalOpen} onClick={() => { setTerminalStarted(true); setTerminalOpen((open) => !open); }}><Icon name="terminal" size={14} /></button>
      </header>}
      {/* diff 스냅샷·확인함은 세션당 하나다 — 변경 목록(트리 열)과 diff 탭(에디터)이 같은 값을 본다. */}
      <DiffSessionScope task={task} openDiff={files.openDiff}>
      <div className="flex-1 flex min-h-0" inert={switching} aria-busy={switching}>
        {!treeOpen && (
          <div className="w-8 shrink-0 border-r border-border flex flex-col items-center pt-1">
            <button
              className="p-1 text-text-secondary hover:text-text"
              onClick={() => setTreeOpen(true)}
              title="파일 트리 열기"
              aria-label="파일 트리 열기"
            >
              <Icon name="folder" size={14} />
            </button>
          </div>
        )}
        {/* 접을 때 언마운트하지 않는다 — 펼친 가지가 FileTree의 상태라 다시 열면 처음으로
            되돌아간다. 깊은 트리에서 그것은 접기를 쓰지 않게 만드는 이유가 된다. */}
        <div className={`w-64 shrink-0 flex-col border-r border-border ${treeOpen ? "flex" : "hidden"}`}>
          <div className="h-8 shrink-0 px-2 flex items-center gap-1 text-xs text-text-muted">
            {/* 세션 창은 메인 창과 같은 렌즈 토글을 갖는다 — 변경 목록은 이 열의 둘째 얼굴이다.
                프로젝트 창은 diff 세션이 없으므로 파일 트리뿐이다. */}
            {session ? (
              <div className="flex items-center gap-0.5 min-w-0" role="tablist" aria-label="트리 열 렌즈">
                {([["files", "파일"], ["changes", "변경"]] as const).map(([lens, label]) => (
                  <button
                    key={lens}
                    role="tab"
                    aria-selected={treeLens === lens}
                    aria-pressed={treeLens === lens}
                    className={`h-6 px-1.5 rounded ${
                      treeLens === lens ? "bg-raised text-text" : "hover:text-text"
                    }`}
                    onClick={() => setTreeLens(lens)}
                  >
                    {label}
                  </button>
                ))}
              </div>
            ) : (
              <span className="truncate">파일</span>
            )}
            <span className="ml-auto" />
            {treeLens === "files" && (<>
            <button
              className={`p-1 ${
                showHidden ? "text-primary-bright" : "hover:text-text"
              }`}
              onClick={() => setShowHidden((v) => !v)}
              title={showHidden ? "숨김 항목 감추기" : "숨김 항목 보기 (.*)"}
              aria-label="숨김 항목"
              aria-pressed={showHidden}
            >
              <Icon name={showHidden ? "eye" : "eyeOff"} size={13} />
            </button>
            <button
              className="p-1 hover:text-text"
              onClick={() => files.refreshTree()}
              title="새로고침"
              aria-label="파일 트리 새로고침"
            >
              <Icon name="refresh" size={13} />
            </button>
            </>)}
            <button
              className="p-1 hover:text-text"
              onClick={() => setTreeOpen(false)}
              title="파일 트리 닫기"
              aria-label="파일 트리 닫기"
            >
              <Icon name="x" size={13} />
            </button>
          </div>
          {session && treeLens === "changes" ? (
            <ChangesList />
          ) : (
          <div className="flex flex-1 flex-col overflow-auto">
            <FileTree
              nodes={files.tree}
              activePath={files.activeFile?.path ?? null}
              onOpen={(path) => void files.openFile(path, { preview: true, tree: true })}
              onPin={(path) => files.pinTab(fileTabKey(path))}
              onContextMenu={treeOps.openMenu}
              showHidden={showHidden}
            />
          </div>
          )}
        </div>

        {/* Monaco는 퍼센트 높이 체인이라, 부모가 flex 컨테이너가 아니면 EditorPane의 flex-1이
            무효가 돼 0px로 붕괴한다(마크다운 프리뷰만 콘텐츠 높이로 살아남는다). */}
        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          <EditorSplitView
            taskId={session?.task_id ?? null}
            readHtmlAsset={source?.read}
            host={task?.host ?? LOCAL_HOST}
            rootPath={project?.root ?? session?.worktree_path ?? null}
            windowId="editor"
            sourceChangeRef={workspaceSourceChangeRef}
            // 이 창은 통째로 에디터다 — 단축키를 가려 받을 다른 면이 없다.
            // 마지막 파일까지 닫힌 뒤의 ⌘W는 창을 접는다(onCloseRequested가 팝인으로 돌린다).
            onCloseExhausted={() => void getCurrentWindow().close()}
            // 세션 목록이 없는 창이라 ⌘1‥⌘9를 두고 겨룰 상대도 없다.
            ownsWindow
            files={files.openFiles}
            activeKey={files.activeKey}
            treeOpen={files.treeOpen}
            onTreeOpenHandled={files.consumeTreeOpen}
            dark={theme.kind !== "light"}
            onSelect={files.setActiveKey}
            onClose={files.closeTab}
            onBeforeClose={project ? (key) => {
              const file = files.openFiles.find((item) => item.key === key);
              return !file?.dirty || window.confirm(`${file.path}에 저장하지 않은 변경이 있습니다. 버리시겠습니까?`);
            } : undefined}
            onOpenFile={files.openFile}
            readFile={files.readFile}
            onChange={files.changeFile}
            onSave={(path, content) => void files.saveFile(path, content)}
            onReload={(path) => void files.reloadFile(path)}
            onOpenPath={(path) => void openPathExternal(path)}
            onRevealPath={(path) => void revealPathInFinder(path)}
            onCopyAbsPath={(path) => void copyAbsPath(path)}
            onPinTab={files.pinTab}
            supportsExternalPath={project != null || lsp}
            // 콘솔은 로컬 워크트리에서만 뜬다 — 원격은 PTY를 붙일 프로세스가 이쪽에 없다.
            onRunSelection={session?.host === LOCAL_HOST ? runInRepl : undefined}
            onOpenInRepl={
              session?.host === LOCAL_HOST && session.worktree_path
                ? (path) => runInRepl(pandasOpenSnippet(session.worktree_path ?? "", path))
                : undefined
            }
            codeFontFamily={fontSettings ? codeFontStack(fontSettings.code_family) : undefined}
            codeFontSize={fontSettings?.code_size}
            editorSettings={editorSettings}
            onGoto={lsp ? actions.gotoSymbol : undefined}
            onLspStatus={lsp ? actions.askLspStatus : undefined}
            onSemanticTokens={lsp ? actions.askSemanticTokens : undefined}
            lspScopeKey={workspaceKey ?? undefined}
            onOpenTarget={actions.openLspTarget}
            reveal={actions.revealTarget}
            onRevealed={() => actions.setRevealTarget(null)}
            onNavigationError={setError}
            onAttachCapture={session ? (record) => {
              // 메인·팝아웃의 seq가 둘 다 0부터라 id가 겹칠 수 있다 — 팝아웃 몫에 w 스코프를 섞는다.
              const scoped = { ...record, id: scopeLocalCaptureId(record.id) };
              void emit(EDITOR_CAPTURE_EVENT, scoped)
                // 문구에 목적지를 박는다 — 플러시가 막혀 창이 옛 세션에 머무는 구간에는 칩이
                // 화면 어디에도 보이지 않으므로, 어디로 갔는지는 이 문구가 말해야 한다.
                .then(() => showNotice(`세션 #${record.task_id}에 첨부됨 ✓`))
                .catch((e) => setError(String(e)));
            } : undefined}
            askBusy={status === "busy"}
            onAsk={session ? (input) => {
              const payload = buildAskPayload({ taskId: session.task_id, ...input });
              if (!payload) return;
              void emit(EDITOR_ASK_EVENT, payload).catch((e) => setError(String(e)));
            } : undefined}
          />
          {project && terminalStarted && <section className={`h-64 shrink-0 border-t border-border flex flex-col ${terminalOpen ? "" : "hidden"}`}>
            <div className="h-8 px-3 flex items-center gap-2 text-xs text-text-muted">터미널{terminalExited && <button className="ml-auto text-primary-bright" onClick={() => void restartShell()}>재시작</button>}</div>
            <ProjectTerminal key={project.root} root={project.root} hidden={!terminalOpen} restart={terminalRestart} onSession={setShellSession} onExit={setTerminalExited} onError={setError} />
          </section>}
        </div>
      </div>
      </DiffSessionScope>

      {session && replDock && (
        <ReplDock
          taskId={session.task_id}
          available={session.host === LOCAL_HOST}
          label={session.branch ?? session.worktree_path ?? undefined}
          codeFontFamily={fontSettings ? codeFontStack(fontSettings.code_family) : undefined}
          codeFontSize={fontSettings?.code_size}
          onOpened={onReplOpened}
          onClose={closeReplDock}
        />
      )}
      {session ? <StatusBar session={session} status={status} error={error} notice={notice} /> : error && <div className="shrink-0 border-t border-border px-3 py-1 text-xs text-danger" role="alert">{error}</div>}
      <ContextMenuShell
        at={treeOps.menu}
        ariaLabel={treeOps.menu?.node ? `${treeOps.menu.node.name} 조작` : "워크트리 루트 조작"}
        header={treeOps.menu?.node?.name ?? "워크트리 루트"}
        rows={treeOps.rows}
        onClose={treeOps.closeMenu}
        resetKey={treeOps.menu?.node?.path ?? ""}
      />
      <FilePromptDialog {...treeOps.promptProps} />
      <QuickOpen
        open={quickOpen != null}
        initialEditorTab={quickOpen ?? "all"}
        scopes={project ? ["file"] : ["file", "code"]}
        editorSearch={{
          scopeLabel: project?.root ?? `${session?.branch ?? "현재 워크트리"} · ${session?.host} · 세션 #${session?.task_id}`,
          contentAvailable: session?.host === LOCAL_HOST,
        }}
        files={workspaceFiles}
        // 스킬은 이 창에서 열 수 없다 — 빈 문자열이면 조회 자체를 생략한다.
        repo=""
        taskId={session?.task_id ?? null}
        onClose={() => setQuickOpen(null)}
        onSelect={openFromQuickOpen}
      />
    </div>
  );
}
