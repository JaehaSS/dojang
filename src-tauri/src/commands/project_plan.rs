use super::{pool_of, require_notification_window, AppState};
use crate::project_plan::{self, ProjectPlan, PurposeSelection, PurposeSnapshot};
use tauri::{AppHandle, Emitter, State, WebviewWindow};
fn canonical(repo: &str) -> Result<String, String> {
    std::path::Path::new(repo)
        .canonicalize()
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn project_plan_get(
    state: State<'_, AppState>,
    window: WebviewWindow,
    canonical_repo: String,
) -> Result<ProjectPlan, String> {
    require_notification_window(&window)?;
    let pool = pool_of(&state)?;
    let repo = canonical(&canonical_repo)?;
    let source_id = crate::notifications::source_id(&pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(project_plan::get(&pool, &source_id, &repo)
        .await
        .map_err(|e| e.to_string())?
        .unwrap_or(ProjectPlan {
            schema_version: 2,
            source_id,
            revision: 0,
            repo,
            objective: String::new(),
            phases: vec![],
            task_bindings: vec![],
            dependencies: vec![],
        }))
}
#[tauri::command]
pub async fn project_plan_save(
    state: State<'_, AppState>,
    window: WebviewWindow,
    app: AppHandle,
    canonical_repo: String,
    plan: ProjectPlan,
    expected_revision: i64,
    expected_source_id: String,
) -> Result<ProjectPlan, String> {
    require_notification_window(&window)?;
    let pool = pool_of(&state)?;
    let repo = canonical(&canonical_repo)?;
    let source = crate::notifications::source_id(&pool)
        .await
        .map_err(|e| e.to_string())?;
    let saved = project_plan::save(
        &pool,
        &source,
        &repo,
        plan,
        expected_revision,
        &expected_source_id,
        super::now(),
    )
    .await
    .map_err(|e| e.to_string())?;
    let _ = app.emit("project-plan://changed", &repo);
    Ok(saved)
}
#[tauri::command]
pub async fn project_plan_import(
    state: State<'_, AppState>,
    window: WebviewWindow,
    app: AppHandle,
    canonical_repo: String,
    raw: String,
) -> Result<project_plan::ImportOutcome, String> {
    require_notification_window(&window)?;
    let pool = pool_of(&state)?;
    let repo = canonical(&canonical_repo)?;
    let source = crate::notifications::source_id(&pool)
        .await
        .map_err(|e| e.to_string())?;
    let imported = project_plan::import_v1(&pool, &source, &repo, &raw, super::now())
        .await
        .map_err(|e| e.to_string())?;
    if matches!(imported, project_plan::ImportOutcome::Imported { .. }) {
        let _ = app.emit("project-plan://changed", &repo);
    }
    Ok(imported)
}
#[tauri::command]
pub async fn purpose_get(
    state: State<'_, AppState>,
    window: WebviewWindow,
    task_id: i64,
    execution_round_id: Option<String>,
) -> Result<Option<PurposeSnapshot>, String> {
    require_notification_window(&window)?;
    let pool = pool_of(&state)?;
    let source = crate::notifications::source_id(&pool)
        .await
        .map_err(|e| e.to_string())?;
    match execution_round_id {
        Some(round) => project_plan::purpose_get(&pool, &source, task_id, &round)
            .await
            .map_err(|e| e.to_string()),
        None => project_plan::purpose_latest(&pool, &source, task_id)
            .await
            .map_err(|e| e.to_string()),
    }
}
#[tauri::command]
pub async fn purpose_request_refresh(
    state: State<'_, AppState>,
    window: WebviewWindow,
    app: AppHandle,
    task_id: i64,
    canonical_repo: String,
    expected_plan_revision: i64,
    phase_id: Option<String>,
    expected_source_id: String,
) -> Result<ProjectPlan, String> {
    require_notification_window(&window)?;
    let pool = pool_of(&state)?;
    let repo = canonical(&canonical_repo)?;
    let source = crate::notifications::source_id(&pool)
        .await
        .map_err(|e| e.to_string())?;
    if expected_source_id != source {
        return Err("목적의 원천이 변경되었습니다. 새로고침하세요".into());
    }
    let bound: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM task_purpose_bindings WHERE task_id=? AND source_id=?)",
    )
    .bind(task_id)
    .bind(&source)
    .fetch_one(&pool)
    .await
    .map_err(|e| e.to_string())?;
    let plan = if !bound {
        project_plan::bind_initial(
            &pool,
            task_id,
            &PurposeSelection {
                source_id: source.clone(),
                canonical_repo: repo.clone(),
                plan_revision: expected_plan_revision,
                phase_id: phase_id.clone(),
            },
            super::now(),
        )
        .await
        .map_err(|e| e.to_string())?;
        project_plan::get(&pool, &source, &repo)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("계획을 찾을 수 없습니다")?
    } else {
        project_plan::request_refresh(
            &pool,
            &source,
            task_id,
            &repo,
            expected_plan_revision,
            phase_id.as_deref(),
            super::now(),
        )
        .await
        .map_err(|e| e.to_string())?
    };
    let _ = app.emit("project-plan://changed", &repo);
    Ok(plan)
}
