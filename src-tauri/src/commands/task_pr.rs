//! Main-window commands for an explicit PR delivery action.

use super::{pool_of, require_main_notification_window, AppState};
use tauri::{State, WebviewWindow};

#[tauri::command]
pub async fn task_pr_preview(
    state: State<'_, AppState>,
    window: WebviewWindow,
    id: i64,
    input: crate::github::pr_delivery::PrDeliveryInput,
) -> Result<crate::github::pr_delivery::PrDeliveryPreview, String> {
    require_main_notification_window(&window)?;
    let _reservation = super::reserve_convo_switch(state.convo_active.clone(), id)?;
    let _claim = state.review_claims.claim_finalization(id)?;
    let pool = pool_of(&state)?;
    let task = crate::db::get_task(&pool, id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다.".to_string())?;
    let verification_summary =
        verification_summary(&crate::task_results::snapshot(&pool, id).await?);
    let request = crate::github::pr_delivery::DeliveryRequest {
        task_id: id,
        worktree: task.worktree_path.into(),
        branch: task.branch,
        base: task.base,
        verification_summary,
    };
    tauri::async_runtime::spawn_blocking(move || {
        let _reservation = _reservation;
        let _claim = _claim;
        crate::github::pr_delivery::preview(request, input)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn task_pr_execute(
    state: State<'_, AppState>,
    window: WebviewWindow,
    id: i64,
    token: String,
    action: crate::github::pr_delivery::PrDeliveryAction,
) -> Result<crate::github::pr_delivery::PrDeliveryResult, String> {
    require_main_notification_window(&window)?;
    let _reservation = super::reserve_convo_switch(state.convo_active.clone(), id)?;
    let _claim = state.review_claims.claim_finalization(id)?;
    let pool = pool_of(&state)?;
    let task = crate::db::get_task(&pool, id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다.".to_string())?;
    crate::github::pr_delivery::matches_task(
        &token,
        id,
        std::path::Path::new(&task.worktree_path),
        &task.branch,
        &task.base,
    )?;
    tauri::async_runtime::spawn_blocking(move || {
        let _reservation = _reservation;
        let _claim = _claim;
        crate::github::pr_delivery::execute(id, &token, action)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn verification_summary(snapshot: &crate::task_results::TaskResultSnapshot) -> String {
    if snapshot.receipts.is_empty() {
        return "저장된 검증 결과가 없습니다.".into();
    }
    snapshot
        .receipts
        .iter()
        .take(20)
        .map(|receipt| {
            format!(
                "- {}: {} ({})",
                receipt.observation_scope, receipt.outcome, receipt.freshness
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
