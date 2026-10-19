// diff 본문 구문 강조의 순수 로직 — 토큰 색 해석, 단어 강조와의 병합, hunk 스트림 구성.
// monaco를 import하지 않는다(설계 DR-12). 토크나이저는 monaco.ts가 `setDiffTokenizer`로
// 주입하고, 주입 전(부팅·테스트)에는 diff가 평문으로 그려진다.
import type { HunkLine } from "./annotations";
import type { Segment } from "./diff";
import type { DiffHunk } from "./ipc";
import { defaultTokenColors, syntaxTokenRules } from "./monaco-theme-rules";
import type { Theme } from "./themes";

/** 한 줄 안의 토큰 하나. 문자 offset 기준 [start, end). type은 Monaco 토큰 이름(`keyword.ts`). */
export interface TokenSpan {
  start: number;
  end: number;
  type: string;
}

/** 줄 배열을 받아 줄마다 토큰을 돌려준다. 결과 행은 입력 줄과 1:1로 정렬돼야 한다. */
export type DiffTokenizer = (lines: string[], lang: string) => TokenSpan[][] | Promise<TokenSpan[][]>;

let tokenizer: DiffTokenizer | null = null;

export function setDiffTokenizer(next: DiffTokenizer | null): void {
  tokenizer = next;
}

export function getDiffTokenizer(): DiffTokenizer | null {
  return tokenizer;
}

export interface TokenStyle {
  color?: string;
  italic?: boolean;
}

// 테마 객체는 전환 때만 바뀐다 — 줄마다 규칙 표를 다시 만들지 않도록 테마별로 묶어 둔다.
const styleTables = new WeakMap<Theme, { rules: Map<string, TokenStyle>; defaults: Map<string, TokenStyle> }>();

function tablesFor(theme: Theme) {
  let tables = styleTables.get(theme);
  if (!tables) {
    const rules = new Map<string, TokenStyle>();
    // 같은 토큰이 두 번 나오면 Monaco처럼 뒤의 규칙이 이긴다(semantic 규칙이 뒤에 붙는다).
    for (const rule of syntaxTokenRules(theme)) {
      rules.set(rule.token, {
        color: `#${rule.foreground}`,
        italic: rule.fontStyle?.includes("italic") || undefined,
      });
    }
    // 팔레트 있는 테마는 표가 완결돼 있다. 없는 테마만 Monaco 내장색에 준하는 표로 메운다.
    const defaults = new Map<string, TokenStyle>();
    if (!theme.syntax) {
      for (const [token, color] of Object.entries(defaultTokenColors(theme.kind))) {
        defaults.set(token, { color: `#${color}` });
      }
    }
    tables = { rules, defaults };
    styleTables.set(theme, tables);
  }
  return tables;
}

/** 토큰 종류 → 전경 스타일. Monaco 규칙 매칭과 같이 **가장 긴 접두 범위**가 이긴다 —
 *  `keyword.ts`는 `keyword.ts`, `keyword` 순으로 찾는다. 언어 접미사를 따로 알 필요가 없다.
 *  같은 길이에서는 테마 규칙이 대체 표보다 앞선다. 평문·모르는 토큰은 undefined다. */
export function tokenStyle(type: string, theme: Theme): TokenStyle | undefined {
  if (!type) return undefined;
  const { rules, defaults } = tablesFor(theme);
  let scope = type;
  for (;;) {
    const hit = rules.get(scope) ?? defaults.get(scope);
    if (hit) return hit;
    const dot = scope.lastIndexOf(".");
    if (dot < 0) return undefined;
    scope = scope.slice(0, dot);
  }
}

/** 토큰 종류 → `#rrggbb`. */
export function tokenColor(type: string, theme: Theme): string | undefined {
  return tokenStyle(type, theme)?.color;
}

export interface LinePiece {
  text: string;
  changed: boolean;
  color?: string;
  italic?: boolean;
}

/** 토큰 색(전경)과 단어 강조(배경)를 한 줄 안에서 교차시킨다(설계 DR-13). 두 축이 독립이라
 *  양쪽 경계를 모두 자르면 조각마다 색 하나·강조 여부 하나가 남는다. 이어 붙이면 원문이다. */
export function mergeLinePieces(
  text: string,
  tokens: TokenSpan[] | undefined,
  segments: Segment[] | undefined,
  resolve: (type: string) => TokenStyle | undefined,
): LinePiece[] {
  const cuts = new Set<number>([0, text.length]);
  const clamp = (n: number) => Math.max(0, Math.min(text.length, n));
  for (const token of tokens ?? []) {
    cuts.add(clamp(token.start));
    cuts.add(clamp(token.end));
  }
  const changedRanges: [number, number][] = [];
  let offset = 0;
  for (const segment of segments ?? []) {
    const end = offset + segment.text.length;
    if (segment.changed) changedRanges.push([offset, end]);
    cuts.add(clamp(offset));
    cuts.add(clamp(end));
    offset = end;
  }
  const points = [...cuts].sort((a, b) => a - b);

  const pieces: LinePiece[] = [];
  for (let i = 0; i + 1 < points.length; i++) {
    const from = points[i];
    const to = points[i + 1];
    if (to <= from) continue;
    const token = tokens?.find((t) => t.start <= from && from < t.end);
    const style = token ? resolve(token.type) : undefined;
    const changed = changedRanges.some(([a, b]) => a <= from && from < b);
    const last = pieces[pieces.length - 1];
    // 경계만 다르고 모양이 같은 이웃은 합친다 — span 수를 줄 길이에 비례하지 않게 둔다.
    if (last && last.changed === changed && last.color === style?.color && last.italic === style?.italic) {
      last.text += text.slice(from, to);
      continue;
    }
    const piece: LinePiece = { text: text.slice(from, to), changed };
    if (style?.color) piece.color = style.color;
    if (style?.italic) piece.italic = true;
    pieces.push(piece);
  }
  return pieces;
}

/** hunk 하나의 두 토큰화 스트림(설계 DR-11). 줄 하나만 따로 토큰화하면 여러 줄 주석·문자열
 *  안쪽을 코드로 읽는다 — 앞뒤 줄을 이어서 넘겨야 한다. old = context+del, new = context+add.
 *  context 줄은 new 스트림의 토큰을, del 줄은 old 스트림의 토큰을 쓴다. */
export function hunkTokenStreams(lines: HunkLine[]): {
  oldText: string[];
  newText: string[];
  indexFor: (lineIndex: number) => { stream: "old" | "new"; row: number };
} {
  const oldText: string[] = [];
  const newText: string[] = [];
  const positions: { stream: "old" | "new"; row: number }[] = [];
  for (const line of lines) {
    if (line.kind !== "add") oldText.push(line.text);
    if (line.kind !== "del") newText.push(line.text);
    positions.push(
      line.kind === "del"
        ? { stream: "old", row: oldText.length - 1 }
        : { stream: "new", row: newText.length - 1 },
    );
  }
  return { oldText, newText, indexFor: (lineIndex) => positions[lineIndex] };
}

/** hunk 하나의 토큰화 결과 — `hunkTokenStreams`의 두 스트림과 행이 1:1이다. */
export interface HunkTokens {
  old: TokenSpan[][];
  new: TokenSpan[][];
}

/** 줄 번호로 스트림 행을 찾는다. 분할 모드의 셀은 hunk 안의 줄 위치를 갖지 않지만 줄 번호는
 *  갖는다 — 스트림 행은 hunk 시작 줄부터 하나씩 늘므로 번호의 차가 곧 행이다. */
export function tokensForLine(
  tokens: HunkTokens | undefined,
  line: HunkLine,
  hunk: Pick<DiffHunk, "old_range" | "new_range">,
): TokenSpan[] | undefined {
  if (!tokens) return undefined;
  if (line.kind === "del") {
    return line.oldLine === null ? undefined : tokens.old[line.oldLine - hunk.old_range[0]];
  }
  return line.newLine === null ? undefined : tokens.new[line.newLine - hunk.new_range[0]];
}
