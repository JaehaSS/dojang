#![cfg(unix)]

#[path = "support/temp_root.rs"]
mod temp_root;

use chrono::{Duration as ChronoDuration, Utc};
use praxis_lib::convo::{run_turn_with_effort, TurnOutcome, Vendor};
use praxis_lib::preview_bridge::mcp::McpInjection;
use serde_json::Value;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

/// Produces a process-unique, path-safe UUID-shaped session id without sharing global state with
/// the Codex process.  The observer only accepts conservative ASCII session identifiers.
fn session_id() -> String {
    let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::SeqCst);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}",
        std::process::id(),
        sequence & 0xffff,
        (nanos >> 48) & 0xfff,
        (nanos >> 36) & 0xfff,
        nanos & 0xffff_ffff_ffff
    )
}

struct Stub {
    directory: PathBuf,
    root: String,
}

impl Stub {
    fn new(scenario: &str) -> Self {
        let directory = temp_root::dir().join(format!(
            "codex-reader-{scenario}-{}",
            NEXT_FIXTURE.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let root = session_id();

        if scenario == "resume-stale" {
            let child = session_id();
            let sessions = directory
                .join("home/sessions")
                .join(Utc::now().format("%Y/%m/%d").to_string());
            std::fs::create_dir_all(&sessions).unwrap();
            let stale = (Utc::now() - ChronoDuration::hours(1)).to_rfc3339();
            let old_log = format!(
                "{{\"timestamp\":\"{stale}\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{child}\",\"source\":{{\"subagent\":{{\"thread_spawn\":{{\"parent_thread_id\":\"{root}\",\"agent_nickname\":\"old\"}}}}}}}}}}\n{{\"timestamp\":\"{stale}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_complete\",\"turn_id\":\"old-turn\",\"last_agent_message\":\"STALE_RESULT\"}}}}\n"
            );
            std::fs::write(sessions.join(format!("rollout-{child}.jsonl")), old_log).unwrap();
        }

        let script = format!(
            r#"#!/usr/bin/python3
import datetime, json, os, pathlib, sys, time, uuid

root = {root:?}
scenario = {scenario:?}
sessions = pathlib.Path(os.environ['CODEX_HOME']) / 'sessions'
day = sessions / datetime.datetime.now(datetime.timezone.utc).strftime('%Y/%m/%d')
day.mkdir(parents=True, exist_ok=True)

def stamp(delta=0):
    return (datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(seconds=delta)).isoformat().replace('+00:00', 'Z')

def eid():
    return str(uuid.uuid4())

def row(kind, payload, delta=0):
    return json.dumps({{'timestamp': stamp(delta), 'type': kind, 'payload': payload}}) + '\n'

def meta(cid, parent, label='fixture'):
    return row('session_meta', {{'id': cid, 'source': {{'subagent': {{'thread_spawn': {{
        'parent_thread_id': parent, 'agent_path': '/root/' + label, 'agent_nickname': label
    }}}}}}}})

def event(kind, turn=None, **fields):
    payload = {{'type': kind}}
    if turn is not None:
        payload['turn_id'] = turn
    payload.update(fields)
    return row('event_msg', payload)

def context(turn, model='fixture-reader-model'):
    return row('turn_context', {{'turn_id': turn, 'model': model}})

def write(path, text, mode='a'):
    with path.open(mode) as file:
        file.write(text)
        file.flush()

def child_log(cid):
    return day / ('rollout-' + cid + '.jsonl')

def parent_done():
    print(json.dumps({{'type': 'item.completed', 'item': {{'id': 'parent-message', 'type': 'agent_message', 'text': 'PARENT_RESULT'}}}}), flush=True)
    print(json.dumps({{'type': 'turn.completed', 'usage': {{'input_tokens': 5, 'output_tokens': 2}}}}), flush=True)

if scenario != 'resume-stale':
    print(json.dumps({{'type': 'thread.started', 'thread_id': root}}), flush=True)

if scenario == 'split-append':
    child = eid()
    log = child_log(child)
    first = meta(child, root, 'split') + event('task_started', 'split-turn')
    # The reader must retain this incomplete JSONL record until its newline arrives.
    pivot = len(first) // 2
    write(log, first[:pivot], 'w')
    time.sleep(.35)
    write(log, first[pivot:] + context('split-turn'))
    time.sleep(.35)
    write(log, event('task_complete', 'split-turn', last_agent_message='SPLIT_RESULT'))
    time.sleep(.7)
elif scenario == 'replace':
    child = eid()
    nested = eid()
    log = child_log(child)
    old = meta(child, root, 'replace') + event('task_started', 'old-turn') + context('old-turn') + event('exec_command_begin', 'old-turn', call_id='direct-call', command='printf DIRECT_TOOL', auth='TOOL_AUTH_DO_NOT_EMIT', authorization='TOOL_AUTHZ_DO_NOT_EMIT') + event('exec_command_end', 'old-turn', call_id='direct-call', stdout='DIRECT_OUTPUT') + event('task_complete', 'old-turn', last_agent_message='REPLACED_RESULT')
    write(log, old, 'w')
    nested_old = meta(nested, child, 'nested') + event('task_started', 'nested-turn') + event('exec_command_begin', 'nested-turn', call_id='nested-call', command='printf NESTED_TOOL', encrypted_content='TOOL_ENCRYPTED_DO_NOT_EMIT', reasoning='TOOL_REASONING_DO_NOT_EMIT') + event('exec_command_end', 'nested-turn', call_id='nested-call', stdout='NESTED_OUTPUT') + event('task_complete', 'nested-turn', last_agent_message='NESTED_RESULT')
    write(child_log(nested), nested_old, 'w')
    time.sleep(.75)
    replacement = log.with_suffix('.replacement')
    new = old + event('task_started', 'new-turn') + context('new-turn') + event('task_complete', 'new-turn', last_agent_message='NEW_RESULT')
    write(replacement, new, 'w')
    os.replace(replacement, log)
    time.sleep(.8)
elif scenario == 'filters':
    valid = eid()
    write(child_log(valid), meta(valid, root, 'valid') + event('task_started', 'valid-turn') + event('task_complete', 'valid-turn', last_agent_message='VALID_ROOT'), 'w')
    root2 = eid()
    foreign = eid()
    write(child_log(foreign), meta(foreign, root2, 'foreign') + event('task_complete', 'foreign-turn', last_agent_message='FOREIGN_ROOT2'), 'w')
    cycle_a, cycle_b = eid(), eid()
    write(child_log(cycle_a), meta(cycle_a, cycle_b, 'cycle-a') + event('task_complete', 'cycle-a-turn', last_agent_message='CYCLE_A'), 'w')
    write(child_log(cycle_b), meta(cycle_b, cycle_a, 'cycle-b') + event('task_complete', 'cycle-b-turn', last_agent_message='CYCLE_B'), 'w')
    parent = root
    for depth in range(1, 11):
        child = eid()
        write(child_log(child), meta(child, parent, 'depth-' + str(depth)) + event('task_complete', 'depth-' + str(depth), last_agent_message='DEPTH_' + str(depth)), 'w')
        parent = child
    time.sleep(.9)
elif scenario == 'stream-missing':
    running, completed, unspecified = eid(), eid(), eid()
    states = {{running: {{'status': 'running'}}, completed: {{'status': 'completed'}}}}
    print(json.dumps({{'type': 'item.completed', 'item': {{
        'id': 'spawn-1', 'type': 'collab_tool_call', 'tool': 'spawn_agent',
        'sender_thread_id': root, 'receiver_thread_ids': [running, completed, unspecified],
        'agents_states': states, 'status': 'completed'
    }}}}), flush=True)
    # No log is ever written for any receiver.  The outer receipt is completed, but the last
    # receiver has no per-child state and therefore no completion evidence.
    time.sleep(.35)
elif scenario == 'fallbacks':
    unknown, unsupported, large, private = eid(), eid(), eid(), eid()
    write(child_log(unknown), meta(unknown, root, 'unknown'), 'w')
    write(child_log(unsupported), meta(unsupported, root, 'unsupported') + event('future_event', 'future-turn', value='ignored'), 'w')
    write(child_log(large), meta(large, root, 'large') + ('x' * 70000) + '\n' + event('task_started', 'large-turn') + event('task_complete', 'large-turn', last_agent_message='LARGE_RESULT'), 'w')
    private_payload = event('task_complete', 'private-turn', last_agent_message='PUBLIC_RESULT', reasoning='REASONING_DO_NOT_EMIT', encrypted_content='ENCRYPTED_DO_NOT_EMIT', auth='AUTH_DO_NOT_EMIT', authorization='AUTHZ_DO_NOT_EMIT')
    write(child_log(private), meta(private, root, 'private') + private_payload, 'w')
    time.sleep(.9)
elif scenario == 'resume-stale':
    time.sleep(.75)
else:
    raise SystemExit('unknown scenario: ' + scenario)

parent_done()
"#,
        );
        let bin = directory.join("codex-stub");
        std::fs::write(&bin, script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self { directory, root }
    }

    fn run(&self, resume: bool) -> (TurnOutcome, Vec<(Duration, Value)>) {
        let injection = McpInjection {
            args: vec![],
            env: vec![(
                "CODEX_HOME".into(),
                self.directory.join("home").to_string_lossy().into(),
            )],
            approvals: false,
        };
        let start = Instant::now();
        let mut events = Vec::new();
        let mut process_group = 0;
        let outcome = run_turn_with_effort(
            self.directory.to_str().unwrap(),
            "reader fixture",
            resume.then_some(self.root.as_str()),
            5,
            Vendor::Codex,
            self.directory.join("codex-stub").to_str().unwrap(),
            None,
            None,
            None,
            &[],
            None,
            Some(&injection),
            |pgid| process_group = pgid,
            |event| events.push((start.elapsed(), serde_json::to_value(event).unwrap())),
        );
        assert!(process_group > 0);
        assert_ne!(
            unsafe { nix::libc::kill(process_group as i32, 0) },
            0,
            "the public executor must reap its fixture process"
        );
        (outcome.unwrap(), events)
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn messages(events: &[(Duration, Value)]) -> Vec<&Value> {
    events
        .iter()
        .map(|(_, event)| event)
        .filter(|event| event["kind"] == "subagent_message")
        .collect()
}

fn updates(events: &[(Duration, Value)]) -> Vec<&Value> {
    events
        .iter()
        .map(|(_, event)| event)
        .filter(|event| event["kind"] == "subagent_update")
        .collect()
}

#[test]
fn split_jsonl_is_completed_by_append_while_parent_stdout_is_quiet() {
    let (outcome, events) = Stub::new("split-append").run(false);
    assert!(!outcome.timed_out);
    let split = messages(&events)
        .into_iter()
        .filter(|event| event["text"] == "SPLIT_RESULT")
        .collect::<Vec<_>>();
    assert_eq!(split.len(), 1, "the completed split line must emit once");
    let first_child = events
        .iter()
        .position(|(_, event)| event["kind"] == "subagent_update")
        .expect("child metadata while parent stdout is quiet");
    let parent_text = events
        .iter()
        .position(|(_, event)| event["text"] == "PARENT_RESULT")
        .expect("parent text");
    assert!(first_child < parent_text);
    assert!(events[first_child].0 < Duration::from_secs(3));
}

#[test]
fn replacing_a_log_replays_no_old_result_but_accepts_a_new_child_turn() {
    let stub = Stub::new("replace");
    let root = stub.root.clone();
    let (_, events) = stub.run(false);
    assert_eq!(
        messages(&events)
            .iter()
            .filter(|event| event["text"] == "REPLACED_RESULT")
            .count(),
        1,
        "the replacement repeats old bytes but not the old public result"
    );
    assert_eq!(
        messages(&events)
            .iter()
            .filter(|event| event["text"] == "NEW_RESULT")
            .count(),
        1
    );
    let last = updates(&events)
        .into_iter()
        .rev()
        .find(|event| event["child_turn_id"] == "new-turn")
        .expect("observed replacement child new turn");
    assert_eq!(last["child_turn_id"], "new-turn");
    assert_eq!(last["state"], "done");

    let direct_tools = events
        .iter()
        .map(|(_, event)| event)
        .filter(|event| {
            event["kind"] == "tool_use"
                && event["summary"]
                    .as_str()
                    .is_some_and(|summary| summary.contains("DIRECT_TOOL"))
        })
        .collect::<Vec<_>>();
    let nested_tools = events
        .iter()
        .map(|(_, event)| event)
        .filter(|event| {
            event["kind"] == "tool_use"
                && event["summary"]
                    .as_str()
                    .is_some_and(|summary| summary.contains("NESTED_TOOL"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        direct_tools.len(),
        1,
        "replacement must not replay tool starts"
    );
    assert_eq!(nested_tools.len(), 1, "nested tool must remain observable");
    let direct_parent = direct_tools[0]["parent_id"].as_str().unwrap();
    let nested_parent = nested_tools[0]["parent_id"].as_str().unwrap();
    assert!(direct_parent.starts_with(&format!("codex:{root}:")));
    assert!(nested_parent.starts_with(&format!("codex:{root}:")));
    assert_ne!(
        direct_parent, nested_parent,
        "tool output belongs to its own child card"
    );
    assert_eq!(
        events
            .iter()
            .filter(|(_, event)| {
                event["kind"] == "tool_result" && event["summary"] == "DIRECT_OUTPUT"
            })
            .count(),
        1,
        "replacement must not replay tool results"
    );
    let rendered = serde_json::to_string(&events).unwrap();
    for private in [
        "TOOL_AUTH_DO_NOT_EMIT",
        "TOOL_AUTHZ_DO_NOT_EMIT",
        "TOOL_ENCRYPTED_DO_NOT_EMIT",
        "TOOL_REASONING_DO_NOT_EMIT",
    ] {
        assert!(
            !rendered.contains(private),
            "tool private field {private} leaked"
        );
    }
}

#[test]
fn resume_does_not_replay_or_downgrade_an_old_completed_child_without_new_activity() {
    let (_, events) = Stub::new("resume-stale").run(true);
    assert!(
        updates(&events).is_empty(),
        "a completed child older than this resumed execution must remain historical"
    );
    assert!(messages(&events).is_empty());
    assert!(events
        .iter()
        .any(|(_, event)| event["kind"] == "result" && event["is_error"] == false));
}

#[test]
fn reader_accepts_only_the_root_descendants_within_the_depth_bound() {
    let (_, events) = Stub::new("filters").run(false);
    let rendered = serde_json::to_string(&events).unwrap();
    assert!(rendered.contains("VALID_ROOT"));
    for excluded in ["FOREIGN_ROOT2", "CYCLE_A", "CYCLE_B", "DEPTH_9", "DEPTH_10"] {
        assert!(
            !rendered.contains(excluded),
            "{excluded} must not cross the root, cycle, or depth boundary"
        );
    }
    assert!(updates(&events)
        .iter()
        .all(|event| event["root_thread_id"].is_string()));
}

#[test]
fn explicit_stream_receivers_without_logs_never_invent_completion() {
    let (_, events) = Stub::new("stream-missing").run(false);
    let stream_updates = updates(&events);
    assert_eq!(
        stream_updates.len(),
        4,
        "three initial stream states and the final running fallback"
    );
    assert!(stream_updates.iter().all(|event| {
        event["observation_source"] == "stream" && event["observation_status"] == "missing"
    }));
    assert!(stream_updates
        .iter()
        .any(|event| event["state"] == "running"));
    assert!(stream_updates
        .iter()
        .any(|event| event["state"] == "unknown"));
    assert!(
        stream_updates.iter().any(|event| event["state"] == "done"),
        "an explicit per-child completed state is direct stream evidence"
    );
    assert_eq!(
        stream_updates
            .iter()
            .filter(|event| event["state"] == "done")
            .count(),
        1,
        "the outer collab receipt must not mark the unspecified receiver done"
    );
    assert!(messages(&events).is_empty());
}

#[test]
fn unsupported_or_oversize_records_fall_back_without_leaking_private_fields_or_failing_parent() {
    let (outcome, events) = Stub::new("fallbacks").run(false);
    assert!(!outcome.timed_out);
    assert_eq!(
        messages(&events)
            .iter()
            .filter(|event| event["text"] == "LARGE_RESULT")
            .count(),
        1,
        "a rejected oversize record must not poison later JSONL records"
    );
    assert!(messages(&events)
        .iter()
        .any(|event| event["text"] == "PUBLIC_RESULT"));
    let values = serde_json::to_string(&events).unwrap();
    for secret in [
        "REASONING_DO_NOT_EMIT",
        "ENCRYPTED_DO_NOT_EMIT",
        "AUTH_DO_NOT_EMIT",
        "AUTHZ_DO_NOT_EMIT",
    ] {
        assert!(
            !values.contains(secret),
            "private {secret} leaked into API events"
        );
    }
    assert!(
        updates(&events)
            .iter()
            .filter(|event| event["state"] == "unknown")
            .count()
            >= 2
    );
    for (title, status) in [
        ("/root/unsupported", "unsupported"),
        ("/root/large", "limited"),
    ] {
        let final_update = updates(&events)
            .into_iter()
            .rev()
            .find(|event| event["title"] == title)
            .unwrap_or_else(|| panic!("missing {title} fallback update"));
        assert_eq!(
            final_update["observation_status"], status,
            "the incomplete {status} observation must remain visible after later records"
        );
    }
    assert!(events
        .iter()
        .any(|(_, event)| event["kind"] == "result" && event["is_error"] == false));
}
