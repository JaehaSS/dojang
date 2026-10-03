use std::sync::Arc;

use tauri::{AppHandle, Manager, Runtime, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use super::registry::{
    canonical_root, display_root, OpenAction, Opening, ProjectEditorInfo, ProjectLaunch,
};
use super::ProjectEditorState;

#[tauri::command]
pub async fn project_editor_open<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, ProjectEditorState>,
    window: WebviewWindow<R>,
    root: String,
) -> Result<ProjectEditorInfo, String> {
    if window.label() != "main" {
        return Err("프로젝트 에디터는 메인 창에서만 열 수 있습니다".into());
    }
    let root = canonical_root(&root)?;
    open_project(&app, &state, root, None).await
}

/// 창고 정리 세션처럼 첫 셸을 특정 명령으로 띄워야 하는 호출자를 위한 진입.
pub async fn open_with_launch<R: Runtime>(
    app: &AppHandle<R>,
    state: &ProjectEditorState,
    root: std::path::PathBuf,
    launch: Option<ProjectLaunch>,
) -> Result<ProjectEditorInfo, String> {
    let root = canonical_root(&root.to_string_lossy())?;
    open_project(app, state, root, launch).await
}

async fn open_project<R: Runtime>(
    app: &AppHandle<R>,
    state: &ProjectEditorState,
    root: std::path::PathBuf,
    launch: Option<ProjectLaunch>,
) -> Result<ProjectEditorInfo, String> {
    loop {
        let action = state
            .registry
            .lock()
            .unwrap()
            .begin_open(root.clone(), launch.clone());
        match action {
            OpenAction::Focus(info) => {
                let Some(window) = app.get_webview_window(&info.label) else {
                    state.registry.lock().unwrap().remove_label(&info.label);
                    continue;
                };
                window.set_focus().map_err(|error| error.to_string())?;
                return Ok(info);
            }
            OpenAction::Wait(opening) => return focus_opened(app, opening.wait().await?),
            OpenAction::Create { label, opening } => {
                return create_project_window(app, state, root.clone(), label, opening);
            }
        }
    }
}

fn focus_opened<R: Runtime>(
    app: &AppHandle<R>,
    info: ProjectEditorInfo,
) -> Result<ProjectEditorInfo, String> {
    let window = app
        .get_webview_window(&info.label)
        .ok_or("프로젝트 창을 만들지 못했습니다")?;
    window.set_focus().map_err(|error| error.to_string())?;
    Ok(info)
}

fn create_project_window<R: Runtime>(
    app: &AppHandle<R>,
    state: &ProjectEditorState,
    root: std::path::PathBuf,
    label: String,
    opening: Arc<Opening>,
) -> Result<ProjectEditorInfo, String> {
    let title = format!(
        "Dojang Project Editor — {}",
        root.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Project")
    );
    let built = WebviewWindowBuilder::new(
        app,
        &label,
        WebviewUrl::App("index.html?window=project-editor".into()),
    )
    .title(title)
    .inner_size(1100.0, 800.0)
    .min_inner_size(720.0, 480.0)
    .disable_drag_drop_handler()
    .build();
    let info = match built {
        Ok(_) => state
            .registry
            .lock()
            .unwrap()
            .complete_open(&root, &opening),
        Err(error) => {
            let message = error.to_string();
            state.registry.lock().unwrap().fail_open(&root, &opening);
            opening.complete(Err(message.clone()));
            return Err(message);
        }
    }
    .ok_or("프로젝트 창 등록이 중단되었습니다")?;
    opening.complete(Ok(info.clone()));
    Ok(info)
}

#[tauri::command]
pub fn project_editor_info<R: Runtime>(
    state: State<'_, ProjectEditorState>,
    window: WebviewWindow<R>,
) -> Result<ProjectEditorInfo, String> {
    let root = state.root_for_window(window.label())?;
    let launch = state
        .registry
        .lock()
        .unwrap()
        .launch_for_label(window.label())
        .is_some();
    Ok(ProjectEditorInfo {
        root: display_root(&root),
        label: window.label().to_string(),
        launch,
    })
}

#[tauri::command]
pub async fn project_editor_tree<R: Runtime>(
    state: State<'_, ProjectEditorState>,
    window: WebviewWindow<R>,
) -> Result<Vec<crate::fsapi::FsNode>, String> {
    let root = state.root_for_window(window.label())?;
    tauri::async_runtime::spawn_blocking(move || crate::fsapi::build_tree(&root))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn project_editor_read<R: Runtime>(
    state: State<'_, ProjectEditorState>,
    window: WebviewWindow<R>,
    path: String,
) -> Result<crate::fsapi::FileContent, String> {
    let root = state.root_for_window(window.label())?;
    crate::fsapi::read_file(&root, &path).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn project_editor_write<R: Runtime>(
    state: State<'_, ProjectEditorState>,
    window: WebviewWindow<R>,
    path: String,
    content: String,
    expected_content: String,
) -> Result<i64, String> {
    let root = state.root_for_window(window.label())?;
    let _guard = state.write_guard.lock().unwrap();
    write_if_current(&root, &path, &content, &expected_content)
}

pub(super) fn write_if_current(
    root: &std::path::Path,
    path: &str,
    content: &str,
    expected: &str,
) -> Result<i64, String> {
    let current = crate::fsapi::read_file(root, path).map_err(|error| error.to_string())?;
    if !matches!(current.kind, crate::fsapi::FileKind::Text) {
        return Err("텍스트 파일만 저장할 수 있습니다".into());
    }
    if current.content != expected {
        return Err("파일이 디스크에서 변경되었습니다".into());
    }
    crate::fsapi::write_file(root, path, content).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn project_editor_resolve_path<R: Runtime>(
    state: State<'_, ProjectEditorState>,
    window: WebviewWindow<R>,
    path: String,
) -> Result<String, String> {
    let root = state.root_for_window(window.label())?;
    if path.is_empty() {
        return Ok(display_root(&root));
    }
    crate::fsapi::safe_join(&root, &path)
        .map(|path| crate::fsapi::display_path(&path))
        .map_err(|error| error.to_string())
}
