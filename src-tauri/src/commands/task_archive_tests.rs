use super::*;
use tauri::Manager;

#[tokio::test]
async fn archived_orphan_stays_out_of_sweep_and_flattened_list_keeps_history() {
    let path = crate::testtmp::dir().join("task-archive-command.sqlite");
    let pool = db::init_pool(path.to_str().unwrap()).await.unwrap();
    let id = db::insert_task(
        &pool,
        "/missing/repo",
        "branch",
        "main",
        "/missing/task-archive",
        "history",
        None,
        None,
        "conversation",
        1,
    )
    .await
    .unwrap();
    db::update_state(&pool, id, tstate::AWAITING_REVIEW, 2)
        .await
        .unwrap();
    crate::task_archive::archive(&pool, id, 3).await.unwrap();
    let state = AppState::default();
    *state.pool.lock().unwrap() = Some(pool.clone());
    // Rechecking after the finalization claim closes a concurrent archive against orphan cleanup.
    let error = discard_task(&state, id, BranchDisposal::Preserve)
        .await
        .unwrap_err();
    assert!(error.contains("보관한 작업"));
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    app.manage(state);
    let entries = task_list(app.state::<AppState>()).await.unwrap();
    let row = serde_json::to_value(&entries[0]).unwrap();
    assert_eq!(row["id"], id);
    assert_eq!(row["instruction"], "history");
    assert_eq!(row["state"], tstate::AWAITING_REVIEW);
    assert_eq!(row["archive"]["archived_at"], 3);
    assert_eq!(row["archive"]["worktree_exists"], false);
    let sweep = tasks_discard_orphans(app.state::<AppState>())
        .await
        .unwrap();
    assert!(sweep.retired.is_empty());
    assert!(sweep.failed.is_empty());
    assert!(crate::task_archive::is_archived(&pool, id).await.unwrap());
    assert_eq!(
        db::get_task(&pool, id).await.unwrap().unwrap().state,
        tstate::AWAITING_REVIEW
    );
    pool.close().await;
}
