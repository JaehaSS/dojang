import { useEffect, useMemo, useState, type ReactElement } from "react";
import { todayRange } from "../../../lib/ipc";
import { repoBadge, showRepoBadges, type DayItem } from "../today-items";
import { Icon } from "../icons";
import {
  addMonth,
  fmtDayLabel,
  itemsOf,
  kstToday,
  monthBounds,
  monthGrid,
  monthOf,
  summarize,
  type DaySummary,
} from "./plan-calendar";

const WEEKDAYS = ["일", "월", "화", "수", "목", "금", "토"];

/** 셀에 찍는 도트 상한. 넘으면 `+N`으로 접는다 — 격자가 무너지는 것을 막는다. */
const MAX_DOTS = 5;

/**
 * 상태색(`status-*`)을 쓰지 않는다 — 그 팔레트는 Task 상태 표시 전용이고,
 * 시각화의 강조는 teal 하나로 통일돼 있다 (`parts.tsx` DeltaChip 주석과 같은 규약).
 */
const DOT_COLOR = {
  done: "var(--c-primary-bright)",
  open: "var(--c-border-strong)",
  dropped: "var(--c-border)",
} as const;

export interface PlanCalendarApi {
  range: (from: string, to: string) => Promise<DayItem[]>;
}

interface Props {
  /** Home으로 보내는 링크. 없으면 링크를 숨긴다. */
  onOpenHome?: () => void;
  /** 테스트에서 "오늘"을 고정하기 위한 주입점. */
  nowMs?: number;
  api?: PlanCalendarApi;
}

/** 상태별 도트 묶음. 순서는 done → open → dropped 로 고정한다. */
function Dots({ summary }: { summary: DaySummary }) {
  const kinds: (keyof typeof DOT_COLOR)[] = [
    ...Array<"done">(summary.done).fill("done"),
    ...Array<"open">(summary.open).fill("open"),
    ...Array<"dropped">(summary.dropped).fill("dropped"),
  ];
  const shown = kinds.slice(0, MAX_DOTS);
  const rest = kinds.length - shown.length;

  return (
    <div className="flex items-center gap-[3px] mt-1 h-1.5">
      {shown.map((kind, i) => (
        <span
          key={i}
          className="w-1.5 h-1.5 rounded-full shrink-0"
          style={{ background: DOT_COLOR[kind] }}
          aria-hidden
        />
      ))}
      {rest > 0 && <span className="text-[11px] leading-none text-text-muted">+{rest}</span>}
    </div>
  );
}

/**
 * 인사이트의 계획 캘린더 — 월간 격자 + 선택일 목록 (설계 0023).
 *
 * **읽기 전용이다.** 계획 레이어의 정본 편집처는 Home의 `TodaySection` 하나로 유지한다.
 * 편집처가 둘이 되면 정렬·제안·마감이 어느 화면의 상태를 정본으로 볼지 모호해진다 (DR-2).
 *
 * 인사이트 상단의 range 칩(전체/30d/7d)과는 독립이다 — "전체"가 월 격자에 대응하지 않는다.
 */
export function PlanCalendar({ onOpenHome, nowMs, api }: Props = {}): ReactElement {
  const today = useMemo(() => kstToday(nowMs ?? Date.now()), [nowMs]);
  const [month, setMonth] = useState(() => monthOf(today));
  const [selected, setSelected] = useState(today);
  const [items, setItems] = useState<DayItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);

  const fetchRange = api?.range ?? todayRange;

  useEffect(() => {
    let alive = true;
    const { from, to } = monthBounds(month);
    setLoading(true);
    setErr(null);
    fetchRange(from, to)
      .then((rows) => alive && setItems(rows))
      .catch((e) => alive && setErr(String(e)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
    // fetchRange는 props에서 파생된 안정 참조다 — month가 바뀔 때만 다시 부른다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [month]);

  const grid = useMemo(() => monthGrid(month), [month]);
  const summaries = useMemo(() => summarize(items), [items]);
  const dayItems = useMemo(() => itemsOf(items, selected), [items, selected]);
  const selectedSummary = summaries.get(selected);
  const withRepo = showRepoBadges(dayItems, undefined);

  const [year, mon] = month.split("-").map(Number);

  const step = (delta: number) => {
    const next = addMonth(month, delta);
    setMonth(next);
    // 달을 옮기면 선택도 따라간다 — 안 보이는 날이 선택된 채 남으면 목록이 비어 보인다.
    setSelected(monthOf(today) === next ? today : `${next}-01`);
  };

  const navBtn = "h-7 w-7 rounded-md text-text-secondary hover:text-text hover:bg-raised";

  return (
    <>
      <div className="bg-surface border border-border rounded-lg p-4 mb-2.5">
        <div className="flex items-center justify-between mb-3">
          <button className={navBtn} onClick={() => step(-1)} aria-label="이전 달">
            ‹
          </button>
          <span className="text-sm">
            {year}년 {mon}월
          </span>
          <button className={navBtn} onClick={() => step(1)} aria-label="다음 달">
            ›
          </button>
        </div>

        <div className="grid grid-cols-7 gap-1 mb-1">
          {WEEKDAYS.map((w) => (
            <div key={w} className="text-[11px] text-text-muted text-center py-1">
              {w}
            </div>
          ))}
        </div>

        <div className="grid grid-cols-7 gap-1">
          {grid.flat().map((cell) => {
            const summary = summaries.get(cell.day);
            const isToday = cell.day === today;
            const isSelected = cell.day === selected;
            return (
              <button
                key={cell.day}
                onClick={() => setSelected(cell.day)}
                aria-label={`${fmtDayLabel(cell.day)}${
                  summary ? ` · 완료 ${summary.done}/${summary.total}` : " · 항목 없음"
                }`}
                aria-pressed={isSelected}
                className={`flex flex-col items-center rounded-md py-1.5 min-h-[42px] transition-colors ${
                  isSelected ? "bg-raised" : "hover:bg-raised/60"
                } ${isToday ? "border border-primary" : "border border-transparent"}`}
              >
                <span
                  className={`text-xs leading-none ${
                    cell.outside ? "text-text-muted/50" : isToday ? "text-primary-bright" : ""
                  }`}
                >
                  {cell.date}
                </span>
                {summary ? <Dots summary={summary} /> : <div className="mt-1 h-1.5" />}
              </button>
            );
          })}
        </div>
      </div>

      <div className="bg-surface border border-border rounded-lg">
        <div className="flex items-baseline justify-between gap-3 px-3 pt-3 pb-2">
          <span className="text-sm">
            {fmtDayLabel(selected)}
            {selected === today && <span className="text-text-muted text-xs"> (오늘)</span>}
          </span>
          {selectedSummary && (
            <span className="text-xs font-code text-text-secondary shrink-0">
              완료 {selectedSummary.done} / {selectedSummary.total}
            </span>
          )}
        </div>

        {err ? (
          <div className="text-status-failed text-sm font-code px-3 pb-3">{err}</div>
        ) : loading ? (
          <div className="text-text-muted text-sm px-3 pb-3">불러오는 중…</div>
        ) : dayItems.length === 0 ? (
          <div className="text-text-muted text-sm px-3 pb-3">이 날은 계획한 일이 없습니다.</div>
        ) : (
          dayItems.map((item) => (
            <div
              key={item.id}
              className="flex items-center gap-2.5 px-3 py-2 border-t border-border"
            >
              <span
                aria-hidden
                className={`text-sm w-3.5 shrink-0 ${
                  item.status === "done" ? "text-primary-bright" : "text-text-muted"
                }`}
              >
                <Icon name={item.status === "done" ? "check" : "circle"} size={12} />
              </span>
              <span
                className={`flex-1 text-sm truncate ${
                  item.status === "dropped"
                    ? "line-through text-text-muted"
                    : item.status === "done"
                      ? "text-text-secondary"
                      : ""
                }`}
                title={item.title}
              >
                {item.title}
              </span>
              {withRepo && item.repo && (
                <span
                  aria-label={`레포 ${repoBadge(item.repo)}`}
                  title={item.repo}
                  className="text-[11px] px-1.5 py-0.5 rounded bg-raised text-text-muted font-code shrink-0"
                >
                  {repoBadge(item.repo)}
                </span>
              )}
            </div>
          ))
        )}

        {onOpenHome && (
          <div className="px-3 py-2 border-t border-border text-right">
            <button
              onClick={onOpenHome}
              className="text-xs text-text-secondary hover:text-primary-bright"
            >
              Home에서 편집 →
            </button>
          </div>
        )}
      </div>
    </>
  );
}
