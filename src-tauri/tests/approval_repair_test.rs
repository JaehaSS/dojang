#[path = "support/temp_root.rs"]
mod temp_root;

use praxis_lib::{
    approval::repair::{self, git, Session},
    db,
    review_ops::ReviewClaims,
    worktree::{self, Worktree},
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU32, Ordering},
};
static NEXT: AtomicU32 = AtomicU32::new(0);

struct Fixture {
    root: PathBuf,
    source: Worktree,
    task: db::Task,
    pool: sqlx::SqlitePool,
    claims: ReviewClaims,
}
impl Fixture {
    async fn new() -> Self {
        let root = temp_root::dir().join(format!(
            "praxis-repair-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&root).unwrap();
        cmd(&root, &["init", "-q", "-b", "dev"]);
        cmd(&root, &["config", "user.email", "test@example.invalid"]);
        cmd(&root, &["config", "user.name", "Repair Test"]);
        cmd(&root, &["config", "commit.gpgsign", "false"]);
        cmd(
            &root,
            &[
                "config",
                "core.hooksPath",
                root.join(".git/hooks").to_str().unwrap(),
            ],
        );
        std::fs::write(root.join("shared.txt"), "base\n").unwrap();
        std::fs::write(root.join(".gitignore"), ".praxis/\n").unwrap();
        cmd(&root, &["add", "."]);
        cmd(&root, &["commit", "-qm", "base"]);
        let source = worktree::create_plain(&root, "praxis/source", Some("dev")).unwrap();
        let pool = db::init_pool(root.join(".praxis/test.sqlite").to_str().unwrap())
            .await
            .unwrap();
        let id = db::insert_task(
            &pool,
            root.to_str().unwrap(),
            &source.branch,
            "dev",
            source.path.to_str().unwrap(),
            "preserve both behaviors",
            Some("claude"),
            None,
            "conversation",
            1,
        )
        .await
        .unwrap();
        db::update_state(&pool, id, db::state::AWAITING_REVIEW, 2)
            .await
            .unwrap();
        let task = db::get_task(&pool, id).await.unwrap().unwrap();
        Self {
            root,
            source,
            task,
            pool,
            claims: ReviewClaims::default(),
        }
    }
    async fn prepare(&self) -> Session {
        repair::prepare(&self.pool, &self.task, self.source.clone(), &self.claims)
            .await
            .unwrap()
    }
    async fn ready(&self, mut session: Session) -> Session {
        let candidate = git::candidate(&session);
        cmd(
            &candidate.path,
            &["merge", "--no-edit", &session.target_sha],
        );
        std::fs::write(candidate.path.join("resolved.txt"), "fixed\n").unwrap();
        candidate.commit_for_approval().unwrap();
        session.state = "ready".into();
        session.commands = vec!["test -f resolved.txt".into()];
        session.checks = vec![repair::Check {
            command: session.commands[0].clone(),
            exit_code: 0,
            tail: String::new(),
        }];
        session.candidate_sha = Some(git::revision(&candidate.path, "HEAD").unwrap());
        session.candidate_fingerprint = Some(git::candidate_fingerprint(&session).unwrap());
        self.save(&session).await;
        session
    }
    async fn save(&self, session: &Session) {
        db::append_event(
            &self.pool,
            self.task.id,
            "approval_repair",
            Some(&serde_json::to_string(session).unwrap()),
            3,
        )
        .await
        .unwrap();
    }
    async fn close(self) {
        self.pool.close().await;
        std::fs::remove_dir_all(self.root).unwrap();
    }
}
fn cmd(path: &Path, args: &[&str]) -> String {
    git::git(path, args).unwrap()
}

#[tokio::test]
async fn snapshot_and_accept_preserve_original_index_refs_and_files() {
    let f = Fixture::new().await;
    std::fs::write(f.source.path.join("shared.txt"), "staged\n").unwrap();
    cmd(&f.source.path, &["add", "shared.txt"]);
    std::fs::write(f.source.path.join("shared.txt"), "unstaged\n").unwrap();
    std::fs::write(f.source.path.join("new file.txt"), "draft\n").unwrap();
    let before = git::fingerprint(&f.source.path).unwrap();
    let target = git::revision(&f.root, "HEAD").unwrap();
    let session = f.prepare().await;
    assert_eq!(
        std::fs::read_to_string(Path::new(&session.candidate_path).join("shared.txt")).unwrap(),
        "unstaged\n"
    );
    assert_eq!(cmd(&f.source.path, &["show", ":shared.txt"]), "staged\n");
    assert_eq!(git::fingerprint(&f.source.path).unwrap(), before);
    assert!(repair::accept(&f.pool, &f.task, &f.claims, &session.id)
        .await
        .is_err());
    let session = f.ready(session).await;
    db::upsert_evidence(
        &f.pool,
        f.task.id,
        "old build",
        0,
        "old test",
        0,
        1,
        0,
        true,
        2,
    )
    .await
    .unwrap();
    let accepted = repair::accept(&f.pool, &f.task, &f.claims, &session.id)
        .await
        .unwrap();
    assert_eq!(accepted.state, "accepted");
    assert!(
        !db::get_evidence(&f.pool, f.task.id)
            .await
            .unwrap()
            .unwrap()
            .ready
    );
    let current = db::get_task(&f.pool, f.task.id).await.unwrap().unwrap();
    assert_eq!(current.worktree_path, session.candidate_path);
    assert_eq!(
        current.base_revision.as_deref(),
        Some(session.target_sha.as_str())
    );
    assert_eq!(current.state, db::state::AWAITING_REVIEW);
    assert_eq!(git::fingerprint(&f.source.path).unwrap(), before);
    assert_eq!(git::revision(&f.root, "HEAD").unwrap(), target);
    assert!(f.source.path.is_dir());
    assert!(repair::accept(&f.pool, &current, &f.claims, &session.id)
        .await
        .is_err());
    f.close().await;
}

#[tokio::test]
async fn adoption_rejects_changed_source_target_or_verified_candidate() {
    for changed in ["source", "target", "candidate"] {
        let f = Fixture::new().await;
        let session = f.ready(f.prepare().await).await;
        match changed {
            "source" => std::fs::write(f.source.path.join("late.txt"), "preserve").unwrap(),
            "target" => {
                std::fs::write(f.root.join("late.txt"), "target").unwrap();
                cmd(&f.root, &["add", "late.txt"]);
                cmd(&f.root, &["commit", "-qm", "advanced"]);
            }
            _ => std::fs::write(
                Path::new(&session.candidate_path).join("resolved.txt"),
                "changed",
            )
            .unwrap(),
        }
        assert!(
            repair::accept(&f.pool, &f.task, &f.claims, &session.id)
                .await
                .is_err(),
            "{changed}"
        );
        assert_eq!(
            db::get_task(&f.pool, f.task.id)
                .await
                .unwrap()
                .unwrap()
                .worktree_path,
            f.task.worktree_path
        );
        assert!(f.source.path.exists());
        f.close().await;
    }
}

#[tokio::test]
async fn interrupted_session_and_unconfigured_checks_are_never_ready() {
    let f = Fixture::new().await;
    let mut session = f.prepare().await;
    session.state = "checking".into();
    f.save(&session).await;
    assert_eq!(
        repair::observed_status(&f.pool, f.task.id, &f.claims)
            .await
            .unwrap()
            .unwrap()
            .state,
        "needs_attention"
    );
    let session = f.prepare().await;
    let result = repair::run(
        f.pool.clone(),
        f.task.clone(),
        f.claims.clone(),
        session.id,
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.state, "needs_attention");
    assert_eq!(result.attempts, 0);
    assert!(result.error.unwrap().contains("검사 명령"));
    f.close().await;
}

#[cfg(unix)]
#[tokio::test]
async fn automated_agent_loop_checks_real_result_and_stops_after_two_attempts() {
    use std::os::unix::fs::PermissionsExt;
    // This integration test process alone changes PATH; all steps are sequential.
    let old = std::env::var_os("PATH").unwrap();
    for mode in [
        "success",
        "failed_checks",
        "decision",
        "cancel",
        "hook_failure",
        "check_mutation",
    ] {
        let f = Fixture::new().await;
        std::fs::create_dir_all(f.source.path.join(".praxis")).unwrap();
        std::fs::write(
            f.source.path.join(".praxis/validate.toml"),
            if mode == "check_mutation" {
                "test = \"echo changed >> shared.txt\"\n"
            } else {
                "test = \"test -f resolved.txt\"\n"
            },
        )
        .unwrap();
        std::fs::write(f.source.path.join("shared.txt"), "task\n").unwrap();
        cmd(&f.source.path, &["commit", "-am", "task"]);
        std::fs::write(f.root.join("shared.txt"), "target\n").unwrap();
        cmd(&f.root, &["commit", "-am", "target"]);
        let before = git::fingerprint(&f.source.path).unwrap();
        let session = f.prepare().await;
        if mode == "hook_failure" {
            let hook = f.root.join(".git/hooks/pre-commit");
            std::fs::write(
                &hook,
                "#!/bin/sh\necho fixture-hook-rejection >&2\nexit 1\n",
            )
            .unwrap();
            std::fs::set_permissions(hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let bin = f.root.join(".praxis/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let agent = bin.join("claude");
        let action = if mode == "success" {
            "printf 'both behaviors\\n' > resolved.txt\n"
        } else {
            ""
        };
        let verdict = if mode == "decision" {
            "needs_decision"
        } else {
            "ready"
        };
        std::fs::write(&agent, format!("#!/bin/sh\ngit merge --no-edit {} >/dev/null 2>&1 || true\nprintf 'task and target\\n' > shared.txt\ngit add shared.txt\n{action}printf 'PRAXIS_REPAIR_RESULT: {verdict}\\n'\n", session.target_sha)).unwrap();
        std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755)).unwrap();
        if mode == "cancel" {
            std::fs::write(&agent, "#!/bin/sh\nexec sleep 30\n").unwrap();
        }
        let path =
            std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&old))).unwrap();
        std::env::set_var("PATH", path);
        let pool = f.pool.clone();
        let task = f.task.clone();
        let claims = f.claims.clone();
        let session_id = session.id.clone();
        let worker =
            tokio::spawn(async move { repair::run(pool, task, claims, session_id, None).await });
        if mode == "cancel" {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                while !praxis_lib::runner::review_process::task_is_fenced(&f.pool, f.task.id)
                    .await
                    .unwrap()
                {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
            repair::cancel(&f.pool, f.task.id, &session.id)
                .await
                .unwrap();
        }
        let result = worker.await.unwrap().unwrap();
        std::env::set_var("PATH", &old);
        if mode == "success" {
            assert_eq!(result.state, "ready", "{:?}", result.error);
            assert_eq!(result.attempts, 1);
            assert_eq!(result.checks[0].exit_code, 0);
            assert!(result.diff.contains("resolved.txt"));
        } else {
            assert_eq!(result.state, "needs_attention");
            if mode == "hook_failure" {
                assert!(result
                    .error
                    .as_deref()
                    .unwrap()
                    .contains("fixture-hook-rejection"));
            }
            if mode == "check_mutation" {
                assert!(result.error.as_deref().unwrap().contains("검사 중 후보"));
            }
            assert_eq!(
                result.attempts,
                if mode == "decision" || mode == "cancel" {
                    1
                } else {
                    2
                }
            );
        }
        assert_eq!(git::fingerprint(&f.source.path).unwrap(), before);
        assert!(
            !praxis_lib::runner::review_process::task_is_fenced(&f.pool, f.task.id)
                .await
                .unwrap()
        );
        f.close().await;
    }
}

#[tokio::test]
async fn preparation_copies_declared_environment_and_excludes_generated_mcp() {
    let f = Fixture::new().await;
    std::fs::write(f.source.path.join(".worktreeinclude"), ".env\n").unwrap();
    std::fs::write(
        f.source.path.join(".gitignore"),
        ".praxis/\n.env\n.mcp.json\n",
    )
    .unwrap();
    std::fs::write(f.source.path.join(".env"), "FIXTURE_VALUE=local\n").unwrap();
    std::fs::write(f.source.path.join(".mcp.json"), "generated").unwrap();
    db::append_event(&f.pool, f.task.id, "mcp_generated", None, 3)
        .await
        .unwrap();
    let before = git::fingerprint(&f.source.path).unwrap();
    let session = f.prepare().await;
    assert_eq!(
        std::fs::read_to_string(Path::new(&session.candidate_path).join(".env")).unwrap(),
        "FIXTURE_VALUE=local\n"
    );
    assert!(!Path::new(&session.candidate_path)
        .join(".mcp.json")
        .exists());
    assert_eq!(git::fingerprint(&f.source.path).unwrap(), before);
    let session = f.ready(session).await;
    std::fs::write(
        Path::new(&session.candidate_path).join(".env"),
        "FIXTURE_VALUE=changed\n",
    )
    .unwrap();
    assert!(repair::accept(&f.pool, &f.task, &f.claims, &session.id)
        .await
        .is_err());
    f.close().await;
}

#[tokio::test]
async fn existing_process_schema_upgrades_and_rejects_wrong_phase() {
    use praxis_lib::runner::review_process::{self, ReviewOperation, ReviewPhase};
    let f = Fixture::new().await;
    let mut tx = f.pool.begin().await.unwrap();
    sqlx::query("DROP TRIGGER review_process_receipts_valid")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER review_process_receipts_valid BEFORE INSERT ON review_process_receipts WHEN NEW.operation != 'verify' BEGIN SELECT RAISE(ABORT,'old validator'); END").execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    review_process::migrate(&f.pool).await.unwrap();
    review_process::migrate(&f.pool).await.unwrap();
    assert!(review_process::register(
        &f.pool,
        f.task.id,
        ReviewOperation::Repair,
        ReviewPhase::VerifyBuild,
        1234,
        &"a".repeat(64),
        4
    )
    .await
    .is_err());
    review_process::register(
        &f.pool,
        f.task.id,
        ReviewOperation::Repair,
        ReviewPhase::RepairAgent,
        1234,
        &"a".repeat(64),
        4,
    )
    .await
    .unwrap();
    assert!(review_process::task_is_fenced(&f.pool, f.task.id)
        .await
        .unwrap());
    f.close().await;
}

#[tokio::test]
async fn adoption_requires_exact_successful_checks() {
    let f = Fixture::new().await;
    let verified = f.ready(f.prepare().await).await;
    for mode in ["missing", "failed", "different"] {
        let mut session = verified.clone();
        match mode {
            "missing" => session.checks.clear(),
            "failed" => session.checks[0].exit_code = 1,
            _ => session.checks[0].command = "true".into(),
        }
        f.save(&session).await;
        assert!(
            repair::accept(&f.pool, &f.task, &f.claims, &session.id)
                .await
                .is_err(),
            "{mode}"
        );
    }
    f.close().await;
}

#[cfg(unix)]
#[tokio::test]
async fn candidate_and_management_symlinks_are_rejected() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new().await;
    let mut session = f.prepare().await;
    let alias = f.root.join(".praxis/worktrees/alias");
    symlink(&session.candidate_path, &alias).unwrap();
    session.candidate_path = alias.to_string_lossy().into_owned();
    assert!(git::assert_candidate(&session).is_err());
    let alias_root = f.root.join("alias");
    symlink(f.root.join(".praxis"), &alias_root).unwrap();
    assert!(git::managed_dir(&f.root, &["alias", "other"]).is_err());
    f.close().await;
}
