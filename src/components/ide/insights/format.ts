/** 인사이트 화면 전용 표시 포매터. 공용 포매터는 `src/lib/fmt.ts`. */

const DAY_MS = 86400000;

/** 천단위 구분 정수. */
export const fmtInt = (n: number) => n.toLocaleString();

/** 토큰 수 축약 — 1.2M / 45.3K / 812. */
export const fmtTokens = (n: number) =>
  n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}K` : String(n);

/** 0~23시 → "오전 9시" / "오후 3시". null이면 "—". */
export const fmtHour = (h: number | null) => {
  if (h == null) return "—";
  const ap = h < 12 ? "오전" : "오후";
  const hh = h % 12 === 0 ? 12 : h % 12;
  return `${ap} ${hh}시`;
};

/**
 * claude-opus-4-8 → Opus 4.8, claude-opus-5 → Opus 5 (표시용 축약). 모르면 원문.
 * 버전 조각은 1~2자리로 제한해 뒤따르는 날짜 스탬프(-20260101)를 마이너 버전으로 오인하지 않는다.
 *
 * 컨텍스트 변형 접미사(`[1m]`)는 남긴다 — 지우면 `claude-opus-5`와 `claude-opus-5[1m]`이
 * 한 이름으로 합쳐져, 한도·비용이 다른 둘을 인사이트에서 구분할 수 없다.
 */
export const prettyModel = (m: string) => {
  const x = m.match(/(opus|sonnet|haiku|fable|mythos)-(\d{1,2})(?:-(\d{1,2}))?(?!\d)/i);
  if (!x) return m;
  const family = `${x[1][0].toUpperCase()}${x[1].slice(1).toLowerCase()}`;
  const version = x[3] ? `${x[2]}.${x[3]}` : x[2];
  const variant = m.match(/\[([^\]]+)\]/);
  return variant ? `${family} ${version} · ${variant[1]}` : `${family} ${version}`;
};

/** 비율 → "84%". 분모가 0이면 "—". */
export const fmtPct = (num: number, den: number) =>
  den > 0 ? `${Math.round((num / den) * 100)}%` : "—";

/**
 * "YYYY-MM-DD" → "오늘" / "어제" / "3일 전". 미래거나 파싱 실패면 원문.
 * `now`는 테스트 주입용(기본 현재 시각).
 */
export const relativeDay = (date: string, now: Date = new Date()) => {
  const m = date.match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (!m) return date;
  const then = Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
  const today = Date.UTC(now.getFullYear(), now.getMonth(), now.getDate());
  const diff = Math.round((today - then) / DAY_MS);
  if (diff < 0) return date;
  if (diff === 0) return "오늘";
  if (diff === 1) return "어제";
  return `${diff}일 전`;
};

/**
 * 직전 구간 대비 증감률(%). 직전이 0이거나 없으면 null —
 * 0에서 증가한 비율은 정의되지 않으므로 Δ를 숨긴다.
 */
export const deltaPct = (cur: number, prev: number | undefined | null) => {
  if (prev == null || prev === 0) return null;
  return Math.round(((cur - prev) / prev) * 100);
};

/** 초 → 사람이 읽는 소요. 표본이 없으면 "—". */
export function fmtDuration(secs: number | null): string {
  if (secs == null) return "—";
  if (secs < 60) return `${secs}초`;
  if (secs < 3600) return `${Math.round(secs / 60)}분`;
  const hours = secs / 3600;
  return hours < 10 ? `${hours.toFixed(1)}시간` : `${Math.round(hours)}시간`;
}
