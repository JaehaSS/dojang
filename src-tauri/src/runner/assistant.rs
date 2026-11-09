//! 상시 비서(`kind = 'assistant'`) 단일 세션 관리 및 인라인 승인 큐.
//!
//! 호스트(`praxis-runner` 또는 데스크톱 모바일 표면)당 정확히 1개의 상시 비서 세션을
//! 보장하고, 벤더 전환 시 최근 대화 요약을 `pending_capsule`에 적재하며, 파괴적 관제
//! 동작(머지 승인·폐기·보류 실행)에 대한 인라인 승인 카드 생명주기를 관리한다.

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::db;
use crate::runner::actions::{ApiError, SharedTaskActions};
use crate::runner::config::RunnerConfig;
use crate::runner::http::RunnerHttpState;

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct AssistantApproval {
    pub id: i64,
    pub assistant_task_id: i64,
    pub target_task_id: Option<i64>,
    pub action_kind: String,
    pub summary: String,
    pub payload_json: String,
    pub status: String,
    pub created_at: i64,
    pub resolved_at: Option<i64>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OrchestratorOverview {
    pub running_count: i64,
    pub queued_count: i64,
    pub awaiting_review_count: i64,
    pub question_count: i64,
    pub pending_approval_count: i64,
    pub failed_count: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct AssistantSnapshot {
    pub task: db::Task,
    pub pending_approvals: Vec<AssistantApproval>,
    pub overview: OrchestratorOverview,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AssistantMessageRequest {
    pub message: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AssistantConfigUpdate {
    pub agent: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ApprovalRejectRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS assistant_approvals (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            assistant_task_id INTEGER NOT NULL,
            target_task_id INTEGER,
            action_kind TEXT NOT NULL,
            summary TEXT NOT NULL,
            payload_json TEXT NOT NULL DEFAULT '{}',
            status TEXT NOT NULL DEFAULT 'pending',
            created_at INTEGER NOT NULL,
            resolved_at INTEGER,
            error TEXT
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_assistant_approvals_status
         ON assistant_approvals(status, created_at DESC)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// 단일 상시 비서 세션을 조회하거나 없으면 생성한다(멱등).
pub async fn ensure_assistant_task(
    pool: &SqlitePool,
    config: &RunnerConfig,
    now: i64,
) -> Result<db::Task, String> {
    migrate(pool).await.map_err(|error| error.to_string())?;
    if let Some(existing) = db::get_assistant_task(pool)
        .await
        .map_err(|error| error.to_string())?
    {
        return Ok(existing);
    }

    let workspace = config
        .assistant_workspace
        .clone()
        .or_else(|| config.repository_roots.first().cloned())
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    let workspace_str = workspace.to_string_lossy().into_owned();
    let _ = super::orchestrator_mcp::sync_assistant_mcp_config_at(&workspace, config);

    let agent = if matches!(
        config.assistant_default_agent.as_str(),
        "claude" | "codex" | "agy"
    ) {
        config.assistant_default_agent.clone()
    } else {
        "claude".to_string()
    };

    let insert = sqlx::query(
        "INSERT OR IGNORE INTO tasks (
            repo, branch, base, worktree_path, instruction, state,
            created_at, updated_at, agent, role, mode, kind
        ) VALUES (?, '(direct)', '(direct)', ?, '상시 비서 세션', ?, ?, ?, ?, 'implementer', 'conversation', ?)",
    )
    .bind(&workspace_str)
    .bind(&workspace_str)
    .bind(db::state::AWAITING_REVIEW)
    .bind(now)
    .bind(now)
    .bind(&agent)
    .bind(db::task_kind::ASSISTANT)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;

    let _ = insert;
    db::get_assistant_task(pool)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "상시 비서 세션을 초기화하지 못했습니다".to_string())
}

pub async fn overview(pool: &SqlitePool) -> Result<OrchestratorOverview, String> {
    migrate(pool).await.map_err(|error| error.to_string())?;
    let rows: Vec<(String, Option<String>, i64)> = sqlx::query_as(
        "SELECT state, awaiting_kind, COUNT(*) FROM tasks
         WHERE kind = 'coding'
         GROUP BY state, awaiting_kind",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;

    let mut summary = OrchestratorOverview::default();
    for (state, awaiting_kind, count) in rows {
        match state.as_str() {
            db::state::RUNNING | db::state::STARTING => summary.running_count += count,
            db::state::QUEUED => summary.queued_count += count,
            db::state::AWAITING_REVIEW => {
                summary.awaiting_review_count += count;
                if awaiting_kind.as_deref() == Some(db::awaiting_kind::QUESTION) {
                    summary.question_count += count;
                }
            }
            db::state::PENDING_APPROVAL => summary.pending_approval_count += count,
            db::state::FAILED => summary.failed_count += count,
            _ => {}
        }
    }

    let assistant_pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM assistant_approvals WHERE status = 'pending'",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);
    summary.pending_approval_count += assistant_pending;

    Ok(summary)
}

pub async fn snapshot(
    pool: &SqlitePool,
    config: &RunnerConfig,
    now: i64,
) -> Result<AssistantSnapshot, String> {
    let task = ensure_assistant_task(pool, config, now).await?;
    let pending_approvals = list_approvals(pool, Some("pending")).await?;
    let overview = overview(pool).await?;
    Ok(AssistantSnapshot {
        task,
        pending_approvals,
        overview,
    })
}

/// 상시 비서의 벤더·모델·추론 강도를 변경한다. 벤더가 바뀌면 직전 대화 요약을
/// `pending_capsule`에 저장하고 `convo_session_id`를 비워 새 벤더가 맥락을 이어받게 한다.
pub async fn update_config(
    pool: &SqlitePool,
    config: &RunnerConfig,
    update: AssistantConfigUpdate,
    now: i64,
) -> Result<db::Task, ApiError> {
    let agent = update.agent.trim();
    if !matches!(agent, "claude" | "codex" | "agy") {
        return Err((
            StatusCode::BAD_REQUEST,
            "지원하지 않는 비서 벤더입니다 (claude, codex, agy만 허용)".to_string(),
        ));
    }

    let current = ensure_assistant_task(pool, config, now)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    if current.state == db::state::RUNNING
        || current.state == db::state::STARTING
        || current.state == db::state::QUEUED
    {
        return Err((
            StatusCode::CONFLICT,
            "비서 턴이 진행 중일 때는 벤더·모델을 변경할 수 없습니다".to_string(),
        ));
    }

    let model = update
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let reasoning_effort = update
        .reasoning_effort
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let prev_agent = current.agent.as_deref().unwrap_or("claude");
    if prev_agent != agent {
        let capsule = build_handoff_capsule(pool, current.id, prev_agent, agent)
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
        sqlx::query(
            "UPDATE tasks
             SET agent = ?, model = ?, reasoning_effort = ?,
                 convo_session_id = NULL, pending_capsule = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(agent)
        .bind(&model)
        .bind(&reasoning_effort)
        .bind(&capsule)
        .bind(now)
        .bind(current.id)
        .execute(pool)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    } else {
        sqlx::query(
            "UPDATE tasks
             SET model = ?, reasoning_effort = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&model)
        .bind(&reasoning_effort)
        .bind(now)
        .bind(current.id)
        .execute(pool)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    }

    db::get_task(pool, current.id)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "비서 세션을 찾을 수 없습니다".to_string()))
}

async fn build_handoff_capsule(
    pool: &SqlitePool,
    task_id: i64,
    from_agent: &str,
    to_agent: &str,
) -> Result<Option<String>, String> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT event FROM convo_events
         WHERE task_id = ? AND rewound_at IS NULL
         ORDER BY id DESC
         LIMIT 24",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;

    let mut lines = Vec::new();
    for (raw,) in rows.into_iter().rev() {
        let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let kind = val.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "user" => {
                if let Some(text) = val.get("text").and_then(|v| v.as_str()) {
                    let snippet: String = text.chars().take(240).collect();
                    lines.push(format!("- 사용자: {snippet}"));
                }
            }
            "text" | "result" => {
                if val.get("parent_id").is_some() && !val["parent_id"].is_null() {
                    continue;
                }
                if let Some(text) = val.get("text").and_then(|v| v.as_str()) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        let snippet: String = trimmed.chars().take(240).collect();
                        lines.push(format!("- 비서({from_agent}): {snippet}"));
                    }
                }
            }
            _ => {}
        }
    }

    if lines.is_empty() {
        return Ok(Some(format!(
            "[벤더 전환 인계: 직전 비서 벤더 `{from_agent}`에서 `{to_agent}`로 전환되었습니다.]"
        )));
    }
    let tail: Vec<String> = lines
        .into_iter()
        .rev()
        .take(10)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Ok(Some(format!(
        "[벤더 전환 인계: 직전 비서 벤더 `{from_agent}`에서 `{to_agent}`로 전환되었습니다. 최근 대화 맥락:]\n{}",
        tail.join("\n")
    )))
}

pub async fn list_approvals(
    pool: &SqlitePool,
    status: Option<&str>,
) -> Result<Vec<AssistantApproval>, String> {
    migrate(pool).await.map_err(|error| error.to_string())?;
    let rows = match status {
        Some(filter) => {
            sqlx::query_as::<_, AssistantApproval>(
                "SELECT id, assistant_task_id, target_task_id, action_kind, summary,
                        payload_json, status, created_at, resolved_at, error
                 FROM assistant_approvals
                 WHERE status = ?
                 ORDER BY created_at DESC, id DESC
                 LIMIT 50",
            )
            .bind(filter)
            .fetch_all(pool)
            .await
        }
        None => {
            sqlx::query_as::<_, AssistantApproval>(
                "SELECT id, assistant_task_id, target_task_id, action_kind, summary,
                        payload_json, status, created_at, resolved_at, error
                 FROM assistant_approvals
                 ORDER BY created_at DESC, id DESC
                 LIMIT 50",
            )
            .fetch_all(pool)
            .await
        }
    };
    rows.map_err(|error| error.to_string())
}

pub async fn create_approval(
    pool: &SqlitePool,
    assistant_task_id: i64,
    target_task_id: Option<i64>,
    action_kind: &str,
    summary: &str,
    payload_json: &str,
    now: i64,
) -> Result<AssistantApproval, String> {
    migrate(pool).await.map_err(|error| error.to_string())?;
    if !matches!(
        action_kind,
        "approve_task" | "discard_task" | "run_pending_task" | "custom_command"
    ) {
        return Err(format!("지원하지 않는 action_kind입니다: {action_kind}"));
    }
    let summary = summary.trim();
    if summary.is_empty() {
        return Err("승인 요약(summary)은 비어 있을 수 없습니다".to_string());
    }

    let id = sqlx::query(
        "INSERT INTO assistant_approvals (
            assistant_task_id, target_task_id, action_kind, summary,
            payload_json, status, created_at
        ) VALUES (?, ?, ?, ?, ?, 'pending', ?)",
    )
    .bind(assistant_task_id)
    .bind(target_task_id)
    .bind(action_kind)
    .bind(summary)
    .bind(payload_json)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?
    .last_insert_rowid();

    let _ = db::append_runner_event(
        pool,
        assistant_task_id,
        now,
        db::runner_event_kind::ASSISTANT_APPROVAL_REQUESTED,
        Some(summary),
    )
    .await;

    get_approval(pool, id)
        .await?
        .ok_or_else(|| "생성된 승인 요청을 찾을 수 없습니다".to_string())
}

pub async fn get_approval(
    pool: &SqlitePool,
    id: i64,
) -> Result<Option<AssistantApproval>, String> {
    migrate(pool).await.map_err(|error| error.to_string())?;
    sqlx::query_as::<_, AssistantApproval>(
        "SELECT id, assistant_task_id, target_task_id, action_kind, summary,
                payload_json, status, created_at, resolved_at, error
         FROM assistant_approvals
         WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())
}

pub async fn approve_request(
    cx: &RunnerHttpState,
    actions: &SharedTaskActions,
    approval_id: i64,
    now: i64,
) -> Result<AssistantApproval, ApiError> {
    let existing = get_approval(&cx.pool, approval_id)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "승인 요청을 찾을 수 없습니다".to_string()))?;

    let claimed = sqlx::query(
        "UPDATE assistant_approvals
         SET status = 'approved', resolved_at = ?
         WHERE id = ? AND status = 'pending'",
    )
    .bind(now)
    .bind(approval_id)
    .execute(&cx.pool)
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    if claimed.rows_affected() == 0 {
        return Err((
            StatusCode::CONFLICT,
            format!("이미 처리된 승인 요청입니다 (상태: {})", existing.status),
        ));
    }

    let execution: Result<(), ApiError> = match existing.action_kind.as_str() {
        "approve_task" => {
            let target_id = existing.target_task_id.ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    "approve_task에는 target_task_id가 필요합니다".to_string(),
                )
            })?;
            actions.approve(cx, target_id).await
        }
        "discard_task" => {
            let target_id = existing.target_task_id.ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    "discard_task에는 target_task_id가 필요합니다".to_string(),
                )
            })?;
            actions.discard(cx, target_id).await
        }
        "run_pending_task" => {
            let target_id = existing.target_task_id.ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    "run_pending_task에는 target_task_id가 필요합니다".to_string(),
                )
            })?;
            actions.run_approve(cx, target_id).await.map(|_| ())
        }
        "custom_command" => Ok(()),
        other => Err((
            StatusCode::BAD_REQUEST,
            format!("지원하지 않는 action_kind입니다: {other}"),
        )),
    };

    match execution {
        Ok(()) => {
            let _ = sqlx::query(
                "UPDATE assistant_approvals
                 SET status = 'executed', resolved_at = ?, error = NULL
                 WHERE id = ?",
            )
            .bind(now)
            .bind(approval_id)
            .execute(&cx.pool)
            .await;

            let note = serde_json::json!({
                "kind": "text",
                "text": format!("[승인 실행 완료] {}", existing.summary),
            })
            .to_string();
            let _ = db::append_convo_event_with_runner_output(
                &cx.pool,
                existing.assistant_task_id,
                &note,
                now,
            )
            .await;
        }
        Err((code, message)) => {
            let _ = sqlx::query(
                "UPDATE assistant_approvals
                 SET status = 'failed', resolved_at = ?, error = ?
                 WHERE id = ?",
            )
            .bind(now)
            .bind(&message)
            .bind(approval_id)
            .execute(&cx.pool)
            .await;
            return Err((code, message));
        }
    }

    get_approval(&cx.pool, approval_id)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "승인 요청을 찾을 수 없습니다".to_string()))
}

pub async fn reject_request(
    pool: &SqlitePool,
    approval_id: i64,
    reason: Option<&str>,
    now: i64,
) -> Result<AssistantApproval, ApiError> {
    let existing = get_approval(pool, approval_id)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "승인 요청을 찾을 수 없습니다".to_string()))?;

    let reason_trimmed = reason.map(str::trim).filter(|s| !s.is_empty());
    let updated = sqlx::query(
        "UPDATE assistant_approvals
         SET status = 'rejected', resolved_at = ?, error = ?
         WHERE id = ? AND status = 'pending'",
    )
    .bind(now)
    .bind(reason_trimmed)
    .bind(approval_id)
    .execute(pool)
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    if updated.rows_affected() == 0 {
        return Err((
            StatusCode::CONFLICT,
            format!("이미 처리된 승인 요청입니다 (상태: {})", existing.status),
        ));
    }

    let note_text = match reason_trimmed {
        Some(r) => format!("[승인 거절] {} (사유: {r})", existing.summary),
        None => format!("[승인 거절] {}", existing.summary),
    };
    let note = serde_json::json!({
        "kind": "text",
        "text": note_text,
    })
    .to_string();
    let _ = db::append_convo_event_with_runner_output(
        pool,
        existing.assistant_task_id,
        &note,
        now,
    )
    .await;

    get_approval(pool, approval_id)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "승인 요청을 찾을 수 없습니다".to_string()))
}
