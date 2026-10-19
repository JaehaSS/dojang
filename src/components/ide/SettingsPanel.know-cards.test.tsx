// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => undefined) }));
vi.mock("../../lib/monaco", () => ({ langFromPath: () => "typescript" }));
vi.mock("@monaco-editor/react", async () => {
  const React = await import("react");
  return { default: () => React.createElement("div", { "data-testid": "monaco" }) };
});

// 부분 mock — 여기서 보는 것은 알아둘 것 카드 스위치 하나뿐이다.
const ipc = vi.hoisted(() => ({ get: vi.fn(), set: vi.fn() }));
vi.mock("../../lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../lib/ipc")>()),
  knowCardsEnabledGet: ipc.get,
  knowCardsEnabledSet: ipc.set,
}));

import { SettingsPanel } from "./SettingsPanel";

let container: HTMLDivElement | null = null;
let root: Root | null = null;

const toggle = () =>
  container?.querySelector<HTMLButtonElement>('button[role="switch"][aria-label="알아둘 것 카드"]') ?? null;
const press = async () => {
  const button = toggle();
  if (!button) throw new Error("알아둘 것 카드 스위치가 없다");
  await act(async () => button.click());
};

const mount = async () => {
  await act(async () =>
    root?.render(
      <SettingsPanel
        initialTab="run"
        fontSettings={null}
        onFontSettings={() => undefined}
        editorSettings={{ tree_font_size: 13, minimap: false, word_wrap: true, tab_size: 2 }}
        onEditorSettings={() => undefined}
        onUseWorktreeChange={() => undefined}
      />,
    ),
  );
};

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  ipc.get.mockReset().mockResolvedValue(false);
  ipc.set.mockReset().mockResolvedValue(undefined);
});

afterEach(async () => {
  await act(async () => root?.unmount());
  container?.remove();
  root = null;
  container = null;
  vi.restoreAllMocks();
});

describe("SettingsPanel 알아둘 것 카드", () => {
  it("작업 실행 탭에 비용 섹션으로 있고, 저장된 값을 보여 준다", async () => {
    await mount();
    const section = container?.querySelector('[data-setting-id="know-cards"]');
    expect(section?.textContent).toContain("알아둘 것 카드");
    expect(section?.textContent).toContain("비용");
    expect(toggle()?.getAttribute("aria-checked")).toBe("false");
  });

  it("누르면 반대 값을 저장한다", async () => {
    await mount();
    await press();
    expect(ipc.set).toHaveBeenCalledWith(true);
    expect(toggle()?.getAttribute("aria-checked")).toBe("true");
  });

  it("저장이 실패하면 스위치를 되돌린다 — 화면이 저장된 값보다 앞서면 안 된다", async () => {
    ipc.set.mockRejectedValue(new Error("db locked"));
    await mount();
    await press();
    expect(ipc.set).toHaveBeenCalledWith(true);
    expect(toggle()?.getAttribute("aria-checked")).toBe("false");
  });
});
