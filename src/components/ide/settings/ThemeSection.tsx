import { useState, useSyncExternalStore } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  applyTheme,
  getTheme,
  getThemesRevision,
  subscribeTheme,
  DEFAULT_THEME_ID,
  THEMES,
  type Theme,
} from "../../../lib/themes";
import {
  customIdFromLabel,
  customSpecs,
  deleteCustomTheme,
  exportCustomTheme,
  parseThemeSpec,
  readThemeFile,
  saveCustomTheme,
  specFromTheme,
  uniqueCustomId,
  type CustomThemeSpec,
} from "../../../lib/theme-files";
import { useTheme } from "../../../lib/use-theme";
import { ThemeEditor } from "../ThemeEditor";
import { Icon, type IconName } from "../icons";

/**
 * 테마 미리보기 — 패널·본문·구문색과 액센트·상태색을 작은 작업 공간으로 그린다.
 * 이름만으로는 팔레트를 고를 수 없고, 여기 색은 활성 테마와 무관해야 하므로
 * Tailwind 토큰이 아니라 그 테마의 값을 인라인으로 쓴다.
 */
function ThemeCard({
  theme,
  active,
  onPick,
  actions,
}: {
  theme: Theme;
  active: boolean;
  onPick: () => void;
  /** 커스텀 카드의 손잡이들. 카드 자체가 버튼이라 중첩되지 않도록 형제로 놓는다. */
  actions?: React.ReactNode;
}) {
  const t = theme.tokens;
  return (
    <div
      className={`flex items-center gap-1.5 rounded-lg border p-2 transition-colors ${
        active ? "border-primary bg-surface" : "border-border hover:border-border-strong"
      }`}
    >
      <button
        type="button"
        onClick={onPick}
        aria-pressed={active}
        className="flex min-w-0 flex-1 flex-col gap-2.5 rounded-md text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-primary"
      >
        <span
          aria-hidden="true"
          className="flex h-24 w-full overflow-hidden rounded-md border"
          style={{ background: t.bg, borderColor: t.border, color: t.text }}
        >
          <span className="flex w-1/4 flex-col gap-2 border-r p-2" style={{ background: t.surface, borderColor: t.border }}>
            <span className="h-1.5 w-3/4 rounded" style={{ background: t.primary }} />
            <span className="h-1 w-full rounded" style={{ background: t.borderStrong }} />
            <span className="h-1 w-2/3 rounded" style={{ background: t.borderStrong }} />
          </span>
          <span className="flex min-w-0 flex-1 flex-col gap-1.5 p-2.5">
            <span className="text-[11px] font-medium">나의 작업 공간</span>
            <span className="truncate font-mono text-[11px]" style={{ color: theme.syntax?.keyword ?? t.question }}>
              const <span style={{ color: theme.syntax?.variable ?? t.text }}>idea</span> = <span style={{ color: theme.syntax?.string ?? t.done }}>"hello"</span>
            </span>
            <span className="mt-auto flex items-center gap-1.5">
              {[t.primary, t.running, t.awaiting, t.question, t.done, t.failed].map((c, i) => (
                <span key={i} className="h-2 w-2 rounded-full" style={{ background: c }} />
              ))}
              <span className="ml-auto h-3 w-8 rounded" style={{ background: t.raised, border: `1px solid ${t.borderStrong}` }} />
            </span>
          </span>
        </span>
        <span className="w-full min-w-0">
          <span className="flex items-center gap-2 text-[13px] text-text">
            <span className="truncate font-medium">{theme.label}</span>
            {active && <span className="ml-auto inline-flex shrink-0 items-center gap-1 text-[11px] text-primary"><Icon name="check" size={12} />적용 중</span>}
          </span>
          <span className="mt-0.5 block text-[11px] leading-relaxed text-text-muted">{theme.blurb}</span>
        </span>
      </button>
      {actions}
    </div>
  );
}

/** 카드 위 소형 손잡이 (DESIGN.md IconButton 문법 — 기본 secondary, hover는 raised 배경). */
function CardAction({
  icon,
  title,
  onClick,
}: {
  icon: IconName;
  title: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      onClick={onClick}
      className="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded text-text-secondary hover:bg-text/[.08] hover:text-text"
    >
      <Icon name={icon} size={14} />
    </button>
  );
}

/**
 * 테마 그리드 — 빌트인·커스텀 두 소제목. 편집 화면은 이 섹션의 로컬 상태다: 돌아올 자리가
 * 곧 카드가 있던 자리라 설정 탭 전체를 갈아끼우지 않는다.
 */
export function ThemeSection() {
  const activeTheme = useTheme();
  // 활성 테마가 그대로여도 목록은 바뀐다(삭제·가져오기) — 그 경우 활성 스냅샷은 움직이지 않으므로
  // 레지스트리 revision을 따로 구독해야 그리드가 다시 그려진다.
  useSyncExternalStore(subscribeTheme, getThemesRevision);
  const [editing, setEditing] = useState<CustomThemeSpec | null>(null);
  const [filter, setFilter] = useState<"all" | "light" | "dark">("all");
  const [error, setError] = useState<string | null>(null);

  const customs = customSpecs();
  const takenIds = customs.map((s) => s.id);
  const run = (job: Promise<void>) => {
    setError(null);
    job.catch((e: unknown) => setError(String(e)));
  };

  async function removeTheme(spec: CustomThemeSpec) {
    if (!window.confirm(`테마 “${spec.label}”을 삭제할까요?`)) return;
    // 활성 테마를 지우면 갈 곳이 사라진다 — 먼저 기본 테마로 옮기고 지운다.
    if (activeTheme.id === spec.id) applyTheme(DEFAULT_THEME_ID);
    await deleteCustomTheme(spec.id);
  }

  async function exportTheme(spec: CustomThemeSpec) {
    const dest = await save({
      defaultPath: `${spec.id}.json`,
      filters: [{ name: "테마 JSON", extensions: ["json"] }],
    });
    if (!dest) return;
    await exportCustomTheme(spec.id, dest);
  }

  async function importTheme() {
    const src = await open({ multiple: false, filters: [{ name: "테마 JSON", extensions: ["json"] }] });
    if (typeof src !== "string") return;
    const parsed = parseThemeSpec(await readThemeFile(src));
    if (!parsed) {
      setError("테마 파일을 읽지 못했습니다 — schema 1 형식인지 확인하세요.");
      return;
    }
    // 같은 테마를 두 번 가져와도 먼저 것을 덮지 않는다.
    // 목록에 더할 뿐 적용하지는 않는다 — 파일을 하나 읽었다고 쓰던 테마가 바뀌면 놀란다.
    await saveCustomTheme({ ...parsed, id: uniqueCustomId(parsed.id, takenIds) }, { apply: false });
  }

  if (editing) return <ThemeEditor initial={editing} onDone={() => setEditing(null)} />;

  const duplicateOf = (spec: CustomThemeSpec): CustomThemeSpec => {
    const label = `${spec.label} 사본`;
    return { ...spec, id: uniqueCustomId(customIdFromLabel(label), takenIds), label };
  };

  return (
    <>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="text-xs text-text-secondary">기본 테마 · {THEMES.length}종</span>
        <div className="flex gap-1" role="group" aria-label="테마 밝기">
          {([
            ["all", "전체"], ["light", "라이트"], ["dark", "다크"],
          ] as const).map(([value, label]) => (
            <button
              key={value}
              type="button"
              aria-pressed={filter === value}
              onClick={() => setFilter(value)}
              className={`rounded-md border px-2.5 py-1 text-xs ${filter === value ? "border-primary bg-surface text-text" : "border-border text-text-secondary hover:text-text"}`}
            >
              {label}
            </button>
          ))}
        </div>
      </div>
      <div className="text-[11px] text-text-muted">미리보기를 누르면 바로 적용됩니다. 마음에 드는 테마를 복제해 색을 더 바꿀 수도 있어요.</div>
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        {THEMES.filter((t) => filter === "all" || t.kind === filter).map((t) => (
          <ThemeCard
            key={t.id}
            theme={t}
            active={t.id === activeTheme.id}
            onPick={() => applyTheme(t.id)}
          />
        ))}
      </div>

      <div className="flex items-center gap-2">
        <span className="text-xs text-text-secondary">커스텀</span>
        <span className="ml-auto flex items-center gap-1.5">
          <button
            type="button"
            onClick={() => setEditing(specFromTheme(activeTheme, `${activeTheme.label} 사본`, takenIds))}
            className="h-7 rounded-md border border-border px-2.5 text-sm text-text-secondary hover:text-text"
          >
            + 새 테마
          </button>
          <button
            type="button"
            onClick={() => run(importTheme())}
            className="h-7 rounded-md border border-border px-2.5 text-sm text-text-secondary hover:text-text"
          >
            가져오기
          </button>
        </span>
      </div>

      {customs.length === 0 ? (
        <div className="text-[11px] text-text-muted">
          아직 없다. “새 테마”는 지금 쓰는 테마를 복제해 편집 화면으로 들어간다.
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
          {customs.map((spec) => (
            <ThemeCard
              key={spec.id}
              theme={getTheme(spec.id)}
              active={spec.id === activeTheme.id}
              onPick={() => applyTheme(spec.id)}
              actions={
                <span className="flex shrink-0 flex-col">
                  <span className="flex">
                    <CardAction icon="pencil" title="편집" onClick={() => setEditing(spec)} />
                    <CardAction icon="copy" title="복제" onClick={() => setEditing(duplicateOf(spec))} />
                  </span>
                  <span className="flex">
                    <CardAction icon="download" title="내보내기" onClick={() => run(exportTheme(spec))} />
                    <CardAction icon="x" title="삭제" onClick={() => run(removeTheme(spec))} />
                  </span>
                </span>
              }
            />
          ))}
        </div>
      )}

      {error && (
        <div role="alert" className="text-xs text-status-failed">
          {error}
        </div>
      )}
    </>
  );
}
