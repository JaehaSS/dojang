use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::db;

use super::provenance::{self, ApprovalProvenance};

pub async fn complete(pool: &SqlitePool, task_id: i64, now: i64) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    let task: Option<(String, String)> =
        sqlx::query_as("SELECT state, instruction FROM tasks WHERE id = ?")
            .bind(task_id)
            .fetch_optional(&mut *tx)
            .await?;
    let task = task.ok_or_else(|| anyhow::anyhow!("작업을 찾을 수 없습니다"))?;
    let journal: (String, Option<String>) = sqlx::query_as(
        "SELECT state, commit_sha FROM local_approval_finalizations WHERE task_id = ?",
    )
    .bind(task_id)
    .fetch_one(&mut *tx)
    .await?;
    if task.0 == db::state::DONE && journal.0 == "completed" {
        validate_completed(&mut tx, task_id).await?;
        tx.commit().await?;
        return Ok(());
    }
    if task.0 != db::state::FINALIZING || journal.0 != "cleaned" {
        anyhow::bail!("local approval is not ready for ledger completion");
    }
    let commit = journal
        .1
        .ok_or_else(|| anyhow::anyhow!("local approval journal has no commit SHA"))?;
    let sources = collect_sources(&mut tx, task_id, &task.1, commit).await?;
    super::record::record_approval(&mut tx, &sources, now).await?;
    record_outcome(&mut tx, task_id, now).await?;
    tx.commit().await?;
    Ok(())
}

async fn collect_sources(
    tx: &mut Transaction<'_, Sqlite>,
    task_id: i64,
    instruction: &str,
    commit_sha: String,
) -> anyhow::Result<ApprovalProvenance> {
    let evidence_at: Option<i64> =
        sqlx::query_scalar("SELECT created_at FROM evidence WHERE task_id = ?")
            .bind(task_id)
            .fetch_optional(&mut **tx)
            .await?;
    Ok(ApprovalProvenance {
        task_id,
        instruction_digest: provenance::digest(instruction),
        verification_run: evidence_at.map(|created| provenance::verification_ref(task_id, created)),
        commit_sha,
    })
}

async fn record_outcome(
    tx: &mut Transaction<'_, Sqlite>,
    task_id: i64,
    now: i64,
) -> anyhow::Result<()> {
    let changed =
        sqlx::query("UPDATE tasks SET state = ?, updated_at = ? WHERE id = ? AND state = ?")
            .bind(db::state::DONE)
            .bind(now)
            .bind(task_id)
            .bind(db::state::FINALIZING)
            .execute(&mut **tx)
            .await?;
    if changed.rows_affected() != 1 {
        anyhow::bail!("local approval task state changed during completion");
    }
    sqlx::query(
        "INSERT INTO task_events (task_id, ts, kind, detail) VALUES (?, ?, 'approved', NULL)",
    )
    .bind(task_id)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    let journal = sqlx::query(
        "UPDATE local_approval_finalizations SET state = 'completed', \
         failure_code = NULL, updated_at = ? WHERE task_id = ? AND state = 'cleaned'",
    )
    .bind(now)
    .bind(task_id)
    .execute(&mut **tx)
    .await?;
    if journal.rows_affected() != 1 {
        anyhow::bail!("local approval journal changed during completion");
    }
    Ok(())
}

async fn validate_completed(tx: &mut Transaction<'_, Sqlite>, task_id: i64) -> anyhow::Result<()> {
    let decisions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM decision_records WHERE task_id = ? AND status = 'active'",
    )
    .bind(task_id)
    .fetch_one(&mut **tx)
    .await?;
    if decisions != 1 {
        anyhow::bail!("completed local approval has no unique active decision");
    }
    Ok(())
}
