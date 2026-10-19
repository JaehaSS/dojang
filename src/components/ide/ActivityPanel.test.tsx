import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { Task } from "../../lib/ipc";
import { ActivityPanel } from "./ActivityPanel";

const task: Task = {
  id: 7,
  host: "local",
  repo: "/workspace/praxis",
  branch: "feature/context-panel",
  base: "main",
  worktree_path: "/workspace/praxis/.praxis/worktrees/task-7",
  instruction: "작업 컨텍스트를 활동 탭에 연결한다",
  state: "Running",
  created_at: 1,
  updated_at: 2,
  mode: "conversation",
};

describe("ActivityPanel", () => {
  it("작업 컨텍스트를 현재 활동과 서브에이전트보다 먼저 보여준다", () => {
    const html = renderToStaticMarkup(
      <ActivityPanel
        task={task}
        runtime="local"
        diff={{ state: "ready", value: "2 files changed, 12 insertions(+), 3 deletions(-)" }}
        items={[
          {
            role: "tool",
            name: "Task",
            summary: "접근성 검토",
            toolId: "toolu_agent",
          },
        ]}
        busy
        activity={null}
        onRefreshDiff={() => {}}
        onOpenConversation={() => {}}
        onOpenSubagent={() => {}}
      />,
    );

    expect(html.indexOf("환경")).toBeLessThan(html.indexOf("현재 활동"));
    expect(html.indexOf("현재 활동")).toBeLessThan(html.indexOf("하위 에이전트"));
    expect(html).toContain("+12");
    expect(html).toContain("접근성 검토");
    expect(html).toContain("최근 활동");
  });

  it("터미널 작업에는 존재하지 않는 대화 화면 진입점을 만들지 않는다", () => {
    const html = renderToStaticMarkup(
      <ActivityPanel
        task={{ ...task, mode: "terminal" }}
        runtime="local"
        diff={{ state: "ready", value: "" }}
        items={[]}
        busy={false}
        activity={null}
        onRefreshDiff={() => {}}
        onOpenSubagent={() => {}}
      />,
    );

    expect(html).not.toContain("대화 전체 보기");
  });

  it("플로팅 채널 밀도에서는 하위 에이전트를 접고 최근 활동은 패널 진입점으로만 둔다", () => {
    const html = renderToStaticMarkup(
      <ActivityPanel
        task={task}
        runtime="local"
        diff={{ state: "ready", value: "2 files changed, 12 insertions(+), 3 deletions(-)" }}
        items={[
          { role: "tool", name: "Task", summary: "접근성 검토", toolId: "toolu_agent" },
          { role: "tool", name: "Read", summary: "src/App.tsx" },
        ]}
        busy
        activity={null}
        onRefreshDiff={() => {}}
        onOpenConversation={() => {}}
        onOpenSubagent={() => {}}
        onOpenRecentActivity={() => {}}
        density="rail"
      />,
    );

    // 환경과 현재 활동은 플로팅 채널에서도 그대로 — 곁눈으로 봐야 하는 정보다.
    expect(html).toContain("환경");
    expect(html).toContain("+12");
    expect(html).toContain("현재 활동");
    // 하위 에이전트는 접힌다 — 헤더는 남고 항목은 렌더되지 않는다.
    expect(html).toContain("하위 에이전트");
    expect(html).not.toContain("접근성 검토");
    expect(html).toContain('aria-expanded="false"');
    // 최근 활동 본문은 채널에 두지 않는다 — 진입점만 남기고 사이드 패널이 맡는다.
    expect(html).toContain("사이드 패널에서 최근 활동 보기");
    // 목록 항목 전용 마크업이 없어야 한다 ("현재 활동"의 최근 operation 표시와 구분).
    expect(html).not.toContain("border-l border-border pl-2.5");
    expect(html).not.toContain("이 대화에서 기록된 도구 활동이 없습니다");
    // 대화가 바로 아래에 있으므로 진입점을 만들지 않는다.
    expect(html).not.toContain("대화 전체 보기");
  });

  it("최근 활동 진입점이 없으면 채널에 아무것도 렌더하지 않는다", () => {
    const html = renderToStaticMarkup(
      <ActivityPanel
        task={task}
        runtime="local"
        diff={{ state: "ready", value: "" }}
        items={[]}
        busy={false}
        activity={null}
        onRefreshDiff={() => {}}
        onOpenSubagent={() => {}}
        density="rail"
      />,
    );

    expect(html).not.toContain("최근 활동");
  });

  it("Codex의 관측 미상은 부모 턴 종료만으로 중단으로 바꾸지 않는다", () => {
    const html = renderToStaticMarkup(
      <ActivityPanel
        task={task}
        runtime="local"
        diff={{ state: "ready", value: "" }}
        items={[{ role: "subagent_state", subagentId: "codex:root:child", summary: "기록 점검", state: "unknown", observationStatus: "limited" }]}
        busy={false}
        activity={null}
        onRefreshDiff={() => {}}
        onOpenSubagent={() => {}}
      />,
    );

    expect(html).toContain("완료 확인 불가");
    expect(html).not.toContain("턴 종료로 중단됨");
  });
});
