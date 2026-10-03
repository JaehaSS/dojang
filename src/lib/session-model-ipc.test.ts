import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { debateStart, taskAgentSet, taskModelSet } from "./ipc";
import { registerTransport, unregisterTransport, type PraxisTransport } from "./transport";

const REMOTE_HOST = "session-model-ipc-remote";
const remoteTaskModelSet = vi.fn(async () => undefined);

beforeEach(() => {
  invoke.mockReset();
  remoteTaskModelSet.mockClear();
  unregisterTransport(REMOTE_HOST);
});

const remote = () =>
  registerTransport({
    kind: "remote",
    hostId: REMOTE_HOST,
    taskModelSet: remoteTaskModelSet,
  } as unknown as PraxisTransport);

describe("taskModelSet", () => {
  it("로컬 좌표는 로컬 command로 간다", async () => {
    await taskModelSet({ host: "local", id: 42 }, "opus");

    expect(invoke).toHaveBeenCalledWith("task_model_set", { id: 42, model: "opus" });
  });

  it("원격 좌표는 그 호스트의 transport로 간다 — 같은 번호의 로컬 작업을 고치지 않는다", async () => {
    remote();

    await taskModelSet({ host: REMOTE_HOST, id: 42 }, "sonnet");

    expect(remoteTaskModelSet).toHaveBeenCalledWith(42, "sonnet");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("해제(빈 문자열)도 같은 경로로 간다 — 원격만 벤더 기본으로 못 돌아가면 막다른 길이 된다", async () => {
    remote();

    await taskModelSet({ host: REMOTE_HOST, id: 42 }, "");

    expect(remoteTaskModelSet).toHaveBeenCalledWith(42, "");
  });
});

describe("taskAgentSet", () => {
  it("로컬 좌표는 로컬 command로 간다", async () => {
    invoke.mockResolvedValueOnce({ id: 42 });

    await taskAgentSet({ host: "local", id: 42 }, "codex", "gpt-5.6-terra");

    expect(invoke).toHaveBeenCalledWith("task_agent_set", {
      id: 42,
      agent: "codex",
      model: "gpt-5.6-terra",
    });
  });

  it("원격은 로컬 IPC 전에 거부한다 — Runner에 핸드오프 경로가 없다", async () => {
    remote();

    await expect(
      taskAgentSet({ host: REMOTE_HOST, id: 42 }, "codex", "gpt-5.6-terra"),
    ).rejects.toThrow("원격 세션에서는 에이전트를 바꿀 수 없습니다");
    expect(invoke).not.toHaveBeenCalled();
  });
});

describe("debateStart", () => {
  it("로컬 좌표는 로컬 command로 간다", async () => {
    await debateStart({ host: "local", id: 42 }, [{ agent: "codex" }, { agent: "agy", model: "gemini-3" }]);

    expect(invoke).toHaveBeenCalledWith("debate_start", {
      taskId: 42,
      opponents: [
        { agent: "codex", model: null },
        { agent: "agy", model: "gemini-3" },
      ],
    });
  });

  it("원격은 자리를 만들기 전에 거부한다 — 만들면 그 세션이 다음 턴부터 실행 불가가 된다", async () => {
    remote();

    await expect(debateStart({ host: REMOTE_HOST, id: 42 }, [{ agent: "codex" }])).rejects.toThrow(
      "원격 세션에서는 토론을 시작할 수 없습니다",
    );
    expect(invoke).not.toHaveBeenCalled();
  });
});
