import { useCallback, useState } from "react";
import { ACTIVE_STATES, type Task } from "../../lib/ipc";
import {
  assignProject,
  canMoveGroup,
  createGroup,
  dissolveGroup,
  setGroupColor,
  EMPTY_PROJECT_GROUPS,
  moveGroup,
  unassignProject,
  type ProjectGroups,
} from "../../lib/project-groups";
import { orderSections, visibleProjectRepos } from "../../lib/project-group-tree";
import { taskKey } from "../../lib/transport";
import { HIDDEN_STATES } from "../../lib/recent-sessions";
import { RecentSessions } from "./RecentSessions";
import { SessionTaskProject } from "./SessionTaskProject";
import { SessionTaskSections, type RenamingGroup } from "./SessionTaskSections";
import {
  TaskNavigationMenu,
  type TaskNavigationMenuState,
} from "./TaskNavigationMenu";
import { useProjectGroupDrag } from "./useProjectGroupDrag";
import { SHORTCUT_LIMIT, useSessionTaskShortcuts } from "./useSessionTaskShortcuts";
import { useNotificationSnapshot } from "../../lib/use-notification-snapshot";

export interface SessionTaskNavigationProps {
  tasks: Task[];
  /** 선택된 작업의 (host, id) 좌표 문자열. id만으로는 호스트가 다른 동명 작업과 구분되지 않는다. */
  selectedKey: string | null;
  projects: string[];
  onOpenTask: (task: Task) => void;
  onNewInRepo: (repo: string) => void;
  onDeleteTask: (task: Task) => void;
  /** 우클릭 메뉴의 "<base>에 머지" — 검토 대기 워크트리 작업에만 나타난다. */
  onMergeTask?: (task: Task) => void;
  onRemoveProject: (repo: string) => void;
  /** 워크트리가 사라진 작업 일괄 종결 — 배너는 그런 작업이 있을 때만 나타난다. */
  onDiscardOrphans: () => void;
  /** 프로젝트 묶음. 그룹을 쓰지 않는 호출부는 넘기지 않아도 된다 — 지금과 같은 평면 목록이다. */
  groups?: ProjectGroups;
  /** 다음 그룹 상태 전체. 저장은 `App`의 몫이다(설계 0062 D-8). */
  onProjectGroupsChange?: (next: ProjectGroups) => void;
  /**
   * 답을 기다리는 구조화 질문이 열린 작업의 (host, id) 좌표 — attention 투영(`reason: "question"`)에서
   * 온다. 실행은 `Running`인 채라 `tasks` 행만으로는 드러나지 않는다. 안 넘기면 상태만으로 그린다.
   */
  questionTasks?: ReadonlySet<string>;
}

/** 질문 관측을 넘기지 않는 호출부의 자리 — 렌더마다 새로 만들면 카드가 매번 갱신된다. */
const NO_QUESTIONS: ReadonlySet<string> = new Set();

/** ⌘를 잡고 있지 않을 때 넘기는 빈 맵 — 렌더마다 새로 만들면 카드가 매번 갱신된다. */
const NO_SHORTCUTS: Map<string, number> = new Map();

/** 그룹을 쓰지 않는 호출부의 자리 — 렌더마다 새 함수를 만들면 카드가 매번 갱신된다. */
const NO_GROUP_CHANGE = (): void => undefined;

const COLLAPSED_PROJECTS_STORAGE_KEY = "praxis-desktop-collapsed-projects";

/** 저장소의 지금 값. 키가 없거나 형식이 틀리거나 읽지 못하면 null — "빈 집합"과 구분한다. */
function readStoredCollapsedProjects(): Set<string> | null {
  try {
    const raw = globalThis.localStorage?.getItem(COLLAPSED_PROJECTS_STORAGE_KEY);
    if (!raw) return null;
    const repos: unknown = JSON.parse(raw);
    return Array.isArray(repos) && repos.every((repo) => typeof repo === "string")
      ? new Set(repos)
      : null;
  } catch {
    return null;
  }
}

function loadCollapsedProjects(): Set<string> {
  return readStoredCollapsedProjects() ?? new Set();
}

function saveCollapsedProjects(collapsed: Set<string>): void {
  try {
    globalThis.localStorage?.setItem(COLLAPSED_PROJECTS_STORAGE_KEY, JSON.stringify([...collapsed]));
  } catch {
    // 저장소가 차단된 환경에서도 현재 세션의 접힘 동작은 유지한다.
  }
}

export function SessionTaskNavigation(props: SessionTaskNavigationProps) {
  const notifications = useNotificationSnapshot();
  // 값은 알림 종류다 — 배지 문구가 "새 결과"만이면 질문·실패도 결과 도착처럼 읽힌다.
  const unread = new Map(
    (notifications.snapshot?.items ?? []).map((item) => [taskKey({ host: item.host, id: item.task_id }), item.kind]),
  );
  const questionTasks = props.questionTasks ?? NO_QUESTIONS;
  const activeTasks = props.tasks.filter(
    (task) => !HIDDEN_STATES.has(task.state)
      && (ACTIVE_STATES.includes(task.state) || task.stale || unread.has(taskKey(task))),
  );
  const queuedPosition = new Map(
    props.tasks
      .filter((task) => task.state === "Queued")
      .sort((left, right) => left.created_at - right.created_at || left.id - right.id)
      .map((task, index) => [task.id, index + 1]),
  );
  // 접힌 프로젝트의 고아까지 센다 — 배너는 "정리하면 몇 건이 사라지는가"를 말하고, 정리 자체는
  // 화면에 무엇이 보이는지와 무관하게 전부를 대상으로 한다.
  const orphanCount = activeTasks.filter((task) => task.worktree_missing).length;
  const [collapsedProjects, setCollapsedProjects] = useState<Set<string>>(loadCollapsedProjects);
  const [menu, setMenu] = useState<TaskNavigationMenuState>(null);
  const closeMenu = useCallback((): void => setMenu(null), []);
  const groups = props.groups ?? EMPTY_PROJECT_GROUPS;
  const emit = props.onProjectGroupsChange ?? NO_GROUP_CHANGE;
  const [renaming, setRenaming] = useState<RenamingGroup>(null);
  const drag = useProjectGroupDrag(groups, emit);
  const sections = orderSections(props.projects, props.tasks, groups);
  const grouped = sections.filter((section) => section.group !== null);
  const unassigned = sections[sections.length - 1]?.repos ?? [];
  // 번호는 화면에 보이는 카드에만 붙인다 — 접힌 그룹·프로젝트의 작업까지 세면 목록에 2, 5, 7처럼
  // 건너뛴 번호가 남아 어떤 키를 눌러야 할지 알 수 없게 된다.
  const visibleTasks = visibleProjectRepos(sections)
    .filter((repo) => !collapsedProjects.has(repo))
    .flatMap((repo) => activeTasks.filter((task) => task.repo === repo));
  const orderedTasks = visibleTasks.slice(0, SHORTCUT_LIMIT);
  const shortcutNumbers = new Map(orderedTasks.map((task, index) => [taskKey(task), index + 1]));
  // Delete는 번호 상한과 무관하게 "지금 열려 있고 목록에도 보이는" 작업을 지운다.
  const holdingMeta = useSessionTaskShortcuts({
    orderedTasks,
    deleteTarget: visibleTasks.find((task) => taskKey(task) === props.selectedKey) ?? null,
    onOpenTask: props.onOpenTask,
    onDeleteTask: props.onDeleteTask,
  });

  const toggleProject = (repo: string): void => {
    setCollapsedProjects((previous) => {
      // 방향은 사용자가 보고 있는 상태로 정하고, 쓰는 기준은 저장소의 지금 값이다 — 메모리의
      // 집합으로 통째로 덮으면 다른 창·경로가 접어 둔 프로젝트가 조용히 펼쳐진다.
      const collapsing = !previous.has(repo);
      const next = new Set(readStoredCollapsedProjects() ?? previous);
      if (collapsing) next.add(repo);
      else next.delete(repo);
      saveCollapsedProjects(next);
      return next;
    });
  };

  /** 만들고, 넣고, 헤더를 바로 이름 편집으로 — 다이얼로그 없이 사이드바 안에서 끝난다. */
  const createGroupFor = (repo: string): void => {
    const id = crypto.randomUUID();
    emit(assignProject(createGroup(groups, "새 그룹", id), repo, id));
    setRenaming({ id, created: true, repo, previousGroupId: groups.assignment[repo] ?? null });
  };

  /** 프로젝트 없이 만드는 빈 그룹 — 헤더가 바로 이름 편집으로 열리고, 비워 취소하면 사라진다. */
  const createEmptyGroup = (): void => {
    const id = crypto.randomUUID();
    emit(createGroup(groups, "새 그룹", id));
    setRenaming({ id, created: true, repo: null });
  };

  const renderProject = (repo: string) => (
    <SessionTaskProject
      key={repo}
      repo={repo}
      tasks={activeTasks.filter((task) => task.repo === repo)}
      selectedKey={props.selectedKey}
      collapsed={collapsedProjects.has(repo)}
      queuedPosition={queuedPosition}
      shortcutNumbers={holdingMeta ? shortcutNumbers : NO_SHORTCUTS}
      onToggle={() => toggleProject(repo)}
      onOpenTask={props.onOpenTask}
      onNewInRepo={props.onNewInRepo}
      onOpenMenu={setMenu}
      onDragProject={drag.onDragProject}
      unread={unread}
      questionTasks={questionTasks}
    />
  );

  return (
    <>
      {/* 방금까지 대화하던 세션을 트리를 펼쳐 찾지 않게 한다 — 프로젝트 계층을 건너뛴 평면 목록.
          창 안에 아무것도 없으면 스스로 사라진다. 여기서 그리는 이유는 메뉴 하나다 — 행 우클릭이
          트리 카드와 같은 `TaskNavigationMenu`를 열어야 하고, 그 상태는 이 컴포넌트가 쥐고 있다. */}
      <RecentSessions
        tasks={props.tasks}
        selectedKey={props.selectedKey}
        onOpenTask={props.onOpenTask}
        onOpenMenu={setMenu}
        questionTasks={questionTasks}
      />
      <section
        aria-label="세션 작업"
        className="pb-2"
        // 카드·헤더가 자기 메뉴를 열며 preventDefault 하므로, 남은 것이 곧 빈 곳이다.
        onContextMenu={(event) => {
          if (event.defaultPrevented) return;
          event.preventDefault();
          setMenu({ x: event.clientX, y: event.clientY, kind: "background" });
        }}
      >
        <div className="px-2.5 py-1.5 flex items-center justify-between text-[11px]">
          <span
            className="flex items-center gap-1.5 font-semibold text-text-secondary"
            title="⌘ 길게 누르기: 번호 표시 · ⌘1‥⌘9: 해당 세션으로 이동 · Delete: 열린 세션 삭제"
          >
            <span className="w-1.5 h-1.5 rounded-full bg-status-running" />
            세션 작업 ({activeTasks.length})
          </span>
        </div>
        {/* 카드마다 붙는 "워크트리 없음" 배지는 하나씩만 알린다 — 흩어진 배지를 세어 봐야 몇 건인지,
            무엇을 할 수 있는지는 알 수 없다. 정리 수단은 목록 위에 한 번만 둔다. */}
        {orphanCount > 0 && (
          <div className="mx-2.5 mb-1.5 px-2 py-1.5 rounded-md border border-dangerborder bg-dangerbg text-[11px] flex items-center justify-between gap-2">
            <span className="text-status-failed truncate">
              워크트리가 사라진 작업 {orphanCount}건
            </span>
            <button
              onClick={props.onDiscardOrphans}
              className="shrink-0 px-1.5 py-0.5 rounded bg-raised text-text-secondary hover:text-text"
              title="목록에서 종결합니다. 브랜치는 남기므로 커밋된 작업물은 되찾을 수 있습니다."
            >
              정리
            </button>
          </div>
        )}
        <div className="flex flex-col gap-1.5 px-0.5">
          {props.projects.length === 0 && (
            <div className="px-2.5 py-3 text-center text-xs text-text-muted">
              활성 세션 작업이 없습니다.
            </div>
          )}
          <SessionTaskSections
            grouped={grouped}
            unassigned={unassigned}
            groups={groups}
            drag={drag}
            renaming={renaming}
            onChange={emit}
            onRenaming={setRenaming}
            onOpenMenu={setMenu}
            renderProject={renderProject}
          />
        </div>
        <TaskNavigationMenu
          menu={menu}
          activeTasks={activeTasks}
          onClose={closeMenu}
          onDeleteTask={props.onDeleteTask}
          onMergeTask={props.onMergeTask}
          onRemoveProject={props.onRemoveProject}
          groups={groups.groups}
          groupOf={(repo) => {
            const id = groups.assignment[repo];
            return groups.groups.some((group) => group.id === id) ? id ?? null : null;
          }}
          groupCount={(id) => props.projects.filter((repo) => groups.assignment[repo] === id).length}
          onMoveToGroup={(repo, id) => emit(assignProject(groups, repo, id))}
          onMoveToNewGroup={createGroupFor}
          onCreateGroup={createEmptyGroup}
          onUnassign={(repo) => emit(unassignProject(groups, repo))}
          onRenameGroup={(id) => setRenaming({ id, created: false })}
          onDissolveGroup={(id) => emit(dissolveGroup(groups, id))}
          onSetGroupColor={(id, color) => emit(setGroupColor(groups, id, color))}
          onMoveGroup={(id, parentId) => emit(moveGroup(groups, id, parentId))}
          canMoveGroup={(id, parentId) => canMoveGroup(groups, id, parentId)}
        />
      </section>
    </>
  );
}
