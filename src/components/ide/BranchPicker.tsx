import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { MetaTag } from "../MetaTag";
import { Icon } from "./icons";
import { filterByQuery, matchRanges, queryTokens } from "./picker-search";
import { Highlighted } from "./PickerHighlight";
import { usePickerCursor } from "./usePickerCursor";

interface Props {
  /** 고른 시작 브랜치. 빈 값이면 레포의 현재 체크아웃(`current`)을 따른다. */
  value: string;
  /** 레포가 지금 체크아웃한 브랜치 — 고르지 않았을 때 실제로 쓰이는 base. */
  current: string;
  /** 최근 커밋순 로컬 브랜치. */
  branches: string[];
  onPick: (branch: string) => void;
  /** 직접 실행이면 선택한 브랜치로 작업 생성 전에 메인 체크아웃을 전환한다. */
  direct?: boolean;
  /** 작업 생성과 별개로 메인 체크아웃을 즉시 전환한다. 없으면 행에 체크아웃 버튼을 두지 않는다. */
  onCheckout?: (branch: string) => void;
  /** 전환이 진행 중이면 다른 전환을 받지 않는다. */
  checkingOut?: boolean;
}

/** 세 피커가 같은 규칙을 쓴다 — 구현은 `picker-search.ts`이고 여기 이름은 호출부 호환용 별칭이다. */
export const branchQueryTokens = queryTokens;
export const filterBranches = (branches: string[], query: string): string[] =>
  filterByQuery(branches, query, (branch) => branch);
export const branchMatchRanges = matchRanges;

/**
 * 시작 브랜치 선택 드롭다운 — 격리 실행은 여기서 worktree를 분기하고, 직접 실행은 메인
 * 체크아웃을 여기로 전환한 뒤 작업을 시작한다.
 *
 * 격리 실행의 승인 머지는 이 base로 들어간다(`merge_for_approval`). 직접 실행만 선택 브랜치를
 * 체크아웃하므로, 현재 체크아웃과 다를 때 전환을 미리 알린다.
 *
 * 브랜치는 수백 개가 될 수 있으므로 메뉴 최상단에 검색 필드를 **상시** 둔다 — 개수에 따라
 * 나타났다 사라지면 "열고 바로 타이핑"이 습관이 되지 못한다. 걸러낸 결과는 재정렬하지
 * 않는다: 원래 순서(최근 커밋순)가 곧 "방금 만든 브랜치가 위"라는 뜻이다.
 * 와이어프레임: `docs/designs/wireframes/0010-branch-picker-search.md`.
 */
export function BranchPicker({
  value,
  current,
  branches,
  onPick,
  direct = false,
  onCheckout,
  checkingOut = false,
}: Props) {
  const [openMenu, setOpenMenu] = useState(false);
  const [query, setQuery] = useState("");
  const ref = useRef<HTMLDivElement>(null);

  const selected = value || current;
  // 직접 실행만 고른 브랜치로 체크아웃한다.
  const needsCheckout = direct && !!selected && !!current && selected !== current;

  const tokens = useMemo(() => branchQueryTokens(query), [query]);
  const shown = useMemo(() => filterBranches(branches, query), [branches, query]);

  useEffect(() => {
    if (!openMenu) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpenMenu(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [openMenu]);

  const commit = (branch: string) => {
    onPick(branch);
    setOpenMenu(false);
  };

  // 이미 체크아웃된 브랜치는 전환할 것이 없다.
  const canCheckout = (branch: string) => !!onCheckout && !checkingOut && branch !== current;
  const checkout = (branch: string) => {
    if (!canCheckout(branch)) return;
    onCheckout?.(branch);
    setOpenMenu(false);
  };

  // 커서는 열 때만 지금 고른 값 위에 선다 — 목록이 갱신될 때마다 되돌리면 키보드 이동이 무효가 된다.
  const at = branches.indexOf(selected);
  const { cursor, setCursor, inputRef, listRef, onKeyDown: onCursorKeyDown } = usePickerCursor({
    open: openMenu,
    count: shown.length,
    initial: at >= 0 ? at : 0,
    resetOn: "open",
    query,
    onCommit: (i) => {
      const branch = shown[i];
      if (branch) commit(branch);
    },
    onClose: () => setOpenMenu(false),
  });

  // Shift+Enter는 고르는 대신 커서 행으로 체크아웃한다. 나머지 키는 커서 훅이 맡는다.
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" && e.shiftKey && onCheckout) {
      e.preventDefault();
      e.stopPropagation();
      const branch = shown[cursor];
      if (branch) checkout(branch);
      return;
    }
    onCursorKeyDown(e);
  };

  // 열리면 곧장 검색으로 손이 간다(훅이 포커스한다). 닫으면 질의를 버린다.
  useEffect(() => {
    if (!openMenu) setQuery("");
  }, [openMenu]);

  // 원격 추적 브랜치는 base가 될 수 없다. 그걸 찾고 있었다면 그게 진짜 원인이므로 먼저 말한다.
  const looksRemote = /origin\/|remotes\//i.test(query);
  const emptyLines = looksRemote
    ? ["원격 추적 브랜치(origin/…)는 base가 될 수 없습니다.", `'${query.trim()}'와 일치하는 로컬 브랜치가 없습니다.`]
    : [`'${query.trim()}'와 일치하는 로컬 브랜치가 없습니다.`, "원격 추적 브랜치(origin/…)는 base가 될 수 없습니다."];

  return (
    <div className="relative" ref={ref}>
      <button
        className={`flex items-center gap-1 text-xs border rounded-md px-2 py-1 hover:border-border-strong ${
          needsCheckout
            ? "border-status-awaiting text-status-awaiting"
            : "border-border text-text-secondary"
        }`}
        onClick={() => setOpenMenu((v) => !v)}
        aria-expanded={openMenu}
        aria-haspopup="listbox"
        title={
          needsCheckout
            ? `'${selected}'에서 시작합니다 — 작업 생성 전에 메인 체크아웃을 '${selected}'로 전환합니다 (지금은 '${current}')`
            : direct
              ? `'${selected}'에서 시작합니다`
              : `'${selected}'에서 시작해 승인 시 '${selected}'에 머지합니다`
        }
      >
        <Icon name="branch" size={13} />
        <span className="max-w-[160px] truncate">{selected || "브랜치"}</span>
        <Icon name="chevronDown" size={12} />
      </button>

      {openMenu && (
        <div className="absolute bottom-full left-0 mb-1 z-20 w-80 rounded-lg border border-border-strong bg-raised py-1 shadow-xl">
          <div className="flex items-center justify-between gap-2 px-3 py-1">
            <span className="text-xs text-text-muted">
              시작할 브랜치
              {onCheckout && <span className="ml-1">· Shift+Enter 체크아웃</span>}
            </span>
            {/* 좁혀졌다는 사실을 숫자로 준다 — 스크롤 막대 길이는 근거가 아니다. */}
            <span className="text-xs text-text-muted font-code shrink-0">
              {tokens.length > 0 ? `${shown.length}/${branches.length}` : branches.length}
            </span>
          </div>

          <div className="px-2 pb-1">
            <div className="relative">
              <span className="absolute left-2 top-1/2 -translate-y-1/2 text-text-muted pointer-events-none">
                <Icon name="search" size={12} />
              </span>
              <input
                ref={inputRef}
                className="w-full bg-bg border border-border rounded pl-7 pr-7 py-1 text-sm text-text outline-none focus:border-primary placeholder:text-text-muted"
                placeholder="브랜치 검색 — 공백으로 여러 조각"
                value={query}
                role="combobox"
                aria-expanded
                aria-controls="branch-picker-list"
                aria-activedescendant={shown[cursor] ? `branch-opt-${cursor}` : undefined}
                aria-label="브랜치 검색"
                onChange={(e) => {
                  setQuery(e.target.value);
                  setCursor(0);
                }}
                onKeyDown={onKeyDown}
              />
              {query && (
                <button
                  className="absolute right-1.5 top-1/2 -translate-y-1/2 text-text-muted hover:text-text"
                  onClick={() => {
                    setQuery("");
                    setCursor(0);
                    inputRef.current?.focus();
                  }}
                  aria-label="검색어 지우기"
                  title="검색어 지우기"
                >
                  <Icon name="x" size={12} />
                </button>
              )}
            </div>
          </div>

          <div
            ref={listRef}
            id="branch-picker-list"
            role="listbox"
            aria-label="시작할 브랜치"
            className="max-h-72 overflow-auto"
          >
            {shown.map((branch, i) => (
              <div
                key={branch}
                id={`branch-opt-${i}`}
                role="option"
                aria-selected={branch === selected}
                data-cursor={i === cursor ? "true" : undefined}
                className={`flex items-center text-sm ${
                  i === cursor
                    ? "bg-surface text-text"
                    : branch === selected
                      ? "text-text"
                      : "text-text-secondary"
                }`}
                onMouseEnter={() => setCursor(i)}
                title={branch}
              >
                <button
                  className="min-w-0 flex-1 text-left flex items-center gap-2 pl-3 pr-2 py-1.5"
                  onClick={() => commit(branch)}
                  tabIndex={-1}
                >
                  <span className="truncate flex-1 font-code text-xs">
                    <Highlighted text={branch} tokens={tokens} />
                  </span>
                  {/* 상태가 아니라 분류다 — Badge가 아니라 MetaTag의 자리(DESIGN.md Don't #14). */}
                  {branch === current && <MetaTag>체크아웃됨</MetaTag>}
                  {branch === selected && (
                    <span className="text-primary-bright shrink-0">
                      <Icon name="check" size={14} />
                    </span>
                  )}
                </button>
                {/* 커서 행에만 보여 목록이 버튼으로 뒤덮이지 않게 한다. */}
                {i === cursor && canCheckout(branch) && (
                  <button
                    className="shrink-0 mr-2 text-xs border border-border rounded px-1.5 py-0.5 text-text-secondary hover:border-border-strong hover:text-text"
                    onClick={() => checkout(branch)}
                    tabIndex={-1}
                    title={`메인 체크아웃을 '${branch}'로 지금 전환합니다 — 변경 사항은 stash에 보관됩니다 (Shift+Enter)`}
                  >
                    체크아웃
                  </button>
                )}
              </div>
            ))}

            {shown.length === 0 && (
              <div className="px-3 py-2 space-y-1">
                {emptyLines.map((line) => (
                  <div key={line} className="text-sm text-text-muted">
                    {line}
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
