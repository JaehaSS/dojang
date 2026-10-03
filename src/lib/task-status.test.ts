import { describe, expect, it } from "vitest";
import {
  blockedVendor,
  isAuthBlocked,
  lastRunLabel,
  needsUser,
  statusTone,
  taskDotColor,
  taskStatusLabel,
  taskStatusLabelShort,
  taskTextClass,
} from "./task-status";

const awaiting = (kind: string | null = null) => ({ state: "AwaitingReview", awaiting_kind: kind });

describe("statusTone", () => {
  it("검토 대기를 대기 성격에 따라 가른다", () => {
    expect(statusTone("AwaitingReview", null)).toBe("awaiting");
    expect(statusTone("AwaitingReview", "question")).toBe("question");
  });

  it("검토 대기가 아닌 상태에서는 awaiting_kind를 무시한다", () => {
    // 전이 누락으로 주석이 남아도 실행 중 작업이 답변 대기로 보이면 안 된다.
    expect(statusTone("Running", "question")).toBe("running");
    expect(statusTone("Done", "question")).toBe("done");
  });

  it("알 수 없는 상태는 muted로 떨어뜨린다", () => {
    expect(statusTone("Unknown")).toBe("muted");
  });
});

describe("taskStatusLabel", () => {
  it("두 종류의 대기를 다른 문구로 적는다", () => {
    expect(taskStatusLabel(awaiting())).toBe("검토 대기");
    expect(taskStatusLabel(awaiting("question"))).toBe("답변 대기");
  });

  it("알 수 없는 상태는 원문을 그대로 보여준다", () => {
    expect(taskStatusLabel({ state: "Weird", awaiting_kind: null })).toBe("Weird");
  });
});

describe("taskStatusLabelShort", () => {
  it("좁은 목록에서는 대기 문구를 떼되 구분은 유지한다", () => {
    expect(taskStatusLabelShort(awaiting())).toBe("검토");
    expect(taskStatusLabelShort(awaiting("question"))).toBe("답변");
  });

  it("대기가 아닌 상태는 기존 라벨을 그대로 쓴다", () => {
    expect(taskStatusLabelShort({ state: "Running", awaiting_kind: null })).toBe("실행 중…");
    expect(taskStatusLabelShort({ state: "Failed", awaiting_kind: null })).toBe("실패");
  });

  it("문구를 비우지 않는다 — 색 단독 인코딩 금지(DESIGN.md Do #2)", () => {
    for (const state of ["AwaitingReview", "Running", "Done", "Failed", "Queued"]) {
      expect(taskStatusLabelShort({ state, awaiting_kind: null })).not.toBe("");
    }
  });
});

describe("색 매핑", () => {
  it("답변 대기는 검토 대기와 다른 토큰을 쓴다", () => {
    expect(taskDotColor(awaiting("question"))).toBe("var(--c-question)");
    expect(taskDotColor(awaiting())).toBe("var(--c-awaiting)");
    expect(taskTextClass(awaiting("question"))).toBe("text-status-question");
    expect(taskTextClass(awaiting())).toBe("text-status-awaiting");
  });
});

describe("lastRunLabel", () => {
  it("실행이 끝난 상태의 updated_at은 멈춘 시각으로 읽는다", () => {
    expect(lastRunLabel({ state: "AwaitingReview" })).toBe("마지막 종료");
    expect(lastRunLabel({ state: "Done" })).toBe("마지막 종료");
    expect(lastRunLabel({ state: "Failed" })).toBe("마지막 종료");
  });

  it("진행 중인 상태의 updated_at은 이번 실행이 시작된 시각이다", () => {
    expect(lastRunLabel({ state: "Running" })).toBe("실행 시작");
    expect(lastRunLabel({ state: "Starting" })).toBe("실행 시작");
    expect(lastRunLabel({ state: "Finalizing" })).toBe("실행 시작");
  });

  it("한 번도 실행되지 않았으면 시각을 지어내지 않는다", () => {
    expect(lastRunLabel({ state: "Created" })).toBeNull();
    expect(lastRunLabel({ state: "Queued" })).toBeNull();
    expect(lastRunLabel({ state: "PendingApproval" })).toBeNull();
  });
});

describe("needsUser", () => {
  it("두 종류의 검토 대기와 실행 승인 대기를 모두 센다", () => {
    expect(needsUser(awaiting())).toBe(true);
    expect(needsUser(awaiting("question"))).toBe(true);
    expect(needsUser({ state: "PendingApproval", awaiting_kind: null })).toBe(true);
    expect(needsUser({ state: "Running", awaiting_kind: null })).toBe(false);
  });
});

describe("인증 차단 큐 작업", () => {
  const blocked = { state: "Queued", awaiting_kind: null, blocked_reason: "auth:codex" };

  it("실패가 아니라 사용자 조치 대기로 읽는다", () => {
    // 빨강으로 칠하면 이미 죽은 작업으로 보여, 로그인하면 살아난다는 걸 알 수 없다.
    expect(statusTone(blocked.state, blocked.awaiting_kind, blocked.blocked_reason)).toBe(
      "awaiting",
    );
    expect(taskDotColor(blocked)).not.toBe(taskDotColor({ state: "Failed", awaiting_kind: null }));
  });

  it("색 말고 라벨로도 드러난다", () => {
    expect(taskStatusLabel(blocked)).toBe("로그인 필요");
    expect(taskStatusLabelShort(blocked)).toBe("로그인");
  });

  it("배지에 잡힌다 — 큐에서 조용히 잊히면 안 된다", () => {
    expect(needsUser(blocked)).toBe(true);
    expect(needsUser({ state: "Queued", awaiting_kind: null, blocked_reason: null })).toBe(false);
  });

  it("어느 CLI를 열어야 하는지 알려준다", () => {
    expect(blockedVendor(blocked)).toBe("codex");
    expect(blockedVendor({ blocked_reason: null })).toBeNull();
    expect(blockedVendor({ blocked_reason: "auth:" })).toBeNull();
  });

  it("인증과 무관한 사유는 로그인으로 오인하지 않는다", () => {
    // 나중에 다른 차단 사유가 생겨도 로그인 버튼을 띄우면 안 된다.
    const other = { state: "Queued", awaiting_kind: null, blocked_reason: "quota:exceeded" };
    expect(isAuthBlocked(other)).toBe(false);
    expect(taskStatusLabel(other)).toBe("대기");
  });
});

describe("실행 중 열린 구조화 질문", () => {
  const running = { state: "Running", awaiting_kind: null };

  it("attention이 질문을 관측하면 실행 중 작업도 답변 대기 톤·라벨이 된다", () => {
    expect(statusTone("Running", null, null, true)).toBe("question");
    expect(statusTone("Starting", null, null, true)).toBe("question");
    expect(taskDotColor(running, true)).toBe("var(--c-question)");
    expect(taskTextClass(running, true)).toBe("text-status-question");
    expect(taskStatusLabel(running, true)).toBe("답변 대기");
    expect(taskStatusLabelShort(running, true)).toBe("답변");
  });

  it("관측이 없으면 실행 중 그대로다", () => {
    expect(statusTone("Running", null, null, false)).toBe("running");
    expect(taskStatusLabel(running)).toBe("실행 중…");
  });

  it("실행 중이 아닌 상태의 질문 관측은 상태를 덮지 않는다", () => {
    // 턴이 끝난 뒤의 잔상 — 그때는 awaiting_kind가 같은 뜻을 싣고 있다.
    expect(statusTone("Done", null, null, true)).toBe("done");
    expect(statusTone("AwaitingReview", null, null, true)).toBe("awaiting");
    expect(taskStatusLabel({ state: "Done", awaiting_kind: null }, true)).toBe("완료");
  });

  it("인증 차단은 질문보다 먼저다 — 로그인해야 답도 전달된다", () => {
    expect(statusTone("Running", null, "auth:codex", true)).toBe("awaiting");
  });
});
