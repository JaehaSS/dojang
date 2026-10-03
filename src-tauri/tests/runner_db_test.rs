#[path = "support/temp_root.rs"]
mod temp_root;

use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::db::{self, state};

static COUNTER: AtomicU32 = AtomicU32::new(0);

async fn pool() -> (sqlx::SqlitePool, String) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = temp_root::dir()
        .join(format!(
            "praxis-runner-db-test-{}-{n}.sqlite",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned();
    (db::init_pool(&path).await.unwrap(), path)
}

#[tokio::test]
async fn runner_events_replay_in_global_sequence_order() {
    let (pool, path) = pool().await;
    let task_id = db::insert_task(
        &pool, "/repo", "branch", "main", "/wt", "run", None, None, "terminal", 1,
    )
    .await
    .unwrap();
    db::update_state(&pool, task_id, state::QUEUED, 2)
        .await
        .unwrap();
    let first = db::append_runner_event(&pool, task_id, 3, "queued", Some("one"))
        .await
        .unwrap();
    let second = db::append_runner_event(&pool, task_id, 4, "output", Some("two"))
        .await
        .unwrap();

    let events = db::list_runner_events_after(&pool, first, 10)
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].sequence, second);
    assert_eq!(events[0].detail.as_deref(), Some("two"));
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn task_output_replays_in_global_sequence_order() {
    let (pool, db_path) = pool().await;
    let task_id = db::insert_task(
        &pool, "/repo", "branch", "main", "/wt", "run", None, None, "terminal", 1,
    )
    .await
    .unwrap();
    let first = db::append_task_output(&pool, task_id, 2, "first")
        .await
        .unwrap();
    let second = db::append_task_output(&pool, task_id, 3, "second")
        .await
        .unwrap();

    let output = db::list_task_output_after(&pool, first, 10).await.unwrap();

    assert_eq!(output.len(), 1);
    assert_eq!(output[0].sequence, second);
    assert_eq!(output[0].data, "second");
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn task_output_and_runner_event_commit_together() {
    let (pool, db_path) = pool().await;
    let task_id = db::insert_task(
        &pool, "/repo", "branch", "main", "/wt", "run", None, None, "terminal", 1,
    )
    .await
    .unwrap();

    let (output_sequence, event_sequence) =
        db::append_task_output_with_runner_event(&pool, task_id, 2, "stream chunk")
            .await
            .unwrap();

    let output = db::list_task_output_after(&pool, 0, 10).await.unwrap();
    let events = db::list_runner_events_after(&pool, 0, 10).await.unwrap();
    assert_eq!(output[0].sequence, output_sequence);
    assert_eq!(events[0].sequence, event_sequence);
    assert_eq!(events[0].kind, "output");
    assert_eq!(events[0].detail.as_deref(), Some("stream chunk"));
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn concurrent_task_output_appends_keep_unique_global_sequences() {
    let (pool, db_path) = pool().await;
    let task_id = db::insert_task(
        &pool, "/repo", "branch", "main", "/wt", "run", None, None, "terminal", 1,
    )
    .await
    .unwrap();
    let mut writers = Vec::new();

    for index in 0..8 {
        let pool = pool.clone();
        writers.push(tokio::spawn(async move {
            db::append_task_output(&pool, task_id, index, &format!("output-{index}"))
                .await
                .unwrap()
        }));
    }

    for writer in writers {
        writer.await.unwrap();
    }

    let output = db::list_task_output_after(&pool, 0, 10).await.unwrap();
    assert_eq!(output.len(), 8);
    assert!(output
        .windows(2)
        .all(|pair| pair[0].sequence < pair[1].sequence));
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn state_transition_and_runner_event_commit_together() {
    let (pool, db_path) = pool().await;
    let task_id = db::insert_task(
        &pool, "/repo", "branch", "main", "/wt", "run", None, None, "terminal", 1,
    )
    .await
    .unwrap();

    db::transition_state_with_runner_event(
        &pool,
        task_id,
        state::QUEUED,
        2,
        "queued",
        Some("ready"),
    )
    .await
    .unwrap();

    assert_eq!(
        db::get_task(&pool, task_id).await.unwrap().unwrap().state,
        state::QUEUED
    );
    let events = db::list_runner_events_after(&pool, 0, 10).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "queued");
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn concurrent_workers_cannot_lease_the_same_queued_task() {
    let (pool, db_path) = pool().await;
    let task_id = db::insert_task(
        &pool, "/repo", "branch", "main", "/wt", "run", None, None, "terminal", 1,
    )
    .await
    .unwrap();
    db::update_state(&pool, task_id, state::QUEUED, 2)
        .await
        .unwrap();
    let mut workers = Vec::new();

    for now in 3..7 {
        let pool = pool.clone();
        workers.push(tokio::spawn(async move {
            db::claim_oldest_queued_task(&pool, now)
                .await
                .unwrap()
                .map(|task| task.id)
        }));
    }

    let mut leased = Vec::new();
    for worker in workers {
        if let Some(task_id) = worker.await.unwrap() {
            leased.push(task_id);
        }
    }
    assert_eq!(leased, vec![task_id]);
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn pruning_removes_only_expired_runner_history() {
    let (pool, db_path) = pool().await;
    let task_id = db::insert_task(
        &pool, "/repo", "branch", "main", "/wt", "run", None, None, "terminal", 1,
    )
    .await
    .unwrap();
    db::append_runner_event(&pool, task_id, 10, "output", Some("old"))
        .await
        .unwrap();
    db::append_runner_event(&pool, task_id, 20, "output", Some("new"))
        .await
        .unwrap();
    db::append_task_output(&pool, task_id, 10, "old")
        .await
        .unwrap();
    db::append_task_output(&pool, task_id, 20, "new")
        .await
        .unwrap();

    assert_eq!(db::prune_runner_history(&pool, 20).await.unwrap(), 2);

    assert_eq!(
        db::list_runner_events_after(&pool, 0, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db::list_task_output_after(&pool, 0, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(db::get_task(&pool, task_id).await.unwrap().is_some());
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn runner_history_migration_is_idempotent() {
    let (pool, db_path) = pool().await;
    drop(pool);

    let reopened_pool = db::init_pool(&db_path).await.unwrap();
    let task_id = db::insert_task(
        &reopened_pool,
        "/repo",
        "branch",
        "main",
        "/wt",
        "run",
        None,
        None,
        "terminal",
        1,
    )
    .await
    .unwrap();

    assert!(
        db::append_task_output(&reopened_pool, task_id, 2, "survives reopen")
            .await
            .is_ok()
    );
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn auth_blocked_task_is_skipped_and_a_later_one_is_leased_instead() {
    // 차단은 큐를 멈추는 게 아니라 그 작업만 건너뛴다 — 인증이 필요한 벤더 하나 때문에
    // 나머지 작업까지 서면, 사용자는 원인을 큐 어디에서도 볼 수 없다.
    let (pool, path) = pool().await;
    let blocked_task = db::insert_task(
        &pool, "/repo", "blocked", "main", "/wt-b", "run", None, None, "terminal", 1,
    )
    .await
    .unwrap();
    let healthy = db::insert_task(
        &pool, "/repo", "healthy", "main", "/wt-h", "run", None, None, "terminal", 2,
    )
    .await
    .unwrap();
    db::update_state(&pool, blocked_task, state::QUEUED, 3)
        .await
        .unwrap();
    db::update_state(&pool, healthy, state::QUEUED, 4)
        .await
        .unwrap();
    db::set_blocked_reason(&pool, blocked_task, Some(&db::blocked::auth("claude")))
        .await
        .unwrap();

    // blocked_task가 더 오래됐지만 건너뛴다.
    let leased = db::claim_oldest_queued_task(&pool, 5).await.unwrap().unwrap();
    assert_eq!(leased.id, healthy);

    // 차단된 쪽은 여전히 Queued로 남아 있다 — 실패시키지 않았다.
    let still = db::get_task(&pool, blocked_task).await.unwrap().unwrap();
    assert_eq!(still.state, state::QUEUED);
    assert_eq!(still.blocked_reason.as_deref(), Some("auth:claude"));
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn clearing_the_block_resumes_the_task_without_a_transition() {
    // 재개 전이를 따로 쓰지 않는다 — 작업이 Queued를 떠난 적이 없으므로 다음 lease가 집는다.
    let (pool, path) = pool().await;
    let task_id = db::insert_task(
        &pool, "/repo", "branch", "main", "/wt", "run", None, None, "terminal", 1,
    )
    .await
    .unwrap();
    db::update_state(&pool, task_id, state::QUEUED, 2).await.unwrap();
    db::set_blocked_reason(&pool, task_id, Some(&db::blocked::auth("codex")))
        .await
        .unwrap();
    assert!(db::claim_oldest_queued_task(&pool, 3).await.unwrap().is_none());

    // 다른 벤더를 풀어도 이 작업은 그대로 막혀 있다.
    assert_eq!(db::clear_auth_block(&pool, "claude").await.unwrap(), 0);
    assert!(db::claim_oldest_queued_task(&pool, 4).await.unwrap().is_none());

    assert_eq!(db::clear_auth_block(&pool, "codex").await.unwrap(), 1);
    let leased = db::claim_oldest_queued_task(&pool, 5).await.unwrap().unwrap();
    assert_eq!(leased.id, task_id);
    assert_eq!(leased.state, state::STARTING);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn blocking_only_applies_to_queued_tasks() {
    // 이미 시작한 작업에 차단 표시가 남으면, 다음에 큐로 돌아왔을 때 이유 없이 멈춘다.
    let (pool, path) = pool().await;
    let task_id = db::insert_task(
        &pool, "/repo", "branch", "main", "/wt", "run", None, None, "terminal", 1,
    )
    .await
    .unwrap();
    db::update_state(&pool, task_id, state::RUNNING, 2).await.unwrap();
    db::set_blocked_reason(&pool, task_id, Some(&db::blocked::auth("claude")))
        .await
        .unwrap();
    assert!(db::get_task(&pool, task_id)
        .await
        .unwrap()
        .unwrap()
        .blocked_reason
        .is_none());
    let _ = std::fs::remove_file(path);
}
