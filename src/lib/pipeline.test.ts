import { describe, expect, it } from "vitest";
import {
  RUN_STATE_LABEL,
  attentionCount,
  baselineFailure,
  groupRuns,
  integratedPercent,
  needsAttention,
  nextActionHint,
  parseBaseline,
  relativeTime,
  repoBasename,
  stepKindLabel,
  TICKET_STATE_LABEL,
  boundOutput,
  canAnswerClarification,
  canApprovePlan,
  canCancel,
  canStartPipeline,
  formatDuration,
  groupFindings,
  hasEscalation,
  hasTicketErrors,
  isSlashCommandGoal,
  parseClarification,
  parseSpec,
  parseSynthesis,
  synthesisOf,
  vendorFindingsOf,
  stepDurationSecs,
  elapsedSecs,
  runningStep,
  ticketToDraft,
  validateTickets,
  type TicketDraft,
  type TicketRow,
} from "./pipeline";

const draft = (key: string, extra: Partial<TicketDraft> = {}): TicketDraft => ({
  key,
  title: `${key} 제목`,
  body: "",
  vendor: "claude",
  vendor_reason: "",
  acceptance_commands: ["npm test"],
  allowed_paths: ["src/"],
  deps: [],
  covers: [],
  ...extra,
});

describe("상태 라벨", () => {
  it("모든 실행·티켓 상태에 한국어 라벨이 있다", () => {
    expect(Object.keys(RUN_STATE_LABEL).sort()).toEqual(
      ["awaiting_merge_approval", "awaiting_plan_approval", "cancelled", "done", "drafting", "executing", "failed", "final_review", "paused", "plan_review"],
    );
    expect(Object.keys(TICKET_STATE_LABEL)).toHaveLength(10);
    for (const v of [...Object.values(RUN_STATE_LABEL), ...Object.values(TICKET_STATE_LABEL)]) {
      expect(v.label).toMatch(/[가-힣]/);
    }
  });
});

describe("판정 헬퍼", () => {
  it("canApprovePlan은 계획 승인 대기에서만 참이다", () => {
    expect(canApprovePlan({ state: "awaiting_plan_approval" })).toBe(true);
    expect(canApprovePlan({ state: "executing" })).toBe(false);
    expect(canApprovePlan(null)).toBe(false);
  });
  it("hasEscalation / canCancel", () => {
    expect(hasEscalation([{ state: "running" }, { state: "escalated" }])).toBe(true);
    expect(hasEscalation([{ state: "running" }])).toBe(false);
    expect(canCancel({ state: "paused" })).toBe(true);
    expect(canCancel({ state: "done" })).toBe(false);
  });
  it("canAnswerClarification은 Drafting에서 질문을 남기고 멈춘 실행에서만 참이다", () => {
    expect(canAnswerClarification({ state: "paused", paused_from: "drafting", clarification_json: "{}" })).toBe(true);
    expect(canAnswerClarification({ state: "paused", paused_from: "drafting", clarification_json: null })).toBe(false);
    expect(canAnswerClarification({ state: "paused", paused_from: "executing", clarification_json: "{}" })).toBe(false);
    expect(canAnswerClarification({ state: "drafting", paused_from: null, clarification_json: "{}" })).toBe(false);
  });
  it("슬래시 명령으로 시작하는 목표를 알아본다", () => {
    expect(isSlashCommandGoal("/graph-engineering 브레인스토밍")).toBe(true);
    expect(isSlashCommandGoal("  /x")).toBe(true);
    expect(isSlashCommandGoal("src/lib를 고친다")).toBe(false);
    expect(isSlashCommandGoal("")).toBe(false);
  });
  it("벤더가 2개 미만이면 시작할 수 없다", () => {
    expect(canStartPipeline([{ vendor: "claude", available: true }, { vendor: "codex", available: false }])).toBe(false);
    expect(canStartPipeline([{ vendor: "claude", available: true }, { vendor: "codex", available: true }])).toBe(true);
  });
  it("groupFindings는 blocking과 advisory를 나눈다", () => {
    const g = groupFindings({
      findings: [
        { severity: "advisory" as const, summary: "a" },
        { severity: "blocking" as const, summary: "b" },
      ],
    });
    expect(g.blocking.map((f) => f.summary)).toEqual(["b"]);
    expect(g.advisory.map((f) => f.summary)).toEqual(["a"]);
    expect(groupFindings(null)).toEqual({ blocking: [], advisory: [] });
  });
});

describe("티켓 편집 검증", () => {
  it("유효한 티켓은 오류가 없다", () => {
    expect(hasTicketErrors(validateTickets([draft("T1"), draft("T2", { deps: ["T1"] })]))).toBe(false);
  });
  it("빈 제목, 경로 없음, 수용 명령 없음", () => {
    const e = validateTickets([draft("T1", { title: " ", allowed_paths: [], acceptance_commands: [] })]).T1;
    expect(e.title).toBeDefined();
    expect(e.allowed_paths).toBeDefined();
    expect(e.acceptance_commands).toBeDefined();
  });
  it("절대 경로와 .. 경로를 거부한다", () => {
    expect(validateTickets([draft("T1", { allowed_paths: ["/etc"] })]).T1.allowed_paths).toBeDefined();
    expect(validateTickets([draft("T1", { allowed_paths: ["src/../x"] })]).T1.allowed_paths).toBeDefined();
    expect(validateTickets([draft("T1", { allowed_paths: ["src/a..b"] })]).T1).toBeUndefined();
  });
  it("자기 선행과 없는 선행을 거부한다", () => {
    expect(validateTickets([draft("T1", { deps: ["T1"] })]).T1.deps).toBeDefined();
    expect(validateTickets([draft("T1", { deps: ["T9"] })]).T1.deps).toBeDefined();
  });
});

describe("파싱·서식", () => {
  it("synthesisOf와 vendorFindingsOf는 구조화 필드를 우선하고 없으면 기존 파싱으로 되돌아간다", () => {
    const structured = { decision: "fix", findings: [{ severity: "blocking", summary: "x" }], summary: "s" };
    expect(synthesisOf({ synthesis: "판정: pass", synthesis_structured: structured })?.decision).toBe("fix");
    expect(synthesisOf({ synthesis: '{"decision":"pass","findings":[],"summary":"ok"}' })?.decision).toBe("pass");
    expect(synthesisOf({ synthesis: "text" })).toBeNull();
    const vr = [{ vendor: "codex", ok: true, findings: [{ severity: "advisory", summary: "a" }] }];
    expect(vendorFindingsOf({ vendor_reviews: vr }, "codex", "- [advisory] a")).toHaveLength(1);
    expect(vendorFindingsOf({ vendor_reviews: vr }, "agy", "not json")).toBeNull();
    expect(vendorFindingsOf({}, "codex", '[{"severity":"blocking","summary":"b"}]')).toHaveLength(1);
  });

  it("parseClarification은 판정을 정규화하고 빈 질문을 버린다", () => {
    expect(parseClarification('{"decision":"clarify","questions":["a"," ","b"],"reason":"r"}')).toEqual({ decision: "clarify", questions: ["a", "b"], reason: "r" });
    expect(parseClarification('{"decision":"decline"}')).toEqual({ decision: "decline", questions: [], reason: "" });
    expect(parseClarification('{"decision":"weird","questions":"x"}')?.decision).toBe("clarify");
    expect(parseClarification(null)).toBeNull();
    expect(parseClarification("{broken")).toBeNull();
  });
  it("parseSpec과 parseSynthesis는 깨진 JSON에 null을 돌려준다", () => {
    expect(parseSpec("not json")).toBeNull();
    expect(parseSpec('{"goal":"g"}')).toEqual({ goal: "g", requirements: [], out_of_scope: [] });
    expect(parseSynthesis("text")).toBeNull();
    expect(parseSynthesis('{"decision":"pass","findings":[],"summary":"ok"}')?.decision).toBe("pass");
  });
  it("ticketToDraft는 JSON 열을 목록으로 푼다", () => {
    const row = {
      key: "T1", title: "t", body: "b", vendor: "codex", acceptance_json: '["x"]', allowed_paths_json: '["src/"]', deps_json: '["T0"]', covers_json: "[]",
    } as TicketRow;
    expect(ticketToDraft(row)).toMatchObject({ acceptance_commands: ["x"], allowed_paths: ["src/"], deps: ["T0"] });
  });
  it("소요 시간과 출력 제한", () => {
    expect(stepDurationSecs({ started_at: 10, finished_at: 75 })).toBe(65);
    expect(stepDurationSecs({ started_at: 10, finished_at: null })).toBeNull();
    expect(formatDuration(65)).toBe("1분 5초");
    expect(boundOutput("x".repeat(10), 4)).toContain("6자 생략");
  });
  it("진행 중 단계와 경과 시간", () => {
    const step = (id: number, status: string, started_at: number) =>
      ({ id, run_id: 1, ticket_id: null, kind: "split", vendor: null, step_key: "", status, prompt: null, output: null, started_at, finished_at: null });
    expect(runningStep([])).toBeNull();
    expect(runningStep([step(1, "succeeded", 5), step(2, "running", 10), step(3, "running", 20)])?.id).toBe(3);
    expect(elapsedSecs({ started_at: 100 }, 165_000)).toBe(65);
    expect(elapsedSecs({ started_at: 1_700_000_000_000 }, 1_700_000_065_000)).toBe(65);
    expect(elapsedSecs({ started_at: 200 }, 100_000)).toBe(0);
  });
});

describe("파이프라인 목록 헬퍼", () => {
  const r = (id: number, state: string, extra: Record<string, unknown> = {}) => ({ id, state, updated_at: id, tickets_escalated: 0, ...extra });

  it("needsAttention은 승인 대기·일시정지·에스컬레이션을 잡고 종결·자동 진행은 거른다", () => {
    expect(needsAttention(r(1, "awaiting_plan_approval"))).toBe(true);
    expect(needsAttention(r(1, "awaiting_merge_approval"))).toBe(true);
    expect(needsAttention(r(1, "paused"))).toBe(true);
    expect(needsAttention(r(1, "executing", { tickets_escalated: 1 }))).toBe(true);
    expect(needsAttention(r(1, "executing"))).toBe(false);
    expect(needsAttention(r(1, "done", { tickets_escalated: 1 }))).toBe(false);
  });

  it("attentionCount와 groupRuns", () => {
    const runs = [r(1, "paused"), r(2, "executing"), r(3, "done"), r(4, "drafting"), r(5, "awaiting_merge_approval")];
    expect(attentionCount(runs)).toBe(2);
    const g = groupRuns(runs);
    expect(g.attention.map((x) => x.id)).toEqual([5, 1]);
    expect(g.active.map((x) => x.id)).toEqual([4, 2]);
    expect(g.finished.map((x) => x.id)).toEqual([3]);
  });

  it("repoBasename·integratedPercent·relativeTime·nextActionHint", () => {
    expect(repoBasename("/a/b/app/")).toBe("app");
    expect(integratedPercent(0, 0)).toBe(0);
    expect(integratedPercent(4, 1)).toBe(25);
    const now = 1_000_000_000_000;
    expect(relativeTime(now / 1000 - 30, now)).toBe("방금");
    expect(relativeTime(now / 1000 - 600, now)).toBe("10분 전");
    expect(relativeTime(now / 1000 - 7200, now)).toBe("2시간 전");
    expect(nextActionHint({ state: "awaiting_plan_approval", paused_from: null, clarification_json: null }, 0)).toBe("계획 승인 필요");
    expect(nextActionHint({ state: "executing", paused_from: null, clarification_json: null }, 2)).toContain("2건 에스컬레이션");
    expect(nextActionHint({ state: "executing", paused_from: null, clarification_json: null }, 0)).toContain("조치 불필요");
  });

  it("baseline 출력을 파싱하고 실패 명령이 있을 때만 경고 대상으로 본다", () => {
    expect(stepKindLabel("baseline")).toBe("기준선 검증");
    expect(parseBaseline("깨진 json")).toBeNull();
    expect(parseBaseline('{"failed_commands":["npm test"],"summary":"x"}')).toEqual({ failed_commands: ["npm test"], summary: "x" });
    const step = (output: string) => ({ id: 1, run_id: 1, ticket_id: null, kind: "baseline", vendor: null, step_key: "b", status: "succeeded", prompt: null, output, started_at: 1, finished_at: 2 });
    expect(baselineFailure([step('{"failed_commands":[],"summary":""}')])).toBeNull();
    expect(baselineFailure([step('{"failed_commands":["npm test"],"summary":"s"}')])?.failed_commands).toEqual(["npm test"]);
  });
});
