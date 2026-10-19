import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { copyText } from "../../lib/clipboard";
import { useHostScope } from "../../lib/host-scope";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  fsBrowse,
  fsCopy,
  fsCreateDir,
  fsCreateFile,
  fsOpenTerminal,
  fsRename,
  fsRoots,
  fsTrash,
  type DirEntry,
  type Task,
} from "../../lib/ipc";
import { fileSize, fileTime } from "../../lib/fmt";
import { actionDir, mutationBlock, parentPath } from "../../lib/file-ops";
import { Icon } from "./icons";
import { fileGlyph, fileIcon, folderIcon, iconTint, isHiddenName } from "./file-icons";
import { FileIcon } from "./FileIcon";
import { FileContextMenu, type FileMenuAction, type FileMenuState } from "./FileContextMenu";
import { FilePromptDialog } from "./FilePromptDialog";
import { keyAction, typeAheadIndex, type NavRow } from "./tree-nav";
import { splitLabel } from "./file-label";

export type SortKey = "name" | "mtime";

interface Props {
  /** 처음 열 경로. 비우면 transport의 첫 root(원격은 Runner root, 로컬은 홈). */
  initialPath?: string;
  /** 파일을 눌렀을 때 — 미지정이면 파일은 선택만 되고 아무 일도 하지 않는다. */
  onOpenFile?: (path: string) => void;
  /** 현재 대상 디렉터리가 바뀔 때마다 — 루트 이동과 폴더 클릭 둘 다. 모달의 "이 폴더
   * 선택"이 쓴다. 하이라이트된 행이 곧 대상이므로 눈에 보이는 것과 어긋나지 않는다. */
  onPathChange?: (path: string) => void;
  /** 각 행 우측에 붙일 액션(레포 고르기 등). */
  renderEntryAction?: (entry: DirEntry) => React.ReactNode;
  /** 우클릭 조작 허용 여부. 미지정이면 읽기 전용으로 동작한다 — 모달의 폴더 선택기처럼
   *  조작이 필요 없는 곳은 그대로 둔다. 원격 transport에서도 false다(설계 0024 D4). */
  mutable?: boolean;
  /** 가드 판정용 활성 작업 — 호출부가 ACTIVE_STATES로 걸러 넘긴다. */
  activeTasks?: readonly Task[];
  className?: string;
}

/** 트리에 펼쳐진 행 하나 — 평탄화 결과.
 *
 * `chain`은 한 줄로 접힌 단일 자식 디렉터리 사슬(`src`→`main`→`java`)이고 길이는 1 이상이다.
 * `entry`는 언제나 그 **끝**(가장 깊은 것) — 선택·컨텍스트 메뉴·이동이 모두 "이 행이
 * 대표하는 디렉터리는 하나"라는 규칙 하나만 보면 되게 한다. */
export interface Row {
  entry: DirEntry;
  depth: number;
  chain: DirEntry[];
}

/** 접힌 사슬의 표시 라벨 — `src/main/java`. */
export const chainLabel = (chain: DirEntry[]) => chain.map((e) => e.name).join("/");

/** 폭이 이 값 미만이면 크기·수정 열을 접는다.
 *
 * 두 열이 160px을 예약하는데, 320px 패널에서는 이름 몫이 78px까지 줄고
 * 조금만 깊어지면 0이 된다 — 이름이 통째로 사라지는 것보다 열을 내리는 쪽이 낫다.
 * 640px 모달의 폴더 선택기는 이 값을 넘어 기존 3열을 그대로 쓴다. */
export const DENSE_MAX_WIDTH = 480;

/** 행이 이동/선택 대상이 될 수 있는 디렉터리면 그 경로, 아니면 null.
 *
 * 파일과 읽기 권한 없는 폴더를 한 자리에서 걸러 클릭·더블클릭이 같은 기준을 쓰게 한다. */
export const dirTarget = (entry: DirEntry) =>
  entry.is_dir && !entry.denied ? entry.path : null;

export const sortEntries = (entries: DirEntry[], sort: SortKey, desc: boolean) =>
  [...entries].sort((a, b) => {
    // 디렉터리 우선은 정렬 키와 무관하게 유지 — 탐색기 관례.
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    const cmp = sort === "name" ? a.name.localeCompare(b.name) : a.mtime - b.mtime;
    return desc ? -cmp : cmp;
  });

/** 사슬을 이어붙일 수 있는 유일한 자식, 아니면 null.
 *
 * 자식이 아직 로드되지 않았거나, 둘 이상이거나, 하나뿐이어도 파일이거나 들어갈 수 없는
 * 폴더면 거기서 사슬이 끊긴다 — 그런 자식은 자기 행을 가져야 한다.
 * 숨김 필터를 **먼저** 적용한다. 화면에 하나로 보이는 것과 압축 판정이 어긋나면 안 된다. */
const soleDirChild = (
  children: Record<string, DirEntry[]>,
  dir: string,
  showHidden: boolean,
): DirEntry | null => {
  const kids = children[dir];
  if (!kids) return null;
  const visible = showHidden ? kids : kids.filter((e) => !isHiddenName(e.name));
  if (visible.length !== 1) return null;
  const only = visible[0];
  return only.is_dir && !only.denied ? only : null;
};

/** 열을 접었을 때의 title — 화면에서 뺀 값은 여기에 담는다. 좁아서 감춘 것이지 버린 것이
 *  아니다. `fileTime`은 mtime이 0이면 빈 문자열이라 그 조각은 이어붙이지 않는다. */
const denseTitle = (base: string, entry: DirEntry) =>
  [base, entry.is_dir ? "" : fileSize(entry.size), fileTime(entry.mtime)]
    .filter(Boolean)
    .join(" · ");

/** 펼쳐진 가지를 따라 트리를 행 목록으로 평탄화한다.
 *
 * 아직 자식을 받지 못한 디렉터리는 `children`에 없으므로 자연히 건너뛴다 — 로딩 중에도
 * 이미 그려진 가지는 그대로 유지된다. */
export function flattenTree(
  children: Record<string, DirEntry[]>,
  root: string,
  expanded: ReadonlySet<string>,
  sort: SortKey,
  desc: boolean,
  showHidden = true,
): Row[] {
  const rows: Row[] = [];
  const visited = new Set<string>();
  const walk = (dir: string, depth: number) => {
    // 같은 경로가 자기 조상으로 다시 나타나는 병적 케이스(하드링크 루프) 방어.
    if (visited.has(dir)) return;
    visited.add(dir);
    for (const entry of sortEntries(children[dir] ?? [], sort, desc)) {
      if (!showHidden && isHiddenName(entry.name)) continue;
      // 자식이 하나뿐인 디렉터리는 그 자식과 한 줄로 접는다 — `src/main/java/com/a`가
      // 다섯 단계를 먹고 나면 좁은 패널에서 이름 몫이 남지 않는다. 펼쳐져 있고 자식이
      // 로드된 동안만 이어지므로, 사용자가 접으면 그 자리에서 다시 풀린다.
      const chain = [entry];
      let tail = entry;
      while (tail.is_dir && expanded.has(tail.path) && !visited.has(tail.path)) {
        const only = soleDirChild(children, tail.path, showHidden);
        if (!only) break;
        // 사슬에 삼킨 디렉터리는 방문 처리한다 — 순환이면 다음 바퀴에서 걸린다.
        visited.add(tail.path);
        chain.push(only);
        tail = only;
      }
      rows.push({ entry: tail, depth, chain });
      if (tail.is_dir && expanded.has(tail.path)) walk(tail.path, depth + 1);
    }
  };
  walk(root, 0);
  return rows;
}

/** 원격/로컬 디렉터리 트리 — 펼친 디렉터리만 조회하는 지연 로딩 브라우저.
 *
 * 전체 트리를 한 번에 받는 `FileTree`와 달리 방문한 디렉터리만 요청하므로, 홈처럼
 * 수십만 파일이 달린 경로도 즉시 열린다. 정렬은 `ls -lrt`처럼 이름/수정시각을 토글한다.
 * 폴더는 한 번 눌러 제자리에서 펼치고, 더블클릭하면 탐색기처럼 그 안으로 들어간다. */
export function DirectoryBrowser({
  initialPath,
  onOpenFile,
  onPathChange,
  renderEntryAction,
  mutable = false,
  activeTasks,
  className = "",
}: Props) {
  // 파일 브라우저는 세션에 속하지 않는다 — 어느 머신을 보는지는 스코프가 정한다 (ADR 0133).
  const host = useHostScope();
  const [root, setRoot] = useState("");
  const [parent, setParent] = useState<string | null>(null);
  const [roots, setRoots] = useState<string[]>([]);
  /** 디렉터리 경로 → 자식 목록. 루트도 같은 맵에 담아 렌더를 한 갈래로 유지한다. */
  const [children, setChildren] = useState<Record<string, DirEntry[]>>({});
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState<Set<string>>(new Set());
  const [sort, setSort] = useState<SortKey>("name");
  const [desc, setDesc] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** 도트 파일 표시 — 기본은 감춤. 프로젝트를 훑을 때 `.git*`·`.venv`류가 먼저 나오면
   *  정작 찾는 디렉터리가 스크롤 아래로 밀린다. */
  const [showHidden, setShowHidden] = useState(false);
  /** 키보드 커서가 짚은 행. 선택(selected)과 달리 "여기까지 왔다"만 뜻한다 —
   *  실제 선택은 Enter로 확정해야 "이 폴더 선택"의 대상이 방향키에 휩쓸리지 않는다. */
  const [cursor, setCursor] = useState(-1);
  /** 우클릭 메뉴 — 좌표와 대상 행. */
  const [menu, setMenu] = useState<FileMenuState | null>(null);
  /** 앱 내부 클립보드. OS 클립보드와는 연동하지 않는다(설계 0024 Scope). */
  const [clipboard, setClipboard] = useState<string | null>(null);
  /** `path`는 이름 변경 대상(생성 계열에서는 안 쓴다), `dir`은 결과를 갱신할 디렉터리. */
  const [prompt, setPrompt] = useState<{
    kind: "newFile" | "newDir" | "rename";
    initial: string;
    dir: string;
    path: string;
  } | null>(null);
  const [promptError, setPromptError] = useState<string | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  /** 측정 전 기본값은 좁은 쪽이다 — 넓다고 가정하면 첫 페인트에 열이 보였다 사라진다. */
  const [dense, setDense] = useState(true);
  /** 한 번이라도 자동 확장한 경로.
   *
   * 압축은 자식이 로드돼야 판정되므로 effect가 fetch를 주도하는데, 가드가 없으면
   * 사용자가 사슬 끝을 접는 즉시 effect가 다시 펴서 **사용자와 싸운다**. 경로당 한 번만
   * 펴고, 그 뒤로는 접힌 채 둔다. */
  const autoExpandedRef = useRef<Set<string>>(new Set());

  const mark = (set: Set<string>, path: string, on: boolean) => {
    const next = new Set(set);
    if (on) next.add(path);
    else next.delete(path);
    return next;
  };

  /** 자식 목록을 캐시에 채운다. 이미 있으면 재요청하지 않는다(refresh는 캐시를 비우고 호출). */
  const fetchChildren = useCallback((path: string) => {
    setLoading((set) => mark(set, path, true));
    return fsBrowse(host, path)
      .then((result) => {
        setChildren((map) => ({ ...map, [result.path]: result.entries }));
        return result;
      })
      .catch((e) => {
        setError(String(e));
        return null;
      })
      .finally(() => setLoading((set) => mark(set, path, false)));
  }, []);

  /** 한 디렉터리만 다시 읽는다 — 펼침 상태와 다른 가지는 건드리지 않는다.
   *  트리 전체 refetch는 펼친 가지를 접는다(PRD §7 Performance). */
  const refreshDir = useCallback(
    (dir: string) => {
      setChildren((map) => {
        const next = { ...map };
        delete next[dir];
        return next;
      });
      return fetchChildren(dir);
    },
    [fetchChildren],
  );

  /** 트리의 루트를 옮긴다 — 펼침 상태와 캐시는 새 루트 기준으로 초기화한다. */
  const openRoot = useCallback(
    (target: string) => {
      setError(null);
      setLoading((set) => mark(set, target, true));
      fsBrowse(host, target)
        .then((result) => {
          setRoot(result.path);
          setParent(result.parent);
          setChildren({ [result.path]: result.entries });
          setExpanded(new Set());
          // 펼침을 비우는 자리에서 자동 확장 기록도 함께 비운다 — 남기면 새 루트에서
          // 같은 경로를 다시 만났을 때 압축이 한 번도 일어나지 않는다.
          autoExpandedRef.current = new Set();
          onPathChange?.(result.path);
        })
        .catch((e) => setError(String(e)))
        .finally(() => setLoading((set) => mark(set, target, false)));
    },
    [onPathChange],
  );

  useEffect(() => {
    openRoot(initialPath ?? "");
    fsRoots(host)
      .then(setRoots)
      .catch(() => setRoots([]));
    // 최초 1회만 — 이후 이동은 openRoot()가 담당한다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 열을 접을지는 창 크기가 아니라 **이 패널이 실제로 받은 폭**이 정한다 — 같은 창에서도
  // 사이드 패널과 모달의 사정이 다르다.
  useEffect(() => {
    const el = rootRef.current;
    if (el == null || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      const width = entries[0]?.contentRect.width;
      if (typeof width === "number") setDense(width < DENSE_MAX_WIDTH);
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // 압축이 지연 로딩을 끌고 간다 — 펼치지 않은 디렉터리는 자식이 하나뿐인지 알 수 없으므로,
  // 사슬이 이어질 자리를 발견하면 이쪽에서 먼저 펴고 받아온다.
  useEffect(() => {
    for (const dir of expanded) {
      const only = soleDirChild(children, dir, showHidden);
      if (!only || autoExpandedRef.current.has(only.path)) continue;
      autoExpandedRef.current.add(only.path);
      setExpanded((set) => mark(set, only.path, true));
      if (!children[only.path]) void fetchChildren(only.path);
    }
  }, [children, expanded, showHidden, fetchChildren]);

  const toggleDir = (path: string) => {
    const isOpen = expanded.has(path);
    setExpanded((set) => mark(set, path, !isOpen));
    if (!isOpen && !children[path]) void fetchChildren(path);
  };

  const activate = (entry: DirEntry) => {
    setSelected(entry.path);
    const target = dirTarget(entry);
    // 폴더를 고르면 그 자리가 곧 대상이다 — 하이라이트만 옮기고 경로는 루트에 남겨두면
    // "이 폴더 선택"이 엉뚱하게 루트를 집어간다.
    if (target) {
      onPathChange?.(target);
      toggleDir(target);
    } else if (!entry.is_dir) onOpenFile?.(entry.path);
  };

  /** 폴더 안으로 들어간다 — 루트를 옮겨 깊은 경로도 들여쓰기 없이 한 화면에 담긴다. */
  const enterDir = (entry: DirEntry) => {
    const target = dirTarget(entry);
    if (target) openRoot(target);
  };

  /** 그 디렉터리에 이미 있는 이름들 — 다이얼로그의 중복 검사에 쓴다. */
  const namesIn = (dir: string) => new Set((children[dir] ?? []).map((e) => e.name));

  /** 만들어진 것을 트리에 드러낸다 — 그 디렉터리를 다시 읽고, 접혀 있으면 펼친다.
   *
   * 펼치지 않으면 접힌 폴더에 우클릭해 만든 파일이 캐시에만 들어가 화면에는 아무 일도
   * 일어나지 않는다. AC는 "생성한 항목이 트리에 즉시 나타나고 선택 상태가 됨"이다. */
  const revealIn = async (dir: string, created: string) => {
    // 루트는 펼침 집합과 무관하게 늘 그려진다 — 넣으면 접기 토글만 어긋난다.
    if (dir !== root) setExpanded((set) => mark(set, dir, true));
    await refreshDir(dir);
    setSelected(created);
  };

  /** 컨텍스트 메뉴 액션. 성공은 무음이고 실패만 기존 error 자리에 낸다(PRD §6.3 Feedback). */
  const runAction = async (action: FileMenuAction, entry: DirEntry) => {
    try {
      if (action === "open") return activate(entry);
      if (action === "terminal") return void (await fsOpenTerminal(entry.path));
      if (action === "reveal") return void (await revealItemInDir(entry.path));
      if (action === "copyPath") return void (await copyText(entry.path));
      if (action === "copy") return setClipboard(entry.path);

      // 여기부터는 전부 변경 계열 — 어느 디렉터리를 건드리는지가 액션마다 다르다.
      const dir = actionDir(action, entry.path, entry.is_dir);
      if (action === "paste" || action === "duplicate") {
        const src = action === "paste" ? clipboard : entry.path;
        if (!src) return;
        // 재귀 복사는 길어질 수 있다 — 기존 loading 표시를 재사용한다(PRD §6.3 Loading).
        setLoading((set) => mark(set, dir, true));
        try {
          await revealIn(dir, await fsCopy(src, dir));
        } finally {
          setLoading((set) => mark(set, dir, false));
        }
      } else if (action === "trash") {
        // 확인 모달을 두지 않는다(PRD D6) — 되돌릴 지점이 휴지통 자체다.
        await fsTrash(entry.path);
        // 사라진 폴더의 펼침 상태를 남기면 같은 이름이 다시 생겼을 때 엉뚱하게 펼쳐진다.
        if (entry.is_dir) setExpanded((set) => mark(set, entry.path, false));
        await refreshDir(dir);
        if (selected === entry.path) setSelected(null);
      } else if (action === "rename") {
        setPromptError(null);
        setPrompt({ kind: "rename", initial: entry.name, dir, path: entry.path });
      } else if (action === "newFile" || action === "newDir") {
        setPromptError(null);
        setPrompt({ kind: action, initial: "", dir, path: entry.path });
      }
    } catch (e) {
      setError(String(e));
    }
  };

  const rows = flattenTree(children, root, expanded, sort, desc, showHidden);
  const navRows: NavRow[] = useMemo(
    () =>
      rows.map(({ entry, depth, chain }) => ({
        path: entry.path,
        depth,
        // 권한 없는 폴더는 펼칠 수 없으니 이동 규칙에서는 파일처럼 다룬다.
        isDir: dirTarget(entry) != null,
        // 타입어헤드는 보이는 글자로 점프해야 한다 — 접힌 행은 `src/main`이 보인다.
        name: chainLabel(chain),
      })),
    [rows],
  );

  // 행이 사라지면(접기·숨김 토글) 커서가 목록 밖을 짚는다 — 안으로 되돌린다.
  const at = cursor >= rows.length ? rows.length - 1 : cursor;

  useEffect(() => {
    if (at < 0) return;
    listRef.current?.querySelector(`[data-row="${at}"]`)?.scrollIntoView({ block: "nearest" });
  }, [at]);

  const handleKey = (e: React.KeyboardEvent) => {
    const action = keyAction(e.key, navRows, at, expanded);
    if (action.kind !== "none") {
      e.preventDefault();
      if (action.kind === "focus") setCursor(action.index);
      else if (action.kind === "expand" || action.kind === "collapse") toggleDir(action.path);
      else if (rows[at]) activate(rows[at].entry); // toggle·open 둘 다 클릭과 같은 동선으로.
      return;
    }
    if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
      const found = typeAheadIndex(navRows, e.key, at);
      if (found >= 0) {
        e.preventDefault();
        setCursor(found);
      }
    }
  };

  const toggleSort = (key: SortKey) => {
    if (sort === key) setDesc((v) => !v);
    else {
      setSort(key);
      // 수정시각은 최신이 위로 오는 게 기본(ls -lrt의 역순 감각).
      setDesc(key === "mtime");
    }
  };

  const sortMark = (key: SortKey) => (sort === key ? (desc ? " ↓" : " ↑") : "");
  const rootBusy = loading.has(root) || (!root && loading.size > 0);

  return (
    <div ref={rootRef} className={`flex flex-col min-h-0 ${className}`}>
      <div className="flex items-center gap-1 px-2 py-1.5 border-b border-border shrink-0">
        <button
          className="p-1 text-text-muted hover:text-text disabled:opacity-40"
          disabled={!parent || rootBusy}
          onClick={() => parent && openRoot(parent)}
          title="상위 폴더"
          aria-label="상위 폴더"
        >
          <Icon name="chevronLeft" size={14} />
        </button>
        {roots.length > 0 && (
          <button
            className="p-1 text-text-muted hover:text-text"
            onClick={() => openRoot(roots[0])}
            title={roots[0]}
            aria-label="최상위로"
          >
            <Icon name="home" size={14} />
          </button>
        )}
        <span
          className="flex-1 min-w-0 truncate text-xs font-code text-text-secondary"
          title={root}
          aria-label="현재 경로"
        >
          {root || "…"}
        </span>
        <button
          className={`p-1 ${showHidden ? "text-primary-bright" : "text-text-muted hover:text-text"}`}
          onClick={() => setShowHidden((v) => !v)}
          title={showHidden ? "숨김 항목 감추기" : "숨김 항목 보기 (.*)"}
          aria-label="숨김 항목"
          aria-pressed={showHidden}
        >
          <Icon name={showHidden ? "eye" : "eyeOff"} size={14} />
        </button>
        <button
          className="p-1 text-text-muted hover:text-text disabled:opacity-40"
          disabled={rootBusy}
          onClick={() => openRoot(root)}
          title="새로고침"
          aria-label="새로고침"
        >
          <Icon name="refresh" size={14} />
        </button>
      </div>

      <div className="flex items-center gap-2 px-3 py-1 border-b border-border text-xs text-text-muted shrink-0">
        <button className="flex-1 text-left hover:text-text" onClick={() => toggleSort("name")}>
          이름{sortMark("name")}
        </button>
        {/* 크기는 정렬 키가 아니다(SortKey에 없다) — 버튼으로 두면 눌러도 수정시각이
            토글돼 정렬되는 척만 한다. 열 머리 라벨로 남긴다. */}
        {!dense && <span className="w-16 text-right shrink-0">크기</span>}
        {/* 열을 접어도 정렬 토글은 남긴다 — 좁다고 정렬을 잃을 이유는 없다. */}
        <button
          className={`hover:text-text ${dense ? "shrink-0" : "w-24 text-right"}`}
          onClick={() => toggleSort("mtime")}
        >
          수정{sortMark("mtime")}
        </button>
        {renderEntryAction && <span className="w-14 shrink-0" />}
      </div>

      <div
        ref={listRef}
        className="flex-1 overflow-auto min-h-0 outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-primary/50"
        role="tree"
        aria-label="디렉터리 트리"
        aria-activedescendant={at >= 0 ? `dirbrowser-row-${at}` : undefined}
        tabIndex={0}
        onKeyDown={handleKey}
      >
        {error ? (
          <div className="px-3 py-4 text-xs text-status-failed">{error}</div>
        ) : rows.length === 0 && rootBusy ? (
          <div className="px-3 py-4 text-xs text-text-muted">불러오는 중…</div>
        ) : rows.length === 0 ? (
          <div className="px-3 py-4 text-xs text-text-muted">빈 디렉터리</div>
        ) : (
          rows.map(({ entry, depth, chain }, i) => {
            const open = expanded.has(entry.path);
            const icon = entry.is_dir ? folderIcon(open) : fileIcon(entry.name);
            const [head, tail] = splitLabel(chainLabel(chain));
            const base = entry.denied
              ? `${entry.path} (읽기 권한 없음)`
              : entry.is_dir
                ? `${entry.path} — 더블클릭으로 이동`
                : entry.path;
            return (
              <div
                key={entry.path}
                id={`dirbrowser-row-${i}`}
                data-row={i}
                role="treeitem"
                aria-expanded={entry.is_dir ? open : undefined}
                aria-level={depth + 1}
                aria-selected={selected === entry.path}
                onContextMenu={(e) => {
                  if (!mutable) return;
                  e.preventDefault();
                  setCursor(i);
                  setSelected(entry.path);
                  setMenu({ x: e.clientX, y: e.clientY, entry });
                }}
                className={`flex items-stretch gap-2 pr-3 text-sm ${
                  selected === entry.path ? "bg-surface" : "hover:bg-surface"
                } ${i === at ? "ring-1 ring-inset ring-primary/50" : ""} ${
                  isHiddenName(entry.name) ? "opacity-60" : ""
                }`}
              >
                {/* 들여쓰기 가이드 — 여백만으로는 몇 단계 안인지 세어야 한다. 선을 그으면
                    어느 폴더에 매달린 줄인지 눈으로 따라가진다. */}
                <span className="flex shrink-0 pl-1.5" aria-hidden="true">
                  {Array.from({ length: depth }, (_, d) => (
                    <span key={d} className="w-3.5 border-l border-border" />
                  ))}
                </span>
                <button
                  className={`flex items-center gap-1.5 flex-1 min-w-0 text-left py-1 ${
                    entry.denied ? "text-text-muted" : "text-text-secondary"
                  }`}
                  onClick={() => {
                    setCursor(i);
                    activate(entry);
                  }}
                  onDoubleClick={() => enterDir(entry)}
                  title={dense ? denseTitle(base, entry) : base}
                  disabled={entry.denied && entry.is_dir}
                >
                  <span className="shrink-0 w-3.5 text-text-muted">
                    {entry.is_dir && !entry.denied && (
                      <Icon name={open ? "chevronDown" : "chevronRight"} size={12} />
                    )}
                  </span>
                  {/* 권한 없는 행은 틴트를 건너뛴다 — 죽은 행이 정상 폴더 색으로 보이면
                      상태 신호가 타입 신호에 덮인다. **컬러 글리프도 같은 이유로 끈다**:
                      밝은 언어 로고는 muted 틴트보다 더 살아 있어 보인다. */}
                  <span className="shrink-0 flex items-center">
                    <FileIcon
                      glyph={entry.denied || entry.is_dir ? null : fileGlyph(entry.name)}
                      fallback={icon}
                      fallbackTint={
                        entry.denied ? "text-text-muted" : (iconTint(icon) ?? "text-text-muted")
                      }
                      size={14}
                    />
                  </span>
                  {/* 이름은 머리·꼬리로 갈라 렌더한다. flex의 수축은 (계수 x 기본 크기)에
                      비례하므로 머리에 9999를 주면 수축을 머리가 거의 다 흡수하고, 꼬리는
                      머리가 0이 된 뒤에야 줄어든다 — `.prod.yml` 같은 구분자가 살아남는다.
                      shrink-0으로 "단순화"하면 긴 이름이 폭을 넘겨 열을 밀어낸다.
                      꼬리에도 truncate를 두는 건 사슬 라벨의 마지막 폴더가 아주 길 때
                      넘쳐 잘리는 대신 말줄임으로 물러나게 하기 위함이다. */}
                  <span className="flex min-w-0">
                    <span className="min-w-0 shrink-[9999] truncate">{head}</span>
                    {tail && <span className="min-w-0 shrink truncate">{tail}</span>}
                  </span>
                  {entry.is_repo && (
                    <span className="shrink-0 text-[10px] px-1 rounded border border-border text-primary-bright">
                      git
                    </span>
                  )}
                  {loading.has(entry.path) && (
                    <span className="shrink-0 text-[10px] text-text-muted">…</span>
                  )}
                </button>
                {!dense && (
                  <>
                    <span className="w-16 self-center text-right text-xs font-code text-text-muted shrink-0">
                      {entry.is_dir ? "" : fileSize(entry.size)}
                    </span>
                    <span className="w-24 self-center text-right text-xs font-code text-text-muted shrink-0">
                      {fileTime(entry.mtime)}
                    </span>
                  </>
                )}
                {renderEntryAction && (
                  <span className="w-14 self-center shrink-0 text-right">
                    {renderEntryAction(entry)}
                  </span>
                )}
              </div>
            );
          })
        )}
      </div>

      {/* 가드 판정은 대상 자신과 부모 둘 다 본다 — 액션마다 건드리는 디렉터리가 다르므로
          (`actionDir`) 하나로 특정할 수 없다. 어느 하나라도 걸리면 막는 쪽이 안전하다. */}
      <FileContextMenu
        menu={menu}
        block={
          menu
            ? (mutationBlock(menu.entry.path, activeTasks ?? []) ??
              mutationBlock(parentPath(menu.entry.path), activeTasks ?? []))
            : null
        }
        readOnly={!mutable}
        clipboard={clipboard}
        onAction={(action) => menu && void runAction(action, menu.entry)}
        onClose={() => setMenu(null)}
      />
      <FilePromptDialog
        open={prompt !== null}
        title={
          prompt?.kind === "rename"
            ? "새 이름"
            : prompt?.kind === "newDir"
              ? "새 폴더 이름"
              : "새 파일 이름"
        }
        subtitle={prompt?.dir ?? ""}
        initial={prompt?.initial ?? ""}
        taken={prompt ? namesIn(prompt.dir) : new Set()}
        confirmLabel={prompt?.kind === "rename" ? "이름 변경" : "만들기"}
        serverError={promptError}
        onCancel={() => {
          setPrompt(null);
          setPromptError(null);
        }}
        onConfirm={(name) => {
          const p = prompt;
          if (!p) return;
          setPromptError(null);
          const call =
            p.kind === "rename"
              ? fsRename(p.path, name)
              : p.kind === "newDir"
                ? fsCreateDir(p.dir, name)
                : fsCreateFile(p.dir, name);
          call
            .then(async (created) => {
              setPrompt(null);
              await revealIn(p.dir, created);
            })
            // PRD §6.2 S-02 — 백엔드 거부는 다이얼로그를 닫지 않고 인라인으로 보여준다.
            .catch((e) => setPromptError(String(e)));
        }}
      />
    </div>
  );
}
