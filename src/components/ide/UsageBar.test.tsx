import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { VendorUsage } from "../../lib/ipc";
import { VendorChip, VendorDetail } from "./UsageBar";

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

const full = vendor({
  five_hour: { used_percent: 14, resets_at: 4000, window_minutes: 300 },
  weekly: { used_percent: 25, resets_at: 100_000, window_minutes: 10080 },
  plan: "plus",
  source: "session-log",
  updated_at: 900,
});

describe("VendorChip", () => {
  it("남은 비율을 두 윈도로 보여준다", () => {
    const html = renderToStaticMarkup(<VendorChip usage={full} active={false} onClick={() => {}} />);
    expect(html).toContain("Codex");
    expect(html).toContain("86% · 주 75%");
  });

  it("게이지 폭은 더 급한 윈도(주간 75%)를 따른다", () => {
    const html = renderToStaticMarkup(<VendorChip usage={full} active={false} onClick={() => {}} />);
    expect(html).toContain("width:75%");
  });

  it("지원하지 않는 벤더는 상태 문구만 낸다", () => {
    const html = renderToStaticMarkup(
      <VendorChip
        usage={vendor({ vendor: "agy", label: "Antigravity", status: "unsupported" })}
        active={false}
        onClick={() => {}}
      />,
    );
    expect(html).toContain("미지원");
    expect(html).not.toContain("width:");
  });
});

describe("VendorDetail", () => {
  const props = {
    now: 1000,
    bridge: null,
    busy: false,
    onInstall: () => {},
  };

  it("Antigravity 그룹별로 두 창을 표시하고 대표 창을 중복하지 않는다", () => {
    const usage = vendor({
      vendor: "agy", label: "Antigravity", source: "antigravity-local",
      five_hour: { used_percent: 10, resets_at: 4000, window_minutes: 300 },
      weekly: { used_percent: 2, resets_at: 100000, window_minutes: 10080 },
      quota_groups: [
        { label: "Gemini Models", five_hour: { used_percent: 10, resets_at: 4000, window_minutes: 300 }, weekly: { used_percent: 2, resets_at: 100000, window_minutes: 10080 } },
        { label: "Claude and GPT models", five_hour: { used_percent: 75, resets_at: null, window_minutes: 300 }, weekly: null },
      ],
    });
    const html = renderToStaticMarkup(<VendorDetail usage={usage} {...props} />);
    expect(html).toContain("Gemini Models");
    expect(html).toContain("Claude and GPT models");
    expect(html.match(/90% 남음/g)).toHaveLength(1);
    expect(html.match(/98% 남음/g)).toHaveLength(1);
    expect(html).toContain("25% 남음");
    expect(html).toContain("Antigravity 앱");
    expect(html).not.toContain("브리지 설치");
    const stale = renderToStaticMarkup(<VendorDetail usage={{ ...usage, status: "stale", updated_at: 100, detail: "앱을 실행하세요" }} {...props} />);
    expect(stale).toContain("90% 남음");
    expect(stale).toContain("앱을 실행하세요");
    expect(stale).toContain("15분 전 관측");
  });

  it("윈도별 남은 비율과 리셋 시각을 낸다", () => {
    const html = renderToStaticMarkup(<VendorDetail usage={full} {...props} />);
    expect(html).toContain("5시간");
    expect(html).toContain("86% 남음");
    expect(html).toContain("50분 후");
    expect(html).toContain("주간");
    expect(html).toContain("75% 남음");
    expect(html).toContain("plus");
    expect(html).toContain("세션 로그");
  });

  it("값이 없으면 상태 설명을 대신 보여준다", () => {
    const html = renderToStaticMarkup(
      <VendorDetail
        usage={vendor({ status: "unauthenticated", detail: "토큰이 만료됐습니다" })}
        {...props}
      />,
    );
    expect(html).toContain("토큰이 만료됐습니다");
  });

  it("Claude는 브리지 미설치 시 설치 버튼을 준다", () => {
    const html = renderToStaticMarkup(
      <VendorDetail
        usage={vendor({ vendor: "claude", label: "Claude Code", status: "no_data" })}
        {...props}
        bridge={{ installed: false, wrapped: null, foreign: null }}
      />,
    );
    expect(html).toContain("브리지 설치");
  });

  it("브리지가 설치돼 있으면 브리지 안내를 내지 않는다", () => {
    const html = renderToStaticMarkup(
      <VendorDetail
        usage={vendor({ vendor: "claude", label: "Claude Code" })}
        {...props}
        bridge={{ installed: true, wrapped: "node omc-hud.mjs", foreign: null }}
      />,
    );
    expect(html).not.toContain("브리지 설치");
    expect(html).not.toContain("해제");
    expect(html).not.toContain("기존 statusline");
  });

  it("stale은 값을 보이되 흐리게 두고 관측 시각과 안내를 함께 낸다", () => {
    const html = renderToStaticMarkup(
      <VendorDetail
        {...props}
        usage={vendor({
          vendor: "claude",
          label: "Claude Code",
          status: "stale",
          five_hour: { used_percent: 40, resets_at: null, window_minutes: 300 },
          updated_at: props.now - 7200,
          detail: "토큰이 만료돼 새 값을 받지 못했습니다 — claude CLI를 한 번 실행하면 갱신됩니다",
        })}
      />,
    );
    expect(html).toContain("60% 남음");
    expect(html).toContain("2시간 전 관측");
    expect(html).toContain("claude CLI를 한 번 실행하면");
    expect(html).not.toContain("로그인 필요");
  });

  it("로그아웃 안내는 조회용 토큰 등록도 함께 알린다", () => {
    const html = renderToStaticMarkup(
      <VendorDetail {...props} usage={vendor({ status: "unauthenticated" })} />,
    );
    expect(html).toContain("조회용 토큰");
  });

  it("Antigravity는 실제 로그인 안내만 표시하고 Claude 조회용 토큰을 권하지 않는다", () => {
    const html = renderToStaticMarkup(
      <VendorDetail {...props} usage={vendor({
        vendor: "agy", label: "Antigravity", status: "unauthenticated",
        detail: "앱 또는 agy에서 다시 로그인하세요",
      })} />,
    );
    expect(html).toContain("앱 또는 agy에서 다시 로그인하세요");
    expect(html).not.toContain("조회용 토큰");
  });

  it("Claude가 아닌 벤더에는 브리지 UI가 없다", () => {
    const html = renderToStaticMarkup(
      <VendorDetail usage={full} {...props} bridge={{ installed: false, wrapped: null, foreign: null }} />,
    );
    expect(html).not.toContain("브리지");
  });
});
