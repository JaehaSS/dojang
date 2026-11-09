import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { Task } from "../../lib/ipc";
import { HomeView } from "./HomeView";

const task: Task = {
  id: 7,
  host: "local",
  repo: "/workspace/praxis",
  branch: "task-7",
  base: "main",
  worktree_path: "/tmp/task-7",
  instruction: "메인 홈 작업",
  state: "Running",
  created_at: 7,
  updated_at: 7,
  agent: "codex",
  mode: "conversation",
};

describe("HomeView", () => {
  it("원본 프로젝트 에디터의 진입점을 보인다", () => {
    const html = renderToStaticMarkup(
      <HomeView tasks={[task]} onOpenTask={() => {}} onOpenEnsemble={() => {}} onRefresh={() => {}} onOpenProject={async (root) => root} />,
    );

    expect(html).toContain("에디터 열기");
    expect(html).toContain("작업 없이 원본 폴더");
  });

  it("걷어낸 실행 씬들이 되살아나지 않는다", () => {
    const html = renderToStaticMarkup(
      <HomeView
        tasks={[task]}
        onOpenTask={() => {}}
        onOpenEnsemble={() => {}}
        onRefresh={() => {}}
      />,
    );

    // 마을·어항·스튜디오는 차례로 걷어냈다. 셋 다 여기서만 되살아날 수 있다.
    expect(html).not.toContain("에이전트 마을");
    expect(html).not.toContain("에이전트 어항");
    expect(html).not.toContain("에이전트 스튜디오");
    expect(html).not.toContain("pixel-office");
  });

  it("앙상블은 '비교 실행' 별도 섹션이 아니라 최근 안의 한 행으로 들어온다", () => {
    const html = renderToStaticMarkup(
      <HomeView
        tasks={[
          task,
          { ...task, id: 11, ensemble: "cmp", agent: "claude", state: "Done" },
          { ...task, id: 12, ensemble: "cmp", agent: "codex", state: "Running" },
        ]}
        onOpenTask={() => {}}
        onOpenEnsemble={() => {}}
        onRefresh={() => {}}
      />,
    );

    // 섹션은 사라졌지만 앙상블 진입로는 최근 목록 안에 남아 있어야 한다.
    expect(html).not.toContain("비교 실행");
    expect(html).toContain("최근");
    expect(html).toContain("1/2 완료");
  });

  it("완료·버림 후보만 든 앙상블은 최근에 렌더하지 않는다", () => {
    const html = renderToStaticMarkup(
      <HomeView
        tasks={[
          { ...task, id: 11, ensemble: "settled", instruction: "완료된 비교", state: "Done" },
          { ...task, id: 12, ensemble: "settled", instruction: "버린 비교", state: "Discarded" },
        ]}
        onOpenTask={() => {}}
        onOpenEnsemble={() => {}}
        onRefresh={() => {}}
      />,
    );

    expect(html).not.toContain("완료된 비교");
    expect(html).not.toContain("버린 비교");
    expect(html).toContain("표시할 작업이 없습니다. 아래 입력창에서 새 작업을 시작하세요.");
  });
  it.each(["Done", "Failed", "Discarded"])("%s 종료 이력에도 기록 열기와 별도 보관 진입점을 보인다", (state) => {
    const html = renderToStaticMarkup(<HomeView
      tasks={[{ ...task, instruction: "완료 기록", state }]}
      onOpenTask={() => {}} onOpenEnsemble={() => {}} onRefresh={() => {}} onArchiveTask={() => {}}
    />);
    expect(html).toContain("종료 작업 보관 (1)");
    expect(html).toContain("완료 기록 보관");
    expect(html).toContain("기록·폴더·브랜치를 유지합니다");
  });

});
