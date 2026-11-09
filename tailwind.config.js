/** @type {import('tailwindcss').Config} */
// 토큰 정본: DESIGN.md (Chromatic Discipline). 여기 값은 그 미러.
export default {
  content: ["./index.html", "./mobile.html", "./src/**/*.{ts,tsx}"],
  darkMode: "class",
  theme: {
    extend: {
      // 고도 3단계만 쓴다. 값은 index.css `--shadow-e*`(라이트/다크 분기)
      boxShadow: {
        e1: "var(--shadow-e1)",
        e2: "var(--shadow-e2)",
        e3: "var(--shadow-e3)",
      },
      colors: {
        // 표면/텍스트/보더는 CSS 변수로 → .dark 토글로 라이트/다크 전환 (index.css 정의).
        // `rgb(from … / <alpha-value>)` 상대 색 문법이 없으면 Tailwind가 `/40` 같은
        // 불투명도 수식 클래스를 아예 생성하지 못한다 — 인풋이 UA 기본 흰 배경으로 떨어졌던 원인.
        bg: "rgb(from var(--c-bg) r g b / <alpha-value>)",
        surface: "rgb(from var(--c-surface) r g b / <alpha-value>)",
        raised: "rgb(from var(--c-raised) r g b / <alpha-value>)",
        border: "rgb(from var(--c-border) r g b / <alpha-value>)",
        "border-strong": "rgb(from var(--c-border-strong) r g b / <alpha-value>)",
        // 액센트도 테마마다 달라진다(themes.ts) — 리터럴로 두면 Praxis 기본 테마의 teal이
        // 다른 팔레트 위에 그대로 남는다.
        primary: {
          DEFAULT: "rgb(from var(--c-primary) r g b / <alpha-value>)",
          bright: "rgb(from var(--c-primary-bright) r g b / <alpha-value>)",
          hover: "rgb(from var(--c-primary-hover) r g b / <alpha-value>)",
        },
        text: {
          DEFAULT: "rgb(from var(--c-text) r g b / <alpha-value>)",
          secondary: "rgb(from var(--c-text-2) r g b / <alpha-value>)",
          muted: "rgb(from var(--c-text-muted) r g b / <alpha-value>)",
        },
        status: {
          running: "rgb(from var(--c-running) r g b / <alpha-value>)",
          awaiting: "rgb(from var(--c-awaiting) r g b / <alpha-value>)",
          question: "rgb(from var(--c-question) r g b / <alpha-value>)",
          done: "rgb(from var(--c-done) r g b / <alpha-value>)",
          failed: "rgb(from var(--c-failed) r g b / <alpha-value>)",
        },
        // diff/danger 틴트 (라이트/다크 대비 — index.css 정의)
        addbg: "rgb(from var(--c-addbg) r g b / <alpha-value>)",
        delbg: "rgb(from var(--c-delbg) r g b / <alpha-value>)",
        "addbg-strong": "rgb(from var(--c-addbg-strong) r g b / <alpha-value>)",
        "delbg-strong": "rgb(from var(--c-delbg-strong) r g b / <alpha-value>)",
        dangerbg: "rgb(from var(--c-dangerbg) r g b / <alpha-value>)",
        dangerborder: "rgb(from var(--c-dangerborder) r g b / <alpha-value>)",
        empty: "rgb(from var(--c-empty) r g b / <alpha-value>)",
        // 파일 타입 틴트 — 트리 아이콘 전용 보조 채널. 테마와 무관한 고정 2벌(index.css).
        ft: {
          folder: "rgb(from var(--ft-folder) r g b / <alpha-value>)",
          code: "rgb(from var(--ft-code) r g b / <alpha-value>)",
          doc: "rgb(from var(--ft-doc) r g b / <alpha-value>)",
          style: "rgb(from var(--ft-style) r g b / <alpha-value>)",
          data: "rgb(from var(--ft-data) r g b / <alpha-value>)",
        },
      },
      fontFamily: {
        // 정본: index.css :root --font-ui/--font-code (기본값 = 기존 하드코딩 체인, 런타임 갱신은 App.tsx applyFonts)
        ui: "var(--font-ui)",
        code: "var(--font-code)",
      },
      fontSize: {
        // 2026-11-09 한 단계 올림(11→12, 12→13). 11px 이하가 약 950회 쓰여 화면 대부분이
        // 메타 크기로 읽혔다. 메타·단축키 칩은 `text-[11px]`이 하한이다 — 9·10px은 쓰지 않는다.
        xs: "12px",
        sm: "13px",
        base: "13px",
        md: "14px",
        lg: "16px",
        xl: "20px",
        "2xl": "24px",
      },
    },
  },
  plugins: [],
};
