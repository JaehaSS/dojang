use std::path::Path;

use sqlx::{Sqlite, SqlitePool, Transaction};

pub async fn delete_task(pool: &SqlitePool, id: i64) -> anyhow::Result<()> {
    // This is acquired before inspecting turns. New admission refuses this
    // task and a scheduler that already observed queued rechecks it before
    // spawning, closing queued->running against row deletion.
    let _side_deletion = crate::side_question::begin_task_deletion(id).await
        .map_err(anyhow::Error::msg)?;
    // Side-question children are not main conversation children. Stop and
    // reap their own handles before deleting their durable rows so a later
    // executor cannot write a deleted task back into the database.
    crate::side_question::stop_task(pool, id, crate::now())
        .await
        .map_err(anyhow::Error::msg)?;
    let _admission =
        crate::knowledge::vault::task_retention::try_admission_for_task(pool, id).await?;
    let mut tx = pool.begin().await?;
    let Some(worktree_path) = deletion_worktree(&mut tx, id).await? else {
        return Ok(());
    };

    delete_task_rows(&mut tx, id).await?;
    crate::knowledge::vault::task_retention::delete_task_data(&mut tx, id, _admission.is_some())
        .await?;
    // 이 작업을 이어받은 자식들의 출처를 끊는다. `resumed_from`에는 외래키가 없어서 그냥
    // 두면 삭제된 id를 가리킨 채 남고, 체인을 거슬러 오르던 이력 조회가 그 지점에서 멈춰
    // **그 위 조상들의 대화가 통째로 사라진다**. 자식에게는 지금 이 작업이 곧 뿌리다.
    sqlx::query("UPDATE tasks SET resumed_from = NULL WHERE resumed_from = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM tasks WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    crate::designmode::delete_task_captures(Path::new(&worktree_path), id)
        .map_err(anyhow::Error::msg)?;
    tx.commit().await?;
    Ok(())
}

async fn deletion_worktree(
    tx: &mut Transaction<'_, Sqlite>,
    id: i64,
) -> anyhow::Result<Option<String>> {
    let task: Option<(String, Option<i64>)> =
        sqlx::query_as("SELECT worktree_path, convo_pgid FROM tasks WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?;
    let Some((worktree_path, lease)) = task else {
        return Ok(None);
    };
    if lease.is_some() {
        anyhow::bail!("quarantined process lease must be resolved before task deletion");
    }
    let review_leases: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM review_process_leases WHERE task_id = ?")
            .bind(id)
            .fetch_one(&mut **tx)
            .await?;
    if review_leases > 0 {
        anyhow::bail!("durable review process lease must be resolved before task deletion");
    }
    Ok(Some(worktree_path))
}

async fn delete_task_rows(tx: &mut Transaction<'_, Sqlite>, id: i64) -> anyhow::Result<()> {
    crate::decision::redaction::redact_task_decisions(tx, id, crate::now()).await?;
    for table in [
        "convo_events",
        "convo_checkpoints",
        "task_events",
        "evidence",
        "runner_events",
        "task_output",
        "notification_results",
        "notification_cancels",
        "preview_commands",
        "know_cards",
        "know_card_checks",
    ] {
        delete_task_rows_from(tx, table, id).await?;
    }
    for table in ["partial_checkpoints", "review_annotations", "side_question_turns", "side_question_threads", "conversation_receipts"] {
        if table_exists(tx, table).await? {
            delete_task_rows_from(tx, table, id).await?;
        }
    }
    Ok(())
}

async fn table_exists(tx: &mut Transaction<'_, Sqlite>, table: &str) -> anyhow::Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?)",
    )
    .bind(table)
    .fetch_one(&mut **tx)
    .await
    .map_err(Into::into)
}

async fn delete_task_rows_from(
    tx: &mut Transaction<'_, Sqlite>,
    table: &str,
    id: i64,
) -> anyhow::Result<()> {
    sqlx::query(&format!("DELETE FROM {table} WHERE task_id = ?"))
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
