// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { DesignCanvas, ScreenFrame } from "./DesignCanvas";
import { collectScreens } from "./registry";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement;
let root: Root;
const screens = collectScreens({ "./screens/sample.screen.tsx": {
  meta: { title: "샘플 화면", viewport: { width: 960, height: 640 }, states: { 정상: { label: "정상 데이터" }, "빈 상태": { label: "기록 없음" } } },
  default: ({ label }: { label: string }) => <p>{label}</p>,
} });
beforeEach(() => { host = document.createElement("div"); document.body.appendChild(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
describe("DesignCanvas", () => {
  it("renders each named state at the declared viewport and changes viewport without scaling CSS pixels", async () => {
    await act(async () => root.render(<DesignCanvas screens={screens} />));
    const frame = () => host.querySelector("iframe")!;
    expect([frame().width, frame().height]).toEqual(["960", "640"]);
    expect(new URL(frame().src).searchParams.get("state")).toBe("정상");
    await act(async () => {
      [...host.querySelectorAll("button")].find((button) => button.textContent === "빈 상태")!.click();
    });
    expect(new URL(frame().src).searchParams.get("state")).toBe("빈 상태");
    const viewport = host.querySelector<HTMLSelectElement>('[aria-label="뷰포트"]')!;
    await act(async () => { viewport.value = "390x844"; viewport.dispatchEvent(new Event("change", { bubbles: true })); });
    expect([frame().width, frame().height]).toEqual(["390", "844"]);
    expect(host.querySelector('[aria-label="인스펙터"]')?.textContent).toContain("요소를 선택하면");
  });
  it("keeps invalid screens visible in the list and skips rendering them", async () => {
    await act(async () => root.render(<DesignCanvas screens={collectScreens({ "./screens/bad.screen.tsx": {} })} />));
    expect(host.textContent).toContain("meta가 없거나");
    expect(host.querySelector("iframe")).toBeNull();
  });
  it("shows an empty registry", async () => {
    await act(async () => root.render(<DesignCanvas screens={[]} />));
    expect(host.textContent).toContain("등록된 화면이 없습니다");
  });
  it("supplies only the selected state's props to the screen and rejects unknown states", async () => {
    await act(async () => root.render(<ScreenFrame screens={screens} search={`?screen=sample&state=${encodeURIComponent("빈 상태")}`} />));
    expect(host.textContent).toBe("기록 없음");
    await act(async () => root.render(<ScreenFrame screens={screens} search="?screen=sample&state=unknown" />));
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("상태를 찾을 수 없습니다");
  });
});
