import { useEffect, useState } from "react";
import type { EditorSettings, FontSettings } from "../../lib/ipc";
import { LOCAL_HOST } from "../../lib/transport";
import { McpServersView } from "../McpServersView";
import { EditorSettingsTab } from "./EditorSettingsTab";
import { MobileView } from "../MobileView";
import { SkillsView } from "../SkillsView";
import { SchedulesView } from "../SchedulesView";
import { AppearanceTab } from "./settings/AppearanceTab";
import { RunTab } from "./settings/RunTab";
import { ConnectionTab } from "./settings/ConnectionTab";
import { NotificationsTab } from "./settings/NotificationsTab";
import { SettingsSearch } from "./settings/SettingsSearch";
import { SettingHighlight } from "./settings/SettingRow";
import { MetaTag } from "../MetaTag";
import { KnowledgeView } from "../KnowledgeView";
import { VaultMemoryPanel } from "../knowledge-vault/VaultMemoryPanel";
import {
  DEFAULT_SETTINGS_TAB,
  SETTINGS_TABS,
  type SettingsTab,
} from "./settings/settings-catalog";

/** 탭 정의의 정본은 카탈로그다 — 여기서 다시 내보내는 것은 기존 import 경로를 지키기 위해서다. */
export type { SettingsTab };

/** 강조가 남아 있는 시간. 눈이 따라간 뒤에는 지워야 다음 검색의 강조가 구분된다. */
const HIGHLIGHT_MS = 1600;

interface Props {
  repo?: string;
  /** 퀵오픈처럼 특정 탭을 지목해 여는 경로. 없으면 모양새 탭. */
  initialTab?: SettingsTab;
  /** 메모리 패널이 파일형 메모리를 쓰는 에이전트. */
  agent?: string;
  fontSettings: FontSettings | null;
  onFontSettings: (s: FontSettings) => void;
  /** 파일 트리 치수·Monaco 옵션. 폰트와 나눠 두는 이유는 소비자가 다르기 때문이다. */
  editorSettings: EditorSettings;
  onEditorSettings: (s: EditorSettings) => void;
  onUseWorktreeChange: (on: boolean) => void;
}

/**
 * 설정 — 탭 바·검색·라우팅만 맡는다. 항목의 상태와 IPC는 각 탭이 소유한다.
 *
 * 전에는 18개 설정이 전부 "일반" 한 탭에 있었고 나머지 6개 탭이 관리 화면이었다.
 * 탭이 나뉘어 있다는 것과 탐색 부담이 나뉘어 있다는 것은 다른 말이다
 * (설계 2026-09-14-settings-ia-restructure-design).
 */
export function SettingsPanel({
  repo,
  initialTab,
  agent = "claude",
  fontSettings,
  onFontSettings,
  editorSettings,
  onEditorSettings,
  onUseWorktreeChange,
}: Props) {
  const [tab, setTab] = useState<SettingsTab>(initialTab ?? DEFAULT_SETTINGS_TAB);
  /** 검색이 지목한 항목 id — 컨텍스트로 내려가 해당 행이 스스로 스크롤·강조한다. */
  const [highlight, setHighlight] = useState<string | null>(null);

  useEffect(() => {
    if (!highlight) return;
    const timer = setTimeout(() => setHighlight(null), HIGHLIGHT_MS);
    return () => clearTimeout(timer);
  }, [highlight]);

  return (
    <div className="flex-1 min-h-0 flex flex-col">
      <div className="px-6 pt-4">
        <div className="mx-auto flex max-w-3xl items-center gap-2">
          {/* 설정 묶음과 관리 묶음을 구분선 하나로 가른다 — 값을 바꾸는 곳과
              환경을 확인하는 곳은 성격이 다르다(ADR 0191 결정 4). */}
          <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
            {SETTINGS_TABS.map((spec, index) => (
              <div key={spec.key} className="flex shrink-0 items-center gap-1">
                {index > 0 && spec.group !== SETTINGS_TABS[index - 1].group && (
                  <span aria-hidden className="mx-1 h-4 w-px shrink-0 bg-border" />
                )}
                <button
                  onClick={() => setTab(spec.key)}
                  className={`h-8 shrink-0 rounded-md px-3 text-sm ${
                    tab === spec.key ? "bg-raised text-text" : "text-text-secondary hover:text-text"
                  }`}
                  aria-pressed={tab === spec.key}
                >
                  <span className="flex items-center gap-1.5">
                    {spec.label}
                    {spec.badge && <MetaTag>{spec.badge}</MetaTag>}
                  </span>
                </button>
              </div>
            ))}
          </div>
          <SettingsSearch
            onPick={(entry) => {
              setTab(entry.tab);
              setHighlight(entry.id);
            }}
          />
        </div>
      </div>

      <SettingHighlight.Provider value={highlight}>
        {tab === "appearance" ? (
          <AppearanceTab fontSettings={fontSettings} onFontSettings={onFontSettings} />
        ) : tab === "editor" ? (
          <EditorSettingsTab settings={editorSettings} onChange={onEditorSettings} />
        ) : tab === "run" ? (
          <RunTab onUseWorktreeChange={onUseWorktreeChange} />
        ) : tab === "connection" ? (
          <ConnectionTab />
        ) : tab === "notifications" ? (
          <NotificationsTab />
        ) : tab === "mcp" ? (
          <McpServersView />
        ) : tab === "skills" ? (
          <SkillsView repo={repo ?? ""} onOpenMemory={() => setTab("memory")} />
        ) : tab === "knowledge" ? (
          <div className="flex-1 min-h-0 overflow-auto p-4"><KnowledgeView /></div>
        ) : tab === "memory" ? (
          <div className="flex-1 min-h-0 overflow-auto p-4"><VaultMemoryPanel agent={agent} host={LOCAL_HOST} /></div>
        ) : tab === "mobile" ? (
          <MobileView />
        ) : (
          <SchedulesView />
        )}
      </SettingHighlight.Provider>
    </div>
  );
}
