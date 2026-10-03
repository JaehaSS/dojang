import { describe, expect, it } from "vitest";
import { exampleSentence, findPhrase, parseExpressions, phraseKey, segmentByExpressions } from "./vocab";

const TEXT = "We can rule out the cache. The outcome is that it carries forward the old state.";

describe("표현 찾기", () => {
  it("단어 경계를 지키고 대소문자를 가리지 않는다", () => {
    expect(findPhrase(TEXT, "out")).toBe(TEXT.indexOf("out the"));
    expect(findPhrase(TEXT, "Rule Out")).toBe(TEXT.indexOf("rule out"));
    expect(findPhrase(TEXT, "come")).toBe(-1);
  });

  it("같은 표현 판정은 공백·대소문자를 무시한다", () => {
    expect(phraseKey("  Rule   OUT ")).toBe("rule out");
  });
});

describe("parseExpressions", () => {
  it("원문에 없는 표현과 중복을 버리고 나머지는 살린다", () => {
    const raw = JSON.stringify([
      { phrase: "Rule out", meaning: "배제하다", note: "가능성을 지울 때" },
      { phrase: "rule out", meaning: "중복" },
      { phrase: "take over", meaning: "원문에 없음" },
      { phrase: "carries forward", meaning: "이어 가다" },
      { phrase: 3, meaning: "형식 오류" },
    ]);
    expect(parseExpressions(raw, TEXT)).toEqual([
      // 표기는 원문을 따른다.
      { phrase: "rule out", meaning: "배제하다", note: "가능성을 지울 때" },
      { phrase: "carries forward", meaning: "이어 가다", note: "" },
    ]);
  });

  it("코드 펜스를 벗기고, 배열이 아니거나 JSON이 아니면 빈 목록이다", () => {
    expect(parseExpressions('```json\n[{"phrase":"rule out","meaning":"배제하다"}]\n```', TEXT)).toHaveLength(1);
    expect(parseExpressions('{"phrase":"rule out"}', TEXT)).toEqual([]);
    expect(parseExpressions("번역: rule out", TEXT)).toEqual([]);
  });

  it("다섯 개까지만 받는다", () => {
    const words = "alpha beta gamma delta epsilon zeta";
    const raw = JSON.stringify(words.split(" ").map((phrase) => ({ phrase, meaning: "뜻" })));
    expect(parseExpressions(raw, words)).toHaveLength(5);
  });
});

describe("예문과 분할", () => {
  it("표현이 든 문장 하나를 예문으로 뽑는다", () => {
    expect(exampleSentence(TEXT, "carries forward")).toBe("The outcome is that it carries forward the old state.");
    expect(exampleSentence("**Rule out** first.\nNext line", "rule out")).toBe("Rule out first.");
    expect(exampleSentence(TEXT, "missing")).toBeNull();
  });

  it("표현마다 처음 나온 자리만 가르고, 이어 붙이면 원문이다", () => {
    const expressions = [
      { phrase: "carries forward", meaning: "", note: "" },
      { phrase: "rule out", meaning: "", note: "" },
    ];
    const segments = segmentByExpressions(TEXT, expressions);
    expect(segments.filter((s) => typeof s !== "string")).toEqual([
      { text: "rule out", index: 1 },
      { text: "carries forward", index: 0 },
    ]);
    expect(segments.map((s) => (typeof s === "string" ? s : s.text)).join("")).toBe(TEXT);
  });
});
