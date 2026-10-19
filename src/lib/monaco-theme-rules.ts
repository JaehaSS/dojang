// 테마 → 토큰 색 규칙 표. Monaco 테마 정의(monaco.ts)와 diff 구문 강조(diff-highlight.ts)가
// 같은 표를 읽는다 — 두 곳이 각자 표를 가지면 에디터와 diff 본문의 색이 어긋난다(설계 DR-11).
// monaco를 import하지 않는다: diff 강조는 jsdom 테스트에서도 돌아야 한다.
import type { Theme } from "./themes";

export interface TokenRule {
  token: string;
  foreground: string;
  fontStyle?: string;
}

const hex = (c: string) => c.replace("#", "");

/** Monaco `defineTheme`의 `rules`로 들어가는 표. 색은 `#` 없는 6자리 hex다. */
export function syntaxTokenRules(theme: Theme): TokenRule[] {
  const s = theme.syntax;
  const pythonFunction = theme.kind === "dark" ? "DCDCAA" : "795E26";
  // Monaco standalone은 semantic token type도 일반 token rule로 해석한다. Java LSP의
  // class·method·variable 같은 역할 이름을 여기서 색으로 연결해야 기본 본문색으로 합쳐지지 않는다.
  const semanticType = s ? hex(s.type) : theme.kind === "dark" ? "4EC9B0" : "267F99";
  const semanticFunction = s ? hex(s.func) : pythonFunction;
  const semanticVariable = s ? hex(s.variable) : theme.kind === "dark" ? "9CDCFE" : "001080";
  const semanticRules: TokenRule[] = [
    { token: "class", foreground: semanticType },
    { token: "interface", foreground: semanticType, fontStyle: "italic" },
    { token: "type", foreground: semanticType },
    { token: "enum", foreground: semanticType },
    { token: "method", foreground: semanticFunction },
    { token: "function", foreground: semanticFunction },
    { token: "variable", foreground: semanticVariable },
    { token: "property", foreground: semanticVariable },
    { token: "parameter", foreground: semanticVariable, fontStyle: "italic" },
  ];
  // syntax 없는 테마는 Monaco 기본색을 상속하되 Python 함수만 구분한다.
  // 기본 Praxis 2종은 syntax 팔레트가 없으므로 이 규칙이 def·호출 이름을 분리한다.
  if (!s) return [{ token: "function.python", foreground: pythonFunction }, ...semanticRules];
  // syntax 팔레트에서는 `identifier`도 명시해 색의 출처를 테마로 고정한다.
  // 팔레트 없는 테마의 identifier는 Monaco 본문색을 그대로 상속한다.
  const rules: TokenRule[] = [
    { token: "keyword", foreground: hex(s.keyword) },
    { token: "string", foreground: hex(s.string) },
    { token: "string.escape", foreground: hex(s.string) },
    { token: "number", foreground: hex(s.number) },
    { token: "comment", foreground: hex(s.comment) },
    { token: "comment.doc", foreground: hex(s.comment) },
    { token: "type", foreground: hex(s.type) },
    { token: "type.identifier", foreground: hex(s.type) },
    { token: "namespace", foreground: hex(s.type) },
    { token: "function", foreground: hex(s.func) },
    { token: "annotation", foreground: hex(s.func) },
    { token: "variable", foreground: hex(s.variable) },
    { token: "variable.predefined", foreground: hex(s.variable) },
    { token: "constant", foreground: hex(s.constant) },
    { token: "regexp", foreground: hex(s.constant) },
    { token: "operator", foreground: hex(s.operator) },
    { token: "delimiter", foreground: hex(s.operator) },
    { token: "tag", foreground: hex(s.tag) },
    { token: "attribute.name", foreground: hex(s.tag) },
    { token: "metatag", foreground: hex(s.tag) },
    // Monarch가 언어마다 다른 이름으로 내보내는 것들. 여기 없으면 그 언어에서만
    // 색이 빠져 "TS는 되는데 Rust는 안 된다" 같은 증상이 된다.
    { token: "identifier", foreground: hex(s.variable) },
    { token: "entity.name.function", foreground: hex(s.func) },
    { token: "support.function", foreground: hex(s.func) },
    { token: "entity.name.type", foreground: hex(s.type) },
    { token: "entity.name.type.class", foreground: hex(s.type) },
    { token: "entity.name.type.interface", foreground: hex(s.type), fontStyle: "italic" },
    { token: "support.type", foreground: hex(s.type) },
    { token: "storage.type", foreground: hex(s.keyword) },
    { token: "keyword.control", foreground: hex(s.keyword) },
    { token: "constant.language", foreground: hex(s.constant) },
    { token: "constant.numeric", foreground: hex(s.number) },
    { token: "variable.parameter", foreground: hex(s.variable), fontStyle: "italic" },
    { token: "variable.other.constant", foreground: hex(s.constant) },
  ];
  return rules.concat(semanticRules);
}

/** 팔레트 없는 테마의 diff 강조용 대체 표 — Monaco vs/vs-dark 내장색에 준한다.
 *  Monaco 에디터는 `inherit: true`로 내장 테마를 상속하지만 diff 본문은 그 표를 읽을 수
 *  없으므로, 주요 토큰만 같은 값으로 옮겨 둔다. 키는 `#` 없는 토큰 범위다. */
export function defaultTokenColors(kind: "dark" | "light"): Record<string, string> {
  const dark = kind === "dark";
  return {
    keyword: dark ? "569CD6" : "0000FF",
    string: dark ? "CE9178" : "A31515",
    number: dark ? "B5CEA8" : "098658",
    comment: dark ? "6A9955" : "008000",
    type: dark ? "4EC9B0" : "267F99",
    function: dark ? "DCDCAA" : "795E26",
    variable: dark ? "9CDCFE" : "001080",
    constant: dark ? "4FC1FF" : "0070C1",
    regexp: dark ? "D16969" : "811F3F",
    tag: dark ? "569CD6" : "800000",
    "attribute.name": dark ? "9CDCFE" : "E50000",
    operator: dark ? "D4D4D4" : "000000",
    delimiter: dark ? "D4D4D4" : "000000",
    annotation: dark ? "DCDCAA" : "795E26",
    metatag: dark ? "569CD6" : "800000",
  };
}
