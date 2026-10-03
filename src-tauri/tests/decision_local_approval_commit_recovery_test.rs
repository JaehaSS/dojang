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

async fn fixture() -> Fixture {
    let sequence = COUNTER.fetch_add(1, Ordering::SeqCst);
    let repo = temp_root::dir().join(format!(
        "praxis-local-commit-recovery-{}-{sequence}",
        std::process::id()
    ));
    initialize_repository(&repo);
    let worktree = worktree::create_plain(&repo, "praxis/commit-recovery", None).unwrap();
    std::fs::write(worktree.path.join("result.txt"), "approved\n").unwrap();
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

fn initialize_repository(repo: &std::path::Path) {
    std::fs::create_dir_all(repo).unwrap();
    git(repo, &["init", "-q"]);
    git(
        repo,
        &["config", "user.email", "commit-recovery@example.test"],
    );
    git(repo, &["config", "user.name", "Commit Recovery Test"]);
    std::fs::write(repo.join("README.md"), "before\n").unwrap();
    git(repo, &["add", "README.md"]);
    git(repo, &["commit", "-qm", "initial"]);
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
        "commit crash recovery",
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
    db::set_setting(pool, decision::FLAG_KEY, "true")
        .await
        .unwrap();
    id
}

async fn create_unrecorded_commit(fixture: &Fixture) -> String {
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
    fixture.worktree.commit_for_approval().unwrap()
}

async fn assert_recovered(fixture: &Fixture, commit: &str) {
    let journal = approval_journal::load(&fixture.pool, fixture.task_id)
        .await
        .unwrap();
    assert_eq!(journal.state, "completed");
    assert_eq!(journal.commit_sha.as_deref(), Some(commit));
    let recorded: String = sqlx::query_scalar(
        "SELECT artifact_ref FROM decision_artifact_links WHERE artifact_kind = 'git_commit'",
    )
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(recorded, commit);
    assert_eq!(
        std::fs::read_to_string(fixture.repo.join("result.txt")).unwrap(),
        "approved\n"
    );
}

#[tokio::test]
async fn recovery_reuses_a_commit_created_before_its_stage_was_recorded() {
    let fixture = fixture().await;
    let commit = create_unrecorded_commit(&fixture).await;
    assert_eq!(
        approval_journal::load(&fixture.pool, fixture.task_id)
            .await
            .unwrap()
            .commit_sha,
        None
    );

    assert_eq!(
        local_approval::reconcile(&fixture.pool, 30).await.unwrap(),
        1
    );

    assert_recovered(&fixture, &commit).await;
    cleanup(fixture);
}

fn cleanup(fixture: Fixture) {
    drop(fixture.pool);
    let _ = std::fs::remove_dir_all(fixture.repo);
    let _ = std::fs::remove_file(fixture.db_path);
}
