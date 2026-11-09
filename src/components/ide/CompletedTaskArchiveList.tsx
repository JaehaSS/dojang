import { useState } from "react";
import type { Task } from "../../lib/ipc";
import { canArchiveTask } from "../../lib/task-archive";
import { taskKey } from "../../lib/transport";

/** 일반 최근 목록에서 숨긴 종료 이력에도 보관 진입점을 제공한다. */
export function CompletedTaskArchiveList({ tasks, onOpenTask, onArchiveTask }: {
  tasks: Task[];
  onOpenTask: (task: Task) => void;
  onArchiveTask: (task: Task) => void;
}) {
  const [limit, setLimit] = useState(20);
  const completed = tasks.filter((task) =>
    ["Done", "Failed", "Discarded"].includes(task.state) && canArchiveTask(task),
  );
  if (!completed.length) return null;
  return (
    <details className="mt-4 text-xs text-text-muted">
      <summary className="cursor-pointer">종료 작업 보관 ({completed.length})</summary>
      <p className="py-2">보관은 기록·폴더·브랜치를 유지합니다.</p>
      {completed.slice(0, limit).map((task) => (
        <div key={taskKey(task)} className="flex items-center gap-2 py-1">
          <button className="flex-1 min-w-0 text-left truncate hover:text-text" onClick={() => onOpenTask(task)} title={task.instruction || task.branch}>{task.instruction || task.branch}</button>
          <button className="shrink-0 px-2 py-1 hover:text-text" onClick={() => onArchiveTask(task)} aria-label={`${task.instruction || task.branch} 보관`}>보관</button>
        </div>
      ))}
      {completed.length > limit && <button className="py-2 hover:text-text" onClick={() => setLimit((value) => value + 20)}>더 보기</button>}
    </details>
  );
}
