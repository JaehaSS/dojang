//! 세션홈 승계 HTTP 계약 테스트(설계 2026-09-17: 세션홈에서 벤더 세션을 골라 새 대화
//! 작업으로 이어받기).
//!
//! 이 파일은 실제 `$HOME`을 읽지 않는다 — `serve_with_roots`가 처음 불릴 때
//! `sessionhome::set_projects_root`로 픽스처 세션홈을 프로세스에 한 번 박고, 이후 모든
//! 해석이 그 디렉터리에서만 일어난다. `HOME` 환경변수를 바꾸는 방법과 달리 같은 프로세스의
//! git 등 다른 소비자에 번지지 않는다.
//!
//! 덮는 계약: 세션 미해석 → 404(설계 결정 9), 인가 루트 **밖** cwd → 같은 404 + 목록에도
//! 없음, cwd가 **삭제된** 인가 루트 안 세션 → 승계 성립(인가 완화의 회귀 지점), 중복 승계 →
//! 409 + 점유 중인 작업 id, `validate_request` 거절(모드·agy), 모바일 차단
//! (`POST /v1/tasks`의 `resume_session`), 미인증 거절. 세션홈 목록(`GET /v1/sessions`)은
//! Runner 전용 라우트였고 1.0에서 모바일 서피스와 함께 남지 않았다.

#[path = "support/temp_root.rs"]
mod temp_root;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

use praxis_lib::db;
use praxis_lib::runner::auth::RunnerAuth;
use praxis_lib::runner::config::RunnerConfig;
use praxis_lib::runner::events::EventHub;
use praxis_lib::runner::actions::RunnerTaskActions;
use praxis_lib::runner::http::{self, RunnerHttpState};
use praxis_lib::runner::queue::QueueWorker;
use praxis_lib::runner::session;

static COUNTER: AtomicU32 = AtomicU32::new(0);
const TEST_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// 실제 세션홈 어디에도 있을 수 없는 무작위 UUID. `is_valid_session_id`를 통과하도록
/// 표준 UUID 문법(8-4-4-4-12, 하이픈 위치 고정)을 지킨다.
const NONEXISTENT_SESSION_ID: &str = "deadbeef-dead-4eef-8eef-deadbeefdead";

#[tokio::test]
async fn resume_session_targeting_unresolvable_session_is_rejected_as_not_found() {
    let (pool, db_path) = test_pool("notfound").await;
    let repo = git_repository("notfound");
    let (address, server, token_path) =
        serve_with_roots(pool.clone(), vec![repo.canonicalize().unwrap()]).await;

    let response = authenticated_client()
        .post(format!("http://{address}/v1/tasks"))
        .json(&serde_json::json!({
            "repository": repo.to_string_lossy(),
            "instruction": "continue where we left off",
            "agent": "claude",
            "role": "implementer",
            "model": "",
            "reasoning_effort": "",
            "mode": "conversation",
            "resume_session": NONEXISTENT_SESSION_ID,
        }))
        .send()
        .await
        .unwrap();
    // 없음과 인가 실패는 같은 얼굴이다(설계 결정 9) — 존재 열거를 막는다.
    assert_eq!(response.status(), 404);

    let tasks = db::list_tasks(&pool).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].state, db::state::FAILED);
    assert!(
        !std::path::Path::new(&tasks[0].worktree_path).exists(),
        "실패한 승계는 격리 워크트리를 남기지 않는다"
    );

    server.abort();
    cleanup(&db_path, &token_path, &repo);
}

#[tokio::test]
async fn resume_session_requires_conversation_mode_over_http() {
    let (pool, db_path) = test_pool("terminal-mode").await;
    let repo = git_repository("terminal-mode");
    let (address, server, token_path) =
        serve_with_roots(pool.clone(), vec![repo.canonicalize().unwrap()]).await;

    let response = authenticated_client()
        .post(format!("http://{address}/v1/tasks"))
        .json(&serde_json::json!({
            "repository": repo.to_string_lossy(),
            "instruction": "continue where we left off",
            "agent": "claude",
            "role": "implementer",
            "model": "",
            "reasoning_effort": "",
            "mode": "terminal",
            "resume_session": NONEXISTENT_SESSION_ID,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);

    assert!(db::list_tasks(&pool).await.unwrap().is_empty());

    server.abort();
    cleanup(&db_path, &token_path, &repo);
}

#[tokio::test]
async fn resume_session_rejects_agy_over_http() {
    let (pool, db_path) = test_pool("agy").await;
    let repo = git_repository("agy");
    let (address, server, token_path) =
        serve_with_roots(pool.clone(), vec![repo.canonicalize().unwrap()]).await;

    let response = authenticated_client()
        .post(format!("http://{address}/v1/tasks"))
        .json(&serde_json::json!({
            "repository": repo.to_string_lossy(),
            "instruction": "continue where we left off",
            "agent": "agy",
            "role": "implementer",
            "model": "",
            "reasoning_effort": "",
            "mode": "conversation",
            "resume_session": NONEXISTENT_SESSION_ID,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body = response.text().await.unwrap();
    assert!(body.contains("agy"), "{body}");

    assert!(db::list_tasks(&pool).await.unwrap().is_empty());

    server.abort();
    cleanup(&db_path, &token_path, &repo);
}

#[tokio::test]
async fn unauthenticated_resume_request_is_rejected() {
    let (pool, db_path) = test_pool("unauth").await;
    let repo = git_repository("unauth");
    let (address, server, token_path) =
        serve_with_roots(pool.clone(), vec![repo.canonicalize().unwrap()]).await;

    let response = reqwest::Client::new()
        .post(format!("http://{address}/v1/tasks"))
        .json(&serde_json::json!({
            "repository": repo.to_string_lossy(),
            "instruction": "continue where we left off",
            "agent": "claude",
            "role": "implementer",
            "model": "",
            "reasoning_effort": "",
            "mode": "conversation",
            "resume_session": NONEXISTENT_SESSION_ID,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);

    server.abort();
    cleanup(&db_path, &token_path, &repo);
}

#[tokio::test]
async fn mobile_session_cannot_resume_a_vendor_session() {
    let (pool, db_path) = test_pool("mobile-resume").await;
    let repo = git_repository("mobile-resume");
    let (address, server, token_path) =
        serve_with_roots(pool.clone(), vec![repo.canonicalize().unwrap()]).await;
    let session_cookie = pair_device(&address).await;
    let origin = format!("http://{address}");

    // 모바일이 여는 보통 작업 생성(승계 없음)은 통과해야 한다 — 이 경로 자체를 막는 게
    // 아니라 `resume_session`을 실은 요청만 거절한다.
    let plain = reqwest::Client::new()
        .post(format!("http://{address}/v1/tasks"))
        .header(reqwest::header::COOKIE, &session_cookie)
        .header(reqwest::header::ORIGIN, &origin)
        .json(&serde_json::json!({
            "repository": repo.to_string_lossy(),
            "instruction": "fresh task from mobile",
            "agent": "claude",
            "role": "implementer",
            "model": "",
            "reasoning_effort": "",
            "mode": "conversation",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        plain.status(),
        200,
        "resume_session 없는 모바일 작업 생성은 막지 않는다"
    );

    let resume = reqwest::Client::new()
        .post(format!("http://{address}/v1/tasks"))
        .header(reqwest::header::COOKIE, &session_cookie)
        .header(reqwest::header::ORIGIN, &origin)
        .json(&serde_json::json!({
            "repository": repo.to_string_lossy(),
            "instruction": "continue where we left off",
            "agent": "claude",
            "role": "implementer",
            "model": "",
            "reasoning_effort": "",
            "mode": "conversation",
            "resume_session": NONEXISTENT_SESSION_ID,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resume.status(),
        403,
        "모바일은 resume_session을 실은 작업 생성을 못 한다 — `mobile_scope_denies`는 본문을 못 보므로 핸들러가 막는다"
    );

    // 거절은 승계 시도 전이어야 한다 — task가 만들어졌다가 실패로 남지 않는다.
    let tasks = db::list_tasks(&pool).await.unwrap();
    assert_eq!(tasks.len(), 1, "성공한 plain 작업 하나만 남아야 한다");
    assert_ne!(tasks[0].instruction, "continue where we left off");

    server.abort();
    cleanup(&db_path, &token_path, &repo);
}

/// 목록 필터를 우회해 세션 id를 직접 실어도 인가 루트 밖 세션은 승계되지 않는다(설계 제약 3).
/// 실패 얼굴은 "없음"과 같은 404다(결정 9) — 세션의 존재를 열거할 수 없게 한다.
#[tokio::test]
async fn resume_session_outside_repository_roots_is_rejected_as_not_found() {
    let (pool, db_path) = test_pool("outside-root").await;
    let repo = git_repository("outside-root");
    let outside = temp_root::dir().join(format!(
        "praxis-runner-resume-outside-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&outside).unwrap();
    let session_id = write_session_file("outside", &outside);
    let (address, server, token_path) =
        serve_with_roots(pool.clone(), vec![repo.canonicalize().unwrap()]).await;

    let response = create_resuming_task(&address, &repo, &session_id).await;
    assert_eq!(response.status(), 404);

    let tasks = db::list_tasks(&pool).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].state, db::state::FAILED);
    assert!(tasks[0].resumed_session.is_none());

    server.abort();
    let _ = std::fs::remove_dir_all(&outside);
    cleanup(&db_path, &token_path, &repo);
}

/// **인가 완화의 회귀 지점.** 이 기능의 주 사용은 "끝난 워크트리에서 돌던 세션을 다시 집는
/// 것"이라, cwd 디렉터리는 대개 이미 사라져 있다. `authorize_repository_path`처럼
/// `canonicalize()` 실패를 거절로 다루면 그 사용이 통째로 막힌다 — `sessionhome::authorize_cwd`가
/// 존재하는 조상까지만 정규화하는 이유다(설계 제약 3).
#[tokio::test]
async fn resume_session_whose_cwd_was_deleted_under_an_authorized_root_is_adopted() {
    let (pool, db_path) = test_pool("deleted-cwd").await;
    let repo = git_repository("deleted-cwd");
    let canonical_repo = repo.canonicalize().unwrap();
    // 정리된 Praxis 워크트리의 모양 그대로 — 경로는 루트 안이지만 디렉터리는 없다.
    let gone = canonical_repo.join(".praxis/worktrees/praxis-finished-1789000000000000000");
    assert!(!gone.exists());
    let session_id = write_session_file("deleted-cwd", &gone);
    let (address, server, token_path) =
        serve_with_roots(pool.clone(), vec![canonical_repo.clone()]).await;

    let response = create_resuming_task(&address, &repo, &session_id).await;
    assert_eq!(
        response.status(),
        200,
        "cwd가 사라졌어도 경로가 인가 루트 안이면 승계는 성립한다"
    );

    let tasks = db::list_tasks(&pool).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].resumed_session.as_deref(), Some(session_id.as_str()));
    assert_eq!(
        tasks[0].convo_session_id.as_deref(),
        Some(session_id.as_str()),
        "벤더가 `--resume`으로 이어 쓸 id는 대화 세션 칼럼에도 실려야 한다"
    );

    server.abort();
    cleanup(&db_path, &token_path, &repo);
}

/// 같은 외부 세션을 두 작업이 동시에 물면 두 번째는 409로 거절되고, **어느 작업이 점유 중인지**
/// 본문으로 알려준다(설계 결정 9) — 사용자가 그 작업으로 이동할 수 있어야 한다.
#[tokio::test]
async fn resuming_a_session_already_held_by_a_live_task_conflicts_with_the_holder_id() {
    let (pool, db_path) = test_pool("conflict").await;
    let repo = git_repository("conflict");
    let canonical_repo = repo.canonicalize().unwrap();
    let session_id = write_session_file("conflict", &canonical_repo);
    let (address, server, token_path) =
        serve_with_roots(pool.clone(), vec![canonical_repo.clone()]).await;

    let first = create_resuming_task(&address, &repo, &session_id).await;
    assert_eq!(first.status(), 200);
    let holder: serde_json::Value = first.json().await.unwrap();
    let holder_id = holder["id"].as_i64().unwrap();

    let second = create_resuming_task(&address, &repo, &session_id).await;
    assert_eq!(second.status(), 409);
    let body: serde_json::Value = second.json().await.unwrap();
    assert_eq!(
        body["task_id"].as_i64(),
        Some(holder_id),
        "409는 점유 중인 작업 id를 실어야 한다"
    );

    let tasks = db::list_tasks(&pool).await.unwrap();
    assert_eq!(tasks.len(), 2);
    let loser = tasks.iter().find(|task| task.id != holder_id).unwrap();
    assert_eq!(loser.state, db::state::FAILED);
    assert!(loser.resumed_session.is_none());
    assert!(
        !std::path::Path::new(&loser.worktree_path).exists(),
        "충돌로 실패한 승계는 격리 워크트리를 남기지 않는다"
    );

    server.abort();
    cleanup(&db_path, &token_path, &repo);
}

async fn create_resuming_task(
    address: &SocketAddr,
    repo: &std::path::Path,
    session_id: &str,
) -> reqwest::Response {
    authenticated_client()
        .post(format!("http://{address}/v1/tasks"))
        .json(&serde_json::json!({
            "repository": repo.to_string_lossy(),
            "instruction": "continue where we left off",
            "agent": "claude",
            "role": "implementer",
            "model": "",
            "reasoning_effort": "",
            "mode": "conversation",
            "resume_session": session_id,
        }))
        .send()
        .await
        .unwrap()
}

/// 프로세스에 한 번 박는 픽스처 세션홈. 실제 `$HOME`을 읽지 않게 하는 장치다.
fn session_home() -> &'static std::path::Path {
    static ROOT: OnceLock<std::path::PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = temp_root::dir().join(format!(
            "praxis-runner-resume-home-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        praxis_lib::sessionhome::set_projects_root(root.clone())
            .expect("세션홈 루트는 프로세스당 한 번만 정해진다");
        root
    })
    .as_path()
}

/// 벤더(claude)가 프로젝트 디렉터리 이름을 지을 때 쓰는 것과 같은 치환 — `/`와 `.`를 `-`로
/// 바꾼다. `sessionhome`은 이 변환을 canonicalize 없는 순수 문자열 변환으로만 쓰고 프로덕션에
/// 노출하지 않으므로, 픽스처 이름을 짓는 이 테스트에서만 그대로 흉내낸다.
fn encode_cwd_like_vendor(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect()
}

/// 벤더가 쓰는 모양의 세션 파일 하나를 픽스처 세션홈에 심고 session_id를 돌려준다.
/// 프로젝트 디렉터리 이름은 실제와 같이 cwd 인코딩으로 짓는다.
fn write_session_file(label: &str, cwd: &std::path::Path) -> String {
    let suffix = COUNTER.fetch_add(1, Ordering::SeqCst);
    // UUID 문법(8-4-4-4-12)을 지켜야 `is_valid_session_id`를 통과한다.
    let session_id = format!("{:08x}-1111-4222-8333-444444444444", suffix + 1);
    let cwd_text = cwd.to_string_lossy();
    let project_dir = session_home().join(encode_cwd_like_vendor(&cwd_text));
    std::fs::create_dir_all(&project_dir).unwrap();
    let lines = [
        format!(
            r#"{{"type":"attachment","sessionId":"{session_id}","cwd":"{cwd_text}","gitBranch":"main","version":"2.1.274"}}"#
        ),
        format!(
            r#"{{"type":"user","message":{{"role":"user","content":"fixture {label}"}}}}"#
        ),
    ];
    std::fs::write(
        project_dir.join(format!("{session_id}.jsonl")),
        lines.join("\n"),
    )
    .unwrap();
    session_id
}

async fn test_pool(label: &str) -> (sqlx::SqlitePool, String) {
    let suffix = COUNTER.fetch_add(1, Ordering::SeqCst);
    let db_path = temp_root::dir()
        .join(format!(
            "praxis-runner-resume-{label}-{}-{suffix}.sqlite",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned();
    let pool = db::init_pool(&db_path).await.unwrap();
    praxis_lib::memory::migrate(&pool).await.unwrap();
    praxis_lib::runner::finalization::migrate(&pool)
        .await
        .unwrap();
    session::migrate(&pool).await.unwrap();
    (pool, db_path)
}

fn git_repository(label: &str) -> std::path::PathBuf {
    let root = temp_root::dir().join(format!(
        "praxis-runner-resume-repo-{label}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "runner@example.test"]);
    git(&root, &["config", "user.name", "Runner Test"]);
    std::fs::write(root.join("note.txt"), "before\n").unwrap();
    git(&root, &["add", "note.txt"]);
    git(&root, &["commit", "-qm", "initial"]);
    root
}

fn git(root: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .current_dir(root)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

async fn serve_with_roots(
    pool: sqlx::SqlitePool,
    repository_roots: Vec<std::path::PathBuf>,
) -> (SocketAddr, tokio::task::JoinHandle<()>, String) {
    // 서버를 세우는 모든 테스트가 같은 픽스처 세션홈을 본다 — 실제 `$HOME`에 기대는 테스트가
    // 하나도 남지 않게 하려고 여기서 설치한다.
    session_home();
    let token_path = write_token_file();
    let config = RunnerConfig {
        bind: "127.0.0.1:47831".parse().unwrap(),
        repository_roots,
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

fn authenticated_client() -> reqwest::Client {
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

async fn pair_device(address: &SocketAddr) -> String {
    let code: serde_json::Value = authenticated_client()
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

fn write_token_file() -> String {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    let suffix = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = temp_root::dir().join(format!(
        "praxis-runner-resume-token-{}-{suffix}",
        std::process::id()
    ));
    std::fs::write(&path, TEST_TOKEN).unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    path.to_string_lossy().into_owned()
}

fn cleanup(db_path: &str, token_path: &str, repo: &std::path::Path) {
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(token_path);
    let _ = std::fs::remove_dir_all(repo);
}

