// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { initialInterviewState } from "../../lib/interview";
import { initialGrillState } from "../../lib/grill";
import type { CreationState } from "../../lib/creation-stage";

const mocks = vi.hoisted(() => ({
  gitStatus: vi.fn(async () => true),
  useWorktreeOverrideGet: vi.fn(async (): Promise<boolean | null> => null),
  useWorktreeOverrideClear: vi.fn(async () => {}),
  useWorktreeSet: vi.fn(async () => {}),
}));

vi.mock("../../lib/ipc", () => ({
  skillsList: vi.fn(async () => []),
  fsTreePath: vi.fn(async () => []),
  flattenFiles: vi.fn(() => []),
  gitStatus: mocks.gitStatus,
  gitInit: vi.fn(async () => true),
  gitBranches: vi.fn(async () => ({ current: "main", branches: ["main"] })),
  pasteImageSave: vi.fn(async () => ""),
  useWorktreeOverrideGet: mocks.useWorktreeOverrideGet,
  useWorktreeOverrideClear: mocks.useWorktreeOverrideClear,
  useWorktreeSet: mocks.useWorktreeSet,
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

async function renderComposer(
  onCreate: () => void,
  opts?: { busy?: boolean; creating?: CreationState | null },
): Promise<void> {
  await act(async () => {
    root?.render(
      <Composer
        host="local"
        setHost={() => undefined}
        repo="/repo"
        setRepo={() => undefined}
        recentRepos={[]}
        agents={["codex"]}
        setAgents={() => undefined}
        resumeSession={null}
        onResumeSessionChange={() => undefined}
        model=""
        setModel={() => undefined}
        reasoningEffort=""
        setReasoningEffort={() => undefined}
        instruction="첫 줄"
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
        useWorktree
        baseBranch=""
        setBaseBranch={() => undefined}
        busy={opts?.busy ?? false}
        creating={opts?.creating ?? null}
        onCreate={onCreate}
      />,
    );
  });
}

function instructionField(): HTMLInputElement | HTMLTextAreaElement {
  const field = container?.querySelector<HTMLInputElement | HTMLTextAreaElement>(
    'input[placeholder^="작업을 설명하세요"], textarea[placeholder^="작업을 설명하세요"]',
  );
  if (!field) throw new Error("작업 지시 입력창을 찾을 수 없습니다");
  return field;
}

describe("Composer multiline keyboard contract", () => {
  it("새 작업 지시를 여러 줄 입력할 수 있는 textarea로 렌더한다", async () => {
    await renderComposer(vi.fn());
    expect(instructionField()).toBeInstanceOf(HTMLTextAreaElement);
  });

  it("Shift+Enter는 작업을 생성하지 않고 브라우저의 줄바꿈 기본 동작을 유지한다", async () => {
    const onCreate = vi.fn();
    await renderComposer(onCreate);
    const event = new KeyboardEvent("keydown", {
      key: "Enter",
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    });

    await act(async () => {
      instructionField().dispatchEvent(event);
    });

    expect(onCreate).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });

  it("일반 Enter는 줄바꿈을 막고 작업을 한 번 생성한다", async () => {
    const onCreate = vi.fn();
    await renderComposer(onCreate);
    const event = new KeyboardEvent("keydown", {
      key: "Enter",
      bubbles: true,
      cancelable: true,
    });

    await act(async () => {
      instructionField().dispatchEvent(event);
    });

    expect(event.defaultPrevented).toBe(true);
    expect(onCreate).toHaveBeenCalledOnce();
  });
});

describe("Composer busy 상태 (설계 0059 D-7)", () => {
  it("busy일 때 textarea가 readOnly가 되고 disabled는 아니다", async () => {
    await renderComposer(vi.fn(), { busy: true });
    const field = instructionField();
    expect(field.readOnly).toBe(true);
    expect(field.disabled).toBe(false);
  });

  it("busy일 때 Enter는 onCreate를 부르지 않는다(연타 방지, R-3)", async () => {
    const onCreate = vi.fn();
    await renderComposer(onCreate, { busy: true });
    const event = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });

    await act(async () => {
      instructionField().dispatchEvent(event);
    });

    expect(onCreate).not.toHaveBeenCalled();
  });

  it("busy일 때 붙여넣기를 무시한다(findImageFile이 파일을 찾아도 가로채지 않는다)", async () => {
    await renderComposer(vi.fn(), { busy: true });
    const field = instructionField();
    const file = new File(["x"], "shot.png", { type: "image/png" });
    const clipboardData = {
      items: [{ kind: "file", type: "image/png", getAsFile: () => file }],
    } as unknown as DataTransfer;
    const event = Object.assign(
      new Event("paste", { bubbles: true, cancelable: true }),
      { clipboardData },
    );

    await act(async () => {
      field.dispatchEvent(event);
    });

    expect(event.defaultPrevented).toBe(false);
  });

  it("busy이고 stage가 아직 없으면 '세션 준비 중' 상태줄을 보인다", async () => {
    await renderComposer(vi.fn(), { busy: true, creating: { ref: "r1", stage: null } });
    expect(container?.textContent).toContain("세션 준비 중");
  });

  it("stage가 오면 그 단계 라벨로 갱신된다", async () => {
    await renderComposer(vi.fn(), { busy: true, creating: { ref: "r1", stage: "worktree" } });
    expect(container?.textContent).toContain("워크트리 생성");
  });

  it("앙상블(ref null)이면 후보 수 문구를 보인다", async () => {
    await renderComposer(vi.fn(), { busy: true, creating: { ref: null, stage: null, candidates: 3 } });
    expect(container?.textContent).toContain("후보 3개");
  });

  it("idle이면 상태줄이 없다", async () => {
    await renderComposer(vi.fn(), { busy: false });
    expect(container?.textContent).not.toContain("세션 준비 중");
  });
});
