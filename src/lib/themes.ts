// 테마 정의의 단일 정본. CSS 변수(--c-*)·Monaco·xterm이 모두 여기서 값을 받는다.
//
// DESIGN.md "Chromatic Discipline"은 색이 아니라 **구조**를 규정한다 — 3단 표면 위계,
// 보더 2단, 텍스트 3단, 단일 액센트, 상태 5색. 테마는 그 구조를 자기 팔레트로 채울 뿐이며
// 자유 스타일이 아니다. 그래서 각 테마는 원색 15개만 선언하고 파생 토큰은 계산한다.
//
// 팔레트 출처는 각 테마의 공식 값이다. 표면 3단이 팔레트에 다 없는 경우
// (Catppuccin·Tokyo Night 등) 인접 단계를 섞어 만들며, 그 근거를 값 옆에 남긴다.

import { alpha, contrastRatio, darken, ensureContrast, hexAlpha, lighten, mix } from "./color";

export type ThemeKind = "dark" | "light";

/** 테마가 직접 정하는 원색. 나머지는 전부 여기서 파생된다. */
interface Palette {
  /** 앱·에디터 배경 (elevation 0) */
  bg: string;
  /** 카드·패널·사이드바 (elevation 1) */
  surface: string;
  /** 팝오버·모달 (elevation 2+) */
  raised: string;
  border: string;
  borderStrong: string;
  /** CTA·포커스·활성 — 상태색 어느 것과도 혼동되지 않아야 한다. */
  primary: string;
  text: string;
  text2: string;
  textMuted: string;
  running: string;
  awaiting: string;
  question: string;
  done: string;
  failed: string;
}

/** 커스텀 spec 검증(theme-files.ts)이 필수 키 목록으로 쓴다. */
export type PaletteKey = keyof Palette;

/** 테마가 직접 정하는 구문 강조 원색. Monaco rules·(간접적으로) 편집 UI가 소비한다. */
export interface SyntaxPalette {
  keyword: string;
  string: string;
  number: string;
  /** AA floor 3.5 — 주석은 의도적으로 죽인다 */
  comment: string;
  type: string;
  func: string;
  variable: string;
  constant: string;
  operator: string;
  /** 마크업 (tag/attribute) */
  tag: string;
}

export interface ThemeSpec {
  id: string;
  label: string;
  /** 설정 화면에서 테마를 고르는 근거가 되는 한 줄. */
  blurb: string;
  kind: ThemeKind;
  /** 라이트/다크 토글(⌘ 커맨드)이 오갈 짝. 없으면 같은 kind의 기본 테마로 간다. */
  counterpart?: string;
  palette: Palette;
  /** 없으면 Monaco가 내장 vs/vs-dark 구문색을 상속한다 (Praxis 2종 — DR-1). */
  syntax?: SyntaxPalette;
  /**
   * 파생 공식이 재현하지 못하는 값. Praxis 기본 테마 둘은 기존 렌더를 픽셀 단위로
   * 보존해야 하므로 손으로 고른 diff 틴트와 액센트 변형을 그대로 남긴다.
   */
  overrides?: Partial<DerivedTokens>;
  /**
   * 팔레트 원색이 AA에 못 미칠 때 명도를 자동 보정할지. 기본 true.
   * Praxis 기본 테마는 DESIGN.md에서 이미 검증된 값이라 보정하지 않는다 — 보정하면
   * 기존 화면의 색이 바뀐다.
   */
  autoContrast?: boolean;
}

/** 파생 토큰 — 팔레트에서 계산되거나 override로 고정된다. */
interface DerivedTokens {
  primaryBright: string;
  primaryHover: string;
  addbg: string;
  delbg: string;
  addbgStrong: string;
  delbgStrong: string;
  dangerbg: string;
  dangerborder: string;
  empty: string;
  scrollbarThumb: string;
  scrollbarThumbHover: string;
  scrollbarThumbActive: string;
  termCursor: string;
  termSelection: string;
}

/** 커스텀 spec의 `overrides`가 고를 수 있는 키. */
export type DerivedKey = keyof DerivedTokens;

export type ThemeTokens = Palette & DerivedTokens;

export interface Theme {
  id: string;
  label: string;
  blurb: string;
  kind: ThemeKind;
  counterpart?: string;
  tokens: ThemeTokens;
  syntax?: SyntaxPalette;
}

// ─── 팔레트 ────────────────────────────────────────────────────────────────

const SPECS: ThemeSpec[] = [
  {
    id: "praxis-dark",
    label: "Dojang Dark",
    blurb: "중성 모노크롬 위 teal 단일 신호. 기본값.",
    kind: "dark",
    counterpart: "praxis-light",
    autoContrast: false,
    palette: {
      bg: "#0d0d0d",
      surface: "#161616",
      raised: "#1f1f1f",
      border: "#2a2a2a",
      borderStrong: "#3f3f46",
      primary: "#14b8a6",
      text: "#f4f4f5",
      text2: "#a1a1aa",
      textMuted: "#71717a",
      running: "#60a5fa",
      awaiting: "#fbbf24",
      question: "#c084fc",
      done: "#4ade80",
      failed: "#f87171",
    },
    overrides: {
      primaryBright: "#2dd4bf",
      primaryHover: "#0d9488",
      addbg: "#071a0f",
      delbg: "#1a0a0a",
      addbgStrong: "#14532d",
      delbgStrong: "#7f1d1d",
      dangerbg: "#1a0a0a",
      dangerborder: "#b91c1c",
      empty: "#161616",
    },
  },
  {
    id: "praxis-light",
    label: "Dojang Light",
    blurb: "같은 규율의 밝은 대응. 상태색만 600 톤으로 내린다.",
    kind: "light",
    counterpart: "praxis-dark",
    autoContrast: false,
    palette: {
      bg: "#ffffff",
      surface: "#f7f7f8",
      raised: "#ffffff",
      border: "#e4e4e7",
      borderStrong: "#d4d4d8",
      primary: "#14b8a6",
      text: "#18181b",
      text2: "#52525b",
      textMuted: "#a1a1aa",
      running: "#2563eb",
      awaiting: "#d97706",
      question: "#9333ea",
      done: "#16a34a",
      failed: "#dc2626",
    },
    overrides: {
      primaryBright: "#2dd4bf",
      primaryHover: "#0d9488",
      addbg: "#dcfce7",
      delbg: "#fee2e2",
      addbgStrong: "#86efac",
      delbgStrong: "#fca5a5",
      dangerbg: "#fef2f2",
      dangerborder: "#fca5a5",
      empty: "#f1f1f2",
    },
  },
  {
    // catppuccin.com/palette — Mocha. 커뮤니티 선호 1위(2026), 파스텔 저자극.
    id: "catppuccin-mocha",
    label: "Catppuccin Mocha",
    blurb: "파스텔 저자극. 장시간 응시에 가장 편한 축.",
    kind: "dark",
    counterpart: "catppuccin-latte",
    palette: {
      bg: "#1e1e2e", // base
      surface: "#292a3b", // base↔surface0 0.6 — 팔레트에 중간 단계가 없다
      raised: "#313244", // surface0
      border: "#38394b", // surface0↔surface1 0.35
      borderStrong: "#45475a", // surface1
      primary: "#94e2d5", // teal — 시그니처 mauve는 question과 부딪혀 액센트로 못 쓴다
      text: "#cdd6f4",
      text2: "#a6adc8", // subtext0
      textMuted: "#7f849c", // overlay1
      running: "#89b4fa", // blue
      awaiting: "#f9e2af", // yellow
      question: "#cba6f7", // mauve
      done: "#a6e3a1", // green
      failed: "#f38ba8", // red
    },
    // catppuccin.com/palette — style guide 색 배치 그대로.
    syntax: {
      keyword: "#cba6f7",
      string: "#a6e3a1",
      number: "#fab387",
      comment: "#6c7086",
      type: "#f9e2af",
      func: "#89b4fa",
      variable: "#cdd6f4",
      constant: "#fab387",
      operator: "#89dceb",
      tag: "#89b4fa",
    },
  },
  {
    // catppuccin.com/palette — Latte.
    id: "catppuccin-latte",
    label: "Catppuccin Latte",
    blurb: "Mocha의 밝은 짝. 흰 배경보다 눈부심이 덜하다.",
    kind: "light",
    counterpart: "catppuccin-mocha",
    palette: {
      bg: "#eff1f5", // base
      surface: "#e6e9ef", // mantle
      raised: "#ffffff", // 라이트에서 팝오버는 배경보다 밝아야 뜬다
      border: "#ccd0da", // surface0
      borderStrong: "#bcc0cc", // surface1
      primary: "#179299", // teal
      text: "#4c4f69",
      text2: "#5c5f77", // subtext1
      textMuted: "#8c8fa1", // overlay1
      running: "#1e66f5",
      awaiting: "#df8e1d",
      question: "#8839ef",
      done: "#40a02b",
      failed: "#d20f39",
    },
    // catppuccin.com/palette — style guide 색 배치 그대로.
    syntax: {
      keyword: "#8839ef",
      string: "#40a02b",
      number: "#fe640b",
      comment: "#9ca0b0",
      type: "#df8e1d",
      func: "#1e66f5",
      variable: "#4c4f69",
      constant: "#fe640b",
      operator: "#04a5e5",
      tag: "#1e66f5",
    },
  },
  {
    // github.com/enkia/tokyo-night-vscode-theme — Night 변종.
    id: "tokyo-night",
    label: "Tokyo Night",
    blurb: "차가운 청보라 네온 느와르. 대비가 또렷하다.",
    kind: "dark",
    palette: {
      bg: "#1a1b26",
      surface: "#24283b", // Storm 배경 — Night의 한 단계 위 표면으로 쓴다
      raised: "#292e42", // bg_highlight
      border: "#323851", // bg_highlight↔#3b4261 0.5
      borderStrong: "#3b4261",
      primary: "#73daca", // teal — cyan은 running(blue)과 너무 가깝다
      text: "#c0caf5",
      text2: "#a9b1d6",
      textMuted: "#565f89",
      running: "#7aa2f7",
      awaiting: "#e0af68",
      question: "#bb9af7",
      done: "#9ece6a",
      failed: "#f7768e",
    },
    // github.com/enkia/tokyo-night-vscode-theme — 배포판 tokenColors.
    syntax: {
      keyword: "#bb9af7",
      string: "#9ece6a",
      number: "#ff9e64",
      comment: "#565f89",
      type: "#2ac3de",
      func: "#7aa2f7",
      variable: "#c0caf5",
      constant: "#ff9e64",
      operator: "#89ddff",
      tag: "#f7768e",
    },
  },
  {
    // nordtheme.com/docs/colors-and-palettes — Polar Night / Snow Storm / Frost / Aurora.
    id: "nord",
    label: "Nord",
    blurb: "미니멀·플랫한 북극 팔레트. 절제 철학이 Dojang과 가장 가깝다.",
    kind: "dark",
    palette: {
      bg: "#2e3440", // nord0
      surface: "#3b4252", // nord1
      raised: "#434c5e", // nord2
      border: "#464f62", // nord2↔nord3 0.35
      borderStrong: "#4c566a", // nord3
      primary: "#8fbcbb", // nord7 — nord8은 running(nord9)과 구분이 약하다
      text: "#eceff4", // nord6
      text2: "#d8dee9", // nord4
      textMuted: "#4c566a", // nord3 — 원본은 저대비라 자동 보정이 끌어올린다
      running: "#81a1c1", // nord9
      awaiting: "#ebcb8b", // nord13
      question: "#b48ead", // nord15
      done: "#a3be8c", // nord14
      failed: "#bf616a", // nord11
    },
    // nordtheme.com/docs/colors-and-palettes — Frost가 구문 전면, Aurora가 리터럴.
    syntax: {
      keyword: "#81a1c1", // nord9
      string: "#a3be8c", // nord14
      number: "#b48ead", // nord15
      comment: "#4c566a", // nord3 — 저대비분은 deriveSyntax가 끌어올린다
      type: "#8fbcbb", // nord7
      func: "#88c0d0", // nord8
      variable: "#d8dee9", // nord4
      constant: "#b48ead", // nord15
      operator: "#81a1c1", // nord9
      tag: "#81a1c1", // nord9
    },
  },
  {
    // github.com/morhetz/gruvbox — dark medium.
    id: "gruvbox-dark",
    label: "Gruvbox Dark",
    blurb: "따뜻한 레트로 저대비. 유일하게 난색 계열 배경이다.",
    kind: "dark",
    palette: {
      bg: "#282828", // bg0
      surface: "#32302f", // bg0_s
      raised: "#3c3836", // bg1
      border: "#453f3d", // bg1↔bg2 0.45
      borderStrong: "#504945", // bg2
      primary: "#fe8019", // orange — Gruvbox의 진짜 시그니처
      text: "#ebdbb2",
      text2: "#d5c4a1", // fg2
      textMuted: "#a89984", // fg4
      running: "#83a598", // blue
      awaiting: "#fabd2f", // yellow
      question: "#d3869b", // purple
      done: "#b8bb26", // green
      failed: "#fb4934", // red
    },
    // github.com/morhetz/gruvbox — 배포판 하이라이트 그룹(bright 계열).
    syntax: {
      keyword: "#fb4934", // bright_red
      string: "#b8bb26", // bright_green
      number: "#d3869b", // bright_purple
      comment: "#928374", // gray — 저대비분은 deriveSyntax가 끌어올린다
      type: "#fabd2f", // bright_yellow
      func: "#8ec07c", // bright_aqua
      variable: "#83a598", // bright_blue
      constant: "#d3869b", // bright_purple
      operator: "#fe8019", // bright_orange
      tag: "#8ec07c", // bright_aqua
    },
  },
  {
    // One Dark Pro (12.2M 설치) — Atom One Dark 계보.
    id: "one-dark-pro",
    label: "One Dark Pro",
    blurb: "가장 익숙한 클래식. 중간 대비에 채도가 낮다.",
    kind: "dark",
    palette: {
      bg: "#282c34",
      surface: "#2c313a",
      raised: "#333842",
      border: "#3e4451",
      borderStrong: "#4b5263",
      primary: "#56b6c2", // cyan — blue는 running이 쓴다
      text: "#abb2bf",
      text2: "#9da5b4",
      textMuted: "#5c6370",
      running: "#61afef",
      awaiting: "#e5c07b",
      question: "#c678dd",
      done: "#98c379",
      failed: "#e06c75",
    },
    // One Dark Pro 마켓 팔레트 — Atom One Dark 계보의 토큰 배치.
    syntax: {
      keyword: "#c678dd",
      string: "#98c379",
      number: "#d19a66",
      comment: "#5c6370",
      type: "#e5c07b",
      func: "#61afef",
      variable: "#e06c75",
      constant: "#d19a66",
      operator: "#56b6c2",
      tag: "#e06c75",
    },
  },
  {
    // rosepinetheme.com/palette (MIT). green이 없는 팔레트 — done은 foam, running은 pine을 쓰고
    // 저대비분은 autoContrast가 끌어올린다.
    id: "rose-pine",
    label: "Rosé Pine",
    blurb: "장미·소나무·이끼의 저채도 우아함. 난색 다크의 두 번째 축.",
    kind: "dark",
    counterpart: "rose-pine-dawn",
    palette: {
      bg: "#191724",
      surface: "#1f1d2e",
      raised: "#26233a",
      border: "#2e2b41", // raised↔highlightMed 0.4 — 팔레트에 중간 보더가 없다
      borderStrong: "#403d52", // highlightMed
      primary: "#ebbcba", // rose — iris는 question과 부딪힌다
      text: "#e0def4",
      text2: "#908caa",
      textMuted: "#6e6a86",
      running: "#31748f", // pine
      awaiting: "#f6c177", // gold
      question: "#c4a7e7", // iris
      done: "#9ccfd8", // foam — green 부재의 대체
      failed: "#eb6f92", // love
    },
    syntax: {
      keyword: "#31748f", // pine
      string: "#f6c177", // gold
      number: "#ebbcba", // rose
      comment: "#6e6a86", // muted
      type: "#9ccfd8", // foam
      func: "#ebbcba", // rose
      variable: "#e0def4", // text
      constant: "#c4a7e7", // iris
      operator: "#908caa", // subtle
      tag: "#9ccfd8", // foam
    },
  },
  {
    // rosepinetheme.com/palette (MIT) — Dawn 변종. 역할 배치는 Rosé Pine과 동일하다.
    id: "rose-pine-dawn",
    label: "Rosé Pine Dawn",
    blurb: "같은 장미빛의 낮. 종이 같은 난색 라이트.",
    kind: "light",
    counterpart: "rose-pine",
    palette: {
      bg: "#faf4ed", // base
      surface: "#fffaf3", // surface — 라이트는 카드가 배경보다 밝다
      raised: "#ffffff", // 팝오버는 가장 밝게 — Dawn에는 3단째 표면이 없다
      border: "#dfdad9", // highlightMed
      borderStrong: "#cecacd", // highlightHigh
      primary: "#d7827e", // rose
      text: "#575279",
      text2: "#797593", // subtle
      textMuted: "#9893a5", // muted — 원본은 저대비라 자동 보정이 끌어올린다
      running: "#286983", // pine
      awaiting: "#ea9d34", // gold
      question: "#907aa9", // iris
      done: "#56949f", // foam
      failed: "#b4637a", // love
    },
    syntax: {
      keyword: "#286983", // pine
      string: "#ea9d34", // gold
      number: "#d7827e", // rose
      comment: "#9893a5", // muted — 저대비분은 deriveSyntax가 끌어올린다
      type: "#56949f", // foam
      func: "#d7827e", // rose
      variable: "#575279", // text
      constant: "#907aa9", // iris
      operator: "#797593", // subtle
      tag: "#56949f", // foam
    },
  },
  {
    // draculatheme.com/contribute (MIT). 공식 원색 9개 + Background/Current Line만 있어
    // 표면 중간 단계는 섞어 만든다.
    id: "dracula",
    label: "Dracula",
    blurb: "고채도 네온 6색. 토큰 구분이 가장 선명하다.",
    kind: "dark",
    palette: {
      bg: "#282a36", // Background
      surface: "#303341", // Background↔Current Line 0.3
      raised: "#44475a", // Current Line
      border: "#4d5470", // Current Line↔Comment 0.3
      borderStrong: "#6272a4", // Comment
      primary: "#ff79c6", // pink — purple은 question이 쓴다
      text: "#f8f8f2", // Foreground
      text2: "#bcc2d3", // Foreground↔Comment 0.4 — 보조 텍스트 단계가 없다
      textMuted: "#6272a4", // Comment
      running: "#8be9fd", // cyan — 팔레트에 blue가 없다
      awaiting: "#f1fa8c", // yellow
      question: "#bd93f9", // purple
      done: "#50fa7b", // green
      failed: "#ff5555", // red
    },
    // draculatheme.com/contribute — 공식 스펙의 토큰 배치 그대로.
    syntax: {
      keyword: "#ff79c6", // pink
      string: "#f1fa8c", // yellow
      number: "#bd93f9", // purple
      comment: "#6272a4",
      type: "#8be9fd", // cyan
      func: "#50fa7b", // green
      variable: "#f8f8f2", // foreground
      constant: "#bd93f9", // purple
      operator: "#ff79c6", // pink
      tag: "#ff79c6", // pink
    },
  },
  {
    // github.com/primer/primitives + github/github-vscode-theme (MIT) — dark default.
    id: "github-dark",
    label: "GitHub Dark",
    blurb: "가장 많이 설치된 기본값. PR 화면과 같은 색으로 코드를 읽는다.",
    kind: "dark",
    counterpart: "github-light",
    palette: {
      bg: "#0d1117", // canvas.default
      surface: "#161b22", // canvas.subtle
      raised: "#21262d", // canvas.inset 위 팝오버 단계
      border: "#30363d", // border.default
      borderStrong: "#484f58", // neutral.emphasis
      primary: "#db61a2", // sponsors.fg — accent(파랑)는 running과 부딪혀 액센트로 못 쓴다
      text: "#e6edf3", // fg.default
      text2: "#b1bac4",
      textMuted: "#7d8590", // fg.muted
      running: "#58a6ff", // accent.fg
      awaiting: "#d29922", // attention.fg
      question: "#a371f7", // done.fg — GitHub이 머지 상태에 쓰는 보라
      done: "#3fb950", // success.fg
      failed: "#f85149", // danger.fg
    },
    // github/github-vscode-theme — dark default tokenColors.
    syntax: {
      keyword: "#ff7b72", // red[3] — GitHub은 키워드가 빨강이다
      string: "#a5d6ff", // blue[1]
      number: "#79c0ff", // blue[2] (constant)
      comment: "#8b949e", // gray[3]
      type: "#79c0ff", // blue[2] (support)
      func: "#d2a8ff", // purple[2]
      variable: "#ffa657", // orange[2]
      constant: "#79c0ff", // blue[2]
      operator: "#ff7b72", // red[3] (keyword.operator)
      tag: "#7ee787", // green[1]
    },
  },
  {
    // github.com/primer/primitives + github/github-vscode-theme (MIT) — light default.
    id: "github-light",
    label: "GitHub Light",
    blurb: "Dark의 짝. 문서·리뷰 화면에 가장 익숙한 밝은 톤.",
    kind: "light",
    counterpart: "github-dark",
    palette: {
      bg: "#ffffff", // canvas.default
      surface: "#f6f8fa", // canvas.subtle
      raised: "#ffffff", // 라이트 팝오버는 배경과 같은 흰색 + 보더로 띄운다
      border: "#d0d7de", // border.default
      borderStrong: "#afb8c1", // neutral.emphasis
      primary: "#bf3989", // sponsors.fg — dark와 같은 이유로 accent 대신 sponsors
      text: "#1f2328", // fg.default
      text2: "#656d76", // fg.muted
      textMuted: "#8c959f", // fg.subtle — 저대비분은 자동 보정이 끌어올린다
      running: "#0969da", // accent.fg
      awaiting: "#9a6700", // attention.fg
      question: "#8250df", // done.fg
      done: "#1a7f37", // success.fg
      failed: "#cf222e", // danger.fg
    },
    // github/github-vscode-theme — light default tokenColors.
    syntax: {
      keyword: "#cf222e", // red[5]
      string: "#0a3069", // blue[8]
      number: "#0550ae", // blue[6]
      comment: "#6e7781", // gray[5]
      type: "#0550ae", // blue[6] (support)
      func: "#8250df", // purple[5]
      variable: "#953800", // orange[6]
      constant: "#0550ae", // blue[6]
      operator: "#cf222e", // red[5]
      tag: "#116329", // green[6]
    },
  },
  {
    // github.com/primer/primitives + github/github-vscode-theme (MIT) — dark dimmed.
    id: "github-dimmed",
    label: "GitHub Dimmed",
    blurb: "Dark의 대비를 낮춘 변종. 검정 대신 청회색 위라 오래 봐도 덜 시리다.",
    kind: "dark",
    counterpart: "github-light",
    palette: {
      bg: "#22272e", // canvas.default
      surface: "#2d333b", // canvas.subtle — 카드·패널·사이드바
      raised: "#373e47", // canvas.overlay 위 팝오버 단계
      border: "#444c56", // border.default
      borderStrong: "#545d68", // neutral.emphasis
      primary: "#e275ad", // sponsors.fg — github-dark와 같은 이유로 accent(파랑) 대신 sponsors
      text: "#cdd9e5", // gray[0] — Praxis의 text는 본문 위 최상위 강조 단계다
      text2: "#adbac7", // fg.default
      textMuted: "#768390", // fg.muted
      running: "#6cb6ff", // accent.fg
      awaiting: "#daaa3f", // attention.fg
      question: "#b083f0", // done.fg — GitHub이 머지 상태에 쓰는 보라
      done: "#6bc46d", // success.fg
      failed: "#ff938a", // danger.fg
    },
    // github/github-vscode-theme — dark dimmed tokenColors.
    syntax: {
      keyword: "#f47067", // red[3]
      string: "#96d0ff", // blue[1]
      number: "#6cb6ff", // blue[2] (constant)
      comment: "#768390", // gray[3]
      type: "#6cb6ff", // blue[2] (support)
      func: "#dcbdfb", // purple[2]
      variable: "#f69d50", // orange[2]
      constant: "#6cb6ff", // blue[2]
      operator: "#f47067", // red[3] (keyword.operator)
      tag: "#8ddb8c", // green[1]
    },
  },
  {
    // github.com/sainnhe/everforest (MIT) — dark medium.
    id: "everforest-dark",
    label: "Everforest Dark",
    blurb: "숲 바닥의 녹회색. 난색과 한색 사이에서 눈을 쉬게 한다.",
    kind: "dark",
    palette: {
      bg: "#2d353b", // bg0
      surface: "#343f44", // bg1
      raised: "#3d484d", // bg2
      border: "#475258", // bg3
      borderStrong: "#4f585e", // bg4
      primary: "#83c092", // aqua — 시그니처 green은 done이 쓴다
      text: "#d3c6aa", // fg
      text2: "#9da9a0", // grey2
      textMuted: "#859289", // grey1
      running: "#7fbbb3", // blue
      awaiting: "#dbbc7f", // yellow
      question: "#d699b6", // purple
      done: "#a7c080", // green
      failed: "#e67e80", // red
    },
    // github.com/sainnhe/everforest — 배포판 하이라이트 그룹 매핑.
    syntax: {
      keyword: "#e67e80", // red (Statement)
      string: "#a7c080", // green
      number: "#d699b6", // purple
      comment: "#859289", // grey1
      type: "#dbbc7f", // yellow (Type)
      func: "#a7c080", // green (Function)
      variable: "#d3c6aa", // fg (Identifier)
      constant: "#d699b6", // purple
      operator: "#e69875", // orange
      tag: "#83c092", // aqua
    },
  },
  {
    // github.com/rebelot/kanagawa.nvim (MIT) — wave 변종.
    id: "kanagawa-wave",
    label: "Kanagawa Wave",
    blurb: "호쿠사이의 파도. 먹빛 배경 위 가라앉은 안료색.",
    kind: "dark",
    palette: {
      bg: "#1f1f28", // sumiInk1
      surface: "#2a2a37", // sumiInk2
      raised: "#363646", // sumiInk3
      border: "#444458", // sumiInk3↔sumiInk4 0.45 — 중간 보더가 없다
      borderStrong: "#54546d", // sumiInk4
      primary: "#7aa89f", // waveAqua2 — crystalBlue는 running과 부딪힌다
      text: "#dcd7ba", // fujiWhite
      text2: "#c8c093", // oldWhite
      textMuted: "#727169", // fujiGray
      running: "#7fb4ca", // springBlue
      awaiting: "#dca561", // autumnYellow
      question: "#957fb8", // oniViolet
      done: "#98bb6c", // springGreen
      failed: "#e82424", // samuraiRed — 원본은 저대비라 자동 보정이 끌어올린다
    },
    // github.com/rebelot/kanagawa.nvim — 배포판 하이라이트 그룹 매핑.
    syntax: {
      keyword: "#957fb8", // oniViolet
      string: "#98bb6c", // springGreen
      number: "#d27e99", // sakuraPink
      comment: "#727169", // fujiGray
      type: "#7aa89f", // waveAqua2
      func: "#7e9cd8", // crystalBlue
      variable: "#dcd7ba", // fujiWhite
      constant: "#ffa066", // surimiOrange
      operator: "#c0a36e", // boatYellow2
      tag: "#e6c384", // carpYellow
    },
  },
  {
    // github.com/sdras/night-owl-vscode-theme (MIT).
    id: "night-owl",
    label: "Night Owl",
    blurb: "심야용 감청. 어두운 방에서 가장 덜 눈부시다.",
    kind: "dark",
    palette: {
      bg: "#011627",
      surface: "#0b2336", // bg↔selection 0.35 — 표면 중간 단계가 없다
      raised: "#1d3b53", // selection/highlight
      border: "#314f67", // selection↔panel border 0.3
      borderStrong: "#5f7e97", // panel border 원색
      primary: "#7fdbca", // teal — Night Owl의 시그니처
      text: "#d6deeb",
      text2: "#aebac2", // fg↔comment 0.35 — 보조 텍스트 단계가 없다
      textMuted: "#637777", // comment
      running: "#82aaff",
      awaiting: "#ffcb8b",
      question: "#c792ea",
      done: "#addb67",
      failed: "#ff5874",
    },
    // github.com/sdras/night-owl-vscode-theme — 배포판 tokenColors.
    syntax: {
      keyword: "#c792ea",
      string: "#ecc48d",
      number: "#f78c6c",
      comment: "#637777", // 저대비분은 deriveSyntax가 끌어올린다
      type: "#ffcb8b",
      func: "#82aaff",
      variable: "#addb67",
      constant: "#f78c6c",
      operator: "#c792ea",
      tag: "#7fdbca",
    },
  },
  {
    // Praxis 오리지널 팔레트 — UI·구문 강조·터미널이 같은 색조를 공유한다.
    id: "cream",
    label: "Cream",
    blurb: "따뜻한 크림 종이와 차분한 테라코타.",
    kind: "light",
    counterpart: "coffee",
    palette: {
      bg: "#faf6ed",
      surface: "#f1e9da",
      raised: "#fffcf5",
      border: "#ddd0ba",
      borderStrong: "#b6a58b",
      primary: "#a4492f",
      text: "#302b25",
      text2: "#62564a",
      textMuted: "#887762",
      running: "#34689b",
      awaiting: "#916015",
      question: "#805795",
      done: "#46733c",
      failed: "#b23d4b",
    },
    syntax: {
      keyword: "#805795",
      string: "#46733c",
      number: "#916015",
      comment: "#887762",
      type: "#a4492f",
      func: "#34689b",
      variable: "#302b25",
      constant: "#b23d4b",
      operator: "#62564a",
      tag: "#a4492f",
    },
  },
  {
    // Praxis 오리지널 팔레트 — UI·구문 강조·터미널이 같은 색조를 공유한다.
    id: "coffee",
    label: "Coffee",
    blurb: "에스프레소 배경과 부드러운 캐러멜.",
    kind: "dark",
    counterpart: "cream",
    palette: {
      bg: "#211a17",
      surface: "#2c231e",
      raised: "#392e26",
      border: "#514236",
      borderStrong: "#796350",
      primary: "#d9ad78",
      text: "#f3e5d2",
      text2: "#c5b29c",
      textMuted: "#9b8774",
      running: "#87afd7",
      awaiting: "#e5bd66",
      question: "#bf9ed2",
      done: "#a3bd80",
      failed: "#e89388",
    },
    syntax: {
      keyword: "#bf9ed2",
      string: "#a3bd80",
      number: "#e5bd66",
      comment: "#9b8774",
      type: "#d9ad78",
      func: "#87afd7",
      variable: "#f3e5d2",
      constant: "#e89388",
      operator: "#c5b29c",
      tag: "#d9ad78",
    },
  },
  {
    // Praxis 오리지널 팔레트 — UI·구문 강조·터미널이 같은 색조를 공유한다.
    id: "sakura",
    label: "Sakura",
    blurb: "연한 벚꽃빛과 또렷한 로즈 포인트.",
    kind: "light",
    counterpart: "plum",
    palette: {
      bg: "#fff5f7",
      surface: "#f8e8ed",
      raised: "#fffafd",
      border: "#e6ccd5",
      borderStrong: "#c398aa",
      primary: "#a83261",
      text: "#39252e",
      text2: "#735562",
      textMuted: "#997381",
      running: "#386ba0",
      awaiting: "#93610e",
      question: "#8150a3",
      done: "#36754f",
      failed: "#b73839",
    },
    syntax: {
      keyword: "#8150a3",
      string: "#36754f",
      number: "#93610e",
      comment: "#997381",
      type: "#a83261",
      func: "#386ba0",
      variable: "#39252e",
      constant: "#b73839",
      operator: "#735562",
      tag: "#a83261",
    },
  },
  {
    // Praxis 오리지널 팔레트 — UI·구문 강조·터미널이 같은 색조를 공유한다.
    id: "plum",
    label: "Plum",
    blurb: "깊은 자두빛에 은은한 핑크 조명.",
    kind: "dark",
    counterpart: "sakura",
    palette: {
      bg: "#251b2b",
      surface: "#302238",
      raised: "#3e2e47",
      border: "#553f60",
      borderStrong: "#80658d",
      primary: "#e7a0c0",
      text: "#f4e5f5",
      text2: "#cab1cf",
      textMuted: "#a088a8",
      running: "#95b7ea",
      awaiting: "#e6bd79",
      question: "#bb9aee",
      done: "#9ac5a0",
      failed: "#ef969e",
    },
    syntax: {
      keyword: "#bb9aee",
      string: "#9ac5a0",
      number: "#e6bd79",
      comment: "#a088a8",
      type: "#e7a0c0",
      func: "#95b7ea",
      variable: "#f4e5f5",
      constant: "#ef969e",
      operator: "#cab1cf",
      tag: "#e7a0c0",
    },
  },
  {
    // Praxis 오리지널 팔레트 — UI·구문 강조·터미널이 같은 색조를 공유한다.
    id: "mint",
    label: "Mint",
    blurb: "맑은 민트 바탕의 산뜻한 작업 공간.",
    kind: "light",
    counterpart: "forest",
    palette: {
      bg: "#f0faf5",
      surface: "#e1f0e8",
      raised: "#f8fffb",
      border: "#c5ddd0",
      borderStrong: "#91b7a4",
      primary: "#197565",
      text: "#203c32",
      text2: "#4a6c5d",
      textMuted: "#718c7f",
      running: "#356baa",
      awaiting: "#906012",
      question: "#865299",
      done: "#48752c",
      failed: "#b94350",
    },
    syntax: {
      keyword: "#865299",
      string: "#48752c",
      number: "#906012",
      comment: "#718c7f",
      type: "#197565",
      func: "#356baa",
      variable: "#203c32",
      constant: "#b94350",
      operator: "#4a6c5d",
      tag: "#197565",
    },
  },
  {
    // Praxis 오리지널 팔레트 — UI·구문 강조·터미널이 같은 색조를 공유한다.
    id: "forest",
    label: "Forest",
    blurb: "짙은 숲의 녹색과 부드러운 세이지.",
    kind: "dark",
    counterpart: "mint",
    palette: {
      bg: "#16231e",
      surface: "#203129",
      raised: "#2c4035",
      border: "#3e5648",
      borderStrong: "#65816f",
      primary: "#9ac6ad",
      text: "#e2efe4",
      text2: "#adc8b5",
      textMuted: "#809f89",
      running: "#89b9e3",
      awaiting: "#e4bf7c",
      question: "#c3a1d9",
      done: "#b4ce83",
      failed: "#e9958b",
    },
    syntax: {
      keyword: "#c3a1d9",
      string: "#b4ce83",
      number: "#e4bf7c",
      comment: "#809f89",
      type: "#9ac6ad",
      func: "#89b9e3",
      variable: "#e2efe4",
      constant: "#e9958b",
      operator: "#adc8b5",
      tag: "#9ac6ad",
    },
  },
  {
    // Praxis 오리지널 팔레트 — UI·구문 강조·터미널이 같은 색조를 공유한다.
    id: "lavender",
    label: "Lavender",
    blurb: "옅은 라벤더와 선명한 아이리스.",
    kind: "light",
    counterpart: "midnight",
    palette: {
      bg: "#f7f5ff",
      surface: "#ece8f6",
      raised: "#fcfbff",
      border: "#d6cfea",
      borderStrong: "#aca0c6",
      primary: "#7051b5",
      text: "#302943",
      text2: "#655b7b",
      textMuted: "#897d9f",
      running: "#346da1",
      awaiting: "#936019",
      question: "#9b4384",
      done: "#39774f",
      failed: "#b63e55",
    },
    syntax: {
      keyword: "#9b4384",
      string: "#39774f",
      number: "#936019",
      comment: "#897d9f",
      type: "#7051b5",
      func: "#346da1",
      variable: "#302943",
      constant: "#b63e55",
      operator: "#655b7b",
      tag: "#7051b5",
    },
  },
  {
    // Praxis 오리지널 팔레트 — UI·구문 강조·터미널이 같은 색조를 공유한다.
    id: "midnight",
    label: "Midnight",
    blurb: "깊은 잉크 블루와 시원한 아이스 포인트.",
    kind: "dark",
    counterpart: "lavender",
    palette: {
      bg: "#111c2e",
      surface: "#1b2940",
      raised: "#283951",
      border: "#3c506a",
      borderStrong: "#637d9b",
      primary: "#8cd4df",
      text: "#e3edfa",
      text2: "#aebfd6",
      textMuted: "#8298b4",
      running: "#96b6f5",
      awaiting: "#e7bd7b",
      question: "#c5a5ed",
      done: "#98cbaa",
      failed: "#ef9aab",
    },
    syntax: {
      keyword: "#c5a5ed",
      string: "#98cbaa",
      number: "#e7bd7b",
      comment: "#8298b4",
      type: "#8cd4df",
      func: "#96b6f5",
      variable: "#e3edfa",
      constant: "#ef9aab",
      operator: "#aebfd6",
      tag: "#8cd4df",
    },
  },
];

// ─── 파생 ──────────────────────────────────────────────────────────────────

/** 상태색·보조 텍스트의 최소 명암비. DESIGN.md 기준(본문 AA, muted 3.5). */
const FLOOR = { text: 7, text2: 4.5, muted: 3.5, status: 4.5 } as const;

function derive(spec: ThemeSpec): ThemeTokens {
  const dark = spec.kind === "dark";
  const p = { ...spec.palette };

  if (spec.autoContrast !== false) {
    p.text = ensureContrast(p.text, p.bg, FLOOR.text);
    p.text2 = ensureContrast(p.text2, p.bg, FLOOR.text2);
    p.textMuted = ensureContrast(p.textMuted, p.bg, FLOOR.muted);
    for (const k of ["running", "awaiting", "question", "done", "failed"] as const) {
      p[k] = ensureContrast(p[k], p.bg, FLOOR.status);
    }
  }

  // 다크는 밝은 회색을, 라이트는 어두운 회색을 반투명으로 띄운다(기존 값과 동일한 규칙).
  const thumbBase = dark ? p.text2 : p.text;
  const thumb: [number, number, number] = dark ? [0.3, 0.5, 0.65] : [0.25, 0.4, 0.55];

  // diff 틴트는 배경에 상태색을 섞어 만든다. 라이트는 흰 배경이라 같은 비율로는 티가 안 나
  // 더 진하게 섞는다.
  const t = dark ? { soft: 0.12, strong: 0.32, danger: 0.1, dangerLine: 0.55 } : { soft: 0.2, strong: 0.42, danger: 0.06, dangerLine: 0.45 };

  const derived: DerivedTokens = {
    primaryBright: lighten(p.primary, 0.1),
    primaryHover: darken(p.primary, 0.08),
    addbg: mix(p.bg, p.done, t.soft),
    delbg: mix(p.bg, p.failed, t.soft),
    addbgStrong: mix(p.bg, p.done, t.strong),
    delbgStrong: mix(p.bg, p.failed, t.strong),
    dangerbg: mix(p.bg, p.failed, t.danger),
    dangerborder: mix(p.bg, p.failed, t.dangerLine),
    empty: p.surface,
    scrollbarThumb: alpha(thumbBase, thumb[0]),
    scrollbarThumbHover: alpha(thumbBase, thumb[1]),
    scrollbarThumbActive: alpha(thumbBase, thumb[2]),
    termCursor: "",
    // xterm은 rgba() 대신 8자리 hex로 넘긴다 — 기존 값(#14b8a633)과 같은 형식이다.
    termSelection: hexAlpha(p.primary, 0.2),
  };
  // 커서는 배경에서 튀어야 한다 — 다크는 밝은 변형, 라이트는 어두운 변형.
  derived.termCursor = dark ? derived.primaryBright : derived.primaryHover;

  const merged = { ...derived, ...spec.overrides };
  // override로 액센트 변형이 바뀌면 커서도 따라간다(기본 테마의 기존 커서 색 보존).
  // 단, 커서를 직접 지정했으면 그 값이 이긴다 — 명시가 파생을 이긴다.
  if (!spec.overrides?.termCursor && (spec.overrides?.primaryBright || spec.overrides?.primaryHover)) {
    merged.termCursor = dark ? merged.primaryBright : merged.primaryHover;
  }
  return { ...p, ...merged };
}

/**
 * 구문색 AA 보정. tokens와 **별도로** 돌린다 — 파생 토큰 객체에 섞으면 기존 테마의
 * 픽셀 보존 계약(themes.test.ts)이 깨진다.
 *
 * comment만 floor 3.5(muted 단계)인 것은 주석이 의도적으로 죽인 텍스트이기 때문이고,
 * 나머지 9키는 본문(7:1)이 아니라 보조 신호라 4.5다.
 */
function deriveSyntax(spec: ThemeSpec): SyntaxPalette | undefined {
  if (!spec.syntax) return undefined;
  const bg = spec.palette.bg;
  const out = { ...spec.syntax };
  for (const k of Object.keys(out) as (keyof SyntaxPalette)[]) {
    out[k] = ensureContrast(out[k], bg, k === "comment" ? FLOOR.muted : FLOOR.status);
  }
  return out;
}

/** AA 보정이 입력값을 바꿨을 때 편집 화면이 보여줄 근거. 같으면 null(=보정 없음). */
export interface ContrastNote {
  /** 입력값과 배경의 명암비 — 왜 보정됐는지의 근거다. */
  ratio: string;
  corrected: string;
}

/** 침묵 금지 규약(설계 0049): 보정이 개입하면 입력 옆에 결과를 드러낸다. */
export function contrastNote(input: string, corrected: string, bg: string): ContrastNote | null {
  if (input.toLowerCase() === corrected.toLowerCase()) return null;
  return { ratio: contrastRatio(input, bg).toFixed(1), corrected };
}

/** spec → Theme. 빌트인·커스텀·드래프트가 전부 이 파이프라인을 탄다. */
export function deriveTheme(spec: ThemeSpec): Theme {
  return {
    id: spec.id,
    label: spec.label,
    blurb: spec.blurb,
    kind: spec.kind,
    counterpart: spec.counterpart,
    tokens: derive(spec),
    syntax: deriveSyntax(spec),
  };
}

export const THEMES: Theme[] = SPECS.map((s) => deriveTheme(s));

export const DEFAULT_THEME_ID = "praxis-dark";

/** 커스텀 테마 id 접두사. Rust `theme_store::valid_id`와 같은 규약이다. */
const CUSTOM_ID_PREFIX = "custom-";

/** 편집 중 드래프트가 쓰는 id. 레지스트리에도 파일에도 없는 임시 이름이다. */
export const DRAFT_THEME_ID = "custom-draft";

export function isCustomThemeId(id: string): boolean {
  return id.startsWith(CUSTOM_ID_PREFIX);
}

// 커스텀 테마는 파일에서 오므로 THEMES(빌트인 상수)와 분리한다. 파일 IO는 theme-files.ts가
// 소유하고 여기는 등록만 받는다 — 부팅·보조 창에서도 이 모듈은 동기·무IO로 남는다.
let customThemes: Theme[] = [];
let revision = 0;

/** 레지스트리를 통째로 교체하고 구독자(설정 그리드·Monaco)에게 알린다. */
export function registerCustomThemes(themes: Theme[]): void {
  customThemes = themes;
  revision += 1;
  for (const cb of listeners) cb();
}

/**
 * 목록 스냅샷. 활성 테마가 그대로면 `getActiveTheme`은 같은 참조를 돌려주므로
 * useSyncExternalStore가 리렌더를 건너뛴다 — 목록만 바뀌는 삭제·가져오기를 보려면 이 값을 봐야 한다.
 */
export function getThemesRevision(): number {
  return revision;
}

export function allThemes(): Theme[] {
  return [...THEMES, ...customThemes];
}

export function getTheme(id: string): Theme {
  return allThemes().find((t) => t.id === id) ?? THEMES[0];
}

// ─── CSS 변수 ──────────────────────────────────────────────────────────────

/** 토큰 → CSS 변수. index.css의 :root/.dark 정의와 이름이 일치해야 한다. */
const CSS_VARS: Partial<Record<keyof ThemeTokens, string>> = {
  bg: "--c-bg",
  surface: "--c-surface",
  raised: "--c-raised",
  border: "--c-border",
  borderStrong: "--c-border-strong",
  primary: "--c-primary",
  primaryBright: "--c-primary-bright",
  primaryHover: "--c-primary-hover",
  text: "--c-text",
  text2: "--c-text-2",
  textMuted: "--c-text-muted",
  running: "--c-running",
  awaiting: "--c-awaiting",
  question: "--c-question",
  done: "--c-done",
  failed: "--c-failed",
  addbg: "--c-addbg",
  delbg: "--c-delbg",
  addbgStrong: "--c-addbg-strong",
  delbgStrong: "--c-delbg-strong",
  dangerbg: "--c-dangerbg",
  dangerborder: "--c-dangerborder",
  empty: "--c-empty",
  scrollbarThumb: "--c-scrollbar-thumb",
  scrollbarThumbHover: "--c-scrollbar-thumb-hover",
  scrollbarThumbActive: "--c-scrollbar-thumb-active",
};

// ─── 저장 ──────────────────────────────────────────────────────────────────

const STORAGE_KEY = "praxis-theme";

/**
 * 저장된 테마 id. 이진 토글 시절의 "light"/"dark"도 읽는다 — 같은 키를 계속 쓰므로
 * 구버전 앱으로 되돌아가도 모르는 id는 그쪽에서 다크로 폴백해 무해하다.
 */
export function loadThemeId(): string {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(STORAGE_KEY);
  } catch {
    return DEFAULT_THEME_ID;
  }
  if (raw === "light") return "praxis-light";
  if (raw === "dark" || !raw) return DEFAULT_THEME_ID;
  if (allThemes().some((t) => t.id === raw)) return raw;
  // 커스텀은 부팅 시점에 아직 레지스트리가 비어 있다 — 접두사만 보고 통과시키고, 파일이
  // 사라졌으면 applyTheme의 getTheme 폴백이 기본 테마로 흡수한다.
  return isCustomThemeId(raw) ? raw : DEFAULT_THEME_ID;
}

// ─── 적용 ──────────────────────────────────────────────────────────────────

let active: Theme = getTheme(DEFAULT_THEME_ID);
const listeners = new Set<() => void>();

/** useSyncExternalStore용 — 같은 테마면 같은 참조를 돌려준다. */
export function getActiveTheme(): Theme {
  return active;
}

export function subscribeTheme(cb: () => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

/**
 * CSS 변수·`.dark` 클래스·`color-scheme`을 한 번에 갱신한다.
 * 첫 프레임 전(main.tsx)에 불러 FOUC를 막고, 이후 설정 변경 때 다시 부른다.
 */
export function applyTheme(id: string, persist = true): Theme {
  const theme = getTheme(id);
  // 폴백이 일어났으면 저장하지 않는다 — 커스텀 테마는 파일 로드 전까지 레지스트리에 없어서
  // 부팅 첫 applyTheme이 늘 기본 테마로 떨어진다. 여기서 저장하면 사용자의 선호가 영구히 덮인다.
  return applyThemeObject(theme, persist && theme.id === id);
}

/**
 * 적용 본체. 레지스트리에 없는 테마(편집 드래프트)도 같은 경로를 타도록 id 조회와 분리했다.
 */
function applyThemeObject(theme: Theme, persist: boolean): Theme {
  const root = document.documentElement;

  for (const [key, cssVar] of Object.entries(CSS_VARS)) {
    root.style.setProperty(cssVar, theme.tokens[key as keyof ThemeTokens]);
  }
  // 커스텀 스타일이 닿지 않는 네이티브 UI(폼 컨트롤·스크롤바 폴백)가 테마를 따르게 한다.
  root.style.setProperty("color-scheme", theme.kind);
  // Tailwind `dark:` variant와 .dark 셀렉터가 계속 동작하도록 kind로 유지한다.
  root.classList.toggle("dark", theme.kind === "dark");
  root.dataset.theme = theme.id;

  active = theme;
  if (persist) {
    try {
      localStorage.setItem(STORAGE_KEY, theme.id);
    } catch {
      // 시크릿 모드 등 — 적용은 됐으니 저장 실패는 삼킨다.
    }
  }
  for (const cb of listeners) cb();
  return theme;
}

/** 편집 중 앱 전체 임시 적용. persist 없음 — 취소는 applyTheme(이전 id)로 롤백된다. */
export function applyDraft(spec: ThemeSpec): Theme {
  return applyThemeObject(deriveTheme({ ...spec, id: DRAFT_THEME_ID }), false);
}

/**
 * 다른 창이 계산한 테마를 그대로 입는다 (창 간 동기화 수신부 — `theme-sync.ts`).
 *
 * 레지스트리에 넣지 않고 저장도 하지 않는다. 보조 창에서 테마를 읽는 소비자(CSS 변수·Monaco·
 * xterm)는 전부 활성 테마만 보므로, 목록에 없어도 색은 맞는다. 저장은 테마를 고른 창의 몫이다.
 */
export function adoptTheme(theme: Theme): Theme {
  return applyThemeObject(theme, false);
}

/** 라이트↔다크 토글이 갈 곳. 짝이 없으면 반대 kind의 기본 테마. */
export function counterpartOf(theme: Theme): string {
  if (theme.counterpart) return theme.counterpart;
  return theme.kind === "dark" ? "praxis-light" : DEFAULT_THEME_ID;
}

// ─── 소비자별 파생 ─────────────────────────────────────────────────────────

/**
 * xterm ITheme (구조적 타입 — @xterm/xterm 의존을 만들지 않는다).
 *
 * ANSI 16은 상태 5색에서 파생한다 — 이미 AA 보정을 거친 값이라 어느 테마에서도 읽힌다.
 * 기존 4키는 값이 그대로이고 키만 늘어난다(플랜 0049 DR-2).
 */
export function xtermTheme(theme: Theme = active) {
  const t = theme.tokens;
  const dark = theme.kind === "dark";
  // bright는 "배경에서 더 튄다"는 뜻이다 — 다크 배경에선 밝게, 라이트 배경에선 진하게.
  // 라이트에서 lighten하면 흰 배경에 묻혀 bold 텍스트가 오히려 안 읽힌다.
  const emphasize = dark ? lighten : darken;
  return {
    background: t.bg,
    foreground: t.text,
    cursor: t.termCursor,
    selectionBackground: t.termSelection,
    // 다크의 black은 "배경보다 한 단계 위"(raised)다. 라이트에서 같은 자리를 보더색으로 채우면
    // 흰 배경 대비 1.35:1이라 글자가 사라진다 — 본문색(AA 7:1 보장)을 쓴다.
    black: dark ? t.raised : t.text,
    red: t.failed,
    green: t.done,
    yellow: t.awaiting,
    blue: t.running,
    magenta: t.question,
    cyan: t.primary,
    white: t.text2,
    brightBlack: t.textMuted,
    brightRed: emphasize(t.failed, 0.1),
    brightGreen: emphasize(t.done, 0.1),
    brightYellow: emphasize(t.awaiting, 0.1),
    brightBlue: emphasize(t.running, 0.1),
    brightMagenta: emphasize(t.question, 0.1),
    brightCyan: emphasize(t.primary, 0.1),
    brightWhite: t.text,
  };
}
