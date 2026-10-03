import { useState } from "react";
import type { TaskRef } from "../../lib/transport";
import { taskRef } from "../../lib/ipc";
import { ago } from "../../lib/fmt";
import {
  PENDING_APPROVAL,
  taskRunApprove,
  taskRunReject,
  type Task,
} from "../../lib/ipc";

interface Props {
  tasks: Task[];
  onRefresh: () => void;
}

const repoBase = (path: string): string => path.split("/").filter(Boolean).pop() ?? path;

/** 봇/크론이 만든 작업이 실행 전 대기 중임을 알리고 승인/거부 진입점을 제공. */
export function PendingApprovalSection({ tasks, onRefresh }: Props) {
  const [busyId, setBusyId] = useState<number | null>(null);
  const pending = tasks.filter((task) => task.state === PENDING_APPROVAL);
  if (pending.length === 0) return null;

  const decide = async (task: Task, action: (ref: TaskRef) => Promise<unknown>): Promise<void> => {
    setBusyId(task.id);
    try {
      await action(taskRef(task));
      onRefresh();
    } finally {
      setBusyId(null);
    }
  };

  return (
    <>
      <div className="text-xs uppercase tracking-wide text-text-muted mb-2">승인 대기</div>
      <div className="border border-border-strong rounded-md overflow-hidden mb-6">
        {pending.map((task) => (
          <div
            key={`${task.host}:${task.id}`}
            className="flex items-center gap-2.5 px-3 py-2.5 border-b border-border last:border-b-0 bg-raised"
          >
            <span className="w-2 h-2 rounded-full shrink-0" style={{ background: "var(--c-awaiting)" }} />
            <span className="text-sm text-text truncate">{task.instruction}</span>
            <span className="text-xs text-text-muted font-code shrink-0">
              {repoBase(task.repo)} · {ago(task.created_at)}
            </span>
            <div className="ml-auto flex items-center gap-1 shrink-0">
              <button
                className="h-7 px-2.5 rounded text-status-failed hover:opacity-80 disabled:opacity-40"
                disabled={busyId === task.id}
                onClick={() => void decide(task, taskRunReject)}
              >
                거절
              </button>
              <button
                className="h-7 px-3 rounded font-medium text-status-done hover:opacity-80 disabled:opacity-40"
                disabled={busyId === task.id}
                onClick={() => void decide(task, taskRunApprove)}
              >
                승인
              </button>
            </div>
          </div>
        ))}
      </div>
    </>
  );
}
