#[path = "support/temp_root.rs"]
mod temp_root;

use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::db::{self, state};
use praxis_lib::runner::queue::QueueWorker;
use praxis_lib::memory;

static COUNTER: AtomicU32 = AtomicU32::new(0);

#[tokio::test]
async fn worker_leases_at_most_its_configured_concurrency() {
    let suffix = COUNTER.fetch_add(1, Ordering::SeqCst);
    let db_path = temp_root::dir()
        .join(format!(
            "praxis-runner-queue-{}-{suffix}.sqlite",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned();
    let pool = db::init_pool(&db_path).await.unwrap();
    for now in 1..=10 {
        let task_id = db::insert_task(
            &pool,
            "/repo",
            &format!("task-{now}"),
            "main",
            "/wt",
            "run",
            None,
            None,
            "terminal",
            now,
        )
        .await
        .unwrap();
        db::update_state(&pool, task_id, state::QUEUED, now)
            .await
            .unwrap();
    }
    let worker = QueueWorker::new(pool.clone(), 2);

    let first = worker.lease_next(10).await.unwrap().unwrap();
    let second = worker.lease_next(11).await.unwrap().unwrap();
    assert!(worker.lease_next(12).await.unwrap().is_none());
    let tasks = db::list_tasks(&pool).await.unwrap();
    assert_eq!(
        tasks
            .iter()
            .filter(|task| task.state == state::STARTING)
            .count(),
        2
    );
    assert_eq!(
        tasks
            .iter()
            .filter(|task| task.state == state::QUEUED)
            .count(),
        8
    );

    drop(first);
    assert!(worker.lease_next(13).await.unwrap().is_some());
    drop(second);
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn worker_runs_a_leased_terminal_task_to_awaiting_review() {
    let db_path = temp_root::dir()
        .join(format!(
            "praxis-runner-queue-run-{}.sqlite",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned();
    let worktree =
        temp_root::dir().join(format!("praxis-runner-queue-run-{}", std::process::id()));
    std::fs::create_dir_all(&worktree).unwrap();
    let pool = db::init_pool(&db_path).await.unwrap();
    memory::migrate(&pool).await.unwrap();
    let task_id = db::insert_task(
        &pool,
        "/tmp",
        "branch",
        "main",
        worktree.to_str().unwrap(),
        "queue-run-marker",
        Some("/bin/echo"),
        None,
        "terminal",
        1,
    )
    .await
    .unwrap();
    db::update_state(&pool, task_id, state::QUEUED, 2)
        .await
        .unwrap();
    let worker = QueueWorker::new(pool.clone(), 1);

    assert_eq!(worker.run_next(3).await.unwrap(), Some(0));
    assert_eq!(
        db::get_task(&pool, task_id).await.unwrap().unwrap().state,
        state::AWAITING_REVIEW
    );
    assert!(db::list_task_output_after(&pool, 0, 10).await.unwrap()[0]
        .data
        .contains("queue-run-marker"));
    // 파일형 메모리(설계 2026-09-13, P1)는 검증할 투영 원장이 없어 시작 영수증 자체를
    // 만들지 않는다 — `task_start_receipts`는 신규 설치에서 더는 생성되지 않는다
    // (`db::promote_starting_task`가 그 영수증 없이 항상 승격한다).
    let receipts_table: Option<(String,)> = sqlx::query_as(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'task_start_receipts'",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(receipts_table.is_none());
    let _ = std::fs::remove_dir_all(worktree);
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn cancel_terminates_only_the_active_task_process_group() {
    let db_path = temp_root::dir()
        .join(format!(
            "praxis-runner-queue-cancel-{}.sqlite",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned();
    let worktree =
        temp_root::dir().join(format!("praxis-runner-queue-cancel-{}", std::process::id()));
    std::fs::create_dir_all(&worktree).unwrap();
    let pool = db::init_pool(&db_path).await.unwrap();
    memory::migrate(&pool).await.unwrap();
    let task_id = db::insert_task(
        &pool,
        worktree.to_str().unwrap(),
        "branch",
        "main",
        "/tmp",
        "30",
        Some("/bin/sleep"),
        None,
        "terminal",
        1,
    )
    .await
    .unwrap();
    db::update_state(&pool, task_id, state::QUEUED, 2)
        .await
        .unwrap();
    let worker = QueueWorker::new(pool.clone(), 1);

    assert!(worker.spawn_next(3).await.unwrap());
    let mut cancelled = false;
    for _ in 0..20 {
        if worker.cancel(task_id, 4).await.unwrap() {
            cancelled = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    assert!(cancelled, "task process가 registry에 등록되지 않았습니다");
    assert_eq!(
        db::get_task(&pool, task_id).await.unwrap().unwrap().state,
        state::FAILED
    );
    assert!(db::list_runner_events_after(&pool, 0, 10)
        .await
        .unwrap()
        .iter()
        .any(|event| event.kind == "cancelled"));
    let _ = std::fs::remove_dir_all(worktree);
    let _ = std::fs::remove_file(db_path);
}

/// B-1 annotations resend가 재사용하는 동기 가드 — task 없음/모드 불일치/상태 불일치는
/// 벤더 프로세스를 전혀 건드리지 않고 즉시 거부한다(실제 vendor 스폰은 process.rs 레벨에서
/// 스텁 스크립트로 검증 — 여기서는 실제 claude/codex/agy 바이너리를 건드리지 않기 위해
/// 가드 실패 경로만 다룬다).
#[tokio::test]
async fn resume_conversation_rejects_before_touching_any_vendor_process() {
    let db_path = temp_root::dir()
        .join(format!(
            "praxis-runner-queue-resume-{}.sqlite",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned();
    let pool = db::init_pool(&db_path).await.unwrap();
    let worker = QueueWorker::new(pool.clone(), 2);

    let missing = worker.resume_conversation(999, "hi".into(), 10).await;
    assert!(missing.unwrap_err().contains("찾을 수 없습니다"));

    let terminal_task = db::insert_task(
        &pool,
        "/repo",
        "b1",
        "main",
        "/wt1",
        "i",
        Some("claude"),
        None,
        "terminal",
        1,
    )
    .await
    .unwrap();
    db::update_state(&pool, terminal_task, state::AWAITING_REVIEW, 2)
        .await
        .unwrap();
    let wrong_mode = worker
        .resume_conversation(terminal_task, "hi".into(), 10)
        .await;
    assert!(wrong_mode.unwrap_err().contains("대화 모드"));
    assert_eq!(
        db::get_task(&pool, terminal_task)
            .await
            .unwrap()
            .unwrap()
            .state,
        state::AWAITING_REVIEW,
        "가드 실패는 상태를 바꾸지 않는다"
    );

    let running_convo = db::insert_task(
        &pool,
        "/repo",
        "b2",
        "main",
        "/wt2",
        "i",
        Some("claude"),
        None,
        "conversation",
        3,
    )
    .await
    .unwrap();
    db::update_state(&pool, running_convo, state::RUNNING, 4)
        .await
        .unwrap();
    let wrong_state = worker
        .resume_conversation(running_convo, "hi".into(), 10)
        .await;
    assert!(wrong_state.unwrap_err().contains("검토 대기"));
    assert_eq!(
        db::get_task(&pool, running_convo)
            .await
            .unwrap()
            .unwrap()
            .state,
        state::RUNNING
    );

    let _ = std::fs::remove_file(db_path);
}
