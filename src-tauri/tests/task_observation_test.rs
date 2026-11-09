#[path = "support/temp_root.rs"]
mod temp_root;
use praxis_lib::{db, task_observation as observation};
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);
async fn fixture() -> (sqlx::SqlitePool, i64) {
    let path = temp_root::dir().join(format!(
        "observation-{}.sqlite",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let pool = db::init_pool(path.to_str().unwrap()).await.unwrap();
    let id = db::insert_task(
        &pool,
        "/repo",
        "feature",
        "main",
        "/repo",
        "test",
        Some("agy"),
        None,
        "conversation",
        1,
    )
    .await
    .unwrap();
    db::update_state(&pool, id, db::state::AWAITING_REVIEW, 2)
        .await
        .unwrap();
    (pool, id)
}
#[tokio::test]
async fn missing_observation_does_not_mean_stable_end() {
    let (pool, id) = fixture().await;
    db::update_state(&pool, id, db::state::RUNNING, 3)
        .await
        .unwrap();
    let value = observation::snapshot(&pool, id, false, 4).await.unwrap();
    assert_eq!(value.status, "unknown");
    assert_eq!(value.child_processes, None);
    let active = observation::snapshot(&pool, id, true, 5).await.unwrap();
    assert_eq!(active.status, "running");
    assert_ne!(value.revision, active.revision);
    assert_eq!(
        active
            .capabilities
            .features
            .iter()
            .find(|f| f.key == "cost")
            .unwrap()
            .state,
        "unsupported"
    );
}
#[tokio::test]
async fn side_question_and_delivery_prevent_idle_review_label() {
    let (pool, id) = fixture().await;
    let idle = observation::snapshot(&pool, id, false, 3).await.unwrap();
    assert_eq!(idle.status, "review_needed");
    sqlx::query("INSERT INTO side_question_threads(task_id,generation,provider,model,created_at) VALUES(?,0,'claude','test',3)").bind(id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO side_question_turns(thread_id,task_id,request_id,payload_hash,generation,question,contexts,state,created_at) VALUES((SELECT id FROM side_question_threads WHERE task_id=?),?,'req','hash',0,'q','[]','queued',4)").bind(id).bind(id).execute(&pool).await.unwrap();
    let busy = observation::snapshot(&pool, id, false, 5).await.unwrap();
    assert_eq!(busy.status, "remaining_work");
    assert_eq!(busy.side_questions, 1);
    assert_ne!(idle.revision, busy.revision);
    sqlx::query("UPDATE side_question_turns SET state='completed' WHERE task_id=?")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        observation::snapshot(&pool, id, false, 6)
            .await
            .unwrap()
            .status,
        "review_needed"
    );
}
#[tokio::test]
async fn bound_question_contract_and_policy_control_support() {
    let (pool, id) = fixture().await;
    sqlx::query("UPDATE tasks SET agent='claude' WHERE id=?")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    praxis_lib::convo::interaction::bind_runtime(
        &pool,
        id,
        praxis_lib::convo::interaction::RUNTIME_LOCAL,
    )
    .await
    .unwrap();
    let value = observation::snapshot(&pool, id, false, 3).await.unwrap();
    assert_eq!(value.capabilities.route, "claude-mcp");
    assert_eq!(
        value
            .capabilities
            .features
            .iter()
            .find(|f| f.key == "approvals")
            .unwrap()
            .enabled,
        Some(false)
    );
    praxis_lib::convo::interaction::set_approvals(&pool, id, true)
        .await
        .unwrap();
    let enabled = observation::snapshot(&pool, id, false, 4).await.unwrap();
    assert_eq!(
        enabled
            .capabilities
            .features
            .iter()
            .find(|f| f.key == "approvals")
            .unwrap()
            .enabled,
        Some(true)
    );
}
