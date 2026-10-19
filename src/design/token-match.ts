export type DesignTokens = Record<string, Array<{ name: string; value: string }>>;

/** Resolve CSS variables, rem units and colors with the screen's own CSS engine. */
export function createTokenNormalizer(document: Document, context?: Element | null) {
  const probe = document.createElement("span");
  probe.style.cssText = "position:absolute;visibility:hidden;pointer-events:none;contain:layout;";
  const view = document.defaultView;
  if (context && view) {
    const computed = view.getComputedStyle(context);
    for (const key of Array.from(computed)) if (key.startsWith("--")) probe.style.setProperty(key, computed.getPropertyValue(key));
  }
  (document.body ?? document.documentElement)?.appendChild(probe);
  const cache = new Map<string, string>();
  return {
    normalize(property: string, raw: string): string {
      const key = `${property}:${raw}`;
      if (cache.has(key)) return cache.get(key)!;
      probe.style.removeProperty(property);
      probe.style.setProperty(property, raw.replace(/<alpha-value>/g, "1"));
      const normalized = probe.style.getPropertyValue(property) && view ? view.getComputedStyle(probe).getPropertyValue(property).trim() : "";
      cache.set(key, normalized);
      return normalized;
    },
    dispose() { probe.remove(); },
  };
}

export function matchTokens(property: string, value: string, tokens: DesignTokens, normalize: (property: string, raw: string) => string) {
  const names = (tokens[property] ?? []).filter((entry) => {
    const normalized = normalize(property, entry.value);
    return normalized !== "" && normalized === value.trim();
  }).map((entry) => entry.name);
  return { names, outside: !!tokens[property]?.length && names.length === 0 };
}
