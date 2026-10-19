// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => undefined) }));
vi.mock("../icons", () => ({ Icon: () => null }));

import { PipelineView } from "./PipelineView";
import { usePipelineRuns } from "../../../lib/usePipelineRuns";
import { byText, click, flush, run } from "./testkit";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** App처럼 목록 훅을 한 번 부르고 화면에 내려준다. */
function Harness(props: Omit<Parameters<typeof PipelineView>[0], "list">) {
  return <PipelineView list={usePipelineRuns()} {...props} />;
}

let container: HTMLDivElement;
let root: Root;
const onSelectRun = vi.fn();

const item = (id: number, state: string, extra: Record<string, unknown> = {}) => ({
  ...run({ id, state, goal: `목표${id}`, repo: "/work/app", updated_at: id }),
  tickets_total: 4,
  tickets_integrated: 1,
  tickets_escalated: 0,
  active_step_kind: null,
  ...extra,
});

async function mount(runs: unknown[], props: Record<string, unknown> = {}) {
  mocks.invoke.mockImplementation(async (cmd) => {
    if (cmd === "pipeline_list") return runs;
    if (cmd === "pipeline_vendors") return [];
    if (cmd === "git_branches_path") return { current: "main", branches: ["main"] };
    return null;
  });
  await act(async () =>
    root.render(
      <Harness runId={null} onSelectRun={onSelectRun} projects={["/work/app"]} onOpenTask={() => undefined} onApproveIntegration={() => undefined} {...props} />,
    ),
  );
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
  onSelectRun.mockReset();
});

describe("PipelineView", () => {
  it("저장소 필터 없이 전체 실행을 조회한다", async () => {
    await mount([item(1, "executing")]);
    expect(mocks.invoke).toHaveBeenCalledWith("pipeline_list", { repo: undefined });
  });

  it("조치 필요 / 진행 중 / 종료로 나누고 종료는 접어 둔다", async () => {
    await mount([item(1, "awaiting_plan_approval"), item(2, "executing", { active_step_kind: "ticket_review", tickets_escalated: 0 }), item(3, "done")]);
    const sections = [...container.querySelectorAll("section")].map((s) => s.getAttribute("aria-label"));
    expect(sections).toEqual(["조치 필요 1", "진행 중 1"]);
    const details = container.querySelector("details")!;
    expect(details.open).toBe(false);
    expect(details.textContent).toContain("목표3");
    expect(container.textContent).toContain("통합 1/4");
    expect(container.textContent).toContain("티켓 리뷰 진행 중");
    expect(container.textContent).toContain("app");
  });

  it("실행이 있으면 새 실행 폼을 버튼 뒤에 접어 두고, 없으면 펼친다", async () => {
    await mount([item(1, "executing")]);
    expect(container.querySelector("textarea")).toBeNull();
    await click(byText(container, "button", "새 실행"));
    expect(container.querySelector("textarea")).toBeTruthy();
    await act(async () => root.unmount());
    root = createRoot(container);
    await mount([]);
    expect(container.querySelector("textarea")).toBeTruthy();
  });

  it("행을 누르면 onSelectRun을 부른다", async () => {
    await mount([item(7, "paused")]);
    await click(byText(container, "button", "#7 목표7"));
    expect(onSelectRun).toHaveBeenCalledWith(7);
  });

  it("runId가 있으면 상세를 보인다", async () => {
    mocks.invoke.mockImplementation(async (cmd) => (cmd === "pipeline_get" ? { run: run({ id: 9 }), tickets: [], steps: [], reviews: [] } : cmd === "pipeline_vendors" ? [] : []));
    await act(async () =>
      root.render(<Harness runId={9} onSelectRun={onSelectRun} projects={[]} onOpenTask={() => undefined} onApproveIntegration={() => undefined} />),
    );
    await flush();
    expect(container.textContent).toContain("#9");
    expect(byText(container, "button", "목록으로")).toBeTruthy();
  });
});
