import { useEffect, useRef, useState } from "react";
import {
  systemFontsList,
  fontSettingsGet,
  fontSettingsSet,
  type FontInfo,
  type FontSettings,
} from "../../../lib/ipc";
import { codeFontStack, uiFontStack } from "../../../lib/fonts";
import { useTheme } from "../../../lib/use-theme";
import { useExperimentalFeatures, setExperimentalFeaturesEnabled } from "../../../lib/experimental-features";
import { ThemeSection } from "./ThemeSection";
import { SettingRow, SettingSection, SettingsTabShell, Switch, TabSummary } from "./SettingRow";

const DEFAULT_FONT_SETTINGS: FontSettings = {
  ui_family: "",
  ui_size: 14,
  code_family: "",
  code_size: 13,
};

const numberInput =
  "w-16 bg-bg border border-border rounded px-2 py-1 text-sm text-text outline-none focus:border-primary";

interface Props {
  fontSettings: FontSettings | null;
  onFontSettings: (s: FontSettings) => void;
}

/**
 * 모양새 — 테마·폰트. 눈에 보이는 것을 바꾸는 설정만 여기 온다.
 * 대기 인사이트는 설정 › 지식 탭에 있다(`InsightSection`) — 내용이 지식창고에서 온다.
 */
export function AppearanceTab({ fontSettings, onFontSettings }: Props) {
  const theme = useTheme();
  const advancedFeatures = useExperimentalFeatures();
  const [systemFonts, setSystemFonts] = useState<FontInfo[]>([]);
  const [font, setFont] = useState<FontSettings>(fontSettings ?? DEFAULT_FONT_SETTINGS);

  // 마지막으로 저장(또는 로드)에 성공한 값 — 저장 실패 시 롤백 기준.
  const lastSavedFontRef = useRef<FontSettings>(fontSettings ?? DEFAULT_FONT_SETTINGS);

  useEffect(() => {
    // 매번 재스캔 — 마운트(탭 열림) 시점에 새로 설치된 폰트도 반영.
    systemFontsList().then(setSystemFonts).catch(() => {});
    fontSettingsGet()
      .then((s) => {
        setFont(s);
        lastSavedFontRef.current = s;
      })
      .catch(() => {});
  }, []);

  // 폰트 설정 확정(blur/Enter) — 성공 시 App의 applyFonts로 전 화면 반영, 실패 시 이전 값 롤백.
  // 크기는 저장 전 프론트에서도 clamp — 입력을 비우면 Number("")===0이 상태에 들어와
  // 백엔드 clamp와 무관하게 화면에 0px가 적용되는 것을 막는다(범위는 백엔드와 동일).
  const commitFont = async () => {
    const clampN = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, n));
    const next = {
      ...font,
      code_size: clampN(font.code_size, 10, 24),
      ui_size: clampN(font.ui_size, 12, 16),
    };
    if (JSON.stringify(next) === JSON.stringify(lastSavedFontRef.current)) {
      setFont(next);
      return;
    }
    try {
      await fontSettingsSet(next);
      lastSavedFontRef.current = next;
      setFont(next);
      onFontSettings(next);
    } catch {
      setFont(lastSavedFontRef.current);
    }
  };

  return (
    <SettingsTabShell>
      <TabSummary
        items={[
          `테마 ${theme.label}`,
          `코드 ${font.code_family || "기본"} ${font.code_size}px`,
          `UI ${font.ui_family || "기본"} ${font.ui_size}px`,
        ]}
      />

      <SettingSection
        id="theme"
        title="테마"
        hint="UI·에디터·터미널 색을 한 번에 바꾼다. ⌘ 팔레트의 “테마 전환”은 계열을 유지한 채 밝기만 뒤집는다."
      >
        <ThemeSection />
      </SettingSection>

      <SettingSection
        id="font"
        title="폰트"
        hint="에디터·터미널의 코드 폰트와 앱 UI 폰트. 목록에 없어도 직접 입력할 수 있습니다."
      >
        <SettingRow id="font-code" title="코드 폰트">
          <input
            aria-label="코드 폰트"
            className="w-40 bg-bg border border-border rounded px-2 py-1 text-sm text-text outline-none focus:border-primary font-code"
            list="praxis-fonts-mono"
            value={font.code_family}
            onChange={(e) => setFont((f) => ({ ...f, code_family: e.target.value }))}
            onBlur={() => void commitFont()}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
            placeholder="기본"
          />
          <input
            type="number"
            aria-label="코드 폰트 크기"
            min={10}
            max={24}
            className={numberInput}
            value={font.code_size}
            onChange={(e) => {
              const n = Number(e.target.value);
              if (!Number.isNaN(n)) setFont((f) => ({ ...f, code_size: n }));
            }}
            onBlur={() => void commitFont()}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          />
        </SettingRow>

        <SettingRow id="font-ui" title="UI 폰트">
          <input
            aria-label="UI 폰트"
            className="w-40 bg-bg border border-border rounded px-2 py-1 text-sm text-text outline-none focus:border-primary"
            list="praxis-fonts-all"
            value={font.ui_family}
            onChange={(e) => setFont((f) => ({ ...f, ui_family: e.target.value }))}
            onBlur={() => void commitFont()}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
            placeholder="기본"
          />
          <input
            type="number"
            aria-label="UI 폰트 크기"
            min={12}
            max={16}
            className={numberInput}
            value={font.ui_size}
            onChange={(e) => {
              const n = Number(e.target.value);
              if (!Number.isNaN(n)) setFont((f) => ({ ...f, ui_size: n }));
            }}
            onBlur={() => void commitFont()}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          />
        </SettingRow>

        <datalist id="praxis-fonts-mono">
          {systemFonts.map((f) => (
            <option key={f.family} value={f.family} />
          ))}
        </datalist>
        <datalist id="praxis-fonts-all">
          {systemFonts.map((f) => (
            <option key={f.family} value={f.family} />
          ))}
        </datalist>

        <div className="rounded border border-border bg-raised p-3 space-y-1 overflow-hidden">
          <div style={{ fontFamily: codeFontStack(font.code_family), fontSize: font.code_size }}>
            {"안녕하세요 Dojang != => 0O1lI {}[]"}
          </div>
          <div style={{ fontFamily: uiFontStack(font.ui_family), fontSize: font.ui_size }}>
            빠른 갈색 여우 The quick brown fox
          </div>
        </div>
      </SettingSection>

      <SettingSection
        id="advanced-features"
        title="고급"
        hint="개발자용·실험 기능의 노출 여부."
      >
        <SettingRow
          id="advanced-features"
          title="고급·실험 기능 표시"
          hint="캐시 관측치·앙상블 실험 지표·Design Mode처럼 개발자용·실험 단계인 기능을 화면에 보여준다. 꺼도 기능 자체는 남아 있고 다시 켜면 그대로 나온다."
        >
          <Switch
            on={advancedFeatures}
            onClick={() => setExperimentalFeaturesEnabled(!advancedFeatures)}
            label="고급·실험 기능 표시"
          />
        </SettingRow>
      </SettingSection>
    </SettingsTabShell>
  );
}
