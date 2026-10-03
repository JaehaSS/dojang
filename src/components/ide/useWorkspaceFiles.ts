import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { fsRead, fsTree, fsWrite, readLocalFile, type FileContent, type FsNode } from "../../lib/ipc";
import { LOCAL_HOST, type TaskRef } from "../../lib/transport";
import { planAutosave } from "../../lib/autosave";
import { previewTabsEnabled } from "../../lib/preview-tabs";
import { diffTabKey, fileTabKey, type TabKey } from "../../lib/tab-key";
import type { AutosaveUndoEntry } from "../../lib/editor-window-events";
import type { OpenFile } from "./EditorPane";

/** 작업 worktree와 root-bound 프로젝트 창이 공유하는 파일 경계. */
export interface WorkspaceFileSource {
  key: string;
  tree: () => Promise<FsNode[]>;
  read: (path: string) => Promise<FileContent>;
  write: (path: string, content: string, expectedContent: string) => Promise<number>;
  readOnly?: (path: string) => boolean;
}

const isAbsolutePath = (path: string) => path.startsWith("/") || /^[a-z]:[\\/]/i.test(path);

/** 자동 저장 일괄 처리의 결과. 실패는 첫 건에서 멈추므로 경로가 하나다. */
export type FlushResult =
  | { ok: true; entries: AutosaveUndoEntry[] }
  | { ok: false; path: string; reason: "conflict" | "failed"; detail: string | null };

interface Options {
  /** 열려 있는 작업. null이면 볼 워크트리가 없다. */
  /** 대상 작업의 (host, id) 좌표. id만으로는 호스트가 다른 동명 작업과 구분되지 않는다. */
  task: TaskRef | null;
  /** task 대신 명시한 root-bound source. 두 source를 섞어 같은 탭에 쓰지 않는다. */
  source?: WorkspaceFileSource | null;
  onError: (message: string) => void;
  /** 파일이 실제로 열렸을 때만 불린다 — 메인 창은 여기서 중앙 탭을 에디터로 돌린다.
   *  읽기에 실패하면 부르지 않는다(기존 `showFile`이 성공 경로에만 있던 동작). */
  onFileOpened?: () => void;
  /** 파일 내용이 바뀌면 탐색 controller가 sourceVersion을 올린다. */
  onSourceChange?: (path: string) => void;
}

export interface WorkspaceFiles {
  tree: FsNode[];
  openFiles: OpenFile[];
  /** 지금 보고 있는 **탭**. 같은 경로가 파일 탭과 diff 탭으로 열릴 수 있어 경로로는 못 가린다. */
  activeKey: TabKey | null;
  /** 파일 트리에서 성공적으로 연 파일. 분할 뷰가 현재 포커스된 칸에 앉힌다. */
  treeOpen: { key: TabKey; request: number } | null;
  /** 분할 뷰가 배치한 트리 열기 요청을 소비한다. */
  consumeTreeOpen: (request: number) => void;
  setActiveKey: (key: TabKey | null) => void;
  /** 활성 탭 그 자체 — 실경로가 필요한 소비자(트리 강조·새 파일 위치)는 `activeFile.path`를 쓴다. */
  activeFile: OpenFile | null;
  refreshTree: () => void;
  showFile: (path: string, file: FileContent) => void;
  /** 탭을 연다(이미 열려 있으면 활성화만). **열렸으면 true** — 읽기에 실패하면 false다.
   *  호출자가 뒤이어 커서를 옮길 때 이 값을 봐야 한다. 열리지도 않은 탭에 착지 지점을 걸면
   *  소비되지 않고 남아, 나중에 그 파일을 열 때 커서가 난데없이 뛴다.
   *
   *  `preview`는 **훑어보기**다 — 탭에 표시만 붙인다(`lib/preview-tabs`). 자리를 물려주는
   *  것은 배치의 몫이라 여기서는 목록 끝에 붙일 뿐이다(ADR 0189). 트리
   *  클릭만 이것을 쓰고, 정의 이동·복원처럼 목적이 분명한 열기는 기본값(고정)으로 온다. */
  openFile: (path: string, opts?: { preview?: boolean; tree?: boolean }) => Promise<boolean>;
  /** 이 파일의 변경분 탭을 연다. 내용은 diff 스냅샷에서 오므로 fs를 읽지 않는다(동기). */
  openDiff: (path: string, opts?: { preview?: boolean }) => void;
  /** 프리뷰를 고정으로 승격한다 — 트리·탭 더블클릭. 이미 고정이면 아무 일도 없다. */
  pinTab: (key: TabKey) => void;
  /** 훑어보던 탭을 전부 고정한다 — 프리뷰 토글을 끄는 순간의 정리. */
  pinAll: () => void;
  changeFile: (path: string, content: string) => void;
  saveFile: (path: string, content: string) => Promise<void>;
  reloadFile: (path: string) => Promise<void>;
  closeTab: (key: TabKey) => void;
  /** 그 경로의 탭을 **모두** 닫는다 — 파일 탭과 diff 탭이 함께 사라진다.
   *  이름 변경·휴지통처럼 파일 자체가 없어지는 경로가 부른다(설계 0061 F-11). */
  closeTabsForPath: (path: string) => void;
  flushDirty: () => Promise<FlushResult>;
}

/**
 * 워크트리 파일 상태 — 열린 탭·활성 경로·파일 트리와 그것을 다루는 콜백.
 *
 * `App.tsx`에서 옮겨 왔다. 에디터를 별도 창으로 빼낼 때 두 창이 **같은 훅**을 쓰기 위한 것이며,
 * 복사해 두 벌로 만들면 저장·충돌 규칙이 창마다 갈린다. 파일 트리까지 여기 있는 이유는
 * `FileTree`가 `nodes`를 props로 받기 때문이다(`FileTree.tsx:23`) — 조회가 App에 남으면
 * 에디터 창이 트리를 그릴 수 없다.
 */
export function useWorkspaceFiles({
  task,
  source = null,
  onError,
  onFileOpened,
  onSourceChange,
}: Options): WorkspaceFiles {
  // deps에 객체를 넣으면 매 렌더 새 참조가 되어 effect가 계속 다시 돈다.
  const fileSource = useMemo<WorkspaceFileSource | null>(() => source ?? (task == null ? null : {
    key: `${task.host}:${task.id}`,
    tree: () => fsTree(task),
    read: (path) => isAbsolutePath(path) && task.host === LOCAL_HOST
      ? readLocalFile(task, path)
      : fsRead(task, path),
    write: (path, content) => isAbsolutePath(path)
      ? Promise.reject(new Error("읽기 전용 파일은 저장할 수 없습니다"))
      : fsWrite(task, path, content),
    readOnly: isAbsolutePath,
  }), [source, task?.host, task?.id]);
  const sourceKey = fileSource?.key ?? null;
  const [tree, setTree] = useState<FsNode[]>([]);
  const [openFiles, setOpenFilesState] = useState<OpenFile[]>([]);
  const openFilesRef = useRef<OpenFile[]>([]);
  const taskScopeRef = useRef({ key: sourceKey, generation: 0 });
  if (taskScopeRef.current.key !== sourceKey) {
    taskScopeRef.current = { key: sourceKey, generation: taskScopeRef.current.generation + 1 };
  }
  const sourceRef = useRef(fileSource);
  sourceRef.current = fileSource;
  const revisionsRef = useRef(new Map<string, number>());
  const fileEpochsRef = useRef(new Map<string, number>());
  const nextFileEpochRef = useRef(0);
  const writeQueuesRef = useRef(new Map<string, Promise<void>>());
  // Async operations and queued saves must observe the same buffer immediately,
  // including before React renders. Only an actual file-tab replacement changes
  // its incarnation; closing a same-path diff must not reset file revisions.
  const setOpenFiles = useCallback((update: OpenFile[] | ((files: OpenFile[]) => OpenFile[])) => {
    const before = openFilesRef.current;
    const next = typeof update === "function" ? update(before) : update;
    for (const file of before) {
      if (file.kind !== "diff" && !next.some((item) => item.key === file.key)) {
        fileEpochsRef.current.set(file.path, ++nextFileEpochRef.current);
        revisionsRef.current.delete(file.path);
      }
    }
    for (const file of next) {
      if (file.kind !== "diff" && !before.some((item) => item.key === file.key)) {
        fileEpochsRef.current.set(file.path, ++nextFileEpochRef.current);
        revisionsRef.current.set(file.path, 0);
      }
    }
    openFilesRef.current = next;
    setOpenFilesState(next);
  }, []);
  const enqueueWrite = useCallback(<T,>(path: string, operation: () => Promise<T>): Promise<T> => {
    const prior = writeQueuesRef.current.get(path) ?? Promise.resolve();
    const pending = prior.then(operation);
    writeQueuesRef.current.set(path, pending.then(() => undefined, () => undefined));
    return pending;
  }, []);
  const [activeKey, setActiveKeyState] = useState<TabKey | null>(null);
  const [treeOpen, setTreeOpen] = useState<{ key: TabKey; request: number } | null>(null);
  const treeOpenRequestRef = useRef(0);
  const activeFile = openFiles.find((f) => f.key === activeKey) ?? null;
  /** 훑어보기 표시를 뗀다. 새 배열을 만들면 그 자체가 렌더를 부르므로 붙어 있을 때만 바꾼다. */
  const pinTab = useCallback((key: TabKey) => {
    setOpenFiles((files) =>
      files.some((f) => f.key === key && f.preview)
        ? files.map((f) => (f.key === key ? { ...f, preview: false } : f))
        : files,
    );
  }, [setOpenFiles]);
  const pinAll = useCallback(() => {
    setOpenFiles((files) =>
      files.some((f) => f.preview) ? files.map((f) => (f.preview ? { ...f, preview: false } : f)) : files,
    );
  }, [setOpenFiles]);
  const openedFromTree = useCallback((key: TabKey, tree: boolean | undefined) => {
    if (!tree) {
      setTreeOpen(null);
      return;
    }
    setTreeOpen({ key, request: ++treeOpenRequestRef.current });
  }, []);
  const consumeTreeOpen = useCallback((request: number) => {
    setTreeOpen((current) => current?.request === request ? null : current);
  }, []);
  const setActiveKey = useCallback((key: TabKey | null) => {
    setActiveKeyState(key);
    setTreeOpen(null);
  }, []);

  // 콜백은 ref로 잡아 둔다 — 매 렌더 새 함수가 와도 아래 useCallback들이 다시 만들어지지 않게.
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;
  const onFileOpenedRef = useRef(onFileOpened);
  onFileOpenedRef.current = onFileOpened;
  const onSourceChangeRef = useRef(onSourceChange);
  onSourceChangeRef.current = onSourceChange;

  const refreshTree = useCallback(() => {
    const current = sourceRef.current;
    if (current == null) {
      setTree([]);
      return;
    }
    const generation = taskScopeRef.current.generation;
    current.tree()
      .then((next) => {
        if (taskScopeRef.current.generation === generation && sourceRef.current === current) setTree(next);
      })
      .catch(() => {
        if (taskScopeRef.current.generation === generation && sourceRef.current === current) setTree([]);
      });
  }, [sourceKey]);

  // 작업이 바뀌면 이전 작업의 파일을 들고 있지 않는다. 워크트리가 다르므로 경로의 의미가 다르다.
  useEffect(() => {
    setOpenFiles([]);
    setActiveKeyState(null);
    setTreeOpen(null);
    revisionsRef.current.clear();
    fileEpochsRef.current.clear();
    writeQueuesRef.current.clear();
  }, [sourceKey]);

  useEffect(() => () => {
    taskScopeRef.current = { ...taskScopeRef.current, generation: taskScopeRef.current.generation + 1 };
  }, []);

  const showFile = useCallback((path: string, file: FileContent) => {
    const key = fileTabKey(path);
    const readOnly = sourceRef.current?.readOnly?.(path) ?? false;
    setOpenFiles((files) =>
      files.some((item) => item.key === key)
        ? files
        : [
            ...files,
            {
              key,
              path,
              kind: file.kind,
              content: file.content,
              baseContent: file.content,
              mtime: file.mtime,
              dirty: false,
              readOnly,
            },
          ],
    );
    setActiveKeyState(key);
    setTreeOpen(null);
    onFileOpenedRef.current?.();
    onSourceChangeRef.current?.(path);
  }, []);

  const openFile = useCallback(
    async (path: string, opts?: { preview?: boolean; tree?: boolean }) => {
      const current = sourceRef.current;
      if (current == null) return false;
      const generation = taskScopeRef.current.generation;
      const epoch = fileEpochsRef.current.get(path);
      // 이미 열려 있으면 활성화만 한다. 다시 클릭한 것은 고정 신호가 아니다 — 그 뜻은
      // 더블클릭이 맡는다. 여기서 승격하면 훑어보다 되돌아온 파일이 죄다 붙박이가 된다.
      const key = fileTabKey(path);
      if (openFilesRef.current.some((file) => file.key === key)) {
        setActiveKeyState(key);
        openedFromTree(key, opts?.tree);
        onFileOpenedRef.current?.();
        return true;
      }
      const preview = opts?.preview === true && previewTabsEnabled();
      try {
        const fc = await current.read(path);
        if (taskScopeRef.current.generation !== generation || sourceRef.current !== current) return false;
        // Another read may have opened this path while ours was pending. Focus
        // that buffer without replacing its content or conflict baseline.
        if (openFilesRef.current.some((file) => file.key === key)) {
          setActiveKeyState(key);
          openedFromTree(key, opts?.tree);
          return true;
        }
        if (fileEpochsRef.current.get(path) !== epoch) return false;
        if (!preview) {
          showFile(path, fc);
          openedFromTree(key, opts?.tree);
          return true;
        }
        const entry: OpenFile = {
          key,
          path,
          kind: fc.kind,
          content: fc.content,
          baseContent: fc.content,
          mtime: fc.mtime,
          dirty: false,
          readOnly: current.readOnly?.(path) ?? false,
          preview: true,
        };
        setOpenFiles((files) => [...files, entry]);
        setActiveKeyState(key);
        openedFromTree(key, opts?.tree);
        onFileOpenedRef.current?.();
        return true;
      } catch (e) {
        if (taskScopeRef.current.generation !== generation) return false;
        onErrorRef.current(String(e));
        return false;
      }
    },
    [sourceKey, showFile, openedFromTree],
  );

  /**
   * 변경분 탭을 연다. 파일 탭과 **같은 프리뷰 규칙**을 받되(원장 #339) 읽을 디스크가 없다 —
   * 본문은 세션 diff 스냅샷에서 오므로 여기서는 자리만 만든다(그래서 동기다).
   */
  const openDiff = useCallback((path: string, opts?: { preview?: boolean }) => {
    const key = diffTabKey(path);
    if (openFilesRef.current.some((file) => file.key === key)) {
      setActiveKeyState(key);
      onFileOpenedRef.current?.();
      return;
    }
    const entry: OpenFile = {
      key,
      path,
      kind: "diff",
      content: "",
      baseContent: "",
      mtime: 0,
      dirty: false,
      preview: opts?.preview === true && previewTabsEnabled(),
    };
    setOpenFiles((files) => [...files, entry]);
    setActiveKeyState(key);
    setTreeOpen(null);
    onFileOpenedRef.current?.();
  }, []);

  const changeFile = useCallback((path: string, content: string) => {
    if (openFilesRef.current.find((file) => file.key === fileTabKey(path))?.readOnly) return;
    // baseContent는 건드리지 않는다 — 편집이 원본을 덮으면 충돌 판정도 되돌리기도 기준을 잃는다.
    // 한 글자라도 고쳤으면 훑어보기가 아니다. 다음 프리뷰가 이 탭을 밀어내면 편집이 사라진다.
    setOpenFiles((fs) =>
      fs.map((f) =>
        f.key === fileTabKey(path) ? { ...f, content, dirty: true, preview: false } : f,
      ),
    );
    revisionsRef.current.set(path, (revisionsRef.current.get(path) ?? 0) + 1);
    onSourceChangeRef.current?.(path);
  }, []);

  const saveFile = useCallback(
    async (path: string, content: string) => {
      const current = sourceRef.current;
      if (current == null || openFilesRef.current.find((file) => file.key === fileTabKey(path))?.readOnly) return;
      const generation = taskScopeRef.current.generation;
      const epoch = fileEpochsRef.current.get(path);
      const revision = revisionsRef.current.get(path) ?? 0;
      const isCurrent = () => taskScopeRef.current.generation === generation &&
        sourceRef.current === current && fileEpochsRef.current.get(path) === epoch;
      try {
        await enqueueWrite(path, async () => {
          const file = openFilesRef.current.find((item) => item.key === fileTabKey(path));
          if (!file || !isCurrent()) return;
          const disk = await current.read(path);
          if (!isCurrent()) return;
          if (disk.content !== file.baseContent && !window.confirm(`${path} 가 디스크에서 변경되었습니다. 내 변경으로 덮어쓸까요?`)) return;
          const mtime = await current.write(path, content, disk.content);
          if (!isCurrent()) return;
          const unchanged = (revisionsRef.current.get(path) ?? 0) === revision;
          setOpenFiles((files) => files.map((item) => item.key === file.key
            ? { ...item, content: unchanged ? content : item.content, baseContent: content, mtime, dirty: !unchanged && item.content !== content }
            : item));
          onSourceChangeRef.current?.(path);
        });
      } catch (error) {
        if (isCurrent()) onErrorRef.current(String(error));
      }
    },
    [sourceKey, enqueueWrite, setOpenFiles],
  );

  const reloadFile = useCallback(
    async (path: string) => {
      const current = sourceRef.current;
      if (current == null) return;
      const generation = taskScopeRef.current.generation;
      const epoch = fileEpochsRef.current.get(path);
      const revision = revisionsRef.current.get(path) ?? 0;
      const isCurrent = () => taskScopeRef.current.generation === generation && sourceRef.current === current && fileEpochsRef.current.get(path) === epoch;
      try {
        const fc = await current.read(path);
        if (!isCurrent() || (revisionsRef.current.get(path) ?? 0) !== revision) return;
        // A disk reload replaces this buffer and invalidates any older saves.
        fileEpochsRef.current.set(path, ++nextFileEpochRef.current);
        setOpenFiles((fs) =>
          fs.map((x) =>
            x.key === fileTabKey(path)
              ? {
                  ...x,
                  kind: fc.kind,
                  content: fc.content,
                  baseContent: fc.content,
                  mtime: fc.mtime,
                  dirty: false,
                }
              : x,
          ),
        );
        onSourceChangeRef.current?.(path);
      } catch (e) {
        if (isCurrent())
          onErrorRef.current(String(e));
      }
    },
    [sourceKey],
  );

  /**
   * dirty 파일을 모두 디스크에 쓴다. 충돌·실패가 하나라도 있으면 거기서 멈춘다.
   *
   * `saveFile`과 달리 아무것도 묻지 않는다 — 이 경로는 시야 밖 창에서 도는데, 거기서 뜨는
   * 모달은 아무도 보지 못한다. 대신 충돌이면 덮어쓰지 않고 호출자에게 넘겨 판단하게 한다.
   */
  const flushDirty = useCallback(async (): Promise<FlushResult> => {
    const current = sourceRef.current;
    if (current == null) return { ok: true, entries: [] };
    const generation = taskScopeRef.current.generation;
    const entries: AutosaveUndoEntry[] = [];
    const failed = (path: string, detail: string): FlushResult => ({ ok: false, path, reason: "failed", detail });
    const scopeCurrent = () => taskScopeRef.current.generation === generation && sourceRef.current === current;
    for (const initial of openFilesRef.current) {
      if (initial.kind === "diff") continue;
      if (initial.readOnly) continue;
      const epoch = fileEpochsRef.current.get(initial.path);
      const isCurrent = () => scopeCurrent() && fileEpochsRef.current.get(initial.path) === epoch;
      try {
        const result = await enqueueWrite(initial.path, async (): Promise<FlushResult> => {
          if (!isCurrent()) return failed(initial.path, "파일 범위나 열린 탭이 변경되었습니다.");
          const file = openFilesRef.current.find((item) => item.key === initial.key);
          if (!file?.dirty) return { ok: true, entries: [] };
          const revision = revisionsRef.current.get(file.path) ?? 0;
          const disk = await current.read(file.path);
          if (!isCurrent()) return failed(file.path, "파일 범위나 열린 탭이 변경되었습니다.");
          if ((revisionsRef.current.get(file.path) ?? 0) !== revision) return failed(file.path, "저장 중 내용이 바뀌었습니다. 다시 저장하세요.");
          const plan = planAutosave(file, disk.content);
          if (plan.kind === "skip") return { ok: true, entries: [] };
          if (plan.kind === "conflict") return { ok: false, path: plan.path, reason: "conflict", detail: null };
          const mtime = await current.write(file.path, plan.content, disk.content);
          if (!isCurrent()) return failed(file.path, "파일 범위나 열린 탭이 변경되었습니다.");
          setOpenFiles((files) => files.map((item) => item.key === file.key
            ? { ...item, baseContent: plan.content, mtime, dirty: item.content !== plan.content }
            : item));
          onSourceChangeRef.current?.(file.path);
          if ((revisionsRef.current.get(file.path) ?? 0) !== revision) return failed(file.path, "저장 중 내용이 바뀌었습니다. 다시 저장하세요.");
          return { ok: true, entries: [{ path: file.path, content: plan.undoContent }] };
        });
        if (!result.ok) return result;
        entries.push(...result.entries);
      } catch (error) {
        return failed(initial.path, String(error));
      }
    }
    if (!scopeCurrent()) return failed("", "파일 범위가 변경되었습니다.");
    const pending = openFilesRef.current.find((file) => file.dirty);
    if (pending) return failed(pending.path, "저장 중 내용이 바뀌었습니다. 다시 저장하세요.");
    return { ok: true, entries };
  }, [sourceKey, enqueueWrite, setOpenFiles]);

  /** 키가 이 집합에 든 탭을 모두 닫는다 — 한 번의 상태 갱신으로 끝내야 중간 상태가 새지 않는다. */
  const closeKeys = useCallback((keys: ReadonlySet<TabKey>) => {
    if (keys.size === 0) return;
    const closed = openFilesRef.current.filter((f) => keys.has(f.key));
    const next = openFilesRef.current.filter((f) => !keys.has(f.key));
    setOpenFiles(next);
    closed.forEach((file) => onSourceChangeRef.current?.(file.path));
    setActiveKeyState((current) =>
      current != null && keys.has(current)
        ? next.length
          ? next[next.length - 1].key
          : null
        : current,
    );
  }, []);

  const closeTab = useCallback((key: TabKey) => closeKeys(new Set([key])), [closeKeys]);

  const closeTabsForPath = useCallback(
    (path: string) => closeKeys(new Set([fileTabKey(path), diffTabKey(path)])),
    [closeKeys],
  );

  return {
    tree,
    openFiles,
    activeKey,
    treeOpen,
    consumeTreeOpen,
    setActiveKey,
    activeFile,
    refreshTree,
    showFile,
    openFile,
    openDiff,
    pinTab,
    pinAll,
    changeFile,
    saveFile,
    reloadFile,
    closeTab,
    closeTabsForPath,
    flushDirty,
  };
}
