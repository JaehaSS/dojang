// 커스텀 테마 편집 화면. 편집 중에는 앱 전체가 드래프트로 물들고(applyDraft), 저장하거나
// 취소할 때 정본으로 돌아간다 — 프리뷰 목업이 CSS 변수를 그대로 읽는 것도 그 덕이다.
import { useEffect, useMemo, useRef, useState } from "react";
import Editor from "@monaco-editor/react";
// 로컬 번들 구성(loader.config)과 테마 정의 부수효과. 미니 에디터도 같은 인스턴스를 쓴다.
import "../../lib/monaco";
import {
  applyDraft,
  applyTheme,
  contrastNote,
  deriveTheme,
  getActiveTheme,
  xtermTheme,
  DRAFT_THEME_ID,
  type ContrastNote,
  type DerivedKey,
  type PaletteKey,
  type SyntaxPalette,
  type Theme,
  type ThemeKind,
} from "../../lib/themes";
import { useTheme } from "../../lib/use-theme";
import {
  DERIVED_KEYS,
  PALETTE_KEYS,
  SYNTAX_KEYS,
  parseThemeSpec,
  saveCustomTheme,
  serializeThemeSpec,
  toThemeSpec,
  type CustomThemeSpec,
} from "../../lib/theme-files";

const HEX = /^#[0-9a-f]{6}$/i;
/** 약 5프레임 — 키 입력마다 전역 CSS 변수·defineTheme 비용을 내지 않으면서 지연이 안 느껴지는 값. */
const DRAFT_DEBOUNCE_MS = 80;

/** 구문색 10키가 모두 등장하는 고정 스니펫. monarch 토크나이저의 상한은 설계 0049 "알려진 한계". */
const SNIPPET = `// 주석 — comment
import { render } from "./view";

type Status = "done" | "failed";
const MAX_RETRY = 3;

export function tick(status: Status): number {
  const label = \`재시도 \${MAX_RETRY}회\`;
  if (status === "failed" && /re-?try/.test(label)) return MAX_RETRY * 2;
  return render(label) ?? 0;
}
`;

const ANSI_KEYS = [
  "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
  "brightBlack", "brightRed", "brightGreen", "brightYellow", "brightBlue",
  "brightMagenta", "brightCyan", "brightWhite",
] as const;

const STATUS_KEYS = ["running", "awaiting", "question", "done", "failed"] as const;

const TEXT_INPUT =
  "min-w-0 flex-1 rounded border border-border bg-bg px-1.5 py-1 font-code text-[11px] text-text outline-none focus:border-primary";

function hexOr(value: string, fallback: string): string {
  if (HEX.test(value)) return value;
  return HEX.test(fallback) ? fallback : "#000000";
}

/** 색 한 칸 — 스와치(색 선택기)와 hex 텍스트가 같은 값을 가리킨다. */
function ColorCell({
  label,
  value,
  auto,
  note,
  onChange,
}: {
  label: string;
  value: string;
  /** 비었을 때의 자동 파생값 — 고급 섹션에서만 쓴다. */
  auto?: string;
  note?: ContrastNote | null;
  onChange: (hex: string) => void;
}) {
  // 타이핑 중간 상태("#12")는 spec에 올리지 않는다 — 그 값으로 파생하면 화면이 깨진다.
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);

  const commit = (next: string) => {
    setText(next);
    if (!next || HEX.test(next)) onChange(next);
  };

  return (
    <label className="flex flex-col gap-1">
      <span className="font-code text-[11px] text-text-secondary">{label}</span>
      <span className="flex items-center gap-1.5">
        <input
          type="color"
          aria-label={`${label} 색 선택`}
          value={hexOr(value, auto ?? "")}
          onChange={(e) => commit(e.target.value)}
          className="h-6 w-7 shrink-0 cursor-pointer rounded border border-border bg-bg"
        />
        <input
          type="text"
          aria-label={label}
          spellCheck={false}
          value={text}
          placeholder={auto ?? ""}
          onChange={(e) => commit(e.target.value.trim())}
          className={TEXT_INPUT}
        />
      </span>
      {note && (
        <span className="text-[10px] text-status-failed">
          ⚠ {note.ratio} → {note.corrected} 보정됨
        </span>
      )}
    </label>
  );
}

function CellGrid({ children }: { children: React.ReactNode }) {
  return <div className="grid grid-cols-2 gap-x-3 gap-y-2">{children}</div>;
}

/** 드래프트가 실제로 어떻게 보이는지 — UI 목업·구문 강조·터미널 ANSI 16. */
function Preview({ theme, monacoThemeId }: { theme: Theme; monacoThemeId: string }) {
  const term = xtermTheme(theme);
  return (
    <div className="space-y-2">
      <div className="rounded-md border border-border bg-surface p-2">
        <div className="mb-1.5 flex items-center gap-1.5">
          <span className="h-2 w-2 rounded-full bg-primary" />
          <span className="text-[11px] text-text">Dojang</span>
          <span className="ml-auto text-[10px] text-text-muted">미리보기</span>
        </div>
        <div className="rounded border border-border bg-raised px-1.5 py-1 text-[10px] text-text-secondary">
          작업 카드
        </div>
        <div className="mt-1.5 flex items-center gap-1.5">
          {STATUS_KEYS.map((k) => (
            <span
              key={k}
              title={k}
              className="h-2 w-2 rounded-full"
              style={{ background: theme.tokens[k] }}
            />
          ))}
          <span className="ml-auto text-[10px] text-text-muted">상태 5색</span>
        </div>
      </div>

      <div className="h-40 overflow-hidden rounded-md border border-border">
        <Editor
          height="100%"
          language="typescript"
          value={SNIPPET}
          theme={monacoThemeId}
          options={{
            readOnly: true,
            minimap: { enabled: false },
            lineNumbers: "off",
            scrollBeyondLastLine: false,
            automaticLayout: true,
            fontSize: 11,
            folding: false,
            renderLineHighlight: "none",
            scrollbar: { vertical: "hidden", horizontal: "hidden" },
          }}
        />
      </div>

      <div className="grid grid-cols-8 gap-1" aria-label="터미널 ANSI 16색">
        {ANSI_KEYS.map((k) => (
          <span
            key={k}
            title={k}
            className="h-3 rounded-sm border border-border"
            style={{ background: term[k] }}
          />
        ))}
      </div>
    </div>
  );
}

export function ThemeEditor({
  initial,
  onDone,
}: {
  initial: CustomThemeSpec;
  onDone: () => void;
}) {
  const entryId = useRef(getActiveTheme().id).current;
  const [spec, setSpec] = useState<CustomThemeSpec>(initial);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const activeId = useTheme().id;

  // 저장이 진입하며 이 타이머를 걷어낸다 — 마지막 타이핑의 드래프트가 저장본 뒤에 깨어나면
  // 화면이 임시 색으로 되돌아간 채 남는다.
  const draftTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    draftTimer.current = setTimeout(() => applyDraft(toThemeSpec(spec)), DRAFT_DEBOUNCE_MS);
    const timer = draftTimer.current;
    return () => clearTimeout(timer);
  }, [spec]);

  // 화면을 떠나는 모든 경로에서 드래프트를 걷어낸다. 저장 뒤에는 활성이 이미 저장본이라 걸리지 않는다.
  useEffect(
    () => () => {
      if (getActiveTheme().id === DRAFT_THEME_ID) applyTheme(entryId);
    },
    [entryId],
  );

  // AA 배지와 프리뷰가 같은 파생 결과를 봐야 한다 — 한 번 계산해 둘 다 여기서 읽는다.
  const derived = useMemo(() => deriveTheme(toThemeSpec(spec)), [spec]);

  // 원색·구문은 필수 키다 — 빈 값은 무시하고 마지막 유효값을 지킨다(고급 override만 비울 수 있다).
  const setPalette = (k: PaletteKey, hex: string) => {
    if (!hex) return;
    setSpec((s) => ({ ...s, palette: { ...s.palette, [k]: hex } }));
  };
  const setSyntax = (k: keyof SyntaxPalette, hex: string) => {
    if (!hex) return;
    setSpec((s) => ({ ...s, syntax: { ...s.syntax, [k]: hex } }));
  };
  const setOverride = (k: DerivedKey, hex: string) =>
    setSpec((s) => {
      const overrides = { ...s.overrides };
      if (hex) overrides[k] = hex;
      else delete overrides[k];
      return { ...s, overrides };
    });

  async function save() {
    // 자기 검증 — 파일에서 읽을 때와 같은 문을 통과해야 저장한다.
    const parsed = parseThemeSpec(serializeThemeSpec(spec));
    if (!parsed) {
      setError("색 값이 #rrggbb 형식인지 확인하세요.");
      return;
    }
    if (draftTimer.current) clearTimeout(draftTimer.current);
    setBusy(true);
    try {
      await saveCustomTheme(parsed);
      applyTheme(parsed.id);
      onDone();
    } catch (e) {
      setError(`저장하지 못했습니다 — ${String(e)}`);
      setBusy(false);
    }
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <input
          aria-label="테마 이름"
          value={spec.label}
          onChange={(e) => setSpec((s) => ({ ...s, label: e.target.value }))}
          className="min-w-0 flex-1 rounded border border-border bg-bg px-2 py-1 text-sm text-text outline-none focus:border-primary"
        />
        <select
          aria-label="테마 계열"
          value={spec.kind}
          onChange={(e) => setSpec((s) => ({ ...s, kind: e.target.value as ThemeKind }))}
          className="rounded border border-border bg-bg px-2 py-1 text-sm text-text outline-none focus:border-primary"
        >
          <option value="dark">다크</option>
          <option value="light">라이트</option>
        </select>
        <button
          type="button"
          disabled={busy}
          onClick={onDone}
          className="h-7 shrink-0 rounded-md border border-border px-2.5 text-sm text-text-secondary hover:text-text disabled:opacity-40"
        >
          취소
        </button>
        <button
          type="button"
          disabled={busy || !spec.label.trim()}
          onClick={() => void save()}
          className="h-7 shrink-0 rounded-md bg-primary px-2.5 text-sm text-bg disabled:opacity-40"
        >
          저장
        </button>
      </div>
      <div className="text-[11px] text-text-muted">
        <span className="font-code">{spec.id}</span> · 편집 중인 색은 앱 전체에 바로 적용된다.
        취소하면 되돌아간다.
      </div>
      {error && (
        <div role="alert" className="text-xs text-status-failed">
          {error}
        </div>
      )}

      <div className="grid grid-cols-[minmax(0,1fr)_220px] gap-4">
        <div className="space-y-4">
          <section className="space-y-2">
            <div className="text-xs text-text-secondary">원색</div>
            <CellGrid>
              {PALETTE_KEYS.map((k) => (
                <ColorCell
                  key={k}
                  label={k}
                  value={spec.palette[k]}
                  note={contrastNote(spec.palette[k], derived.tokens[k], spec.palette.bg)}
                  onChange={(hex) => setPalette(k, hex)}
                />
              ))}
            </CellGrid>
          </section>

          <section className="space-y-2">
            <div className="text-xs text-text-secondary">구문 강조</div>
            <CellGrid>
              {SYNTAX_KEYS.map((k) => (
                <ColorCell
                  key={k}
                  label={k}
                  value={spec.syntax[k]}
                  note={contrastNote(
                    spec.syntax[k],
                    derived.syntax?.[k] ?? spec.syntax[k],
                    spec.palette.bg,
                  )}
                  onChange={(hex) => setSyntax(k, hex)}
                />
              ))}
            </CellGrid>
          </section>

          <details className="rounded-md border border-border p-2">
            <summary className="cursor-pointer text-xs text-text-secondary">
              고급 — 파생 토큰 덮어쓰기 (빈 값 = 자동)
            </summary>
            <div className="pt-2">
              <CellGrid>
                {DERIVED_KEYS.map((k) => (
                  <ColorCell
                    key={k}
                    label={k}
                    value={spec.overrides?.[k] ?? ""}
                    auto={derived.tokens[k]}
                    onChange={(hex) => setOverride(k, hex)}
                  />
                ))}
              </CellGrid>
            </div>
          </details>
        </div>

        <Preview theme={derived} monacoThemeId={activeId} />
      </div>
    </div>
  );
}
