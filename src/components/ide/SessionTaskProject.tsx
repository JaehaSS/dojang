import type { MouseEvent } from "react";
import type { Task } from "../../lib/ipc";
import type { NotificationKind } from "../../lib/notifications";
import { LOCAL_HOST, taskKey } from "../../lib/transport";
import { badgeLabelFor } from "../../lib/agents";
import { taskStatusLabel, taskStatusLabelShort, taskTone } from "../../lib/task-status";
import { Icon } from "./icons";
import { PROJECT_DRAG_MIME } from "./project-drag";
import { TaskStatusIcon } from "./TaskStatusIcon";
import type { TaskNavigationMenuState } from "./TaskNavigationMenu";
import { IconButton } from "../ui";

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

/** 상태 마크의 aria-label·title용 — 폭 제약이 없으니 완전한 문구를 쓴다. */
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

/** 목록 오른쪽 끝의 경과 표기 — 행이 좁아 "분 전"을 떼고 단위 한 글자만 남긴다. */
export function compactAge(at: number, nowMs = Date.now()): string {
  const ms = at > 1e12 ? at : at * 1000;
  const mins = Math.floor(Math.max(0, nowMs - ms) / 60_000);
  if (mins < 1) return "방금";
  if (mins < 60) return `${mins}분`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}시간`;
  return `${Math.floor(hours / 24)}일`;
}

/**
 * 세션 행. 카드 테두리를 걷었다 — 항목마다 상자를 두르니 목록 전체가 같은 무게의 상자 더미가 돼
 * 선택·상태·제목 어느 것도 앞으로 나오지 못했다(2026-11-09 디자인 역설계). 이제 행은 투명하고,
 * 선택은 배경 한 단계로만 말한다. 색은 상태 마크 하나가 진다 — teal은 메타 정보에 쓰지 않는다.
 */
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
  const agentLabel = badgeLabelFor(task.agent);
  return (
    <div
      data-task-key={taskKey(task)}
      onClick={onOpen}
      onContextMenu={onOpenMenu}
      aria-current={selected ? "true" : undefined}
      className={`mx-1 rounded-md px-2 py-1.5 cursor-pointer transition-colors ${
        selected ? "bg-raised text-text" : "text-text-secondary hover:bg-raised/50 hover:text-text"
      }`}
    >
      <div className="flex items-center gap-2 min-w-0">
        {/* ⌘ 홀드 중에만 나타나는 이동 번호. 상태 마크를 대체하지 않고 앞에 붙는다. */}
        {shortcut !== null && (
          <span
            className="shrink-0 w-4 h-4 rounded border border-border-strong bg-raised text-[11px] font-code text-text flex items-center justify-center"
            data-shortcut={shortcut}
            aria-hidden
          >
            {shortcut}
          </span>
        )}
        <TaskStatusIcon
          tone={taskTone(task, pendingQuestion)}
          label={taskStatus(task, queuedPosition, pendingQuestion)}
        />
        <span className={`flex-1 min-w-0 truncate text-base ${unreadKind ? "font-semibold text-text" : "font-medium"}`}>
          {task.instruction || task.branch}
        </span>
        {/* 읽지 않음은 teal 라벨이 아니라 굵은 제목 + 종류 문구다 — 질문과 새 결과는 할 일이 달라 문구는 남긴다. */}
        {unreadKind && (
          <span className="shrink-0 flex items-center gap-1 text-xs text-text">
            <span className="w-1.5 h-1.5 rounded-full bg-text" aria-hidden />
            {UNREAD_LABEL[unreadKind]}
          </span>
        )}
        <span className="shrink-0 text-xs text-text-muted tabular-nums" title="마지막 변경">
          {compactAge(task.updated_at)}
        </span>
      </div>
      {/* 상태는 항상 텍스트로도 적는다(DESIGN.md Do #2) — 마크에는 완전한 문구가, 본문에는 축약이 간다. */}
      <div className="flex items-center gap-1.5 pl-[22px] mt-0.5 text-xs text-text-muted min-w-0 whitespace-nowrap">
        <span className="shrink-0">
          {task.stale ? "마지막 확인" : taskStatusShort(task, queuedPosition, pendingQuestion)}
        </span>
        {/* 워크트리 소실은 상태와 독립된 축이다 — 열기 전에 알 수 있게 상태 바로 뒤에 세운다. */}
        {task.worktree_missing && (
          <span
            className="shrink-0 text-status-failed"
            title={`워크트리 디렉터리가 없습니다: ${task.worktree_path}\n앱 밖에서 삭제된 것으로 보입니다. 커밋된 작업물은 브랜치에 남아 있습니다.`}
          >
            · 워크트리 없음
          </span>
        )}
        {task.stale && (
          <span className="shrink-0" title="마지막 성공 목록입니다. 다시 연결하기 전에는 작업을 변경할 수 없습니다.">
            · 연결 끊김
          </span>
        )}
        {/* 고정폭은 브랜치명에만 — 에이전트 이름까지 mono로 쓰면 폭을 먹어 브랜치가 첫 단어에서 잘린다. */}
        <span className="truncate min-w-0">
          · {agentLabel ? `${agentLabel} · ` : ""}<span className="font-code">{task.branch}</span>
        </span>
        {/* 원격 세션만 호스트를 단다 — 로컬이 기본이라 거기 이름표를 붙이면 목록 전체가 소음이 된다. */}
        {task.host !== LOCAL_HOST && (
          <span className="ml-auto shrink-0 font-code" title={`원격 호스트: ${task.host}`}>
            {task.host}
          </span>
        )}
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
    <div className="pb-1">
      <div
        className="group flex items-center justify-between mx-1 px-2 pt-2.5 pb-1 rounded-md text-xs font-medium text-text-muted cursor-pointer hover:text-text-secondary transition-colors"
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
          <Icon name="folder" size={14} />
          <span className="text-text-secondary font-semibold truncate">{repoBase(repo)}</span>
        </span>
        <IconButton
          size="sm"
          label="이 프로젝트에 새 작업 생성"
          onClick={(event) => {
            event.stopPropagation();
            onNewInRepo(repo);
          }}
        >
          <Icon name="plus" size={14} />
        </IconButton>
      </div>
      {!collapsed && tasks.length === 0 && (
        <div className="px-3 pb-2 text-xs text-text-muted">진행 중인 작업 없음</div>
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
