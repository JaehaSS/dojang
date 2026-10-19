#![cfg(unix)]

#[path = "support/temp_root.rs"]
mod temp_root;

use praxis_lib::convo::{run_turn_with_effort, ConvoEvent, TurnOutcome, Vendor};
use praxis_lib::preview_bridge::mcp::McpInjection;
use serde_json::Value;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const ROOT: &str = "10000000-0000-0000-0000-000000000001";
const CHILD: &str = "20000000-0000-0000-0000-000000000002";
const NESTED: &str = "30000000-0000-0000-0000-000000000003";

struct Stub(PathBuf);

impl Stub {
    fn new(scenario: &str) -> Self {
        let directory = temp_root::dir().join(format!("codex-observer-{scenario}"));
        std::fs::create_dir_all(directory.join("home/sessions/2026/10/07")).unwrap();
        let script = format!(
            r#"#!/usr/bin/python3
import datetime, json, os, pathlib, sys, time
root, child, nested = {root:?}, {child:?}, {nested:?}
scenario = {scenario:?}
protocol = json.loads({protocol:?})
directory = pathlib.Path(os.environ['CODEX_HOME']) / 'sessions/2026/10/07'
def stamp():
    return datetime.datetime.now(datetime.timezone.utc).isoformat().replace('+00:00', 'Z')
def append(target, kind, payload):
    with target.open('a') as f:
        f.write(json.dumps({{'timestamp':stamp(),'type':kind,'payload':payload}})+'\n')
def meta(cid, parent):
    p = directory / ('rollout-fixture-'+cid+'.jsonl')
    append(p, 'session_meta', {{'id':cid,'source':{{'subagent':{{'thread_spawn':{{'parent_thread_id':parent,'depth':1,'agent_path':'/root/check','agent_nickname':'Fixture'}}}}}}}})
    return p
def start(p, turn):
    append(p,'event_msg',{{'type':'task_started','turn_id':turn}})
    append(p,'turn_context',{{'turn_id':turn,'model':'fixture-child-model'}})
def complete(p, turn, text):
    append(p,'event_msg',{{'type':'task_complete','turn_id':turn,'last_agent_message':text}})
parent_log = directory / ('rollout-root-'+root+'.jsonl')
append(parent_log,'session_meta',{{'id':root,'source':'cli'}})
append(parent_log,'turn_context',{{'turn_id':'main-turn','model':'fixture-main-model'}})
append(parent_log,'event_msg',{{'type':'token_count','info':{{'last_token_usage':{{'total_tokens':123}},'model_context_window':100000}}}})
print(json.dumps({{'type':'thread.started','thread_id':root}}),flush=True)
if scenario == 'protocol':
    for rows in protocol['children']:
        cid = rows[0]['payload']['id']
        p = directory / ('rollout-protocol-'+cid+'.jsonl')
        for row in rows:
            append(p,row['type'],row['payload'])
elif scenario == 'empty-wait':
    print(json.dumps({{'type':'item.completed','item':{{'id':'wait-1','type':'collab_tool_call','tool':'wait','receiver_thread_ids':[],'agents_states':{{}},'status':'completed'}}}}),flush=True)
else:
    time.sleep(0.15)
    p=meta(child,root)
    start(p,'turn-one')
    if scenario == 'quiet-nested':
        # Matching cwd/name is intentionally insufficient for ownership.
        unrelated=meta('40000000-0000-0000-0000-000000000004','50000000-0000-0000-0000-000000000005')
        start(unrelated,'unrelated-turn')
        complete(unrelated,'unrelated-turn','UNRELATED')
        q=meta(nested,child)
        start(q,'nested-turn')
        complete(q,'nested-turn','NESTED_RESULT')
    if scenario == 'child-heartbeat':
        for index in range(4):
            time.sleep(.6)
            append(p,'event_msg',{{'type':'exec_command_begin','call_id':'heartbeat-'+str(index),'command':['echo','progress']}})
            append(p,'event_msg',{{'type':'exec_command_end','call_id':'heartbeat-'+str(index),'exit_code':0,'aggregated_output':'progress'}})
    if scenario == 'timeout':
        time.sleep(10)
    elif scenario == 'interrupted':
        pathlib.Path(__file__).with_name('ready').touch()
        time.sleep(10)
    elif scenario == 'failed':
        append(p,'event_msg',{{'type':'error','turn_id':'turn-one','message':'public child failure'}})
    elif scenario == 'stopped':
        append(p,'event_msg',{{'type':'turn_aborted','turn_id':'turn-one'}})
    elif scenario != 'unfinished':
        complete(p,'turn-one','CHILD_RESULT')
    if scenario in ('two-turns', 'retry-pending'):
        start(p,'turn-two')
        if scenario == 'two-turns':
            complete(p,'turn-two','CHILD_RESULT')
    if scenario == 'stream-error':
        sys.stdout.buffer.write(b'\xff\n')
        sys.stdout.buffer.flush()
        time.sleep(10)
    # stdout stays quiet long enough to observe logs before parent completion.
    time.sleep(2.5 if scenario == 'quiet-nested' else 0.15)
print(json.dumps({{'type':'item.completed','item':{{'id':'parent-message','type':'agent_message','text':'PARENT_RESULT'}}}}),flush=True)
print(json.dumps({{'type':'turn.completed','usage':{{'input_tokens':7,'output_tokens':3}}}}),flush=True)
"#,
            root = ROOT,
            child = CHILD,
            nested = NESTED,
            protocol = include_str!("fixtures/codex-subagents/protocol-0.159.1.json"),
        );
        let bin = directory.join("codex-stub");
        std::fs::write(&bin, script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self(directory)
    }

    fn run(&self, timeout: u64) -> (TurnOutcome, Vec<(Duration, Value)>) {
        let (outcome, events) = self.execute(timeout, false);
        (outcome.unwrap(), events)
    }

    fn execute(
        &self,
        timeout: u64,
        interrupt: bool,
    ) -> (Result<TurnOutcome, String>, Vec<(Duration, Value)>) {
        let env = McpInjection {
            args: vec![],
            env: vec![(
                "CODEX_HOME".into(),
                self.0.join("home").to_string_lossy().into(),
            )],
            approvals: false,
        };
        let start = Instant::now();
        let mut events = Vec::new();
        let mut pid = 0;
        let mut cancel = None;
        let ready = self.0.join("ready");
        let outcome = run_turn_with_effort(
            self.0.to_str().unwrap(),
            "observer fixture",
            None,
            timeout,
            Vendor::Codex,
            self.0.join("codex-stub").to_str().unwrap(),
            None,
            None,
            None,
            &[],
            None,
            Some(&env),
            |pgid| {
                pid = pgid;
                if interrupt {
                    let ready = ready.clone();
                    cancel = Some(std::thread::spawn(move || {
                        let deadline = Instant::now() + Duration::from_secs(4);
                        while !ready.exists() && Instant::now() < deadline {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        unsafe {
                            nix::libc::kill(-(pgid as i32), nix::libc::SIGKILL);
                        }
                    }));
                }
            },
            |event| events.push((start.elapsed(), serde_json::to_value(event).unwrap())),
        );
        if let Some(cancel) = cancel {
            cancel.join().unwrap();
        }
        assert!(pid > 0);
        assert_ne!(
            unsafe { nix::libc::kill(pid as i32, 0) },
            0,
            "executor must reap its CLI before returning"
        );
        (outcome, events)
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn last_state<'a>(events: &'a [(Duration, Value)], child: &str) -> &'a Value {
    &events
        .iter()
        .rev()
        .find(|(_, v)| v["kind"] == "subagent_update" && v["child_thread_id"] == child)
        .expect("identified child must have durable metadata")
        .1
}

#[test]
fn quiet_logs_use_command_environment_filter_roots_and_precede_parent_result() {
    let stub = Stub::new("quiet-nested");
    let (outcome, events) = stub.run(5);
    assert!(!outcome.timed_out);
    assert_eq!(last_state(&events, CHILD)["state"], "done");
    assert_eq!(last_state(&events, CHILD)["model"], "fixture-child-model");
    assert_eq!(
        last_state(&events, NESTED)["parent_id"],
        format!("codex:{ROOT}:{CHILD}")
    );
    assert!(!events
        .iter()
        .any(|(_, v)| v.to_string().contains("UNRELATED")));
    let first_card = events
        .iter()
        .find(|(_, v)| v["kind"] == "subagent_update")
        .unwrap();
    assert!(
        first_card.0 < Duration::from_secs(3),
        "card latency: {:?}",
        first_card.0
    );
    let parent_text = events
        .iter()
        .position(|(_, v)| v["text"] == "PARENT_RESULT")
        .unwrap();
    assert!(
        events
            .iter()
            .position(|(_, v)| v["kind"] == "subagent_update")
            .unwrap()
            < parent_text,
        "logs must be observed while parent stdout is silent"
    );
    let parent_result = events
        .iter()
        .position(|(_, v)| v["kind"] == "result")
        .unwrap();
    for message in ["CHILD_RESULT", "NESTED_RESULT"] {
        let matched: Vec<_> = events
            .iter()
            .enumerate()
            .filter(|(_, (_, v))| v["kind"] == "subagent_message" && v["text"] == message)
            .collect();
        assert_eq!(matched.len(), 1, "public child result must be emitted once");
        assert!(matched[0].0 < parent_result);
    }
    assert_eq!(events[parent_result].1["tokens_in"], 7);
    let model = events
        .iter()
        .rev()
        .find(|(_, event)| event["kind"] == "model_snapshot" && event["resolved"].is_string())
        .unwrap();
    assert_eq!(model.1["resolved"], "fixture-main-model");
    let context = events
        .iter()
        .rev()
        .find(|(_, event)| event["kind"] == "context_usage")
        .unwrap();
    assert_eq!(context.1["context_tokens"], 123);
    assert_eq!(context.1["valid"], true);
}

#[test]
fn observed_child_activity_keeps_quiet_parent_execution_alive() {
    let (outcome, events) = Stub::new("child-heartbeat").run(1);
    assert!(
        !outcome.timed_out,
        "fresh child activity must renew idle watchdog"
    );
    assert_eq!(last_state(&events, CHILD)["state"], "done");
    assert_eq!(
        events
            .iter()
            .filter(|(_, event)| event["kind"] == "tool_use" && event["parent_id"].is_string())
            .count(),
        4
    );
}

#[test]
fn empty_wait_does_not_invent_a_child() {
    let (_, events) = Stub::new("empty-wait").run(5);
    assert!(!events.iter().any(|(_, v)| v["kind"] == "subagent_update"));
    assert!(events
        .iter()
        .any(|(_, v)| v["kind"] == "result" && v["is_error"] == false));
}

#[test]
fn sanitized_installed_protocol_fixture_reaches_the_executor_card_contract() {
    let (_, events) = Stub::new("protocol").run(5);
    for child in [CHILD, NESTED] {
        let observed = last_state(&events, child);
        assert_eq!(observed["state"], "done");
        assert_eq!(observed["model"], "gpt-6.1-sol");
        assert_eq!(observed["parent_thread_id"], ROOT);
        assert!(observed.get("parent_id").is_none());
    }
    for text in ["FIXTURE_1_RESULT", "FIXTURE_2_RESULT"] {
        assert_eq!(
            events
                .iter()
                .filter(|(_, v)| v["kind"] == "subagent_message" && v["text"] == text)
                .count(),
            1
        );
    }
}

#[test]
fn unfinished_child_is_unknown_after_successful_parent_exit() {
    let (_, events) = Stub::new("unfinished").run(5);
    assert_eq!(last_state(&events, CHILD)["state"], "unknown");
    assert!(!events.iter().any(|(_, v)| v["kind"] == "subagent_message"));
    assert!(events
        .iter()
        .any(|(_, v)| v["kind"] == "result" && v["is_error"] == false));
}

#[test]
fn explicit_failure_and_abort_keep_their_child_state() {
    for (scenario, state) in [("failed", "failed"), ("stopped", "stopped")] {
        let (_, events) = Stub::new(scenario).run(5);
        assert_eq!(last_state(&events, CHILD)["state"], state);
        assert!(events
            .iter()
            .any(|(_, v)| v["kind"] == "result" && v["is_error"] == false));
    }
}

#[test]
fn subsequent_child_turn_can_run_again_and_repeat_the_same_public_result() {
    let (_, events) = Stub::new("two-turns").run(5);
    let messages: Vec<_> = events
        .iter()
        .filter(|(_, v)| v["kind"] == "subagent_message")
        .collect();
    assert_eq!(
        messages.len(),
        2,
        "equal text from different turns must remain distinct"
    );
    assert_ne!(messages[0].1["source_key"], messages[1].1["source_key"]);
    assert_eq!(last_state(&events, CHILD)["child_turn_id"], "turn-two");
    assert_eq!(last_state(&events, CHILD)["state"], "done");
    let (_, retry) = Stub::new("retry-pending").run(5);
    assert_eq!(last_state(&retry, CHILD)["state"], "unknown");
    let states: Vec<_> = retry
        .iter()
        .filter(|(_, v)| v["kind"] == "subagent_update")
        .map(|(_, v)| v["state"].as_str().unwrap())
        .collect();
    let done = states.iter().position(|state| *state == "done").unwrap();
    assert!(states[done + 1..].contains(&"running"));
}

#[test]
fn idle_timeout_collects_final_child_observation_and_reaps_the_cli() {
    let stub = Stub::new("timeout");
    let before = Instant::now();
    let (outcome, events) = stub.run(1);
    assert!(outcome.timed_out);
    assert!(before.elapsed() < Duration::from_secs(5));
    assert_eq!(last_state(&events, CHILD)["state"], "unknown");
}

#[test]
fn cancellation_collects_final_child_observation_and_reaps_the_cli() {
    let stub = Stub::new("interrupted");
    let (outcome, events) = stub.execute(5, true);
    assert!(outcome.is_ok());
    assert_eq!(last_state(&events, CHILD)["state"], "unknown");
}

#[test]
fn invalid_stdout_still_collects_logs_and_reaps_the_cli() {
    let stub = Stub::new("stream-error");
    let (outcome, events) = stub.execute(5, false);
    assert!(outcome.unwrap_err().contains("출력 읽기 실패"));
    assert_eq!(last_state(&events, CHILD)["state"], "done");
}

#[tokio::test]
async fn observed_child_events_survive_conversation_and_runner_replay_storage() {
    use praxis_lib::db;
    let (_, events) = Stub::new("persisted").run(5);
    let db_path = temp_root::dir().join("codex-observer-persistence.sqlite");
    let pool = db::init_pool(db_path.to_str().unwrap()).await.unwrap();
    let task = db::insert_task(
        &pool,
        "/fixture",
        "branch",
        "main",
        "/fixture",
        "storage probe",
        Some("codex"),
        None,
        "conversation",
        1,
    )
    .await
    .unwrap();
    for (_, event) in &events {
        db::append_convo_event_with_runner_output(&pool, task, &event.to_string(), 2)
            .await
            .unwrap();
    }
    let history = db::list_convo_events(&pool, task).await.unwrap();
    let restored: Vec<Value> = history
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(restored.len(), events.len());
    let message = restored
        .iter()
        .find(|v| v["kind"] == "subagent_message")
        .unwrap();
    assert_eq!(message["text"], "CHILD_RESULT");
    assert_eq!(message["parent_id"], format!("codex:{ROOT}:{CHILD}"));
    assert!(message["source_key"].is_string());
    assert!(restored
        .iter()
        .any(|v| v["kind"] == "subagent_update" && v["state"] == "done"));
    assert!(restored
        .iter()
        .filter(|v| v["kind"] == "subagent_update")
        .all(|v| v.get("id").is_none()));
    let replay = db::list_task_output_after(&pool, 0, 1000).await.unwrap();
    assert_eq!(replay.len(), events.len());
    assert_eq!(
        replay
            .iter()
            .filter(|r| r.data.contains("\"kind\":\"subagent_message\""))
            .count(),
        1
    );
    pool.close().await;
}

/// Opt-in uses the same public executor as the desktop exec path. It changes no CLI config.
#[test]
#[ignore = "requires an authenticated installed Codex CLI and invokes two bounded agents"]
fn installed_codex_exec_observes_two_real_children() {
    let bin = std::env::var("DOJANG_CODEX_SMOKE_BIN").expect("set exact app exec binary");
    let dir = temp_root::dir().join("installed-codex-observer");
    std::fs::create_dir_all(&dir).unwrap();
    let mut events = Vec::new();
    let start = Instant::now();
    let outcome = run_turn_with_effort(
        dir.to_str().unwrap(),
        "This is a bounded UI observer smoke test. Spawn exactly two foreground subagents, task names card_alpha and card_beta. Give alpha the instruction to reply exactly CARD_ALPHA_OK and beta to reply exactly CARD_BETA_OK. No file edits, shell commands, research, or additional delegation. Collect both terminal results with collaboration tools before finishing. Your own final reply must be exactly PARENT_OK.",
        None, 90, Vendor::Codex, &bin, None, None, None, &[], None, None,
        |_| {}, |event| events.push((start.elapsed(), event)),
    ).unwrap();
    assert!(
        !outcome.timed_out,
        "live CLI timeout: {}",
        outcome.stderr_tail
    );
    let values: Vec<_> = events
        .iter()
        .map(|(elapsed, event)| (elapsed, serde_json::to_value(event).unwrap()))
        .collect();
    let children: std::collections::BTreeSet<_> = values
        .iter()
        .filter(|(_, v)| {
            v["kind"] == "subagent_update" && v["parent_thread_id"] == outcome.session_id
        })
        .filter_map(|(_, v)| v["child_thread_id"].as_str())
        .collect();
    assert_eq!(
        children.len(),
        2,
        "live child identities not fully observed"
    );
    for child in &children {
        let state = values
            .iter()
            .rev()
            .find(|(_, v)| v["kind"] == "subagent_update" && v["child_thread_id"] == *child)
            .unwrap();
        assert_eq!(state.1["state"], "done");
        assert!(state.1["model"].is_string());
    }
    for reply in ["CARD_ALPHA_OK", "CARD_BETA_OK"] {
        assert_eq!(
            values
                .iter()
                .filter(|(_, v)| v["kind"] == "subagent_message"
                    && v["text"].as_str().is_some_and(|t| t.trim() == reply))
                .count(),
            1
        );
    }
    assert!(events.iter().any(|(_, event)| matches!(
        event,
        ConvoEvent::Result {
            is_error: false,
            ..
        }
    )));
    eprintln!(
        "live Codex: two children completed with observed models/results in {:?}",
        start.elapsed()
    );
    if let Ok(path) = std::env::var("DOJANG_CODEX_SMOKE_EVENTS") {
        let events: Vec<_> = values.iter().map(|(_, v)| v).collect();
        std::fs::write(path, serde_json::to_string_pretty(&events).unwrap()).unwrap();
    }
    let _ = std::fs::remove_dir_all(dir);
}
