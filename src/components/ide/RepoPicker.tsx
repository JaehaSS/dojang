import { useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { ago } from "../../lib/fmt";
import { Icon } from "./icons";
import { filterByQuery, queryTokens } from "./picker-search";
import { Highlighted } from "./PickerHighlight";
import { usePickerCursor } from "./usePickerCursor";

export interface RecentRepo {
  path: string;
  lastUsed: number;
}

interface Props {
  repo: string;
  recentRepos: RecentRepo[];
  onPick: (path: string) => void;
  allowFolderPick?: boolean;
  /** 목록 섹션 헤더 — 로컬은 "최근", 원격은 "Runner 레포" 등 소스에 맞게. */
  label?: string;
  /** 메뉴가 열릴 때 호출 — 원격 목록처럼 열 때마다 새로 조회해야 하는 소스용. */
  onOpen?: () => void;
  /** "폴더 열기…"를 눌렀을 때 네이티브 다이얼로그 대신 호출 — 원격은 자체 브라우저를 띄운다. */
  onBrowse?: () => void;
  triggerLabel?: string;
  /** 메뉴가 펼쳐지는 방향 — Composer(하단)는 위로, Home 상단 같은 자리는 아래로. */
  placement?: "up" | "down";
  /** 메뉴를 트리거의 어느 변에 맞출지 — 트리거가 오른쪽 끝에 있으면 right로 화면 밖 넘침을 막는다. */
  align?: "left" | "right";
}

const base = (p: string) => p.split("/").filter(Boolean).pop() ?? p;
/** 부모 경로 — 같은 이름의 레포가 둘일 때 이것만이 둘을 가른다. */
const parent = (p: string) => p.slice(0, Math.max(0, p.length - base(p).length - 1));

/**
 * 레포 선택 드롭다운 — 최근 레포 목록 + 네이티브 "폴더 열기…".
 *
 * 검색은 **전체 경로**에 걸리고 표시는 basename + 부모 경로다: 같은 이름의 레포가 다른 경로에
 * 둘 있을 때 목록만 보고는 구분할 수 없었고, 경로 조각으로 찾히는 것이 곧 그 문제의 해법이다.
 * "폴더 열기…"는 검색 대상이 아니다 — 후보가 아니라 탈출구다(ADR 0176, 와이어프레임 0011).
 */
export function RepoPicker({
  repo,
  recentRepos,
  onPick,
  allowFolderPick = true,
  label = "최근",
  onOpen,
  onBrowse,
  triggerLabel,
  placement = "up",
  align = "left",
}: Props) {
  const [openMenu, setOpenMenu] = useState(false);
  const [query, setQuery] = useState("");
  const ref = useRef<HTMLDivElement>(null);

  const tokens = useMemo(() => queryTokens(query), [query]);
  const shown = useMemo(
    () => filterByQuery(recentRepos, query, (r) => r.path),
    [recentRepos, query],
  );

  useEffect(() => {
    if (!openMenu) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpenMenu(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [openMenu]);

  const commit = (path: string) => {
    onPick(path);
    setOpenMenu(false);
  };

  // 질의가 없으면 지금 고른 레포, 있으면 첫 매치 — 질의가 바뀔 때마다 다시 맞춘다.
  const at = shown.findIndex((r) => r.path === repo);
  const { cursor, setCursor, inputRef, listRef, onKeyDown } = usePickerCursor({
    open: openMenu,
    count: shown.length,
    initial: tokens.length === 0 && at >= 0 ? at : 0,
    resetOn: "open+query",
    query,
    onCommit: (i) => {
      const row = shown[i];
      if (row) commit(row.path);
    },
    onClose: () => setOpenMenu(false),
  });

  useEffect(() => {
    if (!openMenu) setQuery("");
  }, [openMenu]);

  const pickFolder = async () => {
    setOpenMenu(false);
    // 원격에는 네이티브 다이얼로그가 닿지 않는다 — 호출부가 준 브라우저를 대신 연다.
    if (onBrowse) {
      onBrowse();
      return;
    }
    try {
      const dir = await open({ directory: true, multiple: false, title: "레포 폴더 선택" });
      if (typeof dir === "string") onPick(dir);
    } catch {
      /* 취소/실패 무시 */
    }
  };

  return (
    <div className="relative" ref={ref}>
      <button
        className="flex items-center gap-1 text-xs text-text-secondary border border-border rounded-md px-2 py-1 hover:border-border-strong"
        onClick={() =>
          setOpenMenu((v) => {
            if (!v) onOpen?.();
            return !v;
          })
        }
        aria-expanded={openMenu}
        aria-haspopup="listbox"
        title={repo || "레포 선택"}
      >
        <Icon name="folder" size={14} />
        <span className="max-w-[160px] truncate">{repo ? base(repo) : (triggerLabel ?? "레포 미선택")}</span>
        <Icon name="chevronDown" size={12} />
      </button>

      {openMenu && (
        <div
          className={`absolute z-20 w-72 rounded-lg border border-border-strong bg-raised py-1 shadow-e2 ${
            placement === "down" ? "top-full mt-1" : "bottom-full mb-1"
          } ${align === "right" ? "right-0" : "left-0"}`}
        >
          <div className="flex items-center justify-between gap-2 px-3 py-1">
            <span className="text-xs text-text-muted">{label}</span>
            {/* 좁혀졌다는 사실을 숫자로 준다 — 스크롤 막대 길이는 근거가 아니다. */}
            <span className="text-xs text-text-muted font-code shrink-0">
              {tokens.length > 0 ? `${shown.length}/${recentRepos.length}` : recentRepos.length}
            </span>
          </div>

          <div className="px-2 pb-1">
            <div className="relative">
              <span className="absolute left-2 top-1/2 -translate-y-1/2 text-text-muted pointer-events-none">
                <Icon name="search" size={12} />
              </span>
              <input
                ref={inputRef}
                className="w-full bg-bg border border-border rounded pl-7 py-1 text-sm text-text outline-none focus:border-primary placeholder:text-text-muted"
                placeholder="레포 검색 — 경로 조각도 됩니다"
                value={query}
                role="combobox"
                aria-expanded
                aria-controls="repo-picker-list"
                aria-activedescendant={shown[cursor] ? `repo-opt-${cursor}` : undefined}
                aria-label="레포 검색"
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={onKeyDown}
              />
            </div>
          </div>

          <div
            ref={listRef}
            id="repo-picker-list"
            role="listbox"
            aria-label={label}
            className="max-h-72 overflow-auto"
          >
            {shown.map((r, i) => (
              <button
                key={r.path}
                id={`repo-opt-${i}`}
                role="option"
                aria-selected={r.path === repo}
                data-cursor={i === cursor ? "true" : undefined}
                className={`w-full text-left flex items-center gap-2 px-3 py-1.5 text-sm ${
                  i === cursor ? "bg-surface text-text" : r.path === repo ? "text-text" : "text-text-secondary"
                }`}
                onMouseEnter={() => setCursor(i)}
                onClick={() => commit(r.path)}
                title={r.path}
              >
                <span className="truncate flex-1 flex items-baseline gap-1.5 min-w-0">
                  <span className="truncate shrink-0 max-w-[55%]">
                    <Highlighted text={base(r.path)} tokens={tokens} />
                  </span>
                  {/* 같은 basename 둘을 가르는 것은 이 한 줄이다. */}
                  <span className="truncate text-text-muted text-xs font-code">
                    <Highlighted text={parent(r.path)} tokens={tokens} />
                  </span>
                </span>
                {r.path === repo ? (
                  <span className="text-primary-bright shrink-0">
                    <Icon name="check" size={14} />
                  </span>
                ) : (
                  // 원격 발견 목록은 lastUsed가 없다(0) — 무의미한 시각 표시를 숨긴다.
                  r.lastUsed > 0 && (
                    <span className="text-text-muted text-xs font-code shrink-0">{ago(r.lastUsed)}</span>
                  )
                )}
              </button>
            ))}

            {shown.length === 0 && tokens.length > 0 && (
              <div className="px-3 py-2 space-y-1">
                <div className="text-sm text-text-muted">
                  '{query.trim()}'와 일치하는 최근 레포가 없습니다
                </div>
                <div className="text-sm text-text-muted">
                  폴더 열기…로 다른 레포를 고를 수 있습니다
                </div>
              </div>
            )}
          </div>

          {allowFolderPick && (
            <>
              <div className="my-1 border-t border-border" />
              <button
                className="w-full text-left flex items-center gap-2 px-3 py-1.5 text-sm text-text-secondary hover:bg-surface"
                onClick={pickFolder}
              >
                <Icon name="folder" size={14} /> 폴더 열기…
              </button>
            </>
          )}
        </div>
      )}
    </div>
  );
}
