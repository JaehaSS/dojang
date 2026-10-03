// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { initialGrillState } from "../../lib/grill";
import { initialInterviewState } from "../../lib/interview";

const mocks = vi.hoisted(() => ({
  gitStatus: vi.fn(async () => true),
  gitBranches: vi.fn(async () => ({ current: "main", branches: ["main", "dev"] })),
  gitCheckoutBranch: vi.fn(async (_path: string, branch: string) => ({
    current: branch,
    branches: ["main", "dev"],
  })),
  useWorktreeOverrideGet: vi.fn(async (): Promise<boolean | null> => null),
}));

vi.mock("../../lib/ipc", () => ({
  skillsList: vi.fn(async () => []),
  codexSpeedModels: vi.fn(async () => ["gpt-6-astra"]),
  fsTreePath: vi.fn(async () => []),
  flattenFiles: vi.fn(() => []),
  gitStatus: mocks.gitStatus,
  gitInit: vi.fn(async () => true),
  gitBranches: mocks.gitBranches,
  gitCheckoutBranch: mocks.gitCheckoutBranch,
  pasteImageSave: vi.fn(async () => ""),
  useWorktreeOverrideGet: mocks.useWorktreeOverrideGet,
  useWorktreeOverrideClear: vi.fn(async () => {}),
  useWorktreeSet: vi.fn(async () => {}),
}));
vi.mock("../../lib/transport", () => ({
  LOCAL_HOST: "local",
  getTransport: vi.fn(() => ({ kind: "local" })),
  hasHost: vi.fn(() => true),
  // 호스트가 하나뿐이면 컴포저는 드롭다운 대신 정적 칩을 그린다.
  listHosts: vi.fn(() => ["local"]),
}));
vi.mock("../../lib/transport/runner", () => ({
  RunnerTransport: class RunnerTransport {},
}));
vi.mock("./RepoPicker", () => ({ RepoPicker: () => null }));
vi.mock("./DirectoryPickerModal", () => ({ DirectoryPickerModal: () => null }));
vi.mock("./AgentPicker", () => ({ AgentPicker: () => null }));
vi.mock("./ModelPicker", () => ({ ModelPicker: () => null }));
vi.mock("./EffortPicker", () => ({ EffortPicker: () => null }));
vi.mock("./MentionDropdown", () => ({ MentionDropdown: () => null }));
vi.mock("./SkillDropdown", () => ({ SkillDropdown: () => null }));
vi.mock("./InterviewPanel", () => ({ InterviewPanel: () => null }));
vi.mock("./icons", () => ({ Icon: () => null }));

import { Composer } from "./Composer";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement | null = null;
let root: Root | null = null;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root?.unmount());
  container?.remove();
  root = null;
  container = null;
  vi.clearAllMocks();
});

interface Options {
  speed?: (value: "default" | "fast") => void;
  agents?: string[];
  baseBranch?: string;
  setBaseBranch?: (branch: string) => void;
  useWorktree?: boolean;
}

async function renderComposer({
  speed,
  agents = ["codex"],
  baseBranch = "",
  setBaseBranch = () => undefined,
  useWorktree = true,
}: Options = {}): Promise<void> {
  await act(async () => {
    root?.render(
      <Composer
        host="local"
        setHost={() => undefined}
        repo="/repo"
        setRepo={() => undefined}
        recentRepos={[]}
        agents={agents}
        setAgents={() => undefined}
        resumeSession={null}
        onResumeSessionChange={() => undefined}
        model="gpt-6-astra"
        serviceTier="default"
        onServiceTierChange={speed}
        setModel={() => undefined}
        reasoningEffort=""
        setReasoningEffort={() => undefined}
        instruction=""
        setInstruction={() => undefined}
        interview={initialInterviewState()}
        onInterviewStart={() => undefined}
        onInterviewAnswer={() => undefined}
        onInterviewCrystallize={() => undefined}
        onInterviewRetry={() => undefined}
        grill={initialGrillState()}
        onGrillStart={() => undefined}
        onGrillDraft={() => undefined}
        onGrillAnswer={() => undefined}
        onGrillAcceptRecommendation={() => undefined}
        onGrillDontKnow={() => undefined}
        onGrillEndNow={() => undefined}
        onGrillApplyAndScore={() => undefined}
        onGrillApplyInstruction={() => undefined}
        onGrillSave={() => undefined}
        onGrillRetry={() => undefined}
        useWorktree={useWorktree}
        baseBranch={baseBranch}
        setBaseBranch={setBaseBranch}
        busy={false}
        creating={null}
        onCreate={() => undefined}
      />,
    );
  });
}

/** base 브랜치 칩 — 자기 브랜치명을 라벨로 갖는 유일한 버튼이다. */
function branchChip(label: string): HTMLButtonElement | null {
  return (
    [...(container?.querySelectorAll("button") ?? [])].find((b) =>
      b.textContent?.trim().startsWith(label),
    ) as HTMLButtonElement | undefined
  ) ?? null;
}

describe("Composer의 base 브랜치 선택", () => {
  it("고르기 전에는 레포가 체크아웃한 브랜치를 보여준다", async () => {
    await renderComposer();
    expect(branchChip("main")).not.toBeNull();
  });

  it("메뉴에서 고른 브랜치를 호출부에 올린다", async () => {
    const setBaseBranch = vi.fn();
    await renderComposer({ setBaseBranch });

    await act(async () => branchChip("main")?.click());
    // 메뉴에는 브랜치가 항목으로 뜬다 — 칩과 같은 라벨을 쓰는 dev를 고른다.
    await act(async () => branchChip("dev")?.click());

    expect(setBaseBranch).toHaveBeenCalledWith("dev");
  });

  it("격리 실행은 다른 base를 골라도 승인 머지 대상을 알린다", async () => {
    await renderComposer({ baseBranch: "dev" });

    const chip = branchChip("dev");
    expect(chip?.title).toContain("시작");
    expect(chip?.title).toContain("머지");
    expect(chip?.title).toContain("dev");
    expect(chip?.title).not.toContain("전환");
  });

  it("직접 실행에서도 시작할 로컬 브랜치를 고를 수 있다", async () => {
    const setBaseBranch = vi.fn();
    await renderComposer({ useWorktree: false, setBaseBranch });

    await act(async () => branchChip("main")?.click());
    await act(async () => branchChip("dev")?.click());

    expect(setBaseBranch).toHaveBeenCalledWith("dev");
  });

  it("직접 실행은 다른 base를 고르면 체크아웃 전환을 알린다", async () => {
    await renderComposer({ useWorktree: false, baseBranch: "dev" });

    expect(branchChip("dev")?.title).toContain("전환");
  });

  it("메뉴에서 체크아웃하면 메인 체크아웃을 전환하고 새 현재 브랜치를 따른다", async () => {
    const setBaseBranch = vi.fn();
    await renderComposer({ setBaseBranch });

    await act(async () => branchChip("main")?.click());
    const search = container!.querySelector<HTMLInputElement>("input[role='combobox']")!;
    await act(async () => {
      search.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    });
    const button = [...container!.querySelectorAll("button")].find((b) => b.textContent === "체크아웃");
    await act(async () => button?.click());

    expect(mocks.gitCheckoutBranch).toHaveBeenCalledWith("/repo", "dev");
    expect(setBaseBranch).toHaveBeenCalledWith("");
    expect(branchChip("dev")).not.toBeNull();
  });

  it("체크아웃이 실패하면 이유를 보여준다", async () => {
    mocks.gitCheckoutBranch.mockRejectedValueOnce("작업 #7이 아직 끝나지 않아 브랜치를 전환할 수 없습니다");
    await renderComposer();

    await act(async () => branchChip("main")?.click());
    const search = container!.querySelector<HTMLInputElement>("input[role='combobox']")!;
    await act(async () => {
      search.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    });
    await act(async () => {
      search.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, bubbles: true }));
    });

    expect(container!.textContent).toContain("브랜치 전환 실패");
    expect(container!.textContent).toContain("#7");
  });

  it("git 저장소가 아니면 숨긴다", async () => {
    mocks.gitStatus.mockResolvedValueOnce(false);
    await renderComposer();
    expect(branchChip("main")).toBeNull();
  });
});


it("새 로컬 Codex 작업에서 Fast를 선택할 수 있다", async () => {
  const speed = vi.fn();
  await renderComposer({ speed });
  const select = container!.querySelector<HTMLSelectElement>('[aria-label="Codex 실행 속도"]')!;
  expect(select.value).toBe("default");
  await act(async () => { select.value="fast"; select.dispatchEvent(new Event("change",{bubbles:true})); });
  expect(speed).toHaveBeenCalledWith("fast");
});
it("다른 공급자와 앙상블에는 속도 선택을 표시하지 않는다", async () => {
  for (const agents of [["claude"], ["codex", "claude"]]) {
    await renderComposer({ agents, speed: vi.fn() });
    expect(container!.querySelector('[aria-label="Codex 실행 속도"]')).toBeNull();
  }
});
