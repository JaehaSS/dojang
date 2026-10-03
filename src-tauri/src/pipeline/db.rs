//! 파이프라인 영속 계층(SQLite). 상태 변경은 모두 expected-state CAS이며 허용 전이만 통과한다.

use super::model::TicketDraft;
use super::state::{run_transition_allowed, ticket_transition_allowed, RunState, TicketState};
use anyhow::{anyhow, Context};
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use std::str::FromStr;

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct RunRow {
    pub id: i64,
    pub repo: String,
    pub base_branch: String,
    pub goal: String,
    pub state: String,
    pub paused_from: Option<String>,
    pub paused_reason: Option<String>,
    pub integration_task_id: Option<i64>,
    pub integration_branch: Option<String>,
    pub integration_path: Option<String>,
    pub spec_json: Option<String>,
    pub spec_revision: i64,
    pub plan_review_id: Option<i64>,
    pub final_review_id: Option<i64>,
    pub auto_fix_used: i64,
    pub plan_fix_used: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// 계획 거절·계획 리뷰 fix가 남긴, 다음 분할이 소비할 피드백.
    #[sqlx(default)]
    pub feedback: Option<String>,
}

impl RunRow {
    pub fn run_state(&self) -> Option<RunState> {
        RunState::from_str(&self.state).ok()
    }
}

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct TicketRow {
    pub id: i64,
    pub run_id: i64,
    pub key: String,
    pub title: String,
    pub body: String,
    pub vendor: String,
    pub reviewer_vendor: Option<String>,
    pub acceptance_json: String,
    pub allowed_paths_json: String,
    pub deps_json: String,
    pub covers_json: String,
    pub state: String,
    pub task_id: Option<i64>,
    pub attempt: i64,
    pub reassigned: i64,
    pub last_error: Option<String>,
    pub updated_at: i64,
    /// 에이전트를 새로 띄울 때마다(재시도·재배정) 오르는 세대. 재시작 멱등 step key가 겹치지 않게 한다.
    #[sqlx(default)]
    pub gen: i64,
    /// 이 티켓 worktree를 만들 때의 통합 브랜치 head. 변경 경로 검사의 fork point다.
    #[sqlx(default)]
    pub spawn_head: Option<String>,
}

impl TicketRow {
    pub fn ticket_state(&self) -> Option<TicketState> {
        TicketState::from_str(&self.state).ok()
    }
    pub fn deps(&self) -> Vec<String> {
        serde_json::from_str(&self.deps_json).unwrap_or_default()
    }
    pub fn acceptance(&self) -> Vec<String> {
        serde_json::from_str(&self.acceptance_json).unwrap_or_default()
    }
    pub fn allowed_paths(&self) -> Vec<String> {
        serde_json::from_str(&self.allowed_paths_json).unwrap_or_default()
    }
    /// 스케줄러용 투영. 상태가 해석되지 않으면 None.
    pub fn view(&self) -> Option<super::model::TicketView> {
        Some(super::model::TicketView {
            id: self.id,
            key: self.key.clone(),
            state: self.ticket_state()?,
            deps: self.deps(),
            vendor: self.vendor.clone(),
            reviewer_vendor: self.reviewer_vendor.clone(),
            attempt: self.attempt.max(0) as u32,
            reassigned: self.reassigned != 0,
        })
    }
}

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct StepRow {
    pub id: i64,
    pub run_id: i64,
    pub ticket_id: Option<i64>,
    pub kind: String,
    pub vendor: Option<String>,
    pub step_key: String,
    pub status: String,
    pub prompt: Option<String>,
    pub output: Option<String>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Clone, Debug)]
pub enum StepStart {
    AlreadySucceeded(StepRow),
    Started(i64),
}

pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    for stmt in [
        "CREATE TABLE IF NOT EXISTS pipeline_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            repo TEXT NOT NULL,
            base_branch TEXT NOT NULL,
            goal TEXT NOT NULL,
            state TEXT NOT NULL,
            paused_from TEXT,
            paused_reason TEXT,
            integration_task_id INTEGER,
            integration_branch TEXT,
            integration_path TEXT,
            spec_json TEXT,
            spec_revision INTEGER NOT NULL DEFAULT 0,
            plan_review_id INTEGER,
            final_review_id INTEGER,
            auto_fix_used INTEGER NOT NULL DEFAULT 0,
            plan_fix_used INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )",
        "CREATE INDEX IF NOT EXISTS idx_pipeline_runs_repo ON pipeline_runs(repo, state)",
        "CREATE TABLE IF NOT EXISTS pipeline_tickets (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id INTEGER NOT NULL,
            key TEXT NOT NULL,
            title TEXT NOT NULL,
            body TEXT NOT NULL DEFAULT '',
            vendor TEXT NOT NULL,
            reviewer_vendor TEXT,
            acceptance_json TEXT NOT NULL DEFAULT '[]',
            allowed_paths_json TEXT NOT NULL DEFAULT '[]',
            deps_json TEXT NOT NULL DEFAULT '[]',
            covers_json TEXT NOT NULL DEFAULT '[]',
            state TEXT NOT NULL,
            task_id INTEGER,
            attempt INTEGER NOT NULL DEFAULT 0,
            reassigned INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            updated_at INTEGER NOT NULL,
            UNIQUE(run_id, key)
        )",
        "CREATE TABLE IF NOT EXISTS pipeline_steps (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id INTEGER NOT NULL,
            ticket_id INTEGER,
            kind TEXT NOT NULL,
            vendor TEXT,
            step_key TEXT NOT NULL UNIQUE,
            status TEXT NOT NULL,
            prompt TEXT,
            output TEXT,
            started_at INTEGER NOT NULL,
            finished_at INTEGER
        )",
        "CREATE INDEX IF NOT EXISTS idx_pipeline_steps_run ON pipeline_steps(run_id)",
    ] {
        sqlx::query(stmt)
            .execute(pool)
            .await
            .context("pipeline 스키마 생성 실패")?;
    }
    // 이후에 추가된 컬럼. 이미 있으면 실패하므로 결과를 무시한다(멱등).
    for stmt in [
        "ALTER TABLE pipeline_runs ADD COLUMN feedback TEXT",
        "ALTER TABLE pipeline_tickets ADD COLUMN gen INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE pipeline_tickets ADD COLUMN spawn_head TEXT",
    ] {
        let _ = sqlx::query(stmt).execute(pool).await;
    }
    Ok(())
}

// ---------- runs ----------

pub async fn insert_run(
    pool: &SqlitePool,
    repo: &str,
    base_branch: &str,
    goal: &str,
    now: i64,
) -> anyhow::Result<i64> {
    let r = sqlx::query(
        "INSERT INTO pipeline_runs(repo, base_branch, goal, state, created_at, updated_at)
         VALUES (?, ?, ?, 'drafting', ?, ?)",
    )
    .bind(repo)
    .bind(base_branch)
    .bind(goal)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(r.last_insert_rowid())
}

pub async fn get_run(pool: &SqlitePool, id: i64) -> anyhow::Result<Option<RunRow>> {
    Ok(sqlx::query_as("SELECT * FROM pipeline_runs WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?)
}

pub async fn list_runs(pool: &SqlitePool, repo: Option<&str>) -> anyhow::Result<Vec<RunRow>> {
    Ok(match repo {
        Some(r) => {
            sqlx::query_as("SELECT * FROM pipeline_runs WHERE repo = ? ORDER BY id DESC")
                .bind(r)
                .fetch_all(pool)
                .await?
        }
        None => {
            sqlx::query_as("SELECT * FROM pipeline_runs ORDER BY id DESC")
                .fetch_all(pool)
                .await?
        }
    })
}

const ACTIVE_FILTER: &str = "state NOT IN ('done','cancelled','failed')";

pub async fn active_run_for_repo(pool: &SqlitePool, repo: &str) -> anyhow::Result<Option<RunRow>> {
    Ok(sqlx::query_as(&format!(
        "SELECT * FROM pipeline_runs WHERE repo = ? AND {ACTIVE_FILTER} ORDER BY id DESC LIMIT 1"
    ))
    .bind(repo)
    .fetch_optional(pool)
    .await?)
}

pub async fn list_active_runs(pool: &SqlitePool) -> anyhow::Result<Vec<RunRow>> {
    Ok(sqlx::query_as(&format!(
        "SELECT * FROM pipeline_runs WHERE {ACTIVE_FILTER} ORDER BY id"
    ))
    .fetch_all(pool)
    .await?)
}

/// CAS 전이. 허용되지 않은 전이는 Err, expected 불일치는 Ok(false).
/// Paused로 갈 때는 `paused_from`에 expected를 저장한다.
pub async fn set_run_state(
    pool: &SqlitePool,
    id: i64,
    expected: RunState,
    next: RunState,
    now: i64,
) -> anyhow::Result<bool> {
    if !run_transition_allowed(expected, next) {
        return Err(anyhow!(
            "허용되지 않는 실행 상태 전이: {} -> {}",
            expected.as_str(),
            next.as_str()
        ));
    }
    let paused_from = (next == RunState::Paused).then(|| expected.as_str());
    let r = sqlx::query(
        "UPDATE pipeline_runs SET state = ?, updated_at = ?,
            paused_from = CASE WHEN ? = 'paused' THEN ? ELSE NULL END,
            paused_reason = CASE WHEN ? = 'paused' THEN paused_reason ELSE NULL END
         WHERE id = ? AND state = ?",
    )
    .bind(next.as_str())
    .bind(now)
    .bind(next.as_str())
    .bind(paused_from)
    .bind(next.as_str())
    .bind(id)
    .bind(expected.as_str())
    .execute(pool)
    .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn pause_run(
    pool: &SqlitePool,
    id: i64,
    expected: RunState,
    reason: &str,
    now: i64,
) -> anyhow::Result<bool> {
    if !set_run_state(pool, id, expected, RunState::Paused, now).await? {
        return Ok(false);
    }
    sqlx::query("UPDATE pipeline_runs SET paused_reason = ? WHERE id = ?")
        .bind(reason)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(true)
}

/// Paused인 실행을 멈춘 지점으로 되돌린다. 성공 시 복귀한 상태를 돌려준다.
pub async fn resume_run(pool: &SqlitePool, id: i64, now: i64) -> anyhow::Result<Option<RunState>> {
    let Some(run) = get_run(pool, id).await? else {
        return Ok(None);
    };
    if run.run_state() != Some(RunState::Paused) {
        return Ok(None);
    }
    let Some(from) = run
        .paused_from
        .as_deref()
        .and_then(|s| RunState::from_str(s).ok())
    else {
        return Ok(None);
    };
    if set_run_state(pool, id, RunState::Paused, from, now).await? {
        Ok(Some(from))
    } else {
        Ok(None)
    }
}

/// 스펙을 저장하고 revision을 1 올린다.
pub async fn set_run_spec(
    pool: &SqlitePool,
    id: i64,
    spec_json: &str,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE pipeline_runs SET spec_json = ?, spec_revision = spec_revision + 1, updated_at = ? WHERE id = ?",
    )
    .bind(spec_json)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_run_integration(
    pool: &SqlitePool,
    id: i64,
    task_id: i64,
    branch: &str,
    path: &str,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE pipeline_runs SET integration_task_id = ?, integration_branch = ?, integration_path = ?, updated_at = ? WHERE id = ?",
    )
    .bind(task_id)
    .bind(branch)
    .bind(path)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 초안 단계의 실행에 통합 작업을 연결한다. 그 사이 취소·실패로 상태가 바뀌었거나 이미 연결돼 있으면 Ok(false).
pub async fn claim_run_integration(
    pool: &SqlitePool,
    id: i64,
    task_id: i64,
    branch: &str,
    path: &str,
    now: i64,
) -> anyhow::Result<bool> {
    let r = sqlx::query(
        "UPDATE pipeline_runs SET integration_task_id = ?, integration_branch = ?, integration_path = ?, updated_at = ?
         WHERE id = ? AND state = 'drafting' AND integration_task_id IS NULL",
    )
    .bind(task_id)
    .bind(branch)
    .bind(path)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn set_run_plan_review(
    pool: &SqlitePool,
    id: i64,
    review_id: i64,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE pipeline_runs SET plan_review_id = ?, updated_at = ? WHERE id = ?")
        .bind(review_id)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_run_final_review(
    pool: &SqlitePool,
    id: i64,
    review_id: i64,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE pipeline_runs SET final_review_id = ?, updated_at = ? WHERE id = ?")
        .bind(review_id)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_run_auto_fix_used(
    pool: &SqlitePool,
    id: i64,
    used: bool,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE pipeline_runs SET auto_fix_used = ?, updated_at = ? WHERE id = ?")
        .bind(used as i64)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_run_plan_fix_used(
    pool: &SqlitePool,
    id: i64,
    used: bool,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE pipeline_runs SET plan_fix_used = ?, updated_at = ? WHERE id = ?")
        .bind(used as i64)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_run_error(
    pool: &SqlitePool,
    id: i64,
    err: Option<&str>,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE pipeline_runs SET last_error = ?, updated_at = ? WHERE id = ?")
        .bind(err)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_run_feedback(
    pool: &SqlitePool,
    id: i64,
    feedback: Option<&str>,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE pipeline_runs SET feedback = ?, updated_at = ? WHERE id = ?")
        .bind(feedback)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 분할 결과를 한 트랜잭션으로 확정한다: 스펙 저장·revision 증가·피드백 소비·티켓 교체.
/// 중간에 죽어도 "스펙은 새것인데 티켓은 옛것"인 상태가 남지 않는다.
pub async fn commit_split(
    pool: &SqlitePool,
    run_id: i64,
    spec_json: &str,
    drafts: &[TicketDraft],
    now: i64,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE pipeline_runs SET spec_json = ?, spec_revision = spec_revision + 1,
            feedback = NULL, updated_at = ? WHERE id = ?",
    )
    .bind(spec_json)
    .bind(now)
    .bind(run_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM pipeline_tickets WHERE run_id = ?")
        .bind(run_id)
        .execute(&mut *tx)
        .await?;
    insert_ticket_drafts(&mut tx, run_id, drafts, now).await?;
    tx.commit().await?;
    Ok(())
}

/// 계획 승인 대기 중 사용자가 티켓을 고친다. `expected_revision`이 맞을 때만 교체하고 revision을 올린다.
/// 거절(revision 불일치 또는 상태 불일치)은 Ok(false).
pub async fn update_tickets_cas(
    pool: &SqlitePool,
    run_id: i64,
    expected_revision: i64,
    drafts: &[TicketDraft],
    now: i64,
) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await?;
    let r = sqlx::query(
        "UPDATE pipeline_runs SET spec_revision = spec_revision + 1, updated_at = ?
         WHERE id = ? AND state = 'awaiting_plan_approval' AND spec_revision = ?",
    )
    .bind(now)
    .bind(run_id)
    .bind(expected_revision)
    .execute(&mut *tx)
    .await?;
    if r.rows_affected() != 1 {
        return Ok(false);
    }
    sqlx::query("DELETE FROM pipeline_tickets WHERE run_id = ?")
        .bind(run_id)
        .execute(&mut *tx)
        .await?;
    insert_ticket_drafts(&mut tx, run_id, drafts, now).await?;
    tx.commit().await?;
    Ok(true)
}

/// 계획 승인: revision이 일치할 때만 awaiting_plan_approval → executing.
pub async fn approve_plan_cas(
    pool: &SqlitePool,
    run_id: i64,
    expected_revision: i64,
    now: i64,
) -> anyhow::Result<bool> {
    let r = sqlx::query(
        "UPDATE pipeline_runs SET state = 'executing', updated_at = ?, paused_from = NULL, paused_reason = NULL
         WHERE id = ? AND state = 'awaiting_plan_approval' AND spec_revision = ?",
    )
    .bind(now)
    .bind(run_id)
    .bind(expected_revision)
    .execute(pool)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// 계획 거절: 코멘트를 피드백으로 남기고 awaiting_plan_approval → drafting.
pub async fn reject_plan(
    pool: &SqlitePool,
    run_id: i64,
    comment: &str,
    now: i64,
) -> anyhow::Result<bool> {
    let r = sqlx::query(
        "UPDATE pipeline_runs SET state = 'drafting', feedback = ?, updated_at = ?
         WHERE id = ? AND state = 'awaiting_plan_approval'",
    )
    .bind(comment)
    .bind(now)
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// 비종결 실행을 취소한다. 이미 종결이면 Ok(false).
pub async fn cancel_run(pool: &SqlitePool, run_id: i64, now: i64) -> anyhow::Result<bool> {
    let r = sqlx::query(
        "UPDATE pipeline_runs SET state = 'cancelled', updated_at = ?
         WHERE id = ? AND state NOT IN ('done', 'cancelled', 'failed')",
    )
    .bind(now)
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(r.rows_affected() == 1)
}

// ---------- tickets ----------

async fn insert_ticket_drafts(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    run_id: i64,
    drafts: &[TicketDraft],
    now: i64,
) -> anyhow::Result<()> {
    for d in drafts {
        sqlx::query(
            "INSERT INTO pipeline_tickets(run_id, key, title, body, vendor, acceptance_json,
                allowed_paths_json, deps_json, covers_json, state, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?)",
        )
        .bind(run_id)
        .bind(&d.key)
        .bind(&d.title)
        .bind(&d.body)
        .bind(&d.vendor)
        .bind(json_of(&d.acceptance_commands)?)
        .bind(json_of(&d.allowed_paths)?)
        .bind(json_of(&d.deps)?)
        .bind(json_of(&d.covers)?)
        .bind(now)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

fn json_of<T: Serialize>(v: &T) -> anyhow::Result<String> {
    Ok(serde_json::to_string(v)?)
}

/// 실행의 티켓을 전부 지우고 새로 넣는다(트랜잭션). 재분할·사용자 편집 저장에 쓴다.
pub async fn replace_tickets(
    pool: &SqlitePool,
    run_id: i64,
    drafts: &[TicketDraft],
    now: i64,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM pipeline_tickets WHERE run_id = ?")
        .bind(run_id)
        .execute(&mut *tx)
        .await?;
    insert_ticket_drafts(&mut tx, run_id, drafts, now).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn list_tickets(pool: &SqlitePool, run_id: i64) -> anyhow::Result<Vec<TicketRow>> {
    Ok(
        sqlx::query_as("SELECT * FROM pipeline_tickets WHERE run_id = ? ORDER BY key")
            .bind(run_id)
            .fetch_all(pool)
            .await?,
    )
}

pub async fn get_ticket(pool: &SqlitePool, id: i64) -> anyhow::Result<Option<TicketRow>> {
    Ok(
        sqlx::query_as("SELECT * FROM pipeline_tickets WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

/// 티켓 CAS 전이. 허용되지 않은 전이는 Err, expected 불일치는 Ok(false).
pub async fn set_ticket_state(
    pool: &SqlitePool,
    id: i64,
    expected: TicketState,
    next: TicketState,
    now: i64,
) -> anyhow::Result<bool> {
    if !ticket_transition_allowed(expected, next) {
        return Err(anyhow!(
            "허용되지 않는 티켓 상태 전이: {} -> {}",
            expected.as_str(),
            next.as_str()
        ));
    }
    let r = sqlx::query(
        "UPDATE pipeline_tickets SET state = ?, updated_at = ? WHERE id = ? AND state = ?",
    )
    .bind(next.as_str())
    .bind(now)
    .bind(id)
    .bind(expected.as_str())
    .execute(pool)
    .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn set_ticket_task(
    pool: &SqlitePool,
    id: i64,
    task_id: Option<i64>,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE pipeline_tickets SET task_id = ?, updated_at = ? WHERE id = ?")
        .bind(task_id)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_ticket_vendor(
    pool: &SqlitePool,
    id: i64,
    vendor: &str,
    reassigned: bool,
    attempt: u32,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE pipeline_tickets SET vendor = ?, reassigned = ?, attempt = ?, updated_at = ? WHERE id = ?",
    )
    .bind(vendor)
    .bind(reassigned as i64)
    .bind(attempt as i64)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_ticket_attempt(
    pool: &SqlitePool,
    id: i64,
    attempt: u32,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE pipeline_tickets SET attempt = ?, updated_at = ? WHERE id = ?")
        .bind(attempt as i64)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_ticket_reviewer(
    pool: &SqlitePool,
    id: i64,
    reviewer_vendor: Option<&str>,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE pipeline_tickets SET reviewer_vendor = ?, updated_at = ? WHERE id = ?")
        .bind(reviewer_vendor)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_ticket_spawn_head(
    pool: &SqlitePool,
    id: i64,
    head: Option<&str>,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE pipeline_tickets SET spawn_head = ?, updated_at = ? WHERE id = ?")
        .bind(head)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 에이전트를 새로 띄울 준비: 세대를 올리고 task·spawn_head를 비운다(상태는 호출자가 바꾼다).
pub async fn bump_ticket_gen(pool: &SqlitePool, id: i64, now: i64) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE pipeline_tickets SET gen = gen + 1, task_id = NULL, spawn_head = NULL, updated_at = ? WHERE id = ?",
    )
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 에스컬레이션된 티켓을 재시도 대기로 되돌린다: 시도·재배정 초기화, 오류 제거, 세대 증가.
/// `vendor`가 Some이면 벤더를 바꾼다. escalated가 아니면 Ok(false).
pub async fn retry_ticket(
    pool: &SqlitePool,
    id: i64,
    vendor: Option<&str>,
    now: i64,
) -> anyhow::Result<bool> {
    let r = sqlx::query(
        "UPDATE pipeline_tickets SET state = 'pending', attempt = 0, reassigned = 0, last_error = NULL,
            gen = gen + 1, task_id = NULL, spawn_head = NULL,
            vendor = COALESCE(?, vendor), updated_at = ?
         WHERE id = ? AND state = 'escalated'",
    )
    .bind(vendor)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// 에스컬레이션된 티켓을 건너뛴다(취소). 이 티켓에 (간접적으로) 의존하는 비종결 티켓도 함께 취소한다.
/// 취소된 티켓 id 전체(자신 포함)를 돌려준다. escalated가 아니면 빈 목록.
pub async fn skip_ticket_cascade(
    pool: &SqlitePool,
    run_id: i64,
    id: i64,
    now: i64,
) -> anyhow::Result<Vec<i64>> {
    let mut tx = pool.begin().await?;
    let r = sqlx::query(
        "UPDATE pipeline_tickets SET state = 'cancelled', updated_at = ? WHERE id = ? AND state = 'escalated'",
    )
    .bind(now)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if r.rows_affected() != 1 {
        return Ok(Vec::new());
    }
    let rows: Vec<TicketRow> = sqlx::query_as("SELECT * FROM pipeline_tickets WHERE run_id = ?")
        .bind(run_id)
        .fetch_all(&mut *tx)
        .await?;
    let mut cancelled_keys: Vec<String> = rows
        .iter()
        .filter(|t| t.id == id)
        .map(|t| t.key.clone())
        .collect();
    let mut cancelled_ids = vec![id];
    loop {
        let next: Vec<&TicketRow> = rows
            .iter()
            .filter(|t| {
                !cancelled_ids.contains(&t.id)
                    && !matches!(t.state.as_str(), "integrated" | "cancelled")
                    && t.deps().iter().any(|d| cancelled_keys.contains(d))
            })
            .collect();
        if next.is_empty() {
            break;
        }
        for t in next {
            cancelled_ids.push(t.id);
            cancelled_keys.push(t.key.clone());
        }
    }
    for tid in cancelled_ids.iter().filter(|t| **t != id) {
        sqlx::query("UPDATE pipeline_tickets SET state = 'cancelled', updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(tid)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(cancelled_ids)
}

pub async fn set_ticket_error(
    pool: &SqlitePool,
    id: i64,
    err: Option<&str>,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE pipeline_tickets SET last_error = ?, updated_at = ? WHERE id = ?")
        .bind(err)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ---------- steps ----------

/// 단계 시작. 같은 key가 이미 성공했으면 재실행하지 않도록 알리고,
/// running·failed 행은 running으로 되돌려 재사용한다(재시작 멱등).
#[allow(clippy::too_many_arguments)]
pub async fn begin_step(
    pool: &SqlitePool,
    run_id: i64,
    ticket_id: Option<i64>,
    kind: &str,
    vendor: Option<&str>,
    step_key: &str,
    prompt: &str,
    now: i64,
) -> anyhow::Result<StepStart> {
    let mut tx = pool.begin().await?;
    let existing: Option<StepRow> =
        sqlx::query_as("SELECT * FROM pipeline_steps WHERE step_key = ?")
            .bind(step_key)
            .fetch_optional(&mut *tx)
            .await?;
    let out = match existing {
        Some(row) if row.status == "succeeded" => StepStart::AlreadySucceeded(row),
        Some(row) => {
            sqlx::query(
                "UPDATE pipeline_steps SET status = 'running', vendor = ?, prompt = ?, output = NULL,
                    started_at = ?, finished_at = NULL WHERE id = ?",
            )
            .bind(vendor)
            .bind(prompt)
            .bind(now)
            .bind(row.id)
            .execute(&mut *tx)
            .await?;
            StepStart::Started(row.id)
        }
        None => {
            let r = sqlx::query(
                "INSERT INTO pipeline_steps(run_id, ticket_id, kind, vendor, step_key, status, prompt, started_at)
                 VALUES (?, ?, ?, ?, ?, 'running', ?, ?)",
            )
            .bind(run_id)
            .bind(ticket_id)
            .bind(kind)
            .bind(vendor)
            .bind(step_key)
            .bind(prompt)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            StepStart::Started(r.last_insert_rowid())
        }
    };
    tx.commit().await?;
    Ok(out)
}

pub async fn finish_step(
    pool: &SqlitePool,
    id: i64,
    ok: bool,
    output: &str,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE pipeline_steps SET status = ?, output = ?, finished_at = ? WHERE id = ?")
        .bind(if ok { "succeeded" } else { "failed" })
        .bind(output)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_steps(pool: &SqlitePool, run_id: i64) -> anyhow::Result<Vec<StepRow>> {
    Ok(
        sqlx::query_as("SELECT * FROM pipeline_steps WHERE run_id = ? ORDER BY id")
            .bind(run_id)
            .fetch_all(pool)
            .await?,
    )
}

/// 같은 실행에서 벤더별 리뷰어 배정 수(티켓 `reviewer_vendor` 기준).
pub async fn reviewer_counts(
    pool: &SqlitePool,
    run_id: i64,
) -> anyhow::Result<std::collections::HashMap<String, u32>> {
    let rows = sqlx::query(
        "SELECT reviewer_vendor AS v, COUNT(*) AS c FROM pipeline_tickets
         WHERE run_id = ? AND reviewer_vendor IS NOT NULL GROUP BY reviewer_vendor",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.get::<String, _>("v"), r.get::<i64, _>("c").max(0) as u32))
        .collect())
}

/// 일반 작업 UI(승인·폐기·충돌 해소)가 이 작업을 건드려도 되는지 판정한다. 막아야 하면 사용자에게 보일 사유를 돌려준다.
/// 파이프라인이 관리하는 티켓 작업은 실행이 끝나기 전까지, 통합 작업은 최종 승인 대기(G2) 밖에서 막는다.
/// 종결된 실행(완료·취소·실패)은 더 이상 드라이버가 만지지 않으므로 정리할 수 있게 풀어 준다.
pub const PIPELINE_MANAGED_MESSAGE: &str = "파이프라인이 관리하는 작업입니다. 파이프라인 화면에서 처리하세요";

pub async fn pipeline_guard_for_task(pool: &SqlitePool, task_id: i64) -> anyhow::Result<Option<String>> {
    let ticket_run: Option<String> = sqlx::query_scalar(
        "SELECT r.state FROM pipeline_tickets t JOIN pipeline_runs r ON r.id = t.run_id
         WHERE t.task_id = ? AND r.state NOT IN ('done', 'cancelled', 'failed') LIMIT 1",
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await?;
    if ticket_run.is_some() {
        return Ok(Some(PIPELINE_MANAGED_MESSAGE.into()));
    }
    let integration_run: Option<String> = sqlx::query_scalar(
        "SELECT state FROM pipeline_runs
         WHERE integration_task_id = ? AND state NOT IN ('done', 'cancelled', 'failed', 'awaiting_merge_approval') LIMIT 1",
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await?;
    Ok(integration_run.map(|_| PIPELINE_MANAGED_MESSAGE.into()))
}
