import { useEffect, useState } from "react";
import type { Task } from "../lib/ipc";
import { ago } from "../lib/fmt";
import { badgeLabelFor } from "../lib/agents";
import { api } from "./api";
import { Empty, Spinner, StatusPill, StatusStrip } from "./primitives";
import { taskStateLabel } from "./status";
import { finishedTasks, groupByRepo, repoName, type RepoGroup } from "./grouping";
import { navigate } from "./router";
import { taskHref } from "./routes";
import { Icon } from "../components/ide/icons";

// 작업 목록 — 프로젝트 섹션. (설계 0013 §5.3)
//
// 폰은 한 화면에 몇 줄 못 보므로 섹션 순서가 곧 정보 설계다. 내 결정을 기다리는 작업이
// 있는 프로젝트가 위로 온다. 끝난 작업은 프로젝트와 무관하게 맨 아래 한 곳으로 모은다 —
// 섹션마다 이력이 쌓이면 지금 할 일이 묻힌다.

const COLLAPSED_KEY = "praxis-mobile-collapsed";

function loadCollapsed(): Set<string> {
  try {
    const raw = globalThis.localStorage?.getItem(COLLAPSED_KEY);
    return new Set(raw ? (JSON.parse(raw) as string[]) : []);
  } catch {
    return new Set();
  }
}

function saveCollapsed(collapsed: Set<string>): void {
  try {
    globalThis.localStorage?.setItem(COLLAPSED_KEY, JSON.stringify([...collapsed]));
  } catch {
    /* 저장 실패는 접힘이 기억되지 않을 뿐이다 */
  }
}

function TaskRow({ task, showRepo }: { task: Task; showRepo?: boolean }) {
  const state = taskStateLabel(task.state, task.awaiting_kind);
  const badge = badgeLabelFor(task.agent ?? null);
  return (
    <li className="relative">
      {/* 좌측 상태 스트립 — 배지만으론 시선 이동 비용이 크다 (DESIGN.md Do #1). */}
      <StatusStrip tone={state.tone} />
      <button
        type="button"
        aria-label={`작업 ${task.id} ${state.label}`}
        onClick={() => navigate(taskHref(task.id))}
        className="flex min-h-[56px] w-full flex-col gap-1 py-3 pl-5 pr-4 text-left active:bg-raised"
      >
        <div className="flex items-center gap-2">
          <StatusPill tone={state.tone}>{state.label}</StatusPill>
          {/* 타임스탬프는 code 레지스터 (DESIGN.md Do #3). */}
          <span className="ml-auto shrink-0 font-code text-xs text-text-muted">
            {ago(task.updated_at)}
          </span>
        </div>
        {/* 지시문은 UI body — md(14px). sm은 metadata 크기다. */}
        <div className="line-clamp-2 text-md text-text">{task.instruction}</div>
        <div className="flex items-center gap-1.5 text-xs text-text-muted">
          {/* 프로젝트 섹션 안에서는 레포를 반복하지 않는다 — 같은 값이 줄마다 반복되면 잡음이다. */}
          {showRepo ? <span className="truncate font-code">{repoName(task.repo)} ·</span> : null}
          <span className="shrink-0 font-code">#{task.id}</span>
          {badge ? <span className="shrink-0 truncate">· {badge}</span> : null}
          {task.mode === "conversation" ? <span className="shrink-0">· 대화</span> : null}
        </div>
      </button>
    </li>
  );
}

function Section({
  title,
  count,
  actionable,
  collapsed,
  onToggle,
  children,
}: {
  title: string;
  count: number;
  actionable?: number;
  collapsed: boolean;
  onToggle: () => void;
  children: React.ReactNode;
}) {
  return (
    <section>
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={!collapsed}
        // sticky: 긴 목록에서 지금 어느 프로젝트를 보고 있는지 잃지 않게 한다.
        className="sticky top-0 z-10 flex min-h-[40px] w-full items-center gap-2 border-y border-border bg-raised px-4 text-left"
      >
        <span className="inline-flex w-3 shrink-0 text-text-muted"><Icon name={collapsed ? "chevronRight" : "chevronDown"} size={12} /></span>
        {/* section-label: 11px semibold (DESIGN.md Sidebar). 프로젝트명은 경로이므로 code 레지스터. */}
        <span className="min-w-0 flex-1 truncate font-code text-xs font-semibold text-text">
          {title}
        </span>
        {actionable ? (
          // 상태색은 채우지 않고 보더+텍스트로 — 상태 표시 전용 규칙을 지키면서 대비를 유지한다.
          <span className="shrink-0 rounded border border-status-awaiting px-1.5 text-[11px] font-medium text-status-awaiting">
            {actionable}
          </span>
        ) : null}
        <span className="shrink-0 font-code text-xs text-text-muted">{count}</span>
      </button>
      {collapsed ? null : <ul className="divide-y divide-border">{children}</ul>}
    </section>
  );
}

export function TaskListScreen({ revision }: { revision: number }) {
  const [tasks, setTasks] = useState<Task[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(loadCollapsed);

  useEffect(() => {
    let cancelled = false;
    setError(null);
    api
      .taskList()
      .then((list) => {
        if (!cancelled) setTasks(list);
      })
      .catch((cause: unknown) => {
        if (!cancelled) setError(cause instanceof Error ? cause.message : String(cause));
      });
    return () => {
      cancelled = true;
    };
  }, [revision]);

  const toggle = (key: string) => {
    setCollapsed((previous) => {
      const next = new Set(previous);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      saveCollapsed(next);
      return next;
    });
  };

  if (error) return <Empty>목록을 불러오지 못했습니다. {error}</Empty>;
  if (!tasks) return <Spinner label="작업을 불러오는 중" />;
  if (tasks.length === 0) return <Empty>아직 작업이 없습니다.</Empty>;

  const groups: RepoGroup[] = groupByRepo(tasks);
  const finished = finishedTasks(tasks);

  return (
    <div className="pb-4">
      {groups.length === 0 ? (
        <Empty>진행 중인 작업이 없습니다.</Empty>
      ) : (
        groups.map((group) => (
          <Section
            key={group.repo}
            title={group.name}
            count={group.tasks.length}
            actionable={group.actionable}
            collapsed={collapsed.has(group.repo)}
            onToggle={() => toggle(group.repo)}
          >
            {group.tasks.map((task) => (
              <TaskRow key={task.id} task={task} />
            ))}
          </Section>
        ))
      )}

      {finished.length > 0 ? (
        <Section
          title="지난 작업"
          count={finished.length}
          // 기본 접힘 — 지금 할 일을 밀어내지 않게 한다.
          collapsed={!collapsed.has("__finished_open")}
          onToggle={() => toggle("__finished_open")}
        >
          {finished.map((task) => (
            <TaskRow key={task.id} task={task} showRepo />
          ))}
        </Section>
      ) : null}
    </div>
  );
}
