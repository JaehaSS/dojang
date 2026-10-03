// src/components/QuickOpen.tsx — ⌘K/⌘P 커맨드팔레트 오버레이.
import { useHostScope } from "../lib/host-scope";
// 작업/세션(백엔드 quickopenSearch)·파일(로컬 flattenFiles)·스킬(skillsList)·정적 커맨드를
// quickopen.ts의 mergeQuickOpenResults로 병합해 그룹 헤더와 함께 렌더한다.
import { useEffect, useMemo, useRef, useState } from "react";
import { Icon, type IconName } from "./ide/icons";
import {
  mergeQuickOpenResults,
  fileQuickOpenItems,
  codeQuickOpenItems,
  QUICK_OPEN_COMMANDS,
  type QuickOpenItem,
  type QuickOpenScope,
  type RankedQuickOpenItem,
} from "../lib/quickopen";
import { projectSearch, quickopenSearch, skillsList, type QuickOpenTaskCandidate } from "../lib/ipc";
import { handleMenuKey } from "../lib/mention";

interface Props {
  open: boolean;
  /**
   * 보여 줄 스코프. 미지정이면 전부(⌘K).
   *
   * 이분법(`fileScopeOnly`)이었던 것을 목록으로 바꾼 이유는 **세 번째 호출부**가 생겼기
   * 때문이다. 팝아웃 에디터 창은 파일과 코드는 열 수 있지만 작업·세션은 열 자리가 없다 —
   * "파일만"도 "전부"도 그 창을 설명하지 못한다.
   */
  scopes?: QuickOpenScope[];
  /** 팝아웃 에디터의 현재 워크트리 검색. 지정하면 파일·내용 범위를 탭으로 나눈다. */
  editorSearch?: {
    scopeLabel: string;
    contentAvailable: boolean;
  };
  /** 열 때 고를 에디터 검색 범위. ⌥F처럼 "파일을 찾겠다"가 분명한 진입로는 `file`로 연다. */
  initialEditorTab?: EditorSearchTab;
  /** 현재 워크트리 파일 경로 목록(flattenFiles 결과) — 미선택이면 빈 배열. */
  files: string[];
  /** 스킬 조회 대상 레포 — 빈 문자열이면 스킬 소스 생략. */
  repo: string;
  /** 내용 검색 대상 작업 id — null이면 코드 스코프를 생략한다(워크트리 미선택). */
  taskId: number | null;
  onClose: () => void;
  onSelect: (item: RankedQuickOpenItem) => void;
}

const DEBOUNCE_MS = 200;
export type EditorSearchTab = "all" | "file" | "code";

const SCOPE_ORDER: QuickOpenScope[] = ["task", "file", "code", "session", "skill", "command"];
const SCOPE_LABEL: Record<QuickOpenScope, string> = {
  task: "작업",
  file: "파일",
  code: "코드",
  session: "세션",
  skill: "스킬",
  command: "커맨드",
};
const SCOPE_ICON: Record<QuickOpenScope, IconName> = {
  task: "terminal",
  file: "folder",
  code: "fileCode",
  session: "chat",
  skill: "sparkle",
  command: "play",
};

function candidateToItem(row: QuickOpenTaskCandidate): QuickOpenItem {
  return {
    scope: row.scope,
    id: String(row.id),
    title: row.title,
    subtitle: row.subtitle,
    updatedAt: row.updated_at * 1000,
  };
}

function groupByScope(items: RankedQuickOpenItem[]): [QuickOpenScope, RankedQuickOpenItem[]][] {
  return SCOPE_ORDER.map(
    (scope) => [scope, items.filter((i) => i.scope === scope)] as [QuickOpenScope, RankedQuickOpenItem[]],
  ).filter(([, list]) => list.length > 0);
}

/** 안정된 빈 배열 — 렌더마다 새 `[]`를 만들면 아래 useMemo가 매번 무효화된다. */
const NO_ITEMS: QuickOpenItem[] = [];
const NO_RANKED: RankedQuickOpenItem[] = [];

export function QuickOpen({ open, scopes, editorSearch, initialEditorTab = "all", files, repo, taskId, onClose, onSelect }: Props) {
  const [editorTab, setEditorTab] = useState<EditorSearchTab>("all");
  const effectiveEditorTab = editorSearch && !editorSearch.contentAvailable && editorTab === "code"
    ? "all"
    : editorTab;
  const activeScopes: QuickOpenScope[] | undefined = editorSearch
    ? effectiveEditorTab === "all"
      ? editorSearch.contentAvailable
        ? ["file", "code"]
        : ["file"]
      : [effectiveEditorTab]
    : scopes;
  const scopeKey = activeScopes?.join(",") ?? "all";
  const wants = (scope: QuickOpenScope) => activeScopes == null || activeScopes.includes(scope);
  const [query, setQuery] = useState("");
  const [queryRevision, setQueryRevision] = useState(0);
  const [debounced, setDebounced] = useState({ value: "", revision: 0 });
  const [taskItems, setTaskItems] = useState<QuickOpenItem[]>([]);
  const [skillItems, setSkillItems] = useState<QuickOpenItem[]>([]);
  const [codeItems, setCodeItems] = useState<QuickOpenItem[]>([]);
  /** 상한에 걸려 잘렸는가 — 그룹 헤더가 그 사실을 말해야 한다. */
  const [codeTruncated, setCodeTruncated] = useState(false);
  const [codeStatus, setCodeStatus] = useState<"idle" | "loading" | "error">("idle");
  const [codeResultKey, setCodeResultKey] = useState<string | null>(null);
  const codeReqRef = useRef(0);
  const [sel, setSel] = useState(0);
  const reqRef = useRef(0);
  // 세션에 속하지 않는 화면이라 호스트를 스코프에서 받는다 (ADR 0133).
  const host = useHostScope();
  const inputRef = useRef<HTMLInputElement>(null);
  const priorFocusRef = useRef<HTMLElement | null>(null);
  const selectedRef = useRef<HTMLButtonElement | null>(null);

  // 열릴 때마다 상태 리셋 + 입력 포커스.
  useEffect(() => {
    if (!open) return;
    priorFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setQuery("");
    setDebounced({ value: "", revision: 0 });
    setEditorTab(initialEditorTab);
    setSel(0);
    requestAnimationFrame(() => inputRef.current?.focus());
    // 시작 범위는 여는 순간에만 읽는다 — 열린 뒤 바뀐다고 입력을 지우지 않는다.
  }, [open]);

  useEffect(() => {
    if (editorSearch?.contentAvailable !== false || editorTab !== "code") return;
    setEditorTab("all");
  }, [editorSearch?.contentAvailable, editorTab]);

  useEffect(() => {
    if (open || !priorFocusRef.current) return;
    priorFocusRef.current.focus();
    priorFocusRef.current = null;
  }, [open]);

  // 200ms 디바운스 — 이후 debounced가 백엔드 검색·랭킹 병합 트리거.
  useEffect(() => {
    if (!open) return;
    const timer = setTimeout(() => setDebounced({ value: query, revision: queryRevision }), DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [query, queryRevision, open]);

  useEffect(() => {
    const reqId = ++reqRef.current;
    if (!open || (!wants("task") && !wants("session"))) {
      setTaskItems([]);
      return;
    }
    quickopenSearch(host, debounced.value, ["task", "session"])
      .then((rows) => {
        if (reqId !== reqRef.current) return;
        setTaskItems(rows.map(candidateToItem));
      })
      .catch(() => {
        if (reqId === reqRef.current) setTaskItems([]);
      });
  }, [debounced, open, scopeKey]);

  useEffect(() => {
    if (!open || !wants("skill") || !repo.trim()) {
      setSkillItems([]);
      return;
    }
    skillsList(host, repo).then((list) =>
      setSkillItems(
        list.map((s) => ({ scope: "skill" as const, id: s.name, title: s.name, subtitle: s.description })),
      ),
    );
  }, [open, scopeKey, host, repo]);

  // 코드 내용 검색 — 워크트리가 있고 질의가 2자 이상일 때만. 한 글자로는 거의 모든
  // 파일이 걸려 결과가 무의미하고, 큰 레포에서는 왕복이 비싸다(백엔드도 같은 하한을 둔다).
  useEffect(() => {
    const reqId = ++codeReqRef.current;
    const codeQueryKey = `${scopeKey}:${taskId ?? ""}:${queryRevision}:${query}`;
    if (
      !open ||
      (editorSearch != null && !editorSearch.contentAvailable) ||
      !wants("code") ||
      taskId == null ||
      debounced.revision !== queryRevision ||
      debounced.value.trim().length < 2
    ) {
      setCodeItems([]);
      setCodeTruncated(false);
      setCodeResultKey(null);
      setCodeStatus("idle");
      return;
    }
    setCodeStatus("loading");
    setCodeItems([]);
    setCodeTruncated(false);
    setCodeResultKey(null);
    projectSearch(taskId, debounced.value)
      .then((res) => {
        if (reqId !== codeReqRef.current) return;
        setCodeItems(codeQuickOpenItems(res.matches));
        setCodeTruncated(res.truncated);
        setCodeResultKey(codeQueryKey);
        setCodeStatus("idle");
      })
      .catch(() => {
        if (reqId === codeReqRef.current) {
          setCodeItems([]);
          setCodeTruncated(false);
          setCodeResultKey(null);
          setCodeStatus("error");
        }
      });
    return () => {
      if (codeReqRef.current === reqId) ++codeReqRef.current;
    };
  }, [debounced, open, scopeKey, taskId, query, queryRevision, editorSearch?.contentAvailable]);

  useEffect(() => setSel(0), [query, scopeKey]);

  // 파일 항목과 랭킹은 입력이 바뀔 때만 만들고, 닫혀 있는 동안은 아예 계산하지 않는다.
  // 부모는 스트리밍 이벤트·IME 입력마다 렌더되는데, 그때마다 워크트리 파일 전체를
  // localeCompare로 정렬하면 큰 저장소에서 렌더 한 번이 초 단위가 된다(원장 #448).
  const fileItems = useMemo(() => fileQuickOpenItems(files), [files]);
  const codeQueryKey = `${scopeKey}:${taskId ?? ""}:${queryRevision}:${query}`;
  const visibleCodeItems = codeResultKey === codeQueryKey ? codeItems : NO_ITEMS;
  const ranked = useMemo(
    () =>
      open
        ? mergeQuickOpenResults(query, [taskItems, fileItems, visibleCodeItems, skillItems, QUICK_OPEN_COMMANDS], {
            scopes: activeScopes,
          })
        : NO_RANKED,
    // activeScopes는 렌더마다 새 배열이라 그 내용을 온전히 대표하는 scopeKey를 의존성으로 둔다.
    [open, query, taskItems, fileItems, visibleCodeItems, skillItems, scopeKey],
  );
  const groups = groupByScope(ranked);
  const rendered = groups.flatMap(([, items]) => items);
  const effectiveSel = Math.min(sel, Math.max(rendered.length - 1, 0));
  const indexByKey = new Map(rendered.map((item, i) => [`${item.scope}:${item.id}`, i]));

  useEffect(() => selectedRef.current?.scrollIntoView({ block: "nearest" }), [effectiveSel]);

  if (!open) return null;

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.nativeEvent.isComposing) return;
    if (editorSearch && e.key === "Tab") {
      e.preventDefault();
      const tabs: EditorSearchTab[] = editorSearch.contentAvailable ? ["all", "file", "code"] : ["all", "file"];
      const direction = e.shiftKey ? -1 : 1;
      const next = (tabs.indexOf(effectiveEditorTab) + direction + tabs.length) % tabs.length;
      setEditorTab(tabs[next]);
      return;
    }
    if (rendered.length === 0 && (e.key === "Tab" || e.key === "Enter")) {
      e.preventDefault();
      return;
    }
    handleMenuKey(e, {
      items: rendered,
      sel: effectiveSel,
      setSel,
      onSelect,
      close: onClose,
      enterSelects: true,
    });
  };

  const onQueryChange = (value: string) => {
    ++codeReqRef.current;
    setQuery(value);
    setQueryRevision((revision) => revision + 1);
    setCodeItems([]);
    setCodeTruncated(false);
    setCodeStatus(value.trim().length >= 2 && wants("code") ? "loading" : "idle");
  };
  const codeHint = !editorSearch || effectiveEditorTab === "file"
    ? null
    : !editorSearch.contentAvailable
      ? "내용 검색은 이 원격 워크트리에서 사용할 수 없습니다"
      : query.trim().length < 2
        ? "내용 검색은 두 글자 이상 입력하세요"
        : codeStatus === "loading"
          ? "내용을 검색하는 중…"
          : codeStatus === "error"
            ? "내용 검색을 완료하지 못했습니다"
            : null;

  return (
    <>
      <div className="fixed inset-0 bg-black/50 z-40" onClick={onClose} aria-hidden="true" />
      <div className="fixed inset-0 z-50 flex items-start justify-center pt-24 p-4" onClick={onClose}>
        <div
          role="dialog"
          aria-modal="true"
          aria-label="빠른 검색"
          className="bg-raised border border-border-strong rounded-xl shadow-xl w-full max-w-xl max-h-[70vh] flex flex-col overflow-hidden"
          onClick={(e) => e.stopPropagation()}
          onKeyDown={onKeyDown}
        >
          <div className="flex items-center gap-2 px-3 py-2 border-b border-border shrink-0">
            <span className="text-text-muted shrink-0">
              <Icon name="search" size={16} />
            </span>
            <input
              ref={inputRef}
              aria-label="빠른 검색"
              className="flex-1 bg-transparent outline-none text-md text-text placeholder:text-text-muted"
              placeholder={
                editorSearch
                  ? "파일·내용 검색…"
                  : scopes == null
                  ? "작업·파일·세션·스킬·커맨드 검색…"
                  : `${scopes.map((s) => SCOPE_LABEL[s]).join("·")} 검색…`
              }
              value={query}
              onChange={(e) => onQueryChange(e.target.value)}
            />
            <button type="button" className="text-xs text-text-muted hover:text-text" onClick={onClose}>닫기</button>
          </div>
          {editorSearch && (
            <div className="border-b border-border px-3 py-2">
              <div className="mb-2 text-xs text-text-muted">{editorSearch.scopeLabel}</div>
              <div className="flex items-center gap-1" role="tablist" aria-label="에디터 검색 범위">
                {(["all", "file", "code"] as const).map((tab) => {
                  const disabled = tab === "code" && !editorSearch.contentAvailable;
                  const label = tab === "all" ? "전체" : tab === "file" ? "파일" : "내용";
                  return (
                    <button
                      key={tab}
                      type="button"
                      role="tab"
                      aria-selected={effectiveEditorTab === tab}
                      disabled={disabled}
                      className={`border-b-2 px-2 py-1 text-xs ${effectiveEditorTab === tab ? "border-primary-bright text-primary-bright" : "border-transparent text-text-muted hover:text-text"}`}
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={() => setEditorTab(tab)}
                    >
                      {label}
                    </button>
                  );
                })}
                <span className="ml-auto text-xs text-text-muted">Tab 범위 전환 · Esc 닫기</span>
              </div>
            </div>
          )}
          <div className="flex-1 overflow-auto py-1">
            {ranked.length === 0 ? (
              <div className="px-3 py-6 text-center text-sm text-text-muted">
                {codeHint ?? "일치하는 결과가 없습니다"}
              </div>
            ) : (
              <>
                {codeHint && <div className="px-3 py-2 text-xs text-text-muted">{codeHint}</div>}
                {groups.map(([scope, items]) => (
                <div key={scope}>
                  <div className="px-3 pt-2 pb-1 text-xs text-text-muted uppercase tracking-wide">
                    {SCOPE_LABEL[scope]}
                    {/* 조용히 자르지 않는다 — 잘린 목록은 "이게 전부"로 읽힌다. */}
                    {scope === "code" && codeTruncated && (
                      <span className="ml-2 normal-case tracking-normal">현재 {items.length}건 · 검색은 최대 200건</span>
                    )}
                  </div>
                  {items.map((item) => {
                    const idx = indexByKey.get(`${item.scope}:${item.id}`) ?? -1;
                    return (
                      <button
                        key={`${item.scope}:${item.id}`}
                        className={`w-full text-left px-3 py-1.5 flex items-center gap-2 ${
                          idx === effectiveSel ? "bg-surface text-text" : "text-text-secondary"
                        }`}
                        ref={idx === effectiveSel ? selectedRef : null}
                        onMouseEnter={() => setSel(idx)}
                        onClick={() => onSelect(item)}
                      >
                        <span className="text-text-muted shrink-0">
                          <Icon name={SCOPE_ICON[scope]} size={14} />
                        </span>
                        <span className="flex-1 truncate text-sm">{item.title}</span>
                        {item.subtitle && (
                          <span className="text-xs text-text-muted truncate max-w-[40%]">{item.subtitle}</span>
                        )}
                      </button>
                    );
                  })}
                </div>
                ))}
              </>
            )}
          </div>
        </div>
      </div>
    </>
  );
}
