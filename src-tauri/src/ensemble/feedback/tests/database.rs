use std::sync::atomic::{AtomicU32, Ordering};

use super::super::*;

static DATABASE_COUNTER: AtomicU32 = AtomicU32::new(0);

#[tokio::test]
async fn loads_selected_candidate_model() {
    let pool = test_pool().await;
    let claude = insert_candidate(&pool, "claude", "ens-selected", 10).await;
    let codex = insert_candidate(&pool, "codex", "ens-selected", 11).await;
    set_state(&pool, claude, "Done", 20).await;
    set_state(&pool, codex, "Discarded", 21).await;
    append_model(&pool, claude, Some("opus"), None).await;
    append_model(&pool, claude, None, Some("claude-opus-4-8")).await;

    let history = feedback_history_with_limit(&pool, 20).await.unwrap();

    assert_eq!(history.selected_count, 1);
    assert_eq!(history.entries.len(), 1);
    let entry = &history.entries[0];
    assert_eq!(entry.selection_status, EnsembleSelectionStatus::Selected);
    assert_eq!(entry.selected_task_id, Some(claude));
    assert_eq!(entry.selected_agent.as_deref(), Some("claude"));
    assert_eq!(entry.requested_model.as_deref(), Some("opus"));
    assert_eq!(entry.resolved_model.as_deref(), Some("claude-opus-4-8"));
}

#[tokio::test]
async fn limits_to_recent_ensembles_and_keeps_multiple_done_candidates_ambiguous() {
    let pool = test_pool().await;
    let old = insert_candidate(&pool, "claude", "ens-old", 1).await;
    set_state(&pool, old, "Done", 2).await;
    let first = insert_candidate(&pool, "claude", "ens-ambiguous", 10).await;
    let second = insert_candidate(&pool, "codex", "ens-ambiguous", 11).await;
    set_state(&pool, first, "Done", 12).await;
    set_state(&pool, second, "Done", 13).await;
    insert_candidate(&pool, "claude", "ens-pending", 20).await;
    insert_candidate(&pool, "codex", "ens-pending", 21).await;

    let history = feedback_history_with_limit(&pool, 2).await.unwrap();

    assert_eq!(history.entries.len(), 2);
    assert_eq!(history.entries[0].ensemble, "ens-pending");
    assert_eq!(history.entries[1].ensemble, "ens-ambiguous");
    assert_eq!(history.pending_count, 1);
    assert_eq!(history.ambiguous_count, 1);
    assert_eq!(history.selected_count, 0);
    assert_eq!(history.entries[1].selected_task_id, None);
    assert_eq!(history.entries[1].selected_agent, None);
}

async fn test_pool() -> sqlx::SqlitePool {
    let sequence = DATABASE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = crate::testtmp::dir().join(format!(
        "praxis-ensemble-feedback-{}-{sequence}.sqlite",
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

async fn set_state(pool: &sqlx::SqlitePool, task_id: i64, state: &str, now: i64) {
    crate::db::update_state(pool, task_id, state, now)
        .await
        .unwrap();
}

async fn append_model(
    pool: &sqlx::SqlitePool,
    task_id: i64,
    requested: Option<&str>,
    resolved: Option<&str>,
) {
    let event = serde_json::json!({
        "kind": "model_snapshot",
        "requested": requested,
        "resolved": resolved,
        "source": "test",
    });
    crate::db::append_convo_event(pool, task_id, &event.to_string(), 1)
        .await
        .unwrap();
}

