import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { ACTIVE_STATES, type Task } from "../../lib/ipc";
import { PROJECT_GROUP_COLORS, type ProjectGroupColorId } from "../../lib/project-group-colors";
import type { ProjectGroup } from "../../lib/project-groups";
import { groupPath } from "../../lib/project-group-tree";
import { swallowNextClick } from "./ContextMenuShell";
import { isDirectRun, preservedBranchLabel } from "./discard-confirm";
import { Icon } from "./icons";

export type TaskNavigationMenuState =
  | { x: number; y: number; kind: "task"; task: Task }
  | { x: number; y: number; kind: "project"; repo: string }
  | { x: number; y: number; kind: "group"; groupId: string; trigger?: HTMLElement }
  | { x: number; y: number; kind: "background" }
  | null;

/** 그룹 조작 묶음 — 그룹을 쓰지 않는 호출부는 하나도 넘기지 않아도 된다(항목이 안 그려진다). */
interface GroupActions {
  groups?: ProjectGroup[];
  /** repo가 지금 속한 그룹 id. 없으면 미소속. */
  groupOf?: (repo: string) => string | null;
  /** 그룹 해제 라벨에 병기할 프로젝트 수 — 확인 없이 실행되므로 결과를 미리 적는다. */
  groupCount?: (groupId: string) => number;
  onMoveToGroup?: (repo: string, groupId: string) => void;
  onMoveToNewGroup?: (repo: string) => void;
  /** 프로젝트 없이 빈 그룹을 만든다 — 목록의 빈 곳에서 부른다. */
  onCreateGroup?: () => void;
  onUnassign?: (repo: string) => void;
  onRenameGroup?: (groupId: string) => void;
  onDissolveGroup?: (groupId: string) => void;
  onMoveGroup?: (groupId: string, parentId: string | null) => void;
  canMoveGroup?: (groupId: string, parentId: string | null) => boolean;
  onSetGroupColor?: (groupId: string, color: ProjectGroupColorId | undefined) => void;
}

interface Props extends GroupActions {
  menu: TaskNavigationMenuState;
  activeTasks: Task[];
  onClose: () => void;
  onDeleteTask: (task: Task) => void;
  /**
   * 검토 대기 작업을 리뷰 바의 승인과 같은 경로로 `task.base`(생성 시 체크아웃돼 있던 브랜치)에 머지한다.
   * 안 넘기면 항목이 그려지지 않는다.
   */
  onMergeTask?: (task: Task) => void;
  onRemoveProject: (repo: string) => void;
}

const repoBase = (path: string): string =>
  path.split("/").filter(Boolean).pop() ?? path;

const ITEM =
  "w-full text-left flex items-center gap-2 px-3 py-1.5 text-sm text-text-secondary hover:bg-surface hover:text-text";

const MENU_MARGIN = 8;

function MenuBox({
  at,
  ariaLabel,
  children,
}: {
  at: { x: number; y: number };
  ariaLabel: string;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ top: at.y, left: at.x });
  useLayoutEffect(() => {
    const menu = ref.current;
    if (menu == null) return;
    const updatePosition = () => {
      const box = menu.getBoundingClientRect();
      setPosition({
        top: Math.max(MENU_MARGIN, Math.min(at.y, window.innerHeight - box.height - MENU_MARGIN)),
        left: Math.max(MENU_MARGIN, Math.min(at.x, window.innerWidth - box.width - MENU_MARGIN)),
      });
    };
    updatePosition();
    const observer = new ResizeObserver(updatePosition);
    observer.observe(menu);
    return () => observer.disconnect();
  }, [at]);
  const menu = (
    <div
      ref={ref}
      className="fixed z-50 max-h-[calc(100vh-16px)] min-w-[168px] max-w-[240px] overflow-y-auto rounded-md border border-border-strong bg-raised py-1 shadow-xl"
      style={position}
      onMouseDown={(event) => event.stopPropagation()}
      role="menu"
      aria-label={ariaLabel}
    >
      {children}
    </div>
  );
  return typeof document === "undefined" ? menu : createPortal(menu, document.body);
}

function GroupColorPage({
  group,
  actions,
  onBack,
  onClose,
}: {
  group: ProjectGroup;
  actions: GroupActions;
  onBack: () => void;
  onClose: () => void;
}) {
  const firstItem = useRef<HTMLButtonElement>(null);
  useEffect(() => firstItem.current?.focus(), []);
  return (
    <div data-group-color-page onKeyDown={(event) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      onBack();
      onClose();
    }}>
      <button className={ITEM} onClick={onBack}>
        <Icon name="chevronLeft" size={13} /> 그룹 색상
      </button>
      <Divider />
      {PROJECT_GROUP_COLORS.map((color, index) => (
        <button
          key={color.id}
          ref={index === 0 ? firstItem : undefined}
          role="menuitemradio"
          aria-checked={group.color === color.id}
          className={ITEM}
          onClick={() => {
            actions.onSetGroupColor?.(group.id, color.id);
            onClose();
          }}
        >
          <span className="h-3 w-3 rounded-full" style={{ backgroundColor: color.accent }} />
          <span>{color.label}</span>
          {group.color === color.id && <Icon name="check" size={13} />}
        </button>
      ))}
      <Divider />
      <button
        role="menuitemradio"
        aria-checked={group.color === undefined}
        className={ITEM}
        onClick={() => {
          actions.onSetGroupColor?.(group.id, undefined);
          onClose();
        }}
      >
        기본값으로 되돌리기
        {group.color === undefined && <Icon name="check" size={13} />}
      </button>
    </div>
  );
}

const Divider = () => <div className="my-1 border-t border-border" />;

/** 머지는 워크트리 작업에만 뜻이 있다 — 직접 실행은 이미 체크아웃된 브랜치에서 돌았다. */
const canMerge = (task: Task): boolean =>
  task.state === "AwaitingReview" && !task.stale && !isDirectRun(task);

function TaskMenu({
  task,
  onRun,
  onMerge,
}: {
  task: Task;
  onRun: () => void;
  onMerge?: () => void;
}) {
  return (
    <>
      <div className="px-3 py-1 text-xs text-text-muted truncate">
        {task.instruction || task.branch}
      </div>
      {/* 브랜치가 바뀐 작업은 폴더째 남으므로 두 복구 위치를 함께 표시한다. */}
      {preservedBranchLabel(task) && (
        <div className="px-3 pb-1 text-xs text-text-muted truncate" title={preservedBranchLabel(task) ?? undefined}>
          {preservedBranchLabel(task)}
        </div>
      )}
      <Divider />
      {onMerge && canMerge(task) && (
        <button className={ITEM} onClick={onMerge} title={`${task.branch} → ${task.base}`}>
          <Icon name="check" size={13} />
          <span className="min-w-0 truncate">{task.base}</span>
          <span className="shrink-0">에 머지</span>
        </button>
      )}
      <button
        className="w-full text-left flex items-center gap-2 px-3 py-1.5 text-sm text-status-failed hover:bg-surface"
        onClick={onRun}
      >
        <Icon name="x" size={13} />
        {task.state === "AwaitingReview"
          ? "버리기 (워크트리 정리)"
          : ACTIVE_STATES.includes(task.state)
            ? "진행 중 — 중단 후 버리기"
            : "이력에서 삭제"}
      </button>
    </>
  );
}

/** 그룹 목록으로 **교체되는** 한 단계 메뉴 — 240px 사이드바에서 2단 hover는 화면 밖으로 나간다. */
function MoveToGroupPage({
  repo,
  current,
  actions,
  onBack,
  onClose,
}: {
  repo: string;
  current: string | null;
  actions: GroupActions;
  onBack: () => void;
  onClose: () => void;
}) {
  return (
    <>
      <button className={`${ITEM} text-text-muted`} onClick={onBack}>
        <Icon name="chevronLeft" size={13} /> 그룹으로 이동
      </button>
      <Divider />
      {(actions.groups ?? []).map((group) => (
        <button
          key={group.id}
          className={ITEM}
          onClick={() => {
            onClose();
            actions.onMoveToGroup?.(repo, group.id);
          }}
        >
          <span className="truncate" title={groupPath(actions.groups ?? [], group.id)}>{groupPath(actions.groups ?? [], group.id)}</span>
          {current === group.id && <Icon name="check" size={13} />}
        </button>
      ))}
      <button
        className={ITEM}
        onClick={() => {
          onClose();
          actions.onMoveToNewGroup?.(repo);
        }}
      >
        <Icon name="plus" size={13} /> 새 그룹…
      </button>
    </>
  );
}

function GroupMovePage({ groupId, actions, onBack, onClose }: {
  groupId: string;
  actions: GroupActions;
  onBack: () => void;
  onClose: () => void;
}) {
  const current = actions.groups?.find((group) => group.id === groupId)?.parentId ?? null;
  const groups = actions.groups ?? [];
  return (
    <>
      <button className={`${ITEM} text-text-muted`} onClick={onBack}>
        <Icon name="chevronLeft" size={13} /> 그룹으로 이동
      </button>
      <Divider />
      {groups.filter((group) => group.id !== groupId && actions.canMoveGroup?.(groupId, group.id) !== false).map((group) => (
        <button
          key={group.id}
          disabled={current === group.id}
          className={`${ITEM} disabled:cursor-not-allowed disabled:opacity-40`}
          onClick={() => {
            onClose();
            actions.onMoveGroup?.(groupId, group.id);
          }}
        >
          <span className="truncate" title={groupPath(groups, group.id)}>{groupPath(groups, group.id)}</span>
          {current === group.id && <Icon name="check" size={13} />}
        </button>
      ))}
    </>
  );
}

function ProjectMenu({
  repo,
  hasActiveTasks,
  actions,
  onClose,
  onRemoveProject,
}: {
  repo: string;
  hasActiveTasks: boolean;
  actions: GroupActions;
  onClose: () => void;
  onRemoveProject: (repo: string) => void;
}) {
  const [page, setPage] = useState<"root" | "move">("root");
  const current = actions.groupOf?.(repo) ?? null;
  if (page === "move") {
    return (
      <MoveToGroupPage
        repo={repo}
        current={current}
        actions={actions}
        onBack={() => setPage("root")}
        onClose={onClose}
      />
    );
  }
  return (
    <>
      <div className="px-3 py-1 text-xs text-text-muted truncate">{repoBase(repo)}</div>
      <Divider />
      {actions.onMoveToGroup && (
        <button className={ITEM} onClick={() => setPage("move")}>
          <Icon name="folder" size={13} />
          <span className="truncate">그룹으로 이동</span>
          <Icon name="chevronRight" size={13} />
        </button>
      )}
      {current !== null && (
        <button
          className={ITEM}
          onClick={() => {
            onClose();
            actions.onUnassign?.(repo);
          }}
        >
          그룹에서 빼기
        </button>
      )}
      <button
        disabled={hasActiveTasks}
        title={hasActiveTasks ? "진행 중인 작업이 있어 제거할 수 없습니다" : undefined}
        className="w-full text-left flex items-center gap-2 px-3 py-1.5 text-sm text-status-failed hover:bg-surface disabled:opacity-40 disabled:hover:bg-transparent disabled:cursor-not-allowed"
        onClick={() => {
          onClose();
          onRemoveProject(repo);
        }}
      >
        <Icon name="x" size={13} /> 프로젝트 제거
      </button>
    </>
  );
}

function GroupMenu({
  groupId,
  actions,
  onClose,
  onColorPageChange,
}: {
  groupId: string;
  actions: GroupActions;
  onClose: () => void;
  onColorPageChange: (open: boolean) => void;
}) {
  const [page, setPage] = useState<"root" | "move" | "color">("root");
  const firstItem = useRef<HTMLButtonElement>(null);
  const group = actions.groups?.find((candidate) => candidate.id === groupId);

  useEffect(() => {
    if (page === "root") firstItem.current?.focus();
  }, [page]);

  if (!group) return null;

  const parentId = group.parentId ?? null;
  const childCount = actions.groups?.filter((candidate) => candidate.parentId === groupId).length ?? 0;

  if (page === "move") {
    return (
      <GroupMovePage
        groupId={groupId}
        actions={actions}
        onBack={() => setPage("root")}
        onClose={onClose}
      />
    );
  }

  if (page === "color") {
    return (
      <GroupColorPage
        group={group}
        actions={actions}
        onBack={() => {
          setPage("root");
          onColorPageChange(false);
        }}
        onClose={onClose}
      />
    );
  }

  return (
    <>
      <div className="px-3 py-1 text-xs text-text-muted truncate">{group.name}</div>
      <Divider />
      <button
        ref={firstItem}
        className={ITEM}
        onClick={() => {
          onClose();
          actions.onRenameGroup?.(groupId);
        }}
      >
        이름 바꾸기
      </button>
      <button className={ITEM} onClick={() => setPage("move")}>
        <Icon name="folder" size={13} /> 그룹으로 이동 <Icon name="chevronRight" size={13} />
      </button>
      <button
        disabled={parentId === null}
        className={`${ITEM} disabled:cursor-not-allowed disabled:opacity-40`}
        onClick={() => {
          onClose();
          actions.onMoveGroup?.(groupId, null);
        }}
      >
        그룹에서 빼기
      </button>
      <button
        className={ITEM}
        onClick={() => {
          setPage("color");
          onColorPageChange(true);
        }}
      >
        그룹 색상
        <Icon name="chevronRight" size={13} />
      </button>
      <button
        className={ITEM}
        onClick={() => {
          onClose();
          actions.onDissolveGroup?.(groupId);
        }}
      >
        그룹 해제 (프로젝트 {actions.groupCount?.(groupId) ?? 0}개 {parentId === null ? "미소속으로" : "상위 그룹으로"}, 하위 그룹 {childCount}개 승격)
      </button>
    </>
  );
}

export function TaskNavigationMenu({
  menu,
  activeTasks,
  onClose,
  onDeleteTask,
  onMergeTask,
  onRemoveProject,
  ...actions
}: Props) {
  const [groupColorPage, setGroupColorPage] = useState(false);
  useEffect(() => {
    if (!menu) return;
    const closeOnEscape = (event: KeyboardEvent): void => {
      if (event.key !== "Escape") return;
      onClose();
      if (menu.kind === "group") menu.trigger?.focus();
    };
    // 닫는 클릭은 아래 요소에 닿지 않아야 한다 — 삼키기 리스너는 스스로 사라지므로
    // cleanup에서 지우지 않는다(onClose()가 menu를 null로 만들어 cleanup이 click보다 먼저 돈다).
    const onDown = (): void => {
      swallowNextClick();
      onClose();
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", closeOnEscape);
    window.addEventListener("resize", onClose);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", closeOnEscape);
      window.removeEventListener("resize", onClose);
    };
  }, [menu, onClose]);

  useEffect(() => {
    setGroupColorPage(false);
  }, [menu]);

  if (!menu) return null;

  return (
    <MenuBox at={menu} ariaLabel={groupColorPage ? "그룹 색상" : "세션 작업 조작"}>
      {menu.kind === "task" && (
        <TaskMenu
          task={menu.task}
          onRun={() => {
            onClose();
            onDeleteTask(menu.task);
          }}
          onMerge={
            onMergeTask &&
            (() => {
              onClose();
              onMergeTask(menu.task);
            })
          }
        />
      )}
      {menu.kind === "project" && (
        <ProjectMenu
          key={menu.repo}
          repo={menu.repo}
          hasActiveTasks={activeTasks.some((task) => task.repo === menu.repo)}
          actions={actions}
          onClose={onClose}
          onRemoveProject={onRemoveProject}
        />
      )}
      {menu.kind === "background" && (
        <button
          className={ITEM}
          onClick={() => {
            onClose();
            actions.onCreateGroup?.();
          }}
        >
          <Icon name="plus" size={13} /> 새 그룹 만들기
        </button>
      )}
      {menu.kind === "group" && (
        <GroupMenu
          key={menu.groupId}
          groupId={menu.groupId}
          actions={actions}
          onColorPageChange={setGroupColorPage}
          onClose={() => {
            onClose();
            menu.trigger?.focus();
          }}
        />
      )}
    </MenuBox>
  );
}
