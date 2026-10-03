import { describe, expect, it } from "vitest";
import {
  RUN_STATE_LABEL,
  TICKET_STATE_LABEL,
  boundOutput,
  canApprovePlan,
  canCancel,
  canStartPipeline,
  formatDuration,
  groupFindings,
  hasEscalation,
  hasTicketErrors,
  parseSpec,
  parseSynthesis,
  synthesisOf,
  vendorFindingsOf,
  stepDurationSecs,
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
});
