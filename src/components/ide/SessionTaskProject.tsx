import type { MouseEvent } from "react";
import type { Task } from "../../lib/ipc";
import type { NotificationKind } from "../../lib/notifications";
import { LOCAL_HOST, taskKey } from "../../lib/transport";
import { badgeLabelFor } from "../../lib/agents";
import { taskDotColor, taskStatusLabel, taskStatusLabelShort } from "../../lib/task-status";
import { Icon } from "./icons";
import { PROJECT_DRAG_MIME } from "./project-drag";
import type { TaskNavigationMenuState } from "./TaskNavigationMenu";

export interface SessionTaskProjectProps {
  repo: string;
  tasks: Task[];
  selectedKey: string | null;
  collapsed: boolean;
  queuedPosition: Map<number, number>;
  /** ⌘ 홀드 중에만 채워지는 작업 id → 번호. 비어 있으면 번호를 그리지 않는다. */
  shortcutNumbers: Map<string, number>;
  onToggle: () => void;
  onOpenTask: (task: Task) => void;
  onNewInRepo: (repo: string) => void;
  onOpenMenu: (menu: TaskNavigationMenuState) => void;
  /** 그룹으로 끌어 옮기는 중인가 — 시작에 repo, 끝에 null. 클릭은 여전히 접기다. */
  onDragProject?: (repo: string | null) => void;
  /** 읽지 않은 알림이 있는 작업 → 그 알림의 종류. 배지 문구를 종류별로 가른다. */
  unread: ReadonlyMap<string, NotificationKind>;
  /** 답을 기다리는 구조화 질문이 열린 작업의 좌표 — 점·라벨을 답변 대기로 그린다. */
  questionTasks: ReadonlySet<string>;
}

/** 배지 문구. 질문은 "새 결과"가 아니다 — 결과를 보러 갔다가 답할 것을 찾게 된다. */
const UNREAD_LABEL: Record<NotificationKind, string> = {
  result: "새 결과",
  question: "질문",
  failure: "실패",
};

/** 경로의 마지막 조각 = 프로젝트 이름. 최근 세션 행도 같은 표기를 써야 한다 — 복제하지 말 것. */
export const repoBase = (path: string): string =>
  path.split("/").filter(Boolean).pop() ?? path;

/** 점의 aria-label·title용 — 폭 제약이 없으니 완전한 문구를 쓴다. */
const taskStatus = (
  task: Task,
  queuedPosition: Map<number, number>,
  pendingQuestion: boolean,
): string =>
  task.state === "Queued"
    ? `대기 #${queuedPosition.get(task.id)}`
    : taskStatusLabel(task, pendingQuestion);

/** 카드 본문용 — 좁은 폭에서 같은 문구가 반복되므로 짧게 적는다. */
const taskStatusShort = (
  task: Task,
  queuedPosition: Map<number, number>,
  pendingQuestion: boolean,
): string =>
  task.state === "Queued"
    ? `대기 #${queuedPosition.get(task.id)}`
    : taskStatusLabelShort(task, pendingQuestion);

function SessionTaskCard({
  task,
  selected,
  shortcut,
  queuedPosition,
  onOpen,
  onOpenMenu,
  unreadKind,
  pendingQuestion,
}: {
  task: Task;
  selected: boolean;
  shortcut: number | null;
  queuedPosition: Map<number, number>;
  onOpen: () => void;
  onOpenMenu: (event: MouseEvent<HTMLDivElement>) => void;
  unreadKind: NotificationKind | null;
  /** 실행 중인데 답을 기다린다 — 상태는 Running이지만 사용자가 움직여야 진행된다. */
  pendingQuestion: boolean;
}) {
  return (
    <div
      onClick={onOpen}
      onContextMenu={onOpenMenu}
      className={`mx-1 mb-1 p-2 rounded-md border text-xs cursor-pointer transition-all ${
        selected
          ? "bg-raised border-primary text-text shadow-sm"
          : "bg-surface border-border text-text-secondary hover:border-border-strong hover:text-text"
      }`}
    >
      <div className="flex items-center justify-between gap-1 mb-1">
        <div className="flex items-center gap-1.5 truncate min-w-0">
          {/* ⌘ 홀드 중에만 나타나는 이동 번호. 상태 점을 대체하지 않고 앞에 붙는다 —
              점은 색각 이상 사용자에게도 유일한 상태 신호가 아니어야 한다. */}
          {shortcut !== null && (
            <span
              className="shrink-0 w-4 h-4 rounded border border-border-strong bg-raised text-[9px] font-code text-text flex items-center justify-center"
              data-shortcut={shortcut}
              aria-hidden
            >
              {shortcut}
            </span>
          )}
          <span
            className="w-2 h-2 rounded-full shrink-0"
            style={{ background: taskDotColor(task, pendingQuestion) }}
            role="img"
            aria-label={`작업 상태: ${taskStatus(task, queuedPosition, pendingQuestion)}`}
            title={taskStatus(task, queuedPosition, pendingQuestion)}
          />
          <span className="font-medium truncate">{task.instruction || task.branch}</span>
          {unreadKind && (
            <span className="shrink-0 text-[10px] text-primary-bright">{UNREAD_LABEL[unreadKind]}</span>
          )}
        </div>
        {badgeLabelFor(task.agent) && (
          <span className="text-[9px] px-1.5 py-0.5 rounded bg-raised text-primary-bright font-code shrink-0">
            {badgeLabelFor(task.agent)}
          </span>
        )}
      </div>
      {/* 상태는 항상 텍스트로도 적는다 — 점 색만으로는 "검토"와 "답변"을 구분할 근거가 없고,
          색각 이상 사용자에게는 아예 신호가 되지 못한다(DESIGN.md Do #2). 다만 목록 대부분이
          검토 대기로 수렴하므로 문구는 짧게 줄이고, 색은 점에 맡겨 브랜치명과 같은 무게로 둔다. */}
      <div className="flex items-center justify-between gap-1 text-[10px] text-text-muted font-code">
        <span className="truncate">{task.branch}</span>
        {/* 원격 세션만 호스트를 단다 — 로컬이 기본이라 거기 이름표를 붙이면 목록 전체가 소음이 된다. */}
        {task.host !== LOCAL_HOST && (
          <span
            className="shrink-0 rounded bg-raised px-1 text-primary-bright"
            title={`원격 호스트: ${task.host}`}
          >
            {task.host}
          </span>
        )}
        {/* 워크트리 소실은 상태와 독립된 축이다 — 상태는 여전히 "검토 대기"지만 열어도 아무것도
            실행되지 않는다. 상태 라벨을 덮어쓰지 않고 그 앞에 세워, 열기 전에 알 수 있게 한다. */}
        {task.worktree_missing && (
          <span
            className="shrink-0 px-1 rounded bg-raised text-status-failed"
            title={`워크트리 디렉터리가 없습니다: ${task.worktree_path}\n앱 밖에서 삭제된 것으로 보입니다. 커밋된 작업물은 브랜치에 남아 있습니다.`}
          >
            워크트리 없음
          </span>
        )}
        {task.stale && (
          <span className="shrink-0 px-1 rounded bg-raised text-text-muted" title="마지막 성공 목록입니다. 다시 연결하기 전에는 작업을 변경할 수 없습니다.">
            연결 끊김 · 마지막 확인
          </span>
        )}
        <span className="shrink-0">
          {task.stale ? "마지막 확인" : taskStatusShort(task, queuedPosition, pendingQuestion)}
        </span>
      </div>
    </div>
  );
}

export function SessionTaskProject({
  repo,
  tasks,
  selectedKey,
  collapsed,
  queuedPosition,
  shortcutNumbers,
  onToggle,
  onOpenTask,
  onNewInRepo,
  onOpenMenu,
  onDragProject,
  unread,
  questionTasks,
}: SessionTaskProjectProps) {
  return (
    <div className="rounded-lg border border-border/80 bg-surface/40 overflow-hidden">
      <div
        className="flex items-center justify-between px-2.5 py-2 text-xs font-medium text-text-muted cursor-pointer hover:bg-raised/60 transition-colors"
        draggable
        onDragStart={(event) => {
          event.dataTransfer.setData(PROJECT_DRAG_MIME, repo);
          event.dataTransfer.effectAllowed = "move";
          onDragProject?.(repo);
        }}
        onDragEnd={() => onDragProject?.(null)}
        onClick={onToggle}
        onContextMenu={(event) => {
          event.preventDefault();
          onOpenMenu({ x: event.clientX, y: event.clientY, kind: "project", repo });
        }}
      >
        <span className="flex items-center gap-1.5 truncate">
          <Icon name={collapsed ? "chevronRight" : "chevronDown"} size={12} />
          <Icon name="folder" size={13} />
          <span className="text-text font-semibold truncate">{repoBase(repo)}</span>
        </span>
        <button
          onClick={(event) => {
            event.stopPropagation();
            onNewInRepo(repo);
          }}
          className="p-1 rounded text-text-muted hover:text-text hover:bg-raised"
          title="이 프로젝트에 새 작업 생성"
          aria-label="이 프로젝트에 새 작업 생성"
        >
          <Icon name="plus" size={13} />
        </button>
      </div>
      {!collapsed && tasks.length === 0 && (
        <div className="px-3 pb-2 text-[11px] text-text-muted">진행 중인 작업 없음</div>
      )}
      {!collapsed &&
        tasks.map((task) => (
          <SessionTaskCard
            key={taskKey(task)}
            task={task}
            selected={taskKey(task) === selectedKey}
            shortcut={shortcutNumbers.get(taskKey(task)) ?? null}
            queuedPosition={queuedPosition}
            onOpen={() => onOpenTask(task)}
            onOpenMenu={(event) => {
              event.preventDefault();
              onOpenMenu({ x: event.clientX, y: event.clientY, kind: "task", task });
            }}
            unreadKind={unread.get(taskKey(task)) ?? null}
            pendingQuestion={questionTasks.has(taskKey(task))}
          />
        ))}
    </div>
  );
}
