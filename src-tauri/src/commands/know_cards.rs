//! 알아둘 것 카드 커맨드 — 채널 카드의 조회·읽음·동작과 켜기/끄기.
//!
//! 원격(Runner) 전송에는 넣지 않는다. 판정이 로컬 대화 턴에서만 돌기 때문이다(설계 3.8).

use tauri::{AppHandle, Emitter, State};

use crate::know_cards;

use super::{AppState, pool_of};

#[tauri::command]
pub async fn know_cards_list(
    state: State<'_, AppState>,
    task_id: i64,
) -> Result<Vec<know_cards::KnowCard>, String> {
    let pool = pool_of(&state)?;
    know_cards::list_active(&pool, task_id).await.map_err(|e| e.to_string())
}

/// 카드가 실제로 화면에 보였을 때만 부른다. 통지는 없다 — 부른 쪽이 이미 안다.
#[tauri::command]
pub async fn know_card_seen(state: State<'_, AppState>, card_id: i64) -> Result<(), String> {
    let pool = pool_of(&state)?;
    know_cards::mark_seen(&pool, card_id, crate::now()).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn know_card_act(
    app: AppHandle,
    state: State<'_, AppState>,
    card_id: i64,
    action: know_cards::CardAction,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    let task_id = know_cards::act(&pool, card_id, action, crate::now())
        .await
        .map_err(|e| e.to_string())?;
    if let Some(task_id) = task_id {
        let _ = app.emit(know_cards::CHANGED_EVENT, serde_json::json!({ "taskId": task_id }));
    }
    Ok(())
}

#[tauri::command]
pub async fn know_cards_enabled_get(state: State<'_, AppState>) -> Result<bool, String> {
    let pool = pool_of(&state)?;
    Ok(know_cards::enabled(&pool).await)
}

#[tauri::command]
pub async fn know_cards_enabled_set(state: State<'_, AppState>, on: bool) -> Result<(), String> {
    let pool = pool_of(&state)?;
    know_cards::set_enabled(&pool, on).await.map_err(|e| e.to_string())
}
