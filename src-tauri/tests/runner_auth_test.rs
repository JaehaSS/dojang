#[path = "support/temp_root.rs"]
mod temp_root;

use std::sync::atomic::{AtomicU32, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use praxis_lib::db;
use praxis_lib::runner::auth::{authorize_repository_path, RunnerAuth};
use praxis_lib::runner::config::RunnerConfig;
use praxis_lib::runner::events::EventHub;
use praxis_lib::runner::actions::RunnerTaskActions;
use praxis_lib::runner::http::{self, RunnerHttpState};
use praxis_lib::runner::queue::QueueWorker;
use tower::ServiceExt;

static COUNTER: AtomicU32 = AtomicU32::new(0);
const TOKEN: &str = "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";

#[tokio::test]
async fn missing_or_wrong_token_and_untrusted_peer_are_rejected() {
    let (auth, token_path) = auth_fixture();
    let db_path = temp_root::dir()
        .join(format!(
            "praxis-runner-auth-db-{}-{}.sqlite",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ))
        .to_string_lossy()
        .into_owned();
    let pool = db::init_pool(&db_path).await.unwrap();
    let state = RunnerHttpState {
        auth,
        pool: pool.clone(),
        config: RunnerConfig {
            bind: "127.0.0.1:47831".parse().unwrap(),
            repository_roots: vec![],
            max_concurrent_tasks: 2,
            execution_policy: praxis_lib::runner::config::ExecutionPolicy::AlwaysApprove,
            pairing_token_file: token_path.clone().into(),
        },
        recovered_tasks: 0,
        events: EventHub::start(pool.clone()),
        queue: QueueWorker::new(pool.clone(), 2),
        started_at: 0,
        review_claims: Default::default(),
    };
    let app = http::mobile_surface_router(state, std::sync::Arc::new(RunnerTaskActions));
    let no_peer = app
        .oneshot(
            Request::builder()
                .uri("/v1/health")
                .header("Authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_peer.status(), StatusCode::FORBIDDEN);
    let _ = std::fs::remove_file(token_path);
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn token_file_mode_and_bearer_comparison_are_strict() {
    let (auth, token_path) = auth_fixture();
    let good = format!("Bearer {TOKEN}").parse().unwrap();
    let wrong = format!("Bearer {}", "ef".repeat(32)).parse().unwrap();
    assert!(auth.matches_bearer(Some(&good)));
    assert!(!auth.matches_bearer(Some(&wrong)));
    let mut protocols = axum::http::HeaderMap::new();
    protocols.insert(
        "Sec-WebSocket-Protocol",
        format!("praxis, {TOKEN}").parse().unwrap(),
    );
    assert!(auth.matches_request(&protocols));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(RunnerAuth::from_file(token_path.as_ref()).is_err());
    }
    let _ = std::fs::remove_file(token_path);
}

#[cfg(unix)]
#[test]
fn repository_authorization_rejects_escape_and_symlink_paths() {
    use std::os::unix::fs::symlink;

    let root = temporary_dir("root");
    let outside = temporary_dir("outside");
    let nested = root.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    let canonical_root = root.canonicalize().unwrap();
    assert_eq!(
        authorize_repository_path(std::slice::from_ref(&canonical_root), &nested).unwrap(),
        nested.canonicalize().unwrap()
    );
    assert!(authorize_repository_path(std::slice::from_ref(&canonical_root), &outside).is_err());
    symlink(&outside, root.join("outside-link")).unwrap();
    assert!(authorize_repository_path(
        std::slice::from_ref(&canonical_root),
        &root.join("outside-link")
    )
    .is_err());
    symlink(root.join("missing"), root.join("dangling-link")).unwrap();
    assert!(authorize_repository_path(
        std::slice::from_ref(&canonical_root),
        &root.join("dangling-link")
    )
    .is_err());
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(outside);
}

fn auth_fixture() -> (RunnerAuth, String) {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    let path = temp_root::dir().join(format!(
        "praxis-runner-auth-token-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::write(&path, TOKEN).unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let path = path.to_string_lossy().into_owned();
    (RunnerAuth::from_file(path.as_ref()).unwrap(), path)
}

#[cfg(unix)]
fn temporary_dir(label: &str) -> std::path::PathBuf {
    let path = temp_root::dir().join(format!(
        "praxis-runner-auth-{label}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}
