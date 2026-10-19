//! 로컬(MCP) 질문 런타임의 계약. 대역이 진짜 claude처럼 stream-json을 내고, 질문은 인앱 MCP
//! 툴로 건다 — 툴 호출이 답을 기다리며 멈추는지, 그 대기가 턴 종료에 풀리는지가 요점이다.
#![cfg(unix)]
use praxis_lib::convo::{
    app_server::{self, Context},
    interaction as ledger, ConvoEvent, Vendor,
};
use praxis_lib::preview_bridge::mcp::{self, ControlTokens, PreviewMcpLease};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn db<T>(f: impl std::future::Future<Output = T>) -> T {
    tauri::async_runtime::block_on(f)
}

/// 테스트는 한 프로세스에서 나란히 돈다. 세션 등록도 취소도 **task 단위 전역**이라, 작업 번호를
/// 나눠 쓰지 않으면 서로의 등록을 거둬간다.
fn pool(task: i64) -> sqlx::SqlitePool {
    db(async {
        let p = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(4)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE tasks(id INTEGER PRIMARY KEY,convo_session_id TEXT,pending_capsule TEXT)",
        )
        .execute(&p)
        .await
        .unwrap();
        sqlx::query("INSERT INTO tasks(id) VALUES(?)")
            .bind(task)
            .execute(&p)
            .await
            .unwrap();
        ledger::migrate(&p).await.unwrap();
        ledger::bind_runtime(&p, task, ledger::RUNTIME_LOCAL)
            .await
            .unwrap();
        p
    })
}

struct Fixture {
    dir: PathBuf,
    bin: String,
}
impl Fixture {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("praxis-question-local-{}", ledger::id().unwrap()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("provider");
        std::fs::write(&path, include_str!("fixtures/question_local_provider.py")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            bin: path.to_string_lossy().into(),
            dir,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 브라우저 툴은 이 테스트에서 한 번도 불리지 않는다 — 불렸다면 라우팅이 샌 것이다.
struct NoDispatch;
#[async_trait::async_trait]
impl mcp::Dispatcher for NoDispatch {
    async fn dispatch(&self, _task: i64, cmd: mcp::Command) -> Result<String, mcp::DispatchError> {
        panic!("preview dispatcher should not be reached: {cmd:?}");
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
        let (port, listener) = db(mcp::bind()).unwrap();
        let state = Arc::new(mcp::McpState {
            instance: "question-local".into(),
            tokens: tokens.clone(),
            dispatcher: Arc::new(NoDispatch),
            review: Arc::new(mcp::NoReview),
            tools: mcp::Tools::phase_f(),
        });
        Self {
            endpoint: mcp::inject::endpoint_url(port, "question-local"),
            tokens,
            handle: Some(tauri::async_runtime::spawn(mcp::serve_on(listener, state))),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
            let _ = db(handle);
        }
    }
}

struct Outcome {
    result: Result<praxis_lib::convo::TurnOutcome, String>,
    events: Vec<ConvoEvent>,
    execution: String,
}

/// 질문이 열리면 UI 대신 감시 스레드가 깨어 답한다 — 실제 경로도 `changed` 신호를 받고 스냅샷을
/// 다시 읽는다. 툴 호출이 tokio 위에서 멈춰 있으므로 답은 **다른 스레드에서** 와야 한다.
struct Watcher {
    stop: Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<usize>>,
}
impl Watcher {
    fn start(
        p: &sqlx::SqlitePool,
        task: i64,
        execution: String,
        answer: Option<&'static str>,
        cancel: bool,
    ) -> Self {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop.clone();
        let pool = p.clone();
        let handle = std::thread::spawn(move || {
            let mut seen = 0usize;
            let mut handled = std::collections::HashSet::new();
            while !flag.load(Ordering::SeqCst) {
                let snapshot = db(ledger::snapshot(&pool, task)).unwrap();
                for item in snapshot.items.iter().filter(|i| i.state == "pending") {
                    if !handled.insert(item.id.clone()) {
                        continue;
                    }
                    seen += 1;
                    if cancel {
                        app_server::cancel(task);
                    } else if let Some(option) = answer {
                        db(ledger::submit(
                            &pool,
                            task,
                            &execution,
                            &item.id,
                            &ledger::id().unwrap(),
                            &[ledger::Answer {
                                question_id: "color".into(),
                                option_id: Some(option.into()),
                                text: None,
                            }],
                            chrono::Utc::now().timestamp(),
                        ))
                        .unwrap();
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
            seen
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

fn run(
    p: &sqlx::SqlitePool,
    task: i64,
    fixture: &Fixture,
    server: &Server,
    case: &str,
    answer: Option<&'static str>,
    cancel_on_question: bool,
) -> (Outcome, usize) {
    let lease = PreviewMcpLease::issue_for(
        &server.tokens,
        task,
        Vendor::Claude,
        &server.endpoint,
        &fixture.dir,
        true,
    )
    .unwrap();
    let execution = db(ledger::begin(p, task, chrono::Utc::now().timestamp())).unwrap();
    // 중단은 등록된 Control을 거쳐 들어온다 — 실제 경로와 같게 레지스트리에 올린다.
    let control = app_server::register_local(task, execution.clone()).unwrap();
    let ctx = Context {
        pool: p.clone(),
        task_id: task,
        control,
        changed: Arc::new(|| {}),
    };
    let watcher = Watcher::start(p, task, execution.clone(), answer, cancel_on_question);
    let mut events = Vec::new();
    let result = app_server::run_selected(
        Some(&ctx),
        fixture.dir.to_str().unwrap(),
        case,
        None,
        30,
        Vendor::Claude,
        &fixture.bin,
        None,
        None,
        None,
        &[],
        None,
        Some(&lease),
        |_| {},
        |event| events.push(event),
    );
    let seen = watcher.finish();
    app_server::unregister(task);
    drop(lease);
    (
        Outcome {
            result,
            events,
            execution,
        },
        seen,
    )
}

fn final_text(events: &[ConvoEvent]) -> String {
    events
        .iter()
        .find_map(|event| match event {
            ConvoEvent::Result { text, .. } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

#[test]
fn answer_returns_to_the_same_turn_and_closes_the_receipt() {
    let fixture = Fixture::new();
    let server = Server::start();
    let p = pool(11);
    let (out, seen) = run(&p, 11, &fixture, &server, "normal", Some("blue"), false);
    assert!(seen > 0, "question never reached the ledger");

    assert!(out.result.is_ok(), "{:?}", out.result);
    let text = final_text(&out.events);
    assert!(text.starts_with("answered:"), "{text}");
    assert!(text.contains("\"option_id\":\"blue\""), "{text}");
    // 프로세스를 새로 띄우지 않았다 — 질문 전후가 한 세션이다.
    assert_eq!(out.result.unwrap().session_id, "local-session-1");

    let snapshot = db(ledger::snapshot(&p, 11)).unwrap();
    let item = snapshot.items.last().unwrap();
    assert_eq!(item.state, "closed");
    assert_eq!(item.reason.as_deref(), Some("answered"));
    assert_eq!(item.receipt.as_ref().unwrap().state, "acknowledged");
    // 세션 id가 늦게 와도 실행 행에 붙는다.
    let thread: Option<String> = db(sqlx::query_scalar(
        "SELECT thread_id FROM convo_executions WHERE id=?",
    )
    .bind(&out.execution)
    .fetch_one(&p))
    .unwrap();
    assert_eq!(thread.as_deref(), Some("local-session-1"));
}

#[test]
fn an_unanswered_question_fails_the_turn_without_hanging_teardown() {
    let fixture = Fixture::new();
    let server = Server::start();
    let p = pool(12);
    let started = std::time::Instant::now();
    let (out, _) = run(&p, 12, &fixture, &server, "abandon", None, false);

    assert_eq!(
        out.result.as_ref().err().map(String::as_str),
        Some("응답되지 않은 질문이 남은 채 실행이 종료되었습니다")
    );
    // 닫기가 배수보다 먼저라 대기 중인 호출이 스스로 풀린다 — 배수 타임아웃(5초)을 쓰지 않는다.
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(server.tokens.active_for_task(12), 0);
    let snapshot = db(ledger::snapshot(&p, 12)).unwrap();
    assert_eq!(
        snapshot.items.last().unwrap().reason.as_deref(),
        Some("turn_ended")
    );
    assert!(!app_server::cleanup_failed(12));
}

#[test]
fn cancelling_kills_the_process_group_and_frees_the_waiter() {
    let fixture = Fixture::new();
    let server = Server::start();
    let p = pool(13);
    let (out, seen) = run(&p, 13, &fixture, &server, "normal", None, true);

    assert!(seen > 0, "question never reached the ledger");
    // 플래그만으로는 이 런타임을 멈출 수 없다 — 프로세스 그룹이 실제로 죽어야 한다.
    assert_eq!(
        out.result
            .as_ref()
            .map(|turn| turn.exit_desc.clone())
            .ok()
            .as_deref(),
        Some("signal 9"),
        "{:?}",
        out.result
    );
    assert_eq!(server.tokens.active_for_task(13), 0);
    let snapshot = db(ledger::snapshot(&p, 13)).unwrap();
    assert_eq!(
        snapshot.items.last().unwrap().reason.as_deref(),
        Some("cancelled")
    );
}

#[test]
fn ask_user_is_listed_only_while_a_question_session_runs() {
    let fixture = Fixture::new();
    let server = Server::start();
    let p = pool(14);
    let (out, _) = run(&p, 14, &fixture, &server, "listing", None, false);
    assert!(
        final_text(&out.events).contains("ask_user"),
        "{:?}",
        out.events
    );

    // 질문 세션이 아닌 평범한 턴에는 같은 서버·같은 토큰이라도 툴이 없다.
    let lease = PreviewMcpLease::issue(
        &server.tokens,
        14,
        Vendor::Claude,
        &server.endpoint,
        &fixture.dir,
    )
    .unwrap();
    let mut events = Vec::new();
    let plain = app_server::run_selected(
        None,
        fixture.dir.to_str().unwrap(),
        "listing",
        None,
        30,
        Vendor::Claude,
        &fixture.bin,
        None,
        None,
        None,
        &[],
        None,
        Some(&lease),
        |_| {},
        |event| events.push(event),
    );
    assert!(plain.is_ok(), "{plain:?}");
    let text = final_text(&events);
    assert!(text.starts_with("tools="), "{text}");
    assert!(!text.contains("ask_user"), "{text}");
}

#[test]
fn calling_ask_user_without_a_session_is_a_tool_error() {
    let fixture = Fixture::new();
    let server = Server::start();
    // 세션 등록은 턴 안에서만 산다. 등록 없이 부르면 거절이다.
    assert!(!praxis_lib::convo::question_local::active(15));
    let answer = db(praxis_lib::convo::question_local::ask(
        15,
        &serde_json::json!({}),
    ));
    assert_eq!(
        answer.err().as_deref(),
        Some("이 작업에는 열린 질문 세션이 없습니다")
    );
    drop(fixture);
    drop(server);
}

#[test]
fn the_mcp_descriptor_reuses_the_dynamic_tool_input_schema() {
    let spec = ledger::tool_spec();
    let mcp_spec = ledger::mcp_tool_spec();
    assert_eq!(mcp_spec["name"], ledger::TOOL_NAME);
    assert_eq!(mcp_spec["inputSchema"], spec["tools"][0]["inputSchema"]);
    // 런타임마다 계약 해시가 달라야 한다 — 같으면 툴 표면이 바뀌어도 바인딩이 통과한다.
    let p = pool(16);
    assert_eq!(
        db(ledger::runtime_of(&p, 16)).unwrap().as_deref(),
        Some(ledger::RUNTIME_LOCAL)
    );
}

#[test]
fn a_codex_binding_is_not_accepted_as_a_local_runtime() {
    let p = db(async {
        let p = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE tasks(id INTEGER PRIMARY KEY,convo_session_id TEXT,pending_capsule TEXT);INSERT INTO tasks(id) VALUES(1)").execute(&p).await.unwrap();
        ledger::migrate(&p).await.unwrap();
        ledger::bind(&p, 1).await.unwrap();
        p
    });
    assert_eq!(
        db(ledger::runtime_of(&p, 1)).unwrap().as_deref(),
        Some(ledger::RUNTIME)
    );
    assert_eq!(
        ledger::runtime_for_agent("claude"),
        Some(ledger::RUNTIME_LOCAL)
    );
    assert_eq!(ledger::runtime_for_agent("gemini"), None);
    let _ = Ordering::SeqCst;
}

/// 대역이 답할 수 없는 물음 하나 — **진짜 claude가 이 툴을 보고, 부르고, 답을 기다리는가.**
/// 기다림의 상한은 `MCP_TOOL_TIMEOUT`이 정하는데 그 값은 오프라인에서 확인할 수 없다. 그래서
/// 답을 **기본 상한(90초)보다 늦게** 보낸다 — 그 답이 턴으로 돌아오면 상한이 실제로 올라간 것이고,
/// 90초 언저리에서 끊기면 CLI가 그 변수를 보지 않거나 더 짧은 HTTP 상한이 따로 있다는 뜻이다.
#[test]
#[ignore = "Explicitly run scripts/probe-question-runtime-claude.py with a locally authenticated Claude CLI"]
fn live_claude_blocks_on_ask_user_past_the_default_tool_timeout() {
    let bin =
        std::env::var("PRAXIS_QUESTION_LIVE_CLAUDE").expect("explicit live probe runner required");
    let evidence = std::env::var("PRAXIS_QUESTION_LIVE_EVIDENCE").expect("evidence path required");
    let delay: u64 = std::env::var("PRAXIS_QUESTION_LIVE_DELAY_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);
    assert!(
        delay > 90,
        "지연이 기본 상한을 넘지 않으면 이 탐침은 아무것도 증명하지 못한다"
    );
    let task = 91;
    let fixture = Fixture::new();
    let server = Server::start();
    let p = pool(task);
    let lease = PreviewMcpLease::issue_for(
        &server.tokens,
        task,
        Vendor::Claude,
        &server.endpoint,
        &fixture.dir,
        true,
    )
    .unwrap();
    let token = lease
        .injection()
        .env
        .iter()
        .find(|(key, _)| key == "PRAXIS_PREVIEW_TOKEN")
        .unwrap()
        .1
        .clone();
    let timeout_env = lease
        .injection()
        .env
        .iter()
        .find(|(key, _)| key == "MCP_TOOL_TIMEOUT")
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    let execution = db(ledger::begin(&p, task, chrono::Utc::now().timestamp())).unwrap();
    let control = app_server::register_local(task, execution.clone()).unwrap();
    let ctx = Context {
        pool: p.clone(),
        task_id: task,
        control,
        changed: Arc::new(|| {}),
    };

    // 질문을 보면 `delay`만큼 재우고 답한다. 대기가 끊기면 툴이 먼저 실패해 답이 갈 곳을 잃는다.
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = stop.clone();
    let pool_for_watcher = p.clone();
    let execution_for_watcher = execution.clone();
    let watcher = std::thread::spawn(move || {
        let mut waited: Option<f64> = None;
        let mut handled = std::collections::HashSet::new();
        while !flag.load(Ordering::SeqCst) {
            let snapshot = db(ledger::snapshot(&pool_for_watcher, task)).unwrap();
            for item in snapshot.items.iter().filter(|i| i.state == "pending") {
                if !handled.insert(item.id.clone()) {
                    continue;
                }
                let opened = std::time::Instant::now();
                std::thread::sleep(std::time::Duration::from_secs(delay));
                let answers = item
                    .questions
                    .questions
                    .iter()
                    .map(|q| ledger::Answer {
                        question_id: q.id.clone(),
                        option_id: q.options.first().map(|o| o.id.clone()),
                        text: q.options.is_empty().then(|| "blue".into()),
                    })
                    .collect::<Vec<_>>();
                db(ledger::submit(
                    &pool_for_watcher,
                    task,
                    &execution_for_watcher,
                    &item.id,
                    &ledger::id().unwrap(),
                    &answers,
                    chrono::Utc::now().timestamp(),
                ))
                .unwrap();
                waited = Some(opened.elapsed().as_secs_f64());
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        waited
    });

    let mut events = Vec::new();
    let started = std::time::Instant::now();
    let result = app_server::run_selected(
        Some(&ctx),
        fixture.dir.to_str().unwrap(),
        "This is a bounded protocol integration test in a disposable directory. Do not run commands, read or write files, access the network, credentials, or other tools. Call mcp__praxis-preview__ask_user exactly once with kind clarification and one question whose id is color, question \"Which color?\", options [{id:blue,label:Blue,description:Probe choice}], allow_free_text true and is_secret false. It may take several minutes to answer; wait for it. Then reply with a single short line containing the received option id.",
        None,
        delay + 300,
        Vendor::Claude,
        &bin,
        None,
        None,
        None,
        &[],
        None,
        Some(&lease),
        |_| {},
        |event| events.push(event),
    );
    let elapsed = started.elapsed().as_secs_f64();
    stop.store(true, Ordering::SeqCst);
    let waited = watcher.join().unwrap();
    app_server::unregister(task);

    let text = final_text(&events);
    assert!(result.is_ok(), "live turn failed: {result:?} / {text}");
    let waited = waited.expect("ask_user never reached the ledger");
    assert!(
        waited >= delay as f64,
        "watcher answered too early: {waited}s"
    );
    assert!(
        text.to_ascii_lowercase().contains("blue"),
        "answer did not return into the same turn: {text}"
    );
    let snapshot = db(ledger::snapshot(&p, task)).unwrap();
    let item = snapshot.items.last().unwrap();
    assert_eq!(item.state, "closed");
    assert_eq!(item.reason.as_deref(), Some("answered"));
    assert_eq!(item.receipt.as_ref().unwrap().state, "acknowledged");
    drop(lease);
    assert!(server.tokens.task_for(&token).is_none());
    assert_eq!(server.tokens.active_for_task(task), 0);

    std::fs::write(
        evidence,
        serde_json::to_string_pretty(&serde_json::json!({
            "observed_at": chrono::Utc::now().to_rfc3339(),
            "runtime": ledger::RUNTIME_LOCAL,
            "tool": format!("mcp__praxis-preview__{}", ledger::TOOL_NAME),
            "mcp_tool_timeout_env_ms": timeout_env,
            "held_open_secs": waited,
            "turn_secs": elapsed,
            "answer_returned_to_same_turn": true,
            "receipt": "acknowledged",
            "old_token_revoked": true,
            "scope": "Disposable cwd; production adapter and in-app MCP server; browser dispatcher is never reached"
        }))
        .unwrap()
            + "\n",
    )
    .unwrap();
}
