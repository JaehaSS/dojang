use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::db;

pub(super) async fn complete(
    pool: &SqlitePool,
    task_id: i64,
    decision: &str,
    now: i64,
) -> anyhow::Result<()> {
    let terminal = if decision == "approved" {
        db::state::DONE
    } else {
        db::state::DISCARDED
    };
    let mut tx = pool.begin().await?;
    let changed =
        sqlx::query("UPDATE tasks SET state = ?, updated_at = ? WHERE id = ? AND state = ?")
            .bind(terminal)
            .bind(now)
            .bind(task_id)
            .bind(db::state::FINALIZING)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    if changed == 1 {
        record_terminal_outcome(&mut tx, task_id, decision, now).await?;
    }
    sqlx::query(
        "UPDATE runner_finalizations SET state = 'completed', updated_at = ? WHERE task_id = ?",
    )
    .bind(now)
    .bind(task_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn record_terminal_outcome(
    tx: &mut Transaction<'_, Sqlite>,
    task_id: i64,
    decision: &str,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO runner_events (task_id, ts, kind) VALUES (?, ?, ?)")
        .bind(task_id)
        .bind(now)
        .bind(decision)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
