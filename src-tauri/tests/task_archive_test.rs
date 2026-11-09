#[path = "support/temp_root.rs"]
mod temp_root;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::db;
use praxis_lib::github;
use praxis_lib::runner::review_process::{self, ReviewOperation, ReviewPhase};
use praxis_lib::task_archive;
use praxis_lib::task_specs;

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Fixture {
    pool: sqlx::SqlitePool,
    task_id: i64,
    worktree: PathBuf,
}

#[tokio::test]
async fn archive_persists_metadata_and_keeps_worktree_changes() {
    let fixture = fixture(true).await;
    std::fs::write(fixture.worktree.join("result.md"), "result\n").unwrap();
    sqlx::query(
        "INSERT INTO task_result_references(task_id,result_revision,kind,relative_path,created_at) \
         VALUES(?,1,'report','result.md',10)",
    )
    .bind(fixture.task_id)
    .execute(&fixture.pool)
    .await
    .unwrap();
    github::migrate(&fixture.pool).await.unwrap();
    github::set_pr_ref(&fixture.pool, fixture.task_id, "https://example.test/pr/7")
        .await
        .unwrap();

    let expected_head = git(&fixture.worktree, &["rev-parse", "HEAD"]);
    std::fs::write(fixture.worktree.join("tracked.txt"), "dirty\n").unwrap();
    let status_before_archive = git(&fixture.worktree, &["status", "--porcelain"]);
    let archive = task_archive::archive(&fixture.pool, fixture.task_id, 20)
        .await
        .unwrap();

    assert_eq!(archive.archived_at, 20);
    assert_eq!(archive.branch, "archive-branch");
    assert_eq!(
        archive.head_revision.as_deref(),
        Some(expected_head.as_str())
    );
    assert_eq!(
        archive.result_path,
        Some(
            fixture
                .worktree
                .join("result.md")
                .to_string_lossy()
                .into_owned()
        )
    );
    assert_eq!(archive.pr_url.as_deref(), Some("https://example.test/pr/7"));
    assert!(archive.worktree_exists);
    assert_eq!(
        git(&fixture.worktree, &["status", "--porcelain"]),
        status_before_archive
    );

    assert_eq!(
        std::fs::read(fixture.worktree.join("tracked.txt")).unwrap(),
        b"dirty\n"
    );
    assert_eq!(
        std::fs::read(fixture.worktree.join("result.md")).unwrap(),
        b"result\n"
    );

    let repeated = task_archive::archive(&fixture.pool, fixture.task_id, 99)
        .await
        .unwrap();
    assert_eq!(repeated.archived_at, 20);
    assert_eq!(
        task_archive::list(&fixture.pool).await.unwrap()[&fixture.task_id],
        archive
    );
}

#[tokio::test]
async fn archive_rejects_live_state_and_durable_leases() {
    let fixture = fixture(true).await;
    db::update_state(&fixture.pool, fixture.task_id, db::state::RUNNING, 2)
        .await
        .unwrap();
    assert!(task_archive::archive(&fixture.pool, fixture.task_id, 3)
        .await
        .is_err());

    db::update_state(
        &fixture.pool,
        fixture.task_id,
        db::state::AWAITING_REVIEW,
        4,
    )
    .await
    .unwrap();
    db::set_convo_pgid(&fixture.pool, fixture.task_id, Some(33))
        .await
        .unwrap();
    assert!(task_archive::archive(&fixture.pool, fixture.task_id, 5)
        .await
        .is_err());
    db::set_convo_pgid(&fixture.pool, fixture.task_id, None)
        .await
        .unwrap();
    review_process::register(
        &fixture.pool,
        fixture.task_id,
        ReviewOperation::Verify,
        ReviewPhase::VerifyBuild,
        401,
        &"a".repeat(64),
        6,
    )
    .await
    .unwrap();
    assert!(task_archive::archive(&fixture.pool, fixture.task_id, 7)
        .await
        .is_err());
}

#[tokio::test]
async fn opening_pending_result_review_keeps_archive_eligible() {
    let fixture = fixture(true).await;
    sqlx::query(
        "INSERT INTO result_review_rounds \
         (id,task_id,source_id,result_revision,completion_epoch,revision,state,criteria,active,created_at) \
         VALUES('archive-review', ?, 'source', 1, 1, 1, 'pending', '[]', 1, 3)",
    )
    .bind(fixture.task_id)
    .execute(&fixture.pool)
    .await
    .unwrap();
    let archive = task_archive::archive(&fixture.pool, fixture.task_id, 4)
        .await
        .unwrap();
    assert_eq!(archive.archived_at, 4);
    let review: (String, i64, i64) = sqlx::query_as(
        "SELECT state,active,revision FROM result_review_rounds WHERE id = 'archive-review'",
    )
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(review, ("pending".into(), 1, 1));
}

#[tokio::test]
async fn state_change_restores_and_task_delete_cascades_archive() {
    let fixture = fixture(true).await;
    task_archive::archive(&fixture.pool, fixture.task_id, 20)
        .await
        .unwrap();
    db::update_state(&fixture.pool, fixture.task_id, db::state::RUNNING, 21)
        .await
        .unwrap();
    assert!(!task_archive::list(&fixture.pool)
        .await
        .unwrap()
        .contains_key(&fixture.task_id));

    db::update_state(&fixture.pool, fixture.task_id, db::state::DONE, 22)
        .await
        .unwrap();
    task_archive::archive(&fixture.pool, fixture.task_id, 23)
        .await
        .unwrap();
    sqlx::query("DELETE FROM tasks WHERE id = ?")
        .bind(fixture.task_id)
        .execute(&fixture.pool)
        .await
        .unwrap();
    assert!(task_archive::list(&fixture.pool).await.unwrap().is_empty());
}

#[tokio::test]
async fn queued_side_question_unarchives_an_archived_task() {
    let fixture = fixture(true).await;
    task_archive::archive(&fixture.pool, fixture.task_id, 20)
        .await
        .unwrap();
    let thread_id = sqlx::query(
        "INSERT INTO side_question_threads(task_id,generation,provider,model,created_at) \
         VALUES(?,0,'claude','test',21)",
    )
    .bind(fixture.task_id)
    .execute(&fixture.pool)
    .await
    .unwrap()
    .last_insert_rowid();
    sqlx::query(
        "INSERT INTO side_question_turns \
         (thread_id,task_id,request_id,payload_hash,generation,question,contexts,state,created_at) \
         VALUES(?,?, 'archive-trigger', 'hash', 0, 'question', '[]', 'queued', 22)",
    )
    .bind(thread_id)
    .bind(fixture.task_id)
    .execute(&fixture.pool)
    .await
    .unwrap();
    assert!(!task_archive::is_archived(&fixture.pool, fixture.task_id)
        .await
        .unwrap());
}

#[tokio::test]
async fn absent_worktree_is_archived_without_a_head() {
    let root = temp_root::dir().join(format!(
        "task-archive-missing-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let database = root.with_extension("sqlite");
    let pool = db::init_pool(database.to_str().unwrap()).await.unwrap();
    let task_id = db::insert_task(
        &pool,
        "/repo",
        "missing-branch",
        "main",
        root.to_str().unwrap(),
        "archive",
        None,
        None,
        "terminal",
        1,
    )
    .await
    .unwrap();
    db::update_state(&pool, task_id, db::state::DONE, 2)
        .await
        .unwrap();
    db::append_convo_event(&pool, task_id, r#"{"kind":"assistant","text":"kept"}"#, 3)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO task_spec_snapshots \
         (task_id,relative_path,title,content_hash,byte_length,created_at,base_revision,content) \
         VALUES(?, 'evidence.md', 'Evidence', 'hash', 8, 3, NULL, 'preserved')",
    )
    .bind(task_id)
    .execute(&pool)
    .await
    .unwrap();

    let archive = task_archive::archive(&pool, task_id, 4).await.unwrap();
    assert!(!archive.worktree_exists);
    assert_eq!(archive.head_revision, None);
    assert_eq!(archive.result_path, None);
    assert_eq!(
        db::list_convo_events(&pool, task_id).await.unwrap(),
        vec![r#"{"kind":"assistant","text":"kept"}"#]
    );
    assert_eq!(
        task_specs::list(&pool, task_id).await.unwrap()[0].relative_path,
        "evidence.md"
    );
}

async fn fixture(with_worktree: bool) -> Fixture {
    let root = temp_root::dir().join(format!(
        "task-archive-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    if with_worktree {
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q"]);
        git(&root, &["config", "user.email", "archive@example.test"]);
        git(&root, &["config", "user.name", "Archive Test"]);
        std::fs::write(root.join("tracked.txt"), "clean\n").unwrap();
        git(&root, &["add", "tracked.txt"]);
        git(&root, &["commit", "-qm", "initial"]);
    }
    let database = root.with_extension("sqlite");
    let pool = db::init_pool(database.to_str().unwrap()).await.unwrap();
    let task_id = db::insert_task(
        &pool,
        "/repo",
        "archive-branch",
        "main",
        root.to_str().unwrap(),
        "archive",
        None,
        None,
        "terminal",
        1,
    )
    .await
    .unwrap();
    db::update_state(&pool, task_id, db::state::DONE, 2)
        .await
        .unwrap();
    Fixture {
        pool,
        task_id,
        worktree: root,
    }
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {:?}: {:?}", args, output);
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
