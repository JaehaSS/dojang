// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

import { ConversationView } from "./ConversationView";
import { DEFAULT_TRANSLATE_SETTINGS, resetTranslateStateForTest } from "../../lib/translate";
import { resetVocabStateForTest } from "../../lib/vocab";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const REPLY = "We should **rule out** the cache first. See `carry forward` in code.\n\nThen we can carry forward the fix.";
const EXPRESSIONS = [
  { phrase: "rule out", meaning: "배제하다", note: "가능성을 지울 때 쓴다." },
  { phrase: "carry forward", meaning: "이어 가다", note: "" },
];

let container: HTMLDivElement | null = null;
let root: Root | null = null;

function answer(command: string, args?: Record<string, unknown>): Promise<unknown> {
  if (command === "translate_text") {
    const { direction, text } = args as { direction: string; text: string };
    if (direction === "en_expressions") return Promise.resolve(JSON.stringify(EXPRESSIONS));
    if (direction === "en_to_ko_blocks") return Promise.resolve(JSON.stringify((JSON.parse(text) as string[]).map((b) => `번역: ${b}`)));
    return Promise.resolve(`번역: ${text}`);
  }
  if (command === "vocab_save") return Promise.resolve({ id: 1 });
  if (command === "vocab_mark_known") return Promise.resolve(null);
  return Promise.reject(new Error(command));
}

async function renderReply(study = true) {
  resetTranslateStateForTest({ ...DEFAULT_TRANSLATE_SETTINGS, study_expressions: study });
  await act(async () =>
    root?.render(
      <ConversationView conversationId={1} items={[{ role: "text", text: REPLY, complete: true }]} busy={false} translatable />,
    ),
  );
  const toggle = [...container!.querySelectorAll("button")].find((b) => b.textContent === "번역")!;
  await act(async () => toggle.click());
}

const marks = () => [...container!.querySelectorAll('[aria-label^="표현: "]')] as HTMLElement[];
const inlineMarks = () => marks().filter((m) => !m.closest('[aria-label="공부할 표현"]'));
const directions = () =>
  mocks.invoke.mock.calls.filter(([c]) => c === "translate_text").map(([, a]) => (a as { direction: string }).direction);

beforeEach(() => {
  resetVocabStateForTest();
  mocks.invoke.mockImplementation(answer);
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

describe("답변 번역의 공부할 표현", () => {
  it("번역과 나란히 표현을 불러 원문에 밑줄을 긋되 코드 안은 긋지 않는다", async () => {
    await renderReply();
    expect(directions().sort()).toEqual(["en_expressions", "en_to_ko_blocks"]);
    expect(mocks.invoke).toHaveBeenCalledWith("translate_text", { direction: "en_expressions", text: REPLY });

    // 굵게 안의 표현도 긋고, 인라인 코드의 `carry forward`는 건너뛰어 다음 문단의 것을 긋는다.
    expect(inlineMarks().map((m) => [m.textContent, m.closest("strong") !== null, m.closest("code") !== null])).toEqual([
      ["rule out", true, false],
      ["carry forward", false, false],
    ]);
    // 목록에는 짚은 표현이 모두 있다.
    const list = container!.querySelector('[aria-label="공부할 표현"]')!;
    expect(list.textContent).toContain("rule out");
    expect(list.textContent).toContain("carry forward");
  });

  it("올리면 뜻 카드가 뜨고, 저장하면 예문과 함께 단어장에 들어간다", async () => {
    await renderReply();
    const mark = inlineMarks()[0];
    await act(async () => mark.parentElement!.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })));
    const card = container!.querySelector('[role="dialog"][aria-label="rule out 뜻"]')!;
    expect(card.textContent).toContain("배제하다");
    expect(card.textContent).toContain("가능성을 지울 때 쓴다.");

    const save = [...card.querySelectorAll("button")].find((b) => b.textContent === "단어장에 저장")!;
    await act(async () => save.click());
    expect(mocks.invoke).toHaveBeenCalledWith("vocab_save", {
      entry: { phrase: "rule out", meaning: "배제하다", note: "가능성을 지울 때 쓴다.", example: "We should rule out the cache first." },
    });
    expect(card.textContent).toContain("단어장에 저장됨");
  });

  it("알아요를 누르면 그 표현의 밑줄과 목록 칩이 사라진다", async () => {
    await renderReply();
    await act(async () => inlineMarks()[0].click());
    const known = [...container!.querySelectorAll("button")].find((b) => b.textContent === "알아요")!;
    await act(async () => known.click());
    expect(mocks.invoke).toHaveBeenCalledWith("vocab_mark_known", { phrase: "rule out", meaning: "배제하다" });
    expect(marks().map((m) => m.textContent)).toEqual(["carry forward", "carry forward"]);
  });

  it("설정에서 끄면 표현을 부르지 않는다", async () => {
    await renderReply(false);
    expect(directions()).toEqual(["en_to_ko_blocks"]);
    expect(marks()).toEqual([]);
  });

  it("표현 호출이 실패해도 번역은 그대로 보인다", async () => {
    mocks.invoke.mockImplementation((command, args) =>
      (args as { direction?: string } | undefined)?.direction === "en_expressions" ? Promise.reject(new Error("timeout")) : answer(command, args),
    );
    await renderReply();
    expect(container!.querySelectorAll('[aria-label="한국어 번역"]').length).toBeGreaterThan(0);
    expect(marks()).toEqual([]);
    expect(container!.querySelector('[role="alert"]')).toBeNull();
  });
});
