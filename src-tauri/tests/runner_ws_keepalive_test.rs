//! WebSocket keepalive와 replay 페이징 (설계 0013 §7.3)
//!
//! 이 테스트 바이너리는 별도 프로세스라 ping 주기 환경변수를 안전하게 줄일 수 있다.

#[path = "support/temp_root.rs"]
mod temp_root;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use praxis_lib::db;
use praxis_lib::runner::auth::RunnerAuth;
use praxis_lib::runner::config::RunnerConfig;
use praxis_lib::runner::events::{EventHub, REPLAY_LIMIT};
use praxis_lib::runner::actions::RunnerTaskActions;
use praxis_lib::runner::http::{self, RunnerHttpState};
use praxis_lib::runner::queue::QueueWorker;
use tokio_tungstenite::tungstenite;

static COUNTER: AtomicU32 = AtomicU32::new(0);
const TEST_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// 모든 테스트가 시작되기 전에 주기를 1초로 줄인다. 프로세스 단위라 다른 테스트에 영향 없다.
fn shorten_ping_interval() {
    std::env::set_var("PRAXIS_RUNNER_WS_PING_SECS", "1");
}

#[tokio::test]
async fn 서버는_주기적으로_ping을_보낸다() {
    shorten_ping_interval();
    let (pool, db_path) = test_pool("ws-ping").await;
    let (address, server, token_path) = serve(pool).await;

    let mut socket = connect(&address, 0).await;
    // 첫 메시지는 watermark. 그 다음 keepalive ping이 와야 한다.
    let mut saw_ping = false;
    for _ in 0..5 {
        let message = tokio::time::timeout(Duration::from_secs(4), socket.next())
            .await
            .expect("ping이 오지 않았다 — 좀비 소켓을 탐지할 수 없다")
            .unwrap()
            .unwrap();
        if matches!(message, tungstenite::Message::Ping(_)) {
            saw_ping = true;
            break;
        }
    }
    assert!(saw_ping, "keepalive ping을 받지 못했다");

    server.abort();
    cleanup(&db_path, &token_path);
}

#[tokio::test]
async fn pong이_끊기면_서버가_소켓을_정리한다() {
    shorten_ping_interval();
    let (pool, db_path) = test_pool("ws-zombie").await;
    let (address, server, token_path) = serve(pool).await;

    // tungstenite는 read할 때 ping에 자동 응답한다. 읽지 않으면 pong도 나가지 않아
    // "응답하지 않는 클라이언트"를 그대로 재현한다.
    let socket = connect(&address, 0).await;
    tokio::time::sleep(Duration::from_secs(4)).await;

    // 서버가 끊었다면 스트림이 종료되거나 에러로 끝난다.
    let mut socket = socket;
    let closed = loop {
        match tokio::time::timeout(Duration::from_secs(3), socket.next()).await {
            Err(_) => break false,
            Ok(None) => break true,
            Ok(Some(Err(_))) => break true,
            // 대기 중 쌓여 있던 ping/close 프레임은 흘려보낸다.
            Ok(Some(Ok(tungstenite::Message::Close(_)))) => break true,
            Ok(Some(Ok(_))) => continue,
        }
    };
    assert!(closed, "pong 없는 소켓이 계속 열려 있으면 좀비가 쌓인다");

    server.abort();
    cleanup(&db_path, &token_path);
}

#[tokio::test]
async fn replay가_상한을_넘어도_watermark까지_빠짐없이_전달된다() {
    shorten_ping_interval();
    let (pool, db_path) = test_pool("ws-replay").await;
    let task_id = db::insert_task(
        &pool, "/tmp", "branch", "main", "/tmp", "replay", None, None, "terminal", 1,
    )
    .await
    .unwrap();

    // REPLAY_LIMIT을 넘겨 쌓는다 — 오래 오프라인이었던 모바일이 재접속하는 상황.
    let total = REPLAY_LIMIT + 25;
    for index in 0..total {
        db::append_runner_event(&pool, task_id, index + 2, "output", None)
            .await
            .unwrap();
    }

    let (address, server, token_path) = serve(pool.clone()).await;
    let mut socket = connect(&address, 0).await;

    let mut watermark = 0_i64;
    let mut received = Vec::new();
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .expect("replay가 끝나지 않았다")
            .unwrap()
            .unwrap();
        let tungstenite::Message::Text(text) = message else {
            continue;
        };
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        if value["kind"] == "watermark" {
            watermark = value["sequence"].as_i64().unwrap();
            continue;
        }
        received.push(value["sequence"].as_i64().unwrap());
        if received.len() as i64 >= total {
            break;
        }
    }

    assert_eq!(watermark, received.last().copied().unwrap());
    assert_eq!(
        received.len() as i64,
        total,
        "REPLAY_LIMIT 이후 구간이 조용히 사라지면 안 된다"
    );
    // 순서가 단조 증가여야 커서 갱신이 안전하다.
    assert!(received.windows(2).all(|pair| pair[0] < pair[1]));

    server.abort();
    cleanup(&db_path, &token_path);
}

async fn connect(
    address: &SocketAddr,
    after: i64,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let request = tungstenite::handshake::client::Request::builder()
        .uri(format!("ws://{address}/v1/events/live?after={after}"))
        .header("Host", address.to_string())
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tungstenite::handshake::client::generate_key(),
        )
        .header("Sec-WebSocket-Protocol", format!("praxis, {TEST_TOKEN}"))
        .body(())
        .unwrap();
    let (socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket
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
            http::mobile_surface_router(state, std::sync::Arc::new(RunnerTaskActions)).into_make_service_with_connect_info::<SocketAddr>(),
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
        "praxis-ws-keepalive-token-{}-{suffix}",
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
