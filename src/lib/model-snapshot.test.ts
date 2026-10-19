import { describe, expect, it } from "vitest";
import { EMPTY_MODEL_SNAPSHOT, foldModelSnapshot } from "./model-snapshot";

const snap = (over: Record<string, unknown> = {}) => ({
  kind: "model_snapshot",
  ...over,
});

describe("foldModelSnapshot", () => {
  it("keeps the main model when a debate peer or child reports a different tier", () => {
    const main = { resolved: "claude-opus-5", requested: "opus" };
    expect(foldModelSnapshot(main, [snap({ resolved: "claude-opus-5[1m]", speaker: "right" }), snap({ resolved: "child", parent_id: "child" }), { kind: "context_cleared", speaker: "right" }])).toEqual(main);
  });
  it("requested만 실은 이벤트가 앞선 resolved를 지우지 않는다", () => {
    // 정상 턴의 순서가 바로 이것이다 — invocation(requested) 다음 claude_stream(resolved).
    // 덮어쓰기로 처리하면 둘 중 하나가 매 턴 사라진다.
    const first = foldModelSnapshot(EMPTY_MODEL_SNAPSHOT, [
      snap({ requested: "opus", source: "invocation" }),
      snap({ resolved: "claude-opus-5[1m]", source: "claude_stream" }),
    ]);
    expect(first).toEqual({ resolved: "claude-opus-5[1m]", requested: "opus" });

    const second = foldModelSnapshot(first, [snap({ requested: "opus", source: "invocation" })]);
    expect(second).toEqual({ resolved: "claude-opus-5[1m]", requested: "opus" });
  });

  it("나중 값이 이긴다 — 같은 필드를 다시 실으면 갱신된다", () => {
    const out = foldModelSnapshot(EMPTY_MODEL_SNAPSHOT, [
      snap({ resolved: "claude-opus-5[1m]" }),
      snap({ resolved: "claude-sonnet-5" }),
    ]);
    expect(out.resolved).toBe("claude-sonnet-5");
  });

  it("model_snapshot이 아닌 이벤트는 통과시킨다", () => {
    const prev = { resolved: "claude-opus-5", requested: "opus" };
    expect(
      foldModelSnapshot(prev, [
        { kind: "text" },
        { kind: "context_usage" },
        { kind: "result" },
      ]),
    ).toEqual(prev);
  });

  it("Codex 자식 상태와 본문이 메인 모델을 덮어쓰지 않는다", () => {
    const parent = { resolved: "parent-observed-model", requested: "parent-requested-model" };
    const child = {
      kind: "subagent_update",
      model: "child-observed-model",
      resolved: "child-observed-model",
      requested: "child-requested-model",
      state: "done",
    };
    expect(foldModelSnapshot(parent, [child, { kind: "subagent_message" }])).toEqual(parent);
  });

  it("빈 열이면 이전 스냅샷을 그대로 돌려준다 — 원격 이어읽기의 빈 배치", () => {
    const prev = { resolved: "claude-opus-5", requested: null };
    expect(foldModelSnapshot(prev, [])).toEqual(prev);
  });

  it("EMPTY부터 접으면 이전 세션 값이 새지 않는다", () => {
    const out = foldModelSnapshot(EMPTY_MODEL_SNAPSHOT, [{ kind: "text" }]);
    expect(out).toEqual({ resolved: null, requested: null });
  });

  it("context_cleared 뒤에는 이전 에이전트의 관측 모델을 이어 쓰지 않는다", () => {
    const out = foldModelSnapshot({ resolved: "claude-opus-5", requested: "opus" }, [
      { kind: "context_cleared" },
      snap({ requested: "gpt-5.6-terra" }),
    ]);
    expect(out).toEqual({ resolved: null, requested: "gpt-5.6-terra" });
  });
});
