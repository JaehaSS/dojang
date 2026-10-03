//! 토론의 각 자리도 질문 실행을 독립적으로 열고 닫는다. 실제 두 질문 어댑터 대역을 함께
//! 돌려, 실행·답변·벤더 세션이 앞 자리의 것을 덮지 않는지 확인한다.
#![cfg(unix)]

use super::*;
use crate::convo::{
    app_server::{self, Context},
    interaction as ledger, interaction_commands, ConvoEvent, Side, Vendor,
};
use crate::preview_bridge::mcp::{self, ControlTokens, PreviewMcpLease};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const CLAUDE_CODEX_CLAUDE_TASK: i64 = 9_994_207;
const CLEANUP_BLOCK_TASK: i64 = 9_994_208;
const AGY_FALLBACK_TASK: i64 = 9_994_209;

fn block<T>(future: impl std::future::Future<Output = T>) -> T {
    tauri::async_runtime::block_on(future)
}

async fn pool(name: &str, task: i64) -> SqlitePool {
    let path = crate::testtmp::dir().join(format!(
        "question-debate-{name}-{}.sqlite",
        ledger::id().unwrap()
    ));
    let pool = db::init_pool(path.to_str().unwrap()).await.unwrap();
    let id = db::insert_task(
        &pool,
        "/repo",
        "praxis/question-debate",
        "main",
        "/repo/.praxis/wt/question-debate",
        "질문 토론",
        Some("claude"),
        None,
        "conversation",
        1000,
    )
    .await
    .unwrap();
    assert_eq!(id, 1);
    db::update_state(&pool, id, db::state::AWAITING_REVIEW, 1001)
        .await
        .unwrap();
    // 이 모듈의 전역 controller 키는 테스트 프로세스 전체에서 유일해야 한다. DB task id는
    // 로컬 풀마다 1이라 바꿔 쓰고, provider 쪽은 실제로 이 키를 쓴다.
    sqlx::query("UPDATE tasks SET id=? WHERE id=?")
        .bind(task)
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    ledger::bind_runtime(&pool, task, ledger::RUNTIME_LOCAL)
        .await
        .unwrap();
    db::insert_debate_sides(&pool, task, &[(Side::Right, "codex", None)])
        .await
        .unwrap();
    pool
}

struct Fixture {
    dir: PathBuf,
    codex: String,
    claude: String,
}

impl Fixture {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("praxis-question-debate-{}", ledger::id().unwrap()));
        std::fs::create_dir_all(&dir).unwrap();
        let codex = dir.join("codex-provider");
        let claude = dir.join("claude-provider");
        std::fs::write(
            &codex,
            include_str!("../../tests/fixtures/question_provider.py"),
        )
        .unwrap();
        std::fs::write(
            &claude,
            include_str!("../../tests/fixtures/question_local_provider.py"),
        )
        .unwrap();
        let mode = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&codex, mode.clone()).unwrap();
        std::fs::set_permissions(&claude, mode).unwrap();
        Self {
            dir,
            codex: codex.to_string_lossy().into(),
            claude: claude.to_string_lossy().into(),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct NoDispatch;

#[async_trait::async_trait]
impl mcp::Dispatcher for NoDispatch {
    async fn dispatch(
        &self,
        _task: i64,
        command: mcp::Command,
    ) -> Result<String, mcp::DispatchError> {
        panic!("question fixture must not dispatch a preview command: {command:?}");
    }
}

struct Server {
    endpoint: String,
    tokens: ControlTokens,
    handle: Option<tauri::async_runtime::JoinHandle<()>>,
}

impl Server {
    fn start() -> Self {
        let tokens = ControlTokens::default();
        let (port, listener) = block(mcp::bind()).unwrap();
        let state = Arc::new(mcp::McpState {
            instance: "question-debate".into(),
            tokens: tokens.clone(),
            dispatcher: Arc::new(NoDispatch),
            tools: mcp::Tools::phase_f(),
        });
        Self {
            endpoint: mcp::inject::endpoint_url(port, "question-debate"),
            tokens,
            handle: Some(tauri::async_runtime::spawn(mcp::serve_on(listener, state))),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
            let _ = block(handle);
        }
    }
}

/// 질문을 열고 기다리는 Claude 대역에는 별도 스레드가 답한다. 실제 UI도 changed 알림 후 다른
/// 요청에서 같은 ledger submit 경로를 탄다.
struct Watcher {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<usize>>,
}

impl Watcher {
    fn answer(pool: &SqlitePool, task: i64, execution: String) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let pool = pool.clone();
        let handle = std::thread::spawn(move || {
            let mut answered = HashSet::new();
            while !flag.load(Ordering::SeqCst) {
                let snapshot = block(ledger::snapshot(&pool, task)).unwrap();
                for item in snapshot
                    .items
                    .into_iter()
                    .filter(|item| item.execution_id == execution && item.state == "pending")
                {
                    if !answered.insert(item.id.clone()) {
                        continue;
                    }
                    block(ledger::submit(
                        &pool,
                        task,
                        &execution,
                        &item.id,
                        &ledger::id().unwrap(),
                        &[ledger::Answer {
                            question_id: "color".into(),
                            option_id: Some("blue".into()),
                            text: None,
                        }],
                        chrono::Utc::now().timestamp(),
                    ))
                    .unwrap();
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            answered.len()
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }

    fn finish(mut self) -> usize {
        self.stop.store(true, Ordering::SeqCst);
        self.handle.take().unwrap().join().unwrap()
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct ControlCleanup(i64);

impl Drop for ControlCleanup {
    fn drop(&mut self) {
        app_server::cancel(self.0);
        app_server::unregister(self.0);
    }
}

async fn start(pool: &SqlitePool, task: i64, agent: &str, side: Side) -> Context {
    next_question_turn(
        pool,
        task,
        agent,
        Some(side),
        interaction_commands::generation(),
        Arc::new(|| {}),
    )
    .await
    .unwrap()
    .expect("supported question provider")
}

async fn persist_session(pool: &SqlitePool, task: i64, side: Side, session: &str) {
    match side {
        Side::Right | Side::Third => db::set_debate_side_session(pool, task, side, session)
            .await
            .unwrap(),
        Side::Left => db::set_convo_session(pool, task, session).await.unwrap(),
    }
}

fn run_codex(
    pool: &SqlitePool,
    task: i64,
    fixture: &Fixture,
    ctx: &Context,
    resume: Option<&str>,
) -> String {
    let mut result = None;
    app_server::run_selected(
        Some(ctx),
        fixture.dir.to_str().unwrap(),
        "normal",
        resume,
        30,
        Vendor::Codex,
        &fixture.codex,
        None,
        None,
        None,
        &[],
        None,
        None,
        |_| {},
        |event| match event {
            ConvoEvent::Interaction { interaction_id } => {
                block(ledger::submit(
                    pool,
                    task,
                    &ctx.control.execution,
                    &interaction_id,
                    &ledger::id().unwrap(),
                    &[ledger::Answer {
                        question_id: "color".into(),
                        option_id: Some("blue".into()),
                        text: None,
                    }],
                    chrono::Utc::now().timestamp(),
                ))
                .unwrap();
            }
            ConvoEvent::SessionInit { session_id } => block(persist_session(
                pool,
                task,
                ctx.control.side.unwrap(),
                &session_id,
            )),
            ConvoEvent::Result { text, .. } => result = Some(text),
            _ => {}
        },
    )
    .unwrap();
    result.expect("Codex fixture result")
}

fn run_claude(
    pool: &SqlitePool,
    task: i64,
    fixture: &Fixture,
    server: &Server,
    ctx: &Context,
    resume: Option<&str>,
) -> String {
    let lease = PreviewMcpLease::issue_for(
        &server.tokens,
        task,
        Vendor::Claude,
        &server.endpoint,
        &fixture.dir,
        true,
    )
    .unwrap();
    let watcher = Watcher::answer(pool, task, ctx.control.execution.clone());
    let mut result = None;
    app_server::run_selected(
        Some(ctx),
        fixture.dir.to_str().unwrap(),
        "normal",
        resume,
        30,
        Vendor::Claude,
        &fixture.claude,
        None,
        None,
        None,
        &[],
        None,
        Some(&lease),
        |_| {},
        |event| match event {
            ConvoEvent::SessionInit { session_id } => block(persist_session(
                pool,
                task,
                ctx.control.side.unwrap(),
                &session_id,
            )),
            ConvoEvent::Result { text, .. } => result = Some(text),
            _ => {}
        },
    )
    .unwrap();
    assert_eq!(
        watcher.finish(),
        1,
        "Claude question did not reach the answer ledger"
    );
    assert_eq!(
        server.tokens.active_for_task(task),
        0,
        "turn lease was not revoked"
    );
    result.expect("Claude fixture result")
}

#[test]
fn claude_codex_claude_turns_keep_answers_and_sessions_on_their_seats() {
    let task = CLAUDE_CODEX_CLAUDE_TASK;
    let pool = block(pool("claude-codex-claude", task));
    let fixture = Fixture::new();
    let server = Server::start();
    let _cleanup = ControlCleanup(task);

    let left_first = block(start(&pool, task, "claude", Side::Left));
    let left_execution = left_first.control.execution.clone();
    assert!(run_claude(&pool, task, &fixture, &server, &left_first, None).contains("answered:"));
    assert_eq!(
        app_server::execution(task).as_deref(),
        Some(left_execution.as_str())
    );
    assert_eq!(
        block(
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT convo_session_id FROM tasks WHERE id=?"
            )
            .bind(task)
            .fetch_one(&pool)
        )
        .unwrap()
        .as_deref(),
        Some("local-session-1")
    );

    // 우측 세션을 쓰는 동안 좌측 캡슐을 지우면 다음 좌측 턴의 인계가 사라진다.
    block(
        sqlx::query("UPDATE tasks SET pending_capsule='left capsule' WHERE id=?")
            .bind(task)
            .execute(&pool),
    )
    .unwrap();
    let right = block(start(&pool, task, "codex", Side::Right));
    let right_execution = right.control.execution.clone();
    assert!(run_codex(&pool, task, &fixture, &right, None).contains("Blue"));
    let main: (Option<String>, Option<String>) = block(
        sqlx::query_as("SELECT convo_session_id,pending_capsule FROM tasks WHERE id=?")
            .bind(task)
            .fetch_one(&pool),
    )
    .unwrap();
    assert_eq!(
        main,
        (Some("local-session-1".into()), Some("left capsule".into()))
    );
    assert_eq!(
        block(db::debate_sides(&pool, task)).unwrap()[0]
            .vendor_session_id
            .as_deref(),
        Some("test-thread")
    );

    let left_second = block(start(&pool, task, "claude", Side::Left));
    let left_second_execution = left_second.control.execution.clone();
    assert!(run_claude(
        &pool,
        task,
        &fixture,
        &server,
        &left_second,
        Some("local-session-1")
    )
    .contains("answered:"));
    assert_eq!(
        app_server::execution(task).as_deref(),
        Some(left_second_execution.as_str())
    );

    let executions = [left_execution, right_execution, left_second_execution];
    assert_eq!(
        executions.iter().collect::<HashSet<_>>().len(),
        3,
        "each provider turn needs its own execution"
    );
    let rows: Vec<(String, String, Option<String>)> = block(sqlx::query_as(
        "SELECT id,state,(SELECT a.state FROM convo_interaction_answers a JOIN convo_interactions i ON i.id=a.interaction_id WHERE i.execution_id=e.id) FROM convo_executions e WHERE task_id=? ORDER BY created_at,rowid",
    ).bind(task).fetch_all(&pool)).unwrap();
    assert_eq!(rows.len(), 3);
    assert!(
        rows.iter().all(|(_, state, receipt)| state == "completed"
            && receipt.as_deref() == Some("acknowledged")),
        "{rows:?}"
    );
}

#[test]
fn cleanup_failed_question_blocks_the_next_debate_turn_until_repaired() {
    let task = CLEANUP_BLOCK_TASK;
    let pool = block(pool("cleanup-block", task));
    let _cleanup = ControlCleanup(task);
    let first = block(start(&pool, task, "claude", Side::Left));
    block(ledger::phase(
        &pool,
        &first.control.execution,
        "cleanup_failed",
    ))
    .unwrap();
    let error = block(next_question_turn(
        &pool,
        task,
        "codex",
        Some(Side::Right),
        interaction_commands::generation(),
        Arc::new(|| {}),
    ))
    .err()
    .expect("cleanup failure blocks replacement");
    assert!(error.contains("진행 중인 질문 실행"), "{error}");
    assert_eq!(
        app_server::execution(task).as_deref(),
        Some(first.control.execution.as_str())
    );

    block(ledger::finish(
        &pool,
        &first.control.execution,
        "failed",
        Some("repaired"),
    ))
    .unwrap();
    let second = block(start(&pool, task, "codex", Side::Right));
    assert_ne!(first.control.execution, second.control.execution);
    assert_eq!(
        app_server::execution(task).as_deref(),
        Some(second.control.execution.as_str())
    );
}

#[test]
fn unsupported_agt_turn_retires_a_completed_question_controller() {
    let task = AGY_FALLBACK_TASK;
    let pool = block(pool("agy-fallback", task));
    let _cleanup = ControlCleanup(task);
    let ctx = block(start(&pool, task, "codex", Side::Right));
    block(finish_unstarted_question(&ctx)).unwrap();
    assert_eq!(block(ledger::snapshot(&pool, task)).unwrap().phase, "idle");

    let fallback = block(next_question_turn(
        &pool,
        task,
        "agy",
        Some(Side::Right),
        interaction_commands::generation(),
        Arc::new(|| {}),
    ))
    .unwrap();
    assert!(fallback.is_none());
    assert!(app_server::execution(task).is_none());
}
