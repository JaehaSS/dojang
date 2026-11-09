import { invoke } from "@tauri-apps/api/core";
import type { Task } from "./ipc";
import { LOCAL_HOST } from "./transport";

/**
 * 사용자가 작업 목록에서 숨길 때 고정하는 복구 단서다. 보관은 작업 이력이나 워크트리를 지우지
 * 않으며, `worktree_exists`는 목록을 읽은 당시의 관측값이다.
 */
export interface TaskArchive {
  archived_at: number;
  branch: string;
  head_revision: string | null;
  worktree_path: string;
  worktree_exists: boolean;
  result_path: string | null;
  pr_url: string | null;
}

/** 로컬 작업을 보관한다. Runner에는 이 명령이 없으므로 host 좌표를 받지 않는다. */
export const taskArchive = (id: number) => invoke<TaskArchive>("task_archive", { id });

/** 로컬 보관 표지만 없앤다. 워크트리를 만들거나 작업을 재실행하지 않는다. */
export const taskRestore = (id: number) => invoke<void>("task_restore", { id });

type ArchivedTask = Task & { archive: TaskArchive };

const ARCHIVABLE_STATES = new Set(["AwaitingReview", "Done", "Failed", "Discarded"]);

/** 보관된 행만 좁힌다. id는 호스트마다 독립이므로 행 자신의 archive만 판정한다. */
export const isTaskArchived = (task: Task): task is ArchivedTask => task.archive != null;

/**
 * 로컬에서 끝난 작업만 수동 보관할 수 있다. 대화 프로세스 그룹이 남아 있으면 종료 경합을 피하기
 * 위해 보관을 제시하지 않는다.
 */
export const canArchiveTask = (task: Task): boolean =>
  task.host === LOCAL_HOST
  && !task.stale
  && task.archive == null
  && ARCHIVABLE_STATES.has(task.state)
  && task.convo_pgid == null;
