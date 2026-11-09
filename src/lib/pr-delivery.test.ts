// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
import { taskPrExecute, taskPrPreview } from "./pr-delivery";

describe("PR delivery IPC", () => {
  it("sends an editable preview request and an explicit action", async () => {
    invoke.mockResolvedValue({});
    await taskPrPreview(12, { title: "제목", body: "본문" });
    await taskPrExecute(12, "prepared", "create");
    expect(invoke).toHaveBeenNthCalledWith(1, "task_pr_preview", { id: 12, input: { title: "제목", body: "본문" } });
    expect(invoke).toHaveBeenNthCalledWith(2, "task_pr_execute", { id: 12, token: "prepared", action: { action: "create" } });
  });
});
