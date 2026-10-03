// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>(),
  handlers: [] as ((ev: { payload: unknown }) => void)[],
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (_name: string, cb: (ev: { payload: unknown }) => void) => {
    mocks.handlers.push(cb);
    return Promise.resolve(() => undefined);
  },
}));

import { RunDetail } from "./RunDetail";
import { byLabel, byText, click, confirmYes, detail, flush, run, setValue, ticket } from "./testkit";
import type { PipelineDetail } from "../../../lib/pipeline";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const VENDORS = [
  { vendor: "claude", available: true },
  { vendor: "codex", available: true },
  { vendor: "agy", available: false },
];
const SYNTH = JSON.stringify({
  decision: "fix",
  summary: "고칠 것 있음",
  findings: [
    { severity: "blocking", summary: "치명 결함", sources: ["codex"] },
    { severity: "advisory", summary: "사소한 제안" },
  ],
});
const record = { meta: {}, result: { items: [{ vendor: "codex", ok: true, text: '{"findings":[{"severity":"blocking","summary":"원 지적"}]}' }], synthesis: SYNTH }, detail: {} };

let current: PipelineDetail;
let container: HTMLDivElement;
let root: Root;
const onOpenTask = vi.fn();
const onApprove = vi.fn(async () => undefined);

const calls = (cmd: string) => mocks.invoke.mock.calls.filter(([c]) => c === cmd);

async function mount() {
  await act(async () => root.render(<RunDetail runId={1} onBack={() => undefined} onOpenTask={onOpenTask} onApproveIntegration={onApprove} />));
  await flush();
}

beforeEach(() => {
  mocks.handlers.length = 0;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  mocks.invoke.mockImplementation(async (cmd) => {
    if (cmd === "pipeline_get") return current;
    if (cmd === "pipeline_vendors") return VENDORS;
    if (cmd === "review_get") return record;
    if (cmd === "pipeline_update_tickets") return { ...current.run, spec_revision: 4 };
    return current.run;
  });
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.restoreAllMocks();
  mocks.invoke.mockReset();
  onOpenTask.mockReset();
  onApprove.mockClear();
});

describe("RunDetail", () => {
  it("상태 배지와 티켓 표를 한국어로 그린다", async () => {
    current = detail(run({ state: "executing" }), [ticket("T1", { state: "reviewing", task_id: 7, attempt: 2 })]);
    await mount();
    expect(container.textContent).toContain("티켓 실행 중");
    expect(container.querySelector("th")?.textContent).toBe("키");
    expect(container.textContent).toContain("리뷰 중");
    await click(byLabel(container, "T1 세션 열기"));
    expect(onOpenTask).toHaveBeenCalledWith(7);
  });

  it("pipeline://changed 이벤트로 다시 불러온다", async () => {
    current = detail(run(), [ticket("T1")]);
    await mount();
    const before = calls("pipeline_get").length;
    expect(before).toBeGreaterThanOrEqual(1);
    await act(async () => mocks.handlers.forEach((h) => h({ payload: 1 })));
    await flush();
    expect(calls("pipeline_get").length).toBe(before + 1);
    await act(async () => mocks.handlers.forEach((h) => h({ payload: 99 })));
    await flush();
    expect(calls("pipeline_get").length).toBe(before + 1);
  });

  it("일시정지 해제와 취소(확인 후)를 호출한다", async () => {
    current = detail(run({ state: "paused" }), [ticket("T1")]);
    await mount();
    await click(byText(container, "button", "일시정지 해제"));
    expect(calls("pipeline_resume")[0][1]).toEqual({ runId: 1 });
    const confirm = confirmYes();
    await click(byText(container, "button", "취소"));
    expect(confirm).toHaveBeenCalled();
    expect(calls("pipeline_cancel")[0][1]).toEqual({ runId: 1 });
  });

  describe("G1", () => {
    beforeEach(() => {
      current = detail(run({ state: "awaiting_plan_approval", plan_review_id: 5 }), [ticket("T1"), ticket("T2")]);
    });

    it("편집하면 update 후 새 revision으로 승인한다", async () => {
      await mount();
      await setValue(byLabel(container, "T1 제목"), "바뀐 제목");
      await click(byText(container, "button", "승인"));
      const [, upd] = calls("pipeline_update_tickets")[0];
      expect(upd).toMatchObject({ runId: 1, expectedRevision: 3 });
      expect((upd!.tickets as { title: string }[])[0].title).toBe("바뀐 제목");
      expect(calls("pipeline_approve_plan")[0][1]).toEqual({ runId: 1, expectedRevision: 4 });
    });

    it("편집이 없으면 update 없이 현재 revision으로 승인한다", async () => {
      await mount();
      await click(byText(container, "button", "승인"));
      expect(calls("pipeline_update_tickets")).toHaveLength(0);
      expect(calls("pipeline_approve_plan")[0][1]).toEqual({ runId: 1, expectedRevision: 3 });
    });

    it("검증 오류가 있으면 승인할 수 없다", async () => {
      await mount();
      await setValue(byLabel(container, "T1 허용 경로"), "/etc");
      expect(container.textContent).toContain("절대 경로");
      expect((byText(container, "button", "승인") as HTMLButtonElement).disabled).toBe(true);
    });

    it("개정 충돌이면 오류를 보이고 다시 불러온다", async () => {
      await mount();
      mocks.invoke.mockImplementation(async (cmd) => {
        if (cmd === "pipeline_approve_plan") throw "revision mismatch";
        if (cmd === "pipeline_get") return current;
        if (cmd === "pipeline_vendors") return VENDORS;
        return record;
      });
      const before = calls("pipeline_get").length;
      await click(byText(container, "button", "승인"));
      await flush();
      expect(container.textContent).toContain("다시 불러옵니다");
      expect(calls("pipeline_get").length).toBe(before + 1);
    });

    it("반려는 코멘트가 있어야 한다", async () => {
      await mount();
      const reject = () => byText(container, "button", "반려") as HTMLButtonElement;
      expect(reject().disabled).toBe(true);
      const comment = [...container.querySelectorAll("textarea")].pop()!;
      await setValue(comment, "범위가 너무 넓음");
      expect(reject().disabled).toBe(false);
      await click(reject());
      expect(calls("pipeline_reject_plan")[0][1]).toEqual({ runId: 1, comment: "범위가 너무 넓음" });
    });

    it("계획 리뷰 종합을 blocking/advisory로 나눠 보인다", async () => {
      await mount();
      expect(container.querySelector('[aria-label="blocking 지적"]')?.textContent).toContain("치명 결함");
      expect(container.querySelector('[aria-label="advisory 지적"]')?.textContent).toContain("사소한 제안");
      expect(container.textContent).toContain("GPT(codex)");
    });
  });

  it("에스컬레이션: 선택한 벤더로 재시도하고 티켓을 제외한다", async () => {
    current = detail(run({ state: "paused", paused_reason: "T1 실패" }), [ticket("T1", { state: "escalated", last_error: "빌드 실패" })]);
    await mount();
    expect(container.textContent).toContain("빌드 실패");
    await setValue(byLabel(container, "T1 재시도 벤더"), "claude");
    await click(byText(container, "button", "벤더 변경 후 재시도"));
    expect(calls("pipeline_retry_ticket")[0][1]).toEqual({ runId: 1, ticketId: 11, vendor: "claude" });
    await click(byText(container, "button", "티켓 제외"));
    expect(calls("pipeline_skip_ticket")[0][1]).toEqual({ runId: 1, ticketId: 11 });
  });

  describe("G2", () => {
    beforeEach(() => {
      current = detail(run({ state: "awaiting_merge_approval", final_review_id: 9, integration_task_id: 42, integration_branch: "pipe/1" }), [ticket("T1", { state: "integrated" })]);
    });

    it("남은 blocking을 강조하고 기존 승인 경로를 호출한다", async () => {
      await mount();
      await flush();
      expect(container.querySelector('[role="status"]')?.textContent).toContain("blocking 지적 1건");
      expect(container.textContent).toContain("main에 머지");
      confirmYes();
      await click(byText(container, "button", "승인하고 머지"));
      expect(onApprove).toHaveBeenCalledWith(42);
    });

    it("승인 실패를 G2 화면에 표시한다", async () => {
      onApprove.mockRejectedValueOnce("충돌: a.txt");
      await mount();
      confirmYes();
      await click(byText(container, "button", "승인하고 머지"));
      await flush();
      expect(container.textContent).toContain("충돌: a.txt");
    });

    it("자동 수정 실패로 남은 blocking 안내(last_error)를 보여 준다", async () => {
      current = detail(
        run({ state: "awaiting_merge_approval", final_review_id: 9, integration_task_id: 42, last_error: "최종 리뷰 자동 수정에 실패해 되돌렸습니다: 검증 실패\n남은 blocking 지적:\n- 치명 결함" }),
        [ticket("T1", { state: "integrated" })],
      );
      await mount();
      const note = container.querySelector('[role="note"]');
      expect(note?.textContent).toContain("자동 수정에 실패해 되돌렸습니다");
      expect(note?.textContent).toContain("치명 결함");
    });

    it("통합 작업 열기는 작업 화면으로 이동한다", async () => {
      await mount();
      await click(byText(container, "button", "통합 작업 열기 (diff)"));
      expect(onOpenTask).toHaveBeenCalledWith(42);
    });
  });
});
