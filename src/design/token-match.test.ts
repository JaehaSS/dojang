// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { createTokenNormalizer, matchTokens } from "./token-match";

describe("resolved CSS token matching", () => {
  it("matches resolved rem values, multiple names, and warns only for known token properties", () => {
    const tokens = { "padding-top": [{ name: "spacing.4", value: "1rem" }, { name: "spacing.custom", value: "16px" }] };
    const normalize = (_property: string, value: string) => value === "1rem" ? "16px" : value;
    expect(matchTokens("padding-top", "16px", tokens, normalize)).toEqual({ names: ["spacing.4", "spacing.custom"], outside: false });
    expect(matchTokens("padding-top", "13px", tokens, normalize)).toEqual({ names: [], outside: true });
    expect(matchTokens("font-family", "Inter", tokens, normalize)).toEqual({ names: [], outside: false });
  });
  it("normalizes with a temporary probe and removes it after use", () => {
    const before = document.body.childElementCount;
    const normalizer = createTokenNormalizer(document);
    expect(normalizer.normalize("padding-top", "13px")).toBe("13px");
    expect(normalizer.normalize("background-color", "#ff0000")).toBe("rgb(255, 0, 0)");
    normalizer.dispose();
    expect(document.body.childElementCount).toBe(before);
  });
});
