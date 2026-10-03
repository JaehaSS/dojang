// @vitest-environment jsdom

import { beforeEach, describe, expect, it } from "vitest";
import { contrastRatio } from "./color";
import {
  applyTheme,
  contrastNote,
  counterpartOf,
  DEFAULT_THEME_ID,
  getActiveTheme,
  getTheme,
  loadThemeId,
  subscribeTheme,
  THEMES,
  xtermTheme,
} from "./themes";

describe("기존 테마 보존", () => {
  // 이 두 블록은 다중 테마 도입 **전**의 index.css / TerminalView 값이다.
  // 하나라도 어긋나면 기존 사용자 화면의 색이 바뀐 것이다.
  it("Dojang Dark가 옛 .dark 토큰과 정확히 같다", () => {
    expect(getTheme("praxis-dark").tokens).toEqual({
      bg: "#0d0d0d",
      surface: "#161616",
      raised: "#1f1f1f",
      border: "#2a2a2a",
      borderStrong: "#3f3f46",
      primary: "#14b8a6",
      primaryBright: "#2dd4bf",
      primaryHover: "#0d9488",
      text: "#f4f4f5",
      text2: "#a1a1aa",
      textMuted: "#71717a",
      running: "#60a5fa",
      awaiting: "#fbbf24",
      question: "#c084fc",
      done: "#4ade80",
      failed: "#f87171",
      addbg: "#071a0f",
      delbg: "#1a0a0a",
      addbgStrong: "#14532d",
      delbgStrong: "#7f1d1d",
      dangerbg: "#1a0a0a",
      dangerborder: "#b91c1c",
      empty: "#161616",
      scrollbarThumb: "rgba(161, 161, 170, 0.3)",
      scrollbarThumbHover: "rgba(161, 161, 170, 0.5)",
      scrollbarThumbActive: "rgba(161, 161, 170, 0.65)",
      termCursor: "#2dd4bf",
      termSelection: "#14b8a633",
    });
  });

  it("Dojang Light가 옛 :root 토큰과 정확히 같다", () => {
    expect(getTheme("praxis-light").tokens).toEqual({
      bg: "#ffffff",
      surface: "#f7f7f8",
      raised: "#ffffff",
      border: "#e4e4e7",
      borderStrong: "#d4d4d8",
      primary: "#14b8a6",
      primaryBright: "#2dd4bf",
      primaryHover: "#0d9488",
      text: "#18181b",
      text2: "#52525b",
      textMuted: "#a1a1aa",
      running: "#2563eb",
      awaiting: "#d97706",
      question: "#9333ea",
      done: "#16a34a",
      failed: "#dc2626",
      addbg: "#dcfce7",
      delbg: "#fee2e2",
      addbgStrong: "#86efac",
      delbgStrong: "#fca5a5",
      dangerbg: "#fef2f2",
      dangerborder: "#fca5a5",
      empty: "#f1f1f2",
      scrollbarThumb: "rgba(24, 24, 27, 0.25)",
      scrollbarThumbHover: "rgba(24, 24, 27, 0.4)",
      scrollbarThumbActive: "rgba(24, 24, 27, 0.55)",
      termCursor: "#0d9488",
      termSelection: "#14b8a633",
    });
  });

  // ANSI 16키는 **추가**분이다 — 기존 4키 값은 옛 DARK/LIGHT 상수 그대로여야 한다(DR-2).
  it("xterm 팔레트가 옛 DARK/LIGHT 상수와 같다 + ANSI 16이 붙는다", () => {
    expect(xtermTheme(getTheme("praxis-dark"))).toEqual({
      background: "#0d0d0d",
      foreground: "#f4f4f5",
      cursor: "#2dd4bf",
      selectionBackground: "#14b8a633",
      black: "#1f1f1f",
      red: "#f87171",
      green: "#4ade80",
      yellow: "#fbbf24",
      blue: "#60a5fa",
      magenta: "#c084fc",
      cyan: "#14b8a6",
      white: "#a1a1aa",
      brightBlack: "#71717a",
      brightRed: "#faa2a2",
      brightGreen: "#75e69e",
      brightYellow: "#fcce56",
      brightBlue: "#91c1fc",
      brightMagenta: "#dab6fd",
      brightCyan: "#19e6cf",
      brightWhite: "#f4f4f5",
    });
    // 라이트의 bright 6색과 black은 이번 릴리스에서 값이 바뀌었다 — ANSI 16키 자체가 신규라
    // 픽셀 보존 계약의 대상이 아니다. 흰 배경에서 lighten은 대비를 **깎는다**(brightGreen 2.0:1,
    // black 1.35:1로 사실상 안 보였다). "bright = 배경에서 더 튄다"를 지키려면 라이트는 진해져야 한다.
    expect(xtermTheme(getTheme("praxis-light"))).toEqual({
      background: "#ffffff",
      foreground: "#18181b",
      cursor: "#0d9488",
      selectionBackground: "#14b8a633",
      black: "#18181b",
      red: "#dc2626",
      green: "#16a34a",
      yellow: "#d97706",
      blue: "#2563eb",
      magenta: "#9333ea",
      cyan: "#14b8a6",
      white: "#52525b",
      brightBlack: "#a1a1aa",
      brightRed: "#b21d1d",
      brightGreen: "#107636",
      brightYellow: "#a75c05",
      brightBlue: "#134cca",
      brightMagenta: "#7a16d4",
      brightCyan: "#0f8a7c",
      brightWhite: "#18181b",
    });
  });
});

describe("Chromatic Discipline 불변식", () => {
  // 팔레트를 새로 얹을 때 규율을 지켰는지 자동으로 걸러낸다.
  const external = THEMES.filter((t) => !t.id.startsWith("praxis-"));

  it.each(external)("$label — 본문·보조 텍스트가 AA를 넘는다", (theme) => {
    const t = theme.tokens;
    expect(contrastRatio(t.text, t.bg)).toBeGreaterThanOrEqual(7);
    expect(contrastRatio(t.text2, t.bg)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(t.textMuted, t.bg)).toBeGreaterThanOrEqual(3.5);
  });

  it.each(external)("$label — 상태색 5종이 AA를 넘는다", (theme) => {
    const t = theme.tokens;
    for (const c of [t.running, t.awaiting, t.question, t.done, t.failed]) {
      expect(contrastRatio(c, t.bg)).toBeGreaterThanOrEqual(4.5);
    }
  });

  it.each(THEMES)("$label — 표면 위계가 무너지지 않는다", (theme) => {
    const t = theme.tokens;
    expect(t.border).not.toBe(t.borderStrong);
    // "그림자 대신 보더로 계층"(DESIGN.md) — 표면끼리 색이 같아도 되지만,
    // 그 경우 보더가 유일한 경계이므로 표면과 반드시 달라야 한다.
    expect(t.border).not.toBe(t.surface);
    expect(t.border).not.toBe(t.raised);
    if (theme.kind === "dark") {
      // 다크는 elevation을 명도로도 표현한다(라이트는 bg=raised=흰색이 정상).
      expect(t.bg).not.toBe(t.raised);
    }
  });

  it.each(THEMES)("$label — 액센트가 어떤 상태색과도 겹치지 않는다", (theme) => {
    const t = theme.tokens;
    // 같은 색이면 "누를 수 있는 것"과 "상태"를 색으로 구분할 수 없다.
    for (const c of [t.running, t.awaiting, t.question, t.done, t.failed]) {
      expect(t.primary).not.toBe(c);
    }
  });

  it("빌트인 25종, 라이트 짝 페어가 유효하다", () => {
    expect(THEMES).toHaveLength(25);
    for (const t of THEMES) {
      if (t.counterpart) {
        expect(
          THEMES.some((o) => o.id === t.counterpart),
          t.id,
        ).toBe(true);
      }
    }
  });

  it("테마 id가 중복되지 않는다 — Monaco 테마 이름으로도 쓰인다", () => {
    expect(new Set(THEMES.map((t) => t.id)).size).toBe(THEMES.length);
  });
});

describe("저장·마이그레이션", () => {
  beforeEach(() => localStorage.clear());

  it("이진 토글 시절 값을 새 id로 읽는다", () => {
    localStorage.setItem("praxis-theme", "light");
    expect(loadThemeId()).toBe("praxis-light");
    localStorage.setItem("praxis-theme", "dark");
    expect(loadThemeId()).toBe("praxis-dark");
  });

  it("저장된 값이 없거나 모르는 id면 기본 테마다", () => {
    expect(loadThemeId()).toBe(DEFAULT_THEME_ID);
    localStorage.setItem("praxis-theme", "sunset-vaporwave");
    expect(loadThemeId()).toBe(DEFAULT_THEME_ID);
  });

  it("적용하면 저장되어 다음 부팅에 살아남는다", () => {
    applyTheme("nord");
    expect(loadThemeId()).toBe("nord");
  });
});

describe("적용", () => {
  beforeEach(() => {
    localStorage.clear();
    applyTheme(DEFAULT_THEME_ID);
  });

  it("CSS 변수와 .dark 클래스를 함께 갱신한다", () => {
    applyTheme("catppuccin-latte");
    const root = document.documentElement;
    expect(root.style.getPropertyValue("--c-bg")).toBe("#eff1f5");
    expect(root.classList.contains("dark")).toBe(false);
    expect(root.dataset.theme).toBe("catppuccin-latte");
    expect(root.style.getPropertyValue("color-scheme")).toBe("light");

    applyTheme("tokyo-night");
    expect(root.classList.contains("dark")).toBe(true);
    expect(root.style.getPropertyValue("--c-bg")).toBe("#1a1b26");
  });

  it("구독자에게 알리고 활성 테마를 바꾼다", () => {
    let hits = 0;
    const off = subscribeTheme(() => hits++);
    applyTheme("gruvbox-dark");
    expect(hits).toBe(1);
    expect(getActiveTheme().id).toBe("gruvbox-dark");
    off();
    applyTheme("nord");
    expect(hits).toBe(1);
  });

  it("같은 테마를 두 번 읽으면 같은 참조다 — useSyncExternalStore가 무한 렌더에 빠지지 않는다", () => {
    applyTheme("nord");
    expect(getActiveTheme()).toBe(getActiveTheme());
  });
});

describe("AA 배지", () => {
  it("보정이 없으면 표시할 것도 없다", () => {
    expect(contrastNote("#ffffff", "#ffffff", "#0d0d0d")).toBeNull();
    // 대소문자만 다른 것은 보정이 아니다 — hex 입력은 어느 쪽으로도 들어온다.
    expect(contrastNote("#FFFFFF", "#ffffff", "#0d0d0d")).toBeNull();
  });

  it("명암비는 보정값이 아니라 **입력값** 기준이다 — 왜 보정됐는지의 근거이므로", () => {
    const note = contrastNote("#333333", "#8a8a8a", "#0d0d0d");
    expect(note).toEqual({
      ratio: contrastRatio("#333333", "#0d0d0d").toFixed(1),
      corrected: "#8a8a8a",
    });
  });
});

describe("라이트↔다크 전환", () => {
  it("짝이 있으면 계열을 유지한다", () => {
    expect(counterpartOf(getTheme("catppuccin-mocha"))).toBe("catppuccin-latte");
    expect(counterpartOf(getTheme("catppuccin-latte"))).toBe("catppuccin-mocha");
  });

  it("짝이 없는 다크 테마는 기본 라이트로 간다", () => {
    expect(counterpartOf(getTheme("nord"))).toBe("praxis-light");
  });

  it("어느 테마에서 눌러도 명암이 뒤집힌다", () => {
    for (const theme of THEMES) {
      expect(getTheme(counterpartOf(theme)).kind).not.toBe(theme.kind);
    }
  });
});

describe("구문 강조 팔레트", () => {
  it("praxis 2종만 syntax가 없다 (내장 상속 보존 — 플랜 0049 DR-1)", () => {
    const without = THEMES.filter((t) => !t.syntax)
      .map((t) => t.id)
      .sort();
    expect(without).toEqual(["praxis-dark", "praxis-light"]);
  });

  it("syntax 10색이 전부 있고 AA floor를 지킨다 (comment 3.5, 나머지 4.5)", () => {
    for (const t of THEMES.filter((t) => t.syntax)) {
      const s = t.syntax!;
      expect(Object.keys(s).sort(), t.id).toEqual([
        "comment",
        "constant",
        "func",
        "keyword",
        "number",
        "operator",
        "string",
        "tag",
        "type",
        "variable",
      ]);
      for (const [k, v] of Object.entries(s)) {
        const floor = k === "comment" ? 3.5 : 4.5;
        expect(
          contrastRatio(v, t.tokens.bg),
          `${t.id}.${k}=${v}`,
        ).toBeGreaterThanOrEqual(floor);
      }
    }
  });
});

describe("터미널 ANSI", () => {
  it("ANSI 16이 상태색에서 파생된다", () => {
    const theme = getTheme("praxis-dark");
    const t = xtermTheme(theme);
    expect(t.red).toBe(theme.tokens.failed);
    expect(t.green).toBe(theme.tokens.done);
    expect(t.blue).toBe(theme.tokens.running);
    expect(Object.keys(t)).toHaveLength(4 + 16);
  });

  it.each(THEMES)("$label — bright 계열이 기본 계열과 다르다", (theme) => {
    const t = xtermTheme(theme);
    // 같은 값이면 굵은 텍스트(bright)와 보통 텍스트를 터미널에서 구분할 수 없다.
    expect(t.brightRed).not.toBe(t.red);
    expect(t.brightGreen).not.toBe(t.green);
    expect(t.brightBlue).not.toBe(t.blue);
  });
});
