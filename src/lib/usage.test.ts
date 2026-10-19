import { describe, expect, it } from "vitest";
import type { UsageWindow, VendorUsage } from "./ipc";
import {
  fmtAge,
  fmtReset,
  gaugeColor,
  gaugeTextClass,
  remaining,
  sourceLabel,
  summarize,
  weeklyCells,
  weeklyTitle,
  windowLabel,
  worstRemaining,
  STALE_AFTER_SECS,
} from "./usage";

const win = (used: number, minutes: number | null = 300): UsageWindow => ({
  used_percent: used,
  resets_at: null,
  window_minutes: minutes,
});

const vendor = (over: Partial<VendorUsage> = {}): VendorUsage => ({
  vendor: "codex",
  label: "Codex",
  status: "ok",
  detail: null,
  plan: null,
  five_hour: null,
  weekly: null,
  source: null,
  updated_at: null,
  ...over,
});

describe("remaining", () => {
  it("소진율의 여집합", () => {
    expect(remaining(win(14))).toBe(86);
    expect(remaining(win(0))).toBe(100);
  });

  it("범위를 벗어난 값도 0~100으로 눌린다", () => {
    expect(remaining(win(140))).toBe(0);
    expect(remaining(win(-5))).toBe(100);
  });
});

describe("gaugeColor / gaugeTextClass", () => {
  it("20% 미만은 위험색", () => {
    expect(gaugeColor(19)).toBe("var(--c-failed)");
    expect(gaugeTextClass(0)).toBe("text-status-failed");
  });

  it("20~50%는 주의색", () => {
    expect(gaugeColor(20)).toBe("var(--c-awaiting)");
    expect(gaugeTextClass(49)).toBe("text-status-awaiting");
  });

  it("50% 이상은 여유색", () => {
    expect(gaugeColor(50)).toBe("#2dd4bf");
    expect(gaugeTextClass(86)).toBe("text-text-secondary");
  });
});

describe("fmtReset", () => {
  it("없으면 null", () => {
    expect(fmtReset(null, 1000)).toBeNull();
  });

  it("이미 지났으면 곧 리셋", () => {
    expect(fmtReset(900, 1000)).toBe("곧 리셋");
    expect(fmtReset(1000, 1000)).toBe("곧 리셋");
  });

  it("한 시간 미만은 분, 하루 미만은 시간, 그 이상은 일", () => {
    expect(fmtReset(1000 + 12 * 60, 1000)).toBe("12분 후");
    expect(fmtReset(1000 + 3 * 3600, 1000)).toBe("3시간 후");
    expect(fmtReset(1000 + 2 * 86400, 1000)).toBe("2일 후");
  });

  it("1분 미만도 0분이 아니라 1분으로 올린다", () => {
    expect(fmtReset(1010, 1000)).toBe("1분 후");
  });
});

describe("windowLabel", () => {
  it("길이에 맞는 단위", () => {
    expect(windowLabel(300, "-")).toBe("5시간");
    expect(windowLabel(10080, "-")).toBe("주간");
    expect(windowLabel(2880, "-")).toBe("2일");
    expect(windowLabel(30, "-")).toBe("30분");
  });

  it("값이 없으면 fallback", () => {
    expect(windowLabel(null, "5시간")).toBe("5시간");
  });
});

describe("summarize", () => {
  it("Antigravity 칩은 Gemini 대표 창을 명시하고 다른 그룹과 합치지 않는다", () => {
    const v = vendor({
      vendor: "agy", five_hour: win(10), weekly: win(2, 10080),
      quota_groups: [
        { label: "Gemini Models", five_hour: win(10), weekly: win(2, 10080) },
        { label: "Claude and GPT models", five_hour: win(75), weekly: null },
      ],
    });
    expect(summarize(v).text).toBe("Gemini 90% · 주 98%");
    expect(worstRemaining(v)).toBe(90);
    expect(summarize({ ...v, status: "stale" }).tone).toBe("muted");
  });

  it("Gemini 창이 없어도 반환된 그룹의 값을 표시한다", () => {
    const v = vendor({ quota_groups: [{ label: "Claude and GPT models", five_hour: win(75), weekly: null }] });
    expect(summarize(v).text).toBe("Claude and GPT models 25%");
  });
  it("ok면 남은 비율을 두 윈도로", () => {
    const v = vendor({ five_hour: win(14), weekly: win(25, 10080) });
    expect(summarize(v)).toEqual({ text: "86% · 주 75%", tone: "value" });
  });

  it("한쪽 윈도만 있어도 표시", () => {
    expect(summarize(vendor({ five_hour: win(40) })).text).toBe("60%");
  });

  it("ok가 아니면 상태 문구", () => {
    expect(summarize(vendor({ status: "unauthenticated" }))).toEqual({
      text: "로그인 필요",
      tone: "muted",
    });
    expect(summarize(vendor({ status: "unsupported" })).text).toBe("미지원");
    expect(summarize(vendor({ status: "error" })).text).toBe("조회 실패");
  });

  it("ok인데 윈도가 비면 데이터 없음", () => {
    expect(summarize(vendor()).tone).toBe("muted");
  });

  it("stale은 값을 그대로 내되 tone을 죽인다 — 만료는 로그아웃이 아니다", () => {
    const v = vendor({ status: "stale", five_hour: win(14), weekly: win(25, 10080) });
    expect(summarize(v)).toEqual({ text: "86% · 주 75%", tone: "muted" });
  });
});

describe("worstRemaining", () => {
  it("두 윈도 중 더 급한 쪽", () => {
    expect(worstRemaining(vendor({ five_hour: win(14), weekly: win(90, 10080) }))).toBe(10);
  });

  it("값이 없으면 null", () => {
    expect(worstRemaining(vendor())).toBeNull();
  });
});

describe("fmtAge", () => {
  it("관측 나이를 단위 하나로 줄인다", () => {
    expect(fmtAge(0)).toBe("방금");
    expect(fmtAge(59)).toBe("방금");
    expect(fmtAge(600)).toBe("10분 전");
    expect(fmtAge(6 * 3600)).toBe("6시간 전");
    expect(fmtAge(7 * 86400)).toBe("7일 전");
  });
});

describe("weeklyCells", () => {
  const NOW = 1_000_000;
  /** 주간 창 — 리셋은 기본적으로 미래(아직 유효한 주기). */
  const weekWin = (used: number, resetsAt: number | null = NOW + 86_400): UsageWindow => ({
    used_percent: used,
    resets_at: resetsAt,
    window_minutes: 10080,
  });
  const cell = (cells: ReturnType<typeof weeklyCells>, vendor: string) =>
    cells.find((c) => c.vendor === vendor)!;

  it("벤더마다 한 칸을 고정 순서로 세운다 — 값이 없어도 자리는 지킨다", () => {
    const cells = weeklyCells([], NOW);
    expect(cells.map((c) => c.vendor)).toEqual(["claude", "codex"]);
    expect(cells.map((c) => c.label)).toEqual(["Claude 주간", "Codex 주간"]);
    expect(cells.every((c) => c.remaining == null)).toBe(true);
  });

  it("두 벤더 값을 하나로 뭉치지 않는다 — 각자의 잔량이 각자의 칸에 남는다", () => {
    const cells = weeklyCells(
      [
        vendor({ vendor: "codex", weekly: weekWin(30), updated_at: NOW }),
        vendor({ vendor: "claude", weekly: weekWin(72), updated_at: NOW }),
      ],
      NOW,
    );
    expect(cell(cells, "codex").remaining).toBe(70);
    expect(cell(cells, "claude").remaining).toBe(28);
  });

  it("5시간 창만 있는 벤더는 잔량이 아니라 사정을 남긴다", () => {
    const cells = weeklyCells([vendor({ vendor: "codex", five_hour: win(50) })], NOW);
    expect(cell(cells, "codex").remaining).toBeNull();
    expect(cell(cells, "codex").reason).toContain("주간 한도");
  });

  it("리셋이 지난 창은 무효 — 옛 소진율을 지금 잔량으로 내주지 않는다", () => {
    const cells = weeklyCells(
      [vendor({ vendor: "codex", weekly: weekWin(20, NOW - 1), updated_at: NOW - 10 })],
      NOW,
    );
    expect(cell(cells, "codex").remaining).toBeNull();
    expect(cell(cells, "codex").reason).toContain("리셋");
  });

  it("리셋 시각을 주지 않는 소스는 그 이유로 버리지 않는다", () => {
    const cells = weeklyCells(
      [vendor({ vendor: "codex", weekly: weekWin(20, null), updated_at: NOW })],
      NOW,
    );
    expect(cell(cells, "codex").remaining).toBe(80);
  });

  it("오래된 관측은 값을 남기되 낡음으로 표시한다", () => {
    const cells = weeklyCells(
      [
        vendor({
          vendor: "codex",
          weekly: weekWin(32),
          updated_at: NOW - STALE_AFTER_SECS - 1,
        }),
      ],
      NOW,
    );
    const c = cell(cells, "codex");
    expect(c.remaining).toBe(68);
    expect(c.stale).toBe(true);
    expect(c.ageSecs).toBe(STALE_AFTER_SECS + 1);
  });

  it("신선한 관측은 낡음이 아니다", () => {
    const cells = weeklyCells(
      [vendor({ vendor: "codex", weekly: weekWin(32), updated_at: NOW - 60 })],
      NOW,
    );
    expect(cell(cells, "codex").stale).toBe(false);
    expect(cell(cells, "codex").reason).toBeNull();
  });

  it("조회에 실패한 벤더는 그 칸에서 이유를 말한다 — 다른 벤더 값이 대신 서지 않는다", () => {
    const cells = weeklyCells(
      [
        vendor({ vendor: "claude", status: "unauthenticated", detail: "다시 로그인하세요" }),
        vendor({ vendor: "codex", weekly: weekWin(40), updated_at: NOW }),
      ],
      NOW,
    );
    expect(cell(cells, "claude").remaining).toBeNull();
    expect(cell(cells, "claude").reason).toBe("다시 로그인하세요");
    expect(cell(cells, "codex").remaining).toBe(60);
  });

  it("detail이 없는 실패는 상태 문구로 대신한다", () => {
    const cells = weeklyCells([vendor({ vendor: "claude", status: "error" })], NOW);
    expect(cell(cells, "claude").reason).toBe("조회 실패");
  });

  it("stale은 값을 남긴 채 낡음으로 세운다 — 나이가 짧아도 마찬가지다", () => {
    const cells = weeklyCells(
      [
        vendor({
          vendor: "claude",
          status: "stale",
          weekly: weekWin(30),
          updated_at: NOW - 60,
          detail: "토큰이 만료돼 새 값을 받지 못했습니다",
        }),
      ],
      NOW,
    );
    const c = cell(cells, "claude");
    expect(c.remaining).toBe(70);
    expect(c.stale).toBe(true);
    expect(c.reason).toBe("토큰이 만료돼 새 값을 받지 못했습니다");
  });
});

describe("weeklyTitle", () => {
  it("라벨·관측 나이·사정을 한 줄로 잇는다", () => {
    expect(
      weeklyTitle({
        vendor: "codex",
        label: "Codex 주간",
        remaining: 68,
        stale: true,
        ageSecs: 6 * 3600,
        reason: "관측 이후 쓴 만큼은 아직 반영되지 않았습니다",
      }),
    ).toBe("Codex 주간 · 6시간 전 관측 · 관측 이후 쓴 만큼은 아직 반영되지 않았습니다");
  });

  it("사정이 없으면 라벨과 나이만", () => {
    expect(
      weeklyTitle({
        vendor: "claude",
        label: "Claude 주간",
        remaining: 80,
        stale: false,
        ageSecs: 120,
        reason: null,
      }),
    ).toBe("Claude 주간 · 2분 전 관측");
  });
});

describe("sourceLabel", () => {
  it("알려진 출처는 한국어로", () => {
    expect(sourceLabel("session-log")).toBe("세션 로그");
    expect(sourceLabel("statusline")).toBe("statusline 브리지");
    expect(sourceLabel("oauth")).toBe("사용량 API");
    expect(sourceLabel("manual-token")).toBe("등록한 조회 토큰");
    expect(sourceLabel("antigravity-local")).toBe("Antigravity 앱");
    expect(sourceLabel("antigravity-oauth")).toBe("Antigravity 사용량 API");
  });

  it("모르는 값/없음은 null", () => {
    expect(sourceLabel(null)).toBeNull();
    expect(sourceLabel("mystery")).toBeNull();
  });
});
