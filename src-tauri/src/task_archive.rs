//! Manual task-archive metadata.
//!
//! Archiving only hides an already settled task from the default list.  It
//! never changes the task, its worktree, or any of its durable history.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use tokio::io::AsyncReadExt;

use crate::db;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Archive {
    pub archived_at: i64,
    pub branch: String,
    pub head_revision: Option<String>,
    pub worktree_path: String,
    /// This is observed when the archive is read.  The archived snapshot does
    /// not claim that a worktree remains on disk forever.
    pub worktree_exists: bool,
    pub result_path: Option<String>,
    pub pr_url: Option<String>,
}

#[derive(FromRow)]
struct ArchiveRow {
    task_id: i64,
    archived_at: i64,
    branch: String,
    head_revision: Option<String>,
    worktree_path: String,
    result_path: Option<String>,
    pr_url: Option<String>,
}

impl From<ArchiveRow> for Archive {
    fn from(row: ArchiveRow) -> Self {
        Self {
            archived_at: row.archived_at,
            branch: row.branch,
            head_revision: row.head_revision,
            worktree_exists: Path::new(&row.worktree_path).is_dir(),
            worktree_path: row.worktree_path,
            result_path: row.result_path,
            pr_url: row.pr_url,
        }
    }
}

pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    // `side_question::migrate` normally runs in app setup, after `init_pool`.
    // Archive eligibility and its activity triggers also run in focused DB
    // tests, so their durable table must exist here as well.
    crate::side_question::migrate(pool).await?;
    let mut tx = pool.begin().await?;
    for statement in [
        "CREATE TABLE IF NOT EXISTS task_archives (\
           task_id INTEGER PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,\
           archived_at INTEGER NOT NULL,\
           branch TEXT NOT NULL,\
           head_revision TEXT,\
           worktree_path TEXT NOT NULL,\
           result_path TEXT,\
           pr_url TEXT)",
        // A transition back into a live state must make the task visible
        // without coordinating every executor with archive commands.
        "CREATE TRIGGER IF NOT EXISTS task_archives_unarchive_on_state_change \
           AFTER UPDATE OF state ON tasks WHEN OLD.state <> NEW.state BEGIN \
           DELETE FROM task_archives WHERE task_id = NEW.id; END",
        "CREATE TRIGGER IF NOT EXISTS task_archives_unarchive_on_review_lease \
           AFTER INSERT ON review_process_leases BEGIN \
           DELETE FROM task_archives WHERE task_id = NEW.task_id; END",
        "CREATE TRIGGER IF NOT EXISTS task_archives_unarchive_on_side_question_insert \
           AFTER INSERT ON side_question_turns \
           WHEN NEW.state IN ('queued', 'running', 'stopping') BEGIN \
           DELETE FROM task_archives WHERE task_id = NEW.task_id; END",
        "CREATE TRIGGER IF NOT EXISTS task_archives_unarchive_on_side_question_update \
           AFTER UPDATE OF state ON side_question_turns \
           WHEN NEW.state IN ('queued', 'running', 'stopping') AND OLD.state <> NEW.state BEGIN \
           DELETE FROM task_archives WHERE task_id = NEW.task_id; END",
        "CREATE TRIGGER IF NOT EXISTS task_archives_unarchive_on_convo_execution_insert \
           AFTER INSERT ON convo_executions \
           WHEN NEW.state IN ('starting', 'running', 'cancelling', 'finalizing', 'cleanup_failed') BEGIN \
           DELETE FROM task_archives WHERE task_id = NEW.task_id; END",
        "CREATE TRIGGER IF NOT EXISTS task_archives_unarchive_on_convo_execution_update \
           AFTER UPDATE OF state ON convo_executions \
           WHEN NEW.state IN ('starting', 'running', 'cancelling', 'finalizing', 'cleanup_failed') \
             AND OLD.state <> NEW.state BEGIN \
           DELETE FROM task_archives WHERE task_id = NEW.task_id; END",
        "CREATE TRIGGER IF NOT EXISTS task_archives_unarchive_on_review_delivery_insert \
           AFTER INSERT ON result_review_outbox \
           WHEN NEW.state IN ('queued', 'dispatching', 'delivery_unknown') BEGIN \
           DELETE FROM task_archives WHERE task_id = NEW.task_id; END",
        "CREATE TRIGGER IF NOT EXISTS task_archives_unarchive_on_review_delivery_update \
           AFTER UPDATE OF state ON result_review_outbox \
           WHEN NEW.state IN ('queued', 'dispatching', 'delivery_unknown') AND OLD.state <> NEW.state BEGIN \
           DELETE FROM task_archives WHERE task_id = NEW.task_id; END",
        "CREATE TRIGGER IF NOT EXISTS task_archives_unarchive_on_approval_insert \
           AFTER INSERT ON local_approval_finalizations WHEN NEW.state <> 'completed' BEGIN \
           DELETE FROM task_archives WHERE task_id = NEW.task_id; END",
        "CREATE TRIGGER IF NOT EXISTS task_archives_unarchive_on_approval_update \
           AFTER UPDATE OF state ON local_approval_finalizations \
           WHEN NEW.state <> 'completed' AND OLD.state <> NEW.state BEGIN \
           DELETE FROM task_archives WHERE task_id = NEW.task_id; END",
    ] {
        sqlx::query(statement).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Store an immutable archive snapshot.  The conditional INSERT is the
/// authority for eligibility so a state change between observation and write
/// cannot archive a live task.
pub async fn archive(pool: &SqlitePool, id: i64, now: i64) -> Result<Archive, String> {
    if let Some(archive) = one(pool, id).await? {
        return Ok(archive);
    }

    let task = db::get_task(pool, id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
    let head_revision = git_head(Path::new(&task.worktree_path)).await;
    let pr_url = stored_pr_url(pool, id).await?;
    let result_path = stored_result_path(pool, id, Path::new(&task.worktree_path)).await?;

    let inserted = sqlx::query(
        "INSERT INTO task_archives \
           (task_id, archived_at, branch, head_revision, worktree_path, result_path, pr_url) \
         SELECT t.id, ?, t.branch, ?, t.worktree_path, ?, ? FROM tasks t \
         WHERE t.id = ? AND t.branch = ? AND t.worktree_path = ? \
           AND t.state IN ('AwaitingReview', 'Done', 'Failed', 'Discarded') \
           AND t.convo_pgid IS NULL \
           AND NOT EXISTS (SELECT 1 FROM review_process_leases lease WHERE lease.task_id = t.id) \
           AND NOT EXISTS (SELECT 1 FROM result_review_outbox outbox \
                           WHERE outbox.task_id = t.id \
                             AND outbox.state IN ('queued', 'dispatching', 'delivery_unknown')) \
           AND NOT EXISTS (SELECT 1 FROM local_approval_finalizations approval \
                           WHERE approval.task_id = t.id AND approval.state <> 'completed') \
           AND NOT EXISTS (SELECT 1 FROM convo_executions execution \
                           WHERE execution.task_id = t.id \
                             AND execution.state IN ('starting', 'running', 'cancelling', 'finalizing', 'cleanup_failed')) \
           AND NOT EXISTS (SELECT 1 FROM side_question_turns question \
                           WHERE question.task_id = t.id \
                             AND question.state IN ('queued', 'running', 'stopping')) \
         ON CONFLICT(task_id) DO NOTHING",
    )
    .bind(now)
    .bind(&head_revision)
    .bind(&result_path)
    .bind(&pr_url)
    .bind(id)
    .bind(&task.branch)
    .bind(&task.worktree_path)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;

    if inserted.rows_affected() == 0 {
        return one(pool, id).await?.ok_or_else(|| {
            "실행 중이거나 검토·승인 처리가 남은 작업은 보관할 수 없습니다".to_string()
        });
    }
    one(pool, id)
        .await?
        .ok_or_else(|| "작업 보관 정보를 읽지 못했습니다".to_string())
}

pub async fn restore(pool: &SqlitePool, id: i64) -> Result<(), String> {
    sqlx::query("DELETE FROM task_archives WHERE task_id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// This narrow check is used by paths that already hold a task lifecycle
/// claim and must recheck before making an orphan-disposal decision.
pub async fn is_archived(pool: &SqlitePool, id: i64) -> Result<bool, String> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM task_archives WHERE task_id = ?)")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|error| error.to_string())
}

pub async fn list(pool: &SqlitePool) -> Result<HashMap<i64, Archive>, String> {
    let rows = sqlx::query_as::<_, ArchiveRow>(
        "SELECT task_id, archived_at, branch, head_revision, worktree_path, result_path, pr_url \
         FROM task_archives",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let id = row.task_id;
            (id, Archive::from(row))
        })
        .collect())
}

async fn one(pool: &SqlitePool, id: i64) -> Result<Option<Archive>, String> {
    sqlx::query_as::<_, ArchiveRow>(
        "SELECT task_id, archived_at, branch, head_revision, worktree_path, result_path, pr_url \
         FROM task_archives WHERE task_id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map(|row| row.map(Archive::from))
    .map_err(|error| error.to_string())
}

async fn stored_pr_url(pool: &SqlitePool, task_id: i64) -> Result<Option<String>, String> {
    let table_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'task_pr_refs')",
    )
    .fetch_one(pool)
    .await
    .map_err(|error| error.to_string())?;
    if !table_exists {
        return Ok(None);
    }
    sqlx::query_scalar("SELECT pr_url FROM task_pr_refs WHERE task_id = ?")
        .bind(task_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| error.to_string())
}

/// References are stored as normalized relative paths.  The path is useful as
/// an archive snapshot only when it remains safely below the task worktree.
async fn stored_result_path(
    pool: &SqlitePool,
    task_id: i64,
    worktree: &Path,
) -> Result<Option<String>, String> {
    let relative: Option<String> = sqlx::query_scalar(
        "SELECT relative_path FROM task_result_references WHERE task_id = ? \
         ORDER BY result_revision DESC, created_at DESC, rowid DESC LIMIT 1",
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?;
    let Some(relative) = relative else {
        return Ok(None);
    };
    let relative_path = Path::new(&relative);
    if !relative_path.is_relative()
        || relative_path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Ok(None);
    }
    Ok(Some(
        worktree.join(relative_path).to_string_lossy().into_owned(),
    ))
}

async fn git_head(worktree: &Path) -> Option<String> {
    if !worktree.is_dir() {
        return None;
    }
    let mut child = tokio::process::Command::new("git")
        .arg("-C")
        .arg(worktree)
        .args(["rev-parse", "HEAD"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let mut output = tokio::spawn(async move {
        let mut bytes = Vec::new();
        // A valid SHA plus its newline is at most 65 bytes.  Bound the reader
        // as well as the process lifetime so an unexpected executable cannot
        // turn archive metadata into an unbounded allocation.
        stdout
            .take(256)
            .read_to_end(&mut bytes)
            .await
            .map(|_| bytes)
    });
    let status = match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(_)) => {
            let _ = child.kill().await;
            output.abort();
            let _ = output.await;
            return None;
        }
        Err(_) => {
            let _ = child.kill().await;
            output.abort();
            let _ = output.await;
            return None;
        }
    };
    let output = match tokio::time::timeout(Duration::from_secs(1), &mut output).await {
        Ok(Ok(Ok(bytes))) => bytes,
        Ok(Ok(Err(_))) | Ok(Err(_)) | Err(_) => {
            output.abort();
            let _ = output.await;
            return None;
        }
    };
    if !status.success() {
        return None;
    }
    let revision = String::from_utf8(output).ok()?;
    let revision = revision.trim();
    if matches!(revision.len(), 40 | 64) && revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Some(revision.to_owned())
    } else {
        None
    }
}
