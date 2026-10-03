// @vitest-environment jsdom

import { act, useEffect, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { initialInterviewState } from "../../lib/interview";
import { initialGrillState } from "../../lib/grill";
import type { HostId } from "../../lib/transport";

const mocks = vi.hoisted(() => ({
  translateText: vi.fn<(direction: string, text: string) => Promise<string>>(),
  onCreate: vi.fn(),
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
vi.mock("../../lib/translate", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../lib/translate")>()),
  translateText: mocks.translateText,
}));
vi.mock("../../lib/transport", () => ({
  LOCAL_HOST: "local",
  getTransport: vi.fn(() => ({ kind: "local" })),
  hasHost: vi.fn(() => true),
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
import { resetTranslateStateForTest, type ComposerTranslateMode } from "../../lib/translate";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement | null = null;
let root: Root | null = null;

/** 래퍼가 밖으로 꺼내 주는 setInstruction — 훅을 거치지 않은 외부 값 변경을 흉내 낸다. */
const external: {
  set: ((next: string) => void) | null;
  switchHost: ((next: HostId, draft: string) => void) | null;
} = { set: null, switchHost: null };

beforeEach(() => {
  external.set = null;
  external.switchHost = null;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root?.unmount());
  container?.remove();
  root = null;
  container = null;
  vi.useRealTimers();
  vi.clearAllMocks();
});

/** instruction을 스스로 들고 있는 래퍼 — 바꿔 넣기는 controlled 값이 실제로 바뀌어야 성립한다. */
function Harness({ initial = "", busy = false }: { initial?: string; busy?: boolean }) {
  const [instruction, setInstruction] = useState(initial);
  const [host, setHost] = useState<HostId>("local");
  useEffect(() => {
    external.set = setInstruction;
    // 호스트 전환은 App이 한 핸들러에서 호스트와 지시문을 함께 바꾼다 — 한 커밋이다.
    external.switchHost = (next, draft) => {
      setHost(next);
      setInstruction(draft);
    };
  }, []);
  return (
    <Composer
      host={host}
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
      instruction={instruction}
      setInstruction={setInstruction}
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
      busy={busy}
      creating={null}
      onCreate={mocks.onCreate}
    />
  );
}

async function renderHarness(opts?: { initial?: string; busy?: boolean }): Promise<void> {
  await act(async () => {
    root?.render(<Harness initial={opts?.initial} busy={opts?.busy} />);
  });
}

function instructionField(): HTMLTextAreaElement {
  const field = container?.querySelector<HTMLTextAreaElement>(
    'textarea[placeholder^="작업을 설명하세요"]',
  );
  if (!field) throw new Error("작업 지시 입력창을 찾을 수 없습니다");
  return field;
}

const nativeValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;

/** controlled textarea에 사용자 입력을 흉내 낸다 — React value tracker를 우회해야 onChange가 뜬다. */
async function typeValue(next: string): Promise<void> {
  const field = instructionField();
  await act(async () => {
    nativeValue.call(field, next);
    field.setSelectionRange(next.length, next.length);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function press(init: KeyboardEventInit): Promise<void> {
  await act(async () => {
    instructionField().dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  });
}

function deferred() {
  let resolve!: (v: string) => void;
  const promise = new Promise<string>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

async function mount(mode: ComposerTranslateMode, opts?: { busy?: boolean; initial?: string }): Promise<void> {
  resetTranslateStateForTest({ composer_mode: mode, model: "sonnet", study_expressions: false });
  await renderHarness(opts);
}

const cmdJ = { key: "j", code: "KeyJ", metaKey: true };
const query = (label: string) => container!.querySelector(`[aria-label="${label}"]`);

describe("홈 Composer ⌘J 영어로 다듬기", () => {
  it("참고 모드는 영어 제안만 보여 주고 작업을 만들지 않는다", async () => {
    mocks.translateText.mockResolvedValueOnce("Fix the sidebar");
    await mount("reference");
    await typeValue("사이드바 고쳐줘");

    await press(cmdJ);
    expect(mocks.translateText).toHaveBeenCalledWith("ko_to_en_prompt", "사이드바 고쳐줘");
    expect(query("영어 제안")?.textContent).toContain("Fix the sidebar");
    expect(instructionField().value).toBe("사이드바 고쳐줘");
    expect(mocks.onCreate).not.toHaveBeenCalled();
  });

  it("바꿔 넣기 모드는 지시문을 영어로 바꾸고 ⌘Z 한 번에 한국어로 돌아온다", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(0);
    const reply = deferred();
    mocks.translateText.mockReturnValueOnce(reply.promise);
    await mount("replace");
    await typeValue("사이드바 고쳐줘");
    await press(cmdJ);
    vi.setSystemTime(3_000);
    await act(async () => reply.resolve("Fix the sidebar"));

    expect(instructionField().value).toBe("Fix the sidebar");
    expect(query("한국어 원문")?.textContent).toContain("사이드바 고쳐줘");

    await press({ key: "z", code: "KeyZ", metaKey: true });
    expect(instructionField().value).toBe("사이드바 고쳐줘");
  });

  it("결과가 오기 전에 호스트를 바꾸면 새 호스트의 지시문에 들어가지 않는다", async () => {
    const reply = deferred();
    mocks.translateText.mockReturnValueOnce(reply.promise);
    await mount("replace");
    await typeValue("사이드바 고쳐줘");
    await press(cmdJ);

    await act(async () => external.switchHost?.("remote" as HostId, "원격 초안"));
    await act(async () => reply.resolve("Fix the sidebar"));

    expect(instructionField().value).toBe("원격 초안");
    expect(container!.textContent).not.toContain("Fix the sidebar");
  });

  it("작업을 만드는 중(readOnly)에는 호출하지 않는다", async () => {
    await mount("replace", { busy: true, initial: "사이드바 고쳐줘" });
    await press(cmdJ);
    expect(mocks.translateText).not.toHaveBeenCalled();
  });
});
