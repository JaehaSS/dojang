use super::{pool_of, require_main_notification_window, AppState};
use tauri::{State, WebviewWindow};

#[tauri::command]
pub async fn task_spec_search(
    state: State<'_, AppState>,
    window: WebviewWindow,
    id: i64,
    query: String,
) -> Result<crate::task_specs::SpecSearchPage, String> {
    require_main_notification_window(&window)?;
    crate::task_specs::search(&pool_of(&state)?, id, &query).await
}

#[tauri::command]
pub async fn task_spec_read(
    state: State<'_, AppState>,
    window: WebviewWindow,
    id: i64,
    relative_path: String,
) -> Result<crate::task_specs::SpecDocument, String> {
    require_main_notification_window(&window)?;
    crate::task_specs::read(&pool_of(&state)?, id, &relative_path).await
}

#[tauri::command]
pub async fn task_spec_save(
    state: State<'_, AppState>,
    window: WebviewWindow,
    id: i64,
    input: crate::task_specs::SpecSaveInput,
) -> Result<crate::task_specs::SpecSnapshot, String> {
    require_main_notification_window(&window)?;
    crate::task_specs::save(&pool_of(&state)?, id, input).await
}

#[tauri::command]
pub async fn task_spec_list(
    state: State<'_, AppState>,
    window: WebviewWindow,
    id: i64,
) -> Result<Vec<crate::task_specs::SpecVersion>, String> {
    require_main_notification_window(&window)?;
    crate::task_specs::list(&pool_of(&state)?, id).await
}

#[tauri::command]
pub async fn task_spec_get(
    state: State<'_, AppState>,
    window: WebviewWindow,
    id: i64,
    snapshot_id: i64,
) -> Result<crate::task_specs::SpecSnapshot, String> {
    require_main_notification_window(&window)?;
    crate::task_specs::get(&pool_of(&state)?, id, snapshot_id).await
}

#[tauri::command]
pub async fn task_spec_validate_links(
    state: State<'_, AppState>,
    window: WebviewWindow,
    id: i64,
    relative_path: String,
    expected_hash: String,
) -> Result<crate::task_specs::links::SpecLinkReport, String> {
    require_main_notification_window(&window)?;
    crate::task_specs::links::validate(&pool_of(&state)?, id, &relative_path, &expected_hash).await
}
