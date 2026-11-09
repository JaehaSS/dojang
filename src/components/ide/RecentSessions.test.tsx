// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Task } from "../../lib/ipc";
import { RecentSessions } from "./RecentSessions";
import type { TaskNavigationMenuState } from "./TaskNavigationMenu";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const NOW = 1_700_000_000;

const task = (overrides: Partial<Task> = {}): Task => ({
  id: 1,
  host: "local",
  repo: "/workspace/praxis",
  branch: "feature/one",
  base: "main",
  worktree_path: "/workspace/praxis/.praxis/worktrees/task-1",
  instruction: "사이드바 정리",
  state: "AwaitingReview",
  created_at: NOW - 5_000,
  updated_at: NOW - 60,
  mode: "conversation",
  ...overrides,
});

const render = (tasks: Task[], selectedKey: string | null = null): string =>
  renderToStaticMarkup(
    <RecentSessions tasks={tasks} selectedKey={selectedKey} onOpenTask={() => {}} nowSec={NOW} />,
  );

describe("RecentSessions", () => {
  // "최근 세션 (0)"은 자리만 차지한다 — 창 안에 아무것도 없으면 섹션 자체가 없어야 한다(결정 2).
  it("창 안에 세션이 없으면 아무 마크업도 그리지 않는다", () => {
    expect(render([task({ updated_at: NOW - 7_200 })])).toBe("");
    expect(render([])).toBe("");
  });

  it("창 안에 세션이 있으면 건수와 설명을 단 헤더를 그린다", () => {
    const html = render([task()]);

    expect(html).toContain('aria-label="최근 세션"');
    expect(html).toContain("최근 세션 (1)");
    expect(html).toContain('title="지난 1시간 안에 대화한 세션"');
  });

  // 찾고 있는 것은 "어느 프로젝트의 세션인가"다 — 프로젝트명이 제목보다 앞에 와야 한다.
  it("프로젝트명을 세션 제목 앞에 둔다", () => {
    const html = render([task()]);

    expect(html.indexOf("praxis")).toBeLessThan(html.indexOf("사이드바 정리"));
    expect(html).toMatch(/<span class="shrink-0 font-medium"[^>]*>praxis<\/span>/);
  });

  it("instruction이 비면 브랜치명을 대신 적는다", () => {
    expect(render([task({ instruction: "" })])).toContain("feature/one");
  });

  // 로컬 3번과 원격 3번은 다른 작업이다 — 원격에만 이름표를 붙여 구분한다.
  it("원격 세션에만 호스트 배지를 단다", () => {
    const remote = render([task({ id: 3, host: "workstation" })]);
    const local = render([task({ id: 3 })]);

    expect(remote).toContain('title="원격 호스트: workstation"');
    expect(local).not.toContain("원격 호스트");
  });

  it("선택된 행은 트리 행과 같은 선택 스타일을 쓴다", () => {
    const html = render([task({ id: 4 }), task({ id: 5, updated_at: NOW - 120 })], "local:4");

    expect(html).toContain('aria-current="true"');
    expect(html).toContain("bg-raised text-text");
    expect(html).not.toContain("border-primary");
    expect(html).toContain('data-task-key="local:4"');
  });

  it("최신순으로 그린다", () => {
    const html = render([
      task({ id: 1, instruction: "오래된 쪽", updated_at: NOW - 600 }),
      task({ id: 2, instruction: "최근 쪽", updated_at: NOW - 10 }),
    ]);

    expect(html.indexOf("최근 쪽")).toBeLessThan(html.indexOf("오래된 쪽"));
  });

  it("상태 점에 완전한 상태 문구를 적는다", () => {
    expect(render([task({ state: "AwaitingReview", awaiting_kind: "question" })])).toContain(
      'aria-label="작업 상태: 답변 대기"',
    );
  });
});

describe("RecentSessions 상호작용", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => {
      root.unmount();
    });
    host.remove();
    vi.useRealTimers();
  });

  it("행을 클릭하면 그 작업으로 onOpenTask를 부른다", () => {
    const opened: number[] = [];
    act(() => {
      root.render(
        <RecentSessions
          tasks={[task({ id: 9 })]}
          selectedKey={null}
          onOpenTask={(t) => opened.push(t.id)}
          nowSec={NOW}
        />,
      );
    });

    const row = host.querySelector<HTMLDivElement>('[data-task-key="local:9"]');
    act(() => {
      row?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(opened).toEqual([9]);
  });

  // 트리 카드와 같은 메뉴를 열어야 한다 — 여기서 워크트리를 버리는 길이 트리와 달라지면 안 된다.
  it("행을 우클릭하면 기본 메뉴를 막고 그 작업의 task 메뉴 상태를 넘긴다", () => {
    const opened: TaskNavigationMenuState[] = [];
    act(() => {
      root.render(
        <RecentSessions
          tasks={[task({ id: 9 })]}
          selectedKey={null}
          onOpenTask={() => {}}
          onOpenMenu={(menu) => opened.push(menu)}
          nowSec={NOW}
        />,
      );
    });

    const row = host.querySelector<HTMLDivElement>('[data-task-key="local:9"]')!;
    const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 20, clientY: 30 });
    act(() => {
      row.dispatchEvent(event);
    });

    expect(event.defaultPrevented).toBe(true);
    expect(opened).toHaveLength(1);
    expect(opened[0]).toMatchObject({ x: 20, y: 30, kind: "task", task: { id: 9 } });
  });

  it("onOpenMenu가 없으면 우클릭을 가로채지 않는다", () => {
    act(() => {
      root.render(
        <RecentSessions tasks={[task({ id: 9 })]} selectedKey={null} onOpenTask={() => {}} nowSec={NOW} />,
      );
    });

    const row = host.querySelector<HTMLDivElement>('[data-task-key="local:9"]')!;
    const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    act(() => {
      row.dispatchEvent(event);
    });

    expect(event.defaultPrevented).toBe(false);
  });

  // 목록 갱신은 이벤트 구동이라, 시계를 직접 돌리지 않으면 한 시간 지난 세션이 그대로 남는다.
  it("nowSec 없이 쓰면 주기적으로 다시 판정해 창 밖 세션을 떨어뜨린다", () => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW * 1000);
    act(() => {
      root.render(
        <RecentSessions
          tasks={[task({ id: 9, updated_at: NOW - 3_590 })]}
          selectedKey={null}
          onOpenTask={() => {}}
        />,
      );
    });
    expect(host.textContent).toContain("최근 세션 (1)");

    vi.setSystemTime((NOW + 120) * 1000);
    act(() => {
      vi.advanceTimersByTime(120_000);
    });

    expect(host.innerHTML).toBe("");
  });
});
