use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use tauri::{Runtime, State, WebviewWindow};

use super::ProjectEditorState;
use crate::commands::AppState;
use crate::lspclient::{GotoKind, LspStatus, LspTarget, SemanticTokens};

/// 작업 ID는 양수다. 프로젝트 창마다 음수 영역의 별도 버퍼 소유권을 부여한다.
/// 실제 루트는 LspPool 키에 별도로 포함되어 프로젝트 전환도 격리된다.
pub fn project_lsp_scope(label: &str) -> i64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    label.hash(&mut hasher);
    -1 - (hasher.finish() & 0x3fff_ffff_ffff_ffff) as i64
}

fn validated_root(
    projects: &ProjectEditorState,
    label: &str,
    expected: &str,
    path: &str,
) -> Result<PathBuf, String> {
    let root = projects.root_for_request(label, Some(expected))?;
    crate::fsapi::safe_join(&root, path).map_err(|e| e.to_string())?;
    Ok(root)
}

#[tauri::command]
pub async fn project_editor_lsp_goto<R: Runtime>(
    state: State<'_, AppState>,
    projects: State<'_, ProjectEditorState>,
    window: WebviewWindow<R>,
    root: String,
    path: String,
    text: String,
    line: u32,
    column: u32,
    kind: String,
) -> Result<Vec<LspTarget>, String> {
    let root = validated_root(&projects, window.label(), &root, &path)?;
    let result = state
        .lsp
        .goto(
            project_lsp_scope(window.label()),
            &root,
            &path,
            &text,
            line,
            column,
            GotoKind::parse(&kind)?,
        )
        .await?;
    if !projects.root_is_current(window.label(), &root) {
        return Err("에디터의 프로젝트 루트가 변경되었습니다".into());
    }
    Ok(result)
}

#[tauri::command]
pub async fn project_editor_lsp_semantic_tokens<R: Runtime>(
    state: State<'_, AppState>,
    projects: State<'_, ProjectEditorState>,
    window: WebviewWindow<R>,
    root: String,
    path: String,
    text: String,
) -> Result<Option<SemanticTokens>, String> {
    let root = validated_root(&projects, window.label(), &root, &path)?;
    let result = state
        .lsp
        .semantic_tokens(project_lsp_scope(window.label()), &root, &path, &text)
        .await?;
    if !projects.root_is_current(window.label(), &root) {
        return Err("에디터의 프로젝트 루트가 변경되었습니다".into());
    }
    Ok(result)
}

#[tauri::command]
pub async fn project_editor_lsp_status<R: Runtime>(
    state: State<'_, AppState>,
    projects: State<'_, ProjectEditorState>,
    window: WebviewWindow<R>,
    root: String,
    path: String,
) -> Result<LspStatus, String> {
    let root = validated_root(&projects, window.label(), &root, &path)?;
    Ok(state
        .lsp
        .status_for(project_lsp_scope(window.label()), &root, &path)
        .await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_windows_use_separate_negative_buffer_scopes() {
        assert!(project_lsp_scope("editor") < 0);
        assert_eq!(project_lsp_scope("editor"), project_lsp_scope("editor"));
        assert_ne!(
            project_lsp_scope("editor"),
            project_lsp_scope("project-editor-1")
        );
    }

    #[test]
    fn lsp_requests_reject_unbound_stale_and_escaped_roots_before_starting_a_server() {
        let first = crate::testtmp::dir().join("project-lsp-first");
        let second = crate::testtmp::dir().join("project-lsp-second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let first = first.canonicalize().unwrap();
        let second = second.canonicalize().unwrap();
        std::fs::write(first.join("Main.java"), "class Main {}").unwrap();
        let projects = ProjectEditorState::default();
        assert!(validated_root(&projects, "editor", first.to_str().unwrap(), "Main.java").is_err());
        projects.prepare_shared_root(first.clone());
        projects.prepare_shared_root(second.clone());
        projects
            .activate_shared_root(Some(first.to_str().unwrap()))
            .unwrap();
        assert_eq!(
            validated_root(&projects, "editor", first.to_str().unwrap(), "Main.java").unwrap(),
            first
        );
        assert!(validated_root(
            &projects,
            "editor",
            first.to_str().unwrap(),
            "../outside.java"
        )
        .is_err());
        projects
            .activate_shared_root(Some(second.to_str().unwrap()))
            .unwrap();
        assert!(validated_root(&projects, "editor", first.to_str().unwrap(), "Main.java").is_err());
    }
}
