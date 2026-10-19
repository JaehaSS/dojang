use super::*;
use crate::pipeline::db as pdb;

async fn fixture(task_state: &str, run_state: &str) -> (AppState, i64, i64) {
    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
    let path = crate::testtmp::dir().join(format!(
        "dojang-task-removal-{}.sqlite",
        SEQUENCE.fetch_add(1, Ordering::SeqCst)
    ));
    let pool = db::init_pool(path.to_str().unwrap()).await.unwrap();
    // 직접 실행 작업으로 구성해 명령 라우팅과 상태 전이만 검사한다.
    let task = db::insert_task(
        &pool, "/repo", "branch", "main", "/repo", "test", None, None, "terminal", 1,
    )
    .await
    .unwrap();
    db::update_state(&pool, task, task_state, 2).await.unwrap();
    let run = pdb::insert_run(&pool, "/repo", "main", "test", 1)
        .await
        .unwrap();
    assert!(
        pdb::claim_run_integration(&pool, run, task, "branch", "/repo", 2)
            .await
            .unwrap()
    );
    sqlx::query("UPDATE pipeline_runs SET state = ? WHERE id = ?")
        .bind(run_state)
        .bind(run)
        .execute(&pool)
        .await
        .unwrap();
    let state = AppState::default();
    *state.pool.lock().unwrap() = Some(pool);
    (state, task, run)
}

fn app() -> tauri::App<tauri::test::MockRuntime> {
    tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap()
}

#[tokio::test]
async fn created_integration_removal_cancels_run_and_repeated_discard_succeeds() {
    let app = app();
    let (state, task, run) = fixture(tstate::CREATED, "awaiting_plan_approval").await;
    let pool = pool_of(&state).unwrap();
    // 문제의 세션에는 PTY가 없어서 일반 중단 경로가 실패한다.
    assert!(cancel_task_inner(&state, task)
        .await
        .unwrap_err()
        .contains("터미널 세션이 없습니다"));

    cancel_task_with_app(app.handle(), &state, task)
        .await
        .unwrap();
    discard_task_with_app(app.handle(), &state, task)
        .await
        .unwrap();
    discard_task_with_app(app.handle(), &state, task)
        .await
        .unwrap();

    assert_eq!(
        pdb::get_run(&pool, run).await.unwrap().unwrap().state,
        "cancelled"
    );
    assert_eq!(
        db::get_task(&pool, task).await.unwrap().unwrap().state,
        tstate::DISCARDED
    );
    assert_eq!(
        pdb::pipeline_guard_for_task(&pool, task).await.unwrap(),
        None
    );
    pool.close().await;
}

#[tokio::test]
async fn review_integration_discard_cancels_the_active_run() {
    let app = app();
    let (state, task, run) = fixture(tstate::AWAITING_REVIEW, "paused").await;
    let pool = pool_of(&state).unwrap();

    discard_task_with_app(app.handle(), &state, task)
        .await
        .unwrap();

    assert_eq!(
        pdb::get_run(&pool, run).await.unwrap().unwrap().state,
        "cancelled"
    );
    assert_eq!(
        db::get_task(&pool, task).await.unwrap().unwrap().state,
        tstate::DISCARDED
    );
    pool.close().await;
}

#[tokio::test]
async fn final_approval_discard_uses_normal_review_cleanup() {
    let app = app();
    let (state, task, run) = fixture(tstate::AWAITING_REVIEW, "awaiting_merge_approval").await;
    let pool = pool_of(&state).unwrap();

    discard_task_with_app(app.handle(), &state, task)
        .await
        .unwrap();

    assert_eq!(
        pdb::get_run(&pool, run).await.unwrap().unwrap().state,
        "awaiting_merge_approval"
    );
    assert_eq!(
        db::get_task(&pool, task).await.unwrap().unwrap().state,
        tstate::DISCARDED
    );
    assert!(db::recent_events(&pool, task, 10)
        .await
        .unwrap()
        .iter()
        .any(|event| event.kind == "discarded"));
    pool.close().await;
}
