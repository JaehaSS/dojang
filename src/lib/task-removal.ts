export interface TaskRemovalTarget {
  state: string;
  /** 워크트리가 이미 사라진 작업 — 정리가 지우는 대상이 달라진다(`removalDetail` 참고). */
  worktree_missing?: boolean;
  /**
   * 아래 셋은 선택이다 — 배너가 "무엇이 남는가"를 이름으로 말하는 데만 쓰이고, 없으면
   * 상태만 보고 뭉뚱그린 문장으로 되돌아간다. 삭제 경로 전체에 이 셋을 강제하면
   * 종료 상태 이력처럼 워크트리와 무관한 대상까지 채워야 한다.
   */
  repo?: string;
  branch?: string;
  worktree_path?: string;
}

export interface TaskRemovalSubject extends TaskRemovalTarget {
  instruction: string;
  branch: string;
}

export interface TaskRemovalActions {
  kind: "local" | "remote";
  cancel(): Promise<void>;
  discard(): Promise<void>;
  deleteHistory(): Promise<void>;
  rejectPending(): Promise<void>;
}

const STOPPABLE_STATES: ReadonlySet<string> = new Set(["Created", "Queued", "Running"]);

/**
 * 삭제를 확정하기 전에 붙잡아 두는 시간. Delete는 실행 중인 세션이면 중단까지 포함하는데,
 * 그 파괴를 키를 누른 순간에 저지르면 되돌릴 방법이 없다 — 이 창이 열려 있는 동안은
 * 아무것도 건드리지 않으므로 ⌘Z 한 번으로 세션이 그대로 돌아온다.
 */
export const REMOVAL_GRACE_MS = 10_000;

/**
 * 무엇이 사라지는지 한 문장. 상태마다 다르므로 뭉뚱그리지 않는다 — 진행 중인 작업은 "중단"까지
 * 포함하고, 검토 대기는 아직 안 본 변경을 버린다.
 *
 * **`worktree_missing`은 상태를 가로채지 못한다.** 워크트리가 없을 때 브랜치의 운명이 상태마다
 * 정반대이기 때문이다 — 폐기는 브랜치를 남기고, 승인 거부만 지운다(설계 0056).
 * - 폐기(`AwaitingReview`, 그리고 중단 후 같은 `task_discard`를 타는 실행 중 상태):
 *   `Worktree::preserve_and_retire()`가 워크트리 부재를 `RetirementTarget::Missing`으로 보고
 *   `git worktree prune`만 한다. `branch -D`는 없다 — `src-tauri/src/worktree/mod.rs:794-799,
 *   815-827`, 단언은 같은 파일 1115행 `retiring_an_orphan_keeps_the_branch_that_discard_would_delete`.
 * - 승인 거부: `task_run_reject` → `reject_pending_task`가 파괴적 `Worktree::discard()`를 부른다
 *   (`src-tauri/src/commands.rs:5574`). 브랜치가 함께 사라지는 곳은 여기 하나뿐이다.
 * - 종료 상태는 `is_orphan`(`commands.rs:5637`)이 건너뛰므로 `worktree_missing`이 참이 되지 않고,
 *   `task_delete`는 워크트리도 브랜치도 건드리지 않는다.
 *
 * 확인 모달이 사라진 지금 이 한 줄이 "무엇이 남는가"의 유일한 창구다. 거짓이면 전달 내용이 전부 거짓이다.
 */
export function removalDetail(task: TaskRemovalTarget): string {
  if (task.state === "AwaitingReview") {
    // 버리기는 확인 모달을 잃었다. 모달이 유일하게 담고 있던 "무엇이 남는가"를 여기서 갚는다 —
    // 워크트리가 없으면 커밋할 것도 지울 것도 없고, 직접 실행은 지울 워크트리가 애초에 없으며,
    // 격리 작업은 브랜치가 남아 되찾을 지점이 된다(설계 0056).
    if (task.worktree_missing) {
      return "워크트리는 이미 없습니다. 커밋된 작업물이 남은 브랜치는 그대로 둡니다.";
    }
    if (task.repo && task.worktree_path && task.repo === task.worktree_path) {
      return "직접 실행이라 지울 워크트리가 없습니다. 메인 체크아웃의 변경은 그대로 남습니다.";
    }
    if (task.branch) {
      return `검토하지 않은 변경을 버리고 워크트리를 정리합니다. 브랜치 ${task.branch}는 남습니다.`;
    }
    return "검토하지 않은 변경을 버리고 워크트리를 정리합니다.";
  }
  if (task.state === "PendingApproval") {
    // 워크트리가 없으면 남은 브랜치가 커밋된 작업물의 **유일한** 사본인데, 거부는 그것까지 지운다.
    if (task.worktree_missing) {
      return "워크트리는 이미 없습니다. 커밋된 작업물이 남은 브랜치까지 함께 삭제됩니다.";
    }
    return "승인 대기 중인 요청을 거부하고 워크트리를 정리합니다.";
  }
  if (STOPPABLE_STATES.has(task.state)) {
    if (task.worktree_missing) {
      return "진행 중인 작업을 중단합니다. 워크트리는 이미 없고, 커밋된 작업물이 남은 브랜치는 그대로 둡니다.";
    }
    return "진행 중인 작업을 중단하고 워크트리를 정리합니다.";
  }
  return "세션 이력과 Dojang이 보관한 대화 원본·첨부를 삭제합니다.";
}

/** 유예 배너에 세우는 이름 — 지시문이 비어 있는 작업은 브랜치로 가린다. */
export function removalTitle(task: TaskRemovalSubject): string {
  return task.instruction.trim() || task.branch;
}

/**
 * 유예 배너의 제목. 워크트리를 걷어내는 상태는 "버리기", 종료 상태 이력은 "삭제"다.
 *
 * 사용자가 누른 버튼·메뉴가 "버리기"라고 말했는데 배너가 "삭제"라고 하면 같은 일인지 알 수 없다 —
 * `TaskNavigationMenu`가 상태에 따라 라벨을 가르는 것과 같은 갈래를 여기서도 쓴다.
 */
export function removalHeadline(task: TaskRemovalSubject): string {
  const discarding =
    task.state === "AwaitingReview"
    || task.state === "PendingApproval"
    || STOPPABLE_STATES.has(task.state);
  return `"${removalTitle(task)}" ${discarding ? "버리기" : "삭제"}`;
}

/**
 * 배선이 요구하는 transport의 최소 형태. `Transport`가 구조적으로 이를 만족하므로 전체를
 * 끌어오지 않고도 잇을 수 있고, 테스트는 네 메서드만 가진 스텁으로 호출을 관측한다.
 */
export interface TaskRemovalTransport {
  kind: "local" | "remote";
  taskCancel(id: number): Promise<void>;
  taskDiscard(id: number): Promise<void>;
  taskDelete(id: number): Promise<void>;
  taskRunReject(id: number): Promise<void>;
}

/**
 * 삭제 액션을 실제 IPC에 잇는 **유일한** 지점.
 *
 * 화면에서 직접 배선하면 각 액션이 어떤 IPC를 부르는지가 테스트 밖에 놓인다. 실제로 그렇게
 * 어긋난 적이 있다(#239): `cancel`이 `taskCancel` 대신 `convoInterrupt`로 바뀌어,
 * 중단은 요청됐지만 `AwaitingReview` 전이를 기다리지 않은 채 `taskDiscard`가 날아가 서버가
 * 거부했다. `removeTask`의 단위 테스트는 액션을 mock으로 받으므로 이 어긋남을 볼 수 없었다.
 *
 * 각 액션이 서로 다른 IPC인 데에는 이유가 있다 — 바꾸기 전에 확인할 것.
 * - `cancel`: `task_cancel`은 프로세스를 죽인 뒤 **`AwaitingReview` 전이까지 기다린다.**
 *   `discard`의 전제조건을 만드는 것이 이 대기다. `convo_interrupt`는 kill만 하고 즉시 돌아온다.
 * - `rejectPending`: `task_run_reject`는 `PendingApproval` 전용이다. `task_discard`는
 *   `AwaitingReview`만 받으므로 승인 대기 작업에는 쓸 수 없다.
 * - `deleteHistory`: 종료 상태 이력을 DB에서 지운다. 화면 상태만 걸러내면 다음 갱신에 되살아난다.
 */
export function transportRemovalActions(
  transport: TaskRemovalTransport,
  id: number,
): TaskRemovalActions {
  return {
    kind: transport.kind,
    cancel: () => transport.taskCancel(id),
    discard: () => transport.taskDiscard(id),
    deleteHistory: () => transport.taskDelete(id),
    rejectPending: () => transport.taskRunReject(id),
  };
}

export async function removeTask(
  task: TaskRemovalTarget,
  actions: TaskRemovalActions,
): Promise<void> {
  if (task.state === "AwaitingReview") {
    await actions.discard();
    return;
  }
  if (task.state === "PendingApproval") {
    await actions.rejectPending();
    return;
  }
  if (STOPPABLE_STATES.has(task.state)) {
    await actions.cancel();
    if (actions.kind === "local") {
      await actions.discard();
    } else {
      await actions.deleteHistory();
    }
    return;
  }
  if (task.state === "Finalizing") {
    throw new Error("완료 처리 중인 작업은 처리가 끝난 뒤 정리할 수 있습니다.");
  }
  await actions.deleteHistory();
}
