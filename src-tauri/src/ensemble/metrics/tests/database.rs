use std::sync::atomic::{AtomicU32, Ordering};

use super::super::*;

static DATABASE_COUNTER: AtomicU32 = AtomicU32::new(0);

#[tokio::test]
async fn loads_candidate_identity_and_events_for_one_ensemble() {
    let pool = test_pool().await;
    let claude = insert_candidate(&pool, "claude", "ens-metrics", 1).await;
    let codex = insert_candidate(&pool, "codex", "ens-metrics", 2).await;
    insert_candidate(&pool, "agy", "other-ensemble", 3).await;
    crate::db::set_task_model(&pool, claude, "opus")
        .await
        .unwrap();
    append_model(&pool, claude, 9, Some("opus"), None).await;
    append_model(&pool, claude, 9, None, Some("claude-opus-4-8")).await;
    append_event(&pool, claude, 10, r#"{"kind":"user","text":"ship"}"#).await;
    append_event(
        &pool,
        claude,
        14,
        r#"{"kind":"result","is_error":false,"tokens_in":12,"tokens_out":3,"cost_usd":0.04}"#,
    )
    .await;
    append_model(&pool, codex, 10, Some("gpt-5.6-sol"), None).await;
    append_model(&pool, codex, 11, None, Some("gpt-5.6-sol")).await;

    let metrics = candidate_metrics(&pool, "ens-metrics").await.unwrap();

    assert_eq!(metrics.len(), 2);
    assert_eq!(metrics[0].task_id, claude);
    assert_eq!(metrics[0].agent, "claude");
    assert_eq!(metrics[0].model.as_deref(), Some("opus"));
    assert_eq!(
        metrics[0].resolved_model.as_deref(),
        Some("claude-opus-4-8")
    );
    assert_eq!(metrics[0].active_seconds, 4);
    assert_eq!(metrics[0].tokens_in, 12);
    assert_eq!(metrics[1].task_id, codex);
    assert_eq!(metrics[1].model.as_deref(), Some("gpt-5.6-sol"));
    assert_eq!(metrics[1].resolved_model.as_deref(), Some("gpt-5.6-sol"));
}

async fn test_pool() -> sqlx::SqlitePool {
    let sequence = DATABASE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = crate::testtmp::dir().join(format!(
        "praxis-ensemble-metrics-{}-{sequence}.sqlite",
        std::process::id()
    ));
    let pool = crate::db::init_pool(path.to_str().unwrap()).await.unwrap();
    crate::memory::migrate(&pool).await.unwrap();
    pool
}

async fn insert_candidate(pool: &sqlx::SqlitePool, agent: &str, ensemble: &str, now: i64) -> i64 {
    crate::db::insert_task(
        pool,
        "/repo",
        &format!("branch-{agent}"),
        "main",
        "/worktree",
        "ship",
        Some(agent),
        Some(ensemble),
        "conversation",
        now,
    )
    .await
    .unwrap()
}

async fn append_model(
    pool: &sqlx::SqlitePool,
    task_id: i64,
    timestamp: i64,
    requested: Option<&str>,
    resolved: Option<&str>,
) {
    let event = serde_json::json!({
        "kind": "model_snapshot",
        "requested": requested,
        "resolved": resolved,
        "source": "test",
    });
    append_event(pool, task_id, timestamp, &event.to_string()).await;
}

async fn append_event(pool: &sqlx::SqlitePool, task_id: i64, timestamp: i64, event: &str) {
    crate::db::append_convo_event(pool, task_id, event, timestamp)
        .await
        .unwrap();
}
