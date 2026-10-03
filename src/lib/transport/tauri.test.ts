import { beforeEach, describe, expect, it, vi } from "vitest";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { tauriTransport } from "./tauri";

describe("tauriTransport", () => {
  beforeEach(() => invoke.mockReset());

  it("routes repair preparation, execution and adoption separately", async () => {
    await tauriTransport.approvalRepairPrepare(7);
    await tauriTransport.approvalRepairRun(7, "session");
    await tauriTransport.approvalRepairAccept(7, "session");
    expect(invoke.mock.calls).toEqual([["approval_repair_prepare", {id:7}], ["approval_repair_run", {id:7,sessionId:"session"}], ["approval_repair_accept", {id:7,sessionId:"session"}]]);
  });

  it("routes advisory approval inspection by task id", async () => {
    invoke.mockResolvedValue({ readiness: null, inspection_error: null, attempts: [] });
    await tauriTransport.taskApprovalStatus(7);
    expect(invoke).toHaveBeenCalledWith("task_approval_status", { id: 7 });
  });

  it("maps the shared snake_case contract field to Tauri's camelCase command argument", async () => {
    invoke.mockResolvedValue({ id: 1 });

    await tauriTransport.taskCreate({
      host: "local",
      repo: "/repo",
      instruction: "ship it",
      agent: "codex",
      role: "reviewer",
      model: "gpt-5.6-sol",
      reasoning_effort: "high",
      service_tier: "fast",
      headless: false,
      ensemble: "",
      mode: "conversation",
      cmd: "",
      args: [],
      cols: 80,
      rows: 24,
      goal_contract: {
        schema_version: 1,
        objective: "ship safely",
        acceptance: [],
        stop_conditions: [],
        must_preserve: [],
        protected_paths: [],
        non_goals: [],
      },
    });

    expect(invoke).toHaveBeenCalledWith(
      "task_create",
      expect.objectContaining({
        goalContract: expect.objectContaining({ objective: "ship safely" }),
        reasoningEffort: "high",
        serviceTier: "fast",
        role: "reviewer",
      }),
    );
    expect(invoke.mock.calls[0][1]).not.toHaveProperty("goal_contract");
    expect(invoke.mock.calls[0][1]).not.toHaveProperty("reasoning_effort");
    expect(invoke.mock.calls[0][1]).not.toHaveProperty("service_tier");
  });

  it("taskResume은 clientRef가 있을 때만 실어 보낸다 — taskCreate와 같은 관례", async () => {
    invoke.mockResolvedValue({ id: 9 });

    await tauriTransport.taskResume(3, "이어서 계속해줘");
    expect(invoke).toHaveBeenLastCalledWith("task_resume", { id: 3, message: "이어서 계속해줘" });

    await tauriTransport.taskResume(3, "이어서 계속해줘", "ref-1");
    expect(invoke).toHaveBeenLastCalledWith("task_resume", {
      id: 3,
      message: "이어서 계속해줘",
      clientRef: "ref-1",
    });
  });

  it("maps quickopen search to local Tauri commands", async () => {
    invoke.mockResolvedValue([]);

    await tauriTransport.quickopenSearch("deploy", ["task", "session"]);

    expect(invoke.mock.calls).toEqual([
      ["quickopen_search", { query: "deploy", scopes: ["task", "session"] }],
    ]);
  });

  it("maps review methods only to local Tauri commands", async () => {
    invoke.mockResolvedValue(null);

    await tauriTransport.verifySpec(7);
    await tauriTransport.taskVerify(7, "preview-1");

    expect(invoke.mock.calls).toEqual([
      ["verify_spec", { id: 7 }],
      ["task_verify", { id: 7, previewToken: "preview-1" }],
    ]);
  });

  it("maps task cancellation to the stop command, not discard", async () => {
    invoke.mockResolvedValue(null);

    await tauriTransport.taskCancel(7);

    expect(invoke).toHaveBeenCalledWith("task_cancel", { id: 7 });
  });

  it("maps diff hunks and annotation methods to local Tauri commands", async () => {
    invoke.mockResolvedValue([]);

    await tauriTransport.diffHunks(7);
    await tauriTransport.annotationsList(7);
    await tauriTransport.annotationSave(7, {
      hunk_id: "h1",
      path: "a.ts",
      line: 11,
      side: "new",
      body_md: "고쳐줘",
    });
    await tauriTransport.annotationSave(7, {
      id: "ann-1",
      hunk_id: "h1",
      path: "a.ts",
      line: 11,
      side: "new",
      body_md: "수정본",
    });
    await tauriTransport.annotationsResend(7, ["ann-1", "ann-2"]);

    expect(invoke.mock.calls).toEqual([
      ["diff_hunks", { id: 7 }],
      ["annotations_list", { taskId: 7 }],
      [
        "annotation_save",
        { taskId: 7, id: null, hunkId: "h1", path: "a.ts", line: 11, side: "new", bodyMd: "고쳐줘" },
      ],
      [
        "annotation_save",
        { taskId: 7, id: "ann-1", hunkId: "h1", path: "a.ts", line: 11, side: "new", bodyMd: "수정본" },
      ],
      ["annotations_resend", { taskId: 7, ids: ["ann-1", "ann-2"] }],
    ]);
  });

  it("maps partial apply/rollback to local Tauri commands", async () => {
    invoke.mockResolvedValue({ checkpoint: "sha1", kept_hunk_ids: ["h1"], discarded_hunk_ids: ["h2"] });

    await tauriTransport.partialApply(7, ["h1"]);
    await tauriTransport.partialRollback(7);

    expect(invoke.mock.calls).toEqual([
      ["partial_apply", { id: 7, hunkIds: ["h1"] }],
      ["partial_rollback", { id: 7 }],
    ]);
  });

  it("maps ensemble matrix/compose to local Tauri commands", async () => {
    invoke.mockResolvedValue({ candidate_ids: [1, 2], exclusive_groups: [] });

    await tauriTransport.ensembleMatrix("ens-1");
    await tauriTransport.ensembleCompose("ens-1", 1, [{ task_id: 2, hunk_id: "h1" }]);

    expect(invoke.mock.calls).toEqual([
      ["ensemble_matrix", { ensemble: "ens-1" }],
      [
        "ensemble_compose",
        { ensemble: "ens-1", winnerTaskId: 1, selections: [{ task_id: 2, hunk_id: "h1" }] },
      ],
    ]);
  });

  it("maps GitHub issue methods to local Tauri commands", async () => {
    invoke.mockResolvedValue({ status: "not_github_repo" });

    await tauriTransport.githubIssuesList("/repo");
    await tauriTransport.githubReposList(["/repo", "/other"]);
    await tauriTransport.githubCreateTaskFromIssue("/repo", 42, "claude");
    await tauriTransport.githubIssueDelete("/repo", 42);

    expect(invoke.mock.calls).toEqual([
      ["github_issues_list", { repo: "/repo" }],
      ["github_repos_list", { repos: ["/repo", "/other"] }],
      ["github_create_task_from_issue", { repo: "/repo", number: 42, agent: "claude" }],
      ["github_issue_delete", { repo: "/repo", number: 42 }],
    ]);
  });

  it("asks task_diff for the atomic review snapshot with Tauri camelCase arguments", async () => {
    invoke.mockResolvedValue({ files: [], baseline: { kind: "pinned" } });

    await tauriTransport.taskDiff(7, "uncommitted", true);

    expect(invoke).toHaveBeenCalledWith("task_diff", {
      id: 7,
      range: "uncommitted",
      includeReview: true,
    });
  });
});


it("maps isolated questions and idempotent main receipts to their dedicated commands", async () => {
  invoke.mockReset();
  invoke.mockResolvedValue({});
  const input = { request_id: "q-1", generation: 2, question: "why?", contexts: [{ label: "selected", text: "only this" }] };
  await tauriTransport.sideQuestionRead(7);
  await tauriTransport.sideQuestionSend(7, input);
  await tauriTransport.sideQuestionCancel(7, 9);
  await tauriTransport.sideQuestionReset(7, 2);
  await tauriTransport.conversationSubmit(7, "main-1", "apply this", ["/image.png"]);
  await tauriTransport.conversationReceipt(7, "main-1");
  expect(invoke.mock.calls).toEqual([
    ["side_question_read", { taskId: 7 }],
    ["side_question_send", { taskId: 7, input }],
    ["side_question_cancel", { taskId: 7, turnId: 9 }],
    ["side_question_reset", { taskId: 7, generation: 2 }],
    ["conversation_submit", { taskId: 7, requestId: "main-1", message: "apply this", imagePaths: ["/image.png"] }],
    ["conversation_receipt", { taskId: 7, requestId: "main-1" }],
  ]);
});

it("forwards the active composer generation for explicit wiki attachments", async () => {
  invoke.mockReset(); invoke.mockResolvedValue({});
  await tauriTransport.conversationSubmit(7, "request", "message", [], "vault-followup:7:nonce");
  expect(invoke).toHaveBeenCalledWith("conversation_submit", { taskId: 7, requestId: "request", message: "message", imagePaths: [], previewClientRef: "vault-followup:7:nonce" });
});
