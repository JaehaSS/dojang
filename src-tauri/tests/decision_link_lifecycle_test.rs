#[path = "support/temp_root.rs"]
mod temp_root;

use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::decision::provenance::ApprovalProvenance;
use praxis_lib::{db, decision};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_db() -> String {
    let sequence = COUNTER.fetch_add(1, Ordering::SeqCst);
    temp_root::dir()
        .join(format!(
            "praxis-decision-link-lifecycle-{}-{sequence}.sqlite",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned()
}

async fn completed_decision(pool: &sqlx::SqlitePool) -> i64 {
    let task_id = db::insert_task(
        pool,
        "/repo",
        "branch",
        "main",
        "/worktree",
        "sealed",
        None,
        None,
        "terminal",
        10,
    )
    .await
    .unwrap();
    let provenance = ApprovalProvenance {
        task_id,
        instruction_digest: decision::provenance::digest("sealed"),
        verification_run: None,
        commit_sha: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into(),
    };
    let mut tx = pool.begin().await.unwrap();
    let decision_id = decision::record::record_approval(&mut tx, &provenance, 20)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    insert_completed_journal(pool, task_id).await;
    decision_id
}

async fn insert_completed_journal(pool: &sqlx::SqlitePool, task_id: i64) {
    sqlx::query(
        "INSERT INTO local_approval_finalizations \
         (task_id, state, commit_sha, exclude_generated_mcp, created_at, updated_at) \
         VALUES (?, 'completed', 'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee', 0, 20, 20)",
    )
    .bind(task_id)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn completed_decision_links_are_sealed_until_redaction() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.unwrap();
    let decision_id = completed_decision(&pool).await;

    let insert = insert_link(&pool, decision_id).await.unwrap_err();
    assert!(insert.to_string().contains("sealed"));
    let delete = sqlx::query(
        "DELETE FROM decision_artifact_links WHERE decision_id = ? AND artifact_kind = 'actor'",
    )
    .bind(decision_id)
    .execute(&pool)
    .await
    .unwrap_err();
    assert!(delete.to_string().contains("redaction"));
    let orphan = insert_link(&pool, 999_999).await.unwrap_err();
    assert!(orphan.to_string().contains("active decision"));
    drop(pool);
    let _ = std::fs::remove_file(path);
}

async fn insert_link(
    pool: &sqlx::SqlitePool,
    decision_id: i64,
) -> Result<sqlx::sqlite::SqliteQueryResult, sqlx::Error> {
    sqlx::query(
        "INSERT INTO decision_artifact_links \
         (decision_id, relation, artifact_kind, artifact_ref, created_at) \
         VALUES (?, 'used', 'task_start_receipt', '999', 30)",
    )
    .bind(decision_id)
    .execute(pool)
    .await
}
