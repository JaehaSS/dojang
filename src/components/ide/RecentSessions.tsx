import { useEffect, useState, type MouseEvent } from "react";
import type { Task } from "../../lib/ipc";
import { LOCAL_HOST, taskKey } from "../../lib/transport";
import { taskStatusLabel, taskTone } from "../../lib/task-status";
import { recentSessions } from "../../lib/recent-sessions";
import { compactAge, repoBase } from "./SessionTaskProject";
import { TaskStatusIcon } from "./TaskStatusIcon";
import type { TaskNavigationMenuState } from "./TaskNavigationMenu";

export interface RecentSessionsProps {
  tasks: Task[];
  /** 선택된 작업의 (host, id) 좌표 문자열 — 아래 트리와 같은 값을 써 하이라이트가 어긋나지 않게. */
  selectedKey: string | null;
  onOpenTask: (task: Task) => void;
  /**
   * 행 우클릭으로 여는 메뉴 — 트리 카드와 **같은** `TaskNavigationMenu`를 쓴다(버리기·삭제).
   * 안 넘기면 브라우저 기본 메뉴가 그대로 뜬다(정적 렌더·테스트 호출부).
   */
  onOpenMenu?: (menu: TaskNavigationMenuState) => void;
  /** 창을 판정할 기준 시각(초). 주면 내부 tick을 쓰지 않는다 — 테스트가 시계를 고정하는 통로다. */
  nowSec?: number;
  /** 답을 기다리는 구조화 질문이 열린 작업의 좌표 — 트리 카드와 같은 점·라벨을 그린다. */
  questionTasks?: ReadonlySet<string>;
}

/** 창 밖으로 나간 세션을 떨어뜨리는 주기(ms). */
const TICK_MS = 60_000;

const nowInSec = (): number => Math.floor(Date.now() / 1000);

/**
 * 목록 갱신은 이벤트 구동이라(`App.tsx`의 `task://state`) 아무 일도 일어나지 않으면 재렌더가 없다.
 * 그러면 한 시간이 지난 세션이 화면에 그대로 남는다 — 시계를 직접 돌려 창을 밀어낸다.
 * `nowSec`를 받은 경우는 호출부가 시각을 쥐고 있으므로 타이머를 걸지 않는다.
 */
function useNowSec(fixed: number | undefined): number {
  const [tick, setTick] = useState(nowInSec);
  useEffect(() => {
    if (fixed !== undefined) return;
    const timer = setInterval(() => setTick(nowInSec()), TICK_MS);
    return () => clearInterval(timer);
  }, [fixed]);
  return fixed ?? tick;
}

/**
 * 지난 1시간 안에 대화한 세션의 평면 목록. 프로젝트 계층을 다시 세우지 않는다 — 헷갈림의 본질은
 * "어느 프로젝트인가"이므로 프로젝트명이 **보이기만** 하면 되고, 접는 단을 또 넣으면 아래 트리를
 * 두 번 만든 꼴이 된다(discovery brief).
 *
 * 비면 헤더까지 통째로 렌더하지 않는다(결정 2) — "최근 세션 (0)"은 자리만 차지한다.
 */
export function RecentSessions({
  tasks,
  selectedKey,
  onOpenTask,
  onOpenMenu,
  nowSec,
  questionTasks,
}: RecentSessionsProps) {
  const now = useNowSec(nowSec);
  const recent = recentSessions(tasks, now);
  if (recent.length === 0) return null;

  // 트리 카드(`SessionTaskProject`)와 같은 좌표·형태로 연다 — 여기서 버린 워크트리는 아래 트리에서
  // 버린 것과 같은 경로(`onDeleteTask`)를 타야 한다. 두 번째 삭제 경로를 만들지 않는다.
  const openMenu = (event: MouseEvent<HTMLDivElement>, task: Task): void => {
    if (!onOpenMenu) return;
    event.preventDefault();
    onOpenMenu({ x: event.clientX, y: event.clientY, kind: "task", task });
  };

  return (
    <section aria-label="최근 세션" className="mb-1.5 border-b border-border pb-1.5">
      <div className="px-2.5 py-1.5 flex items-center text-[11px] font-semibold text-text-secondary">
        <span title="지난 1시간 안에 대화한 세션">최근 세션 ({recent.length})</span>
      </div>
      {recent.map((task) => {
        const selected = taskKey(task) === selectedKey;
        const pendingQuestion = questionTasks?.has(taskKey(task)) ?? false;
        return (
          <div
            key={taskKey(task)}
            data-task-key={taskKey(task)}
            aria-current={selected ? "true" : undefined}
            onClick={() => onOpenTask(task)}
            onContextMenu={(event) => openMenu(event, task)}
            // 아래 트리 행과 같은 모양이다 — 테두리 없이 선택은 배경으로만 드러낸다(2026-11-09 Orca 차용).
            className={`mx-1 flex items-center gap-1.5 rounded-md px-2 py-1 text-sm cursor-pointer transition-colors ${
              selected ? "bg-raised text-text" : "text-text-secondary hover:bg-raised/50 hover:text-text"
            }`}
          >
            <TaskStatusIcon tone={taskTone(task, pendingQuestion)} label={taskStatusLabel(task, pendingQuestion)} />
            {/* 프로젝트명이 먼저다 — 찾고 있는 것이 "어느 프로젝트의 세션인가"이므로, 제목보다
                프로젝트가 앞에 와야 눈이 한 열만 훑고 끝난다. */}
            <span className="shrink-0 font-medium">{repoBase(task.repo)}</span>
            <span className="shrink-0 text-text-muted">·</span>
            <span className="truncate min-w-0 flex-1">{task.instruction || task.branch}</span>
            {/* 원격 세션만 호스트를 단다 — 로컬이 기본이라 거기 이름표를 붙이면 목록이 소음이 된다.
                teal은 행동·포커스 색이라 메타 이름표에는 쓰지 않는다(DESIGN.md Do #4). */}
            {task.host !== LOCAL_HOST && (
              <span
                className="shrink-0 rounded bg-raised px-1 text-[11px] font-code text-text-secondary"
                title={`원격 호스트: ${task.host}`}
              >
                {task.host}
              </span>
            )}
            <span className="shrink-0 text-xs text-text-muted">{compactAge(task.updated_at, now * 1000)}</span>
          </div>
        );
      })}
    </section>
  );
}
