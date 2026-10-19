import type { UsageWindow, VendorUsage } from "./ipc";

/** 사용 한도 표시 로직 — 컴포넌트에서 분리해 단위 테스트로 고정한다. */

/** 남은 비율(0~100). 소진율의 여집합. */
export const remaining = (w: UsageWindow) => Math.max(0, Math.min(100, 100 - w.used_percent));

/** 남은 비율 → 게이지/텍스트 색. 20% 미만 위험, 50% 미만 주의. */
export function gaugeColor(remainingPercent: number): string {
  if (remainingPercent < 20) return "var(--c-failed)";
  if (remainingPercent < 50) return "var(--c-awaiting)";
  return "#2dd4bf";
}

/** 색과 짝이 되는 텍스트 클래스 — 배지 숫자에 쓴다. */
export function gaugeTextClass(remainingPercent: number): string {
  if (remainingPercent < 20) return "text-status-failed";
  if (remainingPercent < 50) return "text-status-awaiting";
  return "text-text-secondary";
}

/** 리셋까지 남은 시간 → "12분 후" / "3시간 후" / "2일 후". 지났으면 "곧 리셋". */
export function fmtReset(resetsAt: number | null, nowSecs: number): string | null {
  if (resetsAt == null) return null;
  const d = resetsAt - nowSecs;
  if (d <= 0) return "곧 리셋";
  if (d < 3600) return `${Math.max(1, Math.round(d / 60))}분 후`;
  if (d < 86400) return `${Math.round(d / 3600)}시간 후`;
  return `${Math.round(d / 86400)}일 후`;
}

/** 관측 나이(초) → "방금" / "12분 전" / "6시간 전" / "3일 전". */
export function fmtAge(secs: number): string {
  if (secs < 60) return "방금";
  if (secs < 3600) return `${Math.floor(secs / 60)}분 전`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}시간 전`;
  return `${Math.floor(secs / 86400)}일 전`;
}

/** 윈도 길이(분) → "5시간" / "주간" 같은 라벨. 값이 없으면 fallback. */
export function windowLabel(minutes: number | null, fallback: string): string {
  if (minutes == null) return fallback;
  if (minutes >= 10080) return "주간";
  if (minutes >= 1440) return `${Math.round(minutes / 1440)}일`;
  if (minutes >= 60) return `${Math.round(minutes / 60)}시간`;
  return `${minutes}분`;
}

/** 값을 보여줄 수 있는 상태 — stale은 낡았을 뿐 값은 있다. */
const hasValues = (status: string) => status === "ok" || status === "stale";

/**
 * 상태바 배지에 쓸 한 줄 요약. 값이 없는 상태는 상태 문구를 대신 보여준다.
 * stale은 값을 그대로 내되 tone을 죽인다 — 숫자가 지금 값이 아니라는 뜻이다.
 */
export function summarize(v: VendorUsage): { text: string; tone: "value" | "muted" } {
  if (!hasValues(v.status)) {
    const label: Record<string, string> = {
      no_data: "데이터 없음",
      unauthenticated: "로그인 필요",
      unsupported: "미지원",
      error: "조회 실패",
    };
    return { text: label[v.status] ?? "알 수 없음", tone: "muted" };
  }
  const parts: string[] = [];
  if (v.five_hour) {
    parts.push(`${v.quota_groups?.length ? "Gemini " : ""}${Math.round(remaining(v.five_hour))}%`);
  }
  if (v.weekly) {
    parts.push(`${!v.five_hour && v.quota_groups?.length ? "Gemini " : ""}주 ${Math.round(remaining(v.weekly))}%`);
  }
  if (parts.length === 0 && v.quota_groups?.length) {
    for (const group of v.quota_groups) {
      const windows = [group.five_hour, group.weekly].filter((w): w is UsageWindow => w != null);
      if (windows.length) parts.push(`${group.label} ${Math.round(Math.min(...windows.map(remaining)))}%`);
    }
  }
  if (parts.length === 0) return { text: "데이터 없음", tone: "muted" };
  return { text: parts.join(" · "), tone: v.status === "stale" ? "muted" : "value" };
}

/** 벤더 전체에서 가장 급한 남은 비율 — 상태바 대표색 판단용. 값이 없으면 null. */
export function worstRemaining(v: VendorUsage): number | null {
  const values = [v.five_hour, v.weekly].filter((w): w is UsageWindow => w != null).map(remaining);
  return values.length > 0 ? Math.min(...values) : null;
}

// ------------------------------------------------------- 홈 스트립의 주간 잔량

/**
 * 관측을 신선하다고 볼 최대 나이(초).
 *
 * 두 벤더 모두 값이 **관측 시점의 스냅샷**이지 실시간 잔량이 아니다 — Codex는 세션 로그에
 * 마지막으로 찍힌 `rate_limits`이고, Claude는 statusline 브리지가 마지막으로 덤프한 값이다.
 * 그 사이에 쓴 만큼은 어느 쪽에도 반영되지 않으므로, 오래된 값은 "맞는 값"이 아니라
 * "낡은 값"으로 보여야 한다.
 */
export const STALE_AFTER_SECS = 3600;

/** 홈이 잔량 칸을 세우는 벤더 — 주간 창을 내는 곳만, 고정 순서. 값이 없어도 자리는 지킨다. */
export const WEEKLY_VENDORS: readonly { vendor: string; label: string }[] = [
  { vendor: "claude", label: "Claude 주간" },
  { vendor: "codex", label: "Codex 주간" },
];

/** 홈 스트립 잔량 한 칸이 쓰는 벤더별 주간 상태. */
export interface WeeklyCell {
  vendor: string;
  label: string;
  /** 남은 비율(0~100). 값을 신뢰할 수 없으면 null. */
  remaining: number | null;
  /** 관측이 오래됐다 — 값은 남기되 색을 죽이고 나이를 함께 세운다. */
  stale: boolean;
  /** 관측 나이(초). 관측 시각을 모르면 null. */
  ageSecs: number | null;
  /** 값이 없거나 낡은 사정 — 툴팁으로 나간다. ok이고 신선하면 null. */
  reason: string | null;
}

function weeklyCell(
  vendor: string,
  label: string,
  v: VendorUsage | undefined,
  now: number,
): WeeklyCell {
  const blank = (reason: string): WeeklyCell => ({
    vendor,
    label,
    remaining: null,
    stale: false,
    ageSecs: null,
    reason,
  });

  if (!v) return blank("사용량 스냅샷에 이 벤더가 없습니다");
  if (!hasValues(v.status)) return blank(v.detail ?? summarize(v).text);
  if (!v.weekly) return blank("주간 한도 정보를 주지 않는 소스입니다");
  // 리셋 시각이 지난 창은 이미 새 주기로 넘어갔다 — 마지막 관측의 소진율은 지금 잔량이 아니다.
  // 여기서 100%로 단정하지 않는 이유: 새 주기에 이미 쓴 양을 우리는 모른다.
  if (v.weekly.resets_at != null && v.weekly.resets_at <= now) {
    return blank("주간 창이 리셋돼 마지막 관측이 무효입니다 — 새 관측을 기다립니다");
  }

  const ageSecs = v.updated_at != null ? Math.max(0, now - v.updated_at) : null;
  const aged = ageSecs != null && ageSecs > STALE_AFTER_SECS;
  return {
    vendor,
    label,
    remaining: remaining(v.weekly),
    stale: aged || v.status === "stale",
    ageSecs,
    reason: staleReason(v, aged),
  };
}

/** 값이 지금 값이 아닐 수 있는 사정. 신선한 ok면 null. */
function staleReason(v: VendorUsage, aged: boolean): string | null {
  if (v.status === "stale") return v.detail ?? "새 값을 받지 못해 마지막 관측을 보여줍니다";
  if (aged) return "관측 이후 쓴 만큼은 아직 반영되지 않았습니다";
  return null;
}

/**
 * 벤더별 주간 잔량 — 홈은 한 숫자로 뭉치지 않는다.
 *
 * 예전에는 두 벤더 중 더 급한 쪽 하나만 보여줬는데, 그러면 (a) 어느 벤더의 값인지 알 수 없고
 * (b) 한쪽이 조회에 실패하면 남은 한쪽 값이 조용히 전체를 대표해 "안 맞는" 것처럼 보였다.
 * 벤더 수만큼 칸을 세우고 값이 없으면 그 자리에서 이유를 말한다.
 */
export function weeklyCells(vendors: readonly VendorUsage[], now: number): WeeklyCell[] {
  return WEEKLY_VENDORS.map(({ vendor, label }) =>
    weeklyCell(
      vendor,
      label,
      vendors.find((v) => v.vendor === vendor),
      now,
    ),
  );
}

/** 잔량 칸 툴팁 — 값의 출처와 한계를 한 줄로. */
export function weeklyTitle(c: WeeklyCell): string {
  const parts = [c.label];
  if (c.ageSecs != null) parts.push(`${fmtAge(c.ageSecs)} 관측`);
  if (c.reason) parts.push(c.reason);
  return parts.join(" · ");
}

/** 값의 출처를 사람이 읽는 문구로. */
export function sourceLabel(source: string | null): string | null {
  if (source === "antigravity-oauth") return "Antigravity 사용량 API";
  if (source === "antigravity-local") return "Antigravity 앱";
  if (source === "session-log") return "세션 로그";
  if (source === "statusline") return "statusline 브리지";
  if (source === "oauth") return "사용량 API";
  if (source === "manual-token") return "등록한 조회 토큰";
  return null;
}
