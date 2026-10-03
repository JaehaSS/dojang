use super::{pool_of, AppState};
use crate::{agent::service_tier, db, db::Task};
use sqlx::SqlitePool;
use tauri::State;

#[tauri::command]
pub fn codex_speed_models() -> Vec<String> {
    service_tier::supported_models()
}

pub async fn set_checked(pool: &SqlitePool, id: i64, tier: &str) -> Result<Task, String> {
    let task = db::get_task(pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    if task.mode != "conversation"
        || task.ensemble.as_deref().is_some_and(|s| !s.is_empty())
        || !db::debate_sides(pool, id)
            .await
            .map_err(|e| e.to_string())?
            .is_empty()
    {
        return Err("실행 속도는 Codex 일반 대화에서만 선택할 수 있습니다".into());
    }
    let tier = service_tier::normalize(Some(tier))?.ok_or("실행 속도를 선택해 주세요")?;
    service_tier::validate(
        task.agent.as_deref().unwrap_or_default(),
        task.model.as_deref().unwrap_or_default(),
        Some(tier),
        &service_tier::supported_models(),
    )?;
    if !db::set_task_service_tier(pool, &task, tier)
        .await
        .map_err(|e| e.to_string())?
    {
        return Err("모델 또는 에이전트가 변경되었습니다. 다시 선택해 주세요".into());
    }
    db::get_task(pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다".into())
}

#[tauri::command]
pub async fn task_service_tier_set(
    state: State<'_, AppState>,
    id: i64,
    service_tier: String,
) -> Result<Task, String> {
    set_checked(&pool_of(&state)?, id, &service_tier).await
}
