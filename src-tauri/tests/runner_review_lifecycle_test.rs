#[path = "support/temp_root.rs"]
mod temp_root;

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode};
use praxis_lib::db;
use praxis_lib::review_ops::ReviewClaims;
use praxis_lib::runner::auth::RunnerAuth;
use praxis_lib::runner::config::{ExecutionPolicy, RunnerConfig};
use praxis_lib::runner::events::EventHub;
use praxis_lib::runner::actions::RunnerTaskActions;
use praxis_lib::runner::http::{self, RunnerHttpState};
use praxis_lib::runner::queue::QueueWorker;
use tower::ServiceExt;

#[allow(dead_code)]
#[path = "support/runner_review_paths.rs"]
mod paths;

const TOKEN: &str = "abababababababababababababababababababababababababababababababab";

#[tokio::test]
async fn active_review_claim_blocks_runner_terminal_transitions() {
    let root = paths::temporary_dir("lifecycle");
    let pool = db::init_pool(&root.join("test.sqlite").to_string_lossy())
        .await
        .unwrap();
    let token_path = paths::token_file(TOKEN);
    let claims = ReviewClaims::default();
    let _verify = claims.claim_verify(7).unwrap();
    let state = RunnerHttpState {
        auth: RunnerAuth::from_file(&token_path).unwrap(),
        events: EventHub::start(pool.clone()),
        queue: QueueWorker::new(pool.clone(), 1),
        pool,
        config: RunnerConfig {
            bind: "127.0.0.1:47831".parse().unwrap(),
            repository_roots: vec![],
            max_concurrent_tasks: 1,
            execution_policy: ExecutionPolicy::AlwaysApprove,
            pairing_token_file: token_path,
        },
        recovered_tasks: 0,
        started_at: 0,
        review_claims: claims,
    };
    let app = http::mobile_surface_router(state, std::sync::Arc::new(RunnerTaskActions));

    for (method, uri) in [
        (Method::POST, "/v1/tasks/7/approve"),
        (Method::POST, "/v1/tasks/7/discard"),
    ] {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header("Authorization", format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:43123".parse::<SocketAddr>().unwrap(),
        ));
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn durable_quarantine_blocks_runner_review_and_mutation_after_claim_reset() {
    let root = paths::temporary_dir("durable-lifecycle");
    let pool = db::init_pool(&root.join("test.sqlite").to_string_lossy())
        .await
        .unwrap();
    praxis_lib::runner::review_process::migrate(&pool)
        .await
        .unwrap();
    let root_text = root.to_string_lossy();
    let task_id = db::insert_task(
        &pool,
        &root_text,
        "branch",
        "main",
        &root_text,
        "durable lifecycle",
        None,
        None,
        "terminal",
        1,
    )
    .await
    .unwrap();
    db::update_state(&pool, task_id, db::state::AWAITING_REVIEW, 2)
        .await
        .unwrap();
    let receipt = praxis_lib::runner::review_process::register(
        &pool,
        task_id,
        praxis_lib::runner::review_process::ReviewOperation::Verify,
        praxis_lib::runner::review_process::ReviewPhase::VerifyBuild,
        999_999,
        &"a".repeat(64),
        3,
    )
    .await
    .unwrap();
    praxis_lib::runner::review_process::quarantine(
        &pool,
        task_id,
        praxis_lib::runner::review_process::ReviewOperation::Verify,
        receipt.id,
        "unresolved child",
        4,
    )
    .await
    .unwrap();
    let app = http::mobile_surface_router(state(pool, &root), std::sync::Arc::new(RunnerTaskActions));

    for (method, suffix, body) in [
        (Method::POST, "approve", ""),
        (Method::POST, "discard", ""),
        (Method::POST, "run", ""),
        (Method::POST, "verify", r#"{"preview_token":"x"}"#),
    ] {
        let uri = format!("/v1/tasks/{task_id}/{suffix}");
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header("Authorization", format!("Bearer {TOKEN}"))
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:43123".parse::<SocketAddr>().unwrap(),
        ));
        let response = app.clone().oneshot(request).await.unwrap();
        assert!(
            matches!(
                response.status(),
                StatusCode::BAD_REQUEST | StatusCode::CONFLICT
            ),
            "unexpected status for {suffix}: {}",
            response.status()
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            String::from_utf8_lossy(&body).contains("durable review process lease"),
            "missing durable fence detail for {suffix}"
        );
    }
}

fn state(pool: sqlx::SqlitePool, root: &std::path::Path) -> RunnerHttpState {
    let token_path = paths::token_file(TOKEN);
    RunnerHttpState {
        auth: RunnerAuth::from_file(&token_path).unwrap(),
        events: EventHub::start(pool.clone()),
        queue: QueueWorker::new(pool.clone(), 1),
        pool,
        config: RunnerConfig {
            bind: "127.0.0.1:47831".parse().unwrap(),
            repository_roots: vec![root.canonicalize().unwrap()],
            max_concurrent_tasks: 1,
            execution_policy: ExecutionPolicy::AlwaysApprove,
            pairing_token_file: token_path,
        },
        recovered_tasks: 0,
        started_at: 0,
        review_claims: ReviewClaims::default(),
    }
}
