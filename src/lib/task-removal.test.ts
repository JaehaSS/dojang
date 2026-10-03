import { describe, expect, it, vi } from "vitest";
import {
  removalDetail,
  removalHeadline,
  removeTask,
  transportRemovalActions,
  type TaskRemovalActions,
  type TaskRemovalTransport,
} from "./task-removal";

const actions = (kind: "local" | "remote" = "local"): TaskRemovalActions => ({
  kind,
  cancel: vi.fn(async () => {}),
  discard: vi.fn(async () => {}),
  deleteHistory: vi.fn(async () => {}),
  rejectPending: vi.fn(async () => {}),
});

describe("removeTask", () => {
  it("로컬 실행 중 작업은 중단 완료 후 워크트리를 버린다", async () => {
    const calls: string[] = [];
    const deps = actions();
    deps.cancel = vi.fn(async () => {
      calls.push("cancel");
    });
    deps.discard = vi.fn(async () => {
      calls.push("discard");
    });

    await removeTask({ state: "Running" }, deps);

    expect(calls).toEqual(["cancel", "discard"]);
    expect(deps.deleteHistory).not.toHaveBeenCalled();
  });

  it("검토 대기는 바로 버리고 완료 이력은 영구 삭제한다", async () => {
    const reviewActions = actions();
    await removeTask({ state: "AwaitingReview" }, reviewActions);
    expect(reviewActions.discard).toHaveBeenCalledOnce();

    const doneActions = actions();
    await removeTask({ state: "Done" }, doneActions);
    expect(doneActions.deleteHistory).toHaveBeenCalledOnce();
  });

  it("실행 전 승인 대기는 전용 거부 경로로 정리한다", async () => {
    const deps = actions();

    await removeTask({ state: "PendingApproval" }, deps);

    expect(deps.rejectPending).toHaveBeenCalledOnce();
    expect(deps.cancel).not.toHaveBeenCalled();
  });
});

/**
 * 액션을 mock으로 받는 위 테스트들은 "무엇을 부르는가"만 보고 "어디에 이어져 있는가"는 보지 못한다.
 * 배선이 어긋나 삭제가 서버에서 거부됐던 회귀(#239)가 여기를 통과했으므로, IPC 대응을 못 박는다.
 */
describe("transportRemovalActions", () => {
  const transport = (kind: "local" | "remote" = "local") => {
    const calls: string[] = [];
    const stub: TaskRemovalTransport = {
      kind,
      taskCancel: vi.fn(async () => void calls.push("taskCancel")),
      taskDiscard: vi.fn(async () => void calls.push("taskDiscard")),
      taskDelete: vi.fn(async () => void calls.push("taskDelete")),
      taskRunReject: vi.fn(async () => void calls.push("taskRunReject")),
    };
    return { stub, calls };
  };

  it("실행 중 작업은 taskCancel로 검토 대기 전이를 기다린 뒤 taskDiscard한다", async () => {
    const { stub, calls } = transport();

    await removeTask({ state: "Running" }, transportRemovalActions(stub, 7));

    // 순서가 곧 계약이다 — taskDiscard는 AwaitingReview만 받으므로 taskCancel의 대기가 선행해야 한다.
    expect(calls).toEqual(["taskCancel", "taskDiscard"]);
    expect(stub.taskCancel).toHaveBeenCalledWith(7);
    expect(stub.taskDiscard).toHaveBeenCalledWith(7);
  });

  it("승인 대기는 taskDiscard가 아니라 전용 taskRunReject로 거부한다", async () => {
    const { stub } = transport();

    await removeTask({ state: "PendingApproval" }, transportRemovalActions(stub, 3));

    expect(stub.taskRunReject).toHaveBeenCalledWith(3);
    expect(stub.taskDiscard).not.toHaveBeenCalled();
  });

  it("종료 상태 이력은 taskDelete로 서버에서 지운다", async () => {
    const { stub } = transport();

    await removeTask({ state: "Done" }, transportRemovalActions(stub, 11));

    expect(stub.taskDelete).toHaveBeenCalledWith(11);
  });

  it.each(["Created", "Queued", "Running"])(
    "원격 %s 작업은 taskCancel 완료 뒤 taskDelete로 Runner 이력도 지운다",
    async (state) => {
      const { stub, calls } = transport("remote");

      await removeTask({ state }, transportRemovalActions(stub, 5));

      expect(calls).toEqual(["taskCancel", "taskDelete"]);
      expect(stub.taskCancel).toHaveBeenCalledWith(5);
      expect(stub.taskDelete).toHaveBeenCalledWith(5);
    },
  );
});

describe("removalDetail", () => {
  /**
   * 워크트리 부재는 상태를 가로채지 못한다 — 폐기(preserve_and_retire)는 브랜치를 남기고
   * 승인 거부(Worktree::discard)만 지운다. 한 문장으로 뭉치면 절반이 거짓이 된다.
   */
  it("검토 대기는 워크트리가 없어도 브랜치를 남긴다 — preserve_and_retire는 prune만 한다", () => {
    expect(
      removalDetail({
        state: "AwaitingReview",
        worktree_missing: true,
        branch: "praxis/fix-login-1724",
      }),
    ).toBe("워크트리는 이미 없습니다. 커밋된 작업물이 남은 브랜치는 그대로 둡니다.");
  });

  it.each(["Created", "Queued", "Running"])(
    "%s도 중단 뒤 같은 폐기 경로를 타므로 워크트리가 없어도 브랜치를 남긴다",
    (state) => {
      const detail = removalDetail({ state, worktree_missing: true });
      expect(detail).toContain("중단");
      expect(detail).toContain("브랜치는 그대로 둡니다");
    },
  );

  it("승인 거부만 브랜치까지 지운다 — reject_pending_task는 파괴적 discard를 부른다", () => {
    expect(removalDetail({ state: "PendingApproval", worktree_missing: true })).toBe(
      "워크트리는 이미 없습니다. 커밋된 작업물이 남은 브랜치까지 함께 삭제됩니다.",
    );
  });

  it("종료 상태는 워크트리도 브랜치도 건드리지 않는다", () => {
    expect(removalDetail({ state: "Done" })).toBe("세션 이력과 Dojang이 보관한 대화 원본·첨부를 삭제합니다.");
  });

  it("직접 실행에는 지울 워크트리를 그리지 않는다 — 문구가 거짓이면 아무도 안 읽는다", () => {
    const detail = removalDetail({
      state: "AwaitingReview",
      repo: "/w/praxis",
      branch: "dev",
      worktree_path: "/w/praxis",
    });
    expect(detail).toContain("지울 워크트리가 없습니다");
    expect(detail).not.toContain("정리합니다");
  });

  it("격리 작업은 남는 브랜치를 이름으로 말한다 — 모달이 담던 회수 가능성을 배너가 갚는다", () => {
    expect(
      removalDetail({
        state: "AwaitingReview",
        repo: "/w/praxis",
        branch: "praxis/fix-login-1724",
        worktree_path: "/w/praxis/.praxis/worktrees/praxis-fix-login-1724",
      }),
    ).toBe("검토하지 않은 변경을 버리고 워크트리를 정리합니다. 브랜치 praxis/fix-login-1724는 남습니다.");
  });

  it("브랜치도 경로도 모르면 상태별 안내로 되돌아간다", () => {
    expect(removalDetail({ state: "AwaitingReview", worktree_missing: false })).toBe(
      "검토하지 않은 변경을 버리고 워크트리를 정리합니다.",
    );
  });
});

describe("removalHeadline", () => {
  const subject = { instruction: "로그인 고치기", branch: "praxis/fix-login-1724" };

  it.each(["AwaitingReview", "PendingApproval", "Created", "Queued", "Running"])(
    "워크트리를 걷어내는 %s 상태는 사용자가 누른 버튼과 같은 '버리기'로 말한다",
    (state) => {
      expect(removalHeadline({ ...subject, state })).toBe('"로그인 고치기" 버리기');
    },
  );

  it("종료 상태 이력은 '삭제'다 — 되돌릴 워크트리가 없다", () => {
    expect(removalHeadline({ ...subject, state: "Done" })).toBe('"로그인 고치기" 삭제');
  });
});
