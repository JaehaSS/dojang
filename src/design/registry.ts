import type { ComponentType } from "react";
import type { ScreenMeta } from "./types";

export type ScreenEntry = {
  id: string;
  file: string;
} & (
  | { error: string; meta?: never; Screen?: never }
  | { error: null; meta: ScreenMeta; Screen: ComponentType<Record<string, unknown>> }
);

const record = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === "object" && !Array.isArray(value);

export function validateMeta(value: unknown): string | null {
  if (!record(value)) return "meta가 없거나 객체가 아닙니다.";
  if (typeof value.title !== "string" || !value.title.trim()) return "meta.title에 화면 이름이 필요합니다.";
  const viewport = value.viewport;
  if (!record(viewport) || ![viewport.width, viewport.height].every((v) => typeof v === "number" && Number.isSafeInteger(v) && v > 0)) {
    return "meta.viewport의 width와 height는 양의 정수여야 합니다.";
  }
  if (!record(value.states) || Object.keys(value.states).length === 0) return "meta.states에 하나 이상의 상태가 필요합니다.";
  if (Object.entries(value.states).some(([name, props]) => !name.trim() || !record(props))) return "상태 이름과 props 객체를 확인하세요.";
  return null;
}

export function collectScreens(modules: Record<string, unknown>): ScreenEntry[] {
  return Object.entries(modules).sort(([a], [b]) => a.localeCompare(b)).map(([file, module]) => {
    const id = file.replace(/^\.\/screens\//, "").replace(/\.screen\.tsx$/, "");
    const error = validateMeta(record(module) ? module.meta : undefined);
    if (error) return { id, file, error };
    if (!record(module) || typeof module.default !== "function") return { id, file, error: "default 화면 컴포넌트가 필요합니다." };
    return { id, file, error: null, meta: module.meta as ScreenMeta, Screen: module.default as ComponentType<Record<string, unknown>> };
  });
}
