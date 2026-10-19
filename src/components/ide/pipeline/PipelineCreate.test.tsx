// @vitest-environment jsdom

import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../icons", () => ({ Icon: () => null }));

import { PipelineCreate } from "./PipelineCreate";
import { byText, click, flush, run, setValue } from "./testkit";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
const onStarted = vi.fn();

async function mount(vendors: { vendor: string; available: boolean; reason?: string | null }[], props: Partial<ComponentProps<typeof PipelineCreate>> = {}) {
  mocks.invoke.mockImplementation(async (cmd) => {
    if (cmd === "pipeline_vendors") return vendors;
    if (cmd === "git_branches_path") return { current: "main", branches: ["main", "dev"] };
    if (cmd === "pipeline_start") return run({ id: 5 });
    return null;
  });
  await act(async () => root.render(<PipelineCreate projects={["/repo", "/other/app"]} onStarted={onStarted} {...props} />));
  await flush();
  await flush();
}

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  mocks.invoke.mockReset();
  onStarted.mockReset();
});

describe("PipelineCreate", () => {
  it("가용 벤더가 2개 미만이면 시작 버튼을 막고 이유를 보인다", async () => {
    await mount([{ vendor: "claude", available: true }, { vendor: "codex", available: false }, { vendor: "agy", available: false }]);
    await setValue(container.querySelector("textarea")!, "목표");
    expect((byText(container, "button", "실행 시작") as HTMLButtonElement).disabled).toBe(true);
    expect(container.textContent).toContain("2개 이상 필요");
  });

  it("벤더가 충분하고 목표가 있으면 pipeline_start를 호출한다", async () => {
    await mount([{ vendor: "claude", available: true }, { vendor: "codex", available: true }, { vendor: "agy", available: false }]);
    const start = () => byText(container, "button", "실행 시작") as HTMLButtonElement;
    expect(start().disabled).toBe(true);
    await setValue(container.querySelector("textarea")!, "  로그인 추가 ");
    expect(start().disabled).toBe(false);
    await click(start());
    expect(mocks.invoke).toHaveBeenCalledWith("pipeline_start", { repo: "/repo", baseBranch: "main", goal: "로그인 추가" });
    expect(onStarted).toHaveBeenCalledWith(expect.objectContaining({ id: 5 }));
  });

  it("슬래시 명령으로 시작하는 목표에는 경고를 보이되 시작은 막지 않는다", async () => {
    await mount([{ vendor: "claude", available: true }, { vendor: "codex", available: true }]);
    await setValue(container.querySelector("textarea")!, "/graph-engineering 브레인스토밍 해보자");
    expect(container.querySelector('[role="status"]')?.textContent).toContain("슬래시 명령");
    expect((byText(container, "button", "실행 시작") as HTMLButtonElement).disabled).toBe(false);
    await setValue(container.querySelector("textarea")!, "로그인 추가");
    expect(container.querySelector('[role="status"]')).toBeNull();
  });

  it("백엔드 오류를 그대로 보인다", async () => {
    await mount([{ vendor: "claude", available: true }, { vendor: "codex", available: true }]);
    mocks.invoke.mockImplementation(async (cmd) => {
      if (cmd === "pipeline_start") throw "이미 활성 실행이 있습니다";
      return cmd === "pipeline_vendors" ? [] : { current: "main", branches: [] };
    });
    await setValue(container.querySelector("textarea")!, "목표");
    await click(byText(container, "button", "실행 시작"));
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("이미 활성 실행");
  });

  it("기본 선택은 defaultRepo, 없으면 최근 실행 저장소, 없으면 첫 프로젝트다", async () => {
    await mount([], { defaultRepo: "/other/app" });
    expect((container.querySelector('select[aria-label="프로젝트"]') as HTMLSelectElement).value).toBe("/other/app");
    await act(async () => root.unmount());
    root = createRoot(container);
    await mount([], { recentRepo: "/other/app" });
    expect((container.querySelector('select[aria-label="프로젝트"]') as HTMLSelectElement).value).toBe("/other/app");
    await act(async () => root.unmount());
    root = createRoot(container);
    await mount([]);
    expect((container.querySelector('select[aria-label="프로젝트"]') as HTMLSelectElement).value).toBe("/repo");
  });

  it("프로젝트를 바꾸면 그 저장소로 시작한다", async () => {
    await mount([{ vendor: "claude", available: true }, { vendor: "codex", available: true }]);
    await setValue(container.querySelector('select[aria-label="프로젝트"]')!, "/other/app");
    await setValue(container.querySelector("textarea")!, "목표");
    await click(byText(container, "button", "실행 시작"));
    expect(mocks.invoke).toHaveBeenCalledWith("pipeline_start", expect.objectContaining({ repo: "/other/app" }));
  });

  it("프로젝트가 없으면 추가하라고 안내하고 시작할 수 없다", async () => {
    await mount([{ vendor: "claude", available: true }, { vendor: "codex", available: true }], { projects: [] });
    expect(container.textContent).toContain("프로젝트를 먼저 추가하세요");
    await setValue(container.querySelector("textarea")!, "목표");
    expect((byText(container, "button", "실행 시작") as HTMLButtonElement).disabled).toBe(true);
  });

  it("사용 불가 벤더의 사유를 보인다", async () => {
    await mount([{ vendor: "claude", available: true }, { vendor: "codex", available: false, reason: "PATH에 없음" }]);
    expect(container.querySelector('[aria-label="가용 벤더"]')?.textContent).toContain("사용 불가(PATH에 없음)");
  });
});
