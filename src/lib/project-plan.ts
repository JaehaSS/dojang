import { invoke } from "@tauri-apps/api/core";

/** Local-only Phase 3 contract.  Remote Runner graphs retain their v1 UI notes. */
export interface ProjectPlan {
  schema_version: 2;
  source_id: string;
  revision: number;
  repo: string;
  objective: string;
  phases: ProjectPhase[];
  task_bindings: TaskPhaseBinding[];
  dependencies: ProjectDependency[];
}

export interface ProjectPhase {
  /** Stable relation id. Renaming a phase must retain this value. */
  id: string;
  name: string;
  objective: string;
}

export interface TaskPhaseBinding { task_id: number; phase_id: string; }
export interface ProjectDependency { from: number; to: number; }

export interface PurposeSnapshot {
  source_id: string;
  task_id: number;
  execution_round_id: string;
  plan_revision: number | null;
  phase_id: string | null;
  project_objective: string;
  phase_objective: string;
  captured_at: number;
}

export type ProjectPlanImport =
  | { kind: "imported"; plan: ProjectPlan }
  | { kind: "already_imported"; plan: ProjectPlan }
  | { kind: "existing_plan_conflict"; plan: ProjectPlan; legacy_hash: string };

/** A DB-backed empty plan is created only by an explicit first save. */
export function emptyProjectPlan(repo: string): ProjectPlan {
  return { schema_version: 2, source_id: "", revision: 0, repo, objective: "", phases: [], task_bindings: [], dependencies: [] };
}

export const projectPlanGet = (canonicalRepo: string) =>
  invoke<ProjectPlan | null>("project_plan_get", { canonicalRepo });

export const projectPlanSave = (canonicalRepo: string, plan: ProjectPlan, expectedRevision: number, expectedSourceId: string) =>
  invoke<ProjectPlan>("project_plan_save", { canonicalRepo, plan, expectedRevision, expectedSourceId });

/** `raw` is intentionally never removed from localStorage, including after success. */
export const projectPlanImport = (canonicalRepo: string, raw: string) =>
  invoke<ProjectPlanImport>("project_plan_import", { canonicalRepo, raw });

export const purposeGet = (taskId: number, executionRoundId?: string) =>
  invoke<PurposeSnapshot | null>("purpose_get", { taskId, ...(executionRoundId ? { executionRoundId } : {}) });

/** Requests a later admission only. This call does not send a task message or start work. */
export const purposeRequestRefresh = (taskId: number, canonicalRepo: string, expectedPlanRevision: number, expectedSourceId: string, phaseId: string | null) =>
  invoke<ProjectPlan>("purpose_request_refresh", { taskId, canonicalRepo, expectedPlanRevision, expectedSourceId, phaseId });

export function phaseByTask(plan: ProjectPlan): Record<string, string> {
  return Object.fromEntries(plan.task_bindings.map((binding) => [String(binding.task_id), binding.phase_id]));
}

export function phaseForTask(plan: ProjectPlan, taskId: number): ProjectPhase | null {
  const id = plan.task_bindings.find((binding) => binding.task_id === taskId)?.phase_id;
  return id ? plan.phases.find((phase) => phase.id === id) ?? null : null;
}

export function planTextBytes(value: string): number { return new TextEncoder().encode(value).byteLength; }
export const MAX_OBJECTIVE_BYTES = 2_000;
