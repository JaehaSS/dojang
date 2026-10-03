import { describe, expect, it } from "vitest";
import { hasHangul, isMostlyKorean, parseBlockTranslations, splitReplyBlocks } from "./translate";

describe("translate 언어 판정", () => {
  it("한글이 한 글자라도 있으면 다듬을 거리가 있다", () => {
    expect(hasHangul("@src/App.tsx 고쳐줘")).toBe(true);
    expect(hasHangul("fix @src/App.tsx")).toBe(false);
  });

  it("코드·경로가 섞인 한국어 답변은 한국어로 본다", () => {
    expect(isMostlyKorean("`src/components/ide/AgentComposer.tsx`의 onKeyDown을 고쳤습니다.")).toBe(true);
    expect(isMostlyKorean("I updated the 컴포넌트 so that it handles the IME composition state.")).toBe(false);
    expect(isMostlyKorean("Done.")).toBe(false);
    expect(isMostlyKorean("src/lib/translate.ts 와 App.tsx 를 고쳤습니다. 테스트도 통과합니다.")).toBe(true);
    expect(isMostlyKorean("Fixed it. See https://example.com/docs for details.")).toBe(false);
  });

  it("공백 없는 긴 토큰이 있어도 판정이 느려지지 않는다", () => {
    const blob = "a".repeat(20_000);
    const started = performance.now();
    expect(isMostlyKorean(`빌드 결과 해시입니다. ${blob}`)).toBe(true);
    expect(performance.now() - started).toBeLessThan(50);
  });
});

describe("답변 문단 대역 분할", () => {
  it("조각을 이어 붙이면 원문과 같고, 코드 블록은 번역 대상이 아니다", () => {
    const text = "Intro line.\n\n```sh\nnpm test\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n1. one\n2. two\n";
    const blocks = splitReplyBlocks(text);
    expect(blocks.map((b) => b.source).join("")).toBe(text);
    expect(blocks.map((b) => [b.source.trim().slice(0, 5), b.translate])).toEqual([
      ["Intro", true],
      ["```sh", false],
      ["| a |", true],
      ["1. on", true],
    ]);
  });

  it("번역하지 않는 블록이 이어지면 한 조각으로 합친다", () => {
    const blocks = splitReplyBlocks("```a\nx\n```\n\n---\n\n```b\ny\n```\n\nDone.");
    expect(blocks.map((b) => b.translate)).toEqual([false, true]);
  });

  it("모델 응답은 요청한 개수의 문자열 배열일 때만 받는다", () => {
    expect(parseBlockTranslations('["가", "나"]', 2)).toEqual(["가", "나"]);
    expect(parseBlockTranslations('```json\n["가"]\n```', 1)).toEqual(["가"]);
    expect(parseBlockTranslations('["가"]', 2)).toBeNull();
    expect(parseBlockTranslations('["가", 1]', 2)).toBeNull();
    expect(parseBlockTranslations("가\n\n나", 2)).toBeNull();
  });
});
