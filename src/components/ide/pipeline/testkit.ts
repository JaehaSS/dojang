import { act } from "react";
import { vi } from "vitest";
import type { PipelineDetail, RunRow, TicketRow } from "../../../lib/pipeline";

export const run = (extra: Partial<RunRow> = {}): RunRow => ({
  id: 1, repo: "/repo", base_branch: "main", goal: "목표", state: "executing", paused_from: null, paused_reason: null,
  integration_task_id: null, integration_branch: null, integration_path: null, spec_json: null, spec_revision: 3,
  plan_review_id: null, final_review_id: null, auto_fix_used: 0, plan_fix_used: 0, last_error: null, created_at: 0, updated_at: 0,
  ...extra,
});

export const ticket = (key: string, extra: Partial<TicketRow> = {}): TicketRow => ({
  id: key === "T1" ? 11 : 12, run_id: 1, key, title: `${key} 제목`, body: "", vendor: "codex", reviewer_vendor: "claude",
  acceptance_json: '["npm test"]', allowed_paths_json: '["src/"]', deps_json: "[]", covers_json: "[]", state: "pending",
  task_id: null, attempt: 0, reassigned: 0, last_error: null, updated_at: 0,
  ...extra,
});

export const detail = (r: RunRow, tickets: TicketRow[] = []): PipelineDetail => ({ run: r, tickets, steps: [], reviews: [] });

export const flush = () => act(async () => { await Promise.resolve(); });
export const click = (el: Element | undefined) => act(async () => (el as HTMLElement).click());

export function setValue(el: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement, value: string) {
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : el instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(el, value);
  return act(async () => {
    el.dispatchEvent(new Event(el instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }));
  });
}

export const byText = (root: ParentNode, tag: string, text: string) =>
  [...root.querySelectorAll(tag)].find((e) => e.textContent?.trim() === text);
export const byLabel = (root: ParentNode, label: string) => root.querySelector(`[aria-label="${label}"]`) as HTMLInputElement;

export const confirmYes = () => vi.spyOn(window, "confirm").mockReturnValue(true);
