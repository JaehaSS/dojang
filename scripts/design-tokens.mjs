import resolveConfig from "tailwindcss/resolveConfig.js";
import tailwindConfig from "../tailwind.config.js";

export const DESIGN_TOKENS_MODULE_ID = "virtual:dojang-design-tokens";
const RESOLVED_DESIGN_TOKENS_MODULE_ID = `\0${DESIGN_TOKENS_MODULE_ID}`;

function token(name, value) {
  return { name, value: String(value) };
}

function flattenTokens(value, prefix, output) {
  if (typeof value === "string" || typeof value === "number") {
    output.push(token(prefix, value));
    return;
  }
  if (!value || typeof value !== "object") return;

  for (const [key, child] of Object.entries(value)) {
    const name = key === "DEFAULT" ? prefix : `${prefix}.${key}`;
    flattenTokens(child, name, output);
  }
}

function flattenTheme(theme, prefix) {
  const output = [];
  flattenTokens(theme, prefix, output);
  return output;
}

function spacingTokens(spacing, includeNegative = false) {
  const values = flattenTheme(spacing, "spacing");
  if (!includeNegative) return values;

  return values.concat(values
    .filter(({ value }) => value !== "0" && value !== "0px")
    .map(({ name, value }) => token(name.replace("spacing.", "spacing.-"), `-${value}`)));
}

function fontSizeTokens(fontSize) {
  return Object.entries(fontSize)
    .filter(([, value]) => typeof value === "string" || typeof value === "number" || Array.isArray(value))
    .map(([name, value]) => token(
      `typography.fontSize.${name}`,
      Array.isArray(value) ? value[0] : value,
    ));
}

function borderRadiusTokens(borderRadius) {
  return Object.entries(borderRadius)
    .filter(([, value]) => typeof value === "string" || typeof value === "number")
    .map(([name, value]) => token(`rounded.${name}`, value));
}

/** Returns the resolved Tailwind tokens grouped by the CSS properties they can match. */
export function buildDesignTokens(config = tailwindConfig) {
  const theme = resolveConfig(config).theme;
  const colors = flattenTheme(theme.colors, "colors");
  const spacing = spacingTokens(theme.spacing);
  const margin = spacingTokens(theme.spacing, true);

  return {
    "background-color": colors,
    color: colors,
    "border-radius": borderRadiusTokens(theme.borderRadius),
    "font-size": fontSizeTokens(theme.fontSize),
    "font-weight": flattenTheme(theme.fontWeight, "typography.fontWeight"),
    "padding-top": spacing,
    "padding-right": spacing,
    "padding-bottom": spacing,
    "padding-left": spacing,
    "margin-top": margin,
    "margin-right": margin,
    "margin-bottom": margin,
    "margin-left": margin,
  };
}

/** Provides resolved tokens to the Design canvas without a production module. */
export function designTokensPlugin() {
  return {
    name: "dojang-design-tokens",
    resolveId(id) {
      return id === DESIGN_TOKENS_MODULE_ID ? RESOLVED_DESIGN_TOKENS_MODULE_ID : null;
    },
    load(id) {
      if (id !== RESOLVED_DESIGN_TOKENS_MODULE_ID) return null;
      return `export default ${JSON.stringify(buildDesignTokens())};`;
    },
  };
}
