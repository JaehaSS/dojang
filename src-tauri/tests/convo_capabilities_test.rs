#![cfg(unix)]

use praxis_lib::convo::{
    app_server::{self, Context, Control},
    interaction as ledger, ConvoEvent, Vendor,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
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
        sqlx::query("CREATE TABLE tasks(id INTEGER PRIMARY KEY,convo_session_id TEXT,pending_capsule TEXT);INSERT INTO tasks(id) VALUES(1)")
            .execute(&p)
            .await
            .unwrap();
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
        let dir =
            std::env::temp_dir().join(format!("praxis-capabilities-{}", ledger::id().unwrap()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("provider");
        std::fs::write(&path, include_str!("fixtures/capability_provider.py")).unwrap();
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

#[derive(Default)]
struct Seen {
    events: Vec<ConvoEvent>,
    pids: Vec<u32>,
}

fn answers(item: &ledger::Interaction, case: &str) -> Vec<ledger::Answer> {
    item.questions
        .questions
        .iter()
        .map(|q| {
            let option = match (case, q.id.as_str()) {
                ("native-question", "colour") => "option:0:1",
                ("async-message", "question-0") => "option:0:0",
                (case, "decision") if case.starts_with("approval-") => {
                    case.strip_prefix("approval-").unwrap()
                }
                ("permission" | "file", "decision") => "accept",
                ("mcp-form", "enabled") => "true",
                ("mcp-form", "style") => "enum:1",
                ("mcp-form", "__dojang_action") => "accept",
                _ => panic!("unexpected question {case}: {}", q.id),
            };
            ledger::Answer {
                question_id: q.id.clone(),
                option_id: Some(option.into()),
                text: None,
            }
        })
        .collect()
}

fn run(
    p: &sqlx::SqlitePool,
    fixture: &Fixture,
    case: &str,
    resume: Option<&str>,
    submit: bool,
) -> (Result<praxis_lib::convo::TurnOutcome, String>, Seen) {
    let execution = db(ledger::begin(p, 1, chrono::Utc::now().timestamp())).unwrap();
    let ctx = Context {
        pool: p.clone(),
        task_id: 1,
        control: Arc::new(Control::new(execution, false)),
        changed: Arc::new(|| {}),
    };
    let mut seen = Seen::default();
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
        None,
        &[],
        None,
        None,
        |pid| seen.pids.push(pid),
        |event| {
            if let ConvoEvent::Interaction { interaction_id } = &event {
                let snapshot = db(ledger::snapshot(p, 1)).unwrap();
                let item = snapshot
                    .items
                    .iter()
                    .find(|item| item.id == *interaction_id)
                    .unwrap();
                if submit {
                    db(ledger::submit(
                        p,
                        1,
                        &ctx.control.execution,
                        interaction_id,
                        &ledger::id().unwrap(),
                        &answers(item, case),
                        chrono::Utc::now().timestamp(),
                    ))
                    .unwrap();
                }
            }
            seen.events.push(event);
        },
    );
    assert!(
        !ctx.control.cleanup_failed.load(Ordering::SeqCst),
        "cleanup: {result:?}"
    );
    for pid in seen.pids.iter().copied() {
        assert_ne!(
            unsafe { nix::libc::kill(-(pid as i32), 0) },
            0,
            "provider group still alive"
        );
    }
    (result, seen)
}

fn successful(events: &[ConvoEvent]) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            ConvoEvent::Result {
                is_error: false,
                ..
            }
        )
    })
}

#[test]
fn native_async_questions_reuse_wire_ids_across_rounds_and_resolve() {
    let fixture = Fixture::new();
    let p = pool();
    for resume in [None, Some("capability-thread")] {
        let (result, seen) = run(&p, &fixture, "native-question", resume, true);
        assert!(result.is_ok(), "{result:?}");
        assert!(successful(&seen.events));
        let snapshot = db(ledger::snapshot(&p, 1)).unwrap();
        let item = snapshot.items.last().unwrap();
        assert!(!item.request.as_ref().unwrap().blocking);
        assert_eq!(item.state, "closed");
        assert_eq!(item.reason.as_deref(), Some("resolved"));
        assert_eq!(item.receipt.as_ref().unwrap().state, "written");
    }
}

#[test]
fn async_agent_message_answers_with_turn_steer_and_never_a_fabricated_rpc_reply() {
    let fixture = Fixture::new();
    let p = pool();
    let (result, seen) = run(&p, &fixture, "async-message", None, true);
    assert!(result.is_ok(), "{result:?}");
    assert!(successful(&seen.events));
    let item = db(ledger::snapshot(&p, 1)).unwrap().items.pop().unwrap();
    assert!(!item.request.as_ref().unwrap().blocking);
    assert_eq!(item.state, "closed");
    assert_eq!(item.reason.as_deref(), Some("resolved"));
    assert_eq!(item.receipt.as_ref().unwrap().state, "written");
}

#[test]
fn native_approvals_and_mcp_forms_return_exact_protocol_results() {
    let fixture = Fixture::new();
    let p = pool();
    for case in [
        "approval-accept",
        "approval-decline",
        "approval-cancel",
        "file",
        "permission",
        "mcp-form",
    ] {
        let (result, seen) = run(&p, &fixture, case, None, true);
        assert!(result.is_ok(), "{case}: {result:?}");
        assert!(successful(&seen.events), "{case}");
    }
}

#[test]
fn foreign_native_request_fails_closed_without_an_interaction() {
    let fixture = Fixture::new();
    let p = pool();
    let (result, seen) = run(&p, &fixture, "wrong-owner", None, false);
    assert!(result.is_err(), "{result:?}");
    assert!(!successful(&seen.events));
    assert!(db(ledger::snapshot(&p, 1)).unwrap().items.is_empty());
}

#[test]
fn unanswered_nonblocking_native_input_does_not_make_completed_turn_fail() {
    let fixture = Fixture::new();
    let p = pool();
    let (result, seen) = run(&p, &fixture, "native-unanswered", None, false);
    assert!(result.is_ok(), "{result:?}");
    assert!(successful(&seen.events));
    let item = db(ledger::snapshot(&p, 1)).unwrap().items.pop().unwrap();
    assert!(!item.request.as_ref().unwrap().blocking);
    assert_eq!(item.state, "closed");
}

#[test]
fn code_mode_raw_events_preserve_public_rich_output_only() {
    let fixture = Fixture::new();
    let (result, seen) = run(&pool(), &fixture, "raw-rich", None, false);
    assert!(result.is_ok(), "{result:?}");
    assert!(successful(&seen.events));
    assert!(seen.events.iter().any(
        |event| matches!(event, ConvoEvent::ToolUse { tool_id: Some(id), .. } if id == "raw-exec")
    ));
    assert!(seen.events.iter().any(|event| matches!(event, ConvoEvent::ToolResult { tool_use_id: Some(id), .. } if id == "raw-exec")));
    let serialized = serde_json::to_string(&seen.events).unwrap();
    assert!(serialized.contains("Script completed"));
    assert!(serialized.contains("image/png") && serialized.contains("audio/wav"));
    assert!(!serialized.contains("NEVER-SERIALIZE-RAW-REASONING"));
}

#[test]
fn yielded_code_cells_must_be_waited_and_unknown_raw_tools_fail_closed() {
    let fixture = Fixture::new();
    for case in ["raw-yielded", "unknown-raw-tool"] {
        let (result, seen) = run(&pool(), &fixture, case, None, false);
        assert!(result.is_err(), "{case}: {result:?}");
        assert!(!successful(&seen.events), "{case}");
    }
    let (result, seen) = run(&pool(), &fixture, "raw-wait", None, false);
    assert!(result.is_ok(), "{result:?}");
    assert!(successful(&seen.events));
}

#[test]
fn failed_or_error_code_wait_releases_yielded_cell_and_surfaces_an_error_result() {
    let fixture = Fixture::new();
    for case in ["raw-wait-failed", "raw-wait-error"] {
        let (result, seen) = run(&pool(), &fixture, case, None, false);
        assert!(result.is_ok(), "{case}: {result:?}");
        assert!(successful(&seen.events), "{case}");
        assert!(seen.events.iter().any(|event| matches!(
            event,
            ConvoEvent::ToolResult { tool_use_id: Some(id), is_error: true, .. } if id == "raw-wait"
        )), "{case}");
    }
}

#[test]
#[ignore = "set DOJANG_PROBE_PROMPT and explicitly run this live Codex app-server probe"]
fn live_codex_adapter_probe_records_only_summary_evidence() {
    let bin = std::env::var("DOJANG_CODEX_BIN").unwrap_or_else(|_| "codex".into());
    let prompt = std::env::var("DOJANG_PROBE_PROMPT")
        .expect("DOJANG_PROBE_PROMPT is required for the explicit live probe");
    let fixture = Fixture::new();
    let p = pool();
    let execution = db(ledger::begin(&p, 1, chrono::Utc::now().timestamp())).unwrap();
    let ctx = Context {
        pool: p.clone(),
        task_id: 1,
        control: Arc::new(Control::new(execution, false)),
        changed: Arc::new(|| {}),
    };
    let mut provider_pid = None;
    let mut native_requests_cancelled = 0usize;
    let mut tool_outputs = 0usize;
    let mut image_outputs = 0usize;
    let mut audio_outputs = 0usize;
    let mut text_outputs = 0usize;
    let mut answered_requests = 0usize;
    let mut tool_cancellations = 0usize;
    let probe_answer = std::env::var("DOJANG_PROBE_ANSWER")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let cancel_on_tool = std::env::var("DOJANG_PROBE_CANCEL_ON_TOOL").as_deref() == Ok("1");
    let mut final_result = None;
    let started = Instant::now();
    let watchdog_stop = Arc::new(AtomicBool::new(false));
    let stop = watchdog_stop.clone();
    let control = ctx.control.clone();
    let watchdog = std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            if started.elapsed() >= Duration::from_secs(125) {
                control.cancelled.store(true, Ordering::SeqCst);
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    let outcome = app_server::run_selected(
        Some(&ctx),
        fixture.dir.to_str().unwrap(),
        &prompt,
        None,
        120,
        Vendor::Codex,
        &bin,
        None,
        None,
        None,
        &[],
        None,
        None,
        |pid| provider_pid = Some(pid),
        |event| match event {
            ConvoEvent::Interaction { interaction_id } => {
                let snapshot = db(ledger::snapshot(&p, 1)).unwrap();
                let item = snapshot
                    .items
                    .iter()
                    .find(|item| item.id == interaction_id)
                    .unwrap();
                let safe_answers = probe_answer.as_ref().and_then(|answer| {
                    (item.request.as_ref()?.kind
                        == praxis_lib::convo::native_interaction::NativeRequestKind::Question)
                        .then(|| {
                            item.questions
                                .questions
                                .iter()
                                .map(|question| {
                                    if question.allow_free_text {
                                        Some(ledger::Answer {
                                            question_id: question.id.clone(),
                                            option_id: None,
                                            text: Some(answer.clone()),
                                        })
                                    } else {
                                        question
                                            .options
                                            .iter()
                                            .find(|option| option.label == *answer)
                                            .map(|option| ledger::Answer {
                                                question_id: question.id.clone(),
                                                option_id: Some(option.id.clone()),
                                                text: None,
                                            })
                                    }
                                })
                                .collect::<Option<Vec<_>>>()
                        })?
                });
                if let Some(answers) = safe_answers {
                    if db(ledger::submit(
                        &p,
                        1,
                        &ctx.control.execution,
                        &interaction_id,
                        &ledger::id().unwrap(),
                        &answers,
                        chrono::Utc::now().timestamp(),
                    ))
                    .is_ok()
                    {
                        answered_requests += 1;
                    } else {
                        native_requests_cancelled += 1;
                        ctx.control.cancelled.store(true, Ordering::SeqCst);
                    }
                } else {
                    native_requests_cancelled += 1;
                    ctx.control.cancelled.store(true, Ordering::SeqCst);
                }
            }
            ConvoEvent::ToolUse { .. }
                if cancel_on_tool && !ctx.control.cancelled.swap(true, Ordering::SeqCst) =>
            {
                tool_cancellations += 1;
            }
            ConvoEvent::ToolOutput { contents, .. } => {
                tool_outputs += 1;
                for content in serde_json::to_value(contents).unwrap().as_array().unwrap() {
                    match content["type"].as_str() {
                        Some("image") => image_outputs += 1,
                        Some("audio") => audio_outputs += 1,
                        Some("text") => text_outputs += 1,
                        _ => {}
                    }
                }
            }
            ConvoEvent::Result { is_error, .. } => final_result = Some(is_error),
            _ => {}
        },
    );
    watchdog_stop.store(true, Ordering::SeqCst);
    watchdog.join().unwrap();
    let provider_reaped =
        provider_pid.is_some_and(|pid| unsafe { nix::libc::kill(-(pid as i32), 0) } != 0);
    let evidence = serde_json::json!({
        "adapter": "convo/app_server.rs",
        "cli": "codex app-server",
        "outcome_ok": outcome.is_ok(),
        "outcome_error": outcome.as_ref().err().map(|error| error.chars().take(240).collect::<String>()),
        "final_result_is_error": final_result,
        "native_requests_cancelled": native_requests_cancelled,
        "answered_requests": answered_requests,
        "cancel_on_tool": cancel_on_tool,
        "tool_cancellations": tool_cancellations,
        "tool_output_events": tool_outputs,
        "image_output_items": image_outputs,
        "audio_output_items": audio_outputs,
        "text_output_items": text_outputs,
        "provider_reaped": provider_reaped,
    });
    if let Ok(path) = std::env::var("DOJANG_PROBE_EVIDENCE") {
        std::fs::write(
            path,
            serde_json::to_string_pretty(&evidence).unwrap() + "\n",
        )
        .unwrap();
    }
    assert!(
        started.elapsed().as_secs() <= 130,
        "live probe exceeded bounded adapter timeout"
    );
    assert!(
        !ctx.control.cleanup_failed.load(Ordering::SeqCst),
        "cleanup: {outcome:?}"
    );
    assert!(provider_reaped, "live provider group still alive");
    if cancel_on_tool {
        assert_eq!(tool_cancellations, 1, "probe did not interrupt a tool");
        assert_ne!(
            final_result,
            Some(false),
            "tool cancellation reported normal success"
        );
    } else {
        assert!(outcome.is_ok(), "live adapter failed: {outcome:?}");
        assert_eq!(
            final_result,
            Some(false),
            "live adapter did not complete normally"
        );
    }
    if std::env::var("DOJANG_REQUIRE_IMAGE").as_deref() == Ok("1") {
        assert!(image_outputs > 0, "live probe did not emit an image output");
    }
    if std::env::var("DOJANG_REQUIRE_AUDIO").as_deref() == Ok("1") {
        assert!(audio_outputs > 0, "live probe did not emit an audio output");
    }
    if std::env::var("DOJANG_REQUIRE_ANSWER").as_deref() == Ok("1") {
        assert!(
            answered_requests > 0,
            "live probe did not safely answer an ordinary question"
        );
    }
}
