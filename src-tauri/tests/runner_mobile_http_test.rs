//! 모바일 PWA 셸 서빙 경계 테스트 (설계 0013 §5.2)
//!
//! 핵심 계약: `/m/*`는 **인증 바깥**(토큰 없이 200), `/v1/*`는 그대로 인증 뒤(401).
//! 이 두 가지가 동시에 성립하지 않으면 폰에서 앱을 열 수 없거나 API가 무방비가 된다.

#[path = "support/temp_root.rs"]
mod temp_root;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::db;
use praxis_lib::runner::auth::RunnerAuth;
use praxis_lib::runner::config::RunnerConfig;
use praxis_lib::runner::events::EventHub;
use praxis_lib::runner::actions::RunnerTaskActions;
use praxis_lib::runner::http::{self, RunnerHttpState};
use praxis_lib::runner::queue::QueueWorker;

static COUNTER: AtomicU32 = AtomicU32::new(0);
const TEST_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// dist-mobile이 비어 있으면(Node 없는 호스트에서 체크아웃만 한 경우) 셸 본문 검증은
/// 의미가 없다. 그 경우 503 계약만 확인하고 나머지는 건너뛴다.
fn bundle_present() -> bool {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dist-mobile/mobile.html")
        .exists()
}

#[tokio::test]
async fn 셸은_인증_바깥이고_v1은_인증_뒤에_남는다() {
    let (pool, db_path) = test_pool("mobile-auth").await;
    let (address, server, token_path) = serve(pool).await;
    let anonymous = reqwest::Client::new();

    // 토큰 없는 top-level navigation으로 셸을 받을 수 있어야 한다.
    let shell = anonymous
        .get(format!("http://{address}/m/"))
        .send()
        .await
        .unwrap();
    if bundle_present() {
        assert_eq!(shell.status(), 200);
        assert_eq!(shell.headers()["cache-control"], "no-cache");
        assert_eq!(shell.headers()["x-content-type-options"], "nosniff");
        assert!(shell.text().await.unwrap().contains("<div id=\"root\">"));
    } else {
        assert_eq!(
            shell.status(),
            503,
            "번들 부재는 503으로 원인을 알려야 한다"
        );
    }

    // 같은 익명 클라이언트로 API는 여전히 막혀야 한다.
    for path in ["/v1/health", "/v1/tasks"] {
        let response = anonymous
            .get(format!("http://{address}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401, "{path}는 인증 뒤에 남아야 한다");
    }

    server.abort();
    cleanup(&db_path, &token_path);
}

#[tokio::test]
async fn 딥링크는_셸로_폴백하고_없는_자산은_404다() {
    if !bundle_present() {
        return;
    }
    let (pool, db_path) = test_pool("mobile-fallback").await;
    let (address, server, token_path) = serve(pool).await;
    let client = reqwest::Client::new();

    // /m/t/12 는 파일이 아니므로 셸을 반환해야 한다 — 새로고침·푸시 딥링크 경로.
    for path in ["/m/t/12", "/m/t/12/terminal", "/m/settings", "/m"] {
        let response = client
            .get(format!("http://{address}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{path}");
        assert!(response.text().await.unwrap().contains("<div id=\"root\">"));
    }

    // 없는 자산까지 셸로 폴백하면 JS 로드 실패가 200 HTML로 위장된다.
    let missing = client
        .get(format!("http://{address}/m/assets/does-not-exist.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);

    server.abort();
    cleanup(&db_path, &token_path);
}

#[tokio::test]
async fn 해시_자산만_장기_캐시하고_sw는_캐시하지_않는다() {
    if !bundle_present() {
        return;
    }
    let (pool, db_path) = test_pool("mobile-cache").await;
    let (address, server, token_path) = serve(pool).await;
    let client = reqwest::Client::new();

    // sw.js를 캐시하면 배포가 폰에 영영 닿지 않는다.
    let sw = client
        .get(format!("http://{address}/m/sw.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(sw.status(), 200);
    assert_eq!(sw.headers()["cache-control"], "no-cache");

    let manifest = client
        .get(format!("http://{address}/m/manifest.webmanifest"))
        .send()
        .await
        .unwrap();
    assert_eq!(manifest.status(), 200);
    assert_eq!(manifest.headers()["cache-control"], "no-cache");

    // 해시가 붙은 자산은 영구 캐시가 안전하다.
    let asset_name = std::fs::read_dir(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist-mobile/assets"),
    )
    .unwrap()
    .filter_map(Result::ok)
    .map(|entry| entry.file_name().to_string_lossy().into_owned())
    .find(|name| name.ends_with(".js"))
    .expect("해시 자산이 있어야 한다");
    let asset = client
        .get(format!("http://{address}/m/assets/{asset_name}"))
        .send()
        .await
        .unwrap();
    assert_eq!(asset.status(), 200);
    assert_eq!(
        asset.headers()["cache-control"],
        "public, max-age=31536000, immutable"
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
    (pool, path.to_string_lossy().into_owned())
}

async fn serve(pool: sqlx::SqlitePool) -> (SocketAddr, tokio::task::JoinHandle<()>, String) {
    let token_path = write_token_file();
    let config = RunnerConfig {
        bind: "127.0.0.1:47831".parse().unwrap(),
        repository_roots: Vec::new(),
        max_concurrent_tasks: 2,
        execution_policy: praxis_lib::runner::config::ExecutionPolicy::AlwaysApprove,
        pairing_token_file: token_path.clone().into(),
    };
    let queue = QueueWorker::new(pool.clone(), 2);
    let state = RunnerHttpState {
        auth: RunnerAuth::from_file(token_path.as_ref()).unwrap(),
        events: EventHub::start(pool.clone()),
        pool,
        config,
        recovered_tasks: 0,
        queue,
        started_at: 0,
        review_claims: Default::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            http::mobile_surface_router(state, std::sync::Arc::new(RunnerTaskActions))
                .into_make_service_with_connect_info::<SocketAddr>(),
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
        "praxis-mobile-token-{}-{suffix}",
        std::process::id()
    ));
    std::fs::write(&path, TEST_TOKEN).unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    path.to_string_lossy().into_owned()
}

fn cleanup(db_path: &str, token_path: &str) {
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(token_path);
}
