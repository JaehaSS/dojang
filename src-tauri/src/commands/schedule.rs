//! 스케줄·리마인더·cron 커맨드.
//!
//! `commands.rs`에서 갈라져 나왔다 — 모든 기능이 한 파일 끝에 줄을 붙이는
//! 구조가 병행 worktree의 머지 충돌을 만들었다.


use tauri::State;

use crate::db::{self};

use super::{AppState, now, pool_of};

#[tauri::command]
pub async fn schedule_list(state: State<'_, AppState>) -> Result<Vec<db::Schedule>, String> {
    let pool = pool_of(&state)?;
    db::list_schedules(&pool).await.map_err(|e| e.to_string())
}

/// 스케줄 등록. cron 식 유효성(`cron::Schedule::from_str`)과 `kind`(러너가 실행할 수 있는 3종)를
/// 사전 검증한다 — 잘못된 값은 저장 전에 거부. tz_offset_secs는 클라가 제공하는 브라우저 timezone.
#[tauri::command]
pub async fn schedule_add(
    state: State<'_, AppState>,
    label: String,
    cron: String,
    kind: String,
    payload: String,
    tz_offset_secs: i32,
) -> Result<i64, String> {
    std::str::FromStr::from_str(&cron)
        .map(|_: cron::Schedule| ())
        .map_err(|e: cron::error::Error| e.to_string())?;
    if !matches!(kind.as_str(), "task" | "reminder" | "quiz") {
        return Err(format!("알 수 없는 스케줄 종류: {kind}"));
    }
    let pool = pool_of(&state)?;
    db::insert_schedule(&pool, &label, &cron, &kind, &payload, now(), tz_offset_secs)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn schedule_remove(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let pool = pool_of(&state)?;
    db::remove_schedule(&pool, id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn schedule_set_enabled(
    state: State<'_, AppState>,
    id: i64,
    enabled: bool,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    db::set_schedule_enabled(&pool, id, enabled)
        .await
        .map_err(|e| e.to_string())
}

/// 1회성 상대시각 리마인더 등록 — "5분 후/30분 후" 알림. `run_at`은 서버 `now()` 기준으로
/// 계산(클라 시계 불신). cron 반복 스케줄과 달리 틱 루프에서 1회 발화 후 자동 비활성화된다.
#[tauri::command]
pub async fn reminder_add(
    state: State<'_, AppState>,
    text: String,
    delay_minutes: i64,
) -> Result<i64, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("리마인더 내용이 비어있습니다".to_string());
    }
    if delay_minutes <= 0 {
        return Err("delay_minutes는 0보다 커야 합니다".to_string());
    }
    let now = now();
    let run_at = now + delay_minutes * 60;
    let label = if text.chars().count() > 20 {
        text.chars().take(20).collect::<String>()
    } else {
        text.clone()
    };
    let payload = serde_json::json!({ "text": text }).to_string();
    let pool = pool_of(&state)?;
    // 1회성 리마인더는 timezone 적용 안 함 (절대 epoch초)
    db::insert_schedule_with_run_at(
        &pool,
        &label,
        "",
        "reminder",
        &payload,
        Some(run_at),
        now,
        32400,
    )
    .await
    .map_err(|e| e.to_string())
}

/// Cron 표현식의 다음 N개 실행 시각 미리보기 (timezone 적용).
#[tauri::command]
pub async fn cron_next_runs(
    cron: String,
    tz_offset_secs: i32,
    count: usize,
) -> Result<Vec<String>, String> {
    crate::schedule::cron_next_runs(&cron, tz_offset_secs, count.min(10))
        .map_err(|e| format!("Cron 파싱 실패: {e}"))
}

// ── 설정: 캡처/반성 opt-in ──

