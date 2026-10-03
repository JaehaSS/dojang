use std::sync::atomic::{AtomicU32, Ordering};

use super::super::*;

static DATABASE_COUNTER: AtomicU32 = AtomicU32::new(0);

#[tokio::test]
async fn complete_observation_reports_success_over_observed_tasks() {
    let pool = test_pool().await;
    let success = insert_task(&pool, "success", 1_000).await;
    let followup = insert_task(&pool, "followup", 1_000).await;
    set_state(&pool, success, crate::db::state::DONE, 1_100).await;
    set_state(&pool, followup, crate::db::state::AWAITING_REVIEW, 1_200).await;
    for task_id in [success, followup] {
        record_event(&pool, task_id, "followup_observation_started").await;
    }
    record_event(&pool, followup, "user_followup_input_observed").await;

    let outcome = compute_outcomes(&pool, "all", 2_000).await.unwrap();

    assert_eq!(outcome.no_reexplanation_target_task_count, 2);
    assert_eq!(outcome.no_reexplanation_observed_task_count, 2);
    assert_eq!(outcome.no_reexplanation_success_task_count, 1);
    assert_eq!(outcome.no_reexplanation_unmeasured_task_count, 0);
    assert_eq!(outcome.no_reexplanation_completion_rate, Some(0.5));
}

/// 대상 자격은 `followup_observation_started` 마커 하나로 정해진다(파일 메모리가 실제
/// 내용과 함께 투영됐다는 뜻) — 마커가 없는 작업은 완료 상태라도 셀 수 없다.
#[tokio::test]
async fn task_without_marker_is_not_counted() {
    let pool = test_pool().await;
    let observed = insert_task(&pool, "observed", 1_000).await;
    let missing = insert_task(&pool, "missing", 1_000).await;
    for task_id in [observed, missing] {
        set_state(&pool, task_id, crate::db::state::DONE, 1_100).await;
    }
    record_event(&pool, observed, "followup_observation_started").await;
    // `missing`은 완료 상태지만 마커가 없다 — 메모리를 안내한 적이 없으니 대상에서 빠진다.

    let outcome = compute_outcomes(&pool, "all", 2_000).await.unwrap();

    assert_eq!(
        outcome.no_reexplanation_target_task_count, 1,
        "마커 없는 작업은 대상에서 빠진다"
    );
    assert_eq!(outcome.no_reexplanation_observed_task_count, 1);
    assert_eq!(outcome.no_reexplanation_success_task_count, 1);
    assert_eq!(outcome.no_reexplanation_unmeasured_task_count, 0);
    assert_eq!(outcome.no_reexplanation_completion_rate, Some(1.0));
}

#[tokio::test]
async fn running_or_unmarked_tasks_are_excluded() {
    let pool = test_pool().await;
    let eligible = insert_task(&pool, "eligible", 1_000).await;
    let running = insert_task(&pool, "running", 1_000).await;
    let unmarked = insert_task(&pool, "unmarked", 1_000).await;
    set_state(&pool, eligible, crate::db::state::DONE, 1_100).await;
    set_state(&pool, running, crate::db::state::RUNNING, 1_100).await;
    set_state(&pool, unmarked, crate::db::state::DONE, 1_100).await;
    for task_id in [eligible, running] {
        record_event(&pool, task_id, "followup_observation_started").await;
    }
    // `unmarked`는 완료 상태지만 마커가 없다.

    let outcome = compute_outcomes(&pool, "all", 2_000).await.unwrap();

    assert_eq!(outcome.no_reexplanation_target_task_count, 1);
    assert_eq!(outcome.no_reexplanation_success_task_count, 1);
    assert_eq!(outcome.no_reexplanation_completion_rate, Some(1.0));
}

async fn test_pool() -> sqlx::SqlitePool {
    let sequence = DATABASE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = crate::testtmp::dir().join(format!(
        "praxis-no-reexplanation-{}-{sequence}.sqlite",
        std::process::id()
    ));
    let pool = crate::db::init_pool(path.to_str().unwrap()).await.unwrap();
    crate::memory::migrate(&pool).await.unwrap();
    pool
}

async fn insert_task(pool: &sqlx::SqlitePool, branch: &str, now: i64) -> i64 {
    crate::db::insert_task(
        pool,
        "/repo",
        branch,
        "main",
        "/worktree",
        "ship",
        Some("codex"),
        None,
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

async fn record_event(pool: &sqlx::SqlitePool, task_id: i64, kind: &str) {
    sqlx::query("INSERT INTO task_events (task_id, ts, kind, detail) VALUES (?, 1, ?, NULL)")
        .bind(task_id)
        .bind(kind)
        .execute(pool)
        .await
        .unwrap();
}
