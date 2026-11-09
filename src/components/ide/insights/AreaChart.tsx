import { useMemo, useState } from "react";
import type { DayStat } from "../../../lib/ipc";
import { fmtInt, fmtTokens } from "./format";

const DAY_MS = 86400000;
/** viewBox 좌표계 — preserveAspectRatio="none"으로 가로를 늘리고, 선은 non-scaling-stroke로 두께를 지킨다. */
const VW = 1000;

const toDayNum = (s: string) =>
  Date.UTC(Number(s.slice(0, 4)), Number(s.slice(5, 7)) - 1, Number(s.slice(8, 10))) / DAY_MS;
const toDateStr = (day: number) => new Date(day * DAY_MS).toISOString().slice(0, 10);
/** "2026-07-26" → "7/26" */
const shortDate = (s: string) => `${Number(s.slice(5, 7))}/${Number(s.slice(8, 10))}`;

/**
 * 일별 토큰 에리어 + 메시지 라인 오버레이.
 * `days`는 활동이 있는 날만 담고 있으므로 빈 날을 0으로 채워 x축 간격을 실제 날짜에 맞춘다.
 */
export function AreaChart({ days, height = 180 }: { days: DayStat[]; height?: number }) {
  const [hover, setHover] = useState<number | null>(null);

  const series = useMemo(() => {
    if (days.length === 0) return [];
    const map = new Map(days.map((d) => [toDayNum(d.date), d]));
    const start = toDayNum(days[0].date);
    const end = toDayNum(days[days.length - 1].date);
    const out: DayStat[] = [];
    for (let d = start; d <= end; d++) {
      out.push(map.get(d) ?? { date: toDateStr(d), messages: 0, tokens: 0 });
    }
    // 점 하나면 폭 0이라 그려지지 않는다 — 복제해 평평한 구간으로 만든다.
    return out.length === 1 ? [out[0], out[0]] : out;
  }, [days]);

  const n = series.length;
  const maxTokens = Math.max(1, ...series.map((d) => d.tokens));
  const maxMessages = Math.max(1, ...series.map((d) => d.messages));

  const x = (i: number) => (i / Math.max(1, n - 1)) * VW;
  const yOf = (v: number, max: number) => height - (v / max) * height;

  const { area, line, msgLine } = useMemo(() => {
    if (n === 0) return { area: "", line: "", msgLine: "" };
    const pts = series.map((d, i) => `${x(i).toFixed(2)},${yOf(d.tokens, maxTokens).toFixed(2)}`);
    const msgPts = series.map(
      (d, i) => `${x(i).toFixed(2)},${yOf(d.messages, maxMessages).toFixed(2)}`,
    );
    return {
      area: `M0,${height} L${pts.join(" L")} L${VW},${height} Z`,
      line: `M${pts.join(" L")}`,
      msgLine: `M${msgPts.join(" L")}`,
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [series, maxTokens, maxMessages, height]);

  if (n === 0) {
    return (
      <div
        className="flex items-center justify-center text-text-muted text-sm"
        style={{ height }}
      >
        표시할 활동이 없습니다
      </div>
    );
  }

  const active = hover != null ? series[hover] : null;

  const onMove = (e: React.MouseEvent<HTMLDivElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    if (r.width <= 0) return;
    const t = (e.clientX - r.left) / r.width;
    setHover(Math.min(n - 1, Math.max(0, Math.round(t * (n - 1)))));
  };

  return (
    <div className="relative select-none" onMouseMove={onMove} onMouseLeave={() => setHover(null)}>
      <svg
        width="100%"
        height={height}
        viewBox={`0 0 ${VW} ${height}`}
        preserveAspectRatio="none"
        aria-hidden
      >
        <path d={area} style={{ fill: "rgb(from var(--c-primary-bright) r g b / 0.14)" }} />
        <path
          d={line}
          fill="none"
          style={{ stroke: "var(--c-primary-bright)" }}
          strokeWidth={1.5}
          vectorEffect="non-scaling-stroke"
        />
        <path
          d={msgLine}
          fill="none"
          stroke="var(--c-text-muted)"
          strokeWidth={1}
          strokeDasharray="3 3"
          vectorEffect="non-scaling-stroke"
        />
        {hover != null && (
          <line
            x1={x(hover)}
            x2={x(hover)}
            y1={0}
            y2={height}
            stroke="var(--c-border-strong)"
            strokeWidth={1}
            vectorEffect="non-scaling-stroke"
          />
        )}
      </svg>

      <div className="flex justify-between text-xs text-text-muted mt-1 font-code">
        <span>{shortDate(series[0].date)}</span>
        {n > 2 && <span>{shortDate(series[Math.floor((n - 1) / 2)].date)}</span>}
        <span>{shortDate(series[n - 1].date)}</span>
      </div>

      {active && (
        <div
          className="absolute top-0 pointer-events-none bg-raised border border-border-strong rounded-md px-2 py-1 text-xs whitespace-nowrap shadow-e2"
          style={{
            left: `${(hover! / Math.max(1, n - 1)) * 100}%`,
            transform: `translateX(${hover! > n / 2 ? "-105%" : "5%"})`,
          }}
        >
          <div className="font-code text-text-secondary">{active.date}</div>
          <div>
            <span style={{ color: "var(--c-primary-bright)" }}>{fmtTokens(active.tokens)}</span> 토큰
          </div>
          <div className="text-text-secondary">{fmtInt(active.messages)} 메시지</div>
        </div>
      )}
    </div>
  );
}
