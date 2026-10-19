import { describe, expect, it } from "vitest";
import { hunkLines } from "./annotations";
import {
  hunkTokenStreams,
  mergeLinePieces,
  tokenColor,
  tokensForLine,
  type TokenSpan,
} from "./diff-highlight";
import type { DiffHunk } from "./ipc";
import { syntaxTokenRules } from "./monaco-theme-rules";
import { getTheme } from "./themes";

const join = (pieces: { text: string }[]) => pieces.map((piece) => piece.text).join("");
const colors: Record<string, string> = { keyword: "#k", identifier: "#i" };
const resolve = (type: string) => (colors[type] ? { color: colors[type] } : undefined);

describe("mergeLinePieces", () => {
  it("토큰도 구간도 없으면 줄 전체가 한 조각이다", () => {
    expect(mergeLinePieces("const a", undefined, undefined, resolve)).toEqual([
      { text: "const a", changed: false },
    ]);
  });

  it("빈 줄은 조각이 없고 이어 붙이면 빈 문자열이다", () => {
    const pieces = mergeLinePieces("", [], [{ text: "", changed: true }], resolve);
    expect(pieces).toEqual([]);
    expect(join(pieces)).toBe("");
  });

  it("구간만 있으면 단어 강조만 남긴다", () => {
    const pieces = mergeLinePieces(
      "const b = 2;",
      undefined,
      [
        { text: "const b = ", changed: false },
        { text: "2", changed: true },
        { text: ";", changed: false },
      ],
      resolve,
    );
    expect(pieces).toEqual([
      { text: "const b = ", changed: false },
      { text: "2", changed: true },
      { text: ";", changed: false },
    ]);
  });

  it("토큰 경계와 구간 경계가 엇갈리면 양쪽을 모두 자른다", () => {
    const text = "const value";
    const tokens: TokenSpan[] = [
      { start: 0, end: 5, type: "keyword" },
      { start: 5, end: 6, type: "" },
      { start: 6, end: 11, type: "identifier" },
    ];
    // 바뀐 구간 "st val"이 키워드와 식별자에 걸친다.
    const pieces = mergeLinePieces(
      text,
      tokens,
      [
        { text: "con", changed: false },
        { text: "st val", changed: true },
        { text: "ue", changed: false },
      ],
      resolve,
    );
    expect(join(pieces)).toBe(text);
    expect(pieces).toEqual([
      { text: "con", changed: false, color: "#k" },
      { text: "st", changed: true, color: "#k" },
      { text: " ", changed: true },
      { text: "val", changed: true, color: "#i" },
      { text: "ue", changed: false, color: "#i" },
    ]);
  });

  it("토큰이 줄 일부만 덮어도 나머지를 잃지 않는다", () => {
    const text = "let x = 1";
    const pieces = mergeLinePieces(text, [{ start: 0, end: 3, type: "keyword" }], undefined, resolve);
    expect(join(pieces)).toBe(text);
    expect(pieces).toEqual([
      { text: "let", changed: false, color: "#k" },
      { text: " x = 1", changed: false },
    ]);
  });

  it("줄 길이를 넘는 토큰 offset은 잘라 낸다", () => {
    const pieces = mergeLinePieces("ab", [{ start: 0, end: 9, type: "keyword" }], undefined, resolve);
    expect(pieces).toEqual([{ text: "ab", changed: false, color: "#k" }]);
  });
});

describe("tokenColor", () => {
  it("팔레트 테마는 언어 접미사를 떼고 가장 긴 규칙을 고른다", () => {
    const theme = getTheme("catppuccin-mocha");
    expect(theme.syntax).toBeDefined();
    expect(tokenColor("keyword.ts", theme)).toBe(theme.syntax!.keyword);
    expect(tokenColor("string.escape.rust", theme)).toBe(theme.syntax!.string);
    // `type.identifier`가 `type`보다 길다 — 둘 다 같은 색이지만 접두 매칭 경로를 확인한다.
    expect(tokenColor("type.identifier.ts", theme)).toBe(theme.syntax!.type);
    expect(tokenColor("", theme)).toBeUndefined();
    expect(tokenColor("source.ts", theme)).toBeUndefined();
  });

  it("팔레트 없는 테마는 Monaco 기본색에 준하는 표로 메운다", () => {
    const dark = getTheme("praxis-dark");
    expect(dark.syntax).toBeUndefined();
    expect(tokenColor("keyword.ts", dark)).toBe("#569CD6");
    expect(tokenColor("comment.ts", dark)).toBe("#6A9955");
    // 테마 규칙(function.python)이 대체 표의 function보다 길어서 이긴다.
    expect(tokenColor("function.python", dark)).toBe("#DCDCAA");
    // 팔레트 없는 테마의 identifier는 본문색 상속 — 색을 붙이지 않는다.
    expect(tokenColor("identifier.ts", dark)).toBeUndefined();

    const light = getTheme("praxis-light");
    expect(light.kind).toBe("light");
    expect(tokenColor("string.ts", light)).toBe("#A31515");
  });
});

describe("syntaxTokenRules", () => {
  it("팔레트 없는 테마는 Python 함수 규칙과 semantic 규칙만 낸다", () => {
    const rules = syntaxTokenRules(getTheme("praxis-dark"));
    expect(rules[0]).toEqual({ token: "function.python", foreground: "DCDCAA" });
    expect(rules.map((rule) => rule.token)).not.toContain("keyword");
  });
});

describe("hunkTokenStreams", () => {
  const hunk: DiffHunk = {
    id: "h1",
    path: "a.ts",
    old_range: [10, 4],
    new_range: [10, 4],
    protected: false,
    committed: false,
    risk: "low",
    lines: [
      { kind: "context", text: "c1" },
      { kind: "del", text: "d1" },
      { kind: "del", text: "d2" },
      { kind: "add", text: "a1" },
      { kind: "add", text: "a2" },
      { kind: "context", text: "c2" },
    ],
  };
  const lines = hunkLines(hunk);

  it("old = context+del, new = context+add 순서를 지킨다", () => {
    const streams = hunkTokenStreams(lines);
    expect(streams.oldText).toEqual(["c1", "d1", "d2", "c2"]);
    expect(streams.newText).toEqual(["c1", "a1", "a2", "c2"]);
  });

  it("context는 new, del은 old 스트림의 행을 가리킨다", () => {
    const { indexFor } = hunkTokenStreams(lines);
    expect(lines.map((_, i) => indexFor(i))).toEqual([
      { stream: "new", row: 0 },
      { stream: "old", row: 1 },
      { stream: "old", row: 2 },
      { stream: "new", row: 1 },
      { stream: "new", row: 2 },
      { stream: "new", row: 3 },
    ]);
  });

  it("줄 번호로 찾은 토큰이 스트림 위치와 일치한다", () => {
    const streams = hunkTokenStreams(lines);
    const span = (label: string): TokenSpan[] => [{ start: 0, end: 2, type: label }];
    const tokens = {
      old: streams.oldText.map((text) => span(`old:${text}`)),
      new: streams.newText.map((text) => span(`new:${text}`)),
    };
    expect(lines.map((line) => tokensForLine(tokens, line, hunk)?.[0].type)).toEqual([
      "new:c1",
      "old:d1",
      "old:d2",
      "new:a1",
      "new:a2",
      "new:c2",
    ]);
    expect(tokensForLine(undefined, lines[0], hunk)).toBeUndefined();
  });
});
