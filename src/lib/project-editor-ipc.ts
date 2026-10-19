import { invoke } from "@tauri-apps/api/core";
import type { FileContent, FsNode, LspGotoKind, LspStatusInfo, LspTarget, SemanticTokens } from "./ipc";

export interface ProjectEditorInfo {
  root: string;
  label: string;
  /** 창의 첫 셸이 에이전트 CLI로 뜬다 — 창이 터미널을 처음부터 연다. */
  launch?: boolean;
}

export interface ProjectShellOpen {
  session: number;
  existed: boolean;
}

export interface ProjectShellSnapshot {
  session: number;
  sequence: number;
  data: string;
  exited: boolean;
  exit_code: number | null;
}

export interface ProjectShellOutput {
  session: number;
  sequence: number;
  data: string;
}

export interface ProjectShellExit {
  session: number;
  code: number;
}

export const projectEditorOpen = (root: string) =>
  invoke<ProjectEditorInfo>("project_editor_open", { root });
export const editorProjectPrepare = (root: string) =>
  invoke<ProjectEditorInfo>("editor_project_prepare", { root });
export const editorProjectActivate = (root: string | null) =>
  invoke<void>("editor_project_activate", { root });

// 공유 창의 요청은 원래 루트를 함께 보낸다. 늦은 요청이 새 프로젝트를 건드리지 않는다.
const rootScope = (root?: string) => root == null ? {} : { root };

export const projectEditorInfo = () => invoke<ProjectEditorInfo>("project_editor_info");
export const projectEditorTree = (root?: string) => invoke<FsNode[]>("project_editor_tree", root == null ? undefined : { root });
export const projectEditorRead = (path: string, root?: string) =>
  invoke<FileContent>("project_editor_read", { path, ...rootScope(root) });
export const projectEditorWrite = (path: string, content: string, expectedContent: string, root?: string) =>
  invoke<number>("project_editor_write", { path, content, expectedContent, ...rootScope(root) });
export const projectEditorResolvePath = (path: string, root?: string) =>
  invoke<string>("project_editor_resolve_path", { path, ...rootScope(root) });
export const projectEditorOpenPath = (path: string, root?: string) =>
  invoke<void>("project_editor_open_path", { path, ...rootScope(root) });
export const projectEditorLspGoto = (
  root: string, path: string, text: string, line: number, column: number, kind: LspGotoKind,
) => invoke<LspTarget[]>("project_editor_lsp_goto", { root, path, text, line, column, kind });
export const projectEditorLspStatus = (root: string, path: string) =>
  invoke<LspStatusInfo>("project_editor_lsp_status", { root, path });
export const projectEditorLspSemanticTokens = (root: string, path: string, text: string) =>
  invoke<SemanticTokens | null>("project_editor_lsp_semantic_tokens", { root, path, text });
export const projectShellOpen = (cols: number, rows: number, root?: string) =>
  invoke<ProjectShellOpen>("project_editor_shell_open", { cols, rows, ...rootScope(root) });
export const projectShellSnapshot = (session: number, root?: string) =>
  invoke<ProjectShellSnapshot>("project_editor_shell_snapshot", { session, ...rootScope(root) });
export const projectShellWrite = (session: number, data: string, root?: string) =>
  invoke<void>("project_editor_shell_write", { session, data, ...rootScope(root) });
export const projectShellResize = (session: number, cols: number, rows: number, root?: string) =>
  invoke<void>("project_editor_shell_resize", { session, cols, rows, ...rootScope(root) });
export const projectShellClose = (session: number, root?: string) =>
  invoke<void>("project_editor_shell_close", { session, ...rootScope(root) });
