// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

import { VocabView } from "./VocabView";
import type { VocabEntry } from "../../lib/vocab";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const entry = (id: number, phrase: string, extra: Partial<VocabEntry> = {}): VocabEntry => ({
  id,
  phrase,
  meaning: `${phrase} 뜻`,
  note: null,
  example: `We ${phrase} it.`,
  status: "learning",
  box_level: 0,
  due_at: 0,
  created_at: 0,
  reviewed_at: null,
  ...extra,
});

let container: HTMLDivElement | null = null;
let root: Root | null = null;

const button = (label: string) => [...container!.querySelectorAll("button")].find((b) => b.textContent === label)!;
const click = async (el: HTMLElement) => act(async () => el.click());

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

describe("단어장 복습", () => {
  it("예문을 먼저 보이고, 뜻을 연 뒤 채점하면 다음 카드로 넘어간다", async () => {
    mocks.invoke.mockImplementation(async (command) =>
      command === "vocab_due" ? [entry(1, "rule out"), entry(2, "carry forward")] : command === "vocab_review" ? entry(1, "rule out") : null,
    );
    await act(async () => root?.render(<VocabView />));

    const card = () => container!.querySelector('[aria-label="복습 카드"]')!;
    expect(card().textContent).toContain("rule out");
    expect(card().textContent).toContain("We rule out it.");
    expect(card().textContent).not.toContain("rule out 뜻");

    await click(button("뜻 보기"));
    expect(card().textContent).toContain("rule out 뜻");
    await click(button("기억났어요"));
    expect(mocks.invoke).toHaveBeenCalledWith("vocab_review", { id: 1, remembered: true });
    expect(card().textContent).toContain("carry forward");

    await click(button("뜻 보기"));
    await click(button("다시 볼게요"));
    expect(mocks.invoke).toHaveBeenCalledWith("vocab_review", { id: 2, remembered: false });
    expect(container!.textContent).toContain("2개를 복습했습니다");
  });

  it("채점 응답 전에 다시 눌러도 한 번만 채점하고 다음 카드를 건너뛰지 않는다", async () => {
    let finish!: () => void;
    mocks.invoke.mockImplementation((command) =>
      command === "vocab_due"
        ? Promise.resolve([entry(1, "rule out"), entry(2, "carry forward")])
        : new Promise((resolve) => (finish = () => resolve(null))),
    );
    await act(async () => root?.render(<VocabView />));
    await click(button("뜻 보기"));
    await click(button("기억났어요"));
    await click(button("기억났어요"));
    await act(async () => finish());

    expect(mocks.invoke.mock.calls.filter(([c]) => c === "vocab_review")).toHaveLength(1);
    expect(container!.querySelector('[aria-label="복습 카드"]')!.textContent).toContain("carry forward");
  });

  it("전체 목록에서 알아요·삭제를 하고 다시 읽는다", async () => {
    let rows = [entry(1, "rule out", { due_at: Math.floor(Date.now() / 1000) + 3 * 86400 }), entry(2, "take over", { status: "known" })];
    mocks.invoke.mockImplementation(async (command, args) => {
      if (command === "vocab_due") return [];
      if (command === "vocab_list") return rows;
      if (command === "vocab_delete") rows = rows.filter((r) => r.id !== (args as { id: number }).id);
      return null;
    });
    await act(async () => root?.render(<VocabView />));
    expect(container!.textContent).toContain("지금 복습할 표현이 없습니다");

    await click(button("전체 목록"));
    const list = () => container!.querySelector('[aria-label="저장한 표현"]')!;
    expect(list().textContent).toContain("3일 뒤 복습");
    expect(list().textContent).toContain("다시 복습");

    await click(button("알아요"));
    expect(mocks.invoke).toHaveBeenCalledWith("vocab_set_status", { id: 1, status: "known" });
    await click(container!.querySelector<HTMLButtonElement>('[aria-label="take over 삭제"]')!);
    expect(mocks.invoke).toHaveBeenCalledWith("vocab_delete", { id: 2 });
    expect(list().textContent).not.toContain("take over");
  });
});
