//! Local Tauri entry points. These methods are deliberately absent from Runner transport.
use super::{app_server, interaction as ledger};
use crate::{
    commands::{ActiveConvo, AppState},
    db, now,
};
use tauri::{AppHandle, Emitter, Manager, State};

fn changed(app: &AppHandle, id: i64) {
    let _ = app.emit(
        "convo-interaction://changed",
        serde_json::json!({"taskId":id}),
    );
}
#[tauri::command]
pub async fn interaction_snapshot(
    state: State<'_, AppState>,
    id: i64,
) -> Result<ledger::Snapshot, String> {
    let mut snapshot = ledger::snapshot(&crate::commands::pool_of(&state)?, id).await?;
    if snapshot.enabled
        && state
            .convo_active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&id)
        && snapshot.phase == "idle"
    {
        snapshot.phase = "finalizing".into();
    }
    if let Some(execution) = app_server::execution(id) {
        snapshot.execution_id = Some(execution);
        if app_server::cleanup_failed(id) {
            snapshot.phase = "cleanup_failed".into();
        } else if snapshot.phase == "idle" {
            snapshot.phase = "finalizing".into();
        }
    }
    Ok(snapshot)
}
#[tauri::command]
pub async fn interaction_pending_tasks(state: State<'_, AppState>) -> Result<Vec<i64>, String> {
    ledger::pending_tasks(&crate::commands::pool_of(&state)?, now()).await
}
#[tauri::command]
pub async fn interaction_draft(
    state: State<'_, AppState>,
    id: i64,
    execution_id: String,
    interaction_id: String,
    answers: Vec<ledger::Answer>,
    revision: i64,
) -> Result<i64, String> {
    ledger::draft(
        &crate::commands::pool_of(&state)?,
        id,
        &execution_id,
        &interaction_id,
        &answers,
        revision,
        now(),
    )
    .await
}
#[tauri::command]
pub async fn interaction_answer(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    execution_id: String,
    interaction_id: String,
    request_id: String,
    answers: Vec<ledger::Answer>,
) -> Result<ledger::Receipt, String> {
    let receipt = ledger::submit(
        &crate::commands::pool_of(&state)?,
        id,
        &execution_id,
        &interaction_id,
        &request_id,
        &answers,
        now(),
    )
    .await?;
    changed(&app, id);
    Ok(receipt)
}
#[tauri::command]
pub async fn interaction_receipt(
    state: State<'_, AppState>,
    id: i64,
    request_id: String,
) -> Result<ledger::Receipt, String> {
    ledger::receipt(&crate::commands::pool_of(&state)?, id, &request_id).await
}
#[tauri::command]
pub async fn interaction_cleanup_retry(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    let pool = crate::commands::pool_of(&state)?;
    if !app_server::cleanup_failed(id) && !ledger::blocked(&pool, id).await? {
        return Err("정리 재시도가 필요한 실행이 아닙니다".into());
    }
    state.control_tokens.revoke_task(id);
    if state.control_tokens.active_for_task(id) > 0 {
        return Err("진행 중인 도구 요청이 있습니다. 종료 후 다시 확인하세요".into());
    }
    app_server::recover_execution(&pool, id).await?;
    db::set_convo_pgid(&pool, id, None)
        .await
        .map_err(|e| e.to_string())?;
    db::mark_awaiting_review_with_notification(&pool, id, now(), None, "failure")
        .await
        .map_err(|e| e.to_string())?;
    app_server::unregister(id);
    state
        .convo_active
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&id);
    changed(&app, id);
    let _ = app.emit(
        "task://state",
        serde_json::json!({"id":id,"state":"AwaitingReview","awaiting_kind":null}),
    );
    Ok(())
}

/// Recovery precedes legacy adoption, and includes failed-finalization rows regardless of task state.
pub async fn recover(app: &AppHandle, pool: &sqlx::SqlitePool) -> Result<(), String> {
    let tasks:Vec<i64>=sqlx::query_scalar("SELECT task_id FROM convo_executions WHERE state IN ('starting','running','cancelling','finalizing','cleanup_failed') UNION SELECT t.id FROM tasks t JOIN convo_runtime_bindings b ON b.task_id=t.id WHERE t.state='Running'").fetch_all(pool).await.map_err(|e|e.to_string())?;
    let state = app.state::<AppState>();
    for id in tasks {
        if app_server::recover_execution(pool, id).await.is_err() {
            let snapshot = ledger::snapshot(pool, id).await?;
            if let Some(execution) = snapshot.execution_id {
                let _ = app_server::register(id, execution);
                app_server::mark_cleanup_failed(id);
            }
            // 잠금을 잡기 전에 읽는다 — 이 조회는 await이고, std Mutex 가드는 그것을 넘길 수 없다.
            let agent = sqlx::query_scalar::<_, Option<String>>("SELECT agent FROM tasks WHERE id=?")
                .bind(id)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten()
                .flatten()
                .unwrap_or_else(|| "codex".into());
            state
                .convo_active
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(
                    id,
                    ActiveConvo {
                        pgid: None,
                        vendor_bin: agent,
                        started_at: now(),
                        last_event_at: now(),
                        last_operation: Some("실행 정리 필요".into()),
                        interrupted: true,
                    },
                );
        } else {
            db::set_convo_pgid(pool, id, None)
                .await
                .map_err(|e| e.to_string())?;
            db::mark_awaiting_review_with_notification(pool, id, now(), None, "failure")
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

static CLOSE_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub fn generation() -> u64 {
    CLOSE_GENERATION.load(std::sync::atomic::Ordering::SeqCst)
}
pub fn ensure_generation(observed: u64) -> Result<(), String> {
    if generation() != observed || shutting_down() {
        Err("창 종료로 질문 실행 준비를 중단했습니다".into())
    } else {
        Ok(())
    }
}
pub fn shutting_down() -> bool {
    SHUTTING_DOWN.load(std::sync::atomic::Ordering::SeqCst)
        || CLOSING.load(std::sync::atomic::Ordering::SeqCst)
}
static CLOSING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn reopened() {
    if !SHUTTING_DOWN.load(std::sync::atomic::Ordering::SeqCst) {
        CLOSING.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}
fn note_close() {
    CLOSING.store(true, std::sync::atomic::Ordering::SeqCst);
    CLOSE_GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}
static SHUTTING_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The UI loop remains responsive until turn cleanup and state persistence release ownership.
pub fn shutdown(app: &AppHandle, quit: bool) -> bool {
    use std::sync::atomic::Ordering;
    note_close();
    let tasks = app_server::active_tasks();
    if tasks.is_empty() {
        return false;
    }
    if SHUTTING_DOWN.swap(true, Ordering::SeqCst) {
        return true;
    }
    for id in tasks {
        app_server::cancel(id);
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            if app_server::active_tasks().is_empty() {
                SHUTTING_DOWN.store(false, Ordering::SeqCst);
                if quit {
                    app.exit(0)
                } else if let Some(window) = app.get_webview_window("main") {
                    let _ = window.close();
                }
                return;
            }
            if app_server::active_tasks()
                .iter()
                .any(|id| app_server::cleanup_failed(*id))
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        SHUTTING_DOWN.store(false, Ordering::SeqCst);
        reopened();
        let _=app.emit("convo-interaction://shutdown-blocked",serde_json::json!({"message":"질문 세션 정리를 확인하지 못해 앱을 열어 두었습니다. 해당 세션에서 정리를 재시도하세요."}));
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closing_rejects_both_old_preparation_and_late_admission_until_reopened() {
        reopened();
        let before = generation();
        assert!(ensure_generation(before).is_ok());
        note_close();
        assert!(ensure_generation(before).is_err());
        assert!(ensure_generation(generation()).is_err());
        reopened();
        assert!(ensure_generation(generation()).is_ok());
        assert!(ensure_generation(before).is_err());
    }
}
