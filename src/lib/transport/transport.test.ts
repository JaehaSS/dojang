import { beforeEach, describe, expect, it } from "vitest";
import {
  LOCAL_HOST,
  getTransport,
  hasHost,
  listHosts,
  registerTransport,
  taskKey,
  unregisterTransport,
  type PraxisTransport,
} from "../transport";

const transport: PraxisTransport = {
  kind: "local",
  hostId: LOCAL_HOST,
  taskList: async () => [],
  taskDiffStat: async () => "",
  taskDiff: async () => ({ files: [], baseline: { kind: "pinned" as const } }),
  fsTree: async () => [],
  fsTreePath: async () => [],
  fsBrowse: async () => ({ path: "/", parent: null, entries: [] }),
  fsRoots: async () => [],
  fsCreateFile: async () => "",
  fsCreateDir: async () => "",
  fsRename: async () => "",
  fsTrash: async () => {},
  fsCopy: async () => "",
  fsOpenTerminal: async () => {},
  gitStatus: async () => true,
  gitInit: async () => true,
  gitBranches: async () => ({ current: "main", branches: ["main"] }),
  gitCheckoutBranch: async () => ({ current: "main", branches: ["main"] }),
  repositoryList: async () => [],
  fsRead: async () => ({ kind: "text", content: "", mtime: 0 }),
  fsWrite: async () => 0,
  scheduleList: async () => [],
  taskCreate: async () => {
    throw new Error("not used");
  },
  taskResume: async () => {
    throw new Error("not used");
  },
  taskInput: async () => {},
  taskMessage: async () => {},
  taskModelSet: async () => {},
  conversationSubmit: async (_id, requestId) => ({ request_id: requestId, status: "accepted", error: null }),
  conversationReceipt: async (_id, requestId) => ({ request_id: requestId, status: "not_found", error: null }),
  sideQuestionRead: async () => { throw new Error("not used"); },
  sideQuestionSend: async () => { throw new Error("not used"); },
  sideQuestionCancel: async () => { throw new Error("not used"); },
  sideQuestionReset: async () => { throw new Error("not used"); },
  taskCancel: async () => {},
  taskDelete: async () => {},
  taskApprove: async () => {},
  taskApprovalStatus: async () => ({ readiness: null, inspection_error: null, attempts: [] }),
  approvalRepairStatus: async () => null,
  approvalRepairPrepare: async () => { throw new Error("not used"); },
  approvalRepairRun: async () => { throw new Error("not used"); },
  approvalRepairAccept: async () => { throw new Error("not used"); },
  approvalRepairCancel: async () => {},
  taskDiscard: async () => {},
  taskRunApprove: async () => {},
  taskRunReject: async () => {},
  githubIssuesList: async () => ({ status: "ready", owner_repo: "acme/app", issues: [] }),
  githubReposList: async () => [],
  githubCreateTaskFromIssue: async () => {
    throw new Error("not used");
  },
  githubIssueDelete: async () => {},
  verifySpec: async () => ({
    build: null,
    test: null,
    timeout_secs: 60,
    preview_token: "preview",
  }),
  taskVerify: async () => ({
    spec: { build: null, test: null, timeout_secs: 60 },
    build: null,
    test: null,
    summary: null,
    ready: false,
    checks: [],
    warnings: [],
  }),
  reviewProcessQuarantines: async () => [],
  reviewProcessReconcile: async (receiptId) => ({
    receipt_id: receiptId,
    status: "already_resolved",
    reason: null,
    detail: null,
  }),
  scheduleAdd: async () => 1,
  scheduleRemove: async () => {},
  scheduleSetEnabled: async () => {},
  reminderAdd: async () => 1,
  skillsList: async () => [],
  quickopenSearch: async () => [],
  sessionHomeIndex: async () => [],
  sessionHomePreview: async () => ({ meta: { session_id: "", cwd: null, last_cwd: null, git_branch: null, title: null, first_message: null, last_active: 0, messages: 0, vendor_version: null }, messages: [], truncated: false }),
  diffHunks: async () => [],
  annotationsList: async () => [],
  annotationSave: async () => ({
    id: "ann-1",
    task_id: 1,
    hunk_id: "h1",
    path: "a.ts",
    line: 1,
    side: "new",
    body_md: "",
    status: "draft",
    created_at: 0,
  }),
  annotationsResend: async () => {},
  partialApply: async () => ({ checkpoint: "sha", kept_hunk_ids: [], discarded_hunk_ids: [] }),
  partialRollback: async () => {},
  ensembleMatrix: async () => ({ candidate_ids: [], exclusive_groups: [] }),
  ensembleCompose: async () => ({ checkpoint: "sha", applied: [] }),
  interviewStart: async () => {
    throw new Error("not used");
  },
  interviewCrystallize: async () => {
    throw new Error("not used");
  },
  grillRound: async () => {
    throw new Error("not used");
  },
  grillNote: async () => {
    throw new Error("not used");
  },
  grillSaveNote: async () => {
    throw new Error("not used");
  },
};

/** 같은 계약을 가진 다른 호스트의 transport. 레지스트리 키만 다르다. */
const onHost = (hostId: string): PraxisTransport => ({
  ...transport,
  kind: hostId === LOCAL_HOST ? "local" : "remote",
  hostId,
});

describe("transport registry", () => {
  beforeEach(() => {
    // 레지스트리는 모듈 전역이다 — 케이스 사이에 원격 호스트를 남기지 않는다.
    listHosts()
      .filter((host) => host !== LOCAL_HOST)
      .forEach(unregisterTransport);
    registerTransport(transport);
  });

  it("호스트를 명시해 transport를 얻는다 — 무인자 조회는 더 이상 없다", async () => {
    expect(getTransport(LOCAL_HOST)).toBe(transport);
    await expect(getTransport(LOCAL_HOST).taskList()).resolves.toEqual([]);
  });

  it("등록한 호스트를 이름으로 되찾는다", () => {
    const mini = onHost("mini1");
    registerTransport(mini);

    expect(getTransport("mini1")).toBe(mini);
    expect(getTransport(LOCAL_HOST)).toBe(transport);
  });

  it("미등록 호스트 조회는 호스트 이름을 담아 던진다", () => {
    expect(() => getTransport("없는호스트")).toThrow(/없는호스트/);
    expect(hasHost("없는호스트")).toBe(false);
  });

  it("로컬 호스트는 해제할 수 없다", () => {
    expect(() => unregisterTransport(LOCAL_HOST)).toThrow(/로컬/);
    expect(hasHost(LOCAL_HOST)).toBe(true);
  });

  it("해제한 호스트는 조회가 던진다 — 로컬로 조용히 흘러가지 않는다", () => {
    const mini = onHost("mini1");
    registerTransport(mini);
    expect(getTransport("mini1")).toBe(mini);

    unregisterTransport("mini1");

    expect(hasHost("mini1")).toBe(false);
    expect(() => getTransport("mini1")).toThrow(/mini1/);
    expect(getTransport(LOCAL_HOST)).toBe(transport);
  });

  it("listHosts는 등록 순서와 무관하게 로컬을 먼저 준다", () => {
    registerTransport(onHost("zulu"));
    registerTransport(onHost("alpha"));

    expect(listHosts()).toEqual([LOCAL_HOST, "alpha", "zulu"]);
  });

  it("taskKey는 같은 id라도 호스트가 다르면 다른 키다", () => {
    expect(taskKey({ host: LOCAL_HOST, id: 3 })).not.toBe(taskKey({ host: "mini1", id: 3 }));
    expect(taskKey({ host: "mini1", id: 3 })).toBe("mini1:3");
  });
});
