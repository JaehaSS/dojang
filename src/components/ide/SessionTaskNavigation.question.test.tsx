// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Task } from "../../lib/ipc";
import { SessionTaskNavigation } from "./SessionTaskNavigation";

const notifications = vi.hoisted(() => ({
  snapshot: null as { items: Array<{ host: string; task_id: number; kind: string }> } | null,
}));

vi.mock("../../lib/use-notification-snapshot", () => ({
  useNotificationSnapshot: () => ({ snapshot: notifications.snapshot }),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const repo = "/workspace/praxis";

const task = (id: number, state: string, extra: Partial<Task> = {}): Task => ({
  id,
  host: "local",
  repo,
  branch: `task-${id}`,
  base: "main",
  worktree_path: `/tmp/task-${id}`,
  instruction: `작업 ${id}`,
  state,
  created_at: id,
  updated_at: id,
  mode: "conversation",
  ...extra,
});

let container: HTMLDivElement;
let root: Root;

async function render(tasks: Task[], questionTasks?: ReadonlySet<string>): Promise<HTMLDivElement> {
  await act(async () => {
    root.render(
      <SessionTaskNavigation
        tasks={tasks}
        selectedKey={null}
        projects={[repo]}
        onOpenTask={() => {}}
        onNewInRepo={() => {}}
        onDeleteTask={() => {}}
        onRemoveProject={() => {}}
        onDiscardOrphans={() => {}}
        questionTasks={questionTasks}
      />,
    );
  });
  return container;
}

const dotLabel = (): string | null =>
  // 툴팁은 SVG `<title>` 자식이 싣는다 — 마크가 호버 문구를 함께 가져야 한다.
  container.querySelector('svg[aria-label^="작업 상태"]:has(> title)')?.getAttribute("aria-label") ?? null;

describe("SessionTaskNavigation 질문 대기 표시", () => {
  beforeEach(() => {
    notifications.snapshot = null;
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it("실행 중 작업에 질문이 열리면 점과 라벨이 답변 대기로 바뀐다", async () => {
    await render([task(1, "Running")], new Set(["local:1"]));
    const dot = container.querySelector('[aria-label="작업 상태: 답변 대기"]') as HTMLElement | null;
    expect(dot).not.toBeNull();
    expect(dot?.getAttribute("data-tone")).toBe("question");
    expect(container.textContent).toContain("답변");
    expect(container.textContent).not.toContain("실행 중");
    expect(dotLabel()).toBe("작업 상태: 답변 대기");
  });

  it("질문 관측이 없으면 실행 중 그대로다", async () => {
    await render([task(1, "Running")]);
    expect(container.querySelector('[aria-label="작업 상태: 실행 중…"]')).not.toBeNull();
    expect(container.textContent).not.toContain("답변");
  });

  it("같은 목록을 질문이 닫힌 관측으로 다시 그리면 실행 중으로 돌아온다", async () => {
    await render([task(1, "Running")], new Set(["local:1"]));
    expect(container.textContent).toContain("답변");
    await render([task(1, "Running")], new Set());
    expect(container.textContent).not.toContain("답변");
    expect(container.querySelector('[aria-label="작업 상태: 실행 중…"]')).not.toBeNull();
  });

  it("읽지 않은 알림 배지는 종류를 말한다 — 질문은 새 결과가 아니다", async () => {
    notifications.snapshot = { items: [
      { host: "local", task_id: 1, kind: "question" },
      { host: "local", task_id: 2, kind: "result" },
      { host: "local", task_id: 3, kind: "failure" },
    ] };
    await render([task(1, "Running"), task(2, "AwaitingReview"), task(3, "Failed")]);
    const text = container.textContent ?? "";
    expect(text).toContain("질문");
    expect(text).toContain("새 결과");
    expect(text).toContain("실패");
  });
});

// Orca 차용(2026-11-09): 사람을 기다리는 작업이 프로젝트 안에서 위로 온다. 렌더와 ⌘번호가 같은
// 배열에서 나오므로 ⌘1이 곧 가장 급한 작업이어야 한다.
describe("SessionTaskNavigation 주의 순서", () => {
  beforeEach(() => {
    notifications.snapshot = null;
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  const titles = (): string[] =>
    Array.from(container.querySelectorAll("[data-task-key]"))
      .map((node) => `작업 ${node.getAttribute("data-task-key")?.split(":")[1]}`);

  it("답변 대기 → 검토 대기 → 실행 중 순으로 그리고, 같은 순위는 받은 순서를 지킨다", async () => {
    await render(
      [task(1, "Running"), task(2, "AwaitingReview"), task(3, "Running"), task(4, "AwaitingReview", { awaiting_kind: "question" }), task(5, "AwaitingReview")],
    );
    expect(titles()).toEqual(["작업 4", "작업 2", "작업 5", "작업 1", "작업 3"]);
  });

  it("관측된 질문도 답변 대기로 올리고, ⌘1이 그 작업을 가리킨다", async () => {
    vi.useFakeTimers();
    await render([task(1, "AwaitingReview"), task(2, "Running")], new Set(["local:2"]));
    expect(titles()).toEqual(["작업 2", "작업 1"]);
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Meta", metaKey: true }));
    });
    await act(async () => {
      vi.advanceTimersByTime(400);
    });
    const first = container.querySelector('[data-shortcut="1"]')?.closest("[data-task-key]");
    expect(first?.getAttribute("data-task-key")).toBe("local:2");
  });
});
