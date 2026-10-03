use std::sync::atomic::{AtomicU32, Ordering};

use super::super::*;

static DATABASE_COUNTER: AtomicU32 = AtomicU32::new(0);

#[tokio::test]
async fn aggregates_goal_verification_and_ensemble_outcomes() {
    let pool = test_pool().await;
    let selected = insert_task(&pool, "selected", Some("ens-selected"), true, 1_000).await;
    let loser = insert_task(&pool, "loser", Some("ens-selected"), false, 1_000).await;
    set_state(&pool, selected, "Done", 1_100).await;
    set_state(&pool, loser, "Discarded", 1_050).await;
    let ambiguous_a = insert_task(&pool, "amb-a", Some("ens-ambiguous"), false, 1_000).await;
    let ambiguous_b = insert_task(&pool, "amb-b", Some("ens-ambiguous"), false, 1_000).await;
    set_state(&pool, ambiguous_a, "Done", 1_200).await;
    set_state(&pool, ambiguous_b, "Done", 1_300).await;
    let rejected_a = insert_task(&pool, "reject-a", Some("ens-none"), false, 1_000).await;
    let rejected_b = insert_task(&pool, "reject-b", Some("ens-none"), false, 1_000).await;
    set_state(&pool, rejected_a, "Discarded", 1_100).await;
    set_state(&pool, rejected_b, "Discarded", 1_100).await;
    let standalone = insert_task(&pool, "standalone", None, false, 1_000).await;
    set_state(&pool, standalone, "Done", 1_400).await;
    crate::db::upsert_evidence(&pool, selected, "", 0, "", 0, 1, 0, true, 1_100)
        .await
        .unwrap();

    let outcome = compute_outcomes(&pool, "all", 2_000).await.unwrap();

    assert_eq!(outcome.task_count, 7);
    assert_eq!(outcome.accepted_task_count, 4);
    assert_eq!(outcome.goal_contract_task_count, 1);
    assert_eq!(outcome.ready_accepted_task_count, 1);
    assert_eq!(outcome.ensemble_count, 3);
    assert_eq!(outcome.selected_ensemble_count, 1);
    assert_eq!(outcome.ambiguous_ensemble_count, 1);
    assert_eq!(outcome.no_selection_ensemble_count, 1);
    assert_eq!(outcome.average_accept_seconds, Some(250.0));
    assert_eq!(outcome.no_reexplanation_completion_rate, None);
}

#[tokio::test]
async fn range_filters_tasks_but_classifies_every_candidate_in_a_recent_ensemble() {
    let pool = test_pool().await;
    let now = 1_000_000;
    let old = now - 8 * 86_400;
    let winner = insert_task(&pool, "winner", Some("ens-recent"), false, old).await;
    let loser = insert_task(&pool, "loser", Some("ens-recent"), false, old).await;
    set_state(&pool, winner, "Done", now).await;
    set_state(&pool, loser, "Discarded", old).await;
    let old_standalone = insert_task(&pool, "old", None, false, old).await;
    set_state(&pool, old_standalone, "Done", old).await;

    let outcome = compute_outcomes(&pool, "7d", now).await.unwrap();

    assert_eq!(outcome.task_count, 1);
    assert_eq!(outcome.accepted_task_count, 1);
    assert_eq!(outcome.ensemble_count, 1);
    assert_eq!(outcome.selected_ensemble_count, 1);
    assert_eq!(outcome.ambiguous_ensemble_count, 0);
}

#[tokio::test]
async fn empty_database_returns_zero_counts_and_unmeasured_rates() {
    let pool = test_pool().await;

    let outcome = compute_outcomes(&pool, "all", 2_000).await.unwrap();

    assert_eq!(outcome.task_count, 0);
    assert_eq!(outcome.accepted_task_count, 0);
    assert_eq!(outcome.ensemble_count, 0);
    assert_eq!(outcome.average_accept_seconds, None);
    assert_eq!(outcome.no_reexplanation_completion_rate, None);
}

async fn test_pool() -> sqlx::SqlitePool {
    let sequence = DATABASE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = crate::testtmp::dir().join(format!(
        "praxis-outcome-insights-{}-{sequence}.sqlite",
        std::process::id()
    ));
    let pool = crate::db::init_pool(path.to_str().unwrap()).await.unwrap();
    crate::memory::migrate(&pool).await.unwrap();
    pool
}

async fn insert_task(
    pool: &sqlx::SqlitePool,
    branch: &str,
    ensemble: Option<&str>,
    with_contract: bool,
    now: i64,
) -> i64 {
    let contract = with_contract.then(test_contract);
    crate::db::insert_task_with_goal_contract(
        pool,
        "/repo",
        branch,
        "main",
        "/worktree",
        "ship",
        Some("codex"),
        ensemble,
        None,
        None,
        "conversation",
        contract.as_ref(),
        None,
        now,
    )
    .await
    .unwrap()
}

fn test_contract() -> crate::goal_contract::GoalContract {
    crate::goal_contract::GoalContract {
        schema_version: crate::goal_contract::SCHEMA_VERSION,
        objective: "ship".into(),
        acceptance: vec!["tests pass".into()],
        stop_conditions: Vec::new(),
        must_preserve: Vec::new(),
        protected_paths: Vec::new(),
        non_goals: Vec::new(),
    }
}

async fn set_state(pool: &sqlx::SqlitePool, task_id: i64, state: &str, now: i64) {
    crate::db::update_state(pool, task_id, state, now)
        .await
        .unwrap();
}

