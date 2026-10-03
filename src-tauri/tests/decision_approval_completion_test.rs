#[path = "support/temp_root.rs"]
mod temp_root;

use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::decision::approval_completion;
use praxis_lib::{db, memory};

static COUNTER: AtomicU32 = AtomicU32::new(0);
const SHA: &str = "abcdef0123456789abcdef0123456789abcdef01";

fn temp_db(label: &str) -> String {
    let sequence = COUNTER.fetch_add(1, Ordering::SeqCst);
    temp_root::dir()
        .join(format!(
            "praxis-decision-completion-{label}-{}-{sequence}.sqlite",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned()
}

async fn fixture(label: &str) -> (String, sqlx::SqlitePool, i64) {
    let path = temp_db(label);
    let pool = db::init_pool(&path).await.unwrap();
    memory::migrate(&pool).await.unwrap();
    let task_id = insert_finalizing_task(&pool).await;
    insert_sources(&pool, task_id).await;
    (path, pool, task_id)
}

async fn insert_finalizing_task(pool: &sqlx::SqlitePool) -> i64 {
    let task_id = db::insert_task(
        pool,
        "/repo",
        "praxis/ledger",
        "main",
        "/repo/.praxis/worktrees/ledger",
        "PRIVATE_INSTRUCTION_SENTINEL",
        None,
        None,
        "terminal",
        10,
    )
    .await
    .unwrap();
    db::update_state(pool, task_id, db::state::FINALIZING, 11)
        .await
        .unwrap();
    task_id
}

async fn insert_sources(pool: &sqlx::SqlitePool, task_id: i64) {
    sqlx::query(
        "INSERT INTO local_approval_finalizations \
         (task_id, state, commit_sha, exclude_generated_mcp, created_at, updated_at) \
         VALUES (?, 'cleaned', ?, 0, 12, 12)",
    )
    .bind(task_id)
    .bind(SHA)
    .execute(pool)
    .await
    .unwrap();
    insert_evidence(pool, task_id).await;
}

async fn insert_evidence(pool: &sqlx::SqlitePool, task_id: i64) {
    sqlx::query(
        "INSERT INTO evidence (task_id, passed, failed, ready, created_at) VALUES (?, 1, 0, 1, 15)",
    )
    .bind(task_id)
    .execute(pool)
    .await
    .unwrap();
}

async fn assert_authoritative_outcome(pool: &sqlx::SqlitePool, task_id: i64) {
    assert_eq!(
        db::get_task(pool, task_id).await.unwrap().unwrap().state,
        db::state::DONE
    );
    let counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM decision_records), \
                (SELECT COUNT(*) FROM decision_artifact_links), \
                (SELECT COUNT(*) FROM task_events WHERE task_id = ? AND kind = 'approved')",
    )
    .bind(task_id)
    .fetch_one(pool)
    .await
    .unwrap();
    // actor·task·instruction_digest·git_commit 4 + verification_run 1
    assert_eq!(counts, (1, 5, 1));
    assert_outcomes(pool, task_id).await;
}

async fn assert_outcomes(pool: &sqlx::SqlitePool, task_id: i64) {
    let state: String = sqlx::query_scalar(
        "SELECT state FROM local_approval_finalizations WHERE task_id = ?",
    )
    .bind(task_id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(state, "completed");
}

async fn persisted_values(pool: &sqlx::SqlitePool) -> String {
    sqlx::query_scalar(
        "SELECT GROUP_CONCAT(value, '|') FROM (\
         SELECT decision_key_hash AS value FROM decision_records UNION ALL \
         SELECT COALESCE(summary, '') FROM decision_records UNION ALL \
         SELECT artifact_ref FROM decision_artifact_links UNION ALL \
         SELECT state FROM local_approval_finalizations UNION ALL \
         SELECT COALESCE(failure_code, '') FROM local_approval_finalizations)",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn completion_atomically_writes_the_authoritative_approval() {
    let (path, pool, task_id) = fixture("happy").await;

    approval_completion::complete(&pool, task_id, 20)
        .await
        .unwrap();
    approval_completion::complete(&pool, task_id, 21)
        .await
        .unwrap();

    assert_authoritative_outcome(&pool, task_id).await;
    let persisted = persisted_values(&pool).await;
    for forbidden in [
        "PRIVATE_INSTRUCTION_SENTINEL",
        "PRIVATE_DIFF",
        "PRIVATE_TOOL_OUTPUT",
    ] {
        assert!(!persisted.contains(forbidden));
    }
    drop(pool);
    let _ = std::fs::remove_file(path);
}
