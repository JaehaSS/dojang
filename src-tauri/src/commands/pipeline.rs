//! 멀티 벤더 파이프라인 IPC 명령. 상태를 바꾸는 명령은 모두 `pipeline://changed`(payload: run id)를 낸다.
//! 실제 일은 `pipeline_driver`가 한다. 여기서는 입력 검증과 사용자 결정(승인·거절·재시도·건너뛰기·취소)만 처리한다.

use super::pipeline_driver::{self as driver, Closure};
use super::{pool_of, AppState};
use crate::db::{self, state as tstate};
use crate::multireview::{self, PipelineReviewMeta};
use crate::pipeline::db::{self as pdb, RunRow, StepRow, TicketRow};
use crate::pipeline::model::{self, Spec, SplitOutput, TicketDraft, MAX_TICKETS};
use crate::pipeline::registry;
use crate::pipeline::state::{RunState, TicketState};
use serde::Serialize;
use tauri::{AppHandle, State};

fn e2s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[derive(Serialize)]
pub struct PipelineVendor {
    vendor: String,
    available: bool,
}

#[derive(Serialize)]
pub struct PipelineDetail {
    run: RunRow,
    tickets: Vec<TicketRow>,
    steps: Vec<StepRow>,
    reviews: Vec<PipelineReviewMeta>,
}

/// 시작 전 검증에서 쓰는 순수 규칙: 가용 벤더가 2개 이상이고 claude가 있어야 한다.
pub(crate) fn check_vendors(available: &[String]) -> Result<(), String> {
    if available.len() < 2 {
        return Err(format!(
            "교차 리뷰에는 PATH에 있는 벤더가 2개 이상 필요합니다(현재 {}개: {})",
            available.len(),
            available.join(", ")
        ));
    }
    if !available.iter().any(|v| v == "claude") {
        return Err("claude CLI가 PATH에 있어야 합니다(분할과 종합을 맡습니다)".into());
    }
    Ok(())
}

pub(crate) fn revision_conflict() -> String {
    "계획 revision이 바뀌었습니다. 새로고침한 뒤 다시 시도하세요".into()
}

#[tauri::command]
pub async fn pipeline_vendors() -> Result<Vec<PipelineVendor>, String> {
    Ok(["claude", "codex", "agy"]
        .into_iter()
        .map(|v| PipelineVendor { vendor: v.into(), available: crate::reviewer::on_path(v) })
        .collect())
}

#[tauri::command]
pub async fn pipeline_start(
    app: AppHandle,
    state: State<'_, AppState>,
    repo: String,
    base_branch: String,
    goal: String,
) -> Result<RunRow, String> {
    let pool = pool_of(&state)?;
    let goal = goal.trim().to_string();
    if goal.is_empty() {
        return Err("목표를 입력하세요".into());
    }
    let repo = std::path::Path::new(&repo)
        .canonicalize()
        .map_err(|e| format!("저장소 경로를 확인할 수 없습니다: {e}"))?;
    let base = base_branch.trim().to_string();
    if base.is_empty() || !crate::worktree::local_branch_exists(&repo, &base) {
        return Err(format!("base 브랜치 `{base}`가 로컬 브랜치로 존재하지 않습니다"));
    }
    let repo = repo.to_string_lossy().into_owned();
    if pdb::active_run_for_repo(&pool, &repo).await.map_err(e2s)?.is_some() {
        return Err("이 저장소에는 이미 진행 중인 파이프라인이 있습니다".into());
    }
    check_vendors(&driver::available_vendors())?;
    let id = pdb::insert_run(&pool, &repo, &base, &goal, super::now()).await.map_err(e2s)?;
    driver::emit_changed(&app, id);
    pdb::get_run(&pool, id).await.map_err(e2s)?.ok_or_else(|| "실행을 읽지 못했습니다".into())
}

#[tauri::command]
pub async fn pipeline_list(state: State<'_, AppState>, repo: Option<String>) -> Result<Vec<RunRow>, String> {
    let pool = pool_of(&state)?;
    let repo = repo.map(|r| {
        std::path::Path::new(&r)
            .canonicalize()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or(r)
    });
    pdb::list_runs(&pool, repo.as_deref()).await.map_err(e2s)
}

#[tauri::command]
pub async fn pipeline_get(state: State<'_, AppState>, run_id: i64) -> Result<PipelineDetail, String> {
    let pool = pool_of(&state)?;
    let run = get_run(&pool, run_id).await?;
    Ok(PipelineDetail {
        tickets: pdb::list_tickets(&pool, run_id).await.map_err(e2s)?,
        steps: pdb::list_steps(&pool, run_id).await.map_err(e2s)?,
        reviews: multireview::list_pipeline_reviews(&pool, run_id).await.map_err(e2s)?,
        run,
    })
}

async fn get_run(pool: &sqlx::SqlitePool, run_id: i64) -> Result<RunRow, String> {
    pdb::get_run(pool, run_id)
        .await
        .map_err(e2s)?
        .ok_or_else(|| format!("파이프라인 실행 {run_id}을(를) 찾을 수 없습니다"))
}

#[tauri::command]
pub async fn pipeline_update_tickets(
    app: AppHandle,
    state: State<'_, AppState>,
    run_id: i64,
    expected_revision: i64,
    tickets: Vec<TicketDraft>,
) -> Result<RunRow, String> {
    let pool = pool_of(&state)?;
    let run = get_run(&pool, run_id).await?;
    let spec: Spec = run.spec_json.as_deref().and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default();
    let avail = driver::available_vendors();
    let refs: Vec<&str> = avail.iter().map(String::as_str).collect();
    let out = SplitOutput { spec, tickets };
    model::validate_split(&out, &refs, MAX_TICKETS).map_err(|e| e.join("; "))?;
    if !pdb::update_tickets_cas(&pool, run_id, expected_revision, &out.tickets, super::now())
        .await
        .map_err(e2s)?
    {
        return Err(revision_conflict());
    }
    driver::assign_reviewers(&pool, run_id, &avail).await?;
    driver::emit_changed(&app, run_id);
    get_run(&pool, run_id).await
}

#[tauri::command]
pub async fn pipeline_approve_plan(
    app: AppHandle,
    state: State<'_, AppState>,
    run_id: i64,
    expected_revision: i64,
) -> Result<RunRow, String> {
    let pool = pool_of(&state)?;
    get_run(&pool, run_id).await?;
    driver::assign_reviewers(&pool, run_id, &driver::available_vendors()).await?;
    if !pdb::approve_plan_cas(&pool, run_id, expected_revision, super::now()).await.map_err(e2s)? {
        return Err(revision_conflict());
    }
    driver::emit_changed(&app, run_id);
    get_run(&pool, run_id).await
}

#[tauri::command]
pub async fn pipeline_reject_plan(
    app: AppHandle,
    state: State<'_, AppState>,
    run_id: i64,
    comment: String,
) -> Result<RunRow, String> {
    let pool = pool_of(&state)?;
    let comment = comment.trim().to_string();
    if comment.is_empty() {
        return Err("거절 사유를 입력하세요".into());
    }
    if !pdb::reject_plan(&pool, run_id, &comment, super::now()).await.map_err(e2s)? {
        return Err("계획 승인 대기 상태가 아니라 거절할 수 없습니다".into());
    }
    driver::emit_changed(&app, run_id);
    get_run(&pool, run_id).await
}

async fn ticket_of_run(pool: &sqlx::SqlitePool, run_id: i64, ticket_id: i64) -> Result<TicketRow, String> {
    pdb::get_ticket(pool, ticket_id)
        .await
        .map_err(e2s)?
        .filter(|t| t.run_id == run_id)
        .ok_or_else(|| format!("이 실행에 티켓 {ticket_id}이(가) 없습니다"))
}

/// 에스컬레이션된 티켓이 더 없고 실행이 Executing에서 멈춰 있으면 재개한다.
async fn resume_if_clear(pool: &sqlx::SqlitePool, run_id: i64) -> Result<(), String> {
    let run = get_run(pool, run_id).await?;
    if run.run_state() != Some(RunState::Paused) || run.paused_from.as_deref() != Some("executing") {
        return Ok(());
    }
    let any_escalated = pdb::list_tickets(pool, run_id)
        .await
        .map_err(e2s)?
        .iter()
        .any(|t| t.ticket_state() == Some(TicketState::Escalated));
    if !any_escalated {
        pdb::resume_run(pool, run_id, super::now()).await.map_err(e2s)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn pipeline_retry_ticket(
    app: AppHandle,
    state: State<'_, AppState>,
    run_id: i64,
    ticket_id: i64,
    vendor: Option<String>,
) -> Result<RunRow, String> {
    let pool = pool_of(&state)?;
    let t = ticket_of_run(&pool, run_id, ticket_id).await?;
    if t.ticket_state() != Some(TicketState::Escalated) {
        return Err("에스컬레이션된 티켓만 재시도할 수 있습니다".into());
    }
    let vendor = vendor.map(|v| model::normalize_vendor(&v)).filter(|v| !v.is_empty());
    if let Some(v) = &vendor {
        if !driver::available_vendors().contains(v) {
            return Err(format!("벤더 `{v}`을(를) PATH에서 찾을 수 없습니다"));
        }
    }
    if !pdb::retry_ticket(&pool, ticket_id, vendor.as_deref(), super::now()).await.map_err(e2s)? {
        return Err("티켓 상태가 바뀌어 재시도하지 못했습니다".into());
    }
    if let Some(old) = t.task_id {
        driver::stop_and_close_task(&state, old, Closure::Abandon).await;
    }
    driver::assign_reviewers(&pool, run_id, &driver::available_vendors()).await?;
    resume_if_clear(&pool, run_id).await?;
    driver::emit_changed(&app, run_id);
    get_run(&pool, run_id).await
}

#[tauri::command]
pub async fn pipeline_skip_ticket(
    app: AppHandle,
    state: State<'_, AppState>,
    run_id: i64,
    ticket_id: i64,
) -> Result<RunRow, String> {
    let pool = pool_of(&state)?;
    let t = ticket_of_run(&pool, run_id, ticket_id).await?;
    if t.ticket_state() != Some(TicketState::Escalated) {
        return Err("에스컬레이션된 티켓만 건너뛸 수 있습니다".into());
    }
    let before = pdb::list_tickets(&pool, run_id).await.map_err(e2s)?;
    let cancelled = pdb::skip_ticket_cascade(&pool, run_id, ticket_id, super::now()).await.map_err(e2s)?;
    for id in cancelled {
        if let Some(task) = before.iter().find(|x| x.id == id).and_then(|x| x.task_id) {
            driver::stop_and_close_task(&state, task, Closure::Abandon).await;
        }
    }
    resume_if_clear(&pool, run_id).await?;
    driver::emit_changed(&app, run_id);
    get_run(&pool, run_id).await
}

#[tauri::command]
pub async fn pipeline_resume(app: AppHandle, state: State<'_, AppState>, run_id: i64) -> Result<RunRow, String> {
    let pool = pool_of(&state)?;
    if pdb::resume_run(&pool, run_id, super::now()).await.map_err(e2s)?.is_none() {
        return Err("일시정지 상태가 아니라 재개할 수 없습니다".into());
    }
    // 재개 직후 같은 사유로 곧바로 멈추지 않도록 직전 오류를 지운다.
    let _ = pdb::set_run_error(&pool, run_id, None, super::now()).await;
    driver::emit_changed(&app, run_id);
    get_run(&pool, run_id).await
}

#[tauri::command]
pub async fn pipeline_cancel(app: AppHandle, state: State<'_, AppState>, run_id: i64) -> Result<RunRow, String> {
    let pool = pool_of(&state)?;
    let run = get_run(&pool, run_id).await?;
    if run.run_state().is_none_or(|s| s.is_terminal()) {
        return Ok(run);
    }
    // 먼저 자식 프로세스를 죽이고 새 등록을 막는다(R19).
    registry::kill_run(run_id);
    pdb::cancel_run(&pool, run_id, super::now()).await.map_err(e2s)?;
    for t in pdb::list_tickets(&pool, run_id).await.map_err(e2s)? {
        if let Some(task) = t.task_id {
            driver::stop_and_close_task(&state, task, Closure::Preserve).await;
        }
    }
    // 통합 작업은 검토 대기로 올리지 않는다 — 검토되지 않은 부분 통합 결과가 한 번에 병합 가능해지기 때문이다.
    // 워크트리만 걷어내고 브랜치는 보존한 채 Discarded로 닫는다(수동 복구는 브랜치에서 한다).
    if let Some(id) = run.integration_task_id {
        if let Ok(Some(task)) = db::get_task(&pool, id).await {
            if !matches!(task.state.as_str(), tstate::DONE | tstate::DISCARDED) {
                driver::stop_and_close_task(&state, id, Closure::Preserve).await;
            }
        }
    }
    driver::emit_changed(&app, run_id);
    get_run(&pool, run_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn vendors_need_two_and_claude() {
        assert!(check_vendors(&v(&["claude"])).is_err());
        assert!(check_vendors(&v(&[])).is_err());
        assert!(check_vendors(&v(&["codex", "agy"])).unwrap_err().contains("claude"));
        assert!(check_vendors(&v(&["claude", "codex"])).is_ok());
    }

    #[test]
    fn revision_error_mentions_revision() {
        assert!(revision_conflict().contains("revision"));
    }
}

#[cfg(test)]
mod review_passthrough_tests {
    use crate::commands::MultiReviewResult;

    #[test]
    fn structured_fields_survive_a_roundtrip() {
        let raw = r#"{"items":[],"synthesis":"x","vendor_reviews":[{"vendor":"codex"}],"synthesis_structured":{"decision":"fix"},"blocking":true}"#;
        let r: MultiReviewResult = serde_json::from_str(raw).unwrap();
        let back = serde_json::to_value(&r).unwrap();
        assert_eq!(back["blocking"], true);
        assert_eq!(back["vendor_reviews"][0]["vendor"], "codex");
        assert_eq!(back["synthesis_structured"]["decision"], "fix");
    }

    #[test]
    fn plain_reviews_omit_the_extra_fields() {
        let r: MultiReviewResult = serde_json::from_str(r#"{"items":[],"synthesis":null}"#).unwrap();
        let back = serde_json::to_value(&r).unwrap();
        assert!(back.get("blocking").is_none() && back.get("vendor_reviews").is_none());
    }
}
