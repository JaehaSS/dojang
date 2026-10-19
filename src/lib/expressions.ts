import type { Element, ElementContent, Root, RootContent } from "hast";
import { translateText } from "./translate";

/**
 * 영어 표현 공부 — ⌘J 결과와 답변 번역에서 표현 3~5개를 짚어 밑줄을 긋고, 올리면 뜻을 보인다.
 * 표현은 백엔드 `Direction::EnExpressions`가 고른다.
 */

export interface Expression {
  phrase: string;
  meaning: string;
  note: string;
}

const MAX_EXPRESSIONS = 5;

/** 같은 표현인가 — 공백과 대소문자를 가리지 않는다. */
export const phraseKey = (phrase: string) => phrase.trim().split(/\s+/).join(" ").toLowerCase();

const isWordChar = (ch: string | undefined) => !!ch && /[\p{L}\p{N}_]/u.test(ch);

/** 단어 경계를 지키며 대소문자 없이 찾는다. `out`이 `outcome` 안에서 걸리면 안 된다. */
export function findPhrase(text: string, phrase: string, from = 0): number {
  const hay = text.toLowerCase();
  const needle = phrase.toLowerCase();
  if (!needle) return -1;
  for (let at = hay.indexOf(needle, from); at !== -1; at = hay.indexOf(needle, at + 1)) {
    const before = text[at - 1];
    const after = text[at + needle.length];
    if ((!isWordChar(needle[0]) || !isWordChar(before)) && (!isWordChar(needle[needle.length - 1]) || !isWordChar(after))) {
      return at;
    }
  }
  return -1;
}

/**
 * 모델 응답을 검증한다. 원문에 없는 표현은 밑줄을 그을 자리가 없으므로 버린다 —
 * 번역과 달리 일부가 틀려도 나머지는 쓸 수 있으니 전체를 버리지 않는다.
 */
export function parseExpressions(raw: string, text: string): Expression[] {
  const body = raw.trim().replace(/^```(?:json)?\s*\n([\s\S]*?)\n?```$/, "$1");
  let parsed: unknown;
  try {
    parsed = JSON.parse(body);
  } catch {
    return [];
  }
  if (!Array.isArray(parsed)) return [];
  const seen = new Set<string>();
  const out: Expression[] = [];
  for (const item of parsed) {
    if (!item || typeof item !== "object") continue;
    const { phrase, meaning, note } = item as Record<string, unknown>;
    if (typeof phrase !== "string" || typeof meaning !== "string") continue;
    const at = findPhrase(text, phrase.trim());
    if (!phrase.trim() || !meaning.trim() || at === -1) continue;
    // 원문의 표기를 쓴다 — 모델이 대소문자를 바꿔 와도 화면의 글자와 같아야 한다.
    const exact = text.slice(at, at + phrase.trim().length);
    const key = phraseKey(exact);
    if (seen.has(key)) continue;
    seen.add(key);
    out.push({ phrase: exact, meaning: meaning.trim(), note: typeof note === "string" ? note.trim() : "" });
    if (out.length === MAX_EXPRESSIONS) break;
  }
  return out;
}

/** 평문을 표현 자리로 가른다. 표현마다 처음 나온 곳 하나만 — 같은 밑줄이 여러 번이면 시끄럽다. */
export function segmentByExpressions(text: string, expressions: Expression[]): (string | { text: string; index: number })[] {
  const hits: { at: number; len: number; index: number }[] = [];
  expressions.forEach((expr, index) => {
    const at = findPhrase(text, expr.phrase);
    if (at === -1) return;
    if (hits.some((h) => at < h.at + h.len && h.at < at + expr.phrase.length)) return;
    hits.push({ at, len: expr.phrase.length, index });
  });
  hits.sort((a, b) => a.at - b.at);
  const out: (string | { text: string; index: number })[] = [];
  let cursor = 0;
  for (const hit of hits) {
    if (hit.at > cursor) out.push(text.slice(cursor, hit.at));
    out.push({ text: text.slice(hit.at, hit.at + hit.len), index: hit.index });
    cursor = hit.at + hit.len;
  }
  if (cursor < text.length) out.push(text.slice(cursor));
  return out;
}

/** 이 요소 안의 글자는 밑줄을 긋지 않는다 — 코드는 공부할 영어가 아니고, 링크는 이미 밑줄이 있다. */
const SKIP_TAGS = new Set(["code", "pre", "a", "mark"]);

/**
 * rehype 플러그인 — 렌더링된 마크다운의 글자 노드에서 표현을 찾아 `<mark data-expression=i>`로 감싼다.
 * 서식 경계를 넘는 표현(`**carry** forward`)은 한 글자 노드에 없으므로 긋지 못한다. 그런 표현은
 * 답변 아래 표현 목록에서 본다.
 */
export function rehypeExpressions(phrases: string[]) {
  return () => (tree: Root) => {
    const done = new Set<number>();
    const walk = (parent: Root | Element) => {
      const next: (RootContent | ElementContent)[] = [];
      for (const child of parent.children) {
        if (child.type === "element") {
          if (!SKIP_TAGS.has(child.tagName)) walk(child);
          next.push(child);
          continue;
        }
        if (child.type !== "text") {
          next.push(child);
          continue;
        }
        const pending = phrases.map((phrase, index) => ({ phrase, index })).filter(({ index }) => !done.has(index));
        const segments = segmentByExpressions(
          child.value,
          pending.map(({ phrase }) => ({ phrase, meaning: "", note: "" })),
        );
        for (const segment of segments) {
          if (typeof segment === "string") {
            next.push({ type: "text", value: segment });
            continue;
          }
          const index = pending[segment.index].index;
          done.add(index);
          next.push({
            type: "element",
            tagName: "mark",
            properties: { dataExpression: String(index) },
            children: [{ type: "text", value: segment.text }],
          });
        }
      }
      parent.children = next as typeof parent.children;
    };
    walk(tree);
  };
}

// ── 표현 추출 — 같은 글이면 다시 부르지 않는다 ─────────────────────────────────
// 메모리에만 둔다. 답변 번역 캐시(`replyCache`)와 같은 이유다.
const expressionCache = new Map<string, Promise<Expression[]>>();

export function extractExpressions(text: string): Promise<Expression[]> {
  const hit = expressionCache.get(text);
  if (hit) return hit;
  const pending = translateText("en_expressions", text).then((raw) => parseExpressions(raw, text));
  expressionCache.set(text, pending);
  pending.catch(() => expressionCache.delete(text));
  return pending;
}

/** 테스트 전용. */
export function resetExpressionStateForTest(): void {
  expressionCache.clear();
}
