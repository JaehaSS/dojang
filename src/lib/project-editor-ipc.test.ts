import { describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const ipc = await import("./project-editor-ipc");

describe("project editor IPC", () => {
  it("uses the frozen root-bound command names and camelCase write payload", () => {
    ipc.projectEditorOpen("/repo");
    ipc.projectEditorWrite("src/a.ts", "next", "before");
    ipc.projectEditorOpenPath("src/a.ts");
    ipc.projectShellResize(4, 100, 30);

    expect(invoke).toHaveBeenNthCalledWith(1, "project_editor_open", { root: "/repo" });
    expect(invoke).toHaveBeenNthCalledWith(2, "project_editor_write", { path: "src/a.ts", content: "next", expectedContent: "before" });
    expect(invoke).toHaveBeenNthCalledWith(3, "project_editor_open_path", { path: "src/a.ts" });
    expect(invoke).toHaveBeenNthCalledWith(4, "project_editor_shell_resize", { session: 4, cols: 100, rows: 30 });
  });

  it("prepares the singleton editor and scopes its project requests to the selected root", () => {
    invoke.mockClear();
    ipc.editorProjectPrepare("/repo");
    ipc.editorProjectActivate("/repo");
    ipc.projectEditorWrite("src/a.ts", "next", "before", "/repo");
    ipc.projectShellOpen(100, 30, "/repo");
    ipc.editorProjectActivate(null);
    expect(invoke.mock.calls).toEqual([
      ["editor_project_prepare", { root: "/repo" }],
      ["editor_project_activate", { root: "/repo" }],
      ["project_editor_write", { path: "src/a.ts", content: "next", expectedContent: "before", root: "/repo" }],
      ["project_editor_shell_open", { cols: 100, rows: 30, root: "/repo" }],
      ["editor_project_activate", { root: null }],
    ]);
  });
});
