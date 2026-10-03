//! 모바일 세션 인증 경계 테스트 (설계 0013 §6)
//!
//! 지켜야 할 계약:
//!   - 페어링 코드는 **한 번만** 쓰이고 만료된다.
//!   - 세션 쿠키로 `/v1/*`를 쓸 수 있지만, 회수하면 즉시 끊긴다.
//!   - 쿠키 자격의 상태 변경 요청은 Origin이 자기 자신일 때만 통과한다(CSRF).
//!   - `scope=mobile`은 파일 편집과 기기 관리를 할 수 없다.

#[path = "support/temp_root.rs"]
mod temp_root;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::db;
use praxis_lib::runner::auth::{self, RunnerAuth};
use praxis_lib::runner::config::RunnerConfig;
use praxis_lib::runner::events::EventHub;
use praxis_lib::runner::actions::RunnerTaskActions;
use praxis_lib::runner::http::{self, RunnerHttpState};
use praxis_lib::runner::queue::QueueWorker;
use praxis_lib::runner::session;

static COUNTER: AtomicU32 = AtomicU32::new(0);
const TEST_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[tokio::test]
async fn 페어링_코드는_한_번만_쓰이고_쿠키로_교환된다() {
    let (pool, db_path) = test_pool("pair").await;
    let (address, server, token_path) = serve(pool.clone()).await;

    let code: serde_json::Value = desktop_client()
        .post(format!("http://{address}/v1/mobile/pairings"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let code = code["code"].as_str().unwrap().to_string();

    let paired = reqwest::Client::new()
        .post(format!("http://{address}/m/pair"))
        .json(&serde_json::json!({ "code": code, "label": "테스트 폰" }))
        .send()
        .await
        .unwrap();
    assert_eq!(paired.status(), 204);
    let cookie = paired.headers()["set-cookie"].to_str().unwrap().to_string();
    assert!(cookie.contains("HttpOnly"), "토큰이 JS에 노출되면 안 된다");
    assert!(cookie.contains("Secure"));
    assert!(cookie.contains("SameSite=Strict"));

    // 같은 코드를 다시 쓰면 거절되어야 한다 — 코드는 일회용이다.
    let replay = reqwest::Client::new()
        .post(format!("http://{address}/m/pair"))
        .json(&serde_json::json!({ "code": code }))
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status(), 401);

    // 발급된 쿠키로 API를 쓸 수 있어야 한다.
    let session_cookie = session_cookie(&cookie);
    let health = reqwest::Client::new()
        .get(format!("http://{address}/v1/health"))
        .header(reqwest::header::COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);

    server.abort();
    cleanup(&db_path, &token_path);
}

#[tokio::test]
async fn 회수하면_해당_기기만_즉시_끊긴다() {
    let (pool, db_path) = test_pool("revoke").await;
    let (address, server, token_path) = serve(pool.clone()).await;
    let session_cookie = pair_device(&address).await;

    let sessions: Vec<serde_json::Value> = desktop_client()
        .get(format!("http://{address}/v1/mobile/sessions"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(sessions.len(), 1);
    let id = sessions[0]["id"].as_i64().unwrap();
    // 목록에 토큰 해시가 새어나가면 안 된다.
    assert!(sessions[0].get("token_hash").is_none());

    let revoked = desktop_client()
        .delete(format!("http://{address}/v1/mobile/sessions/{id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), 204);

    let after = reqwest::Client::new()
        .get(format!("http://{address}/v1/health"))
        .header(reqwest::header::COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(after.status(), 401, "회수된 세션은 즉시 끊겨야 한다");

    // Desktop(pairing token)은 영향을 받지 않아야 한다.
    let desktop = desktop_client()
        .get(format!("http://{address}/v1/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(desktop.status(), 200);

    server.abort();
    cleanup(&db_path, &token_path);
}

#[tokio::test]
async fn 쿠키_자격의_상태변경은_자기_오리진에서만_통과한다() {
    let (pool, db_path) = test_pool("csrf").await;
    let (address, server, token_path) = serve(pool.clone()).await;
    let session_cookie = pair_device(&address).await;
    let client = reqwest::Client::new();
    // 인증만 격리해서 보려고 존재하지 않는 task를 고른다 — 통과하면 404, 막히면 403.
    let url = format!("http://{address}/v1/tasks/999999/message");

    let no_origin = client
        .post(&url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(no_origin.status(), 403, "Origin 없는 쿠키 POST는 거절");

    let foreign = client
        .post(&url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .header(reqwest::header::ORIGIN, "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(foreign.status(), 403, "타 오리진 쿠키 POST는 거절");

    let same = client
        .post(&url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .header(reqwest::header::ORIGIN, format!("http://{address}"))
        .send()
        .await
        .unwrap();
    assert_ne!(same.status(), 403, "자기 오리진은 인증을 통과해야 한다");

    // Bearer 자격은 쿠키가 아니므로 CSRF 대상이 아니다 — Origin 없이도 통과.
    let bearer = desktop_client().post(&url).send().await.unwrap();
    assert_ne!(bearer.status(), 403);

    // Origin을 생략하는 브라우저를 위해 Sec-Fetch-Site도 받는다. 이게 없으면 폰에서
    // 승인이 조용히 403이 된다 — 이 앱이 가장 피해야 할 실패다.
    let sec_fetch = client
        .post(&url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .header("sec-fetch-site", "same-origin")
        .send()
        .await
        .unwrap();
    assert_ne!(sec_fetch.status(), 403);

    // 반대로 cross-site라고 알려온 요청은 Origin이 맞아 보여도 거절한다.
    let cross = client
        .post(&url)
        .header(reqwest::header::COOKIE, &session_cookie)
        .header("sec-fetch-site", "cross-site")
        .header(reqwest::header::ORIGIN, format!("http://{address}"))
        .send()
        .await
        .unwrap();
    assert_eq!(cross.status(), 403);

    server.abort();
    cleanup(&db_path, &token_path);
}

#[tokio::test]
async fn 모바일_scope는_파일_편집과_기기_관리를_못_한다() {
    let (pool, db_path) = test_pool("scope").await;
    let (address, server, token_path) = serve(pool.clone()).await;
    let session_cookie = pair_device(&address).await;
    let client = reqwest::Client::new();
    let origin = format!("http://{address}");

    let write = client
        .put(format!("http://{address}/v1/files/write"))
        .header(reqwest::header::COOKIE, &session_cookie)
        .header(reqwest::header::ORIGIN, &origin)
        .json(&serde_json::json!({ "repository": "/tmp", "path": "a.txt", "content": "x" }))
        .send()
        .await
        .unwrap();
    // 모바일 서피스에는 쓰기 라우트 자체가 없다(Runner 제거, 1.0). 정책(403)이든 부재(404)든 성공만 아니면 된다.
    assert!(!write.status().is_success(), "모바일은 파일을 수정할 수 없다: {}", write.status());

    // 폰이 스스로 기기를 늘릴 수 있으면 회수가 무의미해진다.
    let pairing = client
        .post(format!("http://{address}/v1/mobile/pairings"))
        .header(reqwest::header::COOKIE, &session_cookie)
        .header(reqwest::header::ORIGIN, &origin)
        .send()
        .await
        .unwrap();
    assert_eq!(pairing.status(), 403);

    let list = client
        .get(format!("http://{address}/v1/mobile/sessions"))
        .header(reqwest::header::COOKIE, &session_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), 403);

    server.abort();
    cleanup(&db_path, &token_path);
}

#[tokio::test]
async fn 만료된_페어링_코드는_교환되지_않는다() {
    let (pool, db_path) = test_pool("expire").await;
    session::migrate(&pool).await.unwrap();
    let now = 1_800_000_000;

    let (code, expires_at) = session::create_pairing(&pool, now).await.unwrap();
    assert_eq!(expires_at, now + session::PAIRING_TTL_SECS);

    // 만료 직후에는 교환 불가.
    let expired = session::redeem_pairing(&pool, &code, "폰", expires_at + 1)
        .await
        .unwrap();
    assert!(expired.is_none());

    // 만료 전이라면 통과하고, 세션 토큰은 코드와 다른 값이어야 한다.
    let (fresh, _) = session::create_pairing(&pool, now).await.unwrap();
    let token = session::redeem_pairing(&pool, &fresh, "폰", now + 1)
        .await
        .unwrap()
        .expect("만료 전 코드는 교환된다");
    assert_ne!(token, fresh);
    assert_eq!(token.len(), 64);

    let authenticated = session::authenticate(&pool, &token, now + 2).await.unwrap();
    assert_eq!(authenticated.unwrap().scope, session::SCOPE_MOBILE);

    // 만료된 세션은 인증되지 않는다.
    let stale = session::authenticate(&pool, &token, now + session::SESSION_TTL_SECS + 10)
        .await
        .unwrap();
    assert!(stale.is_none());

    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn origin과_쿠키_파싱은_경계를_정확히_본다() {
    assert_eq!(
        auth::origin_authority("https://worker.tailnet.ts.net"),
        Some("worker.tailnet.ts.net")
    );
    assert_eq!(
        auth::origin_authority("http://127.0.0.1:47831/some/path"),
        Some("127.0.0.1:47831")
    );
    assert_eq!(auth::origin_authority("null"), None);
    assert_eq!(auth::origin_authority("https://"), None);

    assert_eq!(
        auth::cookie_value("a=1; praxis_mobile=abc; b=2", "praxis_mobile"),
        Some("abc")
    );
    assert_eq!(
        auth::cookie_value("praxis_mobile=abc", "praxis_mobile"),
        Some("abc")
    );
    // 접두사만 같은 이름에 걸리면 안 된다.
    assert_eq!(
        auth::cookie_value("praxis_mobile_x=abc", "praxis_mobile"),
        None
    );
    assert_eq!(auth::cookie_value("", "praxis_mobile"), None);
}

#[test]
fn 모바일_deny_list는_편집과_기기관리만_막는다() {
    use axum::http::Method;

    assert!(auth::mobile_scope_denies(&Method::PUT, "/v1/files/write"));
    assert!(auth::mobile_scope_denies(
        &Method::POST,
        "/v1/mobile/pairings"
    ));
    assert!(auth::mobile_scope_denies(
        &Method::GET,
        "/v1/mobile/sessions"
    ));
    // 리뷰·승인·조회는 모바일의 존재 이유다 — 막으면 안 된다.
    assert!(!auth::mobile_scope_denies(&Method::GET, "/v1/files/read"));
    assert!(!auth::mobile_scope_denies(
        &Method::POST,
        "/v1/tasks/1/approve"
    ));
    assert!(!auth::mobile_scope_denies(&Method::GET, "/v1/health"));
}

async fn pair_device(address: &SocketAddr) -> String {
    let code: serde_json::Value = desktop_client()
        .post(format!("http://{address}/v1/mobile/pairings"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let response = reqwest::Client::new()
        .post(format!("http://{address}/m/pair"))
        .json(&serde_json::json!({ "code": code["code"].as_str().unwrap() }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
    session_cookie(response.headers()["set-cookie"].to_str().unwrap())
}

/// `Set-Cookie` 헤더에서 `name=value`만 떼어 `Cookie` 헤더로 되돌린다.
fn session_cookie(set_cookie: &str) -> String {
    set_cookie.split(';').next().unwrap().to_string()
}

async fn test_pool(label: &str) -> (sqlx::SqlitePool, String) {
    let suffix = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = temp_root::dir().join(format!(
        "praxis-mobile-{label}-{}-{suffix}.sqlite",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let pool = db::init_pool(path.to_str().unwrap()).await.unwrap();
    (pool, path.to_string_lossy().into_owned())
}

async fn serve(pool: sqlx::SqlitePool) -> (SocketAddr, tokio::task::JoinHandle<()>, String) {
    session::migrate(&pool).await.unwrap();
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
            http::mobile_surface_router(state, std::sync::Arc::new(RunnerTaskActions)).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (address, server, token_path)
}

fn desktop_client() -> reqwest::Client {
    reqwest::Client::builder()
        .default_headers({
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {TEST_TOKEN}").parse().unwrap(),
            );
            headers
        })
        .build()
        .unwrap()
}

fn write_token_file() -> String {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    let suffix = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = temp_root::dir().join(format!(
        "praxis-mobile-session-token-{}-{suffix}",
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
