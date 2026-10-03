#![cfg(unix)]
use praxis_lib::convo::{
    app_server::{self, Context, Control},
    interaction as ledger, ConvoEvent, Vendor,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::Ordering,
        Arc,
    },
};
fn db<T>(f: impl std::future::Future<Output = T>) -> T {
    tauri::async_runtime::block_on(f)
}
fn pool() -> sqlx::SqlitePool {
    db(async {
        let p = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE tasks(id INTEGER PRIMARY KEY,convo_session_id TEXT,pending_capsule TEXT);INSERT INTO tasks(id) VALUES(1)").execute(&p).await.unwrap();
        ledger::migrate(&p).await.unwrap();
        ledger::bind(&p, 1).await.unwrap();
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
        let dir = std::env::temp_dir().join(format!("praxis-question-{}", ledger::id().unwrap()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("provider");
        std::fs::write(&path, include_str!("fixtures/question_provider.py")).unwrap();
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
fn run(
    p: &sqlx::SqlitePool,
    fixture: &Fixture,
    case: &str,
    resume: Option<&str>,
) -> (
    Result<praxis_lib::convo::TurnOutcome, String>,
    Vec<ConvoEvent>,
) {
    run_with_speed(p, fixture, case, resume, None)
}
fn run_with_speed(
    p: &sqlx::SqlitePool, fixture: &Fixture, case: &str, resume: Option<&str>, service_tier: Option<&str>,
) -> (Result<praxis_lib::convo::TurnOutcome, String>, Vec<ConvoEvent>) {
    let execution = db(ledger::begin(p, 1, chrono::Utc::now().timestamp())).unwrap();
    let ctx = Context {
        pool: p.clone(),
        task_id: 1,
        control: Arc::new(Control::new(execution, false)),
        changed: Arc::new(|| {}),
    };
    let mut events = Vec::new();
    let mut pid = 0;
    let result = app_server::run_selected(
        Some(&ctx),
        fixture.dir.to_str().unwrap(),
        case,
        resume,
        5,
        Vendor::Codex,
        &fixture.bin,
        None,
        None,
        service_tier,
        &[],
        None,
        None,
        |id| pid = id,
        |event| {
            if let ConvoEvent::Interaction { interaction_id } = &event {
                if case == "cancel" {
                    ctx.control.cancelled.store(true, Ordering::SeqCst);
                } else if case == "expired" {
                    db(
                        sqlx::query("UPDATE convo_interactions SET expires_at=0 WHERE id=?")
                            .bind(interaction_id)
                            .execute(p),
                    )
                    .unwrap();
                } else if case != "eof" {
                    db(ledger::submit(
                        p,
                        1,
                        &ctx.control.execution,
                        interaction_id,
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
            }
            events.push(event);
        },
    );
    assert!(
        !ctx.control.cleanup_failed.load(Ordering::SeqCst),
        "cleanup: {result:?}"
    );
    assert!(pid != 0, "provider did not start: {result:?}");
    assert!(
        unsafe { nix::libc::kill(-(pid as i32), 0) } != 0,
        "provider group {pid} still alive: {result:?}"
    );
    (result, events)
}
#[test]
fn same_call_receives_answer_and_new_process_resumes_same_thread() {
    let fixture = Fixture::new();
    let p = pool();
    for resume in [None, Some("test-thread")] {
        let (result, events) = run(&p, &fixture, "normal", resume);
        assert_eq!(result.unwrap().session_id, "test-thread");
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, ConvoEvent::TextUpdate { complete: true, .. }))
                .count(),
            1
        );
        assert!(events.iter().any(|e| matches!(
            e,
            ConvoEvent::Result {
                is_error: false,
                ..
            }
        )));
        assert_eq!(
            db(ledger::snapshot(&p, 1))
                .unwrap()
                .items
                .last()
                .unwrap()
                .receipt
                .as_ref()
                .unwrap()
                .state,
            "acknowledged"
        );
    }
}
#[test]
fn reversed_request_order_is_correlated() {
    let f = Fixture::new();
    let p = pool();
    assert!(run(&p, &f, "reverse", None).0.is_ok());
}
#[test]
fn unsupported_secret_and_foreign_requests_never_reach_ui_or_ledger() {
    for case in ["secret", "wrong-owner", "native", "unknown-tool"] {
        let f = Fixture::new();
        let p = pool();
        let (result, events) = run(&p, &f, case, None);
        assert!(result.is_err(), "{case}");
        assert!(
            db(ledger::snapshot(&p, 1)).unwrap().items.is_empty(),
            "{case}"
        );
        assert!(!serde_json::to_string(&events)
            .unwrap()
            .contains("NEVER-PERSIST-SECRET"));
    }
}
#[test]
fn cancellation_expiry_and_eof_close_questions_without_resubmission() {
    for case in ["cancel", "expired", "eof"] {
        let f = Fixture::new();
        let p = pool();
        let (result, events) = run(&p, &f, case, None);
        assert!(
            result.is_err()
                || !events.iter().any(|e| matches!(
                    e,
                    ConvoEvent::Result {
                        is_error: false,
                        ..
                    }
                )),
            "{case}"
        );
        let snapshot = db(ledger::snapshot(&p, 1)).unwrap();
        assert_eq!(snapshot.phase, "idle");
        assert!(snapshot.items.iter().all(|q| q.state == "closed"));
    }
}
#[test]
fn unfinished_commands_and_surviving_children_cannot_report_success() {
    for case in ["pending-command", "survivor", "spoofed-helper"] {
        let f = Fixture::new();
        let p = pool();
        let (result, events) = run(&p, &f, case, None);
        assert!(result.is_err(), "{case}: {result:?}");
        assert!(!events.iter().any(|e| matches!(
            e,
            ConvoEvent::Result {
                is_error: false,
                ..
            }
        )));
    }
}
#[test]
fn invalid_version_never_spawns_a_turn_and_releases_execution() {
    let f = Fixture::new();
    let p = pool();
    let source = std::fs::read_to_string(&f.bin)
        .unwrap()
        .replace("codex-cli 0.154.0", "codex-cli 9.0.0");
    std::fs::write(&f.bin, source).unwrap();
    let e = db(ledger::begin(&p, 1, 0)).unwrap();
    let ctx = Context {
        pool: p.clone(),
        task_id: 1,
        control: Arc::new(Control::new(e, false)),
        changed: Arc::new(|| {}),
    };
    let result = app_server::run_selected(
        Some(&ctx),
        f.dir.to_str().unwrap(),
        "normal",
        None,
        5,
        Vendor::Codex,
        &f.bin,
        None,
        None,
        None,
        &[],
        None,
        None,
        |_| panic!("spawned turn"),
        |_| panic!("emitted event"),
    );
    assert!(result.is_err());
    assert_eq!(db(ledger::snapshot(&p, 1)).unwrap().phase, "idle");
}

struct LiveDispatch(std::sync::atomic::AtomicUsize);
#[async_trait::async_trait]
impl praxis_lib::preview_bridge::mcp::Dispatcher for LiveDispatch {
    async fn dispatch(
        &self,
        task: i64,
        cmd: praxis_lib::preview_bridge::mcp::Command,
    ) -> Result<String, praxis_lib::preview_bridge::mcp::DispatchError> {
        assert_eq!(task, 1);
        assert_eq!(cmd, praxis_lib::preview_bridge::mcp::Command::Snapshot);
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(r#"{"ok":true,"snapshot":{"generation":1,"text":"Praxis protocol probe only","truncated":false,"url":"http://localhost/"}}"#.into())
    }
}
struct LiveServer(Option<tauri::async_runtime::JoinHandle<()>>);
impl Drop for LiveServer {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
            let _ = db(handle);
        }
    }
}

#[test]
#[ignore = "Explicitly run scripts/probe-question-runtime.py with a locally authenticated Codex 0.154.0"]
fn live_codex_question_resume_and_fresh_mcp_lease() {
    use praxis_lib::preview_bridge::mcp::{self, ControlTokens, PreviewMcpLease};
    let bin =
        std::env::var("PRAXIS_QUESTION_LIVE_CODEX").expect("explicit live probe runner required");
    let evidence = std::env::var("PRAXIS_QUESTION_LIVE_EVIDENCE").expect("evidence path required");
    let fixture = Fixture::new();
    let p = pool();
    let tokens = ControlTokens::default();
    let dispatcher = Arc::new(LiveDispatch(Default::default()));
    let (port, listener) = db(mcp::bind()).unwrap();
    let endpoint = mcp::inject::endpoint_url(port, "question-probe");
    let state = Arc::new(mcp::McpState {
        instance: "question-probe".into(),
        tokens: tokens.clone(),
        dispatcher: dispatcher.clone(),
        tools: mcp::Tools::phase_f(),
    });
    let server = LiveServer(Some(tauri::async_runtime::spawn(mcp::serve_on(
        listener, state,
    ))));
    let mut session = None;
    let mut rounds = Vec::new();
    for round in 1..=2 {
        let lease =
            PreviewMcpLease::issue(&tokens, 1, Vendor::Codex, &endpoint, &fixture.dir).unwrap();
        let token = lease
            .injection()
            .env
            .iter()
            .find(|(key, _)| key == "PRAXIS_PREVIEW_TOKEN")
            .unwrap()
            .1
            .clone();
        let execution = db(ledger::begin(&p, 1, chrono::Utc::now().timestamp())).unwrap();
        let ctx = Context {
            pool: p.clone(),
            task_id: 1,
            control: Arc::new(Control::new(execution, false)),
            changed: Arc::new(|| {}),
        };
        let mut diagnostics = Vec::new();
        let mut question_seen = false;
        let mut result_seen = false;
        let mut answer_echoed = false;
        let mut provider_pid = 0;
        let previous = dispatcher.0.load(Ordering::SeqCst);
        let output=app_server::run_selected(Some(&ctx),fixture.dir.to_str().unwrap(),"This is a bounded protocol integration test in a disposable directory. Do not run commands, access files, network sites, credentials, other connectors, or agents. First call only praxis_preview's browser_snapshot with empty arguments (it is a harmless in-memory test dispatcher). Then call praxis_ui.ask_user with kind clarification and one ordinary question whose id is color, options [{id:blue,label:Blue,description:Probe choice}], allow_free_text true and is_secret false. Wait for the tool answer. Then produce a short Markdown final message containing the received option id. Do this again even if an earlier probe round already did it.",session.as_deref(),120,Vendor::Codex,&bin,None,Some("low"),None,&[],None,Some(&lease),|pid|provider_pid=pid,|event|{
            match &event {ConvoEvent::ToolUse{name,summary,..}=>diagnostics.push(format!("tool {name}: {summary}")),ConvoEvent::Result{text,is_error,..}=>diagnostics.push(format!("result({is_error}): {text}")),_=>{}}
            if let ConvoEvent::Interaction{interaction_id}=event {
                question_seen=true;
                let snapshot=db(ledger::snapshot(&p,1)).unwrap();let question=snapshot.items.iter().find(|q|q.id==interaction_id).unwrap();
                let answers=question.questions.questions.iter().map(|q|ledger::Answer{question_id:q.id.clone(),option_id:q.options.first().map(|o|o.id.clone()),text:q.options.is_empty().then(||"blue".into())}).collect::<Vec<_>>();
                db(ledger::submit(&p,1,&ctx.control.execution,&interaction_id,&ledger::id().unwrap(),&answers,chrono::Utc::now().timestamp())).unwrap();
            }else if let ConvoEvent::Result{is_error:false,text,..}=event{result_seen=true;answer_echoed=text.to_ascii_lowercase().contains("blue");}
        }).expect("live adapter round failed");
        assert!(
            question_seen && result_seen && answer_echoed,
            "round={round}, question={question_seen}, result={result_seen}, echoed={answer_echoed}"
        );
        assert!(session.as_ref().is_none_or(|s| s == &output.session_id));
        session = Some(output.session_id);
        let snapshot = db(ledger::snapshot(&p, 1)).unwrap();
        assert_eq!(
            snapshot
                .items
                .last()
                .unwrap()
                .receipt
                .as_ref()
                .unwrap()
                .state,
            "acknowledged"
        );
        assert!(
            dispatcher.0.load(Ordering::SeqCst) > previous,
            "MCP snapshot was not dispatched: {diagnostics:?}"
        );
        assert!(tokens.task_for(&token).is_none());
        assert_eq!(tokens.active_for_task(1), 0);
        assert!(unsafe { nix::libc::kill(-(provider_pid as i32), 0) } != 0);
        rounds.push(serde_json::json!({"round":round,"resumed":round>1,"same_thread":true,"question_received":true,"answer_acknowledged":true,"answer_echoed_in_final":true,"mcp_dispatched_to_task":1,"old_token_revoked":true,"inflight_dispatches":0,"provider_reaped":true}));
        drop(lease);
    }
    drop(server);
    let record = serde_json::json!({"observed_at":chrono::Utc::now().to_rfc3339(),"cli_version":app_server::SUPPORTED_VERSION,"adapter":"convo/app_server.rs","tool_schema":"convo/interaction-tool.json","rounds":rounds,"local_mcp_server_stopped":true,"success":true,"scope":"Disposable cwd; production adapter and full question schema; MCP dispatcher is an in-memory snapshot fixture, not a real browser"});
    std::fs::write(
        evidence,
        serde_json::to_string_pretty(&record).unwrap() + "\n",
    )
    .unwrap();
}

#[test]
fn service_tier_reaches_new_and_resumed_structured_turns() {
    let fixture = Fixture::new();
    let p = pool();
    for tier in ["fast", "default"] {
        for resume in [None, Some("test-thread")] {
            let (result, events) = run_with_speed(&p, &fixture, "normal", resume, Some(tier));
            assert!(result.is_ok(), "{tier}: {result:?}");
            assert!(events.iter().any(|event| matches!(event, ConvoEvent::Result {is_error:false,..})));
        }
    }
}

#[test]
fn image_event_over_old_limit_completes_but_oversized_event_fails_and_reaps() {
    let fixture = Fixture::new();
    let (result, events) = run(&pool(), &fixture, "large-event", Some("test-thread"));
    assert!(result.is_ok(), "{result:?}");
    assert!(events.iter().any(|event| matches!(event, ConvoEvent::Result { is_error: false, .. })));
    let (result, events) = run(&pool(), &fixture, "oversized-event", Some("test-thread"));
    assert!(result.unwrap_err().contains("이벤트 크기 제한 초과 (16 MiB)"));
    assert!(!events.iter().any(|event| matches!(event, ConvoEvent::Result { is_error: false, .. })));
}
