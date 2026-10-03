import type { MessageReceipt } from "./side-question";

export interface ConversationAdmission {
  submit(requestId: string, message: string, images: string[]): Promise<MessageReceipt>;
  receipt(requestId: string): Promise<MessageReceipt>;
}

interface Pending {
  requestId: string;
  message: string;
  images: string[];
  contextKey?: string;
}

/** One immutable request per host/task until admission is known. A failed network reply is
 * not permission to create a second turn. The caller owns drafts and consumes only on success. */
export class ConversationSubmitter {
  private pending = new Map<string, Pending>();
  private active = new Set<string>();

  /** A retry preview must show the immutable submission, even if the user edited the draft. */
  inspect(key: string): Readonly<Pending> | null {
    const request = this.pending.get(key);
    return request ? { ...request, images: [...request.images] } : null;
  }

  /** Resolve an uncertain request without submitting it again, even if the task has ended. */
  async reconcile(key: string, api: ConversationAdmission): Promise<"accepted" | "failed" | "unknown"> {
    if (this.active.has(key)) return "unknown";
    const request = this.pending.get(key);
    if (!request) return "failed";
    const receipt = await api.receipt(request.requestId);
    if (this.pending.get(key) !== request) return "unknown";
    if (receipt.status === "accepted" || receipt.status === "failed") {
      this.pending.delete(key);
      return receipt.status;
    }
    return "unknown";
  }

  async send(key: string, api: ConversationAdmission, message: string, images: string[], contextKey?: string): Promise<void> {
    if (this.active.has(key)) throw new Error("전송 상태를 확인하고 있습니다.");
    this.active.add(key);
    try {
      let request = this.pending.get(key);
      if (request && (request.message !== message || JSON.stringify(request.images) !== JSON.stringify(images) || request.contextKey !== contextKey)) {
        const prior = await api.receipt(request.requestId);
        if (prior.status === "accepted" || prior.status === "failed") {
          this.pending.delete(key);
          throw new Error(prior.status === "accepted"
            ? "이전 요청의 접수를 확인했습니다. 현재 초안은 유지했습니다. 새 요청을 보내주세요."
            : "이전 요청이 접수되지 않았습니다. 현재 초안을 다시 보내주세요.");
        }
        throw new Error("이전 요청의 전송 상태가 아직 불명확합니다. 연결 후 다시 확인해주세요. 현재 초안은 유지됩니다.");
      }
      if (!request) {
        request = { requestId: crypto.randomUUID(), message, images: [...images], ...(contextKey ? { contextKey } : {}) };
        this.pending.set(key, request);
      }
      let receipt: MessageReceipt;
      try {
        receipt = await api.submit(request.requestId, request.message, request.images);
      } catch {
        try {
          receipt = await api.receipt(request.requestId);
        } catch {
          throw new Error("전송 결과를 확인할 수 없습니다. 초안을 보존했습니다. 다시 보내면 같은 요청의 상태를 확인합니다.");
        }
      }
      if (receipt.status === "accepted") {
        this.pending.delete(key);
        return;
      }
      if (receipt.status === "failed") this.pending.delete(key);
      throw new Error(receipt.error ?? (receipt.status === "failed"
        ? "요청이 접수되지 않았습니다. 초안을 확인하고 다시 보내주세요."
        : "접수 여부를 확인하지 못했습니다. 초안을 보존했으며 재시도는 같은 요청 ID를 사용합니다."));
    } finally {
      this.active.delete(key);
    }
  }
}
