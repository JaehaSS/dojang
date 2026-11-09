use super::pr_status::{normalize_check_for_test, snapshot_with_bins};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn repo(origin: &str) -> PathBuf {
    let number = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = crate::testtmp::dir().join(format!("pr-status-{number}"));
    std::fs::create_dir_all(&path).unwrap();
    git(&path, &["init", "-q", "-b", "work"]);
    git(&path, &["config", "user.email", "test@example.com"]);
    git(&path, &["config", "user.name", "Test"]);
    std::fs::write(path.join("file"), "one").unwrap();
    git(&path, &["add", "file"]);
    git(&path, &["commit", "-qm", "initial"]);
    git(&path, &["remote", "add", "origin", origin]);
    path
}

#[cfg(unix)]
fn gh_stub(script_body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let number = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = crate::testtmp::dir().join(format!("pr-status-gh-{number}.sh"));
    std::fs::write(&path, script_body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn pr_json(head_repository: &str, head_owner: &str, branch: &str) -> String {
    json!([{
        "number": 17,
        "url": "https://github.com/acme/widgets/pull/17",
        "title": "Fix checks",
        "state": "OPEN",
        "isDraft": false,
        "headRefOid": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "headRefName": branch,
        "baseRefName": "main",
        "reviewDecision": null,
        "reviewRequests": [{"login": "reviewer"}],
        "statusCheckRollup": [{"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"SUCCESS","detailsUrl":"https://ci.example.test/job"}],
        "updatedAt": "2026-10-26T12:00:00Z",
        "headRepositoryOwner": {"login": head_owner},
        "headRepository": {"nameWithOwner": head_repository}
    }]).to_string()
}

fn pr_json_at(number: u64, state: &str, updated_at: &str) -> serde_json::Value {
    let mut value: serde_json::Value =
        serde_json::from_str(&pr_json("acme/widgets", "acme", "work")).unwrap();
    let pr = value[0].as_object_mut().unwrap();
    pr.insert("number".into(), json!(number));
    pr.insert(
        "url".into(),
        json!(format!("https://github.com/acme/widgets/pull/{number}")),
    );
    pr.insert("state".into(), json!(state));
    pr.insert("updatedAt".into(), json!(updated_at));
    value
}

#[test]
fn check_normalization_keeps_unknown_and_never_turns_none_into_pass() {
    let pending = normalize_check_for_test(
        json!({"__typename":"CheckRun","name":"build","status":"IN_PROGRESS","conclusion":null}),
    )
    .unwrap();
    assert_eq!(pending.status, "pending");
    let unknown = normalize_check_for_test(
        json!({"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":null}),
    )
    .unwrap();
    assert_eq!(unknown.status, "unknown");
    let cancelled = normalize_check_for_test(json!({"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":"CANCELLED"})).unwrap();
    assert_eq!(cancelled.status, "cancelled");
    let blank_conclusion = normalize_check_for_test(
        json!({"__typename":"CheckRun","name":"build","status":"QUEUED","conclusion":""}),
    )
    .unwrap();
    assert_eq!(blank_conclusion.status, "pending");
    assert!(normalize_check_for_test(json!({"__typename":"FutureCheck"})).is_err());
}

#[cfg(unix)]
#[test]
fn successful_empty_list_is_no_pr_but_query_failure_is_unavailable() {
    let worktree = repo("https://github.com/ACME/Widgets.git");
    let valid = pr_json("acme/widgets", "acme", "work");
    let valid_stub = gh_stub(&format!(
        "#!/bin/sh\nprintf '%s\\n' '{}'\n",
        valid.replace('\'', "'\\''")
    ));
    let ready = snapshot_with_bins(&worktree, "work", &valid_stub, Duration::from_secs(1), 4096);
    assert_eq!(ready.status, "ready");
    let ready_pr = ready.pr.unwrap();
    assert_eq!(ready_pr.state, "open");
    assert_eq!(ready_pr.checks[0].status, "pass");
    let empty = gh_stub("#!/bin/sh\nprintf '[]\\n'\n");
    let no_pr = snapshot_with_bins(&worktree, "work", &empty, Duration::from_secs(1), 4096);
    assert_eq!(no_pr.status, "no_pr");
    assert_eq!(no_pr.repository.as_deref(), Some("ACME/Widgets"));
    let failed = gh_stub("#!/bin/sh\necho 'network unreachable' >&2\nexit 1\n");
    let unavailable = snapshot_with_bins(&worktree, "work", &failed, Duration::from_secs(1), 4096);
    assert_eq!(unavailable.status, "unavailable");
    assert!(unavailable.message.unwrap().contains("network unreachable"));
    let auth = gh_stub("#!/bin/sh\necho 'Run gh auth login first' >&2\nexit 1\n");
    let unauthenticated =
        snapshot_with_bins(&worktree, "work", &auth, Duration::from_secs(1), 4096);
    assert_eq!(unauthenticated.status, "unavailable");
    assert!(unauthenticated.message.unwrap().contains("인증"));
    let malformed = gh_stub("#!/bin/sh\nprintf '{bad json'\n");
    assert_eq!(
        snapshot_with_bins(&worktree, "work", &malformed, Duration::from_secs(1), 4096).status,
        "unavailable"
    );
}

#[cfg(unix)]
#[test]
fn newest_closed_or_merged_pr_is_selected_when_no_open_pr_exists() {
    let worktree = repo("https://github.com/acme/widgets.git");
    let old = pr_json_at(3, "CLOSED", "2026-10-25T12:00:00+09:00");
    let newest = pr_json_at(4, "MERGED", "2026-10-25T04:00:01Z");
    let body = json!([old[0].clone(), newest[0].clone()]).to_string();
    let script = gh_stub(&format!(
        "#!/bin/sh\nprintf '%s\\n' '{}'\n",
        body.replace('\'', "'\\''")
    ));
    let snapshot = snapshot_with_bins(&worktree, "work", &script, Duration::from_secs(1), 4096);
    assert_eq!(snapshot.status, "ready");
    let pr = snapshot.pr.unwrap();
    assert_eq!(pr.number, 4);
    assert_eq!(pr.state, "merged");

    let capped_history: Vec<_> = (1..=30)
        .map(|number| {
            pr_json_at(number, "CLOSED", &format!("2026-10-25T04:00:{number:02}Z"))[0].clone()
        })
        .collect();
    let body = serde_json::to_string(&capped_history).unwrap();
    let script = gh_stub(&format!(
        "#!/bin/sh\nprintf '%s\\n' '{}'\n",
        body.replace('\'', "'\\''")
    ));
    let capped = snapshot_with_bins(&worktree, "work", &script, Duration::from_secs(1), 65536);
    assert_eq!(capped.status, "unavailable");
    assert!(capped.pr.is_none());
    assert!(capped.message.unwrap().contains("조회 상한"));
}

#[cfg(unix)]
#[test]
fn fork_or_head_branch_mismatch_is_unavailable() {
    let worktree = repo("git@github.com:acme/widgets.git");
    let fork = pr_json("fork/widgets", "fork", "work");
    let fork_stub = gh_stub(&format!(
        "#!/bin/sh\nprintf '%s\\n' '{}'\n",
        fork.replace('\'', "'\\''")
    ));
    assert_eq!(
        snapshot_with_bins(&worktree, "work", &fork_stub, Duration::from_secs(1), 4096).status,
        "unavailable"
    );
    let mismatch = pr_json("acme/widgets", "acme", "other");
    let mismatch_stub = gh_stub(&format!(
        "#!/bin/sh\nprintf '%s\\n' '{}'\n",
        mismatch.replace('\'', "'\\''")
    ));
    assert_eq!(
        snapshot_with_bins(
            &worktree,
            "work",
            &mismatch_stub,
            Duration::from_secs(1),
            4096
        )
        .status,
        "unavailable"
    );
}

#[cfg(unix)]
#[test]
fn branch_switch_during_query_is_rejected() {
    let worktree = repo("https://github.com/acme/widgets.git");
    git(&worktree, &["branch", "other"]);
    let body = pr_json("acme/widgets", "acme", "work");
    let script = gh_stub(&format!(
        "#!/bin/sh\ngit checkout -q other\nprintf '%s\\n' '{}'\n",
        body.replace('\'', "'\\''")
    ));
    let snapshot = snapshot_with_bins(&worktree, "work", &script, Duration::from_secs(1), 4096);
    assert_eq!(snapshot.status, "unavailable");
    assert!(snapshot.message.unwrap().contains("checkout"));
}

#[cfg(unix)]
#[test]
fn origin_or_commit_change_during_query_is_rejected() {
    for (mutation, expected) in [
        (
            "git remote set-url origin https://github.com/other/widgets.git",
            "origin",
        ),
        ("git commit --allow-empty -qm changed", "HEAD"),
    ] {
        let worktree = repo("https://github.com/acme/widgets.git");
        let body = pr_json("acme/widgets", "acme", "work");
        let script = gh_stub(&format!(
            "#!/bin/sh\n{mutation}\nprintf '%s\\n' '{}'\n",
            body.replace('\'', "'\\''")
        ));
        let snapshot = snapshot_with_bins(&worktree, "work", &script, Duration::from_secs(1), 4096);
        assert_eq!(snapshot.status, "unavailable");
        assert!(snapshot.pr.is_none());
        assert!(snapshot.message.unwrap().contains(expected));
    }
}

#[cfg(unix)]
#[test]
fn wrong_origin_is_unsupported_without_running_gh() {
    let worktree = repo("https://github.example/acme/widgets.git");
    let missing = worktree.join("never-run");
    let snapshot = snapshot_with_bins(&worktree, "work", &missing, Duration::from_secs(1), 4096);
    assert_eq!(snapshot.status, "unsupported");
}

#[cfg(unix)]
#[test]
fn timeout_and_output_cap_kill_process_group() {
    let worktree = repo("https://github.com/acme/widgets.git");
    let marker = crate::testtmp::dir().join(format!(
        "pr-status-child-{}",
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let timeout = gh_stub(&format!(
        "#!/bin/sh\n(sleep 0.2; touch '{}') &\nsleep 10\n",
        marker.display()
    ));
    let timed_out =
        snapshot_with_bins(&worktree, "work", &timeout, Duration::from_millis(80), 4096);
    assert_eq!(timed_out.status, "unavailable");
    std::thread::sleep(Duration::from_millis(400));
    assert!(!marker.exists(), "process-group child survived timeout");

    let inherited_marker = crate::testtmp::dir().join(format!(
        "pr-status-inherited-child-{}",
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let inherited = gh_stub(&format!(
        "#!/bin/sh\n(sleep 0.2; touch '{}') &\nexit 0\n",
        inherited_marker.display()
    ));
    let inherited_result = snapshot_with_bins(
        &worktree,
        "work",
        &inherited,
        Duration::from_millis(80),
        4096,
    );
    assert_eq!(inherited_result.status, "unavailable");
    std::thread::sleep(Duration::from_millis(400));
    assert!(
        !inherited_marker.exists(),
        "inherited pipe child survived timeout"
    );

    let closed_pipes = gh_stub("#!/bin/sh\nexec >/dev/null 2>&1\nsleep 10\n");
    let started = Instant::now();
    let closed_pipes_result = snapshot_with_bins(
        &worktree,
        "work",
        &closed_pipes,
        Duration::from_millis(80),
        4096,
    );
    assert_eq!(closed_pipes_result.status, "unavailable");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "closed-pipe child exceeded bounded timeout"
    );

    let noisy = gh_stub("#!/bin/sh\nyes x | head -c 8192\nsleep 10\n");
    let capped = snapshot_with_bins(&worktree, "work", &noisy, Duration::from_secs(1), 128);
    assert_eq!(capped.status, "unavailable");
    assert!(capped.message.unwrap().contains("출력 제한"));
}
