import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PraxisTransport } from "../transport";
import { RunnerTransport } from "./runner";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import {
  taskVerify,
  verifySpec,
} from "../ipc";
import {
  LOCAL_HOST,
  captureReviewTransportSession,
  getTransportRevision,
  listHosts,
  registerTransport,
  subscribeTransportChange,
  unregisterTransport,
} from "../transport";

const token = "ab".repeat(32);

/** 호스트만 다른 리뷰 transport 스텁. */
const reviewStub = (hostId: string) =>
  ({
    kind: "remote",
    hostId,
    verifySpec: vi.fn().mockResolvedValue({ preview_token: `preview-${hostId}` }),
    taskVerify: vi.fn().mockResolvedValue({ ready: true }),
  }) as unknown as PraxisTransport;

describe("review transport ownership", () => {
  beforeEach(() => {
    invoke.mockReset();
    // 레지스트리는 모듈 전역이다 — 케이스가 남긴 원격 호스트를 걷어낸다.
    listHosts()
      .filter((host) => host !== LOCAL_HOST)
      .forEach(unregisterTransport);
  });

  it("routes every IPC review operation only through the selected transport", async () => {
    const remote = {
      kind: "remote",
      hostId: "mini1",
      verifySpec: vi.fn().mockResolvedValue({ preview_token: "preview-1" }),
      taskVerify: vi.fn().mockResolvedValue({ ready: true }),
    } as unknown as PraxisTransport;
    registerTransport(remote);
    const session = captureReviewTransportSession("mini1");

    await verifySpec(7, session);
    await taskVerify(7, "preview-1", session);

    expect(remote.verifySpec).toHaveBeenCalledWith(7);
    expect(remote.taskVerify).toHaveBeenCalledWith(7, "preview-1");
    expect(invoke).not.toHaveBeenCalled();
  });

  // 같은 호스트가 재연결된 경우다 — 터널이 새로 열리면 그 이전에 뜬 preview_token은
  // 새 프로세스에서 유효하다는 보장이 없다. 다른 호스트로 기본값이 옮겨간 경우는
  // 아래 "다른 호스트의 연결 해제가…" 케이스가 따로 고정한다.
  it("rejects a review session after the selected host is replaced", async () => {
    const origin = {
      kind: "remote",
      hostId: "mini1",
      verifySpec: vi.fn().mockResolvedValue({ preview_token: "same-token" }),
      taskVerify: vi.fn().mockResolvedValue({ ready: true }),
    } as unknown as PraxisTransport;
    const replacement = {
      kind: "remote",
      hostId: "mini1",
      taskVerify: vi.fn().mockResolvedValue({ ready: true }),
    } as unknown as PraxisTransport;
    const onChange = vi.fn();
    const unsubscribe = subscribeTransportChange(onChange);
    const initialRevision = getTransportRevision();
    registerTransport(origin);
    const session = captureReviewTransportSession("mini1");

    await verifySpec(7, session);
    registerTransport(replacement);

    await expect(taskVerify(7, "same-token", session)).rejects.toThrow("호스트");
    expect(origin.taskVerify).not.toHaveBeenCalled();
    expect(replacement.taskVerify).not.toHaveBeenCalled();
    expect(onChange).toHaveBeenCalledTimes(2);
    expect(getTransportRevision()).toBe(initialRevision + 2);
    unsubscribe();
  });

  it("다른 호스트의 연결 해제가 이 호스트의 리뷰 세션을 무효화하지 않는다", async () => {
    const mini = reviewStub("mini1");
    const box = reviewStub("box2");
    registerTransport(mini);
    registerTransport(box);
    const session = captureReviewTransportSession("mini1");

    await verifySpec(7, session);
    // 무관한 호스트가 내려간다 — 전역 revision 하나로 판정하면 여기서 mini1 세션이 죽는다.
    unregisterTransport("box2");

    await expect(taskVerify(7, "preview-mini1", session)).resolves.toEqual({ ready: true });
    expect(mini.taskVerify).toHaveBeenCalledWith(7, "preview-mini1");
    unregisterTransport("mini1");
  });

  it("그 호스트가 내려가면 세션이 무효가 된다", async () => {
    const mini = reviewStub("mini1");
    registerTransport(mini);
    const session = captureReviewTransportSession("mini1");

    unregisterTransport("mini1");

    await expect(taskVerify(7, "preview-mini1", session)).rejects.toThrow("호스트");
    expect(mini.taskVerify).not.toHaveBeenCalled();
  });

  it("같은 이름으로 다시 붙으면 이전 세션은 무효가 된다", async () => {
    registerTransport(reviewStub("mini1"));
    const session = captureReviewTransportSession("mini1");

    // 재연결 — 터널이 새로 열려 다른 인스턴스가 같은 이름을 차지한다.
    const reconnected = reviewStub("mini1");
    registerTransport(reconnected);

    await expect(taskVerify(7, "preview-mini1", session)).rejects.toThrow("호스트");
    expect(reconnected.taskVerify).not.toHaveBeenCalled();
    unregisterTransport("mini1");
  });

  it("로컬은 원격이 오갈 때도 살아 있다", () => {
    registerTransport(reviewStub("mini1"));
    expect(listHosts()).toContain(LOCAL_HOST);

    unregisterTransport("mini1");

    expect(listHosts()).toEqual([LOCAL_HOST]);
  });

  it("maps review operations to the authenticated Runner API", async () => {
    const request = vi.fn(async (_url: RequestInfo | URL, _init?: RequestInit) => {
      return new Response("null", {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    });
    const transport = new RunnerTransport(
      { endpoint: "http://127.0.0.1:49123", pairingToken: token },
      request,
    );

    await transport.verifySpec(7);
    await transport.taskVerify(7, "preview-1");

    expect(request.mock.calls.map(([url]) => String(url))).toEqual([
      "http://127.0.0.1:49123/v1/tasks/7/verify/spec",
      "http://127.0.0.1:49123/v1/tasks/7/verify",
    ]);
    expect(request.mock.calls[1][1]).toMatchObject({ method: "POST" });
    expect(JSON.parse(String(request.mock.calls[1][1]?.body))).toEqual({
      preview_token: "preview-1",
    });
  });
});
