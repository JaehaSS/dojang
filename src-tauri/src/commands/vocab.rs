//! 영어 표현 단어장 커맨드 — 저장·"알아요"·간격 반복 복습.

use tauri::State;

use crate::vocab;

use super::{AppState, now, pool_of};

/// 한 번에 꺼내는 복습 카드 수. 한 자리에서 끝낼 만한 양이다.
const DUE_LIMIT: i64 = 20;

#[tauri::command]
pub async fn vocab_list(state: State<'_, AppState>) -> Result<Vec<vocab::Entry>, String> {
    let pool = pool_of(&state)?;
    vocab::list(&pool).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn vocab_due(state: State<'_, AppState>) -> Result<Vec<vocab::Entry>, String> {
    let pool = pool_of(&state)?;
    vocab::due(&pool, now(), DUE_LIMIT).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn vocab_due_count(state: State<'_, AppState>) -> Result<i64, String> {
    let pool = pool_of(&state)?;
    vocab::due_count(&pool, now()).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn vocab_save(state: State<'_, AppState>, entry: vocab::NewEntry) -> Result<vocab::Entry, String> {
    let pool = pool_of(&state)?;
    vocab::save(&pool, entry, now()).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn vocab_mark_known(state: State<'_, AppState>, phrase: String, meaning: String) -> Result<(), String> {
    let pool = pool_of(&state)?;
    vocab::mark_known(&pool, &phrase, &meaning, now()).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn vocab_set_status(state: State<'_, AppState>, id: i64, status: vocab::Status) -> Result<(), String> {
    let pool = pool_of(&state)?;
    vocab::set_status(&pool, id, status, now()).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn vocab_review(
    state: State<'_, AppState>,
    id: i64,
    remembered: bool,
) -> Result<Option<vocab::Entry>, String> {
    let pool = pool_of(&state)?;
    vocab::review(&pool, id, remembered, now()).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn vocab_delete(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let pool = pool_of(&state)?;
    vocab::delete(&pool, id).await.map_err(|e| e.to_string())
}
