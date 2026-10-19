import { useCallback, useMemo, useRef, useState } from "react";
import { copyText } from "../../lib/clipboard";
import {
  fsCreateDir,
  fsCreateFile,
  fsRename,
  fsTrash,
  resolveAbsPath,
  type FsNode,
} from "../../lib/ipc";
import type { MenuRow } from "./ContextMenuShell";
import {
  affectedPaths,
  joinPath,
  namesIn,
  parentOf,
  renamedPath,
  rewritePath,
  targetDir,
} from "./tree-file-ops";
import { isDiffKey, type TabKey } from "../../lib/tab-key";

/**
 * 다이얼로그가 무엇을 묻는 중인가. `dir`는 워크트리 루트 기준 상대 경로(루트는 빈 문자열)이고,
 * 이름 변경일 때는 그 항목이 든 디렉터리다 — 중복 검사를 같은 자리에서 하기 위해서다.
 */
type Pending =
  | { kind: "file" | "dir"; dir: string }
  | { kind: "rename"; dir: string; path: string; name: string };

interface Options {
  /** 열려 있는 작업. null이면 만들 곳이 없다. */
  taskId: number | null;
  tree: FsNode[];
  refreshTree: () => void;
  openFile: (path: string, opts?: { preview?: boolean }) => Promise<boolean>;
  /**
   * 파일을 만들 수 있는 워크트리인가.
   *
   * 생성 IPC는 로컬 호스트에 고정돼 있다(`fsCreateFile`). 원격 워크트리에서는 부를 수 없으므로
   * 항목을 비활성으로 둔다 — 눌러 본 뒤에 실패를 보는 것보다 눌리지 않는 편이 낫다.
   */
  canMutate: boolean;
  /**
   * 열린 탭 — 이름 변경·삭제가 건드릴 것을 찾고, 저장 안 된 것을 지키는 데 쓴다.
   *
   * 무엇이 영향을 받는가는 **실경로**로 판정한다(diff 탭도 그 파일이 사라지면 닫혀야 한다).
   * 다시 여는 쪽만 키를 본다 — 파일 탭이 아닌 것을 파일로 되살리면 없던 탭이 생긴다.
   */
  openFiles: ReadonlyArray<{ key: TabKey; path: string; dirty: boolean }>;
  /** 활성 **파일 탭**의 실경로. diff 탭이 활성이면 `null`이다 — 되돌릴 파일 탭이 없다. */
  activePath: string | null;
  /** 그 경로의 파일 탭과 diff 탭을 함께 닫는다(F-11). */
  closeTabsForPath: (path: string) => void;
  /** 다이얼로그 밖에서 일어난 실패 — 메뉴는 이미 닫혔으므로 화면 배너로 알린다. */
  onError: (message: string) => void;
  onRevealPath?: (path: string) => void;
  onCopyAbsPath?: (path: string) => void;
}

/**
 * 파일 트리의 우클릭 메뉴와 새 파일·폴더 만들기.
 *
 * 메인 창과 팝아웃 에디터 창이 **같은 훅**을 쓴다. 두 벌로 두면 "어디에 만드는가"의 규칙이
 * 창마다 갈리고, 그 규칙은 눈에 보이지 않아 갈린 것도 늦게 발견된다.
 */
export function useTreeFileOps({
  taskId,
  tree,
  refreshTree,
  openFile,
  canMutate,
  openFiles,
  activePath,
  closeTabsForPath,
  onError,
  onRevealPath,
  onCopyAbsPath,
}: Options) {
  const [menu, setMenu] = useState<{ node: FsNode | null; x: number; y: number } | null>(null);
  const [pending, setPending] = useState<Pending | null>(null);
  /** 백엔드가 거부한 사유 — 다이얼로그를 닫지 않고 인라인으로 보여 준다. */
  const [promptError, setPromptError] = useState<string | null>(null);

  /** 빈 공간에서 열었으면 node가 null이고 대상은 루트다. */
  const openMenu = useCallback((node: FsNode | null, x: number, y: number) => {
    setMenu({ node, x, y });
  }, []);
  const closeMenu = useCallback(() => setMenu(null), []);

  const startNew = useCallback((kind: "file" | "dir", dir: string) => {
    setPromptError(null);
    setPending({ kind, dir });
  }, []);

  // 열린 탭은 매 렌더 새 배열로 온다. ref로 잡아 두면 아래 콜백들이 안정적으로 남는다.
  const liveRef = useRef({ openFiles, activePath });
  liveRef.current = { openFiles, activePath };

  /**
   * 이 경로를 건드려도 되는가 — 저장하지 않은 편집이 걸려 있으면 막는다.
   *
   * 이름 변경도 삭제도 열린 탭을 무효로 만든다. 그때 탭을 조용히 닫으면 저장 안 된 편집이
   * 함께 사라지는데, 그것은 되돌릴 수단이 없다(휴지통은 파일을 돌려주지 저장 안 한 내용을
   * 돌려주지 않는다). 그래서 **하기 전에** 멈춘다.
   */
  const blockedBy = useCallback((target: string): string[] => {
    const { openFiles: files } = liveRef.current;
    const hit = new Set(affectedPaths(files.map((f) => f.path), target));
    return files.filter((f) => f.dirty && hit.has(f.path)).map((f) => f.path);
  }, []);

  const startRename = useCallback(
    (node: FsNode) => {
      const dirty = blockedBy(node.path);
      if (dirty.length > 0) {
        onError(`저장하지 않은 편집이 있어 이름을 바꿀 수 없습니다: ${dirty.join(", ")}`);
        return;
      }
      setPromptError(null);
      setPending({ kind: "rename", dir: parentOf(node.path), path: node.path, name: node.name });
    },
    [blockedBy, onError],
  );

  const trash = useCallback(
    (node: FsNode) => {
      if (taskId == null) return;
      const dirty = blockedBy(node.path);
      if (dirty.length > 0) {
        onError(`저장하지 않은 편집이 있어 지울 수 없습니다: ${dirty.join(", ")}`);
        return;
      }
      const hit = affectedPaths(liveRef.current.openFiles.map((f) => f.path), node.path);
      void resolveAbsPath(taskId, node.path)
        .then((abs) => fsTrash(abs))
        .then(() => {
          // 사라진 파일의 탭을 남기면 저장할 때가 되어서야 없다는 것을 알게 된다.
          for (const path of hit) closeTabsForPath(path);
          refreshTree();
        })
        .catch((e) => onError(String(e)));
    },
    [taskId, blockedBy, closeTabsForPath, refreshTree, onError],
  );

  const rows = useMemo<Array<MenuRow | null>>(() => {
    if (menu == null) return [];
    const node = menu.node;
    const dir = targetDir(node);
    const external: Array<MenuRow | null> =
      node == null || onRevealPath == null
        ? []
        : [
            null,
            { key: "reveal", label: "Finder에서 보기", onSelect: () => onRevealPath(node.path) },
          ];
    return [
      { key: "newFile", label: "새 파일…", disabled: !canMutate, onSelect: () => startNew("file", dir) },
      { key: "newDir", label: "새 폴더…", disabled: !canMutate, onSelect: () => startNew("dir", dir) },
      ...(node == null
        ? []
        : ([
            null,
            { key: "rename", label: "이름 변경…", disabled: !canMutate, onSelect: () => startRename(node) },
            {
              key: "trash",
              label: "휴지통으로 이동",
              disabled: !canMutate,
              danger: true,
              // 확인 모달을 두지 않는다(PRD D6) — 되돌릴 지점이 휴지통 자체다.
              onSelect: () => trash(node),
            },
            null,
            { key: "copyPath", label: "경로 복사", onSelect: () => void copyText(node.path).catch(() => undefined) },
            ...(onCopyAbsPath == null
              ? []
              : [{ key: "copyAbsPath", label: "절대 경로 복사", onSelect: () => onCopyAbsPath(node.path) }]),
          ] as Array<MenuRow | null>)),
      ...external,
    ];
  }, [menu, canMutate, startNew, startRename, trash, onRevealPath, onCopyAbsPath]);

  const confirm = useCallback(
    (name: string) => {
      if (pending == null || taskId == null) return;
      setPromptError(null);

      if (pending.kind === "rename") {
        const from = pending.path;
        const to = renamedPath(from, name);
        const open = liveRef.current.openFiles;
        const hit = affectedPaths(open.map((f) => f.path), from);
        // 다시 열 것은 파일 탭뿐이다 — diff 탭은 변경 목록에서 다시 여는 것이지 경로로 열지 않는다.
        const reopenable = affectedPaths(
          open.filter((f) => !isDiffKey(f.key)).map((f) => f.path),
          from,
        );
        const wasActive = liveRef.current.activePath;
        void resolveAbsPath(taskId, from)
          .then((abs) => fsRename(abs, name))
          .then(() => {
            setPending(null);
            for (const path of hit) closeTabsForPath(path);
            refreshTree();
            // 보던 파일을 그대로 남긴다 — 폴더 이름을 바꿨다고 열어 둔 탭이 사라지면 안 된다.
            // 원래 활성이던 것을 맨 마지막에 열어 그 탭이 다시 앞에 오게 한다.
            const moved = reopenable.flatMap((path) => {
              const next = rewritePath(path, from, to);
              return next == null ? [] : [next];
            });
            const active = wasActive == null ? null : rewritePath(wasActive, from, to);
            for (const path of moved) if (path !== active) void openFile(path);
            if (active != null) void openFile(active);
          })
          .catch((e) => setPromptError(String(e)));
        return;
      }

      const { kind, dir } = pending;
      void resolveAbsPath(taskId, dir)
        .then((abs) => (kind === "dir" ? fsCreateDir(abs, name) : fsCreateFile(abs, name)))
        .then(() => {
          setPending(null);
          refreshTree();
          // 만든 파일은 바로 연다 — 만들고 나서 트리에서 다시 찾게 하면 두 동작이 된다.
          // 경로는 상대로 직접 잇는다(백엔드가 돌려주는 절대 경로는 탭이 쓰는 형식이 아니다).
          if (kind === "file") void openFile(joinPath(dir, name));
        })
        .catch((e) => setPromptError(String(e)));
    },
    [pending, taskId, refreshTree, openFile, closeTabsForPath],
  );

  const renaming = pending?.kind === "rename" ? pending : null;
  const taken = useMemo(() => {
    if (pending == null) return new Set<string>();
    const names = namesIn(tree, pending.dir);
    // 이름 변경은 자기 이름이 이미 있는 자리에서 시작한다 — 그것까지 중복으로 치면
    // 다이얼로그가 열리자마자 빨간불이 켜진다.
    if (renaming != null) names.delete(renaming.name);
    return names;
  }, [pending, renaming, tree]);

  const promptProps = {
    open: pending !== null,
    title: renaming != null ? "이름 변경" : pending?.kind === "dir" ? "새 폴더" : "새 파일",
    subtitle: pending == null ? "" : pending.dir === "" ? "(워크트리 루트)" : pending.dir,
    initial: renaming?.name ?? "",
    taken,
    confirmLabel: renaming != null ? "이름 변경" : "만들기",
    serverError: promptError,
    onConfirm: confirm,
    onCancel: () => {
      setPending(null);
      setPromptError(null);
    },
  };

  return { menu, openMenu, closeMenu, rows, promptProps, startNew };
}
