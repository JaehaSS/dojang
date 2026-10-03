use super::{pool_of, require_notification_window, AppState};
use tauri::{AppHandle, Emitter, State, WebviewWindow};

#[tauri::command]
pub async fn task_result_get(
    state: State<'_, AppState>,
    window: WebviewWindow,
    id: i64,
) -> Result<crate::task_results::TaskResultSnapshot, String> {
    require_notification_window(&window)?;
    crate::task_results::snapshot(&pool_of(&state)?, id).await
}
#[tauri::command]
pub async fn task_result_reference(
    state: State<'_, AppState>,
    window: WebviewWindow,
    app: AppHandle,
    id: i64,
    source_id: String,
    expected_revision: i64,
    kind: String,
    relative_path: String,
) -> Result<crate::task_results::TaskResultSnapshot, String> {
    require_notification_window(&window)?;
    let result = crate::task_results::add_reference(
        &pool_of(&state)?,
        id,
        crate::task_results::ReferenceInput {
            expected_source_id: source_id,
            expected_revision,
            kind,
            relative_path,
        },
    )
    .await?;
    let _ = app.emit("task-result://changed", id);
    Ok(result)
}
#[tauri::command]
pub async fn task_review_get(
    state: State<'_, AppState>,
    window: WebviewWindow,
    id: i64,
) -> Result<crate::task_review::Snapshot, String> {
    require_notification_window(&window)?;
    crate::task_review::snapshot(&pool_of(&state)?, id).await
}
#[tauri::command]
pub async fn task_review_decide(
    state: State<'_, AppState>,
    window: WebviewWindow,
    app: AppHandle,
    id: i64,
    input: crate::task_review::Input,
) -> Result<crate::task_review::Snapshot, String> {
    require_notification_window(&window)?;
    let pool = pool_of(&state)?;
    // Decisions serialize against verification/finalization, not against execution authority.
    let claim = state.review_claims.claim_finalization(id)?;
    crate::task_review::decide(&pool, id, &input).await?;
    drop(claim);
    if input.action == "request_changes" {
        if let Some(message) =
            crate::task_review::claim_delivery(&pool, id, &input.mutation_id).await?
        {
            let receipt = format!("review:{}", input.mutation_id);
            let result = super::convo_send_with_receipt(
                app.clone(),
                state,
                id,
                message,
                None,
                Some(&receipt),
                None,
            )
            .await;
            crate::task_review::finish_delivery(&pool, &input.mutation_id, &result).await?;
        }
    }
    let _ = app.emit("task-result://changed", id);
    crate::task_review::snapshot(&pool, id).await
}

/// Resume only a never-claimed request. An uncertain delivery cannot be replayed.
#[tauri::command]
pub async fn task_review_deliver(
    state: State<'_, AppState>,
    window: WebviewWindow,
    app: AppHandle,
    id: i64,
    source_id: String,
    mutation_id: String,
) -> Result<crate::task_review::Snapshot, String> {
    require_notification_window(&window)?;
    let pool = pool_of(&state)?;
    let view = crate::task_review::snapshot(&pool, id).await?;
    if view.source_id != source_id {
        return Err("검토 원천이 변경되었습니다".into());
    }
    if let Some(message) = crate::task_review::claim_delivery(&pool, id, &mutation_id).await? {
        let receipt = format!("review:{mutation_id}");
        let result = super::convo_send_with_receipt(
            app.clone(),
            state,
            id,
            message,
            None,
            Some(&receipt),
            None,
        )
        .await;
        crate::task_review::finish_delivery(&pool, &mutation_id, &result).await?;
    }
    let _ = app.emit("task-result://changed", id);
    crate::task_review::snapshot(&pool, id).await
}
