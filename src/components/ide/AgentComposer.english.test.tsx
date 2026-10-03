// @vitest-environment jsdom

import { act, useEffect, useLayoutEffect, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  removeCapture: vi.fn(async () => undefined),
  skillsList: vi.fn(async () => []),
  translateText: vi.fn<(direction: string, text: string) => Promise<string>>(),
}));

vi.mock("../../lib/ipc", () => ({
  designmodeRemoveCapture: mocks.removeCapture,
  skillsList: mocks.skillsList,
}));

vi.mock("../../lib/translate", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../lib/translate")>()),
  translateText: mocks.translateText,
}));

import { AgentComposer } from "./AgentComposer";
import { resetTranslateStateForTest, type ComposerTranslateMode } from "../../lib/translate";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement | null = null;
let root: Root | null = null;
const swap: { to: ((key: string) => void) | null } = { to: null };
const store = new Map<string, string>();
const onSend = vi.fn();

function deferred() {
  let resolve!: (v: string) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<string>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** useSessionDraft와 같은 모양 — 세션 키마다 초안이 따로 산다. */
function Harness() {
  const [key, setKey] = useState("a");
  const [value, setValue] = useState("");
  useEffect(() => {
    swap.to = setKey;
  }, []);
  useLayoutEffect(() => {
    setValue(store.get(key) ?? "");
  }, [key]);
  const write = (next: string): void => {
    setValue(next);
    store.set(key, next);
  };
  return (
    <AgentComposer
      value={value}
      onChange={write}
      draftKey={key}
      onSend={(text) => {
        onSend(text);
        write("");
      }}
      onInterrupt={() => {}}
      onHistory={() => {}}
      files={[]}
      repo="/repo"
      taskId={7}
      host="local"
    />
  );
}

/** 표현 짚기는 호출을 하나 더 한다 — 켠 동작은 아래 "공부할 표현" 테스트만 본다. */
async function mount(mode: ComposerTranslateMode, study = false): Promise<void> {
  resetTranslateStateForTest({ composer_mode: mode, model: "sonnet", study_expressions: study });
  await act(async () => root?.render(<Harness />));
}

function field(): HTMLTextAreaElement {
  return container!.querySelector<HTMLTextAreaElement>('textarea[placeholder^="에이전트에게 질의"]')!;
}

const nativeValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;

async function typeValue(next: string): Promise<void> {
  await act(async () => {
    nativeValue.call(field(), next);
    field().setSelectionRange(next.length, next.length);
    field().dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function press(init: KeyboardEventInit): Promise<void> {
  await act(async () => {
    field().dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  });
}

const cmdJ = { key: "j", code: "KeyJ", metaKey: true };
const query = (label: string) => container!.querySelector(`[aria-label="${label}"]`);

beforeEach(() => {
  store.clear();
  swap.to = null;
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

describe("AgentComposer ⌘J 공부할 표현", () => {
  const expressions = JSON.stringify([{ phrase: "double-check", meaning: "재확인하다", note: "" }]);
  const answer = (english: string) => async (direction: string) => (direction === "en_expressions" ? expressions : english);

  it("참고 모드의 영어 제안에서 표현에 밑줄을 긋는다", async () => {
    mocks.translateText.mockImplementation(answer("Please double-check the sidebar"));
    await mount("reference", true);
    await typeValue("사이드바 다시 확인해줘");
    await press(cmdJ);

    expect(mocks.translateText).toHaveBeenCalledWith("en_expressions", "Please double-check the sidebar");
    const suggestion = query("영어 제안")!;
    expect(suggestion.textContent).toContain("Please double-check the sidebar");
    const mark = suggestion.querySelector('[aria-label="표현: double-check"]')!;
    expect(mark.closest('[aria-label="공부할 표현"]')).toBeNull();
    expect(suggestion.querySelector('[aria-label="공부할 표현"]')?.textContent).toContain("double-check");
  });

  it("바꿔 넣기 모드는 입력창 대신 목록으로 표현을 보여 준다", async () => {
    mocks.translateText.mockImplementation(answer("Please double-check the sidebar"));
    await mount("replace", true);
    await typeValue("사이드바 다시 확인해줘");
    await press(cmdJ);

    expect(field().value).toBe("Please double-check the sidebar");
    expect(query("공부할 표현")?.textContent).toContain("double-check");
  });
});

describe("AgentComposer ⌘J 영어로 다듬기", () => {
  it("참고 모드는 영어 제안을 보여 주기만 하고 초안과 전송은 건드리지 않는다", async () => {
    const reply = deferred();
    mocks.translateText.mockReturnValueOnce(reply.promise);
    await mount("reference");
    await typeValue("@src/App.tsx 에서 /review 돌려줘");

    await press(cmdJ);
    expect(mocks.translateText).toHaveBeenCalledWith("ko_to_en_prompt", "@src/App.tsx 에서 /review 돌려줘");
    expect(container!.textContent).toContain("영어로 바꾸는 중");

    await act(async () => reply.resolve("Run /review on @src/App.tsx"));
    expect(query("영어 제안")?.textContent).toContain("Run /review on @src/App.tsx");
    expect(field().value).toBe("@src/App.tsx 에서 /review 돌려줘");
    expect(onSend).not.toHaveBeenCalled();
  });

  it("보내면 제안 패널이 걷힌다", async () => {
    mocks.translateText.mockResolvedValueOnce("Fix the sidebar");
    await mount("reference");
    await typeValue("사이드바 고쳐줘");
    await press(cmdJ);
    expect(query("영어 제안")).not.toBeNull();

    await typeValue("Fix the sidebar");
    await press({ key: "Enter", code: "Enter" });
    expect(onSend).toHaveBeenCalledWith("Fix the sidebar");
    expect(query("영어 제안")).toBeNull();
  });

  it("바꿔 넣기 모드는 초안을 영어로 바꾸고 ⌘Z 한 번에 한국어로 돌아온다", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(0);
    const reply = deferred();
    mocks.translateText.mockReturnValueOnce(reply.promise);
    await mount("replace");
    await typeValue("사이드바 고쳐줘");
    await press(cmdJ);
    // 실제 호출은 수 초 걸린다 — 타이핑 묶음(800ms)과 섞이지 않는 시점에 도착시킨다.
    vi.setSystemTime(3_000);
    await act(async () => reply.resolve("Fix the sidebar"));

    expect(field().value).toBe("Fix the sidebar");
    expect(query("한국어 원문")?.textContent).toContain("사이드바 고쳐줘");
    expect(onSend).not.toHaveBeenCalled();

    await press({ key: "z", code: "KeyZ", metaKey: true });
    expect(field().value).toBe("사이드바 고쳐줘");
  });

  it("결과가 오기 전에 세션을 바꾸면 어느 세션 초안에도 들어가지 않는다", async () => {
    const reply = deferred();
    mocks.translateText.mockReturnValueOnce(reply.promise);
    await mount("replace");
    await typeValue("사이드바 고쳐줘");
    await press(cmdJ);

    await act(async () => swap.to?.("b"));
    await act(async () => reply.resolve("Fix the sidebar"));

    expect(field().value).toBe("");
    expect(store.get("a")).toBe("사이드바 고쳐줘");
    expect(container!.textContent).not.toContain("Fix the sidebar");
  });

  it("기다리는 동안 초안을 고쳤으면 덮어쓰지 않고 알린다", async () => {
    const reply = deferred();
    mocks.translateText.mockReturnValueOnce(reply.promise);
    await mount("replace");
    await typeValue("사이드바 고쳐줘");
    await press(cmdJ);
    await typeValue("사이드바 고쳐줘 그리고 테스트도");
    await act(async () => reply.resolve("Fix the sidebar"));

    expect(field().value).toBe("사이드바 고쳐줘 그리고 테스트도");
    expect(container!.textContent).toContain("초안이 달라져 적용하지 않았습니다");
  });

  it("한글 조합 중 ⌘J와 한국어가 없는 초안은 호출하지 않는다", async () => {
    await mount("replace");
    await typeValue("사이드바");
    await press({ ...cmdJ, isComposing: true });
    await press({ ...cmdJ, keyCode: 229 });
    expect(mocks.translateText).not.toHaveBeenCalled();

    await typeValue("fix the sidebar");
    await press(cmdJ);
    expect(mocks.translateText).not.toHaveBeenCalled();
    expect(container!.textContent).toContain("한국어가 없어 바꾸지 않았습니다");
  });

  it("실패하면 이유를 보여 주고 초안은 그대로 둔다", async () => {
    mocks.translateText.mockRejectedValueOnce("claude 실행 실패: not found");
    await mount("replace");
    await typeValue("사이드바 고쳐줘");
    await press(cmdJ);

    expect(container!.querySelector('[role="alert"]')?.textContent).toContain("claude 실행 실패");
    expect(field().value).toBe("사이드바 고쳐줘");
  });

  it("진행 중에 ⌘J를 다시 누르면 취소되고 늦게 온 결과는 버린다", async () => {
    const reply = deferred();
    mocks.translateText.mockReturnValueOnce(reply.promise);
    await mount("reference");
    await typeValue("사이드바 고쳐줘");
    await press(cmdJ);
    await press(cmdJ);
    await act(async () => reply.resolve("Fix the sidebar"));

    expect(query("영어 제안")).toBeNull();
    expect(container!.textContent).not.toContain("영어로 바꾸는 중");
  });
});
