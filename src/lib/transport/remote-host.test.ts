import { afterEach, describe, expect, it, vi } from "vitest";
import { hasHost, unregisterTransport } from "../transport";
import {
  getRemoteRunnerTransport,
  loadRemoteRunnerProfile,
  saveRemoteRunnerProfile,
  syncRemoteRunnerTransport,
} from "./remote-host";
import { RunnerTransport } from "./runner";

describe("remote-host and RunnerTransport assistant contract", () => {
  afterEach(() => {
    saveRemoteRunnerProfile({
      hostId: "mini",
      endpoint: "",
      pairingToken: "",
      enabled: false,
    });
    if (hasHost("mini")) {
      unregisterTransport("mini");
    }
  });

  it("registers and unregisters the remote runner transport when profile is saved", () => {
    const saved = saveRemoteRunnerProfile({
      hostId: "mini",
      endpoint: "https://mini.tailnet.ts.net/",
      pairingToken: "a".repeat(64),
      enabled: true,
    });
    expect(saved.endpoint).toBe("https://mini.tailnet.ts.net");
    expect(hasHost("mini")).toBe(true);
    expect(getRemoteRunnerTransport()?.hostId).toBe("mini");

    saveRemoteRunnerProfile({
      ...loadRemoteRunnerProfile(),
      enabled: false,
    });
    syncRemoteRunnerTransport();
    expect(hasHost("mini")).toBe(false);
  });

  it("calls /v1/assistant endpoints and tags returned tasks with hostId", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith("/v1/assistant") && (!init?.method || init.method === "GET")) {
        return new Response(
          JSON.stringify({
            task: {
              id: 7,
              repo: "/home/mini/vault",
              branch: "(direct)",
              base: "(direct)",
              worktree_path: "/home/mini/vault",
              instruction: "상시 비서 세션",
              state: "AwaitingReview",
              created_at: 1,
              updated_at: 2,
              agent: "claude",
              mode: "conversation",
              kind: "assistant",
            },
            pending_approvals: [
              {
                id: 3,
                assistant_task_id: 7,
                target_task_id: 12,
                action_kind: "approve_task",
                summary: "작업 #12 머지 승인",
                payload_json: "{}",
                status: "pending",
                created_at: 10,
                resolved_at: null,
                error: null,
              },
            ],
            overview: {
              running_count: 1,
              queued_count: 0,
              awaiting_review_count: 2,
              question_count: 1,
              pending_approval_count: 1,
              failed_count: 0,
            },
          }),
          { status: 200 },
        );
      }
      if (url.endsWith("/v1/assistant/config") && init?.method === "PATCH") {
        const body = JSON.parse(String(init.body));
        return new Response(
          JSON.stringify({
            id: 7,
            repo: "/home/mini/vault",
            branch: "(direct)",
            base: "(direct)",
            worktree_path: "/home/mini/vault",
            instruction: "상시 비서 세션",
            state: "AwaitingReview",
            created_at: 1,
            updated_at: 3,
            agent: body.agent,
            model: body.model,
            mode: "conversation",
            kind: "assistant",
          }),
          { status: 200 },
        );
      }
      if (url.endsWith("/v1/assistant/approvals/3/approve") && init?.method === "POST") {
        return new Response(
          JSON.stringify({
            id: 3,
            assistant_task_id: 7,
            target_task_id: 12,
            action_kind: "approve_task",
            summary: "작업 #12 머지 승인",
            payload_json: "{}",
            status: "executed",
            created_at: 10,
            resolved_at: 20,
            error: null,
          }),
          { status: 200 },
        );
      }
      return new Response("not found", { status: 404 });
    });

    const transport = new RunnerTransport(
      {
        profileName: "mini",
        endpoint: "https://mini.tailnet.ts.net",
        pairingToken: "b".repeat(64),
      },
      fetchMock,
    );

    const snapshot = await transport.assistantGet();
    expect(snapshot.task.host).toBe("mini");
    expect(snapshot.task.kind).toBe("assistant");
    expect(snapshot.overview.question_count).toBe(1);
    expect(snapshot.pending_approvals).toHaveLength(1);

    const updated = await transport.assistantUpdateConfig({
      agent: "codex",
      model: "gpt-5.4",
    });
    expect(updated.host).toBe("mini");
    expect(updated.agent).toBe("codex");

    const approved = await transport.assistantApprove(3);
    expect(approved.status).toBe("executed");
  });
});
