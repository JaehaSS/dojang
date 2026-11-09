use tauri::{AppHandle, Emitter, State, WebviewWindow};

use super::{pool_of, require_main_notification_window, reserve_convo_switch, AppState};

#[tauri::command]
pub async fn task_archive(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
    id: i64,
) -> Result<crate::task_archive::Archive, String> {
    require_main_notification_window(&window)?;
    let _review_claim = state.review_claims.claim_finalization(id)?;
    let _convo_reservation = reserve_convo_switch(state.convo_active.clone(), id)?;
    let archive = crate::task_archive::archive(&pool_of(&state)?, id, crate::now()).await?;
    let _ = app.emit("task://archive-changed", id);
    Ok(archive)
}

#[tauri::command]
pub async fn task_restore(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
    id: i64,
) -> Result<(), String> {
    require_main_notification_window(&window)?;
    let _review_claim = state.review_claims.claim_finalization(id)?;
    let _convo_reservation = reserve_convo_switch(state.convo_active.clone(), id)?;
    crate::task_archive::restore(&pool_of(&state)?, id).await?;
    let _ = app.emit("task://archive-changed", id);
    Ok(())
}
