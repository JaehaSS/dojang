import { describe, expect, it, vi } from "vitest";
import { ConversationSubmitter, type ConversationAdmission } from "./conversation-submit";
import type { MessageReceipt } from "./side-question";

const receipt = (request_id: string, status: MessageReceipt["status"]): MessageReceipt => ({ request_id, status, error: null });

describe("main conversation admission", () => {
  it("recovers an accepted request after its response is lost", async () => {
    const api: ConversationAdmission = {
      submit: vi.fn(async () => { throw new Error("lost response"); }),
      receipt: vi.fn(async (id) => receipt(id, "accepted")),
    };
    await new ConversationSubmitter().send("remote:7", api, "apply this", []);
    expect(api.submit).toHaveBeenCalledOnce();
    expect(api.receipt).toHaveBeenCalledOnce();
  });

  it("retries an uncertain request with the same immutable id and images", async () => {
    const submit = vi.fn(async (id: string) => receipt(id, "unknown"));
    const api: ConversationAdmission = { submit, receipt: async (id) => receipt(id, "unknown") };
    const owner = new ConversationSubmitter();
    await expect(owner.send("local:7", api, "apply", ["/image.png"])).rejects.toThrow("접수 여부");
    const original = owner.inspect("local:7")!;
    expect(original.message).toBe("apply");
    original.images.push("/unsubmitted.png");
    expect(owner.inspect("local:7")?.images).toEqual(["/image.png"]);
    submit.mockImplementation(async (id) => receipt(id, "accepted"));
    await owner.send("local:7", api, "apply", ["/image.png"]);
    expect(submit.mock.calls[0][0]).toBe(submit.mock.calls[1][0]);
  });

  it("does not submit a different draft over an uncertain request or leak across hosts", async () => {
    const submit = vi.fn(async (id: string) => receipt(id, "unknown"));
    const api: ConversationAdmission = { submit, receipt: async (id) => receipt(id, "unknown") };
    const owner = new ConversationSubmitter();
    await expect(owner.send("local:7", api, "original", [])).rejects.toThrow();
    await expect(owner.send("local:7", api, "changed", [])).rejects.toThrow("이전 요청");
    expect(submit).toHaveBeenCalledOnce();
    await expect(owner.send("remote:7", api, "changed", [])).rejects.toThrow();
    expect(submit).toHaveBeenCalledTimes(2);
    expect(submit.mock.calls[0][0]).not.toBe(submit.mock.calls[1][0]);
  });

  it("blocks duplicate clicks until the first admission completes", async () => {
    let finish!: (receipt: MessageReceipt) => void;
    let id = "";
    const api: ConversationAdmission = {
      submit: vi.fn((requestId: string) => { id = requestId; return new Promise<MessageReceipt>((resolve) => { finish = resolve; }); }),
      receipt: async (requestId) => receipt(requestId, "not_found"),
    };
    const owner = new ConversationSubmitter();
    const sending = owner.send("local:7", api, "apply", []);
    await expect(owner.send("local:7", api, "apply", [])).rejects.toThrow("확인");
    finish(receipt(id, "accepted"));
    await sending;
    expect(api.submit).toHaveBeenCalledOnce();
  });
});

it("does not reuse an uncertain request with a new wiki draft generation after navigation", async () => {
  const submit = vi.fn(async (id: string) => receipt(id, "unknown"));
  const api: ConversationAdmission = { submit, receipt: vi.fn(async id => receipt(id, "unknown")) };
  const owner = new ConversationSubmitter();
  await expect(owner.send("local:7", api, "same text", [], "draft-old")).rejects.toThrow();
  await expect(owner.send("local:7", api, "same text", [], "draft-new")).rejects.toThrow("이전 요청");
  expect(submit).toHaveBeenCalledOnce();
  expect(owner.inspect("local:7")?.contextKey).toBe("draft-old");
});
