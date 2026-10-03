import {
  Fragment,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent as ReactDragEvent,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type MutableRefObject,
  type ReactElement,
} from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { EditorPane, type EditorPaneProps, type OpenFile } from "./EditorPane";
import { isAbsolutePath, resolveDocumentLink, type DocumentLinkTarget } from "../../lib/document-link";
import {
  DocumentLinkContextMenu,
  type DocumentLinkMenuAction,
  type DocumentLinkMenuState,
} from "./DocumentLinkContextMenu";
import {
  activateInGroup,
  activatePreviewInGroup,
  canDropOntoGroup,
  closeGroup,
  closeKeyInGroup,
  createLayout,
  dropOntoGroup,
  evenSizes,
  focusedGroup,
  focusGroup,
  isOpenInAnyGroup,
  MAX_GROUPS,
  resizeBoundary,
  splitFocused,
  syncLayout,
  type DropEdge,
  type SplitAxis,
  type SplitLayout,
} from "./editor-split";
import {
  draggedKind,
  dropZoneStyle,
  edgeFromPoint,
  insertIndexFromPoint,
  readTabDrag,
  FILE_DRAG_MIME,
  TAB_DRAG_MIME,
  type TabDragPayload,
} from "./editor-drag";
import { TabContextMenu, type TabMenuAction } from "./TabContextMenu";
import { fileTabKey, pathFromTabKey, type TabKey } from "../../lib/tab-key";
import { fsRead, type LspTarget } from "../../lib/ipc";
import type { HostId } from "../../lib/transport";
import { ReferencesPanel } from "./ReferencesPanel";
import { useEditorNavigation } from "./useEditorNavigation";
import type { WorkspaceFiles } from "./useWorkspaceFiles";

/** 칸 하나에 그대로 넘어가는 EditorPane 계약 — 배치가 소유하는 것만 여기서 뺀다. */
type PassThroughProps = Omit<
  EditorPaneProps,
  | "files"
  | "activeKey"
  | "onSelect"
  | "onClose"
  | "retainedPaths"
  | "onSplit"
  | "onCloseGroup"
  | "onTabDragStart"
  | "onTabMenu"
  | "onLinkMenu"
  | "onOpenLink"
>;

export interface EditorSplitViewProps extends PassThroughProps {
  host: HostId;
  windowId: string;
  files: OpenFile[];
  activeKey: TabKey | null;
  treeOpen?: WorkspaceFiles["treeOpen"];
  onTreeOpenHandled?: (request: number) => void;
  /**
   * 포커스된 칸의 활성 탭을 전역에 알린다. 파일 트리 강조·팝아웃 복원·프리뷰 가용 판정이
   * 모두 이 하나를 읽으므로, 칸이 여럿이어도 전역이 가리키는 탭은 **지금 보고 있는 것** 하나다.
   */
  onSelect: (key: TabKey | null) => void;
  /** 전역에서 탭을 닫는다. 다른 칸이 아직 들고 있으면 부르지 않는다. */
  onClose: (key: TabKey) => void;
  /** Synchronous veto before layout changes, used by sources that must protect dirty tabs. */
  onBeforeClose?: (key: TabKey) => boolean;
  /** 이 화면에서 ⌘W·⌘\를 들을지. 코드 열이 닫혀 있거나 다른 탭을 보는 동안에는 꺼 둔다. */
  shortcutsActive?: boolean;
  /**
   * ⌘W가 왔는데 닫을 파일이 없을 때. 지정하지 않으면 이벤트를 건드리지 않고 흘려보낸다 —
   * 메인 창의 "홈으로 복귀"(ADR 0004)가 그대로 살아 있어야 하기 때문이다.
   */
  onCloseExhausted?: () => void;
  /**
   * 아직 열지 않은 파일을 연다 — 트리에서 칸 위로 끌어다 놓는 경로와 문서 링크 클릭이 쓴다.
   *
   * 없으면 트리 드래그도 링크 열기도 받지 않는다. 파일 상태의 주인은 여전히 `useWorkspaceFiles`라
   * 여기서 목록에 직접 끼워 넣을 수는 없고, 열리기를 기다렸다가 배치를 고쳐야 한다.
   */
  onOpenFile?: (path: string) => Promise<boolean>;
  /**
   * 이 창이 통째로 에디터인가(팝아웃 창).
   *
   * ⌘1‥⌘9의 임자를 가르는 데만 쓴다. 메인 창에서 그 조합은 **세션 이동**이므로
   * (`useSessionTaskShortcuts`) 포커스가 이 안에 있을 때만 탭 전환으로 가져오는데,
   * 팝아웃 창에는 겨룰 상대가 없다 — 아직 아무 데도 포커스가 없어도 우리 것이다.
   */
  ownsWindow?: boolean;
  /** 탭 메뉴의 "절대 경로 복사". 없으면 그 항목을 두지 않는다 — 워크트리 루트를 아는 것은
   *  이 컴포넌트가 아니라 작업을 쥔 쪽이다. */
  onCopyAbsPath?: (path: string) => void;
  onNavigationError?: (message: string) => void;
  /** 문서 링크의 절대 경로를 루트 기준 상대 경로로 풀 때 쓴다. 없으면 절대 경로 링크는 열지 않는다. */
  rootPath?: string | null;
  sourceChangeRef?: MutableRefObject<(path: string) => void>;
}

/** 드래그 중 강조를 그릴 자리 — 어느 칸의 어느 가장자리인가. */
interface DropTarget {
  groupId: string;
  edge: DropEdge;
  /** 탭 바 위일 때 몇 번째와 몇 번째 사이인가. 그 밖에서는 undefined. */
  at?: number;
  /** 삽입 표시를 그릴 x좌표 — 칸 컨테이너 기준 px. */
  caretX?: number;
}

/** 경계를 키보드로 밀 때의 한 걸음(px). 사이드 패널·터미널 도크와 같은 보폭. */
const NUDGE_PX = 24;

/**
 * 지금 키보드가 이 에디터의 것인가 — ⌘1‥⌘9를 세션 이동에서 빼앗을지 판단하는 근거.
 *
 * `contains`는 자기 자신도 참으로 치므로 칸 컨테이너에 포커스가 있어도 통과한다. 포커스가
 * 아무 데도 없을 때(body)만 갈린다: 메인 창에서는 사이드바가 임자일 수 있으니 물러나고,
 * 창 전체가 에디터인 팝아웃에서는 겨룰 상대가 없으니 우리 것이다.
 */
function editorHasFocus(host: HTMLElement | null, standalone: boolean): boolean {
  if (host == null) return false;
  const active = document.activeElement;
  if (active == null || active === document.body) return standalone;
  return host.contains(active);
}

/**
 * 에디터 분할 컨테이너 — 한 워크트리를 여러 칸이 나눠 본다.
 *
 * 파일 상태는 여전히 `useWorkspaceFiles` 한 벌이다. 여기가 하는 일은 그 목록을 칸별 탭으로
 * 나누고, 전역 활성 경로를 포커스된 칸과 동기화하는 것뿐이다(설계 근거는 `editor-split.ts`).
 *
 * 메인 창의 코드 열과 팝아웃 에디터 창이 **같은 컴포넌트**를 쓴다. 분할을 한쪽에만 두면
 * 두 자리의 조작법이 갈리고, 팝아웃은 그 순간부터 "기능이 덜한 에디터"가 된다.
 */
export function EditorSplitView({
  host,
  windowId,
  files,
  activeKey,
  treeOpen,
  onTreeOpenHandled,
  onSelect,
  onClose,
  onBeforeClose,
  shortcutsActive = true,
  onCloseExhausted,
  onOpenFile,
  ownsWindow = false,
  onCopyAbsPath,
  onNavigationError = () => undefined,
  rootPath,
  sourceChangeRef,
  ...pane
}: EditorSplitViewProps): ReactElement {
  const hostRef = useRef<HTMLDivElement>(null);
  const paneAreaRef = useRef<HTMLDivElement>(null);
  const [layout, setLayout] = useState<SplitLayout>(() =>
    createLayout(
      files.map((f) => f.key),
      activeKey,
    ),
  );
  /** 지금 끌고 있는 것. null이면 드롭 면을 아예 깔지 않는다 — Monaco의 드래그를 가리지 않게. */
  const [dragging, setDragging] = useState<{
    kind: "tab" | "file";
    tab: TabDragPayload | null;
  } | null>(null);
  const dragFrameRef = useRef<number | null>(null);
  const [dropTarget, setDropTarget] = useState<DropTarget | null>(null);
  /** 열려 있는 탭 메뉴. 어느 칸의 탭인지까지 들어야 "오른쪽 탭 닫기"의 오른쪽이 정해진다. */
  const [tabMenu, setTabMenu] = useState<{
    groupId: string;
    key: TabKey;
    /** 가리킨 탭의 실경로 — 경로 복사·Finder는 키가 아니라 파일에 대한 동작이다. */
    path: string;
    x: number;
    y: number;
  } | null>(null);
  const [documentLinkMenu, setDocumentLinkMenu] = useState<
    (DocumentLinkMenuState & { groupId: string; sourceKey: TabKey }) | null
  >(null);
  const [resizingIndex, setResizingIndex] = useState<number | null>(null);
  const referenceRequestRef = useRef(0);
  const beginReferences = useCallback(() => ++referenceRequestRef.current, []);
  const [references, setReferences] = useState<{
    targets: LspTarget[];
    total: number;
    sourcePath: string;
    stale: boolean;
    origin: {
      path: string;
      groupId: string;
      line: number;
      column: number;
      scrollTop: number;
      scrollLeft: number;
    };
  } | null>(null);
  const [activeTool, setActiveTool] = useState<"references" | null>(null);

  const showReferences = useCallback((
    targets: LspTarget[],
    sourcePath: string,
    request: number,
    origin: { path: string; groupId: string; line: number; column: number; scrollTop: number; scrollLeft: number },
  ) => {
    if (request !== referenceRequestRef.current) return;
    const unique = [...new Map(targets.slice().sort((a, b) => `${a.path}:${a.line}:${a.column}`.localeCompare(`${b.path}:${b.line}:${b.column}`)).map((target) => [`${target.abs_path}:${target.line}:${target.column}`, target])).values()];
    setReferences({ targets: unique.slice(0, 500), total: unique.length, sourcePath, stale: false, origin });
    setActiveTool("references");
  }, []);
  const readReferencePreview = useCallback(async (target: LspTarget) => {
    if (target.external || target.path == null || pane.taskId == null) return null;
    const open = files.find((file) => file.path === target.path && file.kind === "text");
    if (open) return open.content;
    const file = await fsRead({ host, id: pane.taskId }, target.path);
    return file.kind === "text" ? file.content : null;
  }, [files, host, pane.taskId]);
  const sourceVersionsRef = useRef(new Map<string, number>());
  const sourceVersion = useCallback((path: string) => sourceVersionsRef.current.get(`${host}:${pane.taskId}:${path}`) ?? 0, [host, pane.taskId]);
  const navigation = useEditorNavigation({
    identity: pane.taskId == null ? null : { host, taskId: pane.taskId, windowId },
    openTarget: async (target) => pane.onOpenTarget ? pane.onOpenTarget(target) : "failed",
    onError: onNavigationError,
  });
  useEffect(() => {
    referenceRequestRef.current += 1;
    setDocumentLinkMenu(null);
    setReferences(null);
    setActiveTool(null);
  }, [host, pane.taskId, windowId]);
  useEffect(() => {
    if (sourceChangeRef == null) return;
    sourceChangeRef.current = (path) => {
      const key = `${host}:${pane.taskId}:${path}`;
      sourceVersionsRef.current.set(key, sourceVersion(path) + 1);
      setReferences((current) => current?.sourcePath === path ? { ...current, stale: true } : current);
    };
    return () => { sourceChangeRef.current = () => undefined; };
  }, [host, pane.taskId, sourceChangeRef, sourceVersion]);
  const closeReferences = useCallback(() => {
    setActiveTool(null);
    hostRef.current?.querySelector<HTMLElement>('[data-focused="true"] .monaco-editor textarea')?.focus();
  }, []);
  const navigationGroupId = navigation.reveal == null ? null : layout.groups.some((group) => group.id === navigation.reveal?.groupId) ? navigation.reveal.groupId : layout.focusedId;
  useEffect(() => {
    const reveal = navigation.reveal;
    if (reveal == null || navigationGroupId == null) return;
    setLayout((current) => activateInGroup(current, navigationGroupId, fileTabKey(reveal.path)));
  }, [navigation.reveal, navigationGroupId]);
  /**
   * 트리에서 떨군 파일이 열리기를 기다리는 자리.
   *
   * 파일 목록의 주인이 위(`useWorkspaceFiles`)라, 여는 요청을 보낸 뒤 그 파일이 실제로 목록에
   * 들어오는 것은 다음 렌더다. 그때 배치를 고치려고 의도를 여기 적어 둔다.
   */
  const pendingPlacements = useRef(new Map<symbol, {
    path: string;
    target: DropTarget;
    kind: "copy" | "move";
  }>());
  const handledTreeOpen = useRef(0);
  /** 동기화를 effect 본문에서 하므로 최신 배치를 여기서 읽는다(updater의 `current` 대신). */
  const layoutRef = useRef(layout);
  layoutRef.current = layout;
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  // 배열은 매 렌더 새로 오므로 내용으로 비교한다 — 키에 개행이 들어갈 일은 없다.
  const openKeysJoined = files.map((f) => f.key).join("\n");
  const openKeys = useMemo(
    () => (openKeysJoined === "" ? [] : (openKeysJoined.split("\n") as TabKey[])),
    [openKeysJoined],
  );
  // 프리뷰 자리 판정에 필요한 두 가지도 같은 방식으로 뽑는다 — effect가 배열 참조로 다시 돌지 않게.
  const previewJoined = files.filter((f) => f.preview).map((f) => f.key).join("\n");
  const dirtyJoined = files.filter((f) => f.dirty).map((f) => f.key).join("\n");
  /**
   * 모델을 살려 둘 **실경로**. 레이아웃 키와 한 배열에 섞으면 diff 키가 경로 자리로 샌다.
   * diff 탭은 Monaco 모델을 만들지 않으므로 여기 들어가지 않는다.
   */
  const retainedJoined = files
    .filter((f) => f.kind === "text")
    .map((f) => f.path)
    .join("\n");
  const retainedPaths = useMemo(
    () => (retainedJoined === "" ? [] : retainedJoined.split("\n")),
    [retainedJoined],
  );

  /**
   * 배치를 파일 목록에 맞춘다. updater가 아니라 effect 본문에서 계산하는 이유는 **부수효과**다 —
   * 프리뷰 자리를 내준 탭은 전역에서도 닫아야 하는데, StrictMode는 updater를 두 번 부른다.
   */
  useEffect(() => {
    const marks = {
      preview: new Set(previewJoined === "" ? [] : (previewJoined.split("\n") as TabKey[])),
      dirty: new Set(dirtyJoined === "" ? [] : (dirtyJoined.split("\n") as TabKey[])),
    };
    const { layout: synced, evicted } = syncLayout(layoutRef.current, openKeys, activeKey, marks);
    let seated = synced;
    const evictedKeys = [...evicted];
    if (treeOpen != null && treeOpen.request !== handledTreeOpen.current && activeKey === treeOpen.key && openKeys.includes(treeOpen.key)) {
      handledTreeOpen.current = treeOpen.request;
      const placed = activatePreviewInGroup(seated, layoutRef.current.focusedId, treeOpen.key, marks);
      seated = placed.layout;
      evictedKeys.push(...placed.evicted);
      onTreeOpenHandled?.(treeOpen.request);
    }
    const seatedPaths = new Set<string>();
    for (const [request, pending] of pendingPlacements.current) {
      const key = fileTabKey(pending.path);
      if (!openKeys.includes(key)) continue;
      pendingPlacements.current.delete(request);
      if (pending.kind === "copy" && seatedPaths.has(pending.path)) {
        seated = activateInGroup(seated, pending.target.groupId, key, pending.target.at);
      } else {
        const owner = seated.groups.find((group) => group.keys.includes(key));
        seated = dropOntoGroup(seated, key, pending.target.groupId, pending.target.edge, owner?.id ?? null, pending.target.at);
      }
      seatedPaths.add(pending.path);
    }
    setLayout(seated);
    for (const key of new Set(evictedKeys)) closeRef.current(key);
  }, [openKeys, activeKey, previewJoined, dirtyJoined, treeOpen?.request]);

  useEffect(() => {
    pendingPlacements.current.clear();
  }, [host, pane.taskId, windowId]);

  useEffect(() => {
    setDocumentLinkMenu((current) => {
      if (current == null) return null;
      const group = layout.groups.find((item) => item.id === current.groupId);
      return group?.activeKey === current.sourceKey && group.keys.includes(current.sourceKey)
        ? current
        : null;
    });
  }, [layout]);

  /** 키로 잡는다 — 경로로 잡으면 같은 파일의 파일 탭과 diff 탭이 한 자리를 놓고 서로를 덮는다. */
  const byKey = useMemo(() => new Map(files.map((f) => [f.key, f])), [files]);

  /** 배치를 바꾼 뒤 전역 활성 경로를 포커스된 칸에 맞춘다. 두 값이 갈리면 트리 강조가 거짓이 된다. */
  const commit = (next: SplitLayout) => {
    setLayout(next);
    onSelect(focusedGroup(next).activeKey);
  };

  const select = (groupId: string, key: TabKey) => {
    setLayout(activateInGroup(layout, groupId, key));
    onSelect(key);
  };

  const focus = (groupId: string) => {
    if (layout.focusedId === groupId) return;
    commit(focusGroup(layout, groupId));
  };

  const closeInGroup = (groupId: string, key: TabKey) => {
    if (onBeforeClose?.(key) === false) return;
    const next = closeKeyInGroup(layout, groupId, key);
    // 옆 칸이 같은 탭을 띄우고 있으면 전역에서는 열린 채로 둔다 — 그쪽 편집이 사라지면 안 된다.
    if (!isOpenInAnyGroup(next, key)) onClose(key);
    commit(next);
  };

  /** 한 칸에서 여러 탭을 한 번에 닫는다 — 전역 닫기 판정은 **다 뗀 뒤에** 한 번만 한다.
   *  하나씩 판정하면 옆 칸이 들고 있는지를 중간 상태에서 묻게 되어 답이 달라진다. */
  const closeManyInGroup = (groupId: string, keys: TabKey[]) => {
    if (keys.length === 0) return;
    if (keys.some((key) => onBeforeClose?.(key) === false)) return;
    let next = layout;
    for (const key of keys) next = closeKeyInGroup(next, groupId, key);
    for (const key of keys) if (!isOpenInAnyGroup(next, key)) onClose(key);
    commit(next);
  };

  /** 분할은 보던 파일을 옆에도 두겠다는 뜻이다 — 훑어보기가 아니므로 그 탭은 고정된다. */
  const split = (axis: SplitAxis) => {
    const active = focusedGroup(layout).activeKey;
    if (active != null) pane.onPinTab?.(active);
    setLayout(splitFocused(layout, axis));
  };

  const drop = (groupId: string) => commit(closeGroup(layout, groupId));

  /** 클립보드는 실패해도 조용히 넘어간다 — 권한이 없거나 창이 포커스를 잃은 사이의 거부는
   *  사용자가 손쓸 것이 없고, 오류 배너를 띄우면 정작 복사된 경우와 구분이 안 된다. */
  const copy = (text: string) => void navigator.clipboard?.writeText(text).catch(() => undefined);

  const openDocumentLinkMenu = useCallback(
    (groupId: string, sourceKey: TabKey, sourcePath: string, link: string, at: { x: number; y: number }) => {
      setTabMenu(null);
      setDocumentLinkMenu({
        ...at,
        groupId,
        sourceKey,
        link,
        target: resolveDocumentLink(link, sourcePath, rootPath),
      });
    },
    [rootPath],
  );

  /**
   * 문서 링크가 가리키는 것을 연다 — 파일은 **그 문서와 같은 칸**에, URL은 기본 브라우저로.
   *
   * 문서 탭은 그대로 남는다. 링크를 따라간 뒤 돌아올 손잡이가 문서 탭 말고는 없기 때문이다.
   */
  const openLinkTarget = (groupId: string, target: DocumentLinkTarget | null, link: string): void => {
    if (target == null) {
      // `#조각`만 있는 링크는 같은 문서 안의 이동이라 열 파일이 없다 — 조용히 둔다.
      if (link.trim().startsWith("#")) return;
      onNavigationError(`에디터에서 열 수 없는 링크: ${link}`);
      return;
    }
    if (target.kind === "url") {
      void openUrl(target.url).catch((error: unknown) => onNavigationError(String(error)));
      return;
    }
    const key = fileTabKey(target.path);
    // 옆 칸이 들고 있어도 이 칸에 띄운다 — 배치의 복제이지 내용의 복제가 아니다(ADR 0132).
    if (byKey.has(key)) return commit(activateInGroup(layout, groupId, key));
    if (onOpenFile == null) return;
    // 열리는 것은 다음 렌더다 — 요청별 대기 자리에 "이 칸에"를 적어 둔다.
    const request = Symbol(target.path);
    pendingPlacements.current.set(request, { path: target.path, target: { groupId, edge: "center" }, kind: "copy" });
    void onOpenFile(target.path)
      .then((opened) => {
        if (!opened) pendingPlacements.current.delete(request);
      })
      .catch((error: unknown) => {
        pendingPlacements.current.delete(request);
        onNavigationError(String(error));
      });
  };

  const openDocumentLink = (
    groupId: string,
    sourcePath: string,
    link: string,
  ): void => openLinkTarget(groupId, resolveDocumentLink(link, sourcePath, rootPath), link);

  const runDocumentLinkMenu = (action: DocumentLinkMenuAction): void => {
    if (documentLinkMenu == null) return;
    const { groupId, sourceKey, link, target } = documentLinkMenu;
    const group = layout.groups.find((item) => item.id === groupId);
    if (group?.activeKey !== sourceKey || !group.keys.includes(sourceKey)) return;
    if (action === "copyLink") return copy(link);
    if (target == null) return;
    if (target.kind === "file") {
      if (action === "open") return openLinkTarget(groupId, target, link);
      if (action === "copyPath") return copy(target.path);
      // 루트 밖 문서의 옆 파일은 이미 절대 경로다 — 루트에 붙여 달라고 하면 거부만 돌아온다.
      if (action === "copyAbsPath") return isAbsolutePath(target.path) ? copy(target.path) : onCopyAbsPath?.(target.path);
      if (action === "reveal") return pane.onRevealPath(target.path);
      return;
    }
    if (action !== "open") return;
    openLinkTarget(groupId, target, link);
  };

  const runTabMenu = (action: TabMenuAction): void => {
    if (tabMenu == null) return;
    const { groupId, key, path } = tabMenu;
    const group = layout.groups.find((g) => g.id === groupId);
    // 메뉴가 떠 있는 사이 그 탭이 사라질 수 있다(작업 전환·옆 칸에서 닫기). 가리키던 것이
    // 없어진 메뉴는 이미 뜻을 잃었다 — 특히 "오른쪽 탭 닫기"는 `indexOf`가 -1이 되는 순간
    // slice(0)이 되어 **칸 전체**를 닫는다.
    if (group == null || !group.keys.includes(key)) return;
    switch (action) {
      case "close":
        return closeInGroup(groupId, key);
      case "closeOthers":
        return closeManyInGroup(groupId, group.keys.filter((k) => k !== key));
      case "closeRight":
        return closeManyInGroup(groupId, group.keys.slice(group.keys.indexOf(key) + 1));
      case "copyPath":
        return copy(path);
      case "copyAbsPath":
        return onCopyAbsPath?.(path);
      case "copyContent":
        return copy(byKey.get(key)?.content ?? "");
      case "reveal":
        return pane.onRevealPath(path);
      case "openExternal":
        return pane.onOpenPath(path);
      case "splitRight":
      case "splitDown": {
        // 메뉴는 **가리킨 탭**에 대해 동작한다. 그런데 `splitFocused`는 그 칸의 활성 파일을
        // 복제하므로, 활성이 아닌 탭에서 분할을 고르면 엉뚱한 파일이 열린다 — 먼저 띄우고 나눈다.
        const shown = activateInGroup(layout, groupId, key);
        pane.onPinTab?.(key);
        return commit(splitFocused(shown, action === "splitRight" ? "row" : "column"));
      }
    }
  };

  /**
   * 탭을 집어 든다. 운반 형식은 `editor-drag`가 정하고, 원본 칸은 여기서만 알 수 있다 —
   * `EditorPane`은 자기가 몇 번째 칸인지 모른다.
   */
  const startTabDrag = (
    groupId: string,
    key: TabKey,
    event: ReactDragEvent<HTMLElement>,
  ): void => {
    const payload: TabDragPayload = { key, groupId };
    event.dataTransfer.setData(TAB_DRAG_MIME, JSON.stringify(payload));
    event.dataTransfer.effectAllowed = "move";
    scheduleTabDragging(payload);
  };

  /**
   * 브라우저 네이티브 드래그 시작 중에는 원본 탭 아래의 DOM을 바꾸지 않는다. 받는 면이 즉시
   * 생기면 원본을 잃고 dragend로 끝날 수 있으므로, 다음 프레임에만 겹친다.
   */
  const scheduleTabDragging = (tab: TabDragPayload | null): void => {
    if (dragFrameRef.current != null) cancelAnimationFrame(dragFrameRef.current);
    dragFrameRef.current = requestAnimationFrame(() => {
      dragFrameRef.current = null;
      setDragging({ kind: "tab", tab });
    });
  };

  /**
   * 드래그가 시작된 것을 창 단위로 듣는다.
   *
   * 파일 트리는 다른 컴포넌트라 우리에게 알려 줄 길이 없고, 알려 주게 만들면 트리가 에디터
   * 배치를 알아야 한다. `dragstart`는 문서까지 버블하므로 여기서 한 번에 받는다 —
   * 이 단계에서는 dataTransfer가 읽기/쓰기 모드라 값도 꺼낼 수 있다(`dragover`에서는 못 한다).
   */
  useEffect(() => {
    const onDragStart = (event: globalThis.DragEvent): void => {
      const kind = draggedKind(event.dataTransfer?.types);
      if (kind == null) return;
      const raw = kind === "tab" ? (event.dataTransfer?.getData(TAB_DRAG_MIME) ?? "") : "";
      if (kind === "tab") {
        scheduleTabDragging(raw === "" ? null : readTabDrag(raw));
        return;
      }
      setDragging({ kind, tab: null });
    };
    const onDragEnd = (): void => {
      if (dragFrameRef.current != null) cancelAnimationFrame(dragFrameRef.current);
      dragFrameRef.current = null;
      setDragging(null);
      setDropTarget(null);
    };
    window.addEventListener("dragstart", onDragStart);
    window.addEventListener("dragend", onDragEnd);
    window.addEventListener("drop", onDragEnd);
    return () => {
      window.removeEventListener("dragstart", onDragStart);
      window.removeEventListener("dragend", onDragEnd);
      window.removeEventListener("drop", onDragEnd);
      if (dragFrameRef.current != null) cancelAnimationFrame(dragFrameRef.current);
    };
  }, []);

  /**
   * 탭 바의 이 x좌표가 몇 번째 자리인가 — 삽입 표시와 실제 드롭이 같은 답을 써야 한다.
   *
   * 탭 요소를 DOM에서 직접 재는 이유는 폭이 파일명에 따라 제각각이고 탭 바가 가로로
   * 스크롤되기 때문이다. 계산으로 맞히려면 폰트 메트릭을 알아야 한다.
   */
  const tabSeat = (groupId: string, clientX: number): { at: number; caretX: number } | null => {
    const pane = hostRef.current?.querySelector<HTMLElement>(`[data-group-id="${groupId}"]`);
    if (pane == null) return null;
    const rects = Array.from(pane.querySelectorAll<HTMLElement>("[data-tab-path]"), (el) =>
      el.getBoundingClientRect(),
    );
    const at = insertIndexFromPoint(rects, clientX);
    const paneLeft = pane.getBoundingClientRect().left;
    const edgeX = at === 0 ? (rects[0]?.left ?? paneLeft) : (rects[at - 1]?.right ?? paneLeft);
    return { at, caretX: edgeX - paneLeft };
  };

  /** 드래그 중인 것이 이 칸의 이 가장자리로 갈 수 있는가. 트리 파일은 아직 어느 칸에도 없다. */
  const dropAllowed = (groupId: string, edge: DropEdge, at?: number): boolean => {
    if (dragging == null) return false;
    if (dragging.kind === "tab") {
      const tab = dragging.tab;
      // 시작 시점에 값을 못 읽은 드래그는 판정을 미룬다 — 실제 데이터는 drop에서 다시 본다.
      if (tab == null) return true;
      return canDropOntoGroup(layout, tab.key, groupId, edge, tab.groupId, at);
    }
    return onOpenFile != null && (edge === "center" || layout.groups.length < MAX_GROUPS);
  };

  /**
   * 이 이벤트가 가리키는 자리. `fixed`가 오면 좌표를 보지 않는다 — 탭 바처럼 뜻이 정해진 띠다.
   * 재는 기준은 **받는 면 자신의 사각형**이라, 면이 탭 바를 비켜 깔려도 판정이 어긋나지 않는다.
   */
  const edgeOf = (event: ReactDragEvent<HTMLDivElement>, fixed?: DropEdge): DropEdge =>
    fixed ??
    edgeFromPoint(event.currentTarget.getBoundingClientRect(), event.clientX, event.clientY);

  const onPaneDragOver =
    (groupId: string, fixed?: DropEdge) =>
    (event: ReactDragEvent<HTMLDivElement>): void => {
      if (dragging == null) return;
      const edge = edgeOf(event, fixed);
      // 탭 바 위라면 "이 칸에"가 아니라 "이 칸의 몇 번째에"까지 정해진다.
      const seat = edge === "center" && fixed === "center" ? tabSeat(groupId, event.clientX) : null;
      if (!dropAllowed(groupId, edge, seat?.at)) {
        if (dropTarget != null) setDropTarget(null);
        return;
      }
      // preventDefault를 해야 브라우저가 이 자리를 드롭 가능한 곳으로 인정한다.
      event.preventDefault();
      // dropEffect는 끄는 쪽이 `dragstart`에서 정한 effectAllowed와 짝이 맞아야 한다.
      // 어긋나면 브라우저가 드래그 동작을 none으로 확정해 **drop 이벤트를 아예 보내지 않는다** —
      // 강조는 뜨는데 손을 놓으면 아무 일도 없는 형태로만 드러나서 원인이 보이지 않는다.
      // 탭은 떠나온 칸에서 빠지므로 move, 트리 파일은 트리에 그대로 남으므로 copy다.
      event.dataTransfer.dropEffect = dragging.kind === "tab" ? "move" : "copy";
      if (
        dropTarget?.groupId !== groupId ||
        dropTarget.edge !== edge ||
        dropTarget.at !== seat?.at
      ) {
        setDropTarget({ groupId, edge, at: seat?.at, caretX: seat?.caretX });
      }
    };

  const onPaneDrop =
    (groupId: string, fixed?: DropEdge) =>
    (event: ReactDragEvent<HTMLDivElement>): void => {
      const edge = edgeOf(event, fixed);
      const at =
        edge === "center" && fixed === "center" ? tabSeat(groupId, event.clientX)?.at : undefined;
      if (!dropAllowed(groupId, edge, at)) return;
      event.preventDefault();
      event.stopPropagation();
      setDragging(null);
      setDropTarget(null);

      const tabData = event.dataTransfer.getData(TAB_DRAG_MIME);
      if (tabData !== "") {
        const payload = readTabDrag(tabData);
        if (payload == null) return;
        // 끌어다 놓는 것은 붙들기다. 같은 칸 안의 순서 바꾸기만 예외 — 자리를 옮긴 것이 아니다.
        if (payload.groupId !== groupId || edge !== "center") pane.onPinTab?.(payload.key);
        commit(dropOntoGroup(layout, payload.key, groupId, edge, payload.groupId, at));
        return;
      }
      const path = event.dataTransfer.getData(FILE_DRAG_MIME);
      if (path === "" || onOpenFile == null) return;
      // 트리에서 온 것은 언제나 파일 탭이다 — 변경 목록은 이 MIME을 쓰지 않는다.
      const key = fileTabKey(path);
      // 이미 열려 있으면 곧바로 옮긴다 — 여는 쪽을 거치면 배치가 한 프레임 흔들린다.
      if (openKeys.includes(key)) {
        const owner = layout.groups.find((g) => g.keys.includes(key));
        commit(dropOntoGroup(layout, key, groupId, edge, owner?.id ?? null, at));
        return;
      }
      const request = Symbol(path);
      pendingPlacements.current.set(request, { path, target: { groupId, edge, at }, kind: "move" });
      void onOpenFile(path)
        .then((opened) => {
          if (!opened) pendingPlacements.current.delete(request);
        })
        .catch((error: unknown) => {
          pendingPlacements.current.delete(request);
          onNavigationError(String(error));
        });
    };

  // 리스너는 한 번만 걸고 최신 값은 ref로 읽는다 — `useEditorPopOutShortcut`과 같은 형태.
  const latest = useRef({
    layout,
    shortcutsActive,
    closeInGroup,
    split,
    select,
    onCloseExhausted,
    ownsWindow,
  });
  latest.current = {
    layout,
    shortcutsActive,
    closeInGroup,
    split,
    select,
    onCloseExhausted,
    ownsWindow,
  };

  useEffect(() => {
    /**
     * 캡처 단계로 받는다. 메인 창의 ⌘W 핸들러(App)는 이 컴포넌트보다 **먼저** 마운트되므로
     * 버블 단계에 걸면 등록 순서상 그쪽이 이긴다 — 파일 대신 화면이 홈으로 빠진다.
     */
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || event.repeat) return;
      const optionClose = event.altKey && !event.metaKey && !event.ctrlKey
        && !event.shiftKey && event.code === "KeyW";
      if (!optionClose && (!(event.metaKey || event.ctrlKey) || event.altKey)) return;
      const now = latest.current;
      if (!now.shortcutsActive) return;
      /*
        ⌘1‥⌘9 — 포커스된 칸의 N번째 탭으로 간다.

        같은 조합이 사이드바의 세션 이동이기도 하다(`useSessionTaskShortcuts`). 둘 중 하나를
        포기하는 대신 **포커스가 있는 쪽이 가져간다**: 에디터 안에서 눌렀으면 파일, 밖에서
        눌렀으면 세션. 저쪽은 버블 단계에서 `defaultPrevented`를 보므로, 여기서 캡처로
        먼저 막기만 하면 스스로 물러난다.
      */
      const digit = /^Digit([1-9])$/.exec(event.code)?.[1];
      if (digit !== undefined) {
        if (event.shiftKey) return;
        if (!editorHasFocus(hostRef.current, now.ownsWindow)) return;
        const group = focusedGroup(now.layout);
        const key = group.keys[Number(digit) - 1];
        // 없는 번호는 삼키지 않는다 — 탭이 둘뿐인데 ⌘5가 죽으면 세션 이동까지 함께 죽는다.
        if (key === undefined) return;
        event.preventDefault();
        now.select(group.id, key);
        return;
      }
      /*
        ⌘⇧[ · ⌘⇧] — 이전·다음 탭. 번호가 닿지 않는 열 번째 이후를 덮고, 손이 어느 파일이
        몇 번인지 몰라도 된다. 끝에서 한 바퀴 돈다 — 마지막 탭에서 멈추면 되돌아가려고
        반대 키를 여덟 번 눌러야 한다.
      */
      if (event.code === "BracketLeft" || event.code === "BracketRight") {
        if (!event.shiftKey) return;
        const group = focusedGroup(now.layout);
        if (group.activeKey == null || group.keys.length < 2) return;
        const index = group.keys.indexOf(group.activeKey);
        const step = event.code === "BracketRight" ? 1 : -1;
        const next = group.keys[(index + step + group.keys.length) % group.keys.length];
        event.preventDefault();
        now.select(group.id, next);
        return;
      }
      if (event.code === "Backslash") {
        event.preventDefault();
        now.split(event.shiftKey ? "column" : "row");
        return;
      }
      if (event.code !== "KeyW" || event.shiftKey) return;
      const group = focusedGroup(now.layout);
      if (group.activeKey == null) {
        if (now.onCloseExhausted == null) return; // 원래 주인에게 넘긴다.
        event.preventDefault();
        now.onCloseExhausted();
        return;
      }
      event.preventDefault();
      now.closeInGroup(group.id, group.activeKey);
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, []);

  const horizontal = layout.axis === "row";

  /** 드래그 시작 시점의 배치에 **절대 델타**를 적용한다 — 프레임마다 누적하면 오차가 쌓인다. */
  const onSeparatorPointerDown =
    (index: number) =>
    (event: ReactPointerEvent<HTMLDivElement>): void => {
      const host = paneAreaRef.current;
      if (host == null || event.button !== 0) return;
      event.preventDefault();
      const rect = host.getBoundingClientRect();
      const total = horizontal ? rect.width : rect.height;
      const origin = horizontal ? event.clientX : event.clientY;
      const start = layout;
      // 포인터를 손잡이에 붙들어 둔다. 끌다 보면 커서가 Monaco 위로 들어가는데, 잡아 두지
      // 않으면 그쪽이 이벤트를 가져가 경계가 커서를 놓친다.
      const handle = event.currentTarget;
      handle.setPointerCapture?.(event.pointerId);
      // 드래그 내내 커서와 선택을 문서 전체에 고정한다 — 칸 위를 지날 때 커서가 I빔으로
      // 바뀌거나 코드가 파랗게 선택되면 "지금 끌고 있다"는 신호가 끊긴다.
      const body = document.body;
      const restore = {
        cursor: body.style.cursor,
        select: body.style.userSelect,
      };
      body.style.cursor = horizontal ? "col-resize" : "row-resize";
      body.style.userSelect = "none";
      setResizingIndex(index);

      const move = (moveEvent: PointerEvent) => {
        const delta = (horizontal ? moveEvent.clientX : moveEvent.clientY) - origin;
        setLayout(resizeBoundary(start, index, delta, total));
      };
      const stop = () => {
        body.style.cursor = restore.cursor;
        body.style.userSelect = restore.select;
        setResizingIndex(null);
        handle.releasePointerCapture?.(event.pointerId);
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", stop);
        window.removeEventListener("pointercancel", stop);
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", stop);
      window.addEventListener("pointercancel", stop);
    };

  const onSeparatorKeyDown =
    (index: number) =>
    (event: ReactKeyboardEvent<HTMLDivElement>): void => {
      const back = horizontal ? "ArrowLeft" : "ArrowUp";
      const forward = horizontal ? "ArrowRight" : "ArrowDown";
      if (event.key !== back && event.key !== forward) return;
      event.preventDefault();
      const host = paneAreaRef.current;
      const rect = host?.getBoundingClientRect();
      const total = rect == null ? 0 : horizontal ? rect.width : rect.height;
      setLayout(resizeBoundary(layout, index, event.key === back ? -NUDGE_PX : NUDGE_PX, total));
    };

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!shortcutsActive || !editorHasFocus(hostRef.current, ownsWindow)) return;
      const minus = event.code === "Minus" || event.key === "-" || event.key === "_";
      const back =
        (event.ctrlKey && !event.shiftKey && minus) ||
        (event.altKey && event.key === "ArrowLeft");
      const forward = (event.ctrlKey && event.shiftKey && minus) || (event.altKey && event.key === "ArrowRight");
      if (!back && !forward) return;
      event.preventDefault();
      event.stopPropagation();
      if (back) navigation.back(); else navigation.forward();
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [navigation, ownsWindow, shortcutsActive]);
  const multi = layout.groups.length > 1;

  return (
    <div
      ref={hostRef}
      className="relative flex min-h-0 min-w-0 flex-1 flex-col"
    >
      <div className="flex h-8 shrink-0 items-center justify-end gap-1 border-b border-border px-2" aria-label="코드 탐색">
        <button disabled={!navigation.canBack} onClick={navigation.back} title="뒤로 (Alt+← / Ctrl+-)" className="rounded px-2 py-1 text-xs text-text-muted disabled:opacity-40">뒤로</button>
        <button disabled={!navigation.canForward} onClick={navigation.forward} title="앞으로 (Alt+→ / Ctrl+Shift+-)" className="rounded px-2 py-1 text-xs text-text-muted disabled:opacity-40">앞으로</button>
      </div>
      <div ref={paneAreaRef} className={`relative flex min-h-0 min-w-0 flex-1 ${horizontal ? "flex-row" : "flex-col"}`}>
      {layout.groups.map((group, index) => (
        <Fragment key={group.id}>
          {index > 0 && (
            <div
              role="separator"
              aria-label={`편집 칸 ${index}·${index + 1} 크기 조절`}
              aria-orientation={horizontal ? "vertical" : "horizontal"}
              aria-valuenow={Math.round(
                (layout.groups[index - 1].size /
                  (layout.groups[index - 1].size + group.size)) *
                  100,
              )}
              aria-valuemin={0}
              aria-valuemax={100}
              tabIndex={0}
              data-resizing={resizingIndex === index - 1}
              className={`relative z-20 shrink-0 ${
                resizingIndex === index - 1 ? "bg-primary" : "bg-border hover:bg-primary/50"
              } focus-visible:bg-primary/50 ${horizontal ? "w-1 cursor-col-resize" : "h-1 cursor-row-resize"}`}
              // 터치·트랙패드에서 스크롤 제스처로 가로채이지 않게 — 이 면은 끄는 용도뿐이다.
              style={{ touchAction: "none" }}
              onPointerDown={onSeparatorPointerDown(index - 1)}
              onKeyDown={onSeparatorKeyDown(index - 1)}
              onDoubleClick={() => setLayout(evenSizes(layout))}
              title="끌어서 크기 조절 · 더블클릭하면 균등"
            >
              {/*
                보이는 선은 1px이지만 잡히는 면은 그보다 넓어야 한다. 4px짜리 과녁을 맞히려고
                커서를 겨누는 동안에는 "조절이 되긴 하나"부터 의심하게 된다 — 양옆으로 4px씩
                넓혀 실제 과녁을 12px로 만든다(선 자체는 그대로 얇다).
              */}
              <div
                aria-hidden="true"
                className={`absolute ${
                  horizontal ? "inset-y-0 -left-1 -right-1" : "inset-x-0 -top-1 -bottom-1"
                }`}
              />
            </div>
          )}
          <div
            className={`relative flex min-h-0 min-w-0 flex-col ${
              multi && group.id === layout.focusedId ? "ring-1 ring-inset ring-primary/40" : ""
            }`}
            style={{ flexGrow: group.size, flexBasis: 0 }}
            data-focused={group.id === layout.focusedId}
            data-group-id={group.id}
            onPointerDownCapture={() => focus(group.id)}
            onFocusCapture={() => focus(group.id)}
          >
            <EditorPane
              {...pane}
              files={group.keys.flatMap((k) => {
                const file = byKey.get(k);
                return file == null ? [] : [file];
              })}
              activeKey={group.activeKey}
              retainedPaths={retainedPaths}
              focused={group.id === layout.focusedId}
              shortcutsActive={shortcutsActive}
              onSelect={(key) => select(group.id, key)}
              onClose={(key) => closeInGroup(group.id, key)}
              onSplit={split}
              onCloseGroup={multi ? () => drop(group.id) : undefined}
              onTabDragStart={(key, event) => startTabDrag(group.id, key, event)}
              onTabMenu={(key, x, y) => {
                setDocumentLinkMenu(null);
                setTabMenu({
                  groupId: group.id,
                  key,
                  path: byKey.get(key)?.path ?? pathFromTabKey(key),
                  x,
                  y,
                });
              }}
              onLinkMenu={(sourceKey, sourcePath, link, at) =>
                openDocumentLinkMenu(group.id, sourceKey, sourcePath, link, at)
              }
              onOpenLink={(_sourceKey, sourcePath, link) =>
                openDocumentLink(group.id, sourcePath, link)
              }
              beginReferences={beginReferences}
              onReferences={(targets, sourcePath, request, origin) =>
                showReferences(targets, sourcePath, request, { ...origin, groupId: group.id })
              }
              sourceVersion={sourceVersion}
              navigationScope={`${host}:${pane.taskId}:${windowId}`}
              onNavigateTarget={(target, origin) => void navigation.navigate(target, { ...origin, groupId: group.id })}
              reveal={navigation.reveal == null ? pane.reveal : group.id === navigationGroupId ? navigation.reveal : null}
              onRevealed={(ack) => {
                navigation.onRevealed(ack == null ? undefined : { ...ack, groupId: group.id });
                if (!ack) pane.onRevealed?.();
              }}
            />
            {/*
              드래그 중에만 깔리는 받는 면. 늘 깔아 두면 Monaco 자신의 드래그·선택이 이 면에
              막힌다 — 코드 안에서 텍스트를 끌어 옮기는 것이 에디터의 기본 동작이다.
            */}
            {dragging != null && (
              <>
                {/*
                  탭 바 위에 놓는 것은 "이 칸에 넣어 달라"는 뜻이다. 32px짜리 띠에서 좌표로
                  재면 무엇을 하든 위쪽 가장자리로 읽혀, 옆 칸으로 옮기려던 손이 매번 새 칸을
                  만든다. 높이는 EditorPane 탭 바의 `h-8`과 짝이다 — 한쪽만 바꾸면 어긋난다.
                */}
                <div
                  className="absolute inset-x-0 top-0 z-30 h-8"
                  data-drop-strip={group.id}
                  onDragOver={onPaneDragOver(group.id, "center")}
                  onDrop={onPaneDrop(group.id, "center")}
                >
                  {/* 탭 사이 어디에 앉을지 — 이것이 없으면 놓아 본 뒤에야 순서를 안다. */}
                  {dropTarget?.groupId === group.id && dropTarget.caretX != null && (
                    <div
                      aria-hidden="true"
                      data-drop-caret={dropTarget.at}
                      className="pointer-events-none absolute inset-y-0 w-0.5 bg-primary"
                      style={{ left: dropTarget.caretX }}
                    />
                  )}
                </div>
                <div
                  className="absolute inset-x-0 bottom-0 top-8 z-30"
                  data-drop-surface={group.id}
                  onDragOver={onPaneDragOver(group.id)}
                  onDragLeave={(event) => {
                    // 자식으로 들어갈 때도 leave가 오므로, 진짜로 칸을 벗어났는지 확인한다.
                    if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
                    if (dropTarget?.groupId === group.id) setDropTarget(null);
                  }}
                  onDrop={onPaneDrop(group.id)}
                >
                  {dropTarget?.groupId === group.id && (
                    <div
                      aria-hidden="true"
                      className="pointer-events-none absolute rounded-sm border-2 border-primary bg-primary/20 transition-all duration-75"
                      data-drop-edge={dropTarget.edge}
                      style={dropZoneStyle(dropTarget.edge)}
                    />
                  )}
                </div>
              </>
            )}
          </div>
        </Fragment>
      ))}
      </div>
      {references && <div role="tablist" aria-label="코드 탐색 도구" className="flex shrink-0 gap-1 border-t border-border bg-raised px-2 py-1">
        {references && <button role="tab" aria-selected={activeTool === "references"} onClick={() => setActiveTool("references")} className="rounded px-2 py-1 text-xs text-text-muted">사용처</button>}
      </div>}
      {activeTool != null && <div className="flex min-h-0 min-w-0 shrink-0 flex-col overflow-hidden border-t border-border" style={{ height: "45%" }} aria-label="코드 탐색 결과">
        {activeTool === "references" && references && <ReferencesPanel embedded targets={references.targets} total={references.total} stale={references.stale} onOpen={(target) => navigation.navigate(target, references.origin)} onClose={closeReferences} readPreview={readReferencePreview} />}
      </div>}
      <TabContextMenu
        menu={tabMenu}
        supportsExternalPath={pane.supportsExternalPath === true}
        hasOthers={(layout.groups.find((g) => g.id === tabMenu?.groupId)?.keys.length ?? 0) > 1}
        hasContent={tabMenu != null && byKey.get(tabMenu.key)?.kind === "text"}
        hasRight={(() => {
          const group = layout.groups.find((g) => g.id === tabMenu?.groupId);
          if (group == null || tabMenu == null) return false;
          const at = group.keys.indexOf(tabMenu.key);
          return at >= 0 && at < group.keys.length - 1;
        })()}
        onAction={runTabMenu}
        onClose={() => setTabMenu(null)}
      />
      <DocumentLinkContextMenu
        menu={documentLinkMenu}
        supportsExternalPath={pane.supportsExternalPath === true}
        onAction={runDocumentLinkMenu}
        onClose={() => setDocumentLinkMenu(null)}
      />
    </div>
  );
}
