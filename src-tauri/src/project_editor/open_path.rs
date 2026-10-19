use std::path::{Path, PathBuf};

use tauri::{AppHandle, Runtime, State, WebviewWindow};
use tauri_plugin_opener::OpenerExt;

use super::ProjectEditorState;

#[tauri::command]
pub async fn project_editor_open_path<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, ProjectEditorState>,
    window: WebviewWindow<R>,
    path: String,
    root: Option<String>,
) -> Result<(), String> {
    let root = state.root_for_request(window.label(), root.as_deref())?;
    tauri::async_runtime::spawn_blocking(move || {
        open_verified(&root, &path, |file| {
            let file = file
                .to_str()
                .ok_or("검증한 파일 경로를 UTF-8로 표현할 수 없습니다")?;
            app.opener()
                .open_path(file, None::<&str>)
                .map_err(|error| error.to_string())
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(super) fn open_verified(
    root: &Path,
    path: &str,
    opener: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let file = regular_file(root, path)?;
    opener(&file)
}

fn regular_file(root: &Path, path: &str) -> Result<PathBuf, String> {
    let file = crate::fsapi::safe_join(root, path).map_err(|error| error.to_string())?;
    let metadata = std::fs::symlink_metadata(&file).map_err(|error| error.to_string())?;
    if metadata.file_type().is_file() {
        Ok(file)
    } else {
        Err("일반 파일만 기본 앱으로 열 수 있습니다".into())
    }
}
