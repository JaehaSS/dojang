#![cfg(unix)]

#[path = "support/temp_root.rs"]
mod temp_root;

use praxis_lib::convo::{run_turn, ConvoEvent, Vendor};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT_STUB: AtomicU32 = AtomicU32::new(0);

fn run_stub(lines: &[&str]) -> Vec<ConvoEvent> {
    let id = NEXT_STUB.fetch_add(1, Ordering::SeqCst);
    let directory =
        temp_root::dir().join(format!("praxis-subagent-guard-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let binary = directory.join("claude-stub");
    let quoted_lines = lines
        .iter()
        .map(|line| format!("'{line}'"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut file = std::fs::File::create(&binary).unwrap();
    write!(file, "#!/bin/sh\nprintf '%s\\n' {quoted_lines}\n").unwrap();
    let mut permissions = file.metadata().unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&binary, permissions).unwrap();

    let mut events = Vec::new();
    run_turn(
        directory.to_str().unwrap(),
        "구현해줘",
        None,
        5,
        Vendor::Claude,
        binary.to_str().unwrap(),
        None,
        |_| {},
        |event| events.push(event),
    )
    .unwrap();
    std::fs::remove_dir_all(directory).ok();
    events
}

fn base_lines() -> [&'static str; 3] {
    [
        r#"{"type":"system","subtype":"init","session_id":"session"}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"agent-tool","name":"Agent","input":{"description":"구현 Worker"}}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"agent-tool","content":"Async agent launched successfully. agentId: internal","is_error":false}]}}"#,
    ]
}

#[test]
fn unfinished_async_agent_turn_is_not_reported_as_success() {
    let mut lines = base_lines().to_vec();
    lines.push(
        r#"{"type":"result","subtype":"success","is_error":false,"result":"나중에 검증하겠습니다","session_id":"session"}"#,
    );
    let events = run_stub(&lines);

    assert!(!events.iter().any(|event| matches!(
        event,
        ConvoEvent::ToolResult { summary, .. } if summary.contains("agentId")
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        ConvoEvent::Result {
            is_error: true,
            text,
            ..
        } if text.contains("완료 회수하기 전에 턴을 종료")
    )));
}

#[test]
fn completion_notification_allows_success_and_finishes_worker() {
    let mut lines = base_lines().to_vec();
    lines.push(
        r#"{"type":"user","message":{"role":"user","content":"<task-notification>\n<tool-use-id>agent-tool</tool-use-id>\n<status>completed</status>\n</task-notification>"}}"#,
    );
    lines.push(
        r#"{"type":"result","subtype":"success","is_error":false,"result":"검증 완료","session_id":"session"}"#,
    );
    let events = run_stub(&lines);

    assert!(events.iter().any(|event| matches!(
        event,
        ConvoEvent::ToolResult {
            is_error: false,
            tool_use_id: Some(id),
            ..
        } if id == "agent-tool"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        ConvoEvent::Result {
            is_error: false,
            ..
        }
    )));
}

/// claude 2.1.28x는 백그라운드 작업이 살아 있으면 `result` 뒤에도 프로세스를 끝내지 않고,
/// 알림이 오면 모델을 다시 불러 `result`를 또 낸다. 턴의 답은 마지막 것이고, 판정은 EOF에서
/// 한 번이다 — `result`마다 판정하면 회수가 끝난 작업이 미완료로 찍히고 경고가 반복된다.
#[test]
fn a_notification_after_the_first_result_lets_the_reinvoked_turn_finish_cleanly() {
    let mut lines = base_lines().to_vec();
    lines.push(
        r#"{"type":"result","subtype":"success","is_error":false,"result":"결과가 오면 이어가겠습니다","session_id":"session"}"#,
    );
    lines.push(
        r#"{"type":"system","subtype":"task_notification","task_id":"a1d772ce","tool_use_id":"agent-tool","status":"completed","output_file":"/tmp/a.output"}"#,
    );
    lines.push(r#"{"type":"system","subtype":"init","session_id":"session"}"#);
    lines.push(
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"검증 완료"}]}}"#,
    );
    lines.push(
        r#"{"type":"result","subtype":"success","is_error":false,"result":"검증 완료","session_id":"session"}"#,
    );
    let events = run_stub(&lines);

    let results = events
        .iter()
        .filter_map(|event| match event {
            ConvoEvent::Result { text, is_error, .. } => Some((text.clone(), *is_error)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        results,
        vec![("검증 완료".to_string(), false)],
        "Result는 마지막 하나만, 성공으로 나가야 한다"
    );
    assert!(events.iter().any(|event| matches!(
        event,
        ConvoEvent::ToolResult {
            is_error: false,
            tool_use_id: Some(id),
            ..
        } if id == "agent-tool"
    )));
}

#[test]
fn a_task_cut_after_the_result_fails_the_turn() {
    let mut lines = base_lines().to_vec();
    lines.push(
        r#"{"type":"result","subtype":"success","is_error":false,"result":"기다리겠습니다","session_id":"session"}"#,
    );
    lines.push(
        r#"{"type":"system","subtype":"task_notification","task_id":"a1d772ce","tool_use_id":"agent-tool","status":"stopped"}"#,
    );
    let events = run_stub(&lines);

    let results = events
        .iter()
        .filter(|event| matches!(event, ConvoEvent::Result { .. }))
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 1);
    assert!(matches!(
        results[0],
        ConvoEvent::Result { is_error: true, text, .. }
            if text.contains("끊은 작업") && text.contains("구현 Worker")
    ));
}
