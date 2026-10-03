import { beforeEach, describe, expect, it, vi } from "vitest";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import {
  emptyProjectPlan, MAX_OBJECTIVE_BYTES, phaseForTask, planTextBytes, projectPlanImport,
  projectPlanSave, purposeRequestRefresh, type ProjectPlan,
} from "./project-plan";

const plan: ProjectPlan = {
  schema_version: 2, source_id: "db-a", revision: 3, repo: "/repo", objective: "why",
  phases: [{ id: "build", name: "Build", objective: "make it work" }],
  task_bindings: [{ task_id: 4, phase_id: "build" }], dependencies: [{ from: 1, to: 4 }],
};

beforeEach(() => invoke.mockReset());

describe("local project-plan IPC", () => {
  it("uses canonical repository and expected revision for a full-plan CAS save", async () => {
    invoke.mockResolvedValue(plan);
    await projectPlanSave("/repo", plan, 3, "db-a");
    expect(invoke).toHaveBeenCalledWith("project_plan_save", { canonicalRepo: "/repo", plan, expectedRevision: 3, expectedSourceId: "db-a" });
  });

  it("keeps raw v1 import data separate from a later refresh request", async () => {
    invoke.mockResolvedValue({ kind: "imported", plan });
    await projectPlanImport("/repo", '{"version":1}');
    expect(invoke).toHaveBeenLastCalledWith("project_plan_import", { canonicalRepo: "/repo", raw: '{"version":1}' });
    await purposeRequestRefresh(4, "/repo", 3, "db-a", "build");
    expect(invoke).toHaveBeenLastCalledWith("purpose_request_refresh", { taskId: 4, canonicalRepo: "/repo", expectedPlanRevision: 3, expectedSourceId: "db-a", phaseId: "build" });
  });

  it("keeps stable phase ids separate from labels and counts UTF-8 objective bytes", () => {
    expect(phaseForTask(plan, 4)).toEqual(plan.phases[0]);
    expect(phaseForTask(plan, 99)).toBeNull();
    expect(emptyProjectPlan("/repo").revision).toBe(0);
    expect(planTextBytes("가")).toBe(3);
    expect(planTextBytes("x".repeat(MAX_OBJECTIVE_BYTES + 1))).toBeGreaterThan(MAX_OBJECTIVE_BYTES);
  });
});
