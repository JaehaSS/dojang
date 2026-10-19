import assert from "node:assert/strict";
import test from "node:test";
import {
  DESIGN_TOKENS_MODULE_ID,
  buildDesignTokens,
  designTokensPlugin,
} from "../design-tokens.mjs";

test("resolves Tailwind values into CSS-property token candidates", () => {
  const tokens = buildDesignTokens();
  assert.deepEqual(tokens["background-color"], tokens.color);
  assert.ok(tokens["background-color"].some(({ name }) => name === "colors.primary"));
  assert.ok(tokens.color.some(({ name }) => name === "colors.text"));
  assert.ok(tokens["border-radius"].some(({ name }) => name === "rounded.md"));
  assert.ok(tokens["border-radius"].some(({ name }) => name === "rounded.DEFAULT"));
  assert.ok(tokens["font-size"].some(({ name, value }) => name === "typography.fontSize.3xl" && value === "1.875rem"));
  assert.ok(tokens["font-weight"].some(({ name }) => name === "typography.fontWeight.semibold"));
  assert.ok(tokens["padding-top"].some(({ name }) => name === "spacing.4"));
  assert.ok(tokens["margin-left"].some(({ name, value }) => name === "spacing.-4" && value === "-1rem"));
  assert.ok(tokens["background-color"].some(({ value }) => value.includes("var(--c-primary)")));
});

test("serves the resolved token table through a virtual module", () => {
  const plugin = designTokensPlugin();
  const resolvedId = plugin.resolveId(DESIGN_TOKENS_MODULE_ID);
  assert.equal(resolvedId, `\0${DESIGN_TOKENS_MODULE_ID}`);
  const moduleSource = plugin.load(resolvedId);
  assert.equal(typeof moduleSource, "string");
  const payload = JSON.parse(moduleSource.slice("export default ".length, -1));
  assert.ok(Array.isArray(payload["padding-right"]));
  assert.equal(plugin.resolveId("virtual:another-module"), null);
  assert.equal(plugin.load("virtual:another-module"), null);
});
