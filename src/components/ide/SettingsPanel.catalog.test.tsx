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

/**
 * 마운트 IPC를 한 자리에서 받는다. 명령별 기본값이 필요한 것은 그 값이 없으면 섹션이
 * 아예 그려지지 않기 때문이다(캡처 프로파일이 그렇다) — 목록이 비면 이 테스트가 헛돈다.
 */
const REPLIES: Record<string, unknown> = {
  insight_enabled_get: true,
  insight_availability: {
    cards: 3,
    decks: 1,
    deck_cards: 1,
    wiki_notes: 4,
    wiki_cards: 2,
    wiki_root: "/tmp/vault",
    enabled: true,
    warnings: [],
  },
  system_fonts_list: [{ family: "Menlo", monospace: true }],
  font_settings_get: { ui_family: "", ui_size: 14, code_family: "", code_size: 13 },
  capture_profile_get: { model: "sonnet", effort: "medium", lean: true, model_raw: "", effort_raw: "" },
  block_unverified_get: false,
  lsp_autoinject_get: true,
  use_worktree_get: true,
  refresh_base_get: false,
  max_concurrent_get: { value: 3, min: 1, max: 8 },
  default_shell: { cmd: "/bin/zsh", args: ["-l"] },
  agent_models_get: {},
  debate_round_cap_get: 3,
  remote_profile_get: null,
  remote_profiles_list: [],
  notification_snapshot: { enabled: true },
  notification_permission: "granted",
  mobile_surface_status: { running: false, port: 47850, prevent_sleep: false, sleep_prevented: false },
  mobile_session_list: [],
  remote_review_commands_get: false,
};

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string) =>
    command in REPLIES ? Promise.resolve(REPLIES[command]) : Promise.reject(new Error(`stub: ${command}`)),
}));

import { SettingsPanel } from "./SettingsPanel";
import { SETTINGS_CATALOG, type SettingsTab } from "./settings/settings-catalog";

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
});

const renderTab = async (tab: SettingsTab) => {
  await act(async () => {
    root?.render(
      <SettingsPanel
        initialTab={tab}
        fontSettings={null}
        onFontSettings={() => undefined}
        editorSettings={{ tree_font_size: 13, minimap: false, word_wrap: true, tab_size: 2 }}
        onEditorSettings={() => undefined}
        onUseWorktreeChange={() => undefined}
      />,
    );
  });
  return [...(container?.querySelectorAll("[data-setting-id]") ?? [])].map(
    (node) => node.getAttribute("data-setting-id") ?? "",
  );
};

/** 카탈로그가 항목을 두고 있는 탭들 — 관리 탭 중에도 항목이 있는 곳(모바일)이 포함된다. */
const TABS_WITH_ENTRIES = [...new Set(SETTINGS_CATALOG.map((entry) => entry.tab))];

describe("설정 카탈로그와 화면의 대응", () => {
  // 카탈로그와 화면이 갈리면 검색이 없는 자리로 스크롤하고, 그 실패는 조용하다.
  it.each(TABS_WITH_ENTRIES)("%s 탭의 카탈로그 항목이 전부 화면에 있다", async (tab) => {
    const rendered = new Set(await renderTab(tab));
    for (const entry of SETTINGS_CATALOG.filter((item) => item.tab === tab)) {
      expect(rendered.has(entry.id), `${entry.id}가 화면에 없다`).toBe(true);
    }
  });

  it.each(TABS_WITH_ENTRIES)("%s 탭의 data-setting-id가 전부 카탈로그에 있다", async (tab) => {
    const known = new Set(SETTINGS_CATALOG.map((entry) => entry.id));
    for (const id of await renderTab(tab)) {
      expect(known.has(id), `${id}가 카탈로그에 없다`).toBe(true);
    }
  });
});
