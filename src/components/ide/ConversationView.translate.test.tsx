// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  translateText: vi.fn<(direction: string, text: string) => Promise<string>>(),
}));

// 번역 캐시(translateReply)는 모듈 안에서 invoke를 부르므로 Tauri 경계에서 가로챈다.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: { direction: string; text: string }) =>
    command === "translate_text" ? mocks.translateText(args!.direction, args!.text) : Promise.reject(new Error(command)),
}));

import { ConversationView, type ConvoItem } from "./ConversationView";
import { DEFAULT_TRANSLATE_SETTINGS, resetTranslateStateForTest } from "../../lib/translate";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement | null = null;
let root: Root | null = null;

async function render(items: ConvoItem[], props: { busy?: boolean; translatable?: boolean; conversationId?: number } = {}) {
  await act(async () =>
    root?.render(
      <ConversationView
        conversationId={props.conversationId ?? 1}
        items={items}
        busy={props.busy ?? false}
        translatable={props.translatable ?? true}
      />,
    ),
  );
}

const buttons = (label: string) =>
  [...container!.querySelectorAll("button")].filter((b) => b.textContent === label);
const translations = () => [...container!.querySelectorAll('[aria-label="한국어 번역"]')];

async function click(el: Element) {
  await act(async () => (el as HTMLElement).click());
}

async function pressAltCmdJ() {
  await act(async () => {
    window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "∆", code: "KeyJ", metaKey: true, altKey: true }));
  });
}

beforeEach(() => {
  // 표현 짚기는 호출을 하나 더 한다 — 그 동작은 ConversationView.expressions.test.tsx가 본다.
  resetTranslateStateForTest({ ...DEFAULT_TRANSLATE_SETTINGS, study_expressions: false });
  // 대역 호출은 JSON 배열을 받아 같은 길이의 배열을 돌려준다.
  mocks.translateText.mockImplementation(async (direction, text) =>
    direction === "en_to_ko_blocks"
      ? JSON.stringify((JSON.parse(text) as string[]).map((block) => `번역: ${block}`))
      : `번역: ${text}`,
  );
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

describe("ConversationView 답변 번역", () => {
  it("번역을 원문 아래 펼치고, 다시 펼치면 호출하지 않는다", async () => {
    await render([{ role: "user", text: "고쳐줘" }, { role: "text", text: "I fixed the bug.", complete: true }]);

    await click(buttons("번역")[0]);
    expect(mocks.translateText).toHaveBeenCalledWith("en_to_ko_blocks", JSON.stringify(["I fixed the bug."]));
    expect(translations()[0]?.textContent).toContain("번역: I fixed the bug.");
    expect(container!.textContent).toContain("I fixed the bug.");

    await click(buttons("번역 접기")[0]);
    expect(translations()).toHaveLength(0);
    await click(buttons("번역")[0]);
    expect(translations()).toHaveLength(1);
    expect(mocks.translateText).toHaveBeenCalledTimes(1);
  });

  it("한국어 답변과 사용자 말풍선에는 번역 버튼을 달지 않는다", async () => {
    await render([{ role: "user", text: "fix it" }, { role: "text", text: "`App.tsx`의 버그를 고쳤습니다.", complete: true }]);
    expect(buttons("번역")).toHaveLength(0);
  });

  it("스트리밍 중인 마지막 답변의 번역 버튼은 비활성이다", async () => {
    await render([{ role: "text", text: "Working on it" }], { busy: true });
    expect(buttons("번역")[0]?.disabled).toBe(true);
  });

  it("⌥⌘J는 가장 최근 완료된 영어 답변을 펼치고 접는다", async () => {
    await render([
      { role: "text", text: "First reply.", complete: true },
      { role: "text", text: "Second reply.", complete: true },
      { role: "text", text: "아직 쓰는 중", complete: false },
    ]);

    await pressAltCmdJ();
    expect(mocks.translateText).toHaveBeenCalledWith("en_to_ko_blocks", JSON.stringify(["Second reply."]));
    expect(translations()).toHaveLength(1);

    await pressAltCmdJ();
    expect(translations()).toHaveLength(0);
  });

  it("실패하면 이유를 보이고 다시 시도할 수 있다", async () => {
    mocks.translateText.mockRejectedValueOnce("60초 안에 응답이 없습니다");
    await render([{ role: "text", text: "Done.", complete: true }]);
    await click(buttons("번역")[0]);
    expect(container!.querySelector('[role="alert"]')?.textContent).toContain("60초 안에 응답이 없습니다");

    await click(buttons("다시 시도")[0]);
    expect(translations()[0]?.textContent).toContain("번역: Done.");
  });

  it("세션을 바꾸면 펼친 번역이 따라가지 않는다", async () => {
    await render([{ role: "text", text: "Done.", complete: true }], { conversationId: 1 });
    await click(buttons("번역")[0]);
    await render([{ role: "text", text: "Other.", complete: true }], { conversationId: 2 });
    expect(translations()).toHaveLength(0);
  });

  it("영어 블록마다 바로 아래 한국어를 붙이고, 코드 블록은 보내지도 번역하지도 않는다", async () => {
    mocks.translateText.mockResolvedValue(JSON.stringify(["## 요약", "버그를 고쳤습니다.", "- 첫 단계\n- 둘째 단계"]));
    const code = "```ts\nconst x = 1;\n```";
    const reply = ["## Summary", "I fixed the bug.", code, "- first step\n- second step"].join("\n\n");
    await render([{ role: "text", text: reply, complete: true }]);
    await click(buttons("번역")[0]);

    expect(mocks.translateText).toHaveBeenCalledTimes(1);
    expect(mocks.translateText).toHaveBeenCalledWith(
      "en_to_ko_blocks",
      JSON.stringify(["## Summary", "I fixed the bug.", "- first step\n- second step"]),
    );
    // 목록은 항목마다가 아니라 통째로 한 블록이다. 제목 번역은 제목으로 그리지 않는다.
    expect(translations()).toHaveLength(3);
    expect(translations()[0].querySelector("h2")).toBeNull();
    expect(translations()[2].querySelectorAll("li")).toHaveLength(2);

    // 각 번역은 자기 원문 블록 바로 뒤에 온다.
    const reading = container!.textContent!;
    let cursor = -1;
    for (const needle of ["Summary", "요약", "I fixed the bug.", "버그를 고쳤습니다.", "const x = 1;", "first step", "첫 단계"]) {
      const at = reading.indexOf(needle, cursor + 1);
      expect(at, needle).toBeGreaterThan(cursor);
      cursor = at;
    }
  });

  it("번역 블록 개수가 어긋나면 예전처럼 전체 번역을 원문 아래 붙인다", async () => {
    mocks.translateText.mockImplementation(async (direction, text) =>
      direction === "en_to_ko_blocks" ? JSON.stringify(["하나로 합쳤습니다"]) : `전체: ${text}`,
    );
    await render([{ role: "text", text: "First.\n\nSecond.", complete: true }]);
    await click(buttons("번역")[0]);

    expect(mocks.translateText).toHaveBeenLastCalledWith("en_to_ko", "First.\n\nSecond.");
    expect(translations()).toHaveLength(1);
    expect(translations()[0].textContent).toContain("전체: First.");
  });

  it("translatable이 없으면 버튼도 단축키도 없다", async () => {
    await render([{ role: "text", text: "Done.", complete: true }], { translatable: false });
    expect(buttons("번역")).toHaveLength(0);
    await pressAltCmdJ();
    expect(mocks.translateText).not.toHaveBeenCalled();
  });
});
