#[path = "support/temp_root.rs"]
mod temp_root;

use std::time::Duration;

use praxis_lib::db::{self, Task};
use praxis_lib::goal_contract::GoalContract;
use praxis_lib::pty::PtyEvent;
use praxis_lib::runner::process;

#[test]
fn terminal_task_spawns_headless_agent_and_streams_output() {
    let task = Task {
        id: 1,
        repo: "/tmp".into(),
        branch: "branch".into(),
        base: "main".into(),
        base_revision: None,
        worktree_path: "/tmp".into(),
        // echo 후 짧은 sleep — 자식이 즉시 종료하면 Linux PTY가 드레인 전에 버퍼를 버릴 수 있다(flake).
        instruction: "echo runner-process-marker; sleep 1".into(),
        state: db::state::RUNNING.into(),
        created_at: 1,
        updated_at: 1,
        agent: Some("/bin/sh -c".into()),
        role: "implementer".into(),
        ensemble: None,
        model: None,
        reasoning_effort: None,
        service_tier: None,
        mode: "terminal".into(),
        kind: "coding".into(),
        convo_session_id: None,
        convo_pgid: None,
        goal_contract: Some(sqlx::types::Json(GoalContract {
            schema_version: 1,
            objective: "runner-contract-objective".into(),
            acceptance: vec!["runner output is persisted".into()],
            stop_conditions: vec![],
            must_preserve: vec![],
            protected_paths: vec![],
            non_goals: vec![],
        })),
        ambiguity: None,
        awaiting_kind: None,
        blocked_reason: None,
        pending_capsule: None,
        resumed_from: None,
        resumed_session: None,
        worktree_missing: false,
    };

    let (_session, events) = process::spawn_terminal_task(&task).unwrap();
    let mut output = String::new();
    while let Ok(event) = events.recv_timeout(Duration::from_secs(2)) {
        match event {
            PtyEvent::Output(bytes) => output.push_str(&String::from_utf8_lossy(&bytes)),
            PtyEvent::Exit(_) => break,
        }
    }

    assert!(
        output.contains("runner-process-marker"),
        "output: {output:?}"
    );
    assert!(
        output.contains("runner-contract-objective"),
        "Goal Contract was not rendered into the Runner prompt: {output:?}"
    );
}

#[test]
fn terminal_scrollback_reflects_pty_output_and_is_none_when_untracked() {
    let task = Task {
        id: 3,
        repo: "/tmp".into(),
        branch: "branch".into(),
        base: "main".into(),
        base_revision: None,
        worktree_path: "/tmp".into(),
        instruction: "runner-scrollback-marker".into(),
        state: db::state::RUNNING.into(),
        created_at: 1,
        updated_at: 1,
        agent: Some("/bin/echo".into()),
        role: "implementer".into(),
        ensemble: None,
        model: None,
        reasoning_effort: None,
        service_tier: None,
        mode: "terminal".into(),
        kind: "coding".into(),
        convo_session_id: None,
        convo_pgid: None,
        goal_contract: None,
        ambiguity: None,
        awaiting_kind: None,
        blocked_reason: None,
        pending_capsule: None,
        resumed_from: None,
        resumed_session: None,
        worktree_missing: false,
    };

    let (session, events) = process::spawn_terminal_task(&task).unwrap();
    while let Ok(event) = events.recv_timeout(Duration::from_secs(2)) {
        if matches!(event, PtyEvent::Exit(_)) {
            break;
        }
    }

    let active = process::active_terminal_tasks();
    assert!(process::terminal_scrollback(&active, task.id).is_none());
    active
        .lock()
        .unwrap()
        .insert(task.id, std::sync::Arc::new(std::sync::Mutex::new(session)));

    let scrollback = process::terminal_scrollback(&active, task.id).unwrap();
    assert!(
        String::from_utf8_lossy(&scrollback).contains("runner-scrollback-marker"),
        "scrollback: {}",
        String::from_utf8_lossy(&scrollback)
    );
}

#[test]
fn terminal_task_without_contract_executes_the_legacy_instruction() {
    let task = Task {
        id: 2,
        repo: "/tmp".into(),
        branch: "branch".into(),
        base: "main".into(),
        base_revision: None,
        worktree_path: "/tmp".into(),
        instruction: "byte-identical-legacy-marker".into(),
        state: db::state::RUNNING.into(),
        created_at: 1,
        updated_at: 1,
        agent: Some("/bin/echo".into()),
        role: "implementer".into(),
        ensemble: None,
        model: None,
        reasoning_effort: None,
        service_tier: None,
        mode: "terminal".into(),
        kind: "coding".into(),
        convo_session_id: None,
        convo_pgid: None,
        goal_contract: None,
        ambiguity: None,
        awaiting_kind: None,
        blocked_reason: None,
        pending_capsule: None,
        resumed_from: None,
        resumed_session: None,
        worktree_missing: false,
    };

    let (_session, events) = process::spawn_terminal_task(&task).unwrap();
    let mut output = String::new();
    while let Ok(event) = events.recv_timeout(Duration::from_secs(2)) {
        match event {
            PtyEvent::Output(bytes) => output.push_str(&String::from_utf8_lossy(&bytes)),
            PtyEvent::Exit(_) => break,
        }
    }

    assert!(output.contains("byte-identical-legacy-marker"));
    assert!(!output.contains("Praxis Goal Contract"));
}

#[tokio::test]
async fn terminal_process_persists_output_and_marks_task_awaiting_review() {
    let db_path = temp_root::dir()
        .join(format!(
            "praxis-runner-process-{}.sqlite",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned();
    let pool = db::init_pool(&db_path).await.unwrap();
    // echo 후 짧은 sleep — 자식이 즉시 종료하면 Linux PTY가 드레인 전에 버퍼를 버릴 수 있다(flake).
    let task_id = db::insert_task(
        &pool,
        "/tmp",
        "branch",
        "main",
        "/tmp",
        "echo runner-db-marker; sleep 1",
        Some("/bin/sh -c"),
        None,
        "terminal",
        1,
    )
    .await
    .unwrap();
    db::update_state(&pool, task_id, db::state::RUNNING, 2)
        .await
        .unwrap();
    let task = db::get_task(&pool, task_id).await.unwrap().unwrap();

    assert_eq!(
        process::run_terminal_task(pool.clone(), task, 3, process::active_terminal_tasks())
            .await
            .unwrap(),
        0
    );

    assert_eq!(
        db::get_task(&pool, task_id).await.unwrap().unwrap().state,
        db::state::AWAITING_REVIEW
    );
    assert!(db::list_task_output_after(&pool, 0, 10).await.unwrap()[0]
        .data
        .contains("runner-db-marker"));
    assert!(db::list_runner_events_after(&pool, 0, 10)
        .await
        .unwrap()
        .iter()
        .any(|event| event.kind == "completed"));
    let _ = std::fs::remove_file(db_path);
}

#[cfg(unix)]
#[tokio::test]
async fn conversation_process_persists_events_session_and_replay_output() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let suffix = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let db_path = temp_root::dir()
        .join(format!("praxis-runner-conversation-{suffix}.sqlite"))
        .to_string_lossy()
        .into_owned();
    let script_path = temp_root::dir().join(format!("praxis-runner-conversation-{suffix}.sh"));
    let mut script = std::fs::File::create(&script_path).unwrap();
    writeln!(
        script,
        "#!/bin/sh\nprintf '%s\\n' '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"runner-session\"}}'\nprintf '%s\\n' '{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"runner conversation marker\"}}]}}}}'\nprintf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"done\",\"session_id\":\"runner-session\"}}'"
    )
    .unwrap();
    let mut permissions = script.metadata().unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions).unwrap();
    // 쓰기 fd를 exec 전에 닫는다 — 병렬 테스트의 fork가 열린 fd를 상속한 채 exec하면 ETXTBSY.
    drop(script);

    let pool = db::init_pool(&db_path).await.unwrap();
    let task_id = db::insert_task(
        &pool,
        "/tmp",
        "branch",
        "main",
        "/tmp",
        "runner conversation marker",
        Some("claude"),
        None,
        "conversation",
        1,
    )
    .await
    .unwrap();
    db::update_state(&pool, task_id, db::state::RUNNING, 2)
        .await
        .unwrap();
    let task = db::get_task(&pool, task_id).await.unwrap().unwrap();

    process::run_conversation_task_with_bin(
        pool.clone(),
        task,
        3,
        process::active_conversation_tasks(),
        script_path.to_str().unwrap(),
    )
    .await
    .unwrap();

    let stored = db::get_task(&pool, task_id).await.unwrap().unwrap();
    assert_eq!(stored.state, db::state::AWAITING_REVIEW);
    assert_eq!(stored.convo_session_id.as_deref(), Some("runner-session"));
    assert_eq!(stored.convo_pgid, None);
    let convo_events = db::list_convo_events(&pool, task_id).await.unwrap();
    assert!(convo_events
        .iter()
        .any(|event| event.contains("runner conversation marker")));
    // user 턴도 durable transcript에 남아야 원격 데스크톱이 말풍선을 복원한다.
    assert!(convo_events
        .iter()
        .any(|event| event.contains("\"kind\":\"user\"")));
    assert!(db::list_task_output_after(&pool, 0, 10)
        .await
        .unwrap()
        .iter()
        .any(|output| output.data.contains("runner conversation marker")));

    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(script_path);
}

/// `Result` 이벤트 없이 죽은 턴은 "completed"가 아니라 FAILED로 마감되고 사인이 트랜스크립트에
/// 남아야 한다. resume 토큰(session id)이 이미 관측된 턴은 `run_turn`이 `Ok(outcome)`을 돌려주므로,
/// 러너가 outcome의 사인을 읽지 않으면 죽은 세션이 조용히 검토 대기로 넘어간다(회귀 방지).
#[cfg(unix)]
#[tokio::test]
async fn conversation_without_result_event_fails_with_surfaced_cause() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let suffix = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let db_path = temp_root::dir()
        .join(format!("praxis-runner-convo-dead-{suffix}.sqlite"))
        .to_string_lossy()
        .into_owned();
    let script_path = temp_root::dir().join(format!("praxis-runner-convo-dead-{suffix}.sh"));
    let mut script = std::fs::File::create(&script_path).unwrap();
    // session id는 방출하고 result 없이 stderr를 남기며 즉사 — 워치독 kill/프로세스 사망과 같은 형태.
    writeln!(
        script,
        "#!/bin/sh\nprintf '%s\\n' '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"runner-dead\"}}'\necho 'boom: no space left on device' >&2\nexit 3"
    )
    .unwrap();
    let mut permissions = script.metadata().unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions).unwrap();
    drop(script);

    let pool = db::init_pool(&db_path).await.unwrap();
    let task_id = db::insert_task(
        &pool,
        "/tmp",
        "branch",
        "main",
        "/tmp",
        "dead turn",
        Some("claude"),
        None,
        "conversation",
        1,
    )
    .await
    .unwrap();
    db::update_state(&pool, task_id, db::state::RUNNING, 2)
        .await
        .unwrap();
    let task = db::get_task(&pool, task_id).await.unwrap().unwrap();

    let result = process::run_conversation_task_with_bin(
        pool.clone(),
        task,
        3,
        process::active_conversation_tasks(),
        script_path.to_str().unwrap(),
    )
    .await;
    assert!(result.is_err(), "사인 없는 종료는 Err로 전파되어야 한다");

    let stored = db::get_task(&pool, task_id).await.unwrap().unwrap();
    assert_eq!(stored.state, db::state::FAILED);
    // 대화 맥락은 유효하므로 resume 토큰은 남긴다 — 사용자가 이어서 재개할 수 있어야 한다.
    assert_eq!(stored.convo_session_id.as_deref(), Some("runner-dead"));
    let convo_events = db::list_convo_events(&pool, task_id).await.unwrap();
    assert!(
        convo_events
            .iter()
            .any(|event| event.contains("결과 없이 종료") && event.contains("exit 3")),
        "사인이 트랜스크립트에 남아야 한다: {convo_events:?}"
    );
    assert!(
        convo_events
            .iter()
            .any(|event| event.contains("no space left on device")),
        "stderr 테일이 표면화되어야 한다: {convo_events:?}"
    );

    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(script_path);
}

/// 활성 PTY 레지스트리를 통한 stdin 전달 — 원격 후속 입력 경로의 process 계층 검증.
#[cfg(unix)]
#[test]
fn terminal_input_reaches_the_active_pty_session() {
    use std::sync::{Arc, Mutex};

    let task = Task {
        id: 7,
        repo: "/tmp".into(),
        branch: "branch".into(),
        base: "main".into(),
        base_revision: None,
        worktree_path: "/tmp".into(),
        instruction: "-".into(), // `cat -` — stdin을 그대로 되돌린다.
        state: db::state::RUNNING.into(),
        created_at: 1,
        updated_at: 1,
        agent: Some("/bin/cat".into()),
        ensemble: None,
        model: None,
        mode: "terminal".into(),
        kind: "coding".into(),
        convo_session_id: None,
        convo_pgid: None,
        goal_contract: None,
        ambiguity: None,
        awaiting_kind: None,
        blocked_reason: None,
        pending_capsule: None,
        resumed_from: None,
        resumed_session: None,
        worktree_missing: false,
        role: "implementer".into(),
        reasoning_effort: None,
        service_tier: None,
    };
    let (session, events) = process::spawn_terminal_task(&task).unwrap();
    let active = process::active_terminal_tasks();
    active
        .lock()
        .unwrap()
        .insert(task.id, Arc::new(Mutex::new(session)));

    assert!(process::write_terminal_input(&active, 99, b"lost").is_none());
    process::write_terminal_input(&active, task.id, b"input-marker\r")
        .unwrap()
        .unwrap();

    let mut output = String::new();
    while !output.contains("input-marker") {
        match events.recv_timeout(Duration::from_secs(2)).unwrap() {
            PtyEvent::Output(bytes) => output.push_str(&String::from_utf8_lossy(&bytes)),
            PtyEvent::Exit(code) => panic!("premature exit {code}: {output:?}"),
        }
    }
    assert!(process::cancel_terminal_task(&active, task.id));
}

/// B-1: annotations resend가 재사용하는 후속 메시지 재개 경로 — 최초 instruction이 아니라
/// 주어진 message가 그대로 프롬프트가 되고, user 이벤트로도 영속화된다(local start_convo_turn 관측 계약과 동일).
#[cfg(unix)]
#[tokio::test]
async fn resume_conversation_task_with_bin_injects_message_and_returns_to_awaiting_review() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let suffix = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let db_path = temp_root::dir()
        .join(format!("praxis-runner-resume-{suffix}.sqlite"))
        .to_string_lossy()
        .into_owned();
    let script_path = temp_root::dir().join(format!("praxis-runner-resume-{suffix}.sh"));
    let mut script = std::fs::File::create(&script_path).unwrap();
    writeln!(
        script,
        "#!/bin/sh\nprintf '%s\\n' '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"resume-session\"}}'\nprintf '%s\\n' '{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"applied review annotations\"}}]}}}}'\nprintf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"done\",\"session_id\":\"resume-session\"}}'"
    )
    .unwrap();
    let mut permissions = script.metadata().unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions).unwrap();

    let pool = db::init_pool(&db_path).await.unwrap();
    let task_id = db::insert_task(
        &pool,
        "/tmp",
        "branch",
        "main",
        "/tmp",
        "original instruction",
        Some("claude"),
        None,
        "conversation",
        1,
    )
    .await
    .unwrap();
    db::set_convo_session(&pool, task_id, "resume-session")
        .await
        .unwrap();
    db::update_state(&pool, task_id, db::state::AWAITING_REVIEW, 2)
        .await
        .unwrap();
    let task = db::get_task(&pool, task_id).await.unwrap().unwrap();

    process::resume_conversation_task_with_bin(
        pool.clone(),
        task,
        "[리뷰 주석 1건 — 각 항목을 반영하고 완료 후 보고할 것]\n1. a.ts:1 (변경 후 기준)\n   코멘트: 고쳐줘\n"
            .to_string(),
        3,
        process::active_conversation_tasks(),
        script_path.to_str().unwrap(),
    )
    .await
    .unwrap();

    let stored = db::get_task(&pool, task_id).await.unwrap().unwrap();
    assert_eq!(stored.state, db::state::AWAITING_REVIEW);
    let events = db::list_convo_events(&pool, task_id).await.unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.contains("고쳐줘") && event.contains("\"kind\":\"user\"")),
        "후속 메시지가 user 이벤트로 영속화되어야 함: {events:?}"
    );
    assert!(events
        .iter()
        .any(|event| event.contains("applied review annotations")));

    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(script_path);
}

#[cfg(unix)]
#[tokio::test]
async fn conversation_cancel_terminates_the_registered_process_group() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let suffix = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let db_path = temp_root::dir()
        .join(format!("praxis-runner-conversation-cancel-{suffix}.sqlite"))
        .to_string_lossy()
        .into_owned();
    let script_path =
        temp_root::dir().join(format!("praxis-runner-conversation-cancel-{suffix}.sh"));
    let mut script = std::fs::File::create(&script_path).unwrap();
    writeln!(script, "#!/bin/sh\nsleep 10").unwrap();
    let mut permissions = script.metadata().unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions).unwrap();
    // 쓰기 fd를 exec 전에 닫는다 — 병렬 테스트의 fork가 열린 fd를 상속한 채 exec하면 ETXTBSY.
    drop(script);

    let pool = db::init_pool(&db_path).await.unwrap();
    let task_id = db::insert_task(
        &pool,
        "/tmp",
        "branch",
        "main",
        "/tmp",
        "cancel conversation",
        Some("claude"),
        None,
        "conversation",
        1,
    )
    .await
    .unwrap();
    db::update_state(&pool, task_id, db::state::RUNNING, 2)
        .await
        .unwrap();
    let task = db::get_task(&pool, task_id).await.unwrap().unwrap();
    let active = process::active_conversation_tasks();
    let task_active = active.clone();
    let task_pool = pool.clone();
    let bin = script_path.to_string_lossy().into_owned();
    let handle = tokio::spawn(async move {
        process::run_conversation_task_with_bin(task_pool, task, 3, task_active, &bin).await
    });

    for _ in 0..50 {
        if active.lock().unwrap().contains_key(&task_id) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(process::cancel_conversation_task(&active, task_id));
    assert!(handle.await.unwrap().is_err());

    let stored = db::get_task(&pool, task_id).await.unwrap().unwrap();
    assert_eq!(stored.state, db::state::FAILED);
    assert_eq!(stored.convo_pgid, None);
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(script_path);
}
