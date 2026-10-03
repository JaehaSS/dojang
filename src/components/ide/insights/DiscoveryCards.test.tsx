// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SignalCard, Task } from "../../../lib/ipc";

vi.mock("../../../lib/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../lib/ipc")>();
  return { ...actual, insightCardDismiss: vi.fn(), insightMemoryAppend: vi.fn() };
});
vi.mock("../../../lib/memory-file-ipc", () => ({ memoryFileOpen: vi.fn() }));
vi.mock("../../../lib/project-editor-ipc", () => ({ projectEditorOpen: vi.fn() }));

import { insightCardDismiss, insightMemoryAppend } from "../../../lib/ipc";
import { memoryFileOpen } from "../../../lib/memory-file-ipc";
import { projectEditorOpen } from "../../../lib/project-editor-ipc";
import { DiscoveryCards } from "./DiscoveryCards";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement | null = null;
let root: Root | null = null;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  vi.mocked(insightCardDismiss).mockReset().mockResolvedValue(undefined);
  vi.mocked(insightMemoryAppend).mockReset().mockResolvedValue({ kind: "Appended" });
  vi.mocked(memoryFileOpen).mockReset();
  vi.mocked(projectEditorOpen).mockReset();
});

afterEach(async () => {
  await act(async () => root?.unmount());
  container?.remove();
  root = null;
  container = null;
});

const task = (id: number): Task => ({
  id,
  host: "local",
  repo: "repo-a",
  branch: "b",
  base: "main",
  worktree_path: "/tmp/wt",
  instruction: `작업 ${id} 지시문`,
  state: "done",
  created_at: 0,
  updated_at: 0,
  mode: "output",
});

const card = (overrides: Partial<SignalCard> = {}): SignalCard => ({
  key: "S1:7d:2026-09-21",
  signal: "S1",
  sentence: "입력 1회 이하로 머지 없이 닫은 작업이 8건(80%)이다. 직전 구간 2건",
  value: 8,
  baseline: 2,
  sample: 10,
  evidence: { kind: "Tasks", ids: [1] },
  repo: null,
  ...overrides,
});

interface RenderOpts {
  cards?: SignalCard[];
  onDismissed?: (key: string) => void;
  tasks?: Task[];
  onOpenTask?: (t: Task) => void;
  onJumpToSpend?: () => void;
  onStartResearch?: (instruction: string, repo: string | null) => void;
}

async function render(opts: RenderOpts = {}): Promise<void> {
  await act(async () => {
    root?.render(
      <DiscoveryCards
        cards={opts.cards ?? [card()]}
        onDismissed={opts.onDismissed ?? (() => {})}
        tasks={opts.tasks ?? [task(1)]}
        onOpenTask={opts.onOpenTask ?? (() => {})}
        onJumpToSpend={opts.onJumpToSpend ?? (() => {})}
        onStartResearch={opts.onStartResearch ?? (() => {})}
      />,
    );
  });
}

const buttons = (): HTMLButtonElement[] => [...(container?.querySelectorAll("button") ?? [])];
const byText = (text: string): HTMLButtonElement | undefined =>
  buttons().find((b) => b.textContent?.trim() === text);

describe("DiscoveryCards", () => {
  it("카드마다 근거 열기 · 메모리에 남기기 · 조사 작업 시작 · 무시 버튼이 보인다", async () => {
    await render();
    for (const label of ["근거 열기", "메모리에 남기기", "조사 작업 시작", "무시"]) {
      expect(byText(label), `${label} 버튼이 있어야 한다`).toBeTruthy();
    }
  });

  it("메모리 다이얼로그 확인이 고른 대상으로 insightMemoryAppend를 부른다", async () => {
    await render({ cards: [card({ repo: "repo-a" })] });
    await act(async () => byText("메모리에 남기기")?.click());
    // 저장소가 있는 카드는 기본 대상이 그 저장소다(§8 O2).
    await act(async () => byText("저장")?.click());
    expect(insightMemoryAppend).toHaveBeenCalledWith(
      { kind: "Repo", repo: "repo-a" },
      expect.any(String),
    );

    vi.mocked(insightMemoryAppend).mockClear();
    // 저장이 성공하면 패널을 접는다 — 대상을 바꾸려면 다시 연다.
    await act(async () => byText("메모리에 남기기")?.click());
    await act(async () => byText("USER.md")?.click());
    await act(async () => byText("저장")?.click());
    expect(insightMemoryAppend).toHaveBeenCalledWith({ kind: "User" }, expect.any(String));
  });

  it("Full 결과면 안내 문구를 보여주고 파일을 연다", async () => {
    vi.mocked(insightMemoryAppend).mockResolvedValue({ kind: "Full", path: "/vault/USER.md" });
    await render();
    await act(async () => byText("메모리에 남기기")?.click());
    await act(async () => byText("저장")?.click());
    expect(memoryFileOpen).toHaveBeenCalledWith("/vault/USER.md");
    expect(container?.textContent).toContain("상한을 넘어");
  });

  it("조사 작업 시작은 문장을 미리 채우기만 하고 다른 ipc를 부르지 않는다", async () => {
    const onStartResearch = vi.fn();
    await render({ cards: [card({ repo: "repo-a" })], onStartResearch });
    await act(async () => byText("조사 작업 시작")?.click());
    expect(onStartResearch).toHaveBeenCalledTimes(1);
    const [instruction, repo] = onStartResearch.mock.calls[0] as [string, string | null];
    expect(instruction).toContain("입력 1회 이하로 머지 없이 닫은 작업");
    expect(repo).toBe("repo-a");
    expect(insightMemoryAppend).not.toHaveBeenCalled();
    expect(insightCardDismiss).not.toHaveBeenCalled();
  });

  it("무시하면 카드가 즉시 목록에서 빠진다", async () => {
    const onDismissed = vi.fn();
    await render({ cards: [card({ signal: "S2", repo: "repo-a" })], onDismissed });
    await act(async () => byText("무시")?.click());
    expect(insightCardDismiss).toHaveBeenCalledWith("S2", "repo-a");
    expect(onDismissed).toHaveBeenCalledWith("S1:7d:2026-09-21");
  });

  it("Ledger 근거는 저장소 창을 열고, Spend 근거는 지출 섹션으로 스크롤한다", async () => {
    const onJumpToSpend = vi.fn();
    await render({
      cards: [card({ evidence: { kind: "Ledger", repo: "repo-a", numbers: [12] }, repo: "repo-a" })],
    });
    await act(async () => byText("근거 열기")?.click());
    expect(projectEditorOpen).toHaveBeenCalledWith("repo-a");

    await render({ cards: [card({ evidence: { kind: "Spend", project: "repo-a" } })], onJumpToSpend });
    await act(async () => byText("근거 열기")?.click());
    expect(onJumpToSpend).toHaveBeenCalledTimes(1);
  });
});
