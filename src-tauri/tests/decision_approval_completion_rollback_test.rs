#[path = "support/temp_root.rs"]
mod temp_root;

use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::decision::approval_completion;
use praxis_lib::{db, memory};

static COUNTER: AtomicU32 = AtomicU32::new(0);
const SHA: &str = "abcdef0123456789abcdef0123456789abcdef01";

fn temp_db() -> String {
    let sequence = COUNTER.fetch_add(1, Ordering::SeqCst);
    temp_root::dir()
        .join(format!(
            "praxis-decision-completion-rollback-{}-{sequence}.sqlite",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned()
}

async fn fixture() -> (String, sqlx::SqlitePool, i64) {
    let path = temp_db();
    let pool = db::init_pool(&path).await.unwrap();
    memory::migrate(&pool).await.unwrap();
    let task_id = db::insert_task(
        &pool,
        "/repo",
        "praxis/rollback",
        "main",
        "/repo/.praxis/worktrees/rollback",
        "rollback",
        None,
        None,
        "terminal",
        10,
    )
    .await
    .unwrap();
    db::update_state(&pool, task_id, db::state::FINALIZING, 11)
        .await
        .unwrap();
    insert_sources(&pool, task_id).await;
    (path, pool, task_id)
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
}

#[tokio::test]
async fn ledger_failure_rolls_back_every_database_outcome() {
    let (path, pool, task_id) = fixture().await;
    sqlx::raw_sql(
        "CREATE TRIGGER fail_decision_insert BEFORE INSERT ON decision_records \
         BEGIN SELECT RAISE(ABORT, 'forced ledger failure'); END;",
    )
    .execute(&pool)
    .await
    .unwrap();

    let error = approval_completion::complete(&pool, task_id, 20)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("forced ledger failure"));
    assert_rolled_back(&pool, task_id).await;
    drop(pool);
    let _ = std::fs::remove_file(path);
}

async fn assert_rolled_back(pool: &sqlx::SqlitePool, task_id: i64) {
    assert_eq!(
        db::get_task(pool, task_id).await.unwrap().unwrap().state,
        db::state::FINALIZING
    );
    let state: (String, i64, i64) = sqlx::query_as(
        "SELECT (SELECT state FROM local_approval_finalizations WHERE task_id = ?), \
                (SELECT COUNT(*) FROM task_events WHERE task_id = ? AND kind = 'approved'), \
                (SELECT COUNT(*) FROM decision_records)",
    )
    .bind(task_id)
    .bind(task_id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(state, ("cleaned".into(), 0, 0));
}
