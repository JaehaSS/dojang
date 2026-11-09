// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Task } from "../../lib/ipc";
import { TaskArchiveList } from "./TaskArchiveList";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const archived = (overrides: Partial<Task> = {}): Task => ({
  id: 7,
  host: "local",
  repo: "/workspace/praxis",
  branch: "feature/live-name",
  base: "main",
  worktree_path: "/workspace/praxis/.dojang/worktrees/task-7",
  instruction: "보관한 작업",
  state: "Done",
  created_at: 1,
  updated_at: 2,
  mode: "conversation",
  archive: {
    archived_at: 3,
    branch: "feature/snapshot",
    head_revision: "a1b2c3d4e5f6",
    worktree_path: "/workspace/praxis/.dojang/worktrees/task-7",
    worktree_exists: false,
    result_path: "/workspace/praxis/result.md",
    pr_url: "https://github.com/acme/praxis/pull/7",
  },
  ...overrides,
});

let host: HTMLDivElement;
let root: Root;

const render = (task: Task, opened = vi.fn(), restored = vi.fn()) => {
  act(() => {
    root.render(
      <TaskArchiveList
        tasks={[task]}
        selectedKey={null}
        onOpenTask={opened}
        onRestoreTask={restored}
      />,
    );
  });
  return { opened, restored };
};

beforeEach(() => {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe("TaskArchiveList", () => {
  it("워크트리가 없다는 상태와 보관 당시의 복구 단서를 표시한다", () => {
    render(archived());

    expect(host.textContent).toContain("보관한 작업 (1)");
    expect(host.textContent).toContain("feature/snapshot");
    expect(host.textContent).toContain("HEAD a1b2c3d4e5f6");
    expect(host.textContent).toContain("워크트리 없음");
    expect(host.textContent).toContain("결과 경로:");
    expect(host.textContent).toContain("PR 열기");
    expect(host.textContent).toContain("목록만 되돌리며 워크트리를 다시 만들지 않습니다.");
  });

  it("워크트리가 없어도 카드를 열어 이력을 표시하고 복원은 별도 콜백으로 보낸다", () => {
    const { opened, restored } = render(archived());
    const row = host.querySelector<HTMLElement>('[data-archive-task-key="local:7"]')!;
    const restore = Array.from(host.querySelectorAll("button")).find((node) => node.textContent === "복원")!;

    act(() => row.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    act(() => restore.dispatchEvent(new MouseEvent("click", { bubbles: true })));

    expect(opened).toHaveBeenCalledWith(archived());
    expect(restored).toHaveBeenCalledWith(archived());
  });

  it("헤더를 누르면 보관 목록을 접고 다시 펼친다", () => {
    render(archived());
    const header = Array.from(host.querySelectorAll("button")).find((node) => node.textContent?.includes("보관한 작업"))!;

    act(() => header.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(host.querySelector('[data-archive-task-key="local:7"]')).toBeNull();

    act(() => header.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(host.querySelector('[data-archive-task-key="local:7"]')).not.toBeNull();
  });
  it("확인하지 못한 commit과 위험한 PR 링크를 사실대로 표시한다", () => {
    const candidate = archived();
    candidate.archive = { ...candidate.archive!, head_revision: null, pr_url: "javascript:alert(1)" };
    const { opened } = render(candidate);
    expect(host.textContent).toContain("커밋 미확인");
    expect(host.textContent).not.toContain("PR 열기");
    const row = host.querySelector<HTMLElement>("[data-archive-task-key]")!;
    act(() => row.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(opened).toHaveBeenCalledWith(candidate);
  });

});
