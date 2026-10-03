#[path = "support/temp_root.rs"]
mod temp_root;

use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::decision::approval_journal::{self, Stage};
use praxis_lib::decision::local_approval;
use praxis_lib::{db, decision, memory, worktree};

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Fixture {
    repo: std::path::PathBuf,
    db_path: String,
    pool: sqlx::SqlitePool,
    task_id: i64,
    worktree: worktree::Worktree,
}

fn git(root: &std::path::Path, args: &[&str]) {
    assert!(Command::new("git")
        .current_dir(root)
        .args(args)
        .status()
        .unwrap()
        .success());
}

fn repository() -> std::path::PathBuf {
    let sequence = COUNTER.fetch_add(1, Ordering::SeqCst);
    let repo = temp_root::dir().join(format!(
        "praxis-local-recovery-{}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "recovery@example.test"]);
    git(&repo, &["config", "user.name", "Recovery Test"]);
    std::fs::write(repo.join("README.md"), "before\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-qm", "initial"]);
    git(&repo, &["branch", "task.base"]);
    repo
}

async fn fixture() -> Fixture {
    let repo = repository();
    let worktree = worktree::create_plain(&repo, "praxis/recovery", Some("task.base")).unwrap();
    std::fs::write(worktree.path.join("README.md"), "after\n").unwrap();
    let db_path = repo.with_extension("sqlite").to_string_lossy().into_owned();
    let pool = db::init_pool(&db_path).await.unwrap();
    memory::migrate(&pool).await.unwrap();
    let task_id = insert_task(&pool, &repo, &worktree).await;
    Fixture {
        repo,
        db_path,
        pool,
        task_id,
        worktree,
    }
}

async fn insert_task(
    pool: &sqlx::SqlitePool,
    repo: &std::path::Path,
    worktree: &worktree::Worktree,
) -> i64 {
    let id = db::insert_task(
        pool,
        repo.to_str().unwrap(),
        &worktree.branch,
        &worktree.base,
        worktree.path.to_str().unwrap(),
        "recover approval",
        None,
        None,
        "terminal",
        10,
    )
    .await
    .unwrap();
    db::update_state(pool, id, db::state::AWAITING_REVIEW, 11)
        .await
        .unwrap();
    id
}

async fn move_git_ahead_of_journal(fixture: &Fixture) {
    let commit = advance_to_committed(fixture).await;
    fixture.worktree.merge_for_approval(&commit).unwrap();
    fixture.worktree.cleanup_after_finalization().unwrap();
}

async fn advance_to_committed(fixture: &Fixture) -> String {
    approval_journal::claim(&fixture.pool, fixture.task_id, false, 20)
        .await
        .unwrap();
    memory::file::retire_task(&fixture.pool, fixture.task_id)
        .await
        .unwrap();
    approval_journal::stage(
        &fixture.pool,
        fixture.task_id,
        Stage::ProjectionRetired,
        None,
        21,
    )
    .await
    .unwrap();
    let commit = fixture.worktree.commit_for_approval().unwrap();
    approval_journal::stage(
        &fixture.pool,
        fixture.task_id,
        Stage::Committed,
        Some(&commit),
        22,
    )
    .await
    .unwrap();
    commit
}

async fn install_completion_failure(pool: &sqlx::SqlitePool) {
    sqlx::raw_sql(
        "CREATE TRIGGER fail_recovery_ledger BEFORE INSERT ON decision_records \
         BEGIN SELECT RAISE(ABORT, 'forced recovery ledger failure'); END;",
    )
    .execute(pool)
    .await
    .unwrap();
}

async fn assert_failed_recovery(fixture: &Fixture) {
    let failure = local_approval::reconcile(&fixture.pool, 30)
        .await
        .unwrap_err();
    assert!(failure
        .to_string()
        .contains("forced recovery ledger failure"));
    let journal = approval_journal::load(&fixture.pool, fixture.task_id)
        .await
        .unwrap();
    assert_eq!(journal.state, "cleaned");
    assert_eq!(
        journal.failure_code.as_deref(),
        Some("ledger_commit_failed")
    );
    assert_eq!(task_state(fixture).await, db::state::FINALIZING);
}

async fn assert_successful_recovery(fixture: &Fixture) {
    sqlx::query("DROP TRIGGER fail_recovery_ledger")
        .execute(&fixture.pool)
        .await
        .unwrap();
    let recovered = local_approval::reconcile(&fixture.pool, 31).await.unwrap();
    assert_eq!(recovered, 1);
    let repeated = local_approval::reconcile(&fixture.pool, 32).await.unwrap();
    assert_eq!(repeated, 0);
    assert_eq!(task_state(fixture).await, db::state::DONE);
    assert_eq!(
        git_output(&fixture.repo, &["show", "task.base:README.md"]),
        "after\n"
    );
    let decisions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM decision_records")
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    assert_eq!(decisions, 1);
}

async fn task_state(fixture: &Fixture) -> String {
    db::get_task(&fixture.pool, fixture.task_id)
        .await
        .unwrap()
        .unwrap()
        .state
}

fn switch_original_to_feature(fixture: &Fixture) {
    git(&fixture.repo, &["checkout", "-qb", "feature"]);
    std::fs::write(fixture.repo.join("caller-note.txt"), "preserve\n").unwrap();
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

fn commit_is_ancestor(root: &std::path::Path, commit: &str, target: &str) -> bool {
    Command::new("git")
        .current_dir(root)
        .args(["merge-base", "--is-ancestor", commit, target])
        .status()
        .unwrap()
        .success()
}

#[tokio::test]
async fn recovery_converges_when_git_is_ahead_of_the_journal() {
    let fixture = fixture().await;
    move_git_ahead_of_journal(&fixture).await;
    switch_original_to_feature(&fixture);
    assert_eq!(
        git_output(&fixture.repo, &["symbolic-ref", "--quiet", "HEAD"]),
        "refs/heads/feature\n"
    );
    db::set_setting(&fixture.pool, decision::FLAG_KEY, "false")
        .await
        .unwrap();
    assert_eq!(
        local_approval::reconcile(&fixture.pool, 29).await.unwrap(),
        0
    );
    assert_eq!(task_state(&fixture).await, db::state::FINALIZING);
    db::set_setting(&fixture.pool, decision::FLAG_KEY, "true")
        .await
        .unwrap();
    install_completion_failure(&fixture.pool).await;
    assert_failed_recovery(&fixture).await;
    assert_successful_recovery(&fixture).await;
    assert_eq!(
        std::fs::read_to_string(fixture.repo.join("caller-note.txt")).unwrap(),
        "preserve\n"
    );
    cleanup(fixture);
}

#[tokio::test]
async fn recovery_merges_saved_base_when_current_branch_already_contains_task_commit() {
    let fixture = fixture().await;
    let commit = advance_to_committed(&fixture).await;
    switch_original_to_feature(&fixture);
    git(&fixture.repo, &["merge", "--no-edit", &commit]);
    let feature_head = git_output(&fixture.repo, &["rev-parse", "HEAD"]);

    assert!(!commit_is_ancestor(
        &fixture.repo,
        &commit,
        "refs/heads/task.base"
    ));
    assert!(commit_is_ancestor(&fixture.repo, &commit, "HEAD"));
    db::set_setting(&fixture.pool, decision::FLAG_KEY, "true")
        .await
        .unwrap();
    assert_eq!(
        local_approval::reconcile(&fixture.pool, 30).await.unwrap(),
        1
    );

    assert_eq!(
        git_output(&fixture.repo, &["show", "task.base:README.md"]),
        "after\n"
    );
    assert_eq!(
        git_output(&fixture.repo, &["rev-parse", "HEAD"]),
        feature_head
    );
    assert_eq!(
        git_output(&fixture.repo, &["symbolic-ref", "--quiet", "HEAD"]),
        "refs/heads/feature\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.repo.join("caller-note.txt")).unwrap(),
        "preserve\n"
    );
    assert!(!fixture.worktree.path.exists());
    assert_eq!(task_state(&fixture).await, db::state::DONE);
    assert_eq!(
        local_approval::reconcile(&fixture.pool, 31).await.unwrap(),
        0
    );
    cleanup(fixture);
}

fn cleanup(fixture: Fixture) {
    drop(fixture.pool);
    let _ = std::fs::remove_dir_all(fixture.repo);
    let _ = std::fs::remove_file(fixture.db_path);
}
