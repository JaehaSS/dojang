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
        "praxis-local-dirty-recovery-{}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "dirty@example.test"]);
    git(&repo, &["config", "user.name", "Dirty Recovery"]);
    std::fs::write(repo.join("README.md"), "before\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-qm", "initial"]);
    let worktree = worktree::create_plain(&repo, "praxis/dirty-recovery", None).unwrap();
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

async fn insert_task(
    pool: &sqlx::SqlitePool,
    repo: &std::path::Path,
    worktree: &worktree::Worktree,
) -> i64 {
    let task_id = db::insert_task(
        pool,
        repo.to_str().unwrap(),
        &worktree.branch,
        &worktree.base,
        worktree.path.to_str().unwrap(),
        "dirty recovery",
        None,
        None,
        "terminal",
        10,
    )
    .await
    .unwrap();
    db::update_state(pool, task_id, db::state::AWAITING_REVIEW, 11)
        .await
        .unwrap();
    db::set_setting(pool, decision::FLAG_KEY, "true")
        .await
        .unwrap();
    task_id
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

#[tokio::test]
async fn committed_recovery_never_discards_later_worktree_changes() {
    let fixture = fixture().await;
    advance_to_committed(&fixture).await;
    let late = fixture.worktree.path.join("late.txt");
    std::fs::write(&late, "must survive\n").unwrap();

    let error = local_approval::reconcile(&fixture.pool, 30)
        .await
        .unwrap_err();

    assert!(error
        .to_string()
        .contains("recorded approval checkout changed"));
    assert_eq!(
        db::get_task(&fixture.pool, fixture.task_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        db::state::FINALIZING
    );
    assert_eq!(std::fs::read_to_string(late).unwrap(), "must survive\n");
    assert!(!fixture.repo.join("result.txt").exists());
    drop(fixture.pool);
    let _ = std::fs::remove_dir_all(fixture.repo);
    let _ = std::fs::remove_file(fixture.db_path);
}
