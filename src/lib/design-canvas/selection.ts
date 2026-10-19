export type DesignClassKind = "static" | "dynamic" | "none" | "unknown";

export interface DesignAncestor {
  source: string;
  className: string;
  classKind: DesignClassKind;
  tagName: string;
}

export interface DesignSelection {
  version: 1;
  requestId?: string;
  source: string | null;
  instanceSource?: string | null;
  className: string;
  classKind: DesignClassKind;
  tagName: string;
  computedCss: Record<string, string>;
  /** CSS pixels in the screen iframe, independent of canvas zoom. */
  boundingRect: { x: number; y: number; width: number; height: number };
  suggestedAncestor: DesignAncestor | null;
}

export interface DesignSelectionEvent { taskId: number; selection: DesignSelection }

const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === "object" && !Array.isArray(value);
const kind = (value: unknown): value is DesignClassKind => typeof value === "string" && ["static", "dynamic", "none", "unknown"].includes(value);
const text = (value: unknown, max: number): value is string => typeof value === "string" && value.length <= max;

export function isDesignSelection(value: unknown): value is DesignSelection {
  if (!object(value) || value.version !== 1 || !kind(value.classKind) || !text(value.className, 8192) || !text(value.tagName, 128)) return false;
  if (value.requestId !== undefined && !text(value.requestId, 128)) return false;
  if (value.source !== null && !text(value.source, 4096)) return false;
  if (value.instanceSource != null && !text(value.instanceSource, 4096)) return false;
  const bounds = value.boundingRect;
  if (!object(bounds) || !["x", "y", "width", "height"].every((key) => typeof bounds[key] === "number" && Number.isFinite(bounds[key]))) return false;
  if ((bounds.width as number) < 0 || (bounds.height as number) < 0) return false;
  if (!object(value.computedCss) || Object.keys(value.computedCss).length > 64 || !Object.values(value.computedCss).every((entry) => text(entry, 2048))) return false;
  const ancestor = value.suggestedAncestor;
  return ancestor === null || (object(ancestor) && text(ancestor.source, 4096) && text(ancestor.tagName, 128) && text(ancestor.className, 8192) && kind(ancestor.classKind));
}
