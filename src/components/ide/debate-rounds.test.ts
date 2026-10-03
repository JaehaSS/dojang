import { describe, expect, it } from "vitest";
import type { DebateSpeaker } from "../../lib/ipc";
import { activeSide, consensusText, debateRounds, endsSequence, type DebateEventLike } from "./debate-rounds";

const text = (body: string, speaker?: DebateSpeaker): DebateEventLike => ({ kind: "text", text: body, speaker });
const user = (body: string): DebateEventLike => ({ kind: "user", text: body });

describe("debateRounds", () => {
  it("speaker 없는 과거 이벤트는 공통 영역에 남는다 — 없다는 것은 좌측이 아니라 미상이다", () => {
    const { preamble, rounds } = debateRounds([text("옛 대화"), { kind: "text", text: "옛 답" }]);
    expect(preamble.map((item) => (item.role === "text" ? item.text : ""))).toEqual(["옛 대화", "옛 답"]);
    expect(rounds).toHaveLength(0);
  });

  it("한 라운드의 좌 3건·우 1건이 같은 행에 정렬된다", () => {
    const { rounds } = debateRounds([
      user("어느 쪽인가?"),
      text("A안", "left"),
      text("근거", "left"),
      text("보충", "left"),
      text("전제가 틀렸다", "right"),
    ]);
    expect(rounds).toHaveLength(1);
    expect(rounds[0].no).toBe(1);
    expect(rounds[0].user).toBe("어느 쪽인가?");
    expect(rounds[0].panes[0]).toHaveLength(3);
    expect(rounds[0].panes[1]).toHaveLength(1);
  });

  it("우측 다음에 좌측이 오면 라운드가 오른다 — 발화 길이와 무관하다", () => {
    const { rounds } = debateRounds([
      user("어느 쪽인가?"),
      text("A안", "left"),
      text("반박", "right"),
      text("수용", "left"),
      text("동의", "right"),
    ]);
    expect(rounds.map((round) => round.no)).toEqual([1, 2]);
    expect(rounds[1].panes[0]).toHaveLength(1);
    expect(rounds[1].panes[1]).toHaveLength(1);
  });

  it("새 사용자 발화는 라운드 번호를 1로 되돌린다 — 상한이 발화 한 건에 걸린다", () => {
    const { rounds } = debateRounds([
      user("첫 질문"),
      text("좌1", "left"),
      text("우1", "right"),
      user("이어서"),
      text("좌2", "left"),
    ]);
    // 이어 세면 `라운드 4/3` 같은 라벨이 나온다 — 상한은 발화 한 건의 시퀀스에 걸린다.
    expect(rounds.map((round) => round.no)).toEqual([1, 1]);
    expect(rounds[1].user).toBe("이어서");
  });

  it("3자면 셋째 다음에 좌측이 와야 라운드가 오른다 — 우측 다음 셋째는 같은 라운드다", () => {
    const { rounds } = debateRounds([
      user("q"),
      text("좌1", "left"),
      text("우1", "right"),
      text("셋1", "third"),
      text("좌2", "left"),
      text("우2", "right"),
    ]);
    expect(rounds.map((round) => round.no)).toEqual([1, 2]);
    expect(rounds[0].panes.map((pane) => pane.length)).toEqual([1, 1, 1]);
    expect(rounds[1].panes.map((pane) => pane.length)).toEqual([1, 1, 0]);
  });

  it("알 수 없는 발화자는 어느 면에도 접지 않고 공통 영역에 둔다", () => {
    const { rounds } = debateRounds([user("q"), text("좌", "left"), { kind: "text", text: "?", speaker: "fourth" as DebateSpeaker }]);
    expect(rounds[0].common).toHaveLength(1);
    expect(rounds[0].panes.map((pane) => pane.length)).toEqual([1, 0, 0]);
  });

  it("debate_ended가 마지막 배너 상태를 만든다", () => {
    const events: DebateEventLike[] = [user("질문"), text("좌", "left"), text("우", "right")];
    expect(debateRounds(events).ended).toBeUndefined();
    expect(debateRounds([...events, { kind: "debate_ended", reason: "consensus" }]).ended).toBe("consensus");
    expect(debateRounds([...events, { kind: "debate_ended", reason: "round_cap" }]).ended).toBe("round_cap");
    // 배너가 뜬 뒤 사용자가 다시 말하면 화면은 running으로 돌아간다.
    expect(debateRounds([...events, { kind: "debate_ended", reason: "aborted" }, user("계속")]).ended).toBeUndefined();
  });
});

describe("activeSide · consensusText", () => {
  it("커서는 마지막 라운드에서 마지막으로 말한 면에 있다", () => {
    expect(activeSide(debateRounds([user("q"), text("좌", "left")]))).toBe("left");
    expect(activeSide(debateRounds([user("q"), text("좌", "left"), text("우", "right")]))).toBe("right");
    expect(activeSide(debateRounds([user("q"), text("좌", "left"), text("우", "right"), text("셋", "third")]))).toBe("third");
  });

  it("3자 결론 복사는 셋째의 마지막 발화를 넘긴다", () => {
    const transcript = debateRounds([
      user("q"),
      text("좌\n[합의]", "left"),
      text("우\n[합의]", "right"),
      text("셋 결론\n[합의]", "third"),
      { kind: "debate_ended", reason: "consensus" },
    ]);
    expect(consensusText(transcript)).toBe("셋 결론\n[합의]");
  });

  it("결론 복사는 마지막 합의 발화 본문을 넘긴다", () => {
    const transcript = debateRounds([
      user("q"),
      text("좌 결론", "left"),
      text("동의한다. 결론은 A안.\n[합의]", "right"),
      { kind: "debate_ended", reason: "consensus" },
    ]);
    expect(consensusText(transcript)).toBe("동의한다. 결론은 A안.\n[합의]");
  });
});

describe("endsSequence", () => {
  it("면마다 오는 result로는 시퀀스가 끝나지 않는다 — 끝은 debate_ended뿐이다", () => {
    expect(endsSequence({ kind: "result", text: "좌 턴 끝", speaker: "left" })).toBe(false);
    expect(endsSequence({ kind: "result", text: "우 턴 끝", speaker: "right" })).toBe(false);
    expect(endsSequence({ kind: "debate_ended", reason: "round_cap" })).toBe(true);
  });

  it("토론이 아닌 턴은 speaker 없는 result 하나로 끝난다", () => {
    expect(endsSequence({ kind: "result", text: "끝" })).toBe(true);
    expect(endsSequence({ kind: "text", text: "도는 중" })).toBe(false);
  });
});
