import { describe, expect, it, vi } from "vitest";
import { RunnerTransport, type RunnerWebSocket } from "./runner";

const token = "ab".repeat(32);

describe("RunnerTransport host tagging", () => {
  const profiled = (request: unknown) =>
    new RunnerTransport(
      { endpoint: "http://127.0.0.1:49123", pairingToken: token, profileName: "mini1" },
      request as never,
    );

  it("hostId는 프로필 이름이고, 이름이 없으면 endpoint에서 파생한다", () => {
    expect(profiled(vi.fn()).hostId).toBe("mini1");
    // 원격이 로컬 슬롯을 차지하면 로컬 작업이 통째로 사라진다 — 절대 "local"이 아니어야 한다.
    const anonymous = new RunnerTransport({ endpoint: "http://127.0.0.1:49123/", pairingToken: token });
    expect(anonymous.hostId).not.toBe("local");
    expect(anonymous.hostId).toBe("runner:http://127.0.0.1:49123");
  });

  it("taskList의 모든 행에 host가 붙는다", async () => {
    const request = vi.fn(async () => json([{ id: 3, mode: "conversation" }, { id: 9, mode: "terminal" }]));

    await expect(profiled(request).taskList()).resolves.toEqual([
      expect.objectContaining({ id: 3, host: "mini1" }),
      expect.objectContaining({ id: 9, host: "mini1" }),
    ]);
  });

  it("taskCreate 단건 반환에도 붙는다 — 생성 직후 선택이 host를 잃지 않는다", async () => {
    const request = vi.fn(async () => json({ id: 12, mode: "conversation" }));

    await expect(
      profiled(request).taskCreate({
        host: "mini1",
        repo: "/repo",
        instruction: "x",
        agent: "claude",
        role: "implementer",
        model: "",
        headless: false,
        ensemble: "",
        mode: "conversation",
        cmd: "",
        args: [],
        cols: 80,
        rows: 24,
      }),
    ).resolves.toMatchObject({ id: 12, host: "mini1" });
  });

  it("서버 응답에 host가 섞여 와도 transport 값이 이긴다", async () => {
    const request = vi.fn(async () => json([{ id: 3, host: "누군가의-호스트", mode: "terminal" }]));

    const [task] = await profiled(request).taskList();
    expect(task.host).toBe("mini1");
  });
});

describe("RunnerTransport", () => {
  it("normalizes session vendors and preview capability from the sessions response", async () => {
    const entry = { session_id: "same", cwd: "/repo", last_cwd: null, git_branch: null, title: null, first_message: null, last_active: 0, messages: 1, vendor_version: null };
    const request = vi.fn(async () => json({ sessions: [{ ...entry }, { ...entry, vendor: "codex" }, { ...entry, vendor: "unknown-provider" }], capabilities: { vendors: ["claude", "codex"], preview: true } }));
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token, profileName: "mini1" }, request);

    await expect(transport.sessionHomeIndex("/repo", false)).resolves.toMatchObject([
      { host: "mini1", vendor: "claude", preview_supported: true },
      { host: "mini1", vendor: "codex", preview_supported: true },
    ]);
  });

  it("treats a legacy Runner sessions response as Claude-only with no preview", async () => {
    const request = vi.fn(async () => json({ sessions: [] }));
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token, profileName: "mini1" }, request);

    await transport.sessionHomeIndex("/repo", false);
    await expect(transport.sessionHomePreview("/repo", "claude", "old")).rejects.toThrow("미리보기");
  });

  it("loads Runner capabilities before submitting a Codex resume and sends resume_vendor", async () => {
    const request = vi.fn(async (url: RequestInfo | URL, _init?: RequestInit) =>
      String(url).includes("/v1/sessions?")
        ? json({ sessions: [], capabilities: { vendors: ["claude", "codex"], preview: true } })
        : json({ id: 4, mode: "conversation" }),
    );
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token, profileName: "mini1" }, request);
    await transport.taskCreate({ host: "mini1", repo: "/repo", instruction: "x", agent: "codex", role: "implementer", model: "", headless: false, ensemble: "", mode: "conversation", cmd: "", args: [], cols: 80, rows: 24, resumeSession: "thread", resumeVendor: "codex" });

    expect(request.mock.calls).toHaveLength(2);
    expect(JSON.parse(String(request.mock.calls[1][1]?.body))).toMatchObject({ resume_session: "thread", resume_vendor: "codex" });
  });

  it("scopes repair requests to the remote task and session", async () => {
    const request = vi.fn(async (_input: unknown, _init?: RequestInit) => json(null));
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token, profileName: "mini1" }, request as never);
    await transport.approvalRepairStatus(7); await transport.approvalRepairPrepare(7);
    await transport.approvalRepairRun(7,"repair"); await transport.approvalRepairAccept(7,"repair");
    expect(request.mock.calls.map(call => String(call[0]))).toEqual(["/approval-repair","/approval-repair","/approval-repair/run","/approval-repair/accept"].map(path => `http://127.0.0.1:49123/v1/tasks/7${path}`));
    expect(JSON.parse(request.mock.calls[2][1]!.body as string)).toEqual({session_id:"repair"});
  });
  it("reads approval status on the selected host and preserves unsupported errors", async () => {
    const payload = { readiness: null, inspection_error: "missing worktree", attempts: [] };
    const request = vi.fn(async (_input: unknown, _init?: unknown) => json(payload));
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token, profileName: "mini1" }, request as never);
    await expect(transport.taskApprovalStatus(7)).resolves.toEqual(payload);
    expect(String(request.mock.calls[0][0])).toContain("/v1/tasks/7/approval-status");
    const unsupported = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token }, vi.fn(async () => new Response("not found", { status: 404 })) as never);
    await expect(unsupported.taskApprovalStatus(7)).rejects.toThrow();
  });

  it("summarizes remote file patches with the same diff-stat contract as local mode", async () => {
    const request = vi.fn(async () =>
      json({
        files: [
          {
            path: "src/app.ts",
            status: "M",
            patch: "@@ -1 +1,2 @@\n-old\n+new\n+next",
          },
        ],
        baseline: { kind: "pinned" },
      }),
    );
    const transport = new RunnerTransport(
      { endpoint: "http://127.0.0.1:49123/", pairingToken: token },
      request,
    );

    await expect(transport.taskDiffStat(7)).resolves.toContain(
      "1 file changed, 2 insertions(+), 1 deletions(-)",
    );
  });

  it("구버전 Runner가 돌려준 배열 응답도 files/baseline 모양으로 받는다", async () => {
    // 기준점 도입 전의 Runner는 `/v1/tasks/:id/diff`가 배열을 그대로 돌려준다. 200에 정상 JSON이라
    // 오류로 걸러지지 않고, 그대로 두면 화면이 `files === undefined`를 받아 변경 목록에서 죽는다.
    const file = { path: "src/app.ts", status: "M", patch: "@@ -1 +1 @@\n-old\n+new" };
    const request = vi.fn(async () => json([file]));
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token }, request);

    await expect(transport.taskDiff(7)).resolves.toEqual({
      files: [file],
      baseline: { kind: "legacy" },
    });
  });

  it("diff 응답에 files가 없어도 빈 목록으로 떨어진다 — 화면은 절대 undefined를 보지 않는다", async () => {
    const request = vi.fn(async () => json({ baseline: { kind: "pinned" } }));
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token }, request);

    await expect(transport.taskDiff(7)).resolves.toEqual({
      files: [],
      baseline: { kind: "pinned" },
    });
  });

  it("requests an atomic review snapshot with the selected range", async () => {
    const request = vi.fn(async (_input: RequestInfo | URL) => json({ files: [], baseline: { kind: "pinned" } }));
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token }, request);

    await transport.taskDiff(7, "uncommitted", true);

    const url = new URL(String(request.mock.calls[0][0]));
    expect(url.pathname).toBe("/v1/tasks/7/diff");
    expect(url.searchParams.get("range")).toBe("uncommitted");
    expect(url.searchParams.get("include_review")).toBe("true");
  });

  it("adds a bearer token to HTTP calls and maps task-backed file reads", async () => {
    const request = vi.fn(async (url: RequestInfo | URL, init?: RequestInit) => {
      const value = String(url);
      expect(init?.headers).toEqual({ Authorization: `Bearer ${token}` });
      if (value.endsWith("/v1/tasks/7")) return json({ id: 7, worktree_path: "/repo/worktree" });
      if (value.includes("/v1/files/read?")) return json({ kind: "text", content: "ok", mtime: 1 });
      return json([]);
    });
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123/", pairingToken: token }, request);

    await expect(transport.fsRead(7, "src/main.rs")).resolves.toMatchObject({ content: "ok" });
    expect(String(request.mock.calls[1][0])).toContain("repository=%2Frepo%2Fworktree");
    expect(String(request.mock.calls[1][0])).toContain("path=src%2Fmain.rs");
  });

  it("browses a directory through the browse endpoint", async () => {
    const request = vi.fn(async (_url: RequestInfo | URL) =>
      json({ path: "/home/u/work", parent: "/home/u", entries: [] }),
    );
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123/", pairingToken: token }, request);

    await expect(transport.fsBrowse("/home/u/work")).resolves.toMatchObject({ parent: "/home/u" });
    expect(String(request.mock.calls[0][0])).toContain("path=%2Fhome%2Fu%2Fwork");
  });

  it.each([
    ["/srv/reports/한글 report.html", "/srv/reports", "한글 report.html"],
    ["/report.md", "/", "report.md"],
    ["C:\\reports\\note.md", "C:/reports", "note.md"],
    ["C:/note.md", "C:/", "note.md"],
  ])("reads the remote absolute path %s through the existing authorized API", async (path, repository, name) => {
    const request = vi.fn(async (_url: RequestInfo | URL, _init?: RequestInit) =>
      json({ kind: "text", content: "remote document", mtime: 1 }));
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token }, request);

    await expect(transport.fsRead(7, path)).resolves.toMatchObject({ content: "remote document" });
    expect(request).toHaveBeenCalledOnce();
    const url = new URL(String(request.mock.calls[0][0]));
    expect(url.pathname).toBe("/v1/files/read");
    expect(url.searchParams.get("repository")).toBe(repository);
    expect(url.searchParams.get("path")).toBe(name);
    expect(request.mock.calls[0][1]?.headers).toEqual({ Authorization: `Bearer ${token}` });
    expect(request.mock.calls[0][1]?.signal).toBeInstanceOf(AbortSignal);
  });

  it("rejects malformed absolute paths before sending a request", async () => {
    const request = vi.fn();
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token }, request);
    for (const path of ["/srv/../etc/passwd", "/srv/./note.md", "/srv/", "/srv/..", "/srv/\0.md", "//host/share/file.md"]) {
      await expect(transport.fsRead(7, path)).rejects.toThrow("유효하지 않은 원격 파일 경로입니다");
    }
    expect(request).not.toHaveBeenCalled();
  });

  it.each([403, 404, 500])("keeps a remote read failure (%s) without a fallback to another location", async (status) => {
    const request = vi.fn(async () => new Response("원격 파일을 읽을 수 없습니다", { status }));
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token }, request);
    await expect(transport.fsRead(7, "/srv/report.md")).rejects.toThrow("원격 파일을 읽을 수 없습니다");
    expect(request).toHaveBeenCalledOnce();
  });

  it("reports git status and initializes a folder through the git endpoints", async () => {
    const request = vi.fn(async (url: RequestInfo | URL, init?: RequestInit) => {
      if (String(url).includes("/v1/git/status")) return json({ is_repo: false });
      expect(init?.method).toBe("POST");
      expect(init?.body).toBe(JSON.stringify({ path: "/home/u/plain" }));
      return json({ is_repo: true });
    });
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123/", pairingToken: token }, request);

    await expect(transport.gitStatus("/home/u/plain")).resolves.toBe(false);
    expect(String(request.mock.calls[0][0])).toContain("path=%2Fhome%2Fu%2Fplain");
    await expect(transport.gitInit("/home/u/plain")).resolves.toBe(true);
  });

  it("uses a WebSocket subprotocol for the pairing token without putting it in the URL", () => {
    let socket: RunnerWebSocket | undefined;
    const factory = vi.fn((_url: string, _protocols: string[]) => {
      socket = { close: vi.fn(), onmessage: null, onerror: null };
      return socket;
    });
    const onEvent = vi.fn();
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token }, fetch, factory);

    const unsubscribe = transport.subscribeEvents(12, onEvent, vi.fn());
    expect(factory).toHaveBeenCalledWith("ws://127.0.0.1:49123/v1/events/live?after=12", ["praxis", token]);
    socket?.onmessage?.({ data: JSON.stringify({ sequence: 13, task_id: 7, ts: 1, kind: "output", detail: "x" }) } as MessageEvent<string>);
    expect(onEvent).toHaveBeenCalledWith(expect.objectContaining({ sequence: 13 }));
    unsubscribe();
    expect(socket?.close).toHaveBeenCalledOnce();
  });

  it("replays each event once while ignoring the watermark and stale sequences", () => {
    let socket: RunnerWebSocket | undefined;
    const factory = vi.fn((_url: string, _protocols: string[]) => {
      socket = { close: vi.fn(), onmessage: null, onerror: null };
      return socket;
    });
    const onEvent = vi.fn();
    const onError = vi.fn();
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token }, fetch, factory);

    transport.subscribeEvents(10, onEvent, onError);
    socket?.onmessage?.({ data: JSON.stringify({ kind: "watermark", sequence: 20 }) } as MessageEvent<string>);
    socket?.onmessage?.({ data: JSON.stringify({ sequence: 11, task_id: 7, ts: 1, kind: "output", detail: "a" }) } as MessageEvent<string>);
    socket?.onmessage?.({ data: JSON.stringify({ sequence: 11, task_id: 7, ts: 1, kind: "output", detail: "a" }) } as MessageEvent<string>);
    socket?.onmessage?.({ data: JSON.stringify({ sequence: 10, task_id: 7, ts: 1, kind: "output", detail: "old" }) } as MessageEvent<string>);
    socket?.onmessage?.({ data: "not json" } as MessageEvent<string>);

    expect(onEvent).toHaveBeenCalledTimes(1);
    expect(onEvent).toHaveBeenCalledWith(expect.objectContaining({ sequence: 11, detail: "a" }));
    expect(onError).toHaveBeenCalledOnce();
  });

  it("reconnects from the last received sequence after a socket failure", () => {
    vi.useFakeTimers();
    const sockets: RunnerWebSocket[] = [];
    const factory = vi.fn((_url: string, _protocols: string[]) => {
      const socket: RunnerWebSocket = { close: vi.fn(), onmessage: null, onerror: null, onclose: null };
      sockets.push(socket);
      return socket;
    });
    const onEvent = vi.fn();
    const onError = vi.fn();
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token }, fetch, factory);

    const stop = transport.subscribeEvents(4, onEvent, onError);
    sockets[0].onmessage?.({ data: JSON.stringify({ sequence: 5, task_id: 7, ts: 1, kind: "output", detail: "a" }) } as MessageEvent<string>);
    sockets[0].onerror?.(new Event("error"));
    vi.advanceTimersByTime(1_000);

    expect(factory).toHaveBeenNthCalledWith(2, "ws://127.0.0.1:49123/v1/events/live?after=5", ["praxis", token]);
    expect(onError).toHaveBeenCalledOnce();
    stop();
    vi.useRealTimers();
  });

  it("sends task lifecycle and schedule mutations only to Runner JSON endpoints", async () => {
    const request = vi.fn(async (_url: RequestInfo | URL, _init?: RequestInit) => json(7));
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token }, request);

    await transport.taskCreate({
      host: "mini1",
        repo: "/repo",
      instruction: "ship it",
      agent: "codex",
      role: "reviewer",
      model: "gpt-5.6-sol",
      reasoning_effort: "high",
      headless: true,
      ensemble: "",
      mode: "terminal",
      cmd: "",
      args: [],
      cols: 80,
      rows: 24,
      goal_contract: {
        schema_version: 1,
        objective: "ship with evidence",
        acceptance: ["tests pass"],
        stop_conditions: [],
        must_preserve: [],
        protected_paths: ["deploy/**"],
        non_goals: ["auto merge"],
      },
    });
    await transport.taskInput(5, "ls\r");
    await transport.taskMessage(5, "이어서 테스트도 고쳐줘");
    await transport.taskDelete(6);
    await transport.taskRunApprove(7);
    await transport.taskApprove(8);
    await transport.taskDiscard(9);
    await transport.scheduleAdd({
      label: "daily",
      cron: "0 0 * * * *",
      kind: "task",
      payload: "{}",
      tzOffsetSecs: 0,
    });
    await transport.scheduleSetEnabled(7, false);

    expect(request.mock.calls.map(([url]) => String(url))).toEqual([
      "http://127.0.0.1:49123/v1/tasks",
      "http://127.0.0.1:49123/v1/tasks/5/input",
      "http://127.0.0.1:49123/v1/tasks/5/message",
      "http://127.0.0.1:49123/v1/tasks/6",
      "http://127.0.0.1:49123/v1/tasks/7/run",
      "http://127.0.0.1:49123/v1/tasks/8/approve",
      "http://127.0.0.1:49123/v1/tasks/9/discard",
      "http://127.0.0.1:49123/v1/schedules",
      "http://127.0.0.1:49123/v1/schedules/7",
    ]);
    expect(request.mock.calls[0][1]).toMatchObject({
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
    });
    expect(JSON.parse(String(request.mock.calls[0][1]?.body))).toMatchObject({
      repository: "/repo",
      role: "reviewer",
      reasoning_effort: "high",
      goal_contract: { objective: "ship with evidence", acceptance: ["tests pass"] },
    });
    expect(request.mock.calls[1][1]).toMatchObject({
      method: "POST",
      body: JSON.stringify({ data: "ls\r" }),
    });
    expect(request.mock.calls[2][1]).toMatchObject({
      method: "POST",
      body: JSON.stringify({ message: "이어서 테스트도 고쳐줘" }),
    });
    expect(request.mock.calls[8][1]).toMatchObject({ method: "PUT" });
  });

  it("surfaces the Runner error body so users see the actionable message", async () => {
    const request = vi.fn(async () =>
      new Response("검토 대기 상태의 대화 작업만 이어갈 수 있습니다", { status: 409 }),
    );
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token }, request);

    await expect(transport.taskMessage(5, "hello")).rejects.toThrow(
      "Runner 요청 실패 (409): 검토 대기 상태의 대화 작업만 이어갈 수 있습니다",
    );
  });

  it("rejects taskResume — resume 체인은 로컬 DB 전용이라 Runner에 대응 엔드포인트가 없다", async () => {
    const request = vi.fn();
    const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token }, request);

    await expect(transport.taskResume(5, "이어서 계속해줘")).rejects.toThrow(
      "원격 Runner에서는 대화 이어받기를 지원하지 않습니다",
    );
    expect(request).not.toHaveBeenCalled();
  });

  it("omits goal_contract for legacy clients instead of serializing an undefined field", async () => {
    const request = vi.fn(async (_url: RequestInfo | URL, _init?: RequestInit) => json({ id: 1 }));
    const transport = new RunnerTransport(
      { endpoint: "http://127.0.0.1:49123", pairingToken: token },
      request,
    );

    await transport.taskCreate({
      host: "mini1",
        repo: "/repo",
      instruction: "legacy instruction",
      agent: "claude",
      role: "implementer",
      model: "",
      headless: false,
      ensemble: "",
      mode: "terminal",
      cmd: "",
      args: [],
      cols: 80,
      rows: 24,
    });

    expect(JSON.parse(String(request.mock.calls[0][1]?.body))).toEqual({
      repository: "/repo",
      instruction: "legacy instruction",
      agent: "claude",
      role: "implementer",
      model: "",
      mode: "terminal",
    });
  });

  it("routes quickopen search to Runner", async () => {
    const request = vi.fn(async (_url: RequestInfo | URL, _init?: RequestInit) => json([]));
    const transport = new RunnerTransport(
      { endpoint: "http://127.0.0.1:49123", pairingToken: token },
      request,
    );

    await transport.quickopenSearch("deploy", ["task", "session"]);

    expect(request.mock.calls.map(([url]) => String(url))).toEqual([
      "http://127.0.0.1:49123/v1/quickopen?query=deploy&scopes=task%2Csession",
    ]);
  });

  it("routes diff hunks and annotation operations to Runner with snake_case bodies", async () => {
    const request = vi.fn(async (_url: RequestInfo | URL, _init?: RequestInit) => json({}));
    const transport = new RunnerTransport(
      { endpoint: "http://127.0.0.1:49123", pairingToken: token },
      request,
    );

    await transport.diffHunks(7);
    await transport.annotationsList(7);
    await transport.annotationSave(7, {
      id: "ann-1",
      hunk_id: "h1",
      path: "a.ts",
      line: 11,
      side: "new",
      body_md: "고쳐줘",
    });
    await transport.annotationsResend(7, ["ann-1", "ann-2"]);

    expect(request.mock.calls.map(([url]) => String(url))).toEqual([
      "http://127.0.0.1:49123/v1/tasks/7/diff/hunks",
      "http://127.0.0.1:49123/v1/tasks/7/annotations",
      "http://127.0.0.1:49123/v1/tasks/7/annotations",
      "http://127.0.0.1:49123/v1/tasks/7/annotations/resend",
    ]);
    expect(JSON.parse(String(request.mock.calls[2][1]?.body))).toEqual({
      id: "ann-1",
      hunk_id: "h1",
      path: "a.ts",
      line: 11,
      side: "new",
      body_md: "고쳐줘",
    });
    expect(JSON.parse(String(request.mock.calls[3][1]?.body))).toEqual({ ids: ["ann-1", "ann-2"] });
  });

  it("routes partial apply/rollback to Runner with snake_case bodies", async () => {
    const request = vi.fn(async (_url: RequestInfo | URL, _init?: RequestInit) => json({}));
    const transport = new RunnerTransport(
      { endpoint: "http://127.0.0.1:49123", pairingToken: token },
      request,
    );

    await transport.partialApply(7, ["h1", "h2"]);
    await transport.partialRollback(7);

    expect(request.mock.calls.map(([url]) => String(url))).toEqual([
      "http://127.0.0.1:49123/v1/tasks/7/partial/apply",
      "http://127.0.0.1:49123/v1/tasks/7/partial/rollback",
    ]);
    expect(JSON.parse(String(request.mock.calls[0][1]?.body))).toEqual({ hunk_ids: ["h1", "h2"] });
  });

  it("routes ensemble matrix/compose to Runner with snake_case bodies", async () => {
    const request = vi.fn(async (_url: RequestInfo | URL, _init?: RequestInit) => json({}));
    const transport = new RunnerTransport(
      { endpoint: "http://127.0.0.1:49123", pairingToken: token },
      request,
    );

    await transport.ensembleMatrix("ens-1");
    await transport.ensembleCompose("ens-1", 1, [{ task_id: 2, hunk_id: "h1" }]);

    expect(request.mock.calls.map(([url]) => String(url))).toEqual([
      "http://127.0.0.1:49123/v1/ensembles/ens-1/matrix",
      "http://127.0.0.1:49123/v1/ensembles/ens-1/compose",
    ]);
    expect(JSON.parse(String(request.mock.calls[1][1]?.body))).toEqual({
      winner_task_id: 1,
      selections: [{ task_id: 2, hunk_id: "h1" }],
    });
  });

  it("routes GitHub issue methods to Runner with repository query/body", async () => {
    const request = vi.fn(async (_url: RequestInfo | URL, _init?: RequestInit) => json({}));
    const transport = new RunnerTransport(
      { endpoint: "http://127.0.0.1:49123", pairingToken: token },
      request,
    );

    await transport.githubIssuesList("/repo");
    await transport.githubReposList(["/repo", "/other"]);
    await transport.githubCreateTaskFromIssue("/repo", 42, "claude");
    await transport.githubIssueDelete("/repo", 42);

    expect(request.mock.calls.map(([url]) => String(url))).toEqual([
      "http://127.0.0.1:49123/v1/github/issues?repository=%2Frepo",
      "http://127.0.0.1:49123/v1/github/repos",
      "http://127.0.0.1:49123/v1/github/issues/task",
      "http://127.0.0.1:49123/v1/github/issues?repository=%2Frepo&number=42",
    ]);
    // 삭제는 목록 조회와 같은 경로를 쓴다 — 메서드가 둘을 가르므로 그것까지 못 박는다.
    expect(request.mock.calls[3][1]?.method).toBe("DELETE");
    expect(JSON.parse(String(request.mock.calls[1][1]?.body))).toEqual({
      repositories: ["/repo", "/other"],
    });
    expect(JSON.parse(String(request.mock.calls[2][1]?.body))).toEqual({
      repository: "/repo",
      number: 42,
      agent: "claude",
    });
  });

  it("스킬 목록을 repository 질의로 Runner에 묻는다 — 404는 구버전이라 빈 목록", async () => {
    const request = vi.fn(async (_url: RequestInfo | URL, _init?: RequestInit) =>
      json([{ name: "reviewer", description: "검토" }]),
    );
    const transport = new RunnerTransport(
      { endpoint: "http://127.0.0.1:49123", pairingToken: token },
      request,
    );

    await expect(transport.skillsList("/runner/repo")).resolves.toEqual([
      { name: "reviewer", description: "검토" },
    ]);
    expect(String(request.mock.calls[0][0])).toBe(
      "http://127.0.0.1:49123/v1/skills?repository=%2Frunner%2Frepo",
    );
  });

  it("404는 라우트가 없는 구버전 Runner라 빈 목록이고, 403(roots 밖)은 전파한다", async () => {
    const missing = new RunnerTransport(
      { endpoint: "http://127.0.0.1:49123", pairingToken: token },
      (async () => new Response("", { status: 404 })) as never,
    );
    await expect(missing.skillsList("/runner/repo")).resolves.toEqual([]);

    const forbidden = new RunnerTransport(
      { endpoint: "http://127.0.0.1:49123", pairingToken: token },
      (async () => new Response("허용된 root 밖의 경로입니다", { status: 403 })) as never,
    );
    await expect(forbidden.skillsList("/elsewhere")).rejects.toThrow("허용된 root 밖의 경로입니다");
  });
});

describe("RunnerTransport taskOutput 타임아웃", () => {
  // 드레인 루프가 for(;;) 안에서 이걸 await한다 — SSH 터널이 반쯤 죽으면(TCP는 붙었는데
  // 응답이 없음) 타임아웃 없이는 여기서 영원히 대기한다(원장 #448).
  it("AbortSignal.timeout 신호를 실어 보내고, 타임아웃되면 무한 대기 대신 reject한다", async () => {
    const controller = new AbortController();
    const timeoutSpy = vi.spyOn(AbortSignal, "timeout").mockReturnValue(controller.signal);

    const request = vi.fn((_url: RequestInfo | URL, init?: RequestInit) => {
      // 실제 fetch도 signal이 중단되면 Response 없이 reject한다 — 같은 계약을 흉내낸다.
      return new Promise<Response>((_, reject) => {
        init?.signal?.addEventListener("abort", () => reject(init.signal!.reason));
      });
    });
    const transport = new RunnerTransport(
      { endpoint: "http://127.0.0.1:49123", pairingToken: token },
      request as never,
    );

    const pending = transport.taskOutput(7, 0);
    expect(timeoutSpy).toHaveBeenCalledWith(expect.any(Number));

    controller.abort(new DOMException("타임아웃", "TimeoutError"));

    await expect(pending).rejects.toMatchObject({ name: "TimeoutError" });

    timeoutSpy.mockRestore();
  });
});

function json(value: unknown): Response {
  return new Response(JSON.stringify(value), { status: 200, headers: { "Content-Type": "application/json" } });
}


it("routes isolated questions and immutable receipt payloads to the selected Runner", async () => {
  const request = vi.fn(async (_url: RequestInfo | URL, _init?: RequestInit) => json({}));
  const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token }, request);
  const input = { request_id: "q-1", generation: 2, question: "why?", contexts: [{ label: "selected", text: "only this" }] };
  await transport.sideQuestionRead(7);
  await transport.sideQuestionSend(7, input);
  await transport.sideQuestionCancel(7, 9);
  await transport.sideQuestionReset(7, 2);
  await transport.conversationSubmit(7, "main/1", "apply this", []);
  await transport.conversationReceipt(7, "main/1");
  expect(request.mock.calls.map(([url]) => new URL(String(url)).pathname)).toEqual([
    "/v1/tasks/7/side-question", "/v1/tasks/7/side-question/messages", "/v1/tasks/7/side-question/cancel", "/v1/tasks/7/side-question/reset", "/v1/tasks/7/message-receipts/main%2F1", "/v1/tasks/7/message-receipts/main%2F1",
  ]);
  expect(JSON.parse(String(request.mock.calls[1][1]?.body))).toEqual(input);
  expect(JSON.parse(String(request.mock.calls[4][1]?.body))).toEqual({ message: "apply this", image_paths: [] });
});

it("세션 모델 교체를 고른 Runner의 그 작업으로 보낸다", async () => {
  const request = vi.fn(async (_url: RequestInfo | URL, _init?: RequestInit) => new Response(null, { status: 204 }));
  const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token }, request);

  await transport.taskModelSet(7, "sonnet");
  // 빈 문자열은 해제다 — 필드를 빼면 Runner가 "모델 없음"과 "해제"를 구분하지 못한다.
  await transport.taskModelSet(7, "");

  expect(request.mock.calls.map(([url]) => new URL(String(url)).pathname)).toEqual([
    "/v1/tasks/7/model",
    "/v1/tasks/7/model",
  ]);
  expect(request.mock.calls[0][1]).toMatchObject({ method: "PUT", body: JSON.stringify({ model: "sonnet" }) });
  expect(request.mock.calls[1][1]).toMatchObject({ method: "PUT", body: JSON.stringify({ model: "" }) });
});

it("Runner가 거절한 모델은 그 사유를 그대로 올린다", async () => {
  const request = vi.fn(async () => new Response("모델을 바꿀 수 없는 에이전트입니다: mybin", { status: 400 }));
  const transport = new RunnerTransport({ endpoint: "http://127.0.0.1:49123", pairingToken: token }, request);

  await expect(transport.taskModelSet(7, "sonnet")).rejects.toThrow(
    "Runner 요청 실패 (400): 모델을 바꿀 수 없는 에이전트입니다: mybin",
  );
});
