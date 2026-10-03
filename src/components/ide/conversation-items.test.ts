// @vitest-environment jsdom

import { describe, expect, it } from "vitest";
import { eventToItems, GUARD_LAST_RESPONSE_MARKER } from "./ConversationView";

describe("eventToItems — 턴 가드 에러의 응답 전문 중복 제거", () => {
  it("가드가 붙인 마지막 응답 전문은 잘라내고 경고부만 남긴다 — 같은 응답이 이미 text 이벤트로 렌더링돼 있다", () => {
    // 경고 자체가 여러 줄이다 — 생존 그룹마다 pgid와 명령줄을 한 줄씩 싣는다.
    // 백엔드 포맷(turn_guard.rs)의 근사치다. 여기서 검증하는 계약은 마커 절단뿐이다.
    const warning =
      "에이전트가 살아 있는 프로세스 그룹 1건을 남기고 턴을 종료했습니다." +
      "\n\n남은 프로세스 그룹:\n- 91762 · node /opt/mcp/notion-server.js";
    const response = "## 결론\n\n| 요청 | 일치 |\n|---|---|";
    const items = eventToItems({
      kind: "result",
      is_error: true,
      text: `${warning}${GUARD_LAST_RESPONSE_MARKER}${response}`,
    });
    expect(items).toEqual([{ role: "error", text: warning }]);
  });

  it("마커가 없는 일반 에러는 원문 그대로 보여준다", () => {
    const items = eventToItems({ kind: "result", is_error: true, text: "codex 턴 실패" });
    expect(items).toEqual([{ role: "error", text: "codex 턴 실패" }]);
  });

  it("에러가 아닌 result는 기존대로 메타 푸터가 된다", () => {
    const items = eventToItems({
      kind: "result",
      is_error: false,
      text: "본문",
      cost_usd: 0.1,
      num_turns: 2,
      tokens_in: 10,
      tokens_out: 20,
    });
    expect(items).toEqual([{ role: "meta", cost: 0.1, turns: 2, tokensIn: 10, tokensOut: 20 }]);
  });
});

describe("eventToItems — 저장 이력과 라이브 스트림의 도구 출력", () => {
  it("tool_output은 결과나 assistant 답변으로 바꾸지 않고 구조화된 한 항목으로 보존한다", () => {
    const event = {
      kind: "tool_output",
      tool_use_id: "tool-1",
      contents: [{ type: "text" as const, text: "connector output" }],
    };
    expect(eventToItems(event)).toEqual([{ role: "tool_output", toolUseId: "tool-1", contents: event.contents, parentId: undefined }]);
    expect(eventToItems(JSON.parse(JSON.stringify(event)))).toEqual(eventToItems(event));
  });

  it("unknown content blocks are ignored before they reach the transcript renderer", () => {
    expect(eventToItems({ kind: "tool_output", tool_use_id: "tool-1", contents: [{ type: "image", url: 3 }] as never })).toEqual([]);
  });
});

describe("eventToItems — 서브 에이전트 실행 모델", () => {
  it("subagent_model은 부모 id와 모델을 실은 아이템 하나가 된다", () => {
    expect(
      eventToItems({ kind: "subagent_model", parent_id: "toolu_task1", model: "claude-sonnet-4-5" }),
    ).toEqual([{ role: "subagent_model", parentId: "toolu_task1", model: "claude-sonnet-4-5" }]);
  });

  it("부모나 모델이 빠지면 아이템을 만들지 않는다 — 귀속처 없는 관측은 아무 카드도 이름하지 못한다", () => {
    expect(eventToItems({ kind: "subagent_model", model: "claude-sonnet-4-5" })).toEqual([]);
    expect(eventToItems({ kind: "subagent_model", parent_id: "toolu_task1" })).toEqual([]);
  });
});

describe("eventToItems — 컨텍스트 절단 구분선", () => {
  it("context_cleared는 divider 아이템 하나가 된다 — 대화는 지우지 않고 경계만 긋는다", () => {
    expect(eventToItems({ kind: "context_cleared", text: "컨텍스트를 비웠습니다. CLAUDE.md" })).toEqual([
      { role: "divider", text: "컨텍스트를 비웠습니다. CLAUDE.md" },
    ]);
  });

  it("text가 없어도 아이템을 만든다 — 구분선이 사라지면 위아래가 이어져 보인다", () => {
    // 이 경계가 사라지는 것이 이 기능의 최악의 실패다. 아래쪽 에이전트는 위를 기억하지 못하는데
    // 화면은 하나의 연속된 대화로 읽힌다.
    expect(eventToItems({ kind: "context_cleared" })).toEqual([
      { role: "divider", text: "컨텍스트를 비웠습니다" },
    ]);
  });
});


describe("eventToItems — 세션 없이 기록만 이어받은 경계", () => {
  it("resume_no_context는 원본 작업 번호를 담은 divider 하나가 된다", () => {
    // 이 줄이 없으면 사용자는 위 대화를 에이전트가 읽은 줄 알고 "아까 그거"로 말을 건다.
    const items = eventToItems({ kind: "resume_no_context", source_task_id: 7 });
    expect(items).toHaveLength(1);
    const [item] = items;
    expect(item.role).toBe("divider");
    const text = item.role === "divider" ? item.text : "";
    expect(text).toContain("#7");
    expect(text).toContain("기억하지 못합니다");
  });

  it("source_task_id가 없어도 divider를 만든다 — 번호 자리는 #?", () => {
    const items = eventToItems({ kind: "resume_no_context" });
    expect(items).toHaveLength(1);
    const [item] = items;
    const text = item.role === "divider" ? item.text : "";
    expect(text).toContain("#?");
    expect(text).toContain("기억하지 못합니다");
  });

  it("승계된 이벤트면 divider에도 inherited가 전파된다 — 흐림 처리의 근거가 된다", () => {
    const [item] = eventToItems({ kind: "resume_no_context", source_task_id: 12, inherited: true });
    expect(item.inherited).toBe(true);
  });
});

describe("eventToItems — 세션홈에서 이어받은 벤더 세션 경계", () => {
  it("resume_external은 세션 id·경로·마지막 사용·메시지 수를 담은 divider 하나가 된다", () => {
    const now = Math.floor(Date.now() / 1000);
    const items = eventToItems({
      kind: "resume_external",
      session_id: "abcd1234",
      cwd: "/repo",
      last_active: now - 3600,
      messages: 42,
    });
    expect(items).toHaveLength(1);
    const [item] = items;
    expect(item.role).toBe("divider");
    const text = item.role === "divider" ? item.text : "";
    expect(text).toContain("abcd1234");
    expect(text).toContain("/repo");
    expect(text).toContain("시간 전");
    expect(text).toContain("42");
  });

  it("cwd·last_active·messages가 없어도 세션 id만으로 divider를 만든다", () => {
    const items = eventToItems({ kind: "resume_external", session_id: "abcd1234" });
    expect(items).toHaveLength(1);
    const [item] = items;
    const text = item.role === "divider" ? item.text : "";
    expect(text).toContain("abcd1234");
    expect(text).not.toContain("undefined");
    expect(text).not.toContain("null");
  });

  it("session_id가 없으면 번호 자리는 물음표다", () => {
    const items = eventToItems({ kind: "resume_external" });
    const [item] = items;
    const text = item.role === "divider" ? item.text : "";
    expect(text).toContain("세션 ?");
  });

  it("승계된 이벤트면 divider에도 inherited가 전파된다", () => {
    const [item] = eventToItems({ kind: "resume_external", session_id: "abcd1234", inherited: true });
    expect(item.inherited).toBe(true);
  });
});
