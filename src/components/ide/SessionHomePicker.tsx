// src/components/ide/SessionHomePicker.tsx — 세션홈(~/.claude/projects)에서 벤더 세션을
// 골라 새 대화 작업으로 이어받는 모달(설계 2026-09-17 → 2026-09-18 트리 개편). 모양과
// 손동작은 메인 창 Shift 두 번의 SessionNavigator(S-17)를 따른다 — 같은 팝오버, 같은 행
// 문법, ↑↓ 순환·←→ 접힘·Enter 확정. 데이터 소스가 Task가 아니라 세션홈 스캔 결과이고
// 그룹 단이 없어 컴포넌트는 독립이다.
import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { Icon } from "./icons";
import { sessionHomeIndex, sessionHomePreview } from "../../lib/ipc";
import type { HostId, SessionHomeSession } from "../../lib/transport";
import type { SessionHomePreview } from "../../lib/ipc";
import { fmtAge } from "../../lib/usage";
import { isDifferentRepository, isRecentlyActive } from "../../lib/session-resume";
import {
  buildSessionHomeTree,
  initiallyCollapsedProjects,
  mergeSessionHomeResults,
  sessionHomeKey,
  sessionHomeRows,
  sessionLabel,
  sessionVendor,
  type SessionHomeRow,
} from "../../lib/session-home-tree";

interface Props {
  /** 새 작업이 살 호스트 — 이 호스트의 세션홈만 조회한다. */
  host: HostId;
  repo: string;
  /** 등록된 프로젝트 경로 — 세션의 cwd를 프로젝트에 묶는 기준(설계 2026-09-18 결정 2). */
  projects?: readonly string[];
  onSelect: (session: SessionHomeSession) => void;
  onClose: () => void;
}

const DEBOUNCE_MS = 200;
const NO_PROJECTS: readonly string[] = [];

/** 확인이 필요한 사유 — 배열이 비어 있으면 바로 선택된다. */
function warningsFor(session: SessionHomeSession, repo: string): string[] {
  const warnings: string[] = [];
  if (isRecentlyActive(session.last_active)) {
    warnings.push(
      "최근에도 쓰인 세션입니다 — 다른 터미널에서 아직 열려 있으면 이어받는 순간부터 두 대화가 같은 세션 파일에 섞입니다.",
    );
  }
  if (isDifferentRepository(session, repo)) {
    warnings.push("이 세션의 원래 작업 디렉터리가 지금 선택한 저장소와 다릅니다.");
  }
  return warnings;
}

/** 세션홈 이어받기 선택 모달. `title`/`first_message`는 **평문으로만** 렌더한다 — 세션홈은
 *  쓰기 통제가 없는 디렉터리라 마크다운·HTML로 그리면 주입된 제목이 그대로 실행된다
 *  (설계 결정 8). JSX 텍스트 노드로만 출력하고, `dangerouslySetInnerHTML`은 쓰지 않는다. */
export function SessionHomePicker({ host, repo, projects = NO_PROJECTS, onSelect, onClose }: Props) {
  const [query, setQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [sessions, setSessions] = useState<SessionHomeSession[]>([]);
  const [status, setStatus] = useState<"loading" | "ready" | "error">("loading");
  const [pending, setPending] = useState<SessionHomeSession | null>(null);
  const [selected, setSelected] = useState<SessionHomeSession | null>(null);
  const [preview, setPreview] = useState<SessionHomePreview | null>(null);
  const [previewStatus, setPreviewStatus] = useState<"idle" | "loading" | "error" | "ready">("idle");
  const [previewKey, setPreviewKey] = useState<string | null>(null);
  const [previewAttempt, setPreviewAttempt] = useState(0);
  // 접힘은 스냅샷이 아니라 파생값이다 — 기본은 "현재 저장소만 펼침"이고 `toggled`는 사용자가
  // 뒤집은 프로젝트만 쥔다. 결과가 바뀌어 새 프로젝트가 나타나도 접힌 채로 나오고, 사용자가
  // 편 것은 검색을 오가도 유지된다(결정 3).
  const [toggled, setToggled] = useState<Set<string>>(() => new Set());
  const [activeId, setActiveId] = useState<string | null>(null);
  const reqRef = useRef(0);
  const previewReqRef = useRef(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);
  const resumeRef = useRef<HTMLButtonElement>(null);
  const rowRefs = useRef(new Map<string, HTMLElement>());
  // 닫을 때 돌아갈 자리 — SessionNavigator와 같은 규칙(S-17: 취소는 원래 작업으로).
  const returnFocusRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    returnFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    requestAnimationFrame(() => inputRef.current?.focus());
  }, []);

  // 호스트·저장소가 바뀌면 이전 선택의 상세를 새 대상에 붙이지 않는다.
  useEffect(() => {
    previewReqRef.current += 1;
    setSelected(null);
    setPreview(null);
    setPreviewStatus("idle");
    setPreviewKey(null);
    setPending(null);
  }, [host, repo]);

  // 확인 단계로 들어가면 목록(입력창·행)이 언마운트돼 포커스가 body로 떨어진다 — 그러면
  // Esc·Enter가 대화상자에 닿지 않는다. 확인 버튼으로 옮기고, 돌아오면 입력창으로 되돌린다.
  useEffect(() => {
    // 커밋 직후라 요소가 이미 있다 — rAF로 미루면 그 사이 키 입력이 body로 샌다.
    (pending ? confirmRef.current : inputRef.current)?.focus();
  }, [pending]);

  const close = (): void => {
    onClose();
    queueMicrotask(() => returnFocusRef.current?.focus());
  };

  // 200ms 디바운스 — SessionNavigator·QuickOpen과 같은 값.
  useEffect(() => {
    const timer = setTimeout(() => setDebouncedQuery(query), DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [query]);

  useEffect(() => {
    const reqId = ++reqRef.current;
    setStatus("loading");
    const q = debouncedQuery.trim() || undefined;
    // 저장소 조회 + 전체 조회를 함께 보내 합친다(결정 4) — 전체 조회의 상한(200)에
    // 활발한 다른 프로젝트가 현재 저장소의 세션을 밀어내도 저장소 조회가 그 몫을 보장한다.
    Promise.allSettled([
      sessionHomeIndex(host, repo, false, q),
      sessionHomeIndex(host, repo, true, q),
    ]).then((results) => {
      if (reqRef.current !== reqId) return;
      const fulfilled = results.flatMap((r) => (r.status === "fulfilled" ? [r.value] : []));
      if (fulfilled.length === 0) {
        setSessions([]);
        setStatus("error");
        return;
      }
      setSessions(mergeSessionHomeResults(...fulfilled));
      setStatus("ready");
    });
  }, [host, repo, debouncedQuery]);

  const tree = useMemo(() => buildSessionHomeTree(sessions, repo, projects), [sessions, repo, projects]);
  const searching = debouncedQuery.trim().length > 0;
  const collapsed = useMemo(() => {
    const defaults = initiallyCollapsedProjects(tree, repo);
    for (const id of toggled) {
      if (defaults.has(id)) defaults.delete(id);
      else defaults.add(id);
    }
    return defaults;
  }, [tree, repo, toggled]);

  const rows = useMemo(() => sessionHomeRows(tree, collapsed, searching), [tree, collapsed, searching]);
  // 커서가 가리키던 행이 사라지면 첫 행으로 — 단 검색 중에는 첫 **세션**으로 간다. 검색 중
  // 첫 행은 늘 프로젝트 헤더인데 그 위에서 Enter는 아무것도 하지 않기 때문이다.
  const active =
    rows.find((row) => row.id === activeId) ??
    (searching ? rows.find((row) => row.kind === "session") : undefined) ??
    rows[0] ??
    null;

  useEffect(() => {
    if (active) rowRefs.current.get(active.id)?.scrollIntoView?.({ block: "nearest" });
  }, [active]);

  useEffect(() => {
    const request = ++previewReqRef.current;
    setPreview(null);
    setPreviewKey(null);
    if (!selected || selected.host !== host || selected.preview_supported === false) {
      setPreviewStatus("idle");
      return;
    }
    setPreviewStatus("loading");
    void sessionHomePreview(host, repo, sessionVendor(selected), selected.session_id).then(
      (result) => {
        if (request !== previewReqRef.current) return;
        setPreview(result);
        setPreviewKey(sessionHomeKey(selected));
        setPreviewStatus("ready");
      },
      () => {
        if (request !== previewReqRef.current) return;
        setPreviewStatus("error");
      },
    );
    return () => { previewReqRef.current += 1; };
  }, [host, repo, selected, previewAttempt]);

  const clearDetail = (): void => {
    previewReqRef.current += 1;
    setSelected(null);
    setPreview(null);
    setPreviewKey(null);
    setPreviewStatus("idle");
  };

  const selectSession = (session: SessionHomeSession, focusResume = false): void => {
    setPreview(null);
    setPreviewKey(null);
    setPreviewStatus(session.preview_supported === false ? "idle" : "loading");
    setSelected(session);
    setPreviewAttempt((attempt) => attempt + 1);
    if (focusResume) {
      resumeRef.current?.focus();
      requestAnimationFrame(() => resumeRef.current?.focus());
    }
  };

  const expanded = (row: SessionHomeRow): boolean => searching || !collapsed.has(row.id);

  const toggle = (row: SessionHomeRow): void => {
    if (row.kind !== "project" || searching) return;
    setToggled((previous) => {
      const next = new Set(previous);
      if (next.has(row.id)) next.delete(row.id);
      else next.add(row.id);
      return next;
    });
  };

  const pick = (session: SessionHomeSession): void => {
    // 결정 11 — 목록을 가져온 뒤 호스트가 바뀌면(예: 모달이 열린 채 다른 서버로 전환) 제출
    // 자체를 막는다. 서버도 다시 판정하지만(2번째 겹) 여기가 첫 겹이다.
    if (session.host !== host) return;
    const warnings = warningsFor(session, repo);
    if (warnings.length > 0) {
      setPending(session);
      return;
    }
    onSelect(session);
  };

  const activate = (row: SessionHomeRow): void => {
    if (row.session) selectSession(row.session, true);
    else toggle(row);
  };

  const focusRow = (row: SessionHomeRow): void => {
    setActiveId(row.id);
    if (row.session) selectSession(row.session);
    requestAnimationFrame(() => rowRefs.current.get(row.id)?.focus());
  };

  const move = (step: number, focus: boolean): void => {
    if (!active || rows.length === 0) return;
    const index = rows.indexOf(active);
    const target = rows[(index + step + rows.length) % rows.length];
    if (target.session) selectSession(target.session);
    if (focus) focusRow(target);
    else setActiveId(target.id);
  };

  const parentOf = (row: SessionHomeRow): SessionHomeRow | null => {
    for (let index = rows.indexOf(row) - 1; index >= 0; index -= 1) {
      if (rows[index].depth < row.depth) return rows[index];
    }
    return null;
  };

  const onInputKeyDown = (event: KeyboardEvent<HTMLInputElement>): void => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") {
      event.preventDefault();
      if (selected) clearDetail();
      else close();
      return;
    }
    if (rows.length === 0) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      move(event.key === "ArrowDown" ? 1 : -1, false);
      return;
    }
    if (event.key === "Enter" && active) {
      event.preventDefault();
      activate(active);
    }
  };

  const onRowKeyDown = (event: KeyboardEvent<HTMLButtonElement>, row: SessionHomeRow): void => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") {
      event.preventDefault();
      if (selected) clearDetail();
      else close();
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      move(event.key === "ArrowDown" ? 1 : -1, true);
      return;
    }
    if (event.key === "ArrowRight") {
      const child = rows[rows.indexOf(row) + 1];
      if (row.kind === "project" && !expanded(row)) {
        event.preventDefault();
        toggle(row);
      } else if (child && child.depth === row.depth + 1) {
        event.preventDefault();
        focusRow(child);
      }
      return;
    }
    if (event.key === "ArrowLeft") {
      const parent = parentOf(row);
      if (row.kind === "project" && expanded(row) && !searching) {
        event.preventDefault();
        toggle(row);
      } else if (parent) {
        event.preventDefault();
        focusRow(parent);
      }
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      activate(row);
    }
  };

  const pendingWarnings = pending ? warningsFor(pending, repo) : [];
  const now = Math.floor(Date.now() / 1000);

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/50 p-4 pt-24"
      onMouseDown={close}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="세션 이어받기"
        className="flex max-h-[70vh] w-full max-w-4xl flex-col overflow-hidden rounded-xl border border-border-strong bg-raised shadow-xl"
        onMouseDown={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.defaultPrevented) return;
          if (event.key === "Escape" && !event.nativeEvent.isComposing) {
            event.preventDefault();
            if (pending) setPending(null);
            else if (selected) clearDetail();
            else close();
          }
        }}
      >
        {pending ? (
          <div className="flex flex-col gap-3 p-4">
            <div className="text-sm text-text">이 세션을 이어받으시겠습니까?</div>
            <div className="truncate rounded-md border border-border p-2 text-xs text-text-secondary">
              {sessionLabel(pending)}
            </div>
            <ul className="flex flex-col gap-2">
              {pendingWarnings.map((warning) => (
                <li
                  key={warning}
                  className="rounded-lg border border-status-awaiting/40 bg-status-awaiting/5 p-2 text-xs text-status-awaiting"
                >
                  {warning}
                </li>
              ))}
            </ul>
            <div className="flex items-center justify-end gap-2 pt-1">
              <button
                className="h-8 rounded-md border border-border px-3 text-text-secondary hover:border-border-strong"
                onClick={() => setPending(null)}
              >
                돌아가기
              </button>
              <button
                ref={confirmRef}
                className="h-8 rounded-md border border-status-awaiting/40 px-3 text-status-awaiting hover:border-border-strong"
                onClick={() => {
                  if (pending.host === host) onSelect(pending);
                  setPending(null);
                }}
              >
                이어받기
              </button>
            </div>
          </div>
        ) : (
          <>
            <div className="flex items-center gap-2 border-b border-border px-3 py-2">
              <span className="shrink-0 text-text-muted">
                <Icon name="clock" size={16} />
              </span>
              <input
                ref={inputRef}
                aria-label="세션 검색"
                aria-controls="session-home-tree"
                aria-activedescendant={active ? `session-home-${active.id}` : undefined}
                className="flex-1 bg-transparent text-md text-text outline-none placeholder:text-text-muted"
                placeholder="프로젝트·제목·최근 발언·브랜치·경로 검색…"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                onKeyDown={onInputKeyDown}
              />
            </div>
            <div className="flex items-center justify-between border-b border-border px-3 py-1.5 text-xs text-text-muted">
              <span>↑↓ 이동 · 트리에서 ←→ 펼침/접힘 · Enter 상세 · 버튼으로 이어받기 · Esc 닫기</span>
              <button type="button" className="text-text-muted hover:text-text" onClick={close} aria-label="세션 이어받기 닫기">
                닫기
              </button>
            </div>
            <div className="grid min-h-0 flex-1 grid-cols-1 md:grid-cols-2">
            <div id="session-home-tree" className="overflow-auto border-r border-border p-1" role="tree" aria-label="프로젝트와 세션">
              {status === "loading" && rows.length === 0 && (
                <div className="px-3 py-6 text-center text-sm text-text-muted">불러오는 중…</div>
              )}
              {status === "error" && (
                <div className="px-3 py-6 text-center text-sm text-text-muted">세션 목록을 불러오지 못했습니다.</div>
              )}
              {status === "ready" && rows.length === 0 && (
                <div className="px-3 py-6 text-center text-sm text-text-muted">
                  {searching ? "일치하는 세션이 없습니다" : "세션홈에 이어받을 세션이 없습니다"}
                </div>
              )}
              {rows.map((row) => {
                const selected = row.id === active?.id;
                const isProject = row.kind === "project";
                return (
                  <button
                    key={row.id}
                    id={`session-home-${row.id}`}
                    ref={(element) => {
                      if (element) rowRefs.current.set(row.id, element);
                      else rowRefs.current.delete(row.id);
                    }}
                    type="button"
                    role="treeitem"
                    aria-level={row.depth + 1}
                    aria-expanded={isProject ? expanded(row) : undefined}
                    data-kind={row.kind}
                    tabIndex={selected ? 0 : -1}
                    className={`flex w-full items-start gap-2 rounded-md px-2.5 py-1.5 text-left text-sm ${
                      selected ? "bg-primary/10 text-primary-bright" : "text-text-secondary hover:bg-surface hover:text-text"
                    }`}
                    style={{ paddingLeft: `${10 + row.depth * 18}px` }}
                    title={row.session ? (row.session.cwd ?? row.session.last_cwd ?? undefined) : (row.detail ?? undefined)}
                    onKeyDown={(event) => onRowKeyDown(event, row)}
                    onClick={() => {
                      setActiveId(row.id);
                      if (row.session) selectSession(row.session);
                      else activate(row);
                    }}
                  >
                    {isProject ? (
                      <span className="w-4 text-text-muted">
                        <Icon name={expanded(row) ? "chevronDown" : "chevronRight"} size={14} />
                      </span>
                    ) : (
                      <span className="w-4" aria-hidden="true" />
                    )}
                    <span className="min-w-0 flex-1">
                      <span className="block truncate">{row.label}</span>
                      {row.session && <span className="block truncate text-xs text-text-muted">{row.session.recent_user_message?.trim() || "최근 대화 미확인"}</span>}
                      {row.session && <span className="block text-xs text-text-muted">{sessionVendor(row.session)} · {fmtAge(Math.max(0, now - row.session.last_active))}</span>}
                    </span>
                    {isProject && (
                      <span className="max-w-[45%] truncate text-xs text-text-muted">
                        {row.detail ?? ""}
                        {row.detail ? " · " : ""}
                        {row.children.length}개
                      </span>
                    )}
                  </button>
                );
              })}
            </div>
            <aside className="min-h-0 overflow-auto p-3" aria-label="세션 미리보기">
              {!selected ? <p className="text-sm text-text-muted">세션을 선택하면 최근 대화를 확인할 수 있습니다.</p> : <>
                {(() => {
                  const detail = previewKey === sessionHomeKey(selected) && preview ? preview.meta : selected;
                  const canResume = selected.preview_supported === false || (previewStatus === "ready" && previewKey === sessionHomeKey(selected));
                  return <>
                <div className="mb-2 whitespace-pre-wrap break-words text-sm text-text">{sessionLabel(detail)}</div>
                <dl className="mb-3 space-y-1 text-xs text-text-muted">
                  <div><dt className="inline">공급자: </dt><dd className="inline">{sessionVendor(detail)}</dd></div>
                  <div><dt className="inline">경로: </dt><dd className="break-all text-text-secondary">{detail.cwd ?? detail.last_cwd ?? "경로 없음"}</dd></div>
                  {detail.git_branch && <div><dt className="inline">브랜치: </dt><dd className="break-all text-text-secondary">{detail.git_branch}</dd></div>}
                </dl>
                {selected.preview_supported === false ? <p className="mb-3 text-xs text-text-muted">연결된 Runner는 대화 미리보기를 지원하지 않습니다.</p> : previewStatus === "loading" ? <p className="mb-3 text-xs text-text-muted">대화를 불러오는 중…</p> : previewStatus === "error" ? <button className="mb-3 text-xs text-status-failed underline" onClick={() => { setPreviewStatus("loading"); setPreviewAttempt((attempt) => attempt + 1); }}>미리보기를 불러오지 못했습니다. 다시 시도</button> : preview && <div className="mb-3 space-y-2 text-xs text-text-secondary">{preview.messages.map((message, index) => <div key={`${index}:${message.role}`}><span className="text-text-muted">{message.role === "user" ? "나" : message.role}</span><p className="whitespace-pre-wrap break-words">{message.text}</p></div>)}{preview.truncated && <p className="text-text-muted">최근 대화 일부만 표시했습니다.</p>}</div>}
                <button ref={resumeRef} type="button" aria-disabled={!canResume} className={`h-8 rounded-md border border-primary/40 px-3 text-sm text-primary-bright hover:border-primary ${!canResume ? "cursor-not-allowed opacity-50" : ""}`} onClick={() => { if (canResume) pick({ ...selected, last_active: Math.max(selected.last_active, detail.last_active) }); }}>이 세션 이어받기</button>
                  </>;
                })()}
              </>}
            </aside>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
