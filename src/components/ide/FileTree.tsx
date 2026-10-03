import { useEffect, useMemo, useRef, useState } from "react";
import type { FsNode } from "../../lib/ipc";
import { Icon } from "./icons";
import { FileIcon } from "./FileIcon";
import { fileGlyph, fileIcon, folderIcon, iconTint, isHiddenName } from "./file-icons";
import { initialExpanded, keyAction, toNavRows, typeAheadIndex, visibleRows } from "./tree-nav";
import { FILE_DRAG_MIME } from "./editor-drag";

interface Props {
  nodes: FsNode[];
  activePath: string | null;
  onOpen: (path: string) => void;
  /** 더블클릭 — 훑어보던 파일을 붙들어 둔다(프리뷰 해제). 없으면 더블클릭은 두 번의 열기다. */
  onPin?: (path: string) => void;
  /** 우클릭. `node`가 null이면 행이 없는 빈 자리 — 대상은 워크트리 루트다. */
  onContextMenu?: (node: FsNode | null, x: number, y: number) => void;
  /** 도트 파일/디렉터리 표시 여부. 기본은 숨김 — `.claude`·`.github`류가 목록 앞을 차지하면
   *  정작 매일 여는 `src`가 아래로 밀린다. */
  showHidden?: boolean;
}

/** worktree 파일 트리.
 *
 * 렌더는 재귀가 아니라 **평탄화된 행 배열**이다. 방향키 이동은 "다음 행"이라는 한 가지
 * 개념으로 끝나야 하는데, 재귀 컴포넌트 안에서는 그 다음 행이 어느 서브트리에 있는지
 * 알 수 없다. 펼침 상태도 각 노드가 아니라 여기 한 곳에 모아 둔다 — 트리를 새로고침해도
 * 펼친 가지가 살아남는 이유다(경로 기준이라 노드가 새 객체여도 유지된다).
 */
export function FileTree({
  nodes,
  activePath,
  onOpen,
  onPin,
  onContextMenu,
  showHidden = false,
}: Props) {
  const [expanded, setExpanded] = useState<Set<string>>(() => initialExpanded(nodes));
  /** 키보드 커서가 짚고 있는 행. -1이면 아직 아무 데도 없다(첫 키 입력이 맨 위를 잡는다). */
  const [cursor, setCursor] = useState(-1);
  const listRef = useRef<HTMLDivElement>(null);
  const seeded = useRef(nodes.length > 0);

  // 트리는 비어 있다가 나중에 채워진다(비동기 조회). 처음 도착한 그때 루트 한 겹을 펼친다.
  useEffect(() => {
    if (seeded.current || nodes.length === 0) return;
    seeded.current = true;
    setExpanded(initialExpanded(nodes));
  }, [nodes]);

  const rows = useMemo(
    () => visibleRows(nodes, expanded, showHidden),
    [nodes, expanded, showHidden],
  );
  const navRows = useMemo(() => toNavRows(rows), [rows]);

  // 접기·숨김 토글로 행이 사라지면 커서가 허공을 짚는다 — 목록 안으로 되돌린다.
  const at = cursor >= rows.length ? rows.length - 1 : cursor;

  // 커서가 화면 밖으로 나가면 따라 스크롤한다. block:"nearest"라 이미 보이는 행은 안 움직인다.
  useEffect(() => {
    if (at < 0) return;
    listRef.current?.querySelector(`[data-row="${at}"]`)?.scrollIntoView({ block: "nearest" });
  }, [at]);

  const toggle = (path: string, open: boolean) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (open) next.add(path);
      else next.delete(path);
      return next;
    });

  const handleKey = (e: React.KeyboardEvent) => {
    const action = keyAction(e.key, navRows, at, expanded);
    if (action.kind !== "none") {
      e.preventDefault();
      if (action.kind === "focus") setCursor(action.index);
      else if (action.kind === "expand") toggle(action.path, true);
      else if (action.kind === "collapse") toggle(action.path, false);
      else if (action.kind === "toggle") toggle(action.path, !expanded.has(action.path));
      else onOpen(action.path);
      return;
    }
    // 글자를 치면 그 글자로 시작하는 다음 항목으로 — 긴 목록에서 방향키보다 빠르다.
    if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
      const found = typeAheadIndex(navRows, e.key, at);
      if (found >= 0) {
        e.preventDefault();
        setCursor(found);
      }
    }
  };

  /** 행이 없는 자리의 우클릭 — 대상은 워크트리 루트다. */
  const rootMenu = (event: React.MouseEvent) => {
    if (onContextMenu == null) return;
    event.preventDefault();
    onContextMenu(null, event.clientX, event.clientY);
  };

  // 빈 워크트리에서도 우클릭은 받는다 — 새 워크트리의 첫 파일을 만들 자리가 여기뿐이다.
  if (nodes.length === 0) {
    return (
      <div className="flex-1 text-text-muted text-xs px-2 py-1 min-h-[2rem]" onContextMenu={rootMenu}>
        (빈 워크트리)
      </div>
    );
  }
  if (rows.length === 0) {
    return (
      <div className="flex-1 text-text-muted text-xs px-2 py-1 min-h-[2rem]" onContextMenu={rootMenu}>
        (숨김 항목뿐)
      </div>
    );
  }

  return (
    <div
      ref={listRef}
      role="tree"
      data-file-tree
      aria-label="파일 트리"
      aria-activedescendant={at >= 0 ? `filetree-row-${at}` : undefined}
      tabIndex={0}
      onKeyDown={handleKey}
      onContextMenu={rootMenu}
      // 크기는 --file-tree-* 한 벌에서 온다. 그 값은 설정(에디터 탭)의 글자 크기 하나에서
      // 파생되고(`lib/editor-settings.ts`), 아이콘까지 같은 변수를 읽는다 — 치수가 흩어지면
      // 글자만 커지고 행 높이가 안 따라와 디센더가 잘린다.
      style={{
        fontSize: "var(--file-tree-font-size)",
        lineHeight: "var(--file-tree-line-height)",
      }}
      // 부모가 flex 열이면 남은 높이를 채운다 — 마지막 행 아래 빈 자리도 루트 메뉴를 받아야 한다.
      className="flex-1 select-none outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-primary/50"
    >
      {rows.map(({ node, depth }, i) => {
        const open = node.is_dir && expanded.has(node.path);
        const active = node.path === activePath;
        const focused = i === at;
        const iconName = node.is_dir ? folderIcon(open) : fileIcon(node.name);
        // 디렉터리는 펼침 상태를 실루엣으로 말해야 하므로 컬러 글리프를 쓰지 않는다.
        const glyph = node.is_dir ? null : fileGlyph(node.name);
        // 틴트는 활성 행에서도 유지한다 — 종류는 선택 여부와 무관한 사실이다.
        const tint = iconTint(iconName);
        return (
          <div
            key={node.path}
            id={`filetree-row-${i}`}
            data-row={i}
            role="treeitem"
            aria-level={depth + 1}
            aria-expanded={node.is_dir ? open : undefined}
            aria-selected={active}
            title={node.path}
            onClick={() => {
              setCursor(i);
              if (node.is_dir) toggle(node.path, !open);
              else onOpen(node.path);
            }}
            // 더블클릭은 두 번의 클릭이기도 하다 — 첫 클릭이 이미 열었으므로 여기서는
            // 고정만 하면 된다. 타이머로 단·더블을 가르지 않는 이유가 그것이다.
            onDoubleClick={() => {
              if (!node.is_dir) onPin?.(node.path);
            }}
            onContextMenu={(event) => {
              if (onContextMenu == null) return;
              event.preventDefault();
              // 컨테이너까지 올라가면 빈 자리 메뉴가 이 행을 덮어쓴다.
              event.stopPropagation();
              setCursor(i);
              onContextMenu(node, event.clientX, event.clientY);
            }}
            // 파일만 끌린다. 디렉터리를 끌어 놓아도 열 것이 하나로 정해지지 않는다.
            draggable={!node.is_dir}
            onDragStart={(event) => {
              if (node.is_dir) return;
              event.dataTransfer.setData(FILE_DRAG_MIME, node.path);
              // 트리 쪽 행은 그대로 남으므로 이동이 아니라 복사다.
              event.dataTransfer.effectAllowed = "copy";
            }}
            className={`flex items-stretch cursor-pointer ${
              active
                ? "bg-surface text-primary-bright"
                : "text-text-secondary hover:bg-surface hover:text-text"
            } ${focused ? "ring-1 ring-inset ring-primary/50" : ""} ${
              isHiddenName(node.name) ? "opacity-60" : ""
            }`}
          >
            {/* 들여쓰기 가이드 — 여백 대신 세로선을 그린다. 어느 폴더에 속한 줄인지
                눈으로 따라갈 수 있어야 깊은 트리가 읽힌다(보더 기반 위계). */}
            <span className="flex shrink-0 pl-1.5" aria-hidden="true">
              {Array.from({ length: depth }, (_, d) => (
                <span
                  key={d}
                  className="border-l border-border"
                  style={{ width: "var(--file-tree-indent)" }}
                />
              ))}
            </span>
            <span className="flex items-center gap-1 py-1 pr-2 min-w-0">
              {/* size prop 은 CSS 가 없을 때의 폴백이다. 실제 크기는 index.css 의
                  `[data-file-tree] .ft-chevron > svg` 가 변수로 정한다 — 숫자를 여기 두는 것이
                  전에 갈라진 원인이었다(변수 15px 대 prop 14). */}
              <span
                className="ft-chevron shrink-0 text-text-muted"
                style={{ width: "var(--file-tree-chevron)" }}
              >
                {node.is_dir && <Icon name={open ? "chevronDown" : "chevronRight"} size={14} />}
              </span>
              <span className="ft-icon shrink-0 flex items-center">
                <FileIcon
                  glyph={glyph}
                  fallback={iconName}
                  fallbackTint={tint ?? (active ? undefined : "text-text-muted")}
                  size={16}
                />
              </span>
              {/* 디렉터리는 색(골드)과 굵기로 이중 인코딩한다 — 한 채널만으로는 스캔이 약하다. */}
              <span className={`truncate ${node.is_dir ? "font-medium" : ""}`}>{node.name}</span>
            </span>
          </div>
        );
      })}
    </div>
  );
}
