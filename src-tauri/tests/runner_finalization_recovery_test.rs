//! Durable Runner finalization resumes after filesystem/Git side effects outpace the journal.

#[path = "support/temp_root.rs"]
mod temp_root;

use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::runner::config::{ExecutionPolicy, RunnerConfig};
use praxis_lib::runner::{self, QueuedTaskRequest};
use praxis_lib::{db, memory, worktree};

static COUNTER: AtomicU32 = AtomicU32::new(0);

#[tokio::test]
async fn approval_history_keeps_dirty_target_failure_after_successful_retry() {
    let f = fixture("approval-history").await;
    std::fs::write(std::path::Path::new(&f.task.worktree_path).join("README.md"), "after\n").unwrap();
    let draft = f.repo.join("user-draft.txt");
    std::fs::write(&draft, "preserve\n").unwrap();
    let failure = runner::finalize_task(&f.config, &f.pool, &f.locks, f.task.id, true, 2_000_000_010).await.unwrap_err();
    assert!(failure.contains("user-draft.txt"));
    assert_eq!(std::fs::read_to_string(&draft).unwrap(), "preserve\n");
    let history = praxis_lib::approval::history(&f.pool, f.task.id).await.unwrap();
    assert_eq!(history.len(), 1); assert_eq!(history[0].stage, "merge");
    assert_eq!(history[0].outcome, "failed");
    std::fs::remove_file(draft).unwrap();
    runner::finalize_task(&f.config, &f.pool, &f.locks, f.task.id, true, 2_000_000_011).await.unwrap();
    let history = praxis_lib::approval::history(&f.pool, f.task.id).await.unwrap();
    assert_eq!(history.len(), 2); assert_eq!(history[0].outcome, "succeeded");
    assert_eq!(history[1].outcome, "failed"); assert_eq!(history[1].stage, "merge");
    assert_finished(&f).await;
    f.pool.close().await;
    let _ = std::fs::remove_dir_all(&f.repo);
    let _ = std::fs::remove_file(&f.db_path);
}

struct Fixture {
    repo: std::path::PathBuf,
    db_path: std::path::PathBuf,
    pool: sqlx::SqlitePool,
    config: RunnerConfig,
    locks: runner::worktree_lock::WorktreeLocks,
    task: db::Task,
}

#[tokio::test]
async fn recovery_completes_an_already_merged_approval_after_original_checkout_switches() {
    let fixture = fixture("already-merged").await;
    let commit = prepare_committed(&fixture).await;
    let task_worktree = task_worktree(&fixture);
    task_worktree.merge_for_approval(&commit).unwrap();
    task_worktree.cleanup_after_finalization().unwrap();
    switch_original_to_feature(&fixture);

    assert_eq!(reconcile(&fixture, 2_000_000_004).await, 1);
    assert_finished(&fixture).await;
    assert_original_checkout_is_preserved(&fixture);
    assert!(!task_worktree.path.exists());
    assert_eq!(reconcile(&fixture, 2_000_000_005).await, 0);
    cleanup(fixture);
}

#[tokio::test]
async fn recovery_merges_saved_base_when_current_branch_already_contains_task_commit() {
    let fixture = fixture("head-only").await;
    let commit = prepare_committed(&fixture).await;
    switch_original_to_feature(&fixture);
    git(&fixture.repo, &["merge", "--no-edit", &commit]);
    let feature_head = git_output(&fixture.repo, &["rev-parse", "HEAD"]);

    assert!(!commit_is_ancestor(
        &fixture.repo,
        &commit,
        "refs/heads/task.base"
    ));
    assert!(commit_is_ancestor(&fixture.repo, &commit, "HEAD"));
    assert_eq!(reconcile(&fixture, 2_000_000_004).await, 1);

    assert_finished(&fixture).await;
    assert_eq!(
        git_output(&fixture.repo, &["rev-parse", "HEAD"]),
        feature_head
    );
    assert_original_checkout_is_preserved(&fixture);
    assert!(!task_worktree(&fixture).path.exists());
    assert_eq!(reconcile(&fixture, 2_000_000_005).await, 0);
    cleanup(fixture);
}

#[tokio::test]
async fn recovery_refuses_to_cleanup_merged_task_with_later_worktree_edits() {
    let fixture = fixture("merged-dirty").await;
    let commit = prepare_committed(&fixture).await;
    let task_worktree = task_worktree(&fixture);
    task_worktree.merge_for_approval(&commit).unwrap();
    sqlx::query("UPDATE runner_finalizations SET state = 'merged' WHERE task_id = ?")
        .bind(fixture.task.id)
        .execute(&fixture.pool)
        .await
        .unwrap();
    let late_change = task_worktree.path.join("late.txt");
    std::fs::write(&late_change, "must survive\n").unwrap();

    let error = runner::finalization::reconcile(
        &fixture.config,
        &fixture.pool,
        &fixture.locks,
        2_000_000_004,
    )
    .await
    .unwrap_err();

    assert!(error
        .to_string()
        .contains("recorded approval checkout changed"));
    assert_eq!(journal_state(&fixture).await, "merged");
    assert_eq!(
        db::get_task(&fixture.pool, fixture.task.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        db::state::FINALIZING
    );
    assert!(task_worktree.path.exists());
    assert_eq!(
        std::fs::read_to_string(late_change).unwrap(),
        "must survive\n"
    );
    assert_eq!(
        git_output(&fixture.repo, &["show", "task.base:README.md"]),
        "after\n"
    );
    cleanup(fixture);
}

async fn fixture(label: &str) -> Fixture {
    let suffix = COUNTER.fetch_add(1, Ordering::SeqCst);
    let repo = temp_root::dir().join(format!(
        "praxis-finalize-recovery-{label}-{}-{suffix}",
        std::process::id()
    ));
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "runner@example.test"]);
    git(&repo, &["config", "user.name", "Runner Test"]);
    std::fs::write(repo.join("README.md"), "before\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-qm", "initial"]);
    git(&repo, &["checkout", "-qb", "task.base"]);
    let db_path = repo.with_extension("sqlite");
    let pool = db::init_pool(db_path.to_str().unwrap()).await.unwrap();
    memory::migrate(&pool).await.unwrap();
    runner::finalization::migrate(&pool).await.unwrap();
    let config = config(&repo);
    let locks = runner::worktree_lock::WorktreeLocks::default();
    let task = runner::create_queued_task(&config, &pool, &locks, request(&repo), 2_000_000_000)
        .await
        .unwrap();
    assert_eq!(task.base, "task.base");
    db::transition_state_with_runner_event(
        &pool,
        task.id,
        db::state::AWAITING_REVIEW,
        2_000_000_001,
        "review",
        None,
    )
    .await
    .unwrap();
    Fixture {
        repo,
        db_path,
        pool,
        config,
        locks,
        task,
    }
}

async fn prepare_committed(fixture: &Fixture) -> String {
    assert!(
        db::claim_review_finalization(&fixture.pool, fixture.task.id, 2_000_000_002)
            .await
            .unwrap()
    );
    // 파일형 투영에는 회수할 원장이 없다 — 블록만 걷어내고 다음 단계로 넘어간다.
    memory::file::retire_task(&fixture.pool, fixture.task.id)
        .await
        .unwrap();
    let task_worktree = task_worktree(fixture);
    std::fs::write(task_worktree.path.join("README.md"), "after\n").unwrap();
    let commit = task_worktree.commit_for_approval().unwrap();
    sqlx::query(
        "INSERT INTO runner_finalizations \
         (task_id, decision, state, commit_sha, created_at, updated_at) \
         VALUES (?, 'approved', 'committed', ?, ?, ?)",
    )
    .bind(fixture.task.id)
    .bind(&commit)
    .bind(2_000_000_002_i64)
    .bind(2_000_000_003_i64)
    .execute(&fixture.pool)
    .await
    .unwrap();
    commit
}

async fn reconcile(fixture: &Fixture, now: i64) -> u64 {
    runner::finalization::reconcile(&fixture.config, &fixture.pool, &fixture.locks, now)
        .await
        .unwrap()
}

async fn assert_finished(fixture: &Fixture) {
    assert_eq!(
        db::get_task(&fixture.pool, fixture.task.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        db::state::DONE
    );
    assert_eq!(journal_state(fixture).await, "completed");
    assert_eq!(
        git_output(&fixture.repo, &["show", "task.base:README.md"]),
        "after\n"
    );
    let approved: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM runner_events WHERE task_id = ? AND kind = 'approved'",
    )
    .bind(fixture.task.id)
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(approved, 1);
}

async fn journal_state(fixture: &Fixture) -> String {
    sqlx::query_scalar("SELECT state FROM runner_finalizations WHERE task_id = ?")
        .bind(fixture.task.id)
        .fetch_one(&fixture.pool)
        .await
        .unwrap()
}

fn task_worktree(fixture: &Fixture) -> worktree::Worktree {
    worktree::Worktree {
        repo: fixture.repo.canonicalize().unwrap(),
        path: fixture.task.worktree_path.clone().into(),
        branch: fixture.task.branch.clone(),
        base: fixture.task.base.clone(),
        base_revision: fixture.task.base_revision.clone(),
    }
}

fn switch_original_to_feature(fixture: &Fixture) {
    git(&fixture.repo, &["checkout", "-qb", "feature"]);
    std::fs::write(fixture.repo.join("caller-note.txt"), "preserve\n").unwrap();
}

fn assert_original_checkout_is_preserved(fixture: &Fixture) {
    assert_eq!(
        git_output(&fixture.repo, &["symbolic-ref", "--quiet", "HEAD"]),
        "refs/heads/feature\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.repo.join("caller-note.txt")).unwrap(),
        "preserve\n"
    );
}

fn commit_is_ancestor(root: &std::path::Path, commit: &str, target: &str) -> bool {
    Command::new("git")
        .current_dir(root)
        .args(["merge-base", "--is-ancestor", commit, target])
        .status()
        .unwrap()
        .success()
}

fn git_output(root: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}

#[tokio::test]
async fn discard_finalization_checkpoints_a_changed_foreign_branch_before_retiring_worktree() {
    let suffix = COUNTER.fetch_add(1, Ordering::SeqCst);
    let repo = temp_root::dir().join(format!(
        "praxis-finalize-discard-{}-{suffix}",
        std::process::id()
    ));
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "runner@example.test"]);
    git(&repo, &["config", "user.name", "Runner Test"]);
    std::fs::write(repo.join("README.md"), "before\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-qm", "initial"]);
    let db_path = repo.with_extension("sqlite");
    let pool = db::init_pool(db_path.to_str().unwrap()).await.unwrap();
    memory::migrate(&pool).await.unwrap();
    runner::finalization::migrate(&pool).await.unwrap();
    let config = config(&repo);
    let locks = runner::worktree_lock::WorktreeLocks::default();
    let task = runner::create_queued_task(&config, &pool, &locks, request(&repo), 2_000_000_000)
        .await
        .unwrap();
    db::transition_state_with_runner_event(
        &pool,
        task.id,
        db::state::AWAITING_REVIEW,
        2_000_000_001,
        "review",
        None,
    )
    .await
    .unwrap();
    let worktree = praxis_lib::worktree::Worktree {
        repo: repo.canonicalize().unwrap(),
        path: task.worktree_path.clone().into(),
        branch: task.branch.clone(),
        base: task.base.clone(),
        base_revision: task.base_revision.clone(),
    };
    git(&worktree.path, &["switch", "-c", "praxis/foreign-current"]);
    std::fs::write(worktree.path.join("evidence.md"), "retained\n").unwrap();

    runner::finalize_task(&config, &pool, &locks, task.id, false, 2_000_000_002)
        .await
        .unwrap();

    assert_eq!(
        db::get_task(&pool, task.id).await.unwrap().unwrap().state,
        db::state::DISCARDED
    );
    let journal: (String, String, Option<String>) = sqlx::query_as(
        "SELECT decision, state, commit_sha FROM runner_finalizations WHERE task_id = ?",
    )
    .bind(task.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(journal.0, "discarded");
    assert_eq!(journal.1, "completed");
    let commit = journal.2.expect("discard must retain a recovery commit");
    assert_eq!(
        git_output(&repo, &["rev-parse", "refs/heads/praxis/foreign-current"]).trim(),
        commit
    );
    assert_eq!(
        git_output(&repo, &["show", &format!("{commit}:evidence.md")]),
        "retained\n"
    );
    git(&repo, &["show-ref", "--verify", &format!("refs/heads/{}", task.branch)]);
    assert!(!worktree.path.exists());
    let _ = std::fs::remove_dir_all(repo);
    let _ = std::fs::remove_file(db_path);
}

fn config(repo: &std::path::Path) -> RunnerConfig {
    RunnerConfig {
        bind: "127.0.0.1:47831".parse().unwrap(),
        repository_roots: vec![repo.canonicalize().unwrap()],
        max_concurrent_tasks: 1,
        execution_policy: ExecutionPolicy::AlwaysApprove,
        pairing_token_file: temp_root::dir().join("unused-runner-token"),
    }
}

fn request(repo: &std::path::Path) -> QueuedTaskRequest {
    QueuedTaskRequest {
        repository: repo.to_string_lossy().into_owned(),
        instruction: "durable finalize".to_string(),
        agent: "claude".to_string(),
        role: "implementer".to_string(),
        model: String::new(),
        reasoning_effort: String::new(),
        mode: "terminal".to_string(),
        goal_contract: None,
        resume_session: None,
        resume_vendor: None,
    }
}

fn git(root: &std::path::Path, args: &[&str]) {
    assert!(Command::new("git")
        .current_dir(root)
        .args(args)
        .status()
        .unwrap()
        .success());
}

fn cleanup(fixture: Fixture) {
    let Fixture {
        pool,
        repo,
        db_path,
        ..
    } = fixture;
    drop(pool);
    let _ = std::fs::remove_dir_all(repo);
    let _ = std::fs::remove_file(db_path);
}
