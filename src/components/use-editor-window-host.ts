import { emitTo, listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  EDITOR_ASK_EVENT,
  EDITOR_AUTOSAVED_EVENT,
  EDITOR_AUTOSAVE_BLOCKED_EVENT,
  EDITOR_CAPTURE_EVENT,
  EDITOR_CLOSED_EVENT,
  EDITOR_GONE_EVENT,
  EDITOR_READY_EVENT,
  EDITOR_REVEAL_EVENT,
  EDITOR_SESSION_EVENT,
  EDITOR_STATUS_EVENT,
  EDITOR_WINDOW_LABEL,
  type EditorAskPayload,
  type EditorCapturePayload,
  type EditorAutosaveBlockedPayload,
  type EditorAutosavedPayload,
  type EditorClosedPayload,
  type EditorNotificationPayload,
  type EditorRevealPayload,
  type EditorSessionPayload,
  type EditorStatus,
  type EditorWindowClosedPayload,
} from "../lib/editor-window-events";
import {
  editorWindowAlive,
  editorWindowFilesLoad,
  editorWindowFocus,
  editorWindowHide,
  editorWindowOpen,
  type EditorWindowFiles,
} from "../lib/ipc";
import { editorProjectPrepare } from "../lib/project-editor-ipc";
import { loadProjectTabs } from "../lib/project-editor-tabs";

interface Options {
  /** 팝아웃 대상 세션. null이면 뺄 것이 없다. */
  taskId: number | null;
  /** 작업이 사는 호스트 — 팝아웃 창이 원격 파일을 그 호스트에서 읽게 한다. */
  host: string;
  branch: string | null;
  /** 문서 링크의 절대 경로를 루트 기준으로 풀 때 창에 실어 보낸다. */
  worktreePath: string | null;
  /** 원격 워크트리는 언어 서버를 띄울 수 없다. */
  supportsLsp: boolean;
  /**
   * 메인 창이 열어 두었던 탭의 **키**(`tab-key.ts`) — 팝아웃 시 창이 이어받는다.
   * diff 탭은 `diff:<path>`로 들어가고 저쪽 창이 그대로 되살린다(ADR 2026-10-08 결정 5).
   * 이 훅은 목록을 해석하지 않는다 — 문자열 배열을 실어 보낼 뿐이다.
   */
  openPaths: string[];
  /** 활성 탭의 키. 없으면 `null` — 이 훅은 목록의 첫 항목으로 메우지 않는다(AC-10). */
  activePath: string | null;
  status: EditorStatus;
  /** 창이 닫혀 돌아왔다. 메인 창은 이 목록으로 에디터 탭을 되살린다. */
  onPopIn: (payload: EditorClosedPayload) => void | Promise<void>;
  /** 창이 조용히 디스크에 썼다 — 되돌릴 지점을 받아 바로 띄운다. */
  onAutosaved: (payload: EditorAutosavedPayload) => void;
  /** 버블에서 온 질문. 선택 코드와 질문을 합쳐 세션에 보낸다. */
  onAsk: (payload: EditorAskPayload) => void;
  /** ⌘L 선택 첨부가 창을 넘어왔다. 레코드화·store 반영은 메인 창(App)의 몫이다. */
  onCapture: (payload: EditorCapturePayload) => void;
  onNotification: (payload: EditorNotificationPayload) => void;
  onError: (message: string) => void;
}

export interface EditorWindowHost {
  poppedOut: boolean;
  popOut: () => Promise<void>;
  openProject: (root: string) => Promise<string>;
  /** 파일을 나가 있는 창에서 연다. **창에 넘겼으면 true** — 호출자는 이 창에서 열지 않는다.
   *
   *  팝인 상태이거나 창에 닿지 못했으면 false다. 그때는 호출자가 이 창에서 열어야 한다 —
   *  양쪽 다 하지 않으면 클릭이 아무 흔적도 남기지 않고 사라진다.
   *
   *  판단을 안으로 넣은 것은 의도적이다: 호출자가 `poppedOut`을 읽으면 그 값이 콜백 의존성에
   *  들어가 팝아웃 때마다 참조가 바뀐다. */
  revealFile: (payload: EditorRevealPayload) => Promise<boolean>;
}

/**
 * 메인 창 쪽에서 에디터 창을 붙잡고 있는 훅.
 *
 * 세션 정보는 메인 창만 안다(브랜치·transport 종류·열려 있던 파일). 그래서 창을 보이게 하는
 * 것은 Rust가 하고, 무엇을 보여 줄지는 여기서 `emitTo`로 보낸다.
 */
export function useEditorWindowHost({
  taskId,
  host,
  branch,
  worktreePath,
  supportsLsp,
  openPaths,
  activePath,
  status,
  onPopIn,
  onAutosaved,
  onAsk,
  onCapture,
  onNotification,
  onError,
}: Options): EditorWindowHost {
  const [poppedOut, setPoppedOut] = useState(false);
  const projectRef = useRef<string | null>(null);
  const openRequestRef = useRef(0);

  const sessionRef = useRef({ taskId, host, branch, worktreePath, supportsLsp, openPaths, activePath });
  sessionRef.current = { taskId, host, branch, worktreePath, supportsLsp, openPaths, activePath };
  const poppedOutRef = useRef(poppedOut);
  poppedOutRef.current = poppedOut;
  /** 팝아웃 여부를 바꾼다. ref를 함께 갱신하는 것이 핵심이다 — `revealFile`은 await 사이에
   *  이 값을 다시 보는데, 리렌더를 기다리면 그동안 닫힌 창을 focus로 되살려 버린다. */
  const changePoppedOut = useCallback((value: boolean) => {
    poppedOutRef.current = value;
    setPoppedOut(value);
  }, []);
  const onPopInRef = useRef(onPopIn);
  onPopInRef.current = onPopIn;
  const onAutosavedRef = useRef(onAutosaved);
  onAutosavedRef.current = onAutosaved;
  const onAskRef = useRef(onAsk);
  onAskRef.current = onAsk;
  const onCaptureRef = useRef(onCapture);
  onCaptureRef.current = onCapture;
  const onNotificationRef = useRef(onNotification);
  onNotificationRef.current = onNotification;
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  /** 지금 무엇을 열어야 하는지 창에 알린다. 세션이 없으면 보낼 것이 없다. */
  const publishSession = useCallback(async () => {
    const project = projectRef.current;
    if (project != null) {
      const saved = loadProjectTabs(project);
      await emitTo(EDITOR_WINDOW_LABEL, EDITOR_SESSION_EVENT, {
        kind: "project", root: project, open_paths: saved.paths, active_path: saved.active,
      });
      return;
    }
    const s = sessionRef.current;
    if (s.taskId == null) return;
    // 이전에 이 세션에서 열어 두었던 목록이 있으면 그것을 잇는다 — 창을 강제 종료했을 때
    // editor://closed가 오지 않아 메인 창의 목록이 비어 있을 수 있다.
    let openPathsToSend = s.openPaths;
    let activePathToSend = s.activePath;
    // 아무 탭도 없으면 저장된 목록으로 복원한다 — 창을 강제 종료했다가 다시 빼낼 때
    // 빈 창이 뜨는 것보다 지난 탭들(diff 탭 포함)이 돌아오는 편이 낫다.
    if (s.host === "local" && openPathsToSend.length === 0) {
      try {
        const saved = await editorWindowFilesLoad(s.taskId);
        openPathsToSend = saved.open_paths;
        activePathToSend = saved.active_path;
      } catch {
        // 목록을 못 읽는 것이 창을 못 여는 이유가 되어서는 안 된다.
      }
    }
    if (projectRef.current != null || sessionRef.current.taskId !== s.taskId || sessionRef.current.host !== s.host) return;
    const payload: EditorSessionPayload = {
      task_id: s.taskId,
      host: s.host,
      branch: s.branch,
      worktree_path: s.worktreePath,
      open_paths: openPathsToSend,
      active_path: activePathToSend,
      supports_lsp: s.supportsLsp,
    };
    await emitTo(EDITOR_WINDOW_LABEL, EDITOR_SESSION_EVENT, payload).catch(() => {});
  }, []);

  const openProject = useCallback(async (root: string) => {
    const request = ++openRequestRef.current;
    const selected = sessionRef.current;
    const info = await editorProjectPrepare(root);
    const current = () => request === openRequestRef.current &&
      selected.taskId === sessionRef.current.taskId && selected.host === sessionRef.current.host;
    if (!current()) return info.root;
    await editorWindowOpen();
    if (!current()) return info.root;
    projectRef.current = info.root;
    changePoppedOut(true);
    await publishSession();
    return info.root;
  }, [changePoppedOut, publishSession]);

  const popOut = useCallback(async () => {
    if (sessionRef.current.taskId == null) return;
    try {
      // 창을 보인 뒤에 탭을 지운다. 먼저 지우고 실패하면 에디터가 어디에도 없게 된다.
      await editorWindowOpen();
      projectRef.current = null;
      changePoppedOut(true);
      await publishSession();
    } catch (e) {
      onErrorRef.current(String(e));
    }
  }, [publishSession, changePoppedOut]);

  const revealFile = useCallback(async (payload: EditorRevealPayload) => {
    if (!poppedOutRef.current) return false;
    const session = sessionRef.current;
    if (session.taskId !== payload.task_id || session.host !== payload.host) return false;
    // 창을 먼저 앞으로 가져온다. 두 번째 모니터에 있거나 다른 앱에 가려 있으면 열어 봐야 안 보인다.
    try {
      await editorWindowFocus();
    } catch (e) {
      // 창이 죽었으면(destroy) focus는 언제나 실패한다. 그때 `poppedOut`을 두면 파일 탭이
      // 어디에도 없는 채 클릭마다 같은 오류만 난다 — 팝인으로 되돌리고 열기는 호출자에게 넘긴다.
      // 확인 자체가 실패하면 살아 있다고 본다(아래의 보수적 경로).
      if (!(await editorWindowAlive().catch(() => true))) {
        changePoppedOut(false);
        return false;
      }
      // 창은 있는데 focus만 실패했다. `poppedOut`은 건드리지 않는다 — Rust는 show() 다음에
      // set_focus()를 부르므로(`commands.rs`) 창이 화면에 뜬 채 focus만 실패할 수 있고, 그때
      // 팝인으로 뒤집으면 같은 세션의 에디터가 두 곳에 산다. 열기는 호출자에게 돌려준다.
      onErrorRef.current(`코드 창을 앞으로 가져오지 못했습니다 — ${String(e)}`);
      return false;
    }
    // focus를 기다리는 사이 창이 닫혔을 수 있다(editor://closed). 숨긴 창을 되살리지 않는다.
    if (
      !poppedOutRef.current ||
      sessionRef.current.taskId !== payload.task_id ||
      sessionRef.current.host !== payload.host
    ) return false;
    try {
      await emitTo(EDITOR_WINDOW_LABEL, EDITOR_REVEAL_EVENT, payload);
    } catch (e) {
      // 세션·상태 이벤트와 달리 이건 재생되지 않는다 — `editor://ready` 핸드셰이크가 다시
      // 보내 주지 않으므로, 삼키면 클릭 한 번이 통째로 사라진다.
      onErrorRef.current(`코드 창에 링크를 전달하지 못했습니다 — ${String(e)}`);
      return false;
    }
    return true;
  }, [changePoppedOut]);

  useEffect(() => {
    const unReady = listen(EDITOR_READY_EVENT, () => {
      if (poppedOutRef.current) void publishSession();
    });
    const unClosed = listen<EditorWindowClosedPayload>(EDITOR_CLOSED_EVENT, ({ payload }) => {
      void (async () => {
        try {
          await editorWindowHide();
        } catch (reason) {
          // 창이 이미 죽었으면 숨길 것이 없다 — 그래도 접힘 처리는 끝내야 상태가 풀린다.
          if (await editorWindowAlive().catch(() => true)) {
            onErrorRef.current(`코드 창을 접지 못했습니다 — ${String(reason)}`);
            return;
          }
        }
        changePoppedOut(false);
        projectRef.current = null;
        if (!("task_id" in payload)) return;
        const session = sessionRef.current;
        if (session.taskId !== payload.task_id || session.host !== payload.host) return;
        // 알림 이동은 이전 파일 복원과 동시에 시작하면 새 작업 선택을 덮을 수 있다.
        // 저장은 이미 팝아웃 창에서 끝났으므로 알림은 복원 없이 메인으로 넘긴다.
        if (payload.notification) onNotificationRef.current(payload.notification);
        else await onPopInRef.current(payload);
      })();
    });
    // 창이 접힌 것이 아니라 사라졌다(Rust의 Destroyed, 또는 세션 없는 창의 닫기). 파일 목록이
    // 함께 오지 않으므로 로컬 세션은 편집 때마다 DB에 남긴 목록으로 되살린다.
    const unGone = listen(EDITOR_GONE_EVENT, () => {
      void (async () => {
        // 죽은 창이면 hide는 실패한다 — 그것이 여기 온 이유이지 멈출 이유가 아니다.
        await editorWindowHide().catch(() => {});
        if (!poppedOutRef.current) return;
        changePoppedOut(false);
        if (projectRef.current != null) { projectRef.current = null; return; }
        const session = sessionRef.current;
        if (session.taskId == null) return;
        let files: EditorWindowFiles = { open_paths: [], active_path: null };
        if (session.host === "local") {
          try {
            files = await editorWindowFilesLoad(session.taskId);
          } catch {
            // 목록을 못 읽어도 팝인은 끝나야 한다 — 탭이 비어 돌아오는 편이 굳는 것보다 낫다.
          }
        }
        if (sessionRef.current.taskId !== session.taskId || sessionRef.current.host !== session.host) return;
        await onPopInRef.current({
          task_id: session.taskId,
          host: session.host,
          open_paths: files.open_paths,
          active_path: files.active_path,
        });
      })();
    });
    const unAutosaved = listen<EditorAutosavedPayload>(EDITOR_AUTOSAVED_EVENT, ({ payload }) => {
      onAutosavedRef.current(payload);
    });
    // 저장하지 못해 창이 이전 세션에 머물렀다. 창은 스스로 앞으로 나오지만,
    // 메인 창에서도 왜 갈아타지 않았는지 보여야 한다.
    const unBlocked = listen<EditorAutosaveBlockedPayload>(
      EDITOR_AUTOSAVE_BLOCKED_EVENT,
      ({ payload }) => {
        onErrorRef.current(
          payload.reason === "conflict"
            ? `코드 창: ${payload.path} 가 디스크에서 바뀌어 저장하지 못했습니다`
            : `코드 창: ${payload.path} 저장 실패 — ${payload.detail ?? "알 수 없는 오류"}`,
        );
      },
    );
    const unAsk = listen<EditorAskPayload>(EDITOR_ASK_EVENT, ({ payload }) => {
      onAskRef.current(payload);
    });
    const unCapture = listen<EditorCapturePayload>(EDITOR_CAPTURE_EVENT, ({ payload }) => {
      onCaptureRef.current(payload);
    });
    return () => {
      void unReady.then((f) => f()).catch(() => {});
      void unClosed.then((f) => f()).catch(() => {});
      void unGone.then((f) => f()).catch(() => {});
      void unAutosaved.then((f) => f()).catch(() => {});
      void unBlocked.then((f) => f()).catch(() => {});
      void unAsk.then((f) => f()).catch(() => {});
      void unCapture.then((f) => f()).catch(() => {});
    };
  }, [publishSession, changePoppedOut]);

  // 세션이 바뀌면 창이 따라 갈아탄다 — 창은 하나이고 언제나 한 세션에 붙는다.
  const previousSelectionRef = useRef({ taskId, host });
  useEffect(() => {
    const previous = previousSelectionRef.current;
    previousSelectionRef.current = { taskId, host };
    if (previous.taskId !== taskId || previous.host !== host) {
      ++openRequestRef.current;
      if (taskId != null) projectRef.current = null;
    }
    if (poppedOut) void publishSession();
  }, [taskId, host, poppedOut, publishSession]);

  // 응답 진행 여부만 흘려보낸다. 본문은 메인 창에 남는다.
  useEffect(() => {
    if (!poppedOut) return;
    void emitTo(EDITOR_WINDOW_LABEL, EDITOR_STATUS_EVENT, status).catch(() => {});
  }, [status, poppedOut]);

  return { poppedOut, popOut, openProject, revealFile };
}
