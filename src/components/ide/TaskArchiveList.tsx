import { useState, type MouseEvent } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { safeGithubPullRequestUrl } from "../../lib/github-pr-status";
import type { Task } from "../../lib/ipc";
import { isTaskArchived } from "../../lib/task-archive";
import { taskKey } from "../../lib/transport";
import type { TaskNavigationMenuState } from "./TaskNavigationMenu";
import { Icon } from "./icons";

export interface TaskArchiveListProps {
  tasks: Task[];
  selectedKey: string | null;
  onOpenTask: (task: Task) => void;
  onRestoreTask?: (task: Task) => void;
  onOpenMenu?: (menu: TaskNavigationMenuState) => void;
}

const revision = (head: string | null): string => head == null ? "커밋 미확인" : head.slice(0, 12);

/**
 * 보관은 작업의 검색 가능한 이력을 남긴다. 복원은 이 섹션에서만 작업을 되돌리며, 삭제된
 * 워크트리를 다시 만들지 않는다.
 */
export function TaskArchiveList({
  tasks,
  selectedKey,
  onOpenTask,
  onRestoreTask,
  onOpenMenu,
}: TaskArchiveListProps) {
  const [expanded, setExpanded] = useState(true);
  const [openError, setOpenError] = useState<string | null>(null);
  const archived = tasks.filter(isTaskArchived);
  if (archived.length === 0) return null;

  const openMenu = (event: MouseEvent<HTMLDivElement>, task: Task): void => {
    if (!onOpenMenu) return;
    event.preventDefault();
    onOpenMenu({ x: event.clientX, y: event.clientY, kind: "task", task });
  };

  return (
    <section aria-label="보관한 작업" className="mt-1.5 border-t border-border pt-1.5">
      <button
        type="button"
        className="w-full px-2.5 py-1.5 flex items-center gap-1.5 text-left text-[11px] font-semibold text-text-secondary hover:text-text"
        onClick={() => setExpanded((value) => !value)}
        aria-expanded={expanded}
      >
        <Icon name={expanded ? "chevronDown" : "chevronRight"} size={12} />
        <span>보관한 작업 ({archived.length})</span>
      </button>
      {expanded && (
        <div className="flex flex-col gap-1 px-0.5">
          <p className="px-2.5 text-[11px] leading-4 text-text-muted">
            복원은 목록만 되돌리며 워크트리를 다시 만들지 않습니다.
          </p>
          {openError && <p role="alert" className="px-2.5 text-xs text-status-failed">{openError}</p>}
          {archived.map((task) => {
            const { archive } = task;
            const selected = taskKey(task) === selectedKey;
            const prUrl = safeGithubPullRequestUrl(archive.pr_url);
            return (
              <div
                key={taskKey(task)}
                data-archive-task-key={taskKey(task)}
                tabIndex={0}
                role="button"
                aria-label={`${task.instruction || archive.branch} 이력 열기`}
                onKeyDown={(event) => {
                  if (event.target !== event.currentTarget || !["Enter", " "].includes(event.key)) return;
                  event.preventDefault();
                  onOpenTask(task);
                }}
                onClick={() => onOpenTask(task)}
                onContextMenu={(event) => openMenu(event, task)}
                className={`mx-1 rounded-md border p-2 text-xs cursor-pointer transition-all ${
                  selected
                    ? "bg-raised border-primary text-text shadow-e1"
                    : "bg-surface border-border text-text-secondary hover:border-border-strong hover:text-text"
                }`}
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="min-w-0 truncate font-medium">{task.instruction || archive.branch}</span>
                  {onRestoreTask && (
                    <button
                      type="button"
                      className="shrink-0 rounded bg-raised px-1.5 py-0.5 text-[11px] text-primary-bright hover:text-text"
                      title="목록에만 다시 표시합니다. 워크트리를 다시 만들지 않습니다."
                      onClick={(event) => {
                        event.stopPropagation();
                        onRestoreTask(task);
                      }}
                    >
                      복원
                    </button>
                  )}
                </div>
                <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] font-code text-text-muted">
                  <span>보관 {new Date(archive.archived_at * 1000).toLocaleString()}</span>
                  <span title={archive.branch}>{archive.branch}</span>
                  <span title={archive.head_revision ?? undefined}>HEAD {revision(archive.head_revision)}</span>
                  <span
                    className={archive.worktree_exists ? "text-text-muted" : "text-status-failed"}
                    title={archive.worktree_path}
                  >
                    {archive.worktree_exists ? "워크트리 유지" : "워크트리 없음"}
                  </span>
                  {archive.result_path && <span title={archive.result_path}>결과 경로: {archive.result_path}</span>}
                  {prUrl && (
                    <button type="button" title={prUrl} className="text-primary-bright hover:text-text"
                      onClick={(event) => {
                        event.stopPropagation();
                        setOpenError(null);
                        void openUrl(prUrl).catch((error) => setOpenError(`PR을 열지 못했습니다: ${String(error)}`));
                      }}
                    >PR 열기</button>
                  )}
                </div>
                {!archive.worktree_exists && (
                  <p className="mt-1 text-[11px] text-text-muted">
                    워크트리 폴더가 없어도 작업 이력은 열 수 있습니다. 실행하려면 작업 폴더를 먼저 복구해야 합니다.
                  </p>
                )}
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}
