// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Task } from "../../lib/ipc";
import type { TaskArchive } from "../../lib/task-archive";
import { SessionTaskNavigation } from "./SessionTaskNavigation";

const notifications = vi.hoisted(() => ({ snapshot: null }));
vi.mock("../../lib/use-notification-snapshot", () => ({
  useNotificationSnapshot: () => notifications,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

class ResizeObserverStub {
  observe() {}
  disconnect() {}
}

const repo = "/workspace/praxis";

const task = (overrides: Partial<Task> = {}): Task => ({
  id: 7,
  host: "local",
  repo,
  branch: "feature/archive",
  base: "main",
  worktree_path: `${repo}/.dojang/worktrees/task-7`,
  instruction: "보관 대상",
  state: "AwaitingReview",
  created_at: 1,
  updated_at: 1,
  mode: "conversation",
  ...overrides,
});

const archive = (overrides: Partial<TaskArchive> = {}): TaskArchive => ({
  archived_at: 3,
  branch: "feature/archive",
  head_revision: "a1b2c3d4",
  worktree_path: `${repo}/.dojang/worktrees/task-7`,
  worktree_exists: false,
  result_path: null,
  pr_url: null,
  ...overrides,
});

let container: HTMLDivElement;
let root: Root;
let opened: ReturnType<typeof vi.fn>;
let restored: ReturnType<typeof vi.fn>;
let archived: ReturnType<typeof vi.fn>;

const render = (tasks: Task[]) => {
  act(() => {
    root.render(
      <SessionTaskNavigation
        tasks={tasks}
        selectedKey={null}
        projects={[repo]}
        onOpenTask={opened}
        onNewInRepo={() => {}}
        onDeleteTask={() => {}}
        onArchiveTask={archived}
        onRestoreTask={restored}
        onRemoveProject={() => {}}
        onDiscardOrphans={() => {}}
      />,
    );
  });
};

const contextMenu = (node: Element) => {
  act(() => {
    node.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: 20,
      clientY: 30,
    }));
  });
};

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", ResizeObserverStub);
  opened = vi.fn();
  restored = vi.fn();
  archived = vi.fn();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("SessionTaskNavigation 보관 작업", () => {
  it("보관한 로컬 작업은 활성 목록과 최근·단축키 순서에서 빼고, 같은 id의 원격 작업은 남긴다", () => {
    const local = task({ archive: archive() });
    const remote = task({ host: "runner-1", state: "Running", instruction: "원격 활성", archive: null });
    render([local, remote]);

    expect(container.textContent).toContain("세션 작업 (1)");
    expect(container.textContent).toContain("원격 활성");
    expect(container.querySelector('[data-archive-task-key="local:7"]')).not.toBeNull();
    expect(container.textContent).toContain("워크트리 없음");
  });

  it("워크트리가 없더라도 보관 카드를 열고 복원 버튼은 복원 콜백으로 보낸다", () => {
    const local = task({ archive: archive() });
    render([local]);
    const row = container.querySelector<HTMLElement>('[data-archive-task-key="local:7"]')!;
    const restore = Array.from(container.querySelectorAll("button")).find((node) => node.textContent === "복원")!;

    act(() => row.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    act(() => restore.dispatchEvent(new MouseEvent("click", { bubbles: true })));

    expect(opened).toHaveBeenCalledWith(local);
    expect(restored).toHaveBeenCalledWith(local);
  });

  it("보관한 검토 대기는 복원과 실제 폴더 정리를 구분하고 머지는 보이지 않는다", () => {
    const local = task({ archive: archive() });
    render([local]);
    contextMenu(container.querySelector('[data-archive-task-key="local:7"]')!);

    const menu = document.body.querySelector<HTMLElement>('[role="menu"]')!;
    expect(menu.textContent).toContain("복원 (목록만)");
    expect(menu.textContent).toContain("버리기 (워크트리 정리)");
    expect(menu.textContent).not.toContain("이력에서 삭제");
    expect(menu.textContent).not.toContain("에 머지");

    const restore = Array.from(menu.querySelectorAll("button")).find((node) => node.textContent?.includes("복원 (목록만)"))!;
    act(() => restore.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(restored).toHaveBeenCalledWith(local);
  });

  it("원격 작업과 실행 중 작업의 메뉴에는 보관 항목이 없다", () => {
    const remote = task({ host: "runner-1", instruction: "원격 검토" });
    render([remote]);
    const remoteTitle = Array.from(container.querySelectorAll("span")).find((node) => node.textContent === "원격 검토")!;
    contextMenu(remoteTitle);
    expect(document.body.querySelector('[role="menu"]')?.textContent).not.toContain("보관 (기록·폴더 유지)");

    act(() => document.body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true })));
    const running = task({ id: 8, state: "Running", instruction: "실행 중" });
    render([running]);
    const runningTitle = Array.from(container.querySelectorAll("span")).find((node) => node.textContent === "실행 중")!;
    contextMenu(runningTitle);
    expect(document.body.querySelector('[role="menu"]')?.textContent).not.toContain("보관 (기록·폴더 유지)");
  });
});
