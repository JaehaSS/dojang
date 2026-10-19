// Monaco를 로컬 번들로 구성 (CDN 미사용 — local-first 원칙).
// Vite의 `?worker` import로 언어 워커를 별도 청크로 분리해 메인 스레드 차단 방지.
import * as monaco from "monaco-editor";
import { loader } from "@monaco-editor/react";
import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
import jsonWorker from "monaco-editor/esm/vs/language/json/json.worker?worker";
import cssWorker from "monaco-editor/esm/vs/language/css/css.worker?worker";
import htmlWorker from "monaco-editor/esm/vs/language/html/html.worker?worker";
import tsWorker from "monaco-editor/esm/vs/language/typescript/ts.worker?worker";
import { installPythonHighlighting } from "./python-highlighting";
import { getActiveTheme, subscribeTheme, THEMES, type Theme } from "./themes";
import { selectAllInFocusedSurface } from "./select-all";
import { syntaxTokenRules } from "./monaco-theme-rules";
import { setDiffTokenizer, type TokenSpan } from "./diff-highlight";

window.addEventListener("dojang:select-all", (event) => {
  event.preventDefault();
  selectAllInFocusedSurface(monaco.editor.getEditors(), document);
});

(self as unknown as { MonacoEnvironment: monaco.Environment }).MonacoEnvironment = {
  getWorker(_workerId, label) {
    if (label === "json") return new jsonWorker();
    if (label === "css" || label === "scss" || label === "less") return new cssWorker();
    if (label === "html" || label === "handlebars" || label === "razor") return new htmlWorker();
    if (label === "typescript" || label === "javascript") return new tsWorker();
    return new editorWorker();
  },
};

// 앱 색 토큰에 맞춘 표면색 + 테마가 선언한 구문색.
// Monaco 테마 이름 = themes.ts의 테마 id — EditorPane이 활성 테마 id를 그대로 넘긴다.
/** 같은 이름 defineTheme은 덮어쓰기이므로 커스텀/드래프트 재정의에도 그대로 쓴다. */
export function defineMonacoTheme(theme: Theme) {
  const t = theme.tokens;
  monaco.editor.defineTheme(theme.id, {
    base: theme.kind === "dark" ? "vs-dark" : "vs",
    inherit: true,
    // 규칙 표는 monaco-theme-rules.ts가 소유한다 — diff 구문 강조가 같은 표를 읽는다(설계 DR-11).
    rules: syntaxTokenRules(theme),
    colors: {
      "editor.background": t.bg,
      "editor.foreground": t.text,
      "editorLineNumber.foreground": t.textMuted,
      "editorLineNumber.activeForeground": t.text2,
      "editorGutter.background": t.bg,
      "editorIndentGuide.background1": t.border,
      // 선택 배경은 내장 테마 값을 그대로 둔다 — 표면색보다 대비 요구가 까다롭고,
      // 팔레트에서 파생한 회색은 선택 영역을 오히려 흐리게 만든다.
      "editorWidget.background": t.raised,
      "editorWidget.border": t.border,
    },
  });
}

for (const theme of THEMES) defineMonacoTheme(theme);
// 커스텀·드래프트 테마는 로드 시점에 없다 — 활성 전환 때마다 (재)정의한다.
// EditorPane 렌더는 subscribe 콜백 뒤에 오므로 정의가 항상 선행된다.
subscribeTheme(() => defineMonacoTheme(getActiveTheme()));
defineMonacoTheme(getActiveTheme());

// @monaco-editor/react가 번들된 monaco를 쓰도록 (기본 CDN 로더 대체).
loader.config({ monaco });
installPythonHighlighting(monaco);

// diff 본문 구문 강조의 토크나이저 주입(설계 DR-12). diff-highlight.ts는 monaco를 모르므로
// 여기서 등록한다 — 등록 전(부팅·테스트)에는 diff가 평문으로 그려진다.
setDiffTokenizer(async (lines, lang) => {
  const empty = (): TokenSpan[][] => lines.map(() => []);
  const entry = monaco.languages.getLanguages().find((language) => language.id === lang);
  if (!entry || lang === "plaintext") return empty();
  // standalone 빌드의 언어는 지연 로드다. `tokenize`는 토크나이저 팩토리를 깨우기만 하고
  // 기다리지 않아, 에디터가 아직 그 언어를 열지 않았으면 첫 호출이 전부 평문으로 돌아온다.
  // 언어 정의의 `loader`는 모듈 import만 하고 팩토리를 해소하지 않는다(0.55 기준).
  // 공개 API 중 팩토리 해소를 기다리는 것은 `colorize`뿐이라 빈 문자열로 한 번 부른다.
  try {
    await monaco.editor.colorize("", lang, {});
  } catch {
    // 해소에 실패해도 tokenize는 평문으로 동작한다.
  }
  // tokenize는 `\r`도 줄 경계로 본다. CRLF 파일의 줄 끝 `\r`이 남아 있으면 행이 밀려
  // 다른 줄의 색을 칠하게 되므로, 길이가 같은 공백으로 바꿔 offset과 1:1 정렬을 지킨다.
  const rows = monaco.editor.tokenize(lines.map((line) => line.replace(/\r/g, " ")).join("\n"), lang);
  return lines.map((line, index) =>
    (rows[index] ?? []).map((token, i, row) => ({
      start: token.offset,
      end: row[i + 1]?.offset ?? line.length,
      type: token.type,
    })),
  );
});

// 언어 판별은 lang-from-path.ts가 소유한다(monaco 의존 없는 순수 모듈) — 기존 import 경로 보존을 위해 재수출.
export { langFromPath } from "./lang-from-path";

export { monaco };
