//! 데스크톱이 띄우는 모바일 표면의 경계 (설계 2026-09-13 D1·D4)
//!
//! 핵심 계약 둘.
//!
//! 1. 공유 라우트(`/v1/health`·작업 읽기·쓰기)는 데스크톱에서도 **같은 경로로** 응답한다.
//!    경로가 하나라도 어긋나면 폰의 PWA가 그 화면에서만 조용히 404를 받는다.
//! 2. Runner 전용 라우트(취소·삭제·PTY 입력·파일 쓰기·GitHub)는 **마운트되지 않는다.**
//!    데스크톱에는 그 행위를 수행할 큐가 없고, 폰에 줄 권한도 아니다.

#[path = "support/temp_root.rs"]
mod temp_root;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use praxis_lib::db;
use praxis_lib::runner::actions::{ApiError, TaskActions};
use praxis_lib::runner::auth::RunnerAuth;
use praxis_lib::runner::config::{ExecutionPolicy, RunnerConfig};
use praxis_lib::runner::events::EventHub;
use praxis_lib::runner::http::{self, RunnerHttpState};
use praxis_lib::runner::queue::QueueWorker;
use praxis_lib::runner::QueuedTaskRequest;

static COUNTER: AtomicU32 = AtomicU32::new(0);
const TEST_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// 행위를 수행하지 않는 스텁 — 이 테스트가 보는 것은 라우팅이지 행위가 아니다.
struct StubActions;

fn unsupported() -> ApiError {
    (axum::http::StatusCode::NOT_IMPLEMENTED, "stub".to_string())
}

#[async_trait]
impl TaskActions for StubActions {
    async fn create(
        &self,
        _cx: &RunnerHttpState,
        _request: QueuedTaskRequest,
    ) -> Result<db::Task, ApiError> {
        Err(unsupported())
    }
    async fn run_approve(&self, _cx: &RunnerHttpState, _id: i64) -> Result<db::Task, ApiError> {
        Err(unsupported())
    }
    async fn approve(&self, _cx: &RunnerHttpState, _id: i64) -> Result<(), ApiError> {
        Err(unsupported())
    }
    async fn discard(&self, _cx: &RunnerHttpState, _id: i64) -> Result<(), ApiError> {
        Err(unsupported())
    }
    async fn message(
        &self,
        _cx: &RunnerHttpState,
        _id: i64,
        _message: &str,
    ) -> Result<(), ApiError> {
        Err(unsupported())
    }
    async fn verify(
        &self,
        _cx: &RunnerHttpState,
        _id: i64,
        _preview_token: String,
    ) -> Result<praxis_lib::verify::VerifyReport, ApiError> {
        Err(unsupported())
    }
    async fn output(
        &self,
        _cx: &RunnerHttpState,
        _id: i64,
        _after: i64,
        _limit: i64,
    ) -> Result<Vec<db::TaskOutput>, ApiError> {
        Err(unsupported())
    }
}

#[tokio::test]
async fn 데스크톱_표면이_공유_라우트를_같은_경로로_서빙한다() {
    let (pool, db_path) = test_pool("desktop-surface-shared").await;
    let (address, server, token_path) = serve(pool).await;
    let client = reqwest::Client::new();

    let health = client
        .get(format!("http://{address}/v1/health"))
        .bearer_auth(TEST_TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);
    let body: serde_json::Value = health.json().await.unwrap();
    assert_eq!(body["execution_policy"], "require_approval");

    // PWA가 실제로 부르는 경로들 — 404가 아니어야 한다(내용이 아니라 마운트 여부를 본다).
    for path in [
        "/v1/tasks",
        "/v1/repositories",
        "/v1/schedules",
        "/v1/events",
        "/v1/push/key",
    ] {
        let response = client
            .get(format!("http://{address}{path}"))
            .bearer_auth(TEST_TOKEN)
            .send()
            .await
            .unwrap();
        assert_ne!(response.status(), 404, "{path}가 마운트되지 않았다");
    }

    // 푸시 구독은 프런트(`src/mobile/push.ts`)가 부르는 이름 그대로여야 한다.
    let subscribe = client
        .post(format!("http://{address}/v1/push/subscribe"))
        .bearer_auth(TEST_TOKEN)
        .json(&serde_json::json!({ "endpoint": "https://example.invalid/x" }))
        .send()
        .await
        .unwrap();
    assert_ne!(subscribe.status(), 404, "/v1/push/subscribe가 어긋났다");

    server.abort();
    cleanup(&db_path, &token_path);
}

#[tokio::test]
async fn 러너_전용_라우트는_데스크톱_표면에_없다() {
    let (pool, db_path) = test_pool("desktop-surface-runner-only").await;
    let (address, server, token_path) = serve(pool).await;
    let client = reqwest::Client::new();

    for (method, path) in [
        (reqwest::Method::POST, "/v1/tasks/1/cancel"),
        (reqwest::Method::DELETE, "/v1/tasks/1"),
        (reqwest::Method::POST, "/v1/tasks/1/input"),
        (reqwest::Method::GET, "/v1/github/issues"),
    ] {
        let response = client
            .request(method.clone(), format!("http://{address}{path}"))
            .bearer_auth(TEST_TOKEN)
            .send()
            .await
            .unwrap();
        assert!(
            response.status() == 404 || response.status() == 405,
            "{method} {path}는 데스크톱 표면에 없어야 한다 (실제 {})",
            response.status()
        );
    }

    server.abort();
    cleanup(&db_path, &token_path);
}

#[tokio::test]
async fn 페어링_교환은_인증_바깥에_남는다() {
    let (pool, db_path) = test_pool("desktop-surface-pair").await;
    // 아직 자격이 없는 기기가 부르는 유일한 경로다 — 토큰 없이 통과해야 한다.
    let (code, _) =
        praxis_lib::runner::session::create_pairing(&pool, praxis_lib::runner::now_secs())
            .await
            .unwrap();
    let (address, server, token_path) = serve(pool).await;

    let response = reqwest::Client::new()
        .post(format!("http://{address}/m/pair"))
        .json(&serde_json::json!({ "code": code, "label": "phone" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        204,
        "페어링이 인증 뒤로 들어가면 첫 기기를 영영 붙일 수 없다"
    );
    assert!(
        response.headers().contains_key("set-cookie"),
        "세션 쿠키가 없으면 폰이 다음 요청부터 다시 막힌다"
    );

    server.abort();
    cleanup(&db_path, &token_path);
}

async fn test_pool(label: &str) -> (sqlx::SqlitePool, String) {
    let suffix = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = temp_root::dir().join(format!(
        "praxis-{label}-{}-{suffix}.sqlite",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let pool = db::init_pool(path.to_str().unwrap()).await.unwrap();
    praxis_lib::mobile_surface::migrate(&pool).await.unwrap();
    (pool, path.to_string_lossy().into_owned())
}

async fn serve(pool: sqlx::SqlitePool) -> (SocketAddr, tokio::task::JoinHandle<()>, String) {
    let token_path = write_token_file();
    let config = RunnerConfig {
        bind: "127.0.0.1:47832".parse().unwrap(),
        repository_roots: Vec::new(),
        max_concurrent_tasks: 1,
        execution_policy: ExecutionPolicy::RequireApproval,
        pairing_token_file: token_path.clone().into(),
        ..Default::default()
    };
    let state = RunnerHttpState {
        auth: RunnerAuth::from_file(token_path.as_ref()).unwrap(),
        events: EventHub::start(pool.clone()),
        queue: QueueWorker::new(pool.clone(), 1),
        pool,
        config,
        recovered_tasks: 0,
        started_at: 0,
        review_claims: Default::default(),
    };
    let router = http::mobile_surface_router(state, std::sync::Arc::new(StubActions));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (address, server, token_path)
}

fn write_token_file() -> String {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    let suffix = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = temp_root::dir().join(format!(
        "praxis-desktop-surface-token-{}-{suffix}",
        std::process::id()
    ));
    std::fs::write(&path, TEST_TOKEN).unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    path.to_string_lossy().into_owned()
}

fn cleanup(db_path: &str, token_path: &str) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{db_path}{suffix}"));
    }
    let _ = std::fs::remove_file(token_path);
}
