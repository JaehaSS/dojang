use super::*;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

async fn pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL); INSERT INTO settings VALUES('notification_source_id','source'); CREATE TABLE tasks (id INTEGER PRIMARY KEY, repo TEXT NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    migrate(&pool).await.unwrap();
    sqlx::query("INSERT INTO tasks(id, repo) VALUES (1, '/repo'), (2, '/repo'), (3, '/repo'), (9, '/other')").execute(&pool).await.unwrap();
    pool
}
fn plan() -> ProjectPlan {
    ProjectPlan {
        schema_version: 2,
        source_id: String::new(),
        revision: 0,
        repo: "/ignored".into(),
        objective: "project why".into(),
        phases: vec![Phase {
            id: "plan".into(),
            name: "Plan".into(),
            objective: "phase why".into(),
        }],
        task_bindings: vec![TaskBinding {
            task_id: 1,
            phase_id: "plan".into(),
        }],
        dependencies: vec![],
    }
}

#[tokio::test]
async fn save_is_cas_scoped_and_preserves_existing_dangling_references() {
    let pool = pool().await;
    let first = save(&pool, "source", "/repo", plan(), 0, "source", 1)
        .await
        .unwrap();
    assert_eq!(first.revision, 1);
    let conflict = save(&pool, "source", "/repo", plan(), 0, "source", 2)
        .await
        .unwrap_err();
    assert!(matches!(conflict, PlanError::RevisionConflict { .. }));
    sqlx::query("DELETE FROM tasks WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    let mut next = first.clone();
    next.objective = "edited".into();
    let saved = save(&pool, "source", "/repo", next, 1, "source", 3)
        .await
        .unwrap();
    assert_eq!(saved.revision, 2);
    let mut outside = saved.clone();
    outside.task_bindings.push(TaskBinding {
        task_id: 9,
        phase_id: "plan".into(),
    });
    assert!(matches!(
        save(&pool, "source", "/repo", outside, 2, "source", 4).await,
        Err(PlanError::TaskScope { task_id: 9 })
    ));
}

#[tokio::test]
async fn legacy_import_is_idempotent_and_never_merges_a_different_raw_value() {
    let pool = pool().await;
    let raw = r#"{"version":1,"phases":["계획"],"phaseByTask":{"1":"계획"},"dependencies":[]}"#;
    let imported = import_v1(&pool, "source", "/repo", raw, 1).await.unwrap();
    assert!(matches!(imported, ImportOutcome::Imported { .. }));
    assert!(matches!(
        import_v1(&pool, "source", "/repo", raw, 2).await.unwrap(),
        ImportOutcome::AlreadyImported { .. }
    ));
    let other = r#"{"version":1,"phases":["구현"],"phaseByTask":{},"dependencies":[]}"#;
    assert!(matches!(
        import_v1(&pool, "source", "/repo", other, 3).await.unwrap(),
        ImportOutcome::ExistingPlanConflict { .. }
    ));
}

#[tokio::test]
async fn corrupt_and_cyclic_legacy_data_is_rejected_before_any_plan_is_created() {
    let pool = pool().await;
    assert!(matches!(
        import_v1(&pool, "source", "/repo", "not json", 1).await,
        Err(PlanError::ImportInvalid(_))
    ));
    let cycle = r#"{"version":1,"phases":["계획"],"phaseByTask":{"1":"계획","2":"계획"},"dependencies":[{"from":1,"to":2},{"from":2,"to":1}]}"#;
    assert!(matches!(
        import_v1(&pool, "source", "/repo", cycle, 2).await,
        Err(PlanError::ImportInvalid(_))
    ));
    assert!(get(&pool, "source", "/repo").await.unwrap().is_none());
}

#[tokio::test]
async fn purpose_snapshots_are_immutable_and_refresh_is_for_the_next_round_only() {
    let pool = pool().await;
    let saved = save(&pool, "source", "/repo", plan(), 0, "source", 1)
        .await
        .unwrap();
    let first = capture_for_execution_round(
        &pool,
        "source",
        1,
        "round-1",
        "/repo",
        Some("plan"),
        Some(saved.revision),
        None,
        2,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(first.project_objective, "project why");
    let mut changed = saved.clone();
    changed.objective = "new why".into();
    let changed = save(
        &pool,
        "source",
        "/repo",
        changed,
        saved.revision,
        "source",
        3,
    )
    .await
    .unwrap();
    assert!(matches!(
        capture_for_execution_round(
            &pool,
            "source",
            1,
            "round-2",
            "/repo",
            Some("plan"),
            Some(saved.revision),
            None,
            4
        )
        .await,
        Err(PlanError::PurposeConflict { .. })
    ));
    request_refresh(
        &pool,
        "source",
        1,
        "/repo",
        changed.revision,
        Some("plan"),
        5,
    )
    .await
    .unwrap();
    let next = capture_for_execution_round(
        &pool,
        "source",
        1,
        "round-2",
        "/repo",
        None,
        None,
        Some("round-1"),
        6,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(next.project_objective, "new why");
    assert_eq!(
        purpose_get(&pool, "source", 1, "round-1")
            .await
            .unwrap()
            .unwrap(),
        first
    );
}

#[tokio::test]
async fn initial_phase_binding_is_added_with_a_plan_cas_while_its_text_is_frozen() {
    let pool = pool().await;
    let saved = save(&pool, "source", "/repo", plan(), 0, "source", 1)
        .await
        .unwrap();
    bind_initial(
        &pool,
        2,
        &PurposeSelection {
            source_id: "source".into(),
            canonical_repo: "/repo".into(),
            plan_revision: saved.revision,
            phase_id: Some("plan".into()),
        },
        2,
    )
    .await
    .unwrap();
    let live = get(&pool, "source", "/repo").await.unwrap().unwrap();
    assert_eq!(live.revision, 2);
    assert!(live
        .task_bindings
        .iter()
        .any(|binding| binding.task_id == 2));
    let prepared = prepare_purpose(&pool, 2, "round-2", 3)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(prepared.snapshot().project_objective, "project why");
}

#[tokio::test]
async fn admission_rollback_preserves_refresh_and_resume_keeps_its_own_latest_purpose() {
    let pool = pool().await;
    let saved = save(&pool, "source", "/repo", plan(), 0, "source", 1)
        .await
        .unwrap();
    bind_initial(
        &pool,
        1,
        &PurposeSelection {
            source_id: "source".into(),
            canonical_repo: "/repo".into(),
            plan_revision: saved.revision,
            phase_id: None,
        },
        2,
    )
    .await
    .unwrap();
    let first = prepare_purpose(&pool, 1, "parent", 3)
        .await
        .unwrap()
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    persist_purpose_tx(&mut tx, &first).await.unwrap();
    tx.commit().await.unwrap();
    inherit_task(&pool, 1, 2, 4).await.unwrap();
    let mut next = saved;
    next.objective = "new objective".into();
    let next = save(&pool, "source", "/repo", next, 1, "source", 5)
        .await
        .unwrap();
    request_refresh(&pool, "source", 2, "/repo", next.revision, None, 6)
        .await
        .unwrap();
    let candidate = prepare_purpose(&pool, 2, "child", 7)
        .await
        .unwrap()
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    persist_purpose_tx(&mut tx, &candidate).await.unwrap();
    tx.rollback().await.unwrap();
    assert!(purpose_get(&pool, "source", 2, "child")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM purpose_refresh_requests WHERE task_id=2"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    let mut tx = pool.begin().await.unwrap();
    persist_purpose_tx(&mut tx, &candidate).await.unwrap();
    tx.commit().await.unwrap();
    let inherited = prepare_purpose(&pool, 2, "child-next", 7)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(inherited.snapshot().project_objective, "new objective");
    sqlx::query("UPDATE settings SET value='reset' WHERE key='notification_source_id'")
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert!(persist_purpose_tx(&mut tx, &inherited).await.is_err());
    tx.rollback().await.unwrap();
    assert!(save(
        &pool,
        "source",
        "/repo",
        next.clone(),
        next.revision,
        "source",
        8
    )
    .await
    .is_err());
}
