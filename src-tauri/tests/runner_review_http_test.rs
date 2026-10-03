#[path = "support/temp_root.rs"]
mod temp_root;

use axum::http::{Method, StatusCode};
use praxis_lib::db;
use praxis_lib::runner::auth::RunnerAuth;
use praxis_lib::runner::config::{ExecutionPolicy, RunnerConfig};
use praxis_lib::runner::events::EventHub;
use praxis_lib::runner::actions::RunnerTaskActions;
use praxis_lib::runner::http::{self, RunnerHttpState};
use praxis_lib::runner::queue::QueueWorker;
use tower::ServiceExt;

#[path = "support/runner_review_paths.rs"]
mod paths;

const TOKEN: &str = "abababababababababababababababababababababababababababababababab";

#[tokio::test]
async fn runner_review_routes_persist_host_owned_evidence() {
    let fixture = fixture().await;
    let app = http::mobile_surface_router(fixture.state.clone(), std::sync::Arc::new(RunnerTaskActions));
    let preview_response = app
        .clone()
        .oneshot(paths::request(
            TOKEN,
            Method::GET,
            "/v1/tasks/1/verify/spec",
            None,
            true,
        ))
        .await
        .unwrap();
    assert_eq!(preview_response.status(), StatusCode::OK);
    let preview = paths::json_body(preview_response).await;
    let token = preview["preview_token"].as_str().unwrap();

    let verify_response = app
        .clone()
        .oneshot(paths::request(
            TOKEN,
            Method::POST,
            "/v1/tasks/1/verify",
            Some(serde_json::json!({ "preview_token": token })),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(verify_response.status(), StatusCode::OK);
    paths::assert_process_receipt_resolved(&fixture.state.pool, 1, "verify", "verify_test").await;

    let evidence = app
        .oneshot(paths::request(
            TOKEN,
            Method::GET,
            "/v1/tasks/1/evidence",
            None,
            true,
        ))
        .await
        .unwrap();
    assert_eq!(evidence.status(), StatusCode::OK);
    assert_eq!(paths::json_body(evidence).await["ready"], true);
}

#[tokio::test]
async fn review_routes_enforce_auth_and_task_root_ownership() {
    let fixture = fixture().await;
    let outside = paths::temporary_dir("outside");
    let outside_text = outside.to_string_lossy();
    let outside_task = db::insert_task(
        &fixture.state.pool,
        &outside_text,
        "praxis/outside",
        "main",
        &outside_text,
        "outside",
        None,
        None,
        "terminal",
        1,
    )
    .await
    .unwrap();
    let app = http::mobile_surface_router(fixture.state.clone(), std::sync::Arc::new(RunnerTaskActions));
    let unauthorized = app
        .clone()
        .oneshot(paths::request(
            TOKEN,
            Method::GET,
            "/v1/tasks/1/verify/spec",
            None,
            false,
        ))
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    // 허용 루트 밖의 작업은 토큰이 유효해도 거부된다.
    let forbidden = app
        .clone()
        .oneshot(paths::request(
            TOKEN,
            Method::GET,
            &format!("/v1/tasks/{outside_task}/verify/spec"),
            None,
            true,
        ))
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let allowed = app
        .oneshot(paths::request(
            TOKEN,
            Method::GET,
            "/v1/tasks/1/evidence",
            None,
            true,
        ))
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::OK);
}

struct Fixture {
    state: RunnerHttpState,
}

#[tokio::test]
async fn approval_inspection_enforces_auth_and_both_path_roots_without_writes() {
    let fixture = fixture().await;
    let app = http::mobile_surface_router(fixture.state.clone(), std::sync::Arc::new(RunnerTaskActions));
    let unauthorized = app.clone().oneshot(paths::request(TOKEN, Method::GET, "/v1/tasks/1/approval-status", None, false)).await.unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM task_events").fetch_one(&fixture.state.pool).await.unwrap();
    let allowed = app.clone().oneshot(paths::request(TOKEN, Method::GET, "/v1/tasks/1/approval-status", None, true)).await.unwrap();
    assert_eq!(allowed.status(), StatusCode::OK);
    let body = paths::json_body(allowed).await;
    // This fixture isn't a Git checkout: unknown, never a false successful precheck.
    assert!(body["readiness"].is_null()); assert!(body["inspection_error"].is_string());
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM task_events").fetch_one(&fixture.state.pool).await.unwrap();
    assert_eq!(before, after);
    let outside = paths::temporary_dir("approval-outside");
    for column in ["repo", "worktree_path"] {
        let original: String = sqlx::query_scalar(&format!("SELECT {column} FROM tasks WHERE id=1")).fetch_one(&fixture.state.pool).await.unwrap();
        sqlx::query(&format!("UPDATE tasks SET {column}=? WHERE id=1")).bind(outside.to_str().unwrap()).execute(&fixture.state.pool).await.unwrap();
        let forbidden = app.clone().oneshot(paths::request(TOKEN, Method::GET, "/v1/tasks/1/approval-status", None, true)).await.unwrap();
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
        sqlx::query(&format!("UPDATE tasks SET {column}=? WHERE id=1")).bind(original).execute(&fixture.state.pool).await.unwrap();
    }
}

async fn fixture() -> Fixture {
    let root = paths::temporary_dir("allowed");
    std::fs::create_dir_all(root.join(".praxis")).unwrap();
    std::fs::write(
        root.join(".praxis").join("validate.toml"),
        "test = \"printf '2 passed'\"\n",
    )
    .unwrap();
    let pool = db::init_pool(&root.join("test.sqlite").to_string_lossy())
        .await
        .unwrap();
    let root_text = root.to_string_lossy();
    db::insert_task(
        &pool,
        &root_text,
        "praxis/review",
        "main",
        &root_text,
        "verify",
        None,
        None,
        "terminal",
        1,
    )
    .await
    .unwrap();
    let token_path = paths::token_file(TOKEN);
    let config = RunnerConfig {
        bind: "127.0.0.1:47831".parse().unwrap(),
        repository_roots: vec![root.canonicalize().unwrap()],
        max_concurrent_tasks: 2,
        execution_policy: ExecutionPolicy::AlwaysApprove,
        pairing_token_file: token_path.clone(),
    };
    Fixture {
        state: RunnerHttpState {
            auth: RunnerAuth::from_file(&token_path).unwrap(),
            events: EventHub::start(pool.clone()),
            queue: QueueWorker::new(pool.clone(), 2),
            pool,
            config,
            recovered_tasks: 0,
            review_claims: Default::default(),
            started_at: 0,
        },
    }
}

#[tokio::test]
async fn repair_routes_require_auth_and_both_authorized_paths() {
    let fixture = fixture().await;
    let app = http::mobile_surface_router(fixture.state.clone(), std::sync::Arc::new(RunnerTaskActions));
    let outside = paths::temporary_dir("repair-outside");
    let body = serde_json::json!({"session_id":"not-started"});
    for (method, suffix) in [(Method::GET,""),(Method::POST,""),(Method::POST,"/run"),(Method::POST,"/accept"),(Method::POST,"/cancel")] {
        let url = format!("/v1/tasks/1/approval-repair{suffix}");
        let response = app.clone().oneshot(paths::request(TOKEN, method.clone(), &url, Some(body.clone()), false)).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        for column in ["repo", "worktree_path"] {
            let original: String = sqlx::query_scalar(&format!("SELECT {column} FROM tasks WHERE id=1")).fetch_one(&fixture.state.pool).await.unwrap();
            sqlx::query(&format!("UPDATE tasks SET {column}=? WHERE id=1")).bind(outside.to_str().unwrap()).execute(&fixture.state.pool).await.unwrap();
            let response = app.clone().oneshot(paths::request(TOKEN, method.clone(), &url, Some(body.clone()), true)).await.unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{url} {column}");
            sqlx::query(&format!("UPDATE tasks SET {column}=? WHERE id=1")).bind(original).execute(&fixture.state.pool).await.unwrap();
        }
    }
}
