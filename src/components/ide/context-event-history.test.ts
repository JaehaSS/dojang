import { describe, expect, it } from "vitest";
import { coalesceConvoEvent, type ConvoEventLike } from "./ConversationView";

describe("context event history", () => {
  it("replaces optimistic remote input with its durable metrics without counting it twice", () => {
    const optimistic: ConvoEventLike = { kind: "user", text: "요청", optimistic: true };
    const durable: ConvoEventLike = { kind: "user", text: "요청", prompt_metrics: { version: 1, user_bytes: 6 } };
    expect(coalesceConvoEvent([optimistic, { kind: "model_snapshot" }], durable)).toEqual([durable, { kind: "model_snapshot" }]);
  });
  it("keeps repeated durable requests even when an interrupted turn has no result", () => {
    const durable = { kind: "user", text: "요청" };
    expect(coalesceConvoEvent([durable], durable)).toHaveLength(2);
  });
  it("does not replace pending input with a peer or child request", () => {
    const optimistic: ConvoEventLike = { kind: "user", text: "요청", optimistic: true };
    for (const scope of [{ speaker: "right" as const }, { parent_id: "child" }]) {
      expect(coalesceConvoEvent([optimistic], { kind: "user", text: "요청", ...scope })).toHaveLength(2);
    }
  });
  it("keeps identical requests across completed turns and reset boundaries", () => {
    for (const boundary of ["result", "context_cleared", "debate_ended"]) {
      const history = [{ kind: "user", text: "요청" }, { kind: boundary }];
      expect(coalesceConvoEvent(history, { kind: "user", text: "요청" })).toHaveLength(3);
    }
  });
  it("coalesces each streaming snapshot during remote replay and continuation", () => {
    const history = [{ kind: "text_update", item_id: "reply", text: "첫" }];
    expect(coalesceConvoEvent(history, { kind: "text_update", item_id: "reply", text: "완료" })).toEqual([{ kind: "text_update", item_id: "reply", text: "완료" }]);
  });
});
