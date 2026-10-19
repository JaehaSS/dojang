import { describe, expect, it } from "vitest";
import { isDesignSelection } from "./selection";

const valid = { version: 1, source: "src/design/screens/a.screen.tsx:1:0", className: "p-4", classKind: "static", tagName: "div", computedCss: { color: "rgb(1, 2, 3)" }, boundingRect: { x: 2, y: -4, width: 50, height: 25 }, suggestedAncestor: null };
describe("selection event validation", () => {
  it("accepts tagged and untagged readonly snapshots", () => {
    expect(isDesignSelection(valid)).toBe(true);
    expect(isDesignSelection({ ...valid, source: null, classKind: "unknown", suggestedAncestor: { source: valid.source, tagName: "section", className: "", classKind: "none" } })).toBe(true);
  });
  it.each([null, { ...valid, version: 2 }, { ...valid, classKind: "made-up" }, { ...valid, classKind: { toString: (): string => "static" } }, { ...valid, computedCss: [] }, { ...valid, boundingRect: { ...valid.boundingRect, width: -1 } }, { ...valid, boundingRect: { ...valid.boundingRect, x: Infinity } }, { ...valid, suggestedAncestor: {} }, { ...valid, requestId: 3 }])("rejects malformed snapshots: %j", (payload) => {
    expect(isDesignSelection(payload)).toBe(false);
  });
});
