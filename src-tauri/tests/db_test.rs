//! DB(Task 영속화) 통합 테스트. `cargo test`

#[path = "support/temp_root.rs"]
mod temp_root;

use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::db::{self, state};
use praxis_lib::goal_contract::GoalContract;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

static COUNTER: AtomicU32 = AtomicU32::new(0);

#[tokio::test]
async fn service_tier_persists_resumes_and_rejects_stale_model_writes() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.unwrap();
    let id = db::insert_task(&pool, "/r", "b", "main", "/w", "i", Some("codex"), None, "conversation", 1).await.unwrap();
    let old = db::get_task(&pool, id).await.unwrap().unwrap();
    assert_eq!(old.service_tier, None);
    // An explicit Standard choice also survives the empty-string -> NULL model normalization.
    db::set_task_model(&pool, id, "").await.unwrap();
    let standard = db::get_task(&pool, id).await.unwrap().unwrap();
    assert!(db::set_task_service_tier(&pool, &standard, "default").await.unwrap());
    let empty_model_heir = db::insert_task(&pool, "/r", "empty", "main", "/we", "i", Some("codex"), None, "conversation", 2).await.unwrap();
    db::adopt_conversation(&pool, empty_model_heir, id, 3).await.unwrap();
    assert_eq!(db::get_task(&pool, empty_model_heir).await.unwrap().unwrap().service_tier.as_deref(), Some("default"));

    db::set_task_model(&pool, id, "gpt-6-astra").await.unwrap();
    assert!(!db::set_task_service_tier(&pool, &old, "fast").await.unwrap());
    let task = db::get_task(&pool, id).await.unwrap().unwrap();
    assert!(db::set_task_service_tier(&pool, &task, "fast").await.unwrap());
    pool.close().await;
    let pool = db::init_pool(&path).await.unwrap();
    let task = db::get_task(&pool, id).await.unwrap().unwrap();
    assert_eq!(task.service_tier.as_deref(), Some("fast"));

    let heir = db::insert_task(&pool, "/r", "b2", "main", "/w2", "i2", Some("codex"), None, "conversation", 2).await.unwrap();
    db::set_task_model(&pool, heir, "gpt-6-astra").await.unwrap();
    db::set_convo_session(&pool, id, "same-thread").await.unwrap();
    assert!(db::adopt_conversation(&pool, heir, id, 3).await.unwrap());
    assert_eq!(db::get_task(&pool, heir).await.unwrap().unwrap().service_tier.as_deref(), Some("fast"));
    assert_eq!(db::get_task(&pool, id).await.unwrap().unwrap().service_tier.as_deref(), Some("fast"));

    db::set_task_model_for_agent(&pool, id, "codex", "another-model", true, 4).await.unwrap();
    assert!(!db::set_task_service_tier(&pool, &task, "fast").await.unwrap());
    let current = db::get_task(&pool, id).await.unwrap().unwrap();
    assert_eq!(current.service_tier.as_deref(), Some("default"));
    assert!(db::set_task_service_tier(&pool, &current, "default").await.unwrap());
    assert!(db::set_task_service_tier(&pool, &current, "invalid").await.is_err());

    sqlx::query("INSERT INTO convo_debate_sides(task_id,side,agent) VALUES(?, 'right', 'claude')").bind(id).execute(&pool).await.unwrap();
    assert!(!db::set_task_service_tier(&pool, &current, "fast").await.unwrap());
    pool.close().await;
    let _ = std::fs::remove_file(path);
}

fn temp_db() -> String {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    temp_root::dir()
        .join(format!(
            "praxis-db-test-{}-{}.sqlite",
            std::process::id(),
            n
        ))
        .to_string_lossy()
        .into_owned()
}

#[tokio::test]
async fn insert_and_get_task() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let id = db::insert_task(
        &pool,
        "/repo",
        "praxis/x",
        "main",
        "/repo/.praxis/wt/x",
        "do thing",
        None,
        None,
        "terminal",
        1000,
    )
    .await
    .expect("insert");
    let task = db::get_task(&pool, id).await.unwrap().expect("task exists");
    assert_eq!(task.repo, "/repo");
    assert_eq!(task.branch, "praxis/x");
    assert_eq!(task.state, state::CREATED);
    assert_eq!(task.created_at, 1000);
    assert_eq!(task.mode, "terminal");
    assert_eq!(task.role, "implementer");
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn explicit_role_round_trips_and_unknown_role_is_rejected() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let id = db::insert_task_with_role_and_goal_contract(
        &pool,
        "/repo",
        "praxis/test",
        "main",
        "/repo/wt",
        "verify behavior",
        Some("codex"),
        "tester",
        None,
        None,
        None,
        "conversation",
        None,
        None,
        1000,
    )
    .await
    .expect("insert with role");

    assert_eq!(
        db::get_task(&pool, id).await.unwrap().unwrap().role,
        "tester"
    );
    let error = db::insert_task_with_role_and_goal_contract(
        &pool,
        "/repo",
        "praxis/invalid-role",
        "main",
        "/repo/wt",
        "manage",
        None,
        "manager",
        None,
        None,
        None,
        "terminal",
        None,
        None,
        1001,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("planner"));
    let _ = std::fs::remove_file(&path);
}

fn goal_contract() -> GoalContract {
    GoalContract {
        schema_version: 1,
        objective: "Persist the task goal".into(),
        acceptance: vec!["round-trip succeeds".into()],
        stop_conditions: vec!["focused tests pass".into()],
        must_preserve: vec!["legacy tasks".into()],
        protected_paths: vec!["deploy/**".into()],
        non_goals: vec!["automatic approval".into()],
    }
}

#[tokio::test]
async fn goal_contract_round_trips_atomically_with_task() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let contract = goal_contract();

    let id = db::insert_task_with_goal_contract(
        &pool,
        "/repo",
        "praxis/goal",
        "main",
        "/repo/.praxis/wt/goal",
        "legacy instruction",
        None,
        None,
        None,
        None,
        "conversation",
        Some(&contract),
        None,
        1000,
    )
    .await
    .expect("insert with contract");

    let task = db::get_task(&pool, id).await.unwrap().unwrap();
    assert_eq!(task.goal_contract.as_deref(), Some(&contract));
    assert_eq!(
        db::list_tasks(&pool).await.unwrap()[0]
            .goal_contract
            .as_deref(),
        Some(&contract)
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn ambiguity_round_trips_and_defaults_to_null() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let score = praxis_lib::interview::AmbiguityScore::from_dimensions(0.9, 0.6, 0.4);

    let with = db::insert_task_with_goal_contract(
        &pool,
        "/repo",
        "praxis/amb",
        "main",
        "/repo/wt",
        "i",
        None,
        None,
        None,
        None,
        "conversation",
        None,
        Some(&score),
        1000,
    )
    .await
    .expect("insert with ambiguity");
    let loaded = db::get_task(&pool, with).await.unwrap().unwrap();
    assert_eq!(loaded.ambiguity.as_deref(), Some(&score));

    let without = db::insert_task(
        &pool,
        "/repo",
        "praxis/amb2",
        "main",
        "/repo/wt",
        "i",
        None,
        None,
        "terminal",
        1001,
    )
    .await
    .expect("insert without");
    assert!(db::get_task(&pool, without)
        .await
        .unwrap()
        .unwrap()
        .ambiguity
        .is_none());
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn unsupported_goal_contract_is_rejected_at_the_read_boundary() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let id = db::insert_task(
        &pool,
        "/repo",
        "praxis/invalid",
        "main",
        "/repo/wt",
        "legacy",
        None,
        None,
        "terminal",
        1,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE tasks SET goal_contract = ? WHERE id = ?")
        .bind(r#"{"schema_version":2,"objective":"future","acceptance":[],"stop_conditions":[],"must_preserve":[],"protected_paths":[],"non_goals":[]}"#)
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();

    assert!(db::get_task(&pool, id)
        .await
        .unwrap_err()
        .to_string()
        .contains("schema_version"));
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn legacy_task_schema_migrates_to_nullable_goal_contract() {
    let path = temp_db();
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let legacy = SqlitePoolOptions::new()
        .connect_with(options)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE tasks (\
          id INTEGER PRIMARY KEY AUTOINCREMENT, repo TEXT NOT NULL, branch TEXT NOT NULL, \
          base TEXT NOT NULL, worktree_path TEXT NOT NULL, instruction TEXT NOT NULL, \
          state TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, \
          agent TEXT, ensemble TEXT, model TEXT, mode TEXT NOT NULL DEFAULT 'terminal', \
          convo_session_id TEXT, convo_pgid INTEGER)",
    )
    .execute(&legacy)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO tasks (repo, branch, base, worktree_path, instruction, state, created_at, updated_at) \
         VALUES ('/repo', 'legacy', 'main', '/repo', 'legacy instruction', 'Created', 1, 1)",
    )
    .execute(&legacy)
    .await
    .unwrap();
    legacy.close().await;

    let pool = db::init_pool(&path).await.expect("migrate legacy DB");
    let task = db::list_tasks(&pool).await.unwrap().pop().unwrap();
    assert_eq!(task.role, "implementer", "구버전 행은 구현 역할로 보강");
    assert!(task.goal_contract.is_none());
    assert!(task.ambiguity.is_none(), "구버전 행은 ambiguity NULL");
    assert!(
        task.reasoning_effort.is_none(),
        "구버전 행은 reasoning_effort NULL"
    );
    pool.close().await;

    db::init_pool(&path).await.expect("migration is idempotent");
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn mark_awaiting_review_guards_terminal_states() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    // Running → AwaitingReview (전이 성공).
    let a = db::insert_task(
        &pool,
        "/r",
        "a",
        "main",
        "/p",
        "i",
        None,
        None,
        "conversation",
        1,
    )
    .await
    .unwrap();
    db::update_state(&pool, a, state::RUNNING, 2).await.unwrap();
    db::mark_awaiting_review(&pool, a, 3, None).await.unwrap();
    assert_eq!(
        db::get_task(&pool, a).await.unwrap().unwrap().state,
        state::AWAITING_REVIEW
    );
    // Done → (변화 없음: 승인/폐기 레이스 방지).
    let b = db::insert_task(
        &pool,
        "/r",
        "b",
        "main",
        "/p",
        "i",
        None,
        None,
        "conversation",
        4,
    )
    .await
    .unwrap();
    db::update_state(&pool, b, state::DONE, 5).await.unwrap();
    db::mark_awaiting_review(&pool, b, 6, None).await.unwrap();
    assert_eq!(
        db::get_task(&pool, b).await.unwrap().unwrap().state,
        state::DONE,
        "Done은 덮이지 않음"
    );
    let _ = std::fs::remove_file(&path);
}

/// 대기 주석은 매 전이마다 덮어써야 한다 — 직전 턴의 "질문 대기"가 남으면
/// 작업을 끝낸 턴이 계속 답변 대기로 보인다.
#[tokio::test]
async fn awaiting_kind_is_rewritten_every_transition() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let id = db::insert_task(
        &pool,
        "/r",
        "a",
        "main",
        "/p",
        "i",
        None,
        None,
        "conversation",
        1,
    )
    .await
    .unwrap();

    // 질문으로 끝난 턴.
    db::update_state(&pool, id, state::RUNNING, 2)
        .await
        .unwrap();
    db::mark_awaiting_review(&pool, id, 3, Some(db::awaiting_kind::QUESTION))
        .await
        .unwrap();
    assert_eq!(
        db::get_task(&pool, id)
            .await
            .unwrap()
            .unwrap()
            .awaiting_kind,
        Some(db::awaiting_kind::QUESTION.to_string())
    );

    // 후속 턴 시작 — 실행 중 작업에 주석이 남으면 안 된다.
    assert!(db::mark_running_from_review(&pool, id, 4).await.unwrap());
    assert_eq!(
        db::get_task(&pool, id)
            .await
            .unwrap()
            .unwrap()
            .awaiting_kind,
        None,
        "Running 전이가 주석을 지운다"
    );

    // 이번엔 작업을 하고 끝낸 턴 — 이전 주석이 되살아나면 안 된다.
    db::mark_awaiting_review(&pool, id, 5, None).await.unwrap();
    let task = db::get_task(&pool, id).await.unwrap().unwrap();
    assert_eq!(task.state, state::AWAITING_REVIEW);
    assert_eq!(task.awaiting_kind, None);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn mark_running_from_review_only_from_awaiting() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    // AwaitingReview → Running (전이 성공, true).
    let a = db::insert_task(
        &pool,
        "/r",
        "a",
        "main",
        "/p",
        "i",
        None,
        None,
        "conversation",
        1,
    )
    .await
    .unwrap();
    db::update_state(&pool, a, state::AWAITING_REVIEW, 2)
        .await
        .unwrap();
    assert!(db::mark_running_from_review(&pool, a, 3).await.unwrap());
    assert_eq!(
        db::get_task(&pool, a).await.unwrap().unwrap().state,
        state::RUNNING
    );
    // Done → 전이 안 됨(false), Done 유지(동시 approve 부활 방지).
    let b = db::insert_task(
        &pool,
        "/r",
        "b",
        "main",
        "/p",
        "i",
        None,
        None,
        "conversation",
        4,
    )
    .await
    .unwrap();
    db::update_state(&pool, b, state::DONE, 5).await.unwrap();
    assert!(!db::mark_running_from_review(&pool, b, 6).await.unwrap());
    assert_eq!(
        db::get_task(&pool, b).await.unwrap().unwrap().state,
        state::DONE
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn review_finalization_claim_is_single_owner_and_restorable() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let reviewable = db::insert_task(
        &pool,
        "/r",
        "reviewable",
        "main",
        "/p",
        "i",
        None,
        None,
        "terminal",
        1,
    )
    .await
    .unwrap();
    let running = db::insert_task(
        &pool, "/r", "running", "main", "/p", "i", None, None, "terminal", 1,
    )
    .await
    .unwrap();
    db::update_state(&pool, reviewable, state::AWAITING_REVIEW, 2)
        .await
        .unwrap();
    db::update_state(&pool, running, state::RUNNING, 2)
        .await
        .unwrap();

    assert!(db::claim_review_finalization(&pool, reviewable, 3)
        .await
        .unwrap());
    assert_eq!(
        db::get_task(&pool, reviewable)
            .await
            .unwrap()
            .unwrap()
            .state,
        state::FINALIZING
    );
    assert!(
        !db::claim_review_finalization(&pool, reviewable, 4)
            .await
            .unwrap(),
        "이미 점유된 review는 두 번째 finalizer가 가져갈 수 없다"
    );
    assert!(
        !db::claim_review_finalization(&pool, running, 4)
            .await
            .unwrap(),
        "Running 작업은 승인/폐기할 수 없다"
    );

    assert!(db::restore_awaiting_review(&pool, reviewable, 5)
        .await
        .unwrap());
    assert_eq!(
        db::get_task(&pool, reviewable)
            .await
            .unwrap()
            .unwrap()
            .state,
        state::AWAITING_REVIEW
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn update_state_changes_state() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let id = db::insert_task(
        &pool, "/r", "b", "main", "/p", "i", None, None, "terminal", 1,
    )
    .await
    .unwrap();
    db::update_state(&pool, id, state::AWAITING_REVIEW, 2)
        .await
        .unwrap();
    let task = db::get_task(&pool, id).await.unwrap().unwrap();
    assert_eq!(task.state, state::AWAITING_REVIEW);
    assert_eq!(task.updated_at, 2);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn count_done_tasks_scoped() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let a = db::insert_task(
        &pool, "/r", "a", "main", "/p", "i", None, None, "terminal", 1,
    )
    .await
    .unwrap();
    db::insert_task(
        &pool, "/r", "b", "main", "/p", "i", None, None, "terminal", 2,
    )
    .await
    .unwrap();
    db::update_state(&pool, a, state::DONE, 3).await.unwrap();
    assert_eq!(db::count_done_tasks(&pool, "/r").await.unwrap(), 1);
    assert_eq!(db::count_done_tasks(&pool, "/other").await.unwrap(), 0);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn known_repos_distinct() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    db::insert_task(
        &pool, "/r1", "a", "main", "/p", "i", None, None, "terminal", 1,
    )
    .await
    .unwrap();
    db::insert_task(
        &pool, "/r1", "b", "main", "/p", "i", None, None, "terminal", 2,
    )
    .await
    .unwrap();
    db::insert_task(
        &pool, "/r2", "c", "main", "/p", "i", None, None, "terminal", 3,
    )
    .await
    .unwrap();
    let mut repos = db::known_repos(&pool).await.unwrap();
    repos.sort();
    assert_eq!(repos, vec!["/r1".to_string(), "/r2".to_string()]);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn evidence_upsert_and_get() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    assert!(db::get_evidence(&pool, 1).await.unwrap().is_none());
    db::upsert_evidence(
        &pool,
        1,
        "cargo build",
        0,
        "cargo test",
        0,
        12,
        0,
        true,
        100,
    )
    .await
    .unwrap();
    let e = db::get_evidence(&pool, 1).await.unwrap().expect("evidence");
    assert_eq!(e.passed, 12);
    assert!(e.ready);
    db::upsert_evidence(
        &pool,
        1,
        "cargo build",
        0,
        "cargo test",
        1,
        10,
        2,
        false,
        200,
    )
    .await
    .unwrap();
    let e = db::get_evidence(&pool, 1).await.unwrap().unwrap();
    assert_eq!(e.failed, 2);
    assert!(!e.ready, "재실행은 덮어씀");
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn task_events_append_and_recent_desc() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    db::append_event(&pool, 7, "running", None, 100)
        .await
        .unwrap();
    db::append_event(&pool, 7, "verify", Some("ready"), 200)
        .await
        .unwrap();
    db::append_event(&pool, 9, "running", None, 150)
        .await
        .unwrap();
    let ev = db::recent_events(&pool, 7, 8).await.unwrap();
    assert_eq!(ev.len(), 2, "task 7만");
    assert_eq!(ev[0].kind, "verify", "최신순");
    assert_eq!(ev[0].detail.as_deref(), Some("ready"));
    assert!(db::has_task_event(&pool, 7, "verify").await.unwrap());
    assert!(!db::has_task_event(&pool, 7, "mcp_generated").await.unwrap());
    let _ = std::fs::remove_file(&path);
}

/// 계측 행은 에이전트 컨텍스트(캡슐)로 들어가는 목록에서 빠져야 한다 — 사람이 읽을 일 없는
/// 소요 시간 JSON이 섞이면 맥락만 밀어낸다(ADR 0174).
#[tokio::test]
async fn recent_events_hides_metric_rows() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    db::append_event(&pool, 11, "running", None, 100)
        .await
        .unwrap();
    db::append_event(&pool, 11, "metric.prepared", Some("{\"prep_ms\":10}"), 200)
        .await
        .unwrap();
    // 실패한 생성의 계측 행도 같은 접두를 쓴다 — 캡슐이 최근 8건을 에이전트 컨텍스트에
    // 넣으므로(ADR 0174) 새 kind가 그 자리를 차지하면 안 된다.
    db::append_event(
        &pool,
        11,
        "metric.create_failed",
        Some("{\"prep_ms\":30120,\"failed_at\":\"memory\"}"),
        300,
    )
    .await
    .unwrap();
    let ev = db::recent_events(&pool, 11, 8).await.unwrap();
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].kind, "running");
    // 행 자체는 남아 있어야 한다 — 제외는 읽기 경로에만 건다.
    assert!(db::has_task_event(&pool, 11, "metric.prepared")
        .await
        .unwrap());
    assert!(db::has_task_event(&pool, 11, "metric.create_failed")
        .await
        .unwrap());
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn settings_get_set_upsert() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    assert!(db::get_setting(&pool, "capture_enabled")
        .await
        .unwrap()
        .is_none());
    db::set_setting(&pool, "capture_enabled", "true")
        .await
        .unwrap();
    assert_eq!(
        db::get_setting(&pool, "capture_enabled")
            .await
            .unwrap()
            .as_deref(),
        Some("true")
    );
    db::set_setting(&pool, "capture_enabled", "false")
        .await
        .unwrap(); // upsert
    assert_eq!(
        db::get_setting(&pool, "capture_enabled")
            .await
            .unwrap()
            .as_deref(),
        Some("false")
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn list_returns_desc_and_restore_marks_running_failed() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let a = db::insert_task(
        &pool, "/r", "a", "main", "/p", "i", None, None, "terminal", 1,
    )
    .await
    .unwrap();
    let b = db::insert_task(
        &pool, "/r", "b", "main", "/p", "i", None, None, "terminal", 2,
    )
    .await
    .unwrap();
    let c = db::insert_task(
        &pool, "/r", "c", "main", "/p", "i", None, None, "terminal", 3,
    )
    .await
    .unwrap();
    // b를 Running으로
    db::update_state(&pool, b, state::RUNNING, 3).await.unwrap();
    db::update_state(&pool, c, state::STARTING, 4)
        .await
        .unwrap();
    let list = db::list_tasks(&pool).await.unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(list[0].id, c, "DESC order: newest first");
    // 재시작 복원: Starting/Running → Failed
    let n = db::mark_stale_running_failed(&pool, 9).await.unwrap();
    assert_eq!(n, 2);
    let task_b = db::get_task(&pool, b).await.unwrap().unwrap();
    assert_eq!(task_b.state, state::FAILED);
    assert_eq!(
        db::get_task(&pool, c).await.unwrap().unwrap().state,
        state::FAILED
    );
    let task_a = db::get_task(&pool, a).await.unwrap().unwrap();
    assert_eq!(task_a.state, state::CREATED, "a was not running");
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn insert_task_agent_and_ensemble_round_trip() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.unwrap();
    let id = db::insert_task(
        &pool,
        "/r",
        "b",
        "main",
        "/p",
        "i",
        Some("claude"),
        Some("ens-001"),
        "conversation",
        1,
    )
    .await
    .unwrap();
    let t = db::get_task(&pool, id).await.unwrap().unwrap();
    assert_eq!(t.agent.as_deref(), Some("claude"));
    assert_eq!(t.ensemble.as_deref(), Some("ens-001"));
    assert_eq!(t.mode, "conversation");
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn set_task_model_round_trip() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.unwrap();
    let id = db::insert_task(
        &pool,
        "/r",
        "b",
        "main",
        "/p",
        "i",
        Some("claude"),
        None,
        "conversation",
        1,
    )
    .await
    .unwrap();
    // 기본은 NULL — 설정의 벤더 기본(`model:<agent>`)으로 폴백하는 근거.
    assert!(db::get_task(&pool, id)
        .await
        .unwrap()
        .unwrap()
        .model
        .is_none());
    db::set_task_model(&pool, id, "haiku").await.unwrap();
    let t = db::get_task(&pool, id).await.unwrap().unwrap();
    assert_eq!(t.model.as_deref(), Some("haiku"));
    // 재대입(오버라이드 교체) 허용.
    db::set_task_model(&pool, id, "opus").await.unwrap();
    assert_eq!(
        db::get_task(&pool, id)
            .await
            .unwrap()
            .unwrap()
            .model
            .as_deref(),
        Some("opus")
    );
    // 빈 문자열은 가드 없이 그대로 저장되는 계약(NULL 아님) — 폴백 판단은 읽기 쪽
    // (model_for_task의 trim 필터) 책임이며, 호출자는 빈 값이면 호출을 생략한다.
    db::set_task_model(&pool, id, "").await.unwrap();
    assert_eq!(
        db::get_task(&pool, id)
            .await
            .unwrap()
            .unwrap()
            .model
            .as_deref(),
        Some("")
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn convo_events_and_session_round_trip() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.unwrap();
    let id = db::insert_task(
        &pool,
        "/r",
        "b",
        "main",
        "/p",
        "i",
        None,
        None,
        "conversation",
        1,
    )
    .await
    .unwrap();
    assert!(db::list_convo_events(&pool, id).await.unwrap().is_empty());
    db::append_convo_event(&pool, id, r#"{"kind":"user","text":"hi"}"#, 2)
        .await
        .unwrap();
    db::append_convo_event(&pool, id, r#"{"kind":"text","text":"yo"}"#, 3)
        .await
        .unwrap();
    let evs = db::list_convo_events(&pool, id).await.unwrap();
    assert_eq!(evs.len(), 2);
    assert!(evs[0].contains("user"), "oldest first");
    // 세션 id 영속화 — 재시작 후 --resume 근거.
    assert!(db::get_task(&pool, id)
        .await
        .unwrap()
        .unwrap()
        .convo_session_id
        .is_none());
    db::set_convo_session(&pool, id, "sess-abc").await.unwrap();
    let t = db::get_task(&pool, id).await.unwrap().unwrap();
    assert_eq!(t.convo_session_id.as_deref(), Some("sess-abc"));
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn schedules_insert_list_toggle_remove_and_mark_ran() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    praxis_lib::schedule::migrate(&pool)
        .await
        .expect("schedule migrate");
    let id = db::insert_schedule(
        &pool,
        "daily",
        "0 9 * * * *",
        "reminder",
        r#"{"text":"hi"}"#,
        1000,
        32400,
    )
    .await
    .unwrap();
    let list = db::list_schedules(&pool).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].label, "daily");
    assert!(list[0].last_run_at.is_none());
    assert_eq!(
        db::list_enabled_schedules(&pool).await.unwrap().len(),
        1,
        "스모크: enabled 목록"
    );

    db::mark_schedule_ran(&pool, id, 2000).await.unwrap();
    assert_eq!(
        db::list_schedules(&pool).await.unwrap()[0].last_run_at,
        Some(2000)
    );

    db::set_schedule_enabled(&pool, id, false).await.unwrap();
    assert_eq!(db::list_schedules(&pool).await.unwrap()[0].enabled, 0);
    assert!(
        db::list_enabled_schedules(&pool).await.unwrap().is_empty(),
        "스모크: 비활성 제외"
    );

    db::remove_schedule(&pool, id).await.unwrap();
    assert!(
        db::list_schedules(&pool).await.unwrap().is_empty(),
        "스모크: 삭제"
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn schedule_run_at_round_trip_for_one_shot_reminder() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    praxis_lib::schedule::migrate(&pool)
        .await
        .expect("schedule migrate");
    // run_at 없는 기존 cron 스케줄 — None으로 유지(회귀 없음).
    let cron_id = db::insert_schedule(
        &pool,
        "daily",
        "0 9 * * * *",
        "reminder",
        r#"{"text":"hi"}"#,
        1000,
        32400,
    )
    .await
    .unwrap();
    // run_at 있는 1회성 리마인더.
    let once_id = db::insert_schedule_with_run_at(
        &pool,
        "리마인더",
        "",
        "reminder",
        r#"{"text":"5분 후"}"#,
        Some(1300),
        1000,
        32400,
    )
    .await
    .unwrap();
    let list = db::list_schedules(&pool).await.unwrap();
    let cron_row = list.iter().find(|s| s.id == cron_id).unwrap();
    let once_row = list.iter().find(|s| s.id == once_id).unwrap();
    assert!(
        cron_row.run_at.is_none(),
        "기존 cron 스케줄은 run_at None 유지"
    );
    assert_eq!(once_row.run_at, Some(1300));
    let _ = std::fs::remove_file(&path);
}

/// 기존 사용자 DB(스키마 마이그레이션 이전, run_at 컬럼 없음)를 흉내내 `migrate()`를
/// 재실행해도 에러 없이 컬럼이 보강되는지 확인 — 멱등 마이그레이션 실증.
#[tokio::test]
async fn schedule_migrate_is_idempotent_and_backfills_run_at_column() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    // run_at 없는 구버전 스키마를 수동으로 생성(신규 MIGRATION 상수를 우회).
    sqlx::query(
        "CREATE TABLE schedules (\
           id INTEGER PRIMARY KEY AUTOINCREMENT, \
           label TEXT NOT NULL, \
           cron TEXT NOT NULL, \
           kind TEXT NOT NULL, \
           payload TEXT NOT NULL, \
           enabled INTEGER NOT NULL DEFAULT 1, \
           last_run_at INTEGER, \
           created_at INTEGER NOT NULL)",
    )
    .execute(&pool)
    .await
    .expect("create legacy schedules table");
    // 마이그레이션 최초 실행 — ALTER TABLE로 run_at 보강.
    praxis_lib::schedule::migrate(&pool)
        .await
        .expect("migrate backfills run_at");
    let id = db::insert_schedule_with_run_at(
        &pool,
        "리마인더",
        "",
        "reminder",
        "{}",
        Some(2000),
        1000,
        32400,
    )
    .await
    .expect("insert after backfill");
    assert_eq!(
        db::list_schedules(&pool).await.unwrap()[0].run_at,
        Some(2000)
    );
    // 재실행해도 에러 없이 통과(컬럼 중복 추가 에러는 내부에서 무시) — 멱등성.
    praxis_lib::schedule::migrate(&pool)
        .await
        .expect("migrate is idempotent on rerun");
    assert_eq!(db::list_schedules(&pool).await.unwrap().len(), 1);
    let _ = db::remove_schedule(&pool, id).await;
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn set_convo_pgid_round_trip() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.unwrap();
    let id = db::insert_task(
        &pool,
        "/r",
        "b",
        "main",
        "/p",
        "i",
        None,
        None,
        "conversation",
        1,
    )
    .await
    .unwrap();
    // 신규 작업은 pgid 없음.
    assert!(db::get_task(&pool, id)
        .await
        .unwrap()
        .unwrap()
        .convo_pgid
        .is_none());
    // 스폰 시 기록.
    db::set_convo_pgid(&pool, id, Some(4242)).await.unwrap();
    assert_eq!(
        db::get_task(&pool, id).await.unwrap().unwrap().convo_pgid,
        Some(4242)
    );
    // 완료 시 해제.
    db::set_convo_pgid(&pool, id, None).await.unwrap();
    assert!(db::get_task(&pool, id)
        .await
        .unwrap()
        .unwrap()
        .convo_pgid
        .is_none());
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn list_running_tasks_filters_by_state() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.unwrap();
    let a = db::insert_task(
        &pool,
        "/r",
        "a",
        "main",
        "/p",
        "i",
        None,
        None,
        "conversation",
        1,
    )
    .await
    .unwrap();
    let b = db::insert_task(
        &pool,
        "/r",
        "b",
        "main",
        "/p",
        "i",
        None,
        None,
        "conversation",
        2,
    )
    .await
    .unwrap();
    let c = db::insert_task(
        &pool, "/r", "c", "main", "/p", "i", None, None, "terminal", 3,
    )
    .await
    .unwrap();
    db::update_state(&pool, a, state::RUNNING, 4).await.unwrap();
    db::update_state(&pool, b, state::RUNNING, 5).await.unwrap();
    db::update_state(&pool, c, state::STARTING, 6)
        .await
        .unwrap();
    let running = db::list_running_tasks(&pool).await.unwrap();
    assert_eq!(running.len(), 3, "Starting/Running만");
    assert_eq!(running[0].id, a, "ASC order: oldest first");
    assert_eq!(running[1].id, b);
    assert_eq!(running[2].id, c);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn tasks_by_ensemble_filters_and_orders_ascending() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.unwrap();
    let a = db::insert_task(
        &pool,
        "/r",
        "a",
        "main",
        "/p",
        "i",
        Some("claude"),
        Some("ens-X"),
        "terminal",
        1,
    )
    .await
    .unwrap();
    let b = db::insert_task(
        &pool,
        "/r",
        "b",
        "main",
        "/p",
        "i",
        Some("codex"),
        Some("ens-X"),
        "terminal",
        2,
    )
    .await
    .unwrap();
    db::insert_task(
        &pool,
        "/r",
        "c",
        "main",
        "/p",
        "i",
        Some("claude"),
        Some("ens-Y"),
        "terminal",
        3,
    )
    .await
    .unwrap();
    let group = db::tasks_by_ensemble(&pool, "ens-X").await.unwrap();
    assert_eq!(group.len(), 2, "only ens-X members");
    assert_eq!(group[0].id, a, "ASC order: oldest first");
    assert_eq!(group[1].id, b);
    assert!(
        db::tasks_by_ensemble(&pool, "ens-none")
            .await
            .unwrap()
            .is_empty(),
        "no match -> empty"
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn quickopen_search_matches_case_insensitive_and_orders_by_recency() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.unwrap();
    let old = db::insert_task(
        &pool,
        "/repo",
        "b1",
        "main",
        "/wt1",
        "Fix Login Bug",
        None,
        None,
        "terminal",
        1000,
    )
    .await
    .unwrap();
    let recent = db::insert_task(
        &pool,
        "/repo",
        "b2",
        "main",
        "/wt2",
        "fix login redirect",
        None,
        None,
        "conversation",
        2000,
    )
    .await
    .unwrap();
    db::insert_task(
        &pool,
        "/repo",
        "b3",
        "main",
        "/wt3",
        "add dashboard widget",
        None,
        None,
        "terminal",
        3000,
    )
    .await
    .unwrap();

    let results = db::quickopen_search(&pool, "login", &[], 10).await.unwrap();
    assert_eq!(results.len(), 2, "unrelated task는 제외");
    assert_eq!(results[0].id, recent, "최근 수정(updated_at 2000)이 우선");
    assert_eq!(results[1].id, old);
    assert_eq!(results[0].scope, "session", "mode=conversation → session");
    assert_eq!(results[1].scope, "task", "mode=terminal → task");
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn quickopen_search_filters_by_scope_and_respects_limit() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.unwrap();
    db::insert_task(
        &pool,
        "/repo",
        "b1",
        "main",
        "/wt1",
        "task alpha",
        None,
        None,
        "terminal",
        1,
    )
    .await
    .unwrap();
    db::insert_task(
        &pool,
        "/repo",
        "b2",
        "main",
        "/wt2",
        "task beta",
        None,
        None,
        "conversation",
        2,
    )
    .await
    .unwrap();

    let tasks_only = db::quickopen_search(&pool, "task", &["task".to_string()], 10)
        .await
        .unwrap();
    assert_eq!(tasks_only.len(), 1);
    assert_eq!(tasks_only[0].scope, "task");

    let no_scope_match = db::quickopen_search(&pool, "task", &["skill".to_string()], 10)
        .await
        .unwrap();
    assert!(
        no_scope_match.is_empty(),
        "task/session 스코프가 아니면 DB 질의 자체를 건너뛴다"
    );

    let limited = db::quickopen_search(&pool, "task", &[], 1).await.unwrap();
    assert_eq!(limited.len(), 1, "limit 상한 적용");
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn quickopen_search_escapes_like_wildcards_in_query() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.unwrap();
    db::insert_task(
        &pool,
        "/repo",
        "b1",
        "main",
        "/wt1",
        "100% done",
        None,
        None,
        "terminal",
        1,
    )
    .await
    .unwrap();
    db::insert_task(
        &pool,
        "/repo",
        "b2",
        "main",
        "/wt2",
        "unrelated instruction",
        None,
        None,
        "terminal",
        2,
    )
    .await
    .unwrap();

    // '%'를 리터럴로 취급 — 와일드카드로 해석되면 두 번째 행도 매치되어 버린다.
    let results = db::quickopen_search(&pool, "100%", &[], 10).await.unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].title.contains("100%"));
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn recent_convo_events_returns_tail_in_chronological_order() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let id = db::insert_task(
        &pool,
        "/repo",
        "praxis/log",
        "main",
        "/wt",
        "로그 테스트",
        None,
        None,
        "conversation",
        1000,
    )
    .await
    .expect("insert");
    for (n, ts) in (1..=5).zip(1000..) {
        db::append_convo_event(
            &pool,
            id,
            &format!(r#"{{"kind":"user","text":"{n}"}}"#),
            ts,
        )
        .await
        .expect("append");
    }
    let tail = db::recent_convo_events(&pool, id, 2).await.expect("recent");
    assert_eq!(tail.len(), 2);
    assert!(tail[0].contains(r#""text":"4""#));
    assert!(tail[1].contains(r#""text":"5""#));

    let _ = std::fs::remove_file(&path);
}

/// 인자가 여덟이라 호출부마다 구조체를 펼치면 테스트가 읽히지 않는다.
#[allow(clippy::too_many_arguments)]
async fn record(
    pool: &sqlx::SqlitePool,
    op: &str,
    ok: bool,
    changed: Option<bool>,
    elapsed_ms: i64,
    error: Option<&str>,
    snapshot: Option<db::SnapshotMetrics>,
    now: i64,
) {
    db::record_preview_command(
        pool,
        db::PreviewCommandRecord {
            task_id: 42,
            op,
            ok,
            changed,
            elapsed_ms,
            error,
            snapshot,
            now,
        },
    )
    .await
    .unwrap_or_else(|e| panic!("{op} 기록 실패: {e}"));
}

fn metrics(bytes: i64, nodes: i64, truncated: bool, shrinks: i64) -> Option<db::SnapshotMetrics> {
    Some(db::SnapshotMetrics {
        bytes,
        nodes,
        truncated,
        shrinks,
    })
}

#[tokio::test]
async fn preview_command_rows_accumulate_and_changed_ratio_is_queryable() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    record(&pool, "click", true, Some(true), 120, None, None, 1).await;
    record(&pool, "click", true, Some(false), 90, None, None, 2).await;
    record(&pool, "snapshot", true, None, 30, None, None, 3).await;
    record(&pool, "snapshot", false, None, 2000, Some("timeout"), None, 4).await;

    let (n, changed): (i64, Option<f64>) =
        sqlx::query_as("SELECT COUNT(*), AVG(changed) FROM preview_commands WHERE op='click'")
            .fetch_one(&pool)
            .await
            .expect("query");
    assert_eq!(n, 2);
    assert_eq!(changed, Some(0.5));

    // AVG는 NULL을 세지 않는다 — op 필터 없이도 changed 비율은 기록된 두 건만으로 계산된다.
    let (all, changed_all): (i64, Option<f64>) =
        sqlx::query_as("SELECT COUNT(*), AVG(changed) FROM preview_commands")
            .fetch_one(&pool)
            .await
            .expect("query all");
    assert_eq!(all, 4);
    assert_eq!(changed_all, Some(0.5));

    let error: String = sqlx::query_scalar("SELECT error FROM preview_commands WHERE ok = 0")
        .fetch_one(&pool)
        .await
        .expect("query error");
    assert_eq!(error, "timeout");

    // CREATE 문이 멱등이라 같은 경로로 다시 열려도 실패하지 않는다.
    db::init_pool(&path).await.expect("re-init");

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn snapshot_cost_sums_bytes_per_op_and_ignores_rows_without_a_snapshot() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    record(&pool, "click", true, Some(true), 10, None, metrics(4_000, 200, false, 0), 1).await;
    record(&pool, "click", true, Some(true), 10, None, metrics(6_000, 300, true, 1), 2).await;
    record(&pool, "snapshot", true, None, 10, None, metrics(1_000, 50, false, 0), 3).await;
    // console은 스냅샷을 싣지 않고, 실패한 명령에는 본문 자체가 없다 — 둘 다 NULL로 남아야 한다.
    record(&pool, "console", true, None, 5, None, None, 4).await;
    record(&pool, "click", false, None, 20, Some("stale_ref"), None, 5).await;

    let cost = db::preview_snapshot_cost(&pool, 42).await.expect("cost");
    // total_bytes 내림차순 — 가장 비싼 op이 먼저 온다.
    assert_eq!(
        cost.iter().map(|c| c.op.as_str()).collect::<Vec<_>>(),
        ["click", "snapshot", "console"]
    );

    let click = &cost[0];
    assert_eq!(click.calls, 3);
    assert_eq!(click.failures, 1);
    // 스냅샷을 실은 두 건만 센다. 평균이 실패 행 때문에 3,333으로 떨어지면 안 된다.
    assert_eq!(click.snapshots, 2);
    assert_eq!(click.total_bytes, 10_000);
    assert_eq!(click.max_bytes, 6_000);
    assert_eq!(click.avg_bytes, 5_000.0);
    assert_eq!(click.avg_nodes, 250.0);
    assert_eq!(click.truncated, 1);
    assert_eq!(click.shrinks, 1);

    let console = cost.iter().find(|c| c.op == "console").expect("console");
    assert_eq!(console.calls, 1);
    assert_eq!(console.snapshots, 0);
    assert_eq!(console.total_bytes, 0);
    assert_eq!(console.avg_bytes, 0.0);

    // 다른 task의 계측은 섞이지 않는다.
    assert!(db::preview_snapshot_cost(&pool, 43)
        .await
        .expect("other task")
        .is_empty());

    let _ = std::fs::remove_file(&path);
}

/// 이어받기용 대화 작업 하나. 상태와 벤더 세션을 원하는 모양으로 세워 둔다.
async fn resumable_task(
    pool: &sqlx::SqlitePool,
    instruction: &str,
    state: &str,
    session: Option<&str>,
) -> i64 {
    let id = db::insert_task(
        pool,
        "/repo",
        "praxis/resume",
        "main",
        "/repo/.praxis/wt/resume",
        instruction,
        Some("claude"),
        None,
        "conversation",
        1000,
    )
    .await
    .expect("insert");
    db::update_state(pool, id, state, 1001).await.expect("state");
    if let Some(session) = session {
        db::set_convo_session(pool, id, session)
            .await
            .expect("session");
    }
    id
}

/// 이어받기의 핵심 계약 — 새 작업이 세션을 물려받되 **원본 행은 그대로 남는다**.
///
/// 원본이 움직이면 끝난 카드가 되살아난 것처럼 보이고, 거기 매달린 승인 diff·폐기 커밋의
/// 시점 기록이 한 번 더 흔들린다. 이 기능이 "되살리기"가 아니라 "이어받기"인 이유가 여기다.
#[tokio::test]
async fn adopt_conversation_moves_the_session_without_touching_the_source() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let source = resumable_task(&pool, "원본", state::DONE, Some("sess-source")).await;
    let heir = resumable_task(&pool, "이어받기", state::CREATED, None).await;

    let inherited = db::adopt_conversation(&pool, heir, source, 2000)
        .await
        .expect("adopt");
    assert!(inherited, "원본에 세션이 있으면 승계되었다고 답해야 한다");

    let heir_row = db::get_task(&pool, heir).await.unwrap().expect("heir");
    assert_eq!(
        heir_row.convo_session_id.as_deref(),
        Some("sess-source"),
        "벤더 문맥은 새 작업이 이어간다"
    );
    assert_eq!(heir_row.resumed_from, Some(source), "출처가 남아야 이력을 이을 수 있다");

    let source_row = db::get_task(&pool, source).await.unwrap().expect("source");
    assert_eq!(source_row.state, state::DONE, "원본은 끝난 그대로여야 한다");
    assert_eq!(
        source_row.convo_session_id.as_deref(),
        Some("sess-source"),
        "원본에서 세션을 뺏어 오면 원본 카드의 --resume 근거가 사라진다"
    );
    assert_eq!(source_row.resumed_from, None, "원본은 아무것도 이어받지 않았다");

    let _ = std::fs::remove_file(&path);
}

/// 세션이 없는 원본을 이어받아도 **이어받기 자체는 성립한다** — 반환값만 그 차이를 말한다.
///
/// 여기서 실패로 처리하면 첫 턴을 돌기 전에 끝난 대화(세션이 아직 없는 대화)는 영영 이어받을
/// 수 없게 된다. 화면상의 이력은 `resumed_from` 체인이 잇는다.
#[tokio::test]
async fn adopt_conversation_reports_no_session_but_still_records_the_origin() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let source = resumable_task(&pool, "세션 없는 원본", state::FAILED, None).await;
    let heir = resumable_task(&pool, "이어받기", state::CREATED, None).await;

    let inherited = db::adopt_conversation(&pool, heir, source, 2000)
        .await
        .expect("adopt");
    assert!(!inherited, "물려받을 세션이 없었음을 호출부가 알아야 한다");

    let heir_row = db::get_task(&pool, heir).await.unwrap().expect("heir");
    assert_eq!(heir_row.resumed_from, Some(source), "출처는 그래도 남는다");
    assert_eq!(heir_row.convo_session_id, None);

    let _ = std::fs::remove_file(&path);
}

/// 빈 문자열 세션은 세션이 아니다 — 공백만 든 칸을 승계 성공으로 답하면 호출부가
/// "옛 문맥을 이어간다"고 사용자에게 말하는데 벤더는 첫 턴을 돈다.
#[tokio::test]
async fn adopt_conversation_treats_a_blank_session_as_nothing_to_inherit() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let source = resumable_task(&pool, "공백 세션", state::DISCARDED, Some("   ")).await;
    let heir = resumable_task(&pool, "이어받기", state::CREATED, None).await;

    let inherited = db::adopt_conversation(&pool, heir, source, 2000)
        .await
        .expect("adopt");
    assert!(!inherited);
    assert_eq!(
        db::get_task(&pool, heir).await.unwrap().unwrap().resumed_from,
        Some(source)
    );

    let _ = std::fs::remove_file(&path);
}

/// 자기 자신을 이어받는 것은 즉시 에러다. 통과시키면 `resume_chain`이 스스로를 가리키는
/// 행을 만나고, 이력 조립이 자기 이벤트를 무한히 겹쳐 붙인다.
#[tokio::test]
async fn adopt_conversation_rejects_adopting_itself() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let id = resumable_task(&pool, "자기 자신", state::DONE, Some("sess-self")).await;

    let error = db::adopt_conversation(&pool, id, id, 2000)
        .await
        .expect_err("자기 자신은 거부");
    assert!(error.to_string().contains("자기 자신"), "{error}");
    assert_eq!(
        db::get_task(&pool, id).await.unwrap().unwrap().resumed_from,
        None,
        "거부된 호출은 아무것도 쓰지 않아야 한다"
    );

    let _ = std::fs::remove_file(&path);
}

/// 체인은 **오래된 것부터**여야 한다. 순서가 뒤집히면 이어붙인 대화가 시간을 거슬러 흐르고,
/// 사용자는 답이 질문보다 먼저 오는 화면을 본다.
#[tokio::test]
async fn resume_chain_walks_back_to_the_oldest_origin_in_order() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let a = resumable_task(&pool, "A", state::DONE, Some("sess-a")).await;
    let b = resumable_task(&pool, "B", state::DONE, None).await;
    let c = resumable_task(&pool, "C", state::CREATED, None).await;

    // 이어받기가 없으면 붙일 이력도 없다.
    assert!(db::resume_chain(&pool, a).await.expect("chain").is_empty());

    db::adopt_conversation(&pool, b, a, 2000).await.expect("b<-a");
    db::adopt_conversation(&pool, c, b, 2001).await.expect("c<-b");

    assert_eq!(db::resume_chain(&pool, c).await.expect("chain"), vec![a, b]);
    assert_eq!(db::resume_chain(&pool, b).await.expect("chain"), vec![a]);

    let _ = std::fs::remove_file(&path);
}

/// 고리가 생겨도 유한하게 끝나야 한다. `resumed_from`에는 외래키 제약이 없어 손으로 고친
/// DB나 앞으로 생길 다른 쓰기 경로가 고리를 만들 수 있고, 그러면 대화창을 여는 것만으로
/// 앱이 멈춘다.
#[tokio::test]
async fn resume_chain_stops_on_a_cycle_instead_of_spinning_forever() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let a = resumable_task(&pool, "A", state::DONE, None).await;
    let b = resumable_task(&pool, "B", state::DONE, None).await;

    for (id, parent) in [(a, b), (b, a)] {
        sqlx::query("UPDATE tasks SET resumed_from = ? WHERE id = ?")
            .bind(parent)
            .bind(id)
            .execute(&pool)
            .await
            .expect("고리를 직접 만든다");
    }

    let chain = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        db::resume_chain(&pool, a),
    )
    .await
    .expect("고리에서 돌아오지 못하면 앱이 멈춘 것과 같다")
    .expect("chain");
    // a -> b 까지만 따라가고 다시 a를 만나 멈춘다.
    assert_eq!(chain, vec![b]);
    // 자기 자신을 가리키는 행도 한 걸음도 나아가지 않는다.
    sqlx::query("UPDATE tasks SET resumed_from = ? WHERE id = ?")
        .bind(a)
        .bind(a)
        .execute(&pool)
        .await
        .expect("자기 고리");
    assert!(db::resume_chain(&pool, a).await.expect("chain").is_empty());

    let _ = std::fs::remove_file(&path);
}

/// 깊이 상한은 32다. 이력 조립이 체인 길이에 비례해 커지므로, 상한이 없으면 오래 쓴 대화
/// 하나가 세션을 여는 것만으로 수만 건을 읽는다.
#[tokio::test]
async fn resume_chain_stops_at_the_depth_limit() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let mut ids = Vec::new();
    for n in 0..40 {
        ids.push(resumable_task(&pool, &format!("T{n}"), state::DONE, None).await);
    }
    for window in ids.windows(2) {
        db::adopt_conversation(&pool, window[1], window[0], 2000)
            .await
            .expect("adopt");
    }

    let chain = db::resume_chain(&pool, *ids.last().unwrap())
        .await
        .expect("chain");
    assert_eq!(chain.len(), 32, "상한을 넘겨 반환하지 않는다");
    // 잘려도 남는 쪽은 **가까운 과거**여야 한다 — 지금 이어가는 맥락이 거기 있다.
    let expected: Vec<i64> = ids[ids.len() - 33..ids.len() - 1].to_vec();
    assert_eq!(chain, expected);

    let _ = std::fs::remove_file(&path);
}

/// 중복 이어받기 가드의 근거 — 세션을 **아직 쓰고 있는** 작업만 잡아낸다.
///
/// 진행 중인 작업이 걸리지 않으면 같은 벤더 세션을 두 작업이 동시에 resume하고, 벤더 쪽
/// 세션 파일을 둘이 번갈아 덮어쓴다.
#[tokio::test]
async fn live_task_with_session_finds_the_task_still_holding_the_session() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let running = resumable_task(&pool, "진행 중", state::RUNNING, Some("sess-live")).await;

    assert_eq!(
        db::live_task_with_session(&pool, "sess-live")
            .await
            .expect("query"),
        Some(running)
    );

    let _ = std::fs::remove_file(&path);
}

/// 종결된 작업은 세션을 놓은 것으로 본다 — 여기서 None이 나와야 이어받기가 열린다.
///
/// 세 종결 상태를 모두 확인한다. 하나라도 SQL의 제외 목록에서 빠지면 끝난 카드를 영영
/// 이어받을 수 없게 되고, 사용자는 "이미 이어가고 있다"는 말만 반복해서 본다.
#[tokio::test]
async fn live_task_with_session_releases_the_session_once_the_task_is_terminal() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let id = resumable_task(&pool, "끝날 작업", state::RUNNING, Some("sess-end")).await;

    for terminal in [state::DONE, state::FAILED, state::DISCARDED] {
        db::update_state(&pool, id, terminal, 2000)
            .await
            .expect("state");
        assert_eq!(
            db::live_task_with_session(&pool, "sess-end")
                .await
                .expect("query"),
            None,
            "{terminal} 은 종결이므로 세션을 놓아야 한다"
        );
    }

    // 종결이 아닌 상태들은 반대로 전부 잡혀야 한다 — 승인 대기·마무리 중인 작업도
    // 벤더 세션을 여전히 들고 있다.
    for live in [
        state::CREATED,
        state::QUEUED,
        state::STARTING,
        state::AWAITING_REVIEW,
        state::FINALIZING,
        state::PENDING_APPROVAL,
    ] {
        db::update_state(&pool, id, live, 2001).await.expect("state");
        assert_eq!(
            db::live_task_with_session(&pool, "sess-end")
                .await
                .expect("query"),
            Some(id),
            "{live} 은 아직 세션을 쓰는 중이다"
        );
    }

    let _ = std::fs::remove_file(&path);
}

/// 아무도 들고 있지 않은 세션은 None이다 — 첫 이어받기가 가드에 걸리면 안 된다.
#[tokio::test]
async fn live_task_with_session_is_none_when_nobody_holds_the_session() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    // 세션이 없는 작업과, 다른 세션을 든 진행 중 작업이 있어도 섞이지 않아야 한다.
    resumable_task(&pool, "세션 없음", state::RUNNING, None).await;
    resumable_task(&pool, "다른 세션", state::RUNNING, Some("sess-other")).await;

    assert_eq!(
        db::live_task_with_session(&pool, "sess-missing")
            .await
            .expect("query"),
        None
    );

    let _ = std::fs::remove_file(&path);
}

/// 같은 세션을 여럿이 들고 있으면 **id 오름차순 첫 번째**다. 종결된 행은 후보가 아니므로
/// 더 작은 id를 갖고 있어도 건너뛴다 — 그러지 않으면 가드 메시지가 이미 끝난 카드를
/// 가리키고, 사용자는 메시지를 보낼 수 없는 곳으로 안내받는다.
#[tokio::test]
async fn live_task_with_session_returns_the_lowest_live_id_skipping_terminal_ones() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.expect("init");
    let done = resumable_task(&pool, "끝난 원본", state::DONE, Some("sess-dup")).await;
    let first_live = resumable_task(&pool, "이어받은 쪽", state::RUNNING, Some("sess-dup")).await;
    let second_live = resumable_task(&pool, "또 하나", state::QUEUED, Some("sess-dup")).await;
    assert!(done < first_live && first_live < second_live, "id 순서 전제");

    assert_eq!(
        db::live_task_with_session(&pool, "sess-dup")
            .await
            .expect("query"),
        Some(first_live)
    );

    let _ = std::fs::remove_file(&path);
}

/// 외부 세션 승계 — 조건부 UPDATE 하나가 칼럼 둘을 쓰고, 중복 승계를 거절하며, 원본 작업이
/// 끝나면 다시 열린다. 설계 `docs/designs/2026-09-17-session-home-resume-design.md` 결정 4·5·9.
#[tokio::test]
async fn external_session_adoption_is_exclusive_while_a_live_task_holds_the_session() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.unwrap();
    let session = "f4dc1f51-0000-4000-8000-000000000001";

    let first = db::insert_task(&pool, "/r", "b1", "main", "/w1", "i", Some("claude"), None, "conversation", 1).await.unwrap();
    db::adopt_external_session(&pool, first, session, 2).await.unwrap();
    let adopted = db::get_task(&pool, first).await.unwrap().unwrap();
    assert_eq!(adopted.convo_session_id.as_deref(), Some(session));
    assert_eq!(adopted.resumed_session.as_deref(), Some(session));

    // 두 번째 승계는 진행 중인 작업을 지목하며 거절된다 — 409에 실을 id다.
    let second = db::insert_task(&pool, "/r", "b2", "main", "/w2", "i", Some("claude"), None, "conversation", 3).await.unwrap();
    match db::adopt_external_session(&pool, second, session, 4).await {
        Err(db::AdoptError::Conflict(id)) => assert_eq!(id, first),
        other => panic!("중복 승계가 거절되지 않았다: {other:?}"),
    }
    assert_eq!(db::get_task(&pool, second).await.unwrap().unwrap().convo_session_id, None);

    // 턴 에필로그가 `convo_session_id`를 덮어써도 `resumed_session`이 가드를 살려 둔다.
    // 이것이 칼럼을 따로 둔 이유다(결정 5) — 이 줄이 깨지면 가드가 조용히 무력해진 것이다.
    db::set_convo_session(&pool, first, "some-other-thread").await.unwrap();
    match db::adopt_external_session(&pool, second, session, 5).await {
        Err(db::AdoptError::Conflict(id)) => assert_eq!(id, first),
        other => panic!("에필로그가 세션을 덮어쓴 뒤 가드가 빠졌다: {other:?}"),
    }

    // 원본이 끝나면 같은 세션을 다시 이어받을 수 있다.
    db::update_state(&pool, first, state::DONE, 6).await.unwrap();
    db::adopt_external_session(&pool, second, session, 7).await.unwrap();
    let heir = db::get_task(&pool, second).await.unwrap().unwrap();
    assert_eq!(heir.convo_session_id.as_deref(), Some(session));
    assert_eq!(heir.resumed_session.as_deref(), Some(session));

    // 가드는 승계 경로 밖에서도 같은 답을 내야 한다(`task_resume`의 중복 검사가 이것을 쓴다).
    assert_eq!(db::live_task_with_session(&pool, session).await.unwrap(), Some(second));

    pool.close().await;
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn external_session_vendor_identity_and_concurrent_claims() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.unwrap();
    let claude = db::insert_task(&pool, "/r", "c", "main", "/wc", "i", Some("claude"), None, "conversation", 1).await.unwrap();
    let a = db::insert_task(&pool, "/r", "a", "main", "/wa", "i", Some("codex"), None, "conversation", 1).await.unwrap();
    let b = db::insert_task(&pool, "/r", "b", "main", "/wb", "i", Some("codex"), None, "conversation", 1).await.unwrap();
    db::adopt_external_session_vendor(&pool, claude, "claude", "same-id", 2).await.unwrap();
    let (one, two) = tokio::join!(
        db::adopt_external_session_vendor(&pool, a, "codex", "same-id", 2),
        db::adopt_external_session_vendor(&pool, b, "codex", "same-id", 2),
    );
    assert_ne!(one.is_ok(), two.is_ok());
    let winner = if one.is_ok() { a } else { b };
    let loser = if one.is_ok() { b } else { a };
    assert!(matches!(if one.is_err() {one} else {two}, Err(db::AdoptError::Conflict(id)) if id == winner));
    assert_eq!(db::live_task_with_session_vendor(&pool, "claude", "same-id").await.unwrap(), Some(claude));
    // Changing the running task's provider and session does not rewrite its immutable external source.
    sqlx::query("UPDATE tasks SET agent='claude', convo_session_id=NULL WHERE id=?").bind(winner).execute(&pool).await.unwrap();
    assert_eq!(db::live_task_with_session_vendor(&pool, "codex", "same-id").await.unwrap(), Some(winner));
    assert!(db::adopt_external_session_vendor(&pool, loser, "claude", "other", 3).await.is_err());
    pool.close().await;
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn external_session_vendor_migration_preserves_general_codex_sessions() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.unwrap();
    let legacy = db::insert_task(&pool, "/r", "l", "main", "/wl", "i", Some("claude"), None, "conversation", 1).await.unwrap();
    let codex = db::insert_task(&pool, "/r", "c", "main", "/wc", "i", Some("codex"), None, "conversation", 1).await.unwrap();
    sqlx::query("UPDATE tasks SET resumed_session='legacy' WHERE id=?").bind(legacy).execute(&pool).await.unwrap();
    db::set_convo_session(&pool, codex, "codex-original").await.unwrap();
    pool.close().await;
    let pool = db::init_pool(&path).await.unwrap();
    let (vendor,): (Option<String>,) = sqlx::query_as("SELECT resumed_vendor FROM tasks WHERE id=?").bind(legacy).fetch_one(&pool).await.unwrap();
    assert_eq!(vendor.as_deref(), Some("claude"));
    let (vendor,): (Option<String>,) = sqlx::query_as("SELECT resumed_vendor FROM tasks WHERE id=?").bind(codex).fetch_one(&pool).await.unwrap();
    assert_eq!(vendor, None);
    assert_eq!(db::live_task_with_session_vendor(&pool, "codex", "codex-original").await.unwrap(), Some(codex));
    pool.close().await;
    let _ = std::fs::remove_file(path);
}
