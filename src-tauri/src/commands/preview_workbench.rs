use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::{AppHandle, State, Webview};

use crate::preview_workbench::{ReceiptStatus, ReceiptView};

use super::{
    pool_of, start_convo_turn, AppState, ConversationInputOrigin, ConvoAdmissionAction,
    PreviewReceiptAcceptance,
};
mod support;
#[cfg(test)]
mod tests;
use support::{
    current_url, ensure_current_url, ensure_supported, require_main, task_of, unsupported_reason,
};

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewSource {
    Manual,
    PreviewQueue,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewWorkbenchState {
    pub task_id: i64,
    pub busy: &'static str,
    pub url: Option<String>,
    pub convo_active: bool,
    pub taken_over: bool,
    pub app_epoch: String,
    pub supported: bool,
    pub unsupported_reason: Option<String>,
}

#[tauri::command]
pub async fn preview_workbench_state(
    webview: Webview,
    state: State<'_, AppState>,
    task_id: i64,
) -> Result<PreviewWorkbenchState, String> {
    require_main(&webview)?;
    let task = task_of(&state, task_id).await?;
    let active = state
        .convo_active
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains_key(&task_id);
    let unsupported_reason = unsupported_reason(&task);
    Ok(PreviewWorkbenchState {
        task_id,
        busy: if active { "busy" } else { "idle" },
        url: current_url(&state, task_id)?,
        convo_active: active,
        taken_over: state.preview_bridge.is_taken_over(task_id),
        app_epoch: state.preview_workbench.app_epoch().to_string(),
        supported: unsupported_reason.is_none(),
        unsupported_reason,
    })
}

#[tauri::command]
pub async fn preview_workbench_prepare(
    webview: Webview,
    state: State<'_, AppState>,
    task_id: i64,
    correlation_id: String,
    message: String,
    url: String,
    source: PreviewSource,
) -> Result<ReceiptView, String> {
    require_main(&webview)?;
    ensure_supported(&task_of(&state, task_id).await?)?;
    let context = context(&message, &url, &source);
    if let Some(receipt) =
        state
            .preview_workbench
            .prepared_for(task_id, &correlation_id, &context, super::now())?
    {
        return Ok(receipt);
    }
    ensure_current_url(&state, task_id, &url)?;
    state
        .preview_workbench
        .prepare_for(task_id, &correlation_id, &context, super::now())
}

#[tauri::command]
pub async fn preview_workbench_send(
    app: AppHandle,
    webview: Webview,
    state: State<'_, AppState>,
    task_id: i64,
    request_id: String,
    message: String,
    url: String,
    source: PreviewSource,
) -> Result<ReceiptView, String> {
    require_main(&webview)?;
    let context = context(&message, &url, &source);
    if let Some(receipt) =
        existing_receipt(&state.preview_workbench, task_id, &request_id, &context)?
    {
        return Ok(receipt);
    }
    let _claim = state.review_claims.claim_finalization(task_id)?;
    let task = task_of(&state, task_id).await?;
    ensure_supported(&task)?;
    if !Path::new(&task.worktree_path).is_dir() {
        return Err(crate::worktree::missing_worktree_error(&task.worktree_path));
    }
    if let Err(error) = ensure_current_url(&state, task_id, &url) {
        if error == "preview_url_changed" {
            return reject_changed_preview(
                &state.preview_workbench,
                task_id,
                &request_id,
                &context,
            );
        }
        return Err(error);
    }
    if state
        .convo_active
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains_key(&task_id)
    {
        return Ok(state
            .preview_workbench
            .receipt_at(task_id, &request_id, super::now()));
    }
    start_convo_turn(
        app.clone(),
        pool_of(&state)?,
        state.convo_active.clone(),
        task_id,
        task.repo,
        task.worktree_path,
        message,
        Vec::new(),
        None,
        ConversationInputOrigin::UserMessage,
        Some(format!("preview-workbench:{task_id}:{request_id}")),
        state.updating.clone(),
        Some(PreviewReceiptAcceptance {
            workbench: state.preview_workbench.clone(),
            request_id: request_id.clone(),
            context,
        }),
        if matches!(source, PreviewSource::Manual) {
            ConvoAdmissionAction::ReleaseManualTakeover
        } else {
            ConvoAdmissionAction::PreserveTakeover
        },
    )
    .await?;
    Ok(state
        .preview_workbench
        .receipt_at(task_id, &request_id, super::now()))
}

#[tauri::command]
pub fn preview_workbench_receipt(
    webview: Webview,
    state: State<'_, AppState>,
    task_id: i64,
    request_id: String,
) -> Result<ReceiptView, String> {
    require_main(&webview)?;
    Ok(state
        .preview_workbench
        .receipt_at(task_id, &request_id, super::now()))
}

fn existing_receipt(
    workbench: &crate::preview_workbench::PreviewWorkbench,
    task_id: i64,
    request_id: &str,
    context: &str,
) -> Result<Option<ReceiptView>, String> {
    let receipt = workbench.receipt_at(task_id, request_id, super::now());
    if receipt.status == ReceiptStatus::Prepared {
        return Ok(None);
    }
    if matches!(
        receipt.status,
        ReceiptStatus::Accepted | ReceiptStatus::Finished | ReceiptStatus::Rejected
    ) {
        return workbench
            .accept(task_id, request_id, context, super::now())
            .map(Some);
    }
    Ok(Some(receipt))
}

fn reject_changed_preview(
    workbench: &crate::preview_workbench::PreviewWorkbench,
    task_id: i64,
    request_id: &str,
    context: &str,
) -> Result<ReceiptView, String> {
    workbench.reject_prepared(task_id, request_id, context, super::now())
}

fn context(message: &str, url: &str, source: &PreviewSource) -> String {
    let source = match source {
        PreviewSource::Manual => "manual",
        PreviewSource::PreviewQueue => "preview_queue",
    };
    serde_json::json!({ "message": message, "source": source, "url": url }).to_string()
}
