use super::*;
fn db<T>(f: impl std::future::Future<Output = T>) -> T {
    tauri::async_runtime::block_on(f)
}
fn pool() -> SqlitePool {
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
#[test]
#[cfg(unix)]
fn restart_recovers_owned_processes_and_expires_committed_answers() {
    use std::os::unix::process::CommandExt;
    let p = pool();
    let execution = db(ledger::begin(&p, 1, 100)).unwrap();
    let mut command = Command::new("/usr/bin/python3");
    command
        .args(["-c", "import time;time.sleep(30)"])
        .process_group(0);
    let scope = TurnProcessScope::attach(&mut command);
    let mut child = ReapOnDrop::new(command.spawn().unwrap());
    let identity = crate::runner::process_identity::observe_group_leader(child.id())
        .unwrap()
        .unwrap();
    db(ledger::spawned(
        &p,
        &execution,
        child.id(),
        &identity,
        scope.marker(),
    ))
    .unwrap();
    db(ledger::started(&p, &execution, "thread", "turn")).unwrap();
    let args = json!({"kind":"clarification","questions":[{"id":"q","question":"Continue?","options":[],"allow_free_text":true,"is_secret":false}]});
    let question = db(ledger::open(&p, &execution, &json!(1), "call", &args, 100)).unwrap();
    db(ledger::submit(
        &p,
        1,
        &execution,
        &question,
        "request",
        &[ledger::Answer {
            question_id: "q".into(),
            option_id: None,
            text: Some("yes".into()),
        }],
        101,
    ))
    .unwrap();
    db(recover_execution(&p, 1)).unwrap();
    child.wait().unwrap();
    let snapshot = db(ledger::snapshot(&p, 1)).unwrap();
    assert_eq!(snapshot.phase, "idle");
    assert_eq!(snapshot.items[0].state, "closed");
    assert_eq!(snapshot.items[0].receipt.as_ref().unwrap().state, "unknown");
    assert!(db(ledger::take_dispatch(&p, &execution, crate::now()))
        .unwrap()
        .is_none());
    drop(scope);
}
#[test]
#[cfg(unix)]
fn mismatched_process_identity_is_quarantined_and_never_killed() {
    use std::os::unix::process::CommandExt;
    let p = pool();
    let execution = db(ledger::begin(&p, 1, 100)).unwrap();
    let mut command = Command::new("/usr/bin/python3");
    command
        .args(["-c", "import time;time.sleep(30)"])
        .process_group(0);
    let scope = TurnProcessScope::attach(&mut command);
    let mut child = ReapOnDrop::new(command.spawn().unwrap());
    db(sqlx::query(
        "UPDATE convo_executions SET pgid=?,process_marker=?,identity_hash='unrelated-process' WHERE id=?",
    )
    .bind(i64::from(child.id()))
    .bind(scope.marker())
    .bind(&execution)
    .execute(&p))
    .unwrap();
    assert!(db(recover_execution(&p, 1)).is_err());
    assert!(child.try_wait().unwrap().is_none());
    assert!(db(ledger::blocked(&p, 1)).unwrap());
    drop(child);
    drop(scope);
}
#[test]
#[cfg(unix)]
fn blocked_stdin_write_has_a_deadline() {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new("/usr/bin/python3");
    command
        .args(["-c", "import time;time.sleep(30)"])
        .stdin(Stdio::piped())
        .process_group(0);
    let mut child = ReapOnDrop::new(command.spawn().unwrap());
    let input = child.stdin.take().unwrap();
    nonblocking(&input).unwrap();
    let (_tx, rx) = mpsc::sync_channel(1);
    let mut peer = Peer {
        input: Some(input),
        rx,
        queued: VecDeque::new(),
        next: 0,
        control: Arc::new(Control::new("blocked-pipe".into(), false)),
    };
    let began = Instant::now();
    assert!(peer.send(&json!({"large":"x".repeat(1024*1024)})).is_err());
    assert!(began.elapsed() < Duration::from_secs(7));
    drop(peer);
    drop(child);
}
