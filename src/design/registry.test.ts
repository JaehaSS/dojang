import { describe, expect, it } from "vitest";
import { collectScreens, validateMeta } from "./registry";

const valid = { title: "표본", viewport: { width: 960, height: 640 }, states: { 정상: { count: 3 }, "빈 상태": { count: 0 } } };
describe("screen registry", () => {
  it("preserves named sample states and viewport, sorted by source file", () => {
    const Screen = () => null;
    const entries = collectScreens({ "./screens/z.screen.tsx": { meta: valid, default: Screen }, "./screens/a.screen.tsx": { meta: valid, default: Screen } });
    expect(entries.map((entry) => entry.id)).toEqual(["a", "z"]);
    expect(entries[0].meta).toEqual(valid);
    expect(entries[0].Screen).toBe(Screen);
  });
  it.each([undefined, null, [], {}, { ...valid, title: " " }, { ...valid, viewport: { width: 0, height: 640 } }, { ...valid, viewport: { width: Infinity, height: 640 } }, { ...valid, viewport: { width: 1.5, height: 640 } }, { ...valid, states: {} }, { ...valid, states: { 정상: null } }, { ...valid, states: { 정상: [] } }])("rejects invalid metadata: %j", (meta) => {
    expect(validateMeta(meta)).not.toBeNull();
    const entry = collectScreens({ "./screens/bad.screen.tsx": { meta, default: () => null } })[0];
    expect(entry.error).toBeTruthy();
    expect(entry.Screen).toBeUndefined();
  });
  it("lists a missing default export as an error without a render target", () => {
    expect(collectScreens({ "./screens/bad.screen.tsx": { meta: valid } })[0].error).toContain("default");
  });
});
