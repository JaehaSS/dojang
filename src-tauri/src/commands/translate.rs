//! 영어로 일하기 보조 커맨드 — 입력창 영어 다듬기와 답변 한국어 번역.

use tauri::State;

use crate::translate;

use super::{AppState, pool_of};

#[tauri::command]
pub async fn translate_settings_get(state: State<'_, AppState>) -> Result<translate::TranslateSettings, String> {
    let pool = pool_of(&state)?;
    Ok(translate::load_settings(&pool).await)
}

#[tauri::command]
pub async fn translate_settings_set(
    state: State<'_, AppState>,
    settings: translate::TranslateSettings,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    translate::save_settings(&pool, &settings).await
}

/// 한 번 옮긴다. 대화·세션 기록에 남기지 않는다.
#[tauri::command]
pub async fn translate_text(
    state: State<'_, AppState>,
    direction: translate::Direction,
    text: String,
) -> Result<String, String> {
    let pool = pool_of(&state)?;
    translate::translate(&pool, direction, &text).await
}
