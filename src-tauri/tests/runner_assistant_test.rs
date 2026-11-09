#[path = "support/temp_root.rs"]
mod temp_root;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};

use praxis_lib::db;
use praxis_lib::runner::actions::RunnerTaskActions;
use praxis_lib::runner::auth::RunnerAuth;
use praxis_lib::runner::config::{ExecutionPolicy, RunnerConfig};
use praxis_lib::runner::events::EventHub;
use praxis_lib::runner::http::{self, RunnerHttpState};
use praxis_lib::runner::queue::QueueWorker;
use serde_json::{json, Value};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);
const TEST_TOKEN: &str = "abababababababababababababababababababababababababababababababab";

async fn test_pool(label: &str) -> (sqlx::SqlitePool, String) {
    let unique = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = temp_root::dir().join(format!(
        "praxis-runner-assistant-{label}-{}-{unique}.sqlite",
        std::process::id()
    ));
    let path_str = path.to_string_lossy().into_owned();
    let pool = db::init_pool(&path_str).await.unwrap();
    praxis_lib::memory::migrate(&pool).await.unwrap();
    (pool, path_str)
}

fn write_token_file() -> String {
    let unique = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = temp_root::dir().join(format!(
        "praxis-runner-assistant-token-{}-{unique}",
        std::process::id()
    ));
    std::fs::write(&path, TEST_TOKEN).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    path.to_string_lossy().into_owned()
}

fn create_git_repo(label: &str) -> std::path::PathBuf {
    let unique = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = temp_root::dir().join(format!(
        "praxis-assistant-repo-{label}-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let run = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "runner@example.test"]);
    run(&["config", "user.name", "Runner Test"]);
    std::fs::write(root.join("README.md"), "hello\n").unwrap();
    run(&["add", "README.md"]);
    run(&["commit", "-qm", "initial"]);
    root.canonicalize().unwrap()
}

fn authenticated_client() -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {TEST_TOKEN}").parse().unwrap(),
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap()
}

async fn serve_assistant(
    pool: sqlx::SqlitePool,
    repo: std::path::PathBuf,
    policy: ExecutionPolicy,
) -> (SocketAddr, tokio::task::JoinHandle<()>, String) {
    let token_path = write_token_file();
    let config = RunnerConfig {
        bind: "127.0.0.1:47831".parse().unwrap(),
        repository_roots: vec![repo.clone()],
        max_concurrent_tasks: 2,
        execution_policy: policy,
        pairing_token_file: token_path.clone().into(),
        assistant_workspace: Some(repo),
        assistant_default_agent: "claude".into(),
        assistant_mcp_config: None,
    };
    let queue = QueueWorker::new(pool.clone(), 2);
    let state = RunnerHttpState {
        auth: RunnerAuth::from_file(token_path.as_ref()).unwrap(),
        events: EventHub::start(pool.clone()),
        pool,
        config,
        recovered_tasks: 0,
        queue,
        started_at: 100,
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

#[tokio::test]
async fn singleton_assistant_is_isolated_from_coding_task_list_and_supports_vendor_handoff() {
    let (pool, db_path) = test_pool("singleton").await;
    let repo = create_git_repo("singleton");
    let (addr, server, token_path) =
        serve_assistant(pool.clone(), repo.clone(), ExecutionPolicy::AlwaysApprove).await;
    let client = authenticated_client();

    // 1. 최초 GET /v1/assistant 호출 시 단일 비서 세션이 자동 생성된다.
    let snap1: Value = client
        .get(format!("http://{addr}/v1/assistant"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let assistant_id = snap1["task"]["id"].as_i64().unwrap();
    assert_eq!(snap1["task"]["kind"], "assistant");
    assert_eq!(snap1["task"]["agent"], "claude");

    // 2. 두 번째 호출도 동일한 비서 task id를 반환한다(멱등).
    let snap2: Value = client
        .get(format!("http://{addr}/v1/assistant"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(snap2["task"]["id"].as_i64().unwrap(), assistant_id);

    // 3. 코딩 작업 목록(/v1/tasks)에는 비서 세션이 섞이지 않는다.
    let tasks: Vec<Value> = client
        .get(format!("http://{addr}/v1/tasks"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(tasks.is_empty());

    // 4. 대화 기록이 있는 상태에서 벤더를 codex로 전환하면 convo_session_id는 비워지고 pending_capsule이 채워진다.
    db::set_convo_session(&pool, assistant_id, "claude-sess-123")
        .await
        .unwrap();
    db::append_convo_event(
        &pool,
        assistant_id,
        &json!({ "kind": "user", "text": "오늘 백엔드 테스트 상태 확인해줘" }).to_string(),
        10,
    )
    .await
    .unwrap();
    db::append_convo_event(
        &pool,
        assistant_id,
        &json!({ "kind": "text", "text": "현재 러너 테스트 3건이 모두 통과했습니다." }).to_string(),
        11,
    )
    .await
    .unwrap();

    let updated: Value = client
        .patch(format!("http://{addr}/v1/assistant/config"))
        .json(&json!({
            "agent": "codex",
            "model": "gpt-5.4",
            "reasoning_effort": "high"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["agent"], "codex");
    assert_eq!(updated["model"], "gpt-5.4");
    assert!(updated["convo_session_id"].is_null());
    let capsule = updated["pending_capsule"].as_str().unwrap();
    assert!(capsule.contains("오늘 백엔드 테스트 상태 확인해줘"));
    assert!(capsule.contains("현재 러너 테스트 3건이 모두 통과했습니다."));

    server.abort();
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(token_path);
    let _ = std::fs::remove_dir_all(repo);
}

#[tokio::test]
async fn orchestrator_mcp_dispatches_tasks_and_executes_approval_lifecycle() {
    let (pool, db_path) = test_pool("mcp-approval").await;
    let repo = create_git_repo("mcp-approval");
    let (addr, server, token_path) =
        serve_assistant(pool.clone(), repo.clone(), ExecutionPolicy::RequireApproval).await;
    let client = authenticated_client();

    // 1. MCP tools/list 확인
    let list_res: Value = client
        .post(format!("http://{addr}/v1/mcp/orchestrator"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tools = list_res["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 7);

    // 2. MCP dojang_task_dispatch 호출로 새 코딩 작업 생성 (RequireApproval 정책이므로 PendingApproval 상태)
    let dispatch_res: Value = client
        .post(format!("http://{addr}/v1/mcp/orchestrator"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "dojang_task_dispatch",
                "arguments": {
                    "repository": repo.to_string_lossy(),
                    "instruction": "로그인 리다이렉트 버그 수정",
                    "agent": "codex"
                }
            }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let payload_text = dispatch_res["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    let dispatched: Value = serde_json::from_str(payload_text).unwrap();
    let target_task_id = dispatched["task"]["id"].as_i64().unwrap();
    assert_eq!(dispatched["task"]["state"], db::state::PENDING_APPROVAL);
    assert_eq!(dispatched["task"]["kind"], "coding");

    // 3. MCP dojang_request_approval 호출로 보류 중 작업의 실행 허가 카드 생성
    let approval_res: Value = client
        .post(format!("http://{addr}/v1/mcp/orchestrator"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "dojang_request_approval",
                "arguments": {
                    "action_kind": "run_pending_task",
                    "target_task_id": target_task_id,
                    "summary": format!("작업 #{target_task_id} 실행 허가")
                }
            }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let approval_text = approval_res["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    let approval_json: Value = serde_json::from_str(approval_text).unwrap();
    let approval_id = approval_json["approval"]["id"].as_i64().unwrap();
    assert_eq!(approval_json["approval"]["status"], "pending");

    // 4. POST /v1/assistant/approvals/:id/approve 호출 시 실제 작업이 Queued로 전이되고 카드 상태가 executed가 된다.
    let approved_card: Value = client
        .post(format!(
            "http://{addr}/v1/assistant/approvals/{approval_id}/approve"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(approved_card["status"], "executed");

    let target_after = db::get_task(&pool, target_task_id).await.unwrap().unwrap();
    assert_eq!(target_after.state, db::state::QUEUED);

    // 5. 이미 처리된 승인 카드를 다시 승인하면 409 Conflict를 반환한다.
    let dup = client
        .post(format!(
            "http://{addr}/v1/assistant/approvals/{approval_id}/approve"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(dup.status(), reqwest::StatusCode::CONFLICT);

    server.abort();
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(token_path);
    let _ = std::fs::remove_dir_all(repo);
}
