// @vitest-environment jsdom

import { act } from "react";
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

async function mount(vendors: { vendor: string; available: boolean }[]) {
  mocks.invoke.mockImplementation(async (cmd) => {
    if (cmd === "pipeline_vendors") return vendors;
    if (cmd === "git_branches_path") return { current: "main", branches: ["main", "dev"] };
    if (cmd === "pipeline_start") return run({ id: 5 });
    return null;
  });
  await act(async () => root.render(<PipelineCreate repo="/repo" onStarted={onStarted} />));
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
});
