// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * Wiki 채널을 지운 뒤(2026-09-28) 그 안에 살던 지식 연결과 메모리는 설정 탭이 됐다.
 * 탭 본문은 각자 테스트가 있으므로 여기서는 **도달 경로**만 본다.
 */
vi.mock("../KnowledgeView", () => ({ KnowledgeView: () => <div data-testid="knowledge-view" /> }));
vi.mock("../knowledge-vault/VaultMemoryPanel", () => ({
  VaultMemoryPanel: ({ agent, host }: { agent: string; host: string }) => <div data-testid="memory-panel">{agent}@{host}</div>,
}));
vi.mock("../SkillsView", () => ({
  SkillsView: ({ onOpenMemory }: { onOpenMemory?: () => void }) => (
    <button type="button" data-testid="open-memory" onClick={onOpenMemory}>메모리 열기</button>
  ),
}));
vi.mock("./settings/SettingsSearch", () => ({ SettingsSearch: () => null }));
vi.mock("./settings/AppearanceTab", () => ({ AppearanceTab: () => null }));

import { SettingsPanel } from "./SettingsPanel";
import type { SettingsTab } from "./settings/settings-catalog";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

const render = (initialTab?: SettingsTab) =>
  act(async () => root.render(
    <SettingsPanel
      initialTab={initialTab}
      agent="codex"
      fontSettings={null}
      onFontSettings={() => {}}
      editorSettings={{} as never}
      onEditorSettings={() => {}}
      onUseWorktreeChange={() => {}}
    />,
  ));

const tabButton = (label: string) =>
  [...container.querySelectorAll("button")].find((b) => b.textContent?.trim() === label);

describe("설정의 지식·메모리 탭", () => {
  it("지식 탭이 옛 Wiki › 연결 화면을 연다", async () => {
    await render();
    await act(async () => tabButton("지식")?.click());
    expect(container.querySelector('[data-testid="knowledge-view"]')).not.toBeNull();
  });

  it("메모리 탭은 선택한 에이전트로 로컬 메모리를 연다", async () => {
    await render("memory");
    expect(container.querySelector('[data-testid="memory-panel"]')?.textContent).toBe("codex@local");
  });

  it("스킬 탭의 메모리 열기는 설정 안의 메모리 탭으로 옮긴다", async () => {
    await render("skills");
    await act(async () => (container.querySelector('[data-testid="open-memory"]') as HTMLButtonElement).click());
    expect(container.querySelector('[data-testid="memory-panel"]')).not.toBeNull();
    expect(tabButton("메모리")?.getAttribute("aria-pressed")).toBe("true");
  });
});
