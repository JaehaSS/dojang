#[path = "support/temp_root.rs"]
mod temp_root;

use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::decision::provenance::ApprovalProvenance;
use praxis_lib::{db, decision};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_db(label: &str) -> String {
    let sequence = COUNTER.fetch_add(1, Ordering::SeqCst);
    temp_root::dir()
        .join(format!(
            "praxis-decision-redaction-{label}-{}-{sequence}.sqlite",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned()
}

async fn task(pool: &sqlx::SqlitePool, instruction: &str) -> i64 {
    db::insert_task(
        pool,
        "/repo",
        "branch",
        "main",
        "/worktree",
        instruction,
        None,
        None,
        "terminal",
        10,
    )
    .await
    .unwrap()
}

async fn decision_for(pool: &sqlx::SqlitePool, task_id: i64) -> i64 {
    let provenance = ApprovalProvenance {
        task_id,
        instruction_digest: decision::provenance::digest("delete me"),
        verification_run: Some(decision::provenance::verification_ref(task_id, 99)),
        commit_sha: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into(),
    };
    let mut tx = pool.begin().await.unwrap();
    let id = decision::record::record_approval(&mut tx, &provenance, 20)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    id
}

async fn insert_journal(pool: &sqlx::SqlitePool, task_id: i64, state: &str) {
    let commit = if state == "prepared" {
        None
    } else {
        Some("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee")
    };
    sqlx::query(
        "INSERT INTO local_approval_finalizations \
         (task_id, state, commit_sha, exclude_generated_mcp, created_at, updated_at) \
         VALUES (?, ?, ?, 0, 20, 20)",
    )
    .bind(task_id)
    .bind(state)
    .bind(commit)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn deleting_a_task_leaves_only_a_content_free_tombstone() {
    let path = temp_db("delete");
    let pool = db::init_pool(&path).await.unwrap();
    let task_id = task(&pool, "PRIVATE_TASK_INSTRUCTION").await;
    let decision_id = decision_for(&pool, task_id).await;
    insert_journal(&pool, task_id, "completed").await;
    assert!(!decision::is_enabled(&pool).await.unwrap());

    db::delete_task(&pool, task_id).await.unwrap();

    // 삭제 후 남는 것과 지워지는 것을 한 행에서 전부 본다 — 컬럼을 나눠 조회하면
    // "이 행에는 이것만 남았다"는 계약이 여러 assert로 흩어진다.
    type RedactedRow = (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<String>,
        String,
        Option<i64>,
    );
    let row: RedactedRow =
        sqlx::query_as(
            "SELECT decision_key_hash, kind, outcome, actor_kind, task_id, summary, status, redacted_at \
             FROM decision_records WHERE id = ?",
        )
        .bind(decision_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.0.len(), 64);
    assert_eq!(
        (row.1, row.2, row.3, row.4, row.5),
        (None, None, None, None, None)
    );
    assert_eq!(row.6, "redacted");
    assert!(row.7.is_some_and(|redacted_at| redacted_at > 0));
    let links: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM decision_artifact_links")
        .fetch_one(&pool)
        .await
        .unwrap();
    let journals: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM local_approval_finalizations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((links, journals), (0, 0));
    assert!(db::get_task(&pool, task_id).await.unwrap().is_none());
    drop(pool);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn incomplete_journal_or_redaction_failure_rolls_back_task_delete() {
    let path = temp_db("rollback");
    let pool = db::init_pool(&path).await.unwrap();
    let task_id = task(&pool, "keep").await;
    let decision_id = decision_for(&pool, task_id).await;
    insert_journal(&pool, task_id, "prepared").await;

    let incomplete = db::delete_task(&pool, task_id).await.unwrap_err();
    assert!(incomplete
        .to_string()
        .contains("finalization is incomplete"));
    assert!(db::get_task(&pool, task_id).await.unwrap().is_some());

    sqlx::query("DELETE FROM local_approval_finalizations WHERE task_id = ?")
        .bind(task_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(
        "CREATE TRIGGER block_decision_redaction BEFORE UPDATE ON decision_records \
         BEGIN SELECT RAISE(ABORT, 'forced redaction failure'); END;",
    )
    .execute(&pool)
    .await
    .unwrap();
    let forced = db::delete_task(&pool, task_id).await.unwrap_err();
    assert!(forced.to_string().contains("forced redaction failure"));
    assert!(db::get_task(&pool, task_id).await.unwrap().is_some());
    let links: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM decision_artifact_links WHERE decision_id = ?")
            .bind(decision_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(links > 0, "link deletion must roll back with redaction");
    drop(pool);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn source_pruning_does_not_break_an_active_trace() {
    let path = temp_db("prune");
    let pool = db::init_pool(&path).await.unwrap();
    let task_id = task(&pool, "source").await;
    let decision_id = decision_for(&pool, task_id).await;
    db::set_setting(&pool, decision::FLAG_KEY, "true")
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO evidence (task_id, passed, failed, ready, created_at) VALUES (?, 1, 0, 1, 99)",
    )
    .bind(task_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM evidence WHERE task_id = ?")
        .bind(task_id)
        .execute(&pool)
        .await
        .unwrap();

    let trace = decision::why_trace::why_trace(&pool, decision_id, 2)
        .await
        .unwrap();
    assert!(trace
        .nodes
        .iter()
        .any(|node| node.kind == "verification_run"));
    drop(pool);
    let _ = std::fs::remove_file(path);
}
