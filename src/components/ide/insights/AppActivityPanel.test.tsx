import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { AppActivityPanel } from "./AppActivityPanel";
import type { AppActivity } from "../../../lib/ipc";

const base: AppActivity = {
  open_secs: 36_000,
  focus_secs: 18_000,
  input_secs: 7_200,
  active_days: 4,
  since: 1_787_000_000,
  prev: { open_secs: 18_000, focus_secs: 18_000, input_secs: 3_600 },
};

describe("AppActivityPanel", () => {
  it("집계가 오기 전에는 자리를 지킨다", () => {
    expect(renderToStaticMarkup(<AppActivityPanel data={null} />)).toContain("집계 중…");
  });

  it("기록이 한 번도 없으면 0시간 대신 빈 상태를 말한다", () => {
    const html = renderToStaticMarkup(<AppActivityPanel data={{ ...base, since: null }} />);
    expect(html).toContain("아직 기록이 없습니다");
    expect(html).not.toContain("켜져 있던 시간");
  });

  it("조작 시간을 앞세우고 켜진 시간·포커스 시간·하루 평균 조작을 보여준다", () => {
    const html = renderToStaticMarkup(<AppActivityPanel data={base} />);
    expect(html.indexOf("조작 시간")).toBeLessThan(html.indexOf("켜져 있던 시간"));
    expect(html).toContain("2.0시간");
    expect(html).toContain("10시간");
    expect(html).toContain("5.0시간");
    // 7200초 / 4일 = 1800초 → 30분
    expect(html).toContain("30분");
    // 조작·켜진 시간 모두 직전 구간보다 100% 늘었다.
    expect(html).toContain("100%");
  });

  it("켜진 날이 없으면 평균을 —로 쓴다", () => {
    const html = renderToStaticMarkup(
      <AppActivityPanel data={{ ...base, open_secs: 0, focus_secs: 0, input_secs: 0, active_days: 0, prev: null }} />,
    );
    expect(html).toContain("—");
  });
});
