#[path = "support/temp_root.rs"]
mod temp_root;

use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::decision::local_approval;
use praxis_lib::{db, decision, memory, worktree};

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Fixture {
    repo: std::path::PathBuf,
    db_path: String,
    pool: sqlx::SqlitePool,
    task_id: i64,
}

fn git(root: &std::path::Path, args: &[&str]) {
    assert!(Command::new("git")
        .current_dir(root)
        .args(args)
        .status()
        .unwrap()
        .success());
}

async fn fixture(label: &str) -> Fixture {
    let sequence = COUNTER.fetch_add(1, Ordering::SeqCst);
    let repo = temp_root::dir().join(format!(
        "praxis-local-boundary-{label}-{}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "boundary@example.test"]);
    git(&repo, &["config", "user.name", "Boundary Test"]);
    std::fs::write(repo.join("README.md"), "before\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-qm", "initial"]);
    let isolated = worktree::create_plain(&repo, &format!("praxis/{label}-{sequence}"), None).unwrap();
    let db_path = repo.with_extension("sqlite").to_string_lossy().into_owned();
    let pool = db::init_pool(&db_path).await.unwrap();
    memory::migrate(&pool).await.unwrap();
    let task_id = insert_task(&pool, &repo, &isolated).await;
    Fixture {
        repo,
        db_path,
        pool,
        task_id,
    }
}

async fn insert_task(
    pool: &sqlx::SqlitePool,
    repo: &std::path::Path,
    isolated: &worktree::Worktree,
) -> i64 {
    let id = db::insert_task(
        pool,
        repo.to_str().unwrap(),
        &isolated.branch,
        &isolated.base,
        isolated.path.to_str().unwrap(),
        "boundary",
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

#[cfg(unix)]
#[tokio::test]
async fn direct_mode_alias_is_rejected_before_claim() {
    let fixture = fixture("direct-alias").await;
    let alias = fixture.repo.with_extension("alias");
    std::os::unix::fs::symlink(&fixture.repo, &alias).unwrap();
    let direct_id = insert_direct_task(&fixture.pool, &fixture.repo, &alias).await;
    db::set_setting(&fixture.pool, decision::FLAG_KEY, "true")
        .await
        .unwrap();
    let task = db::get_task(&fixture.pool, direct_id)
        .await
        .unwrap()
        .unwrap();

    assert!(local_approval::route(&fixture.pool, &task).await.is_err());
    assert_eq!(
        task_state(&fixture.pool, direct_id).await,
        db::state::AWAITING_REVIEW
    );
    let _ = std::fs::remove_file(alias);
    cleanup(fixture);
}

async fn insert_direct_task(
    pool: &sqlx::SqlitePool,
    repo: &std::path::Path,
    alias: &std::path::Path,
) -> i64 {
    let id = db::insert_task(
        pool,
        repo.to_str().unwrap(),
        "main",
        "main",
        alias.to_str().unwrap(),
        "direct alias",
        None,
        None,
        "terminal",
        20,
    )
    .await
    .unwrap();
    db::update_state(pool, id, db::state::AWAITING_REVIEW, 21)
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn ledger_rejects_an_approval_without_task_changes() {
    let fixture = fixture("no-op").await;
    db::set_setting(&fixture.pool, decision::FLAG_KEY, "true")
        .await
        .unwrap();

    let error = local_approval::finalize(&fixture.pool, fixture.task_id, false, 30)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("contains no task changes"));
    assert_eq!(
        task_state(&fixture.pool, fixture.task_id).await,
        db::state::AWAITING_REVIEW
    );
    let decisions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM decision_records")
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    assert_eq!(decisions, 0);
    cleanup(fixture);
}

async fn task_state(pool: &sqlx::SqlitePool, task_id: i64) -> String {
    db::get_task(pool, task_id).await.unwrap().unwrap().state
}

fn cleanup(fixture: Fixture) {
    drop(fixture.pool);
    let _ = std::fs::remove_dir_all(fixture.repo);
    let _ = std::fs::remove_file(fixture.db_path);
}
