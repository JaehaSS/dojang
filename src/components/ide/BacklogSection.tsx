import { useCallback, useEffect, useState } from "react";
import { todayAdd, todayList, todayMove, todayRemove } from "../../lib/ipc";
import { BACKLOG, repoBadge, showRepoBadges, sortItems, staleDays, type DayItem } from "./today-items";
import { Icon } from "./icons";

/**
 * IPC 경계를 주입 가능하게 열어 둔다 — `TodaySection`과 같은 규약.
 */
export interface BacklogApi {
  list: (day?: string) => Promise<DayItem[]>;
  add: (title: string, day?: string, repo?: string) => Promise<DayItem>;
  move: (id: number, to?: string) => Promise<DayItem>;
  remove: (id: number) => Promise<void>;
}

const defaultApi: BacklogApi = {
  list: todayList,
  add: todayAdd,
  move: todayMove,
  remove: todayRemove,
};

/** 이 날수를 넘기면 적체로 본다. 한 달을 넘겨 안 한 일은 다시 볼 이유가 있다. */
const STALE_DAYS = 30;

interface Props {
  /** 새로 담는 항목에 붙일 레포. 홈의 레포 선택을 따른다. */
  repo?: string;
  /** 테스트에서 "지금"을 고정하기 위한 주입점. */
  nowMs?: number;
  api?: BacklogApi;
}

/**
 * 백로그 — "할 건데 오늘은 아니다"가 사는 자리 (플랜 0054).
 *
 * **빈 백로그는 통째로 숨는다.** 진입로를 따로 두지 않는 것은 의도다 — 백로그의 첫 항목은
 * 언제나 오늘 목록에서 밀려 들어온다(`TodaySection`의 `나중에`). 홈에 "+ 백로그" 한 줄을
 * 상시로 띄우면 오늘 할 일과 나란히 두 개의 빈 진입로가 생기고, 그건 설계 0021 §12가
 * 경계한 껍데기 그 자체다.
 */
export function BacklogSection({ repo, nowMs, api = defaultApi }: Props) {
  const [items, setItems] = useState<DayItem[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const list = await api.list(BACKLOG).catch(() => [] as DayItem[]);
    setItems(list);
    setLoaded(true);
  }, [api]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const run = async (action: () => Promise<unknown>): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      await action();
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const submit = (): void => {
    const title = draft.trim();
    if (!title) return;
    setDraft("");
    void run(() => api.add(title, BACKLOG, repo));
  };

  // 로딩 전에는 아무것도 그리지 않는다 — 빈 상태가 스쳤다 사라지는 깜빡임 방지.
  if (!loaded) return null;
  if (items.length === 0) return null;

  const ordered = sortItems(items);
  const withRepo = showRepoBadges(items, repo);
  const nowSecs = Math.floor((nowMs ?? Date.now()) / 1000);
  const stale = items.filter((i) => staleDays(i, nowSecs) >= STALE_DAYS).length;

  return (
    <section className="mb-6" aria-label="백로그">
      {/* 헤더가 곧 토글이다. 기본은 접힘 — 오늘 하지 않을 일이 홈의 세로를 먹으면 안 된다. */}
      <button
        type="button"
        className="flex w-full items-center gap-2 py-1 text-left"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        <span className="text-xs uppercase tracking-wide text-text-muted">백로그</span>
        <span className="text-xs text-text-muted">{items.length}</span>
        {/* 적체는 접힌 상태에서도 보여야 한다 — 펼쳐야 보이면 아무도 안 본다. */}
        {stale > 0 && (
          <span
            aria-label={`${STALE_DAYS}일 넘게 묵은 항목 ${stale}건`}
            className="rounded bg-surface px-1.5 py-0.5 text-[11px] text-text-muted"
          >
            {stale}건 묵음
          </span>
        )}
        <div className="flex-1" />
        <span className="text-xs text-text-muted">{open ? "접기" : "펼치기"}</span>
      </button>

      {error && <div className="mb-2 text-xs text-status-failed">{error}</div>}

      {open && (
        <>
          <div className="mb-2 overflow-hidden rounded-md border border-border">
            {ordered.map((item) => {
              const days = staleDays(item, nowSecs);
              return (
                <div
                  key={item.id}
                  className="flex items-center gap-2.5 border-b border-border bg-raised px-3 py-2 last:border-b-0"
                >
                  <span className="flex-1 truncate text-sm">{item.title}</span>

                  {days >= STALE_DAYS && (
                    <span
                      aria-label={`${days}일 묵음`}
                      title={`${days}일 전에 담았습니다`}
                      className="shrink-0 text-[11px] text-text-muted"
                    >
                      {days}일
                    </span>
                  )}

                  {withRepo && item.repo && (
                    <span
                      aria-label={`레포 ${repoBadge(item.repo)}`}
                      title={item.repo}
                      className="shrink-0 rounded bg-surface px-1.5 py-0.5 font-code text-[11px] text-text-muted"
                    >
                      {repoBadge(item.repo)}
                    </span>
                  )}

                  <button
                    type="button"
                    aria-label={`${item.title} 오늘로`}
                    className="shrink-0 text-xs text-text-muted hover:text-text disabled:opacity-40"
                    disabled={busy}
                    onClick={() => void run(() => api.move(item.id))}
                  >
                    오늘로
                  </button>
                  <button
                    type="button"
                    aria-label={`${item.title} 삭제`}
                    className="inline-flex shrink-0 items-center text-text-muted hover:text-status-failed"
                    disabled={busy}
                    onClick={() => void run(() => api.remove(item.id))}
                  >
                    <Icon name="x" size={12} />
                  </button>
                </div>
              );
            })}
          </div>

          <input
            className="w-full rounded border border-border bg-transparent px-2.5 py-1.5 text-sm placeholder:text-text-muted"
            placeholder="나중에 할 일을 한 줄로 적어보세요"
            value={draft}
            disabled={busy}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                submit();
              }
            }}
          />
        </>
      )}
    </section>
  );
}
