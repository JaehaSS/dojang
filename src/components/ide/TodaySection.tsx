import { useCallback, useEffect, useState } from "react";
import { copyText } from "../../lib/clipboard";
import {
  todayAdd,
  todayClose,
  todayList,
  todayMove,
  todayRemove,
  todayReorder,
  todaySetStatus,
  todayStart,
  todaySuggest,
  todayTake,
  type DayClosing,
  type Task,
} from "../../lib/ipc";
import { taskStatusLabel, taskTextClass } from "../../lib/task-status";
import { ChannelCard } from "./ChannelCard";
import {
  BACKLOG,
  carriedLabel,
  groupSuggestions,
  moveItem,
  progress,
  repoBadge,
  showRepoBadges,
  sortItems,
  type DayItem,
  type DaySuggestion,
} from "./today-items";

/**
 * IPC 경계를 주입 가능하게 열어 둔다 — 테스트가 Tauri 런타임 없이 동작을 검증한다.
 * 기본값은 `lib/ipc.ts` 래퍼다.
 */
export interface TodayApi {
  list: (day?: string) => Promise<DayItem[]>;
  add: (title: string, day?: string, repo?: string) => Promise<DayItem>;
  setStatus: (id: number, status: DayItem["status"]) => Promise<DayItem>;
  remove: (id: number) => Promise<void>;
  /** 다른 레인으로. `to`를 비우면 오늘, `"backlog"`면 백로그 (플랜 0054). */
  move: (id: number, to?: string) => Promise<DayItem>;
  reorder: (day: string, orderedIds: number[]) => Promise<void>;
  start: (id: number, agent: string, mode: string) => Promise<Task>;
  suggest: (day?: string, repo?: string) => Promise<DaySuggestion[]>;
  take: (suggestion: DaySuggestion, day?: string) => Promise<DayItem | null>;
  close: (day?: string) => Promise<DayClosing>;
}

const defaultApi: TodayApi = {
  list: todayList,
  add: todayAdd,
  setStatus: todaySetStatus,
  remove: todayRemove,
  move: todayMove,
  reorder: todayReorder,
  start: (id, agent, mode) => todayStart(id, agent, mode),
  suggest: todaySuggest,
  take: todayTake,
  close: todayClose,
};

/**
 * 어디에 그려지는가. `home`은 정본 편집처(제안·마감·정렬·삭제·착수 전부), `channel`은 세션 위
 * 플로팅 채널의 참조·체크용 압축본이다.
 */
export type TodayDensity = "home" | "channel";

interface Props {
  /** Task 배지 표시용 — Home이 이미 들고 있는 목록을 그대로 받는다(별도 조회 없음). */
  tasks?: Task[];
  /** 제안의 github·memory 소스가 붙을 레포. 없으면 DB 소스 두 개만 제안된다. */
  repo?: string;
  /** 착수 시 사용할 에이전트/모드 — Home의 기본값을 그대로 넘긴다. */
  agent?: string;
  mode?: string;
  density?: TodayDensity;
  api?: TodayApi;
}

/**
 * Home의 "오늘 할 일" 섹션 — 계획 레이어의 정본 편집처.
 *
 * 사이드패널 탭이 아니라 Home에 두는 이유: 사이드패널은 선택된 Task의 컨텍스트 안에서만
 * 존재해서(App.tsx:1496,1888) Task를 골라야만 열린다. 오늘 할 일은 Task와 무관한
 * 전역 레이어라 그 종속이 의미를 깨뜨린다 (플랜 0026 DR-P1).
 */
export function TodaySection({
  tasks = [],
  repo,
  agent = "claude",
  mode = "conversation",
  density = "home",
  api = defaultApi,
}: Props) {
  const [items, setItems] = useState<DayItem[]>([]);
  const [suggestions, setSuggestions] = useState<DaySuggestion[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [closing, setClosing] = useState<DayClosing | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [suggestOpen, setSuggestOpen] = useState(false);
  // 빈 상태에서 사용자가 진입로를 눌렀는지. 누르기 전에는 입력창도 띄우지 않는다.
  const [opened, setOpened] = useState(false);

  const channel = density === "channel";

  const refresh = useCallback(async () => {
    // 채널은 제안을 그리지 않으므로 조회하지도 않는다 — `suggest`의 github 소스는 `gh issue
    // list`(네트워크)를 타므로, 세션 화면에 상시 떠 있는 표면에서 부르면 값 없는 왕복이 된다.
    const [list, sug] = await Promise.all([
      api.list().catch(() => [] as DayItem[]),
      channel
        ? Promise.resolve([] as DaySuggestion[])
        : api.suggest(undefined, repo).catch(() => [] as DaySuggestion[]),
    ]);
    setItems(list);
    setSuggestions(sug);
    setLoaded(true);
  }, [api, channel, repo]);

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
    void run(() => api.add(title, undefined, repo));
  };

  // 로딩 전에는 아무것도 그리지 않는다 — 빈 상태가 잠깐 스쳤다 사라지는 깜빡임 방지.
  if (!loaded) return null;

  // 항목도 제안도 없으면 섹션(헤더·리스트·진행률)을 그리지 않는다. 빈 껍데기를 매일 보는
  // 것이 이 기능을 죽이는 가장 빠른 길이다 (설계 0021 §12 방어책 3).
  //
  // 다만 완전히 사라지면 시작할 방법이 없어진다 — 첫 실행이 곧 영구 미사용이 된다.
  // 그래서 카드가 아니라 **텍스트 버튼 한 줄**만 남긴다. 누르기 전에는 입력창도 없다.
  const empty = items.length === 0 && suggestions.length === 0;
  if (empty && !opened) {
    // 채널에서도 같은 판단이되, 진입로는 카드 한 줄로 남긴다 — 완전히 없애면 세션 중에
    // 떠오른 할 일을 담을 데가 사라지고, 그것이 이 채널의 큰 값이다.
    if (channel) {
      return (
        <ChannelCard label="오늘 할 일" className="shrink-0">
          <button
            type="button"
            className="px-3 py-2 text-left text-xs text-text-muted hover:text-text"
            onClick={() => setOpened(true)}
          >
            + 오늘 할 일
          </button>
        </ChannelCard>
      );
    }
    return (
      <button
        type="button"
        className="mb-6 text-xs text-text-muted hover:text-text"
        onClick={() => setOpened(true)}
      >
        + 오늘 할 일
      </button>
    );
  }

  const ordered = sortItems(items);
  const { done, total } = progress(items);
  const day = items[0]?.day;
  // 할 일은 레포에 매이지 않는 전역 레이어라 여러 레포의 일이 한 목록에 섞인다.
  // 섞였을 때만 어느 레포 일인지 밝힌다 (today-items.showRepoBadges).
  const withRepo = showRepoBadges(items, repo);

  // 채널은 참조·체크용 압축본이다. 정렬·삭제·착수·제안·마감은 정본인 Home에만 둔다 —
  // 폭 300px에 손잡이를 다 욱여넣으면 무엇 하나 제대로 눌리지 않는다.
  if (channel) {
    return (
      <ChannelCard label="오늘 할 일" className="shrink-0 max-h-[var(--channel-today-max-height,45%)]">
        <div className="flex shrink-0 items-center gap-2 border-b border-border px-3 py-2.5">
          <h2 className="text-xs font-medium text-text-secondary">오늘 할 일</h2>
          {total > 0 && (
            <span className="text-xs text-text-muted">
              {done}/{total}
            </span>
          )}
        </div>

        {error && <div className="px-3 py-1.5 text-xs text-status-failed">{error}</div>}

        <div className="min-h-0 overflow-y-auto">
          {ordered.map((item) => {
            const task = item.task_id ? tasks.find((t) => t.id === item.task_id) : undefined;
            return (
              <div key={item.id} className="flex items-center gap-2 px-3 py-1.5 text-xs">
                <input
                  type="checkbox"
                  aria-label={item.title}
                  checked={item.status === "done"}
                  disabled={busy}
                  onChange={() =>
                    void run(() =>
                      api.setStatus(item.id, item.status === "done" ? "open" : "done"),
                    )
                  }
                />
                <span
                  className={`flex-1 truncate ${
                    item.status === "done"
                      ? "text-text-muted"
                      : item.status === "dropped"
                        ? "line-through text-text-muted"
                        : "text-text-secondary"
                  }`}
                  title={item.title}
                >
                  {item.title}
                </span>
                {withRepo && item.repo && (
                  <span
                    aria-label={`레포 ${repoBadge(item.repo)}`}
                    title={item.repo}
                    className="shrink-0 rounded bg-surface px-1.5 py-0.5 font-code text-[11px] text-text-muted"
                  >
                    {repoBadge(item.repo)}
                  </span>
                )}
                {task && (
                  <span
                    aria-label={`작업 ${task.id}`}
                    className={`shrink-0 ${taskTextClass(task)}`}
                  >
                    {taskStatusLabel(task)}
                  </span>
                )}
              </div>
            );
          })}
        </div>

        {/* 담기까지가 채널의 몫이다 — 세션 중에 떠오른 것을 홈으로 나가지 않고 적는다. */}
        <input
          className="shrink-0 border-t border-border bg-transparent px-3 py-2 text-xs placeholder:text-text-muted"
          placeholder="여기에 한 줄로 적기"
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
      </ChannelCard>
    );
  }

  return (
    <section className="mb-6" aria-label="오늘 할 일">
      <div className="flex items-center gap-2 mb-2">
        <span className="text-xs uppercase tracking-wide text-text-muted">오늘 할 일</span>
        {total > 0 && (
          <span className="text-xs text-text-muted">
            {done}/{total}
          </span>
        )}
        <div className="flex-1" />
        {items.length > 0 && (
          <button
            type="button"
            className="text-xs text-text-muted hover:text-text"
            disabled={busy}
            onClick={() => void run(async () => setClosing(await api.close()))}
          >
            하루 마감
          </button>
        )}
      </div>

      {error && <div className="text-xs text-status-failed mb-2">{error}</div>}

      {/* 항목이 없을 때 빈 테두리 상자를 남기지 않는다 — 그것도 껍데기다. */}
      <div
        className={`${ordered.length > 0 ? "border border-border-strong rounded-md overflow-hidden mb-2" : ""}`}
      >
        {ordered.map((item, index) => {
          const task = item.task_id ? tasks.find((t) => t.id === item.task_id) : undefined;
          const carried = carriedLabel(item);
          return (
            <div
              key={item.id}
              className="flex items-center gap-2.5 px-3 py-2 border-b border-border last:border-b-0 bg-raised"
            >
              <input
                type="checkbox"
                aria-label={item.title}
                checked={item.status === "done"}
                disabled={busy}
                onChange={() =>
                  void run(() =>
                    api.setStatus(item.id, item.status === "done" ? "open" : "done"),
                  )
                }
              />
              <span
                className={`flex-1 text-sm truncate ${
                  item.status === "dropped" ? "line-through text-text-muted" : ""
                }`}
              >
                {item.title}
              </span>

              {/* 오늘 정한 일이 아니라는 표시. 경고가 아니라 사실 전달이라 색을 쓰지 않는다. */}
              {carried && (
                <span
                  aria-label={`${carried}에서 넘어온 항목`}
                  title={`${item.carried_from}에서 넘어왔습니다`}
                  className="text-[11px] text-text-muted shrink-0"
                >
                  ↩ {carried}
                </span>
              )}

              {withRepo && item.repo && (
                <span
                  aria-label={`레포 ${repoBadge(item.repo)}`}
                  title={item.repo}
                  className="text-[11px] px-1.5 py-0.5 rounded bg-surface text-text-muted font-code shrink-0"
                >
                  {repoBadge(item.repo)}
                </span>
              )}

              {task && (
                <span
                  aria-label={`작업 ${task.id}`}
                  className={`text-xs ${taskTextClass(task)}`}
                >
                  {taskStatusLabel(task)}
                </span>
              )}

              <button
                type="button"
                className="text-xs text-text-muted hover:text-text disabled:opacity-40"
                disabled={busy || !item.repo || item.task_id !== null}
                title={item.repo ? "에이전트에게 맡기기" : "레포가 없는 항목은 착수할 수 없습니다"}
                onClick={() => void run(() => api.start(item.id, agent, mode))}
              >
                착수
              </button>

              {/* 오늘 목록의 세 번째 출구. 이 버튼이 없으면 밀린 일의 선택지는 "매일
                  따라오게 두기"와 "안 하기로 접기"뿐이고, 둘 다 사실이 아니다 (플랜 0054).
                  끝난 항목에는 달지 않는다 — 끝난 결정을 미결로 되돌리는 경로다. */}
              {item.status === "open" && (
                <button
                  type="button"
                  aria-label={`${item.title} 나중에`}
                  title="백로그로 미루기"
                  className="text-xs text-text-muted hover:text-text disabled:opacity-40"
                  disabled={busy}
                  onClick={() => void run(() => api.move(item.id, BACKLOG))}
                >
                  나중에
                </button>
              )}

              <button
                type="button"
                aria-label={`${item.title} 위로`}
                className="text-xs text-text-muted hover:text-text disabled:opacity-40"
                disabled={busy || index === 0 || !day}
                onClick={() => void run(() => api.reorder(day!, moveItem(items, item.id, -1)))}
              >
                ↑
              </button>
              <button
                type="button"
                aria-label={`${item.title} 아래로`}
                className="text-xs text-text-muted hover:text-text disabled:opacity-40"
                disabled={busy || index === ordered.length - 1 || !day}
                onClick={() => void run(() => api.reorder(day!, moveItem(items, item.id, 1)))}
              >
                ↓
              </button>
              <button
                type="button"
                aria-label={`${item.title} 삭제`}
                className="text-xs text-text-muted hover:text-status-failed"
                disabled={busy}
                onClick={() => void run(() => api.remove(item.id))}
              >
                ×
              </button>
            </div>
          );
        })}
      </div>

      {/* 입력은 한 줄 + Enter가 전부다. 우선순위·필드 선택 같은 의식을 만들지 않는다
          (설계 0021 §12 방어책 1). */}
      <input
        className="w-full text-sm bg-transparent border border-border rounded px-2.5 py-1.5 placeholder:text-text-muted"
        placeholder="오늘 할 일을 한 줄로 적어보세요"
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

      {suggestions.length > 0 && (
        <div className="mt-2">
          <button
            type="button"
            className="text-xs text-text-muted hover:text-text"
            onClick={() => setSuggestOpen((v) => !v)}
          >
            제안 {suggestions.length}건 {suggestOpen ? "접기" : "펼치기"}
          </button>
          {suggestOpen && (
            <div className="mt-1.5 border border-border rounded-md overflow-hidden">
              {groupSuggestions(suggestions).map((group) => (
                <div key={group.source}>
                  <div className="px-3 py-1 text-xs text-text-muted bg-surface">{group.label}</div>
                  {group.items.map((s) => (
                    <div
                      key={`${s.source}:${s.source_ref}`}
                      className="flex items-center gap-2.5 px-3 py-1.5 border-b border-border last:border-b-0"
                    >
                      <span className="flex-1 text-sm truncate">{s.title}</span>
                      {/* 지금 보고 있는 레포의 제안은 굳이 이름을 반복하지 않는다. */}
                      {s.repo && s.repo !== repo && (
                        <span
                          title={s.repo}
                          className="text-[11px] px-1.5 py-0.5 rounded bg-surface text-text-muted font-code shrink-0"
                        >
                          {repoBadge(s.repo)}
                        </span>
                      )}
                      <button
                        type="button"
                        className="text-xs text-text-muted hover:text-text"
                        disabled={busy}
                        onClick={() => void run(() => api.take(s))}
                      >
                        담기
                      </button>
                    </div>
                  ))}
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {closing && (
        <div className="mt-2 border border-border-strong rounded-md p-3" role="dialog" aria-label="하루 마감">
          <div className="text-sm mb-2">
            한 것 {closing.done} · 못 한 것 {closing.open} · 접은 것 {closing.dropped}
          </div>
          {/* 원장에 자동으로 쓰지 않는다 — 초안까지만 (설계 0021 DR-6). */}
          <textarea
            readOnly
            aria-label="원장 초안"
            className="w-full h-40 text-xs font-mono bg-transparent border border-border rounded p-2"
            value={closing.draft}
          />
          <div className="flex gap-2 mt-2">
            <button
              type="button"
              className="text-xs text-text-muted hover:text-text"
              onClick={() => void copyText(closing.draft).catch(() => undefined)}
            >
              복사
            </button>
            <button
              type="button"
              className="text-xs text-text-muted hover:text-text"
              onClick={() => setClosing(null)}
            >
              닫기
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
