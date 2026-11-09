#[path = "support/temp_root.rs"]
mod temp_root;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::db;
use praxis_lib::task_results::artifacts::{self, ArtifactResourceInput, ArtifactSaveInput};

static COUNTER: AtomicU32 = AtomicU32::new(0);

#[tokio::test]
async fn versions_preserve_content_after_source_edit_and_deletion() {
    let fixture = fixture().await;
    std::fs::write(fixture.root.join("report.html"), "<h1>first</h1>").unwrap();
    let mut first_input = input(&fixture, "report.html", vec![]).await;
    first_input.unavailable = vec!["https://example.invalid/not-fetched.css".into()];
    let first = artifacts::save(&fixture.pool, fixture.task_id, first_input)
        .await
        .unwrap();
    std::fs::write(fixture.root.join("report.html"), "<h1>second</h1>").unwrap();
    let second = artifacts::save(
        &fixture.pool,
        fixture.task_id,
        input(&fixture, "report.html", vec![]).await,
    )
    .await
    .unwrap();
    std::fs::remove_file(fixture.root.join("report.html")).unwrap();

    assert_eq!(first.version, 1);
    assert_eq!(second.version, 2);
    assert_eq!(
        artifacts::get(&fixture.pool, fixture.task_id, first.id)
            .await
            .unwrap()
            .content,
        "<h1>first</h1>"
    );
    assert_eq!(
        artifacts::get(&fixture.pool, fixture.task_id, first.id)
            .await
            .unwrap()
            .unavailable,
        vec!["https://example.invalid/not-fetched.css"]
    );
    assert_eq!(
        artifacts::get(&fixture.pool, fixture.task_id, second.id)
            .await
            .unwrap()
            .content,
        "<h1>second</h1>"
    );
    assert_eq!(
        artifacts::list(&fixture.pool, fixture.task_id)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn rejects_stale_result_and_changed_resource() {
    let fixture = fixture().await;
    std::fs::write(fixture.root.join("report.html"), "<h1>report</h1>").unwrap();
    std::fs::write(fixture.root.join("style.css"), "h1{color:red}").unwrap();
    let mut stale = input(
        &fixture,
        "report.html",
        vec![ArtifactResourceInput {
            relative_path: "style.css".into(),
            expected_content: "h1{color:red}".into(),
        }],
    )
    .await;
    stale.expected_revision += 1;
    assert!(artifacts::save(&fixture.pool, fixture.task_id, stale)
        .await
        .is_err());

    let mut changed = input(
        &fixture,
        "report.html",
        vec![ArtifactResourceInput {
            relative_path: "style.css".into(),
            expected_content: "h1{color:red}".into(),
        }],
    )
    .await;
    std::fs::write(fixture.root.join("style.css"), "h1{color:blue}").unwrap();
    let current = praxis_lib::task_results::snapshot(&fixture.pool, fixture.task_id)
        .await
        .unwrap();
    changed.source_id = current.source_id;
    changed.expected_revision = current.result_revision;
    assert!(artifacts::save(&fixture.pool, fixture.task_id, changed)
        .await
        .is_err());
}

#[tokio::test]
async fn snapshots_cannot_cross_task_boundaries_and_cleanup_on_deletion() {
    let fixture = fixture().await;
    std::fs::write(fixture.root.join("report.md"), "# saved").unwrap();
    let saved = artifacts::save(
        &fixture.pool,
        fixture.task_id,
        input(&fixture, "report.md", vec![]).await,
    )
    .await
    .unwrap();
    let other = new_task(&fixture).await;
    assert!(artifacts::get(&fixture.pool, other, saved.id)
        .await
        .is_err());
    assert!(artifacts::list(&fixture.pool, other)
        .await
        .unwrap()
        .is_empty());

    db::delete_task(&fixture.pool, fixture.task_id)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_review_artifacts WHERE id=?")
        .bind(saved.id)
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn rejects_escape_paths_symlinks_and_internal_files() {
    let fixture = fixture().await;
    std::fs::write(fixture.root.join("report.html"), "<p>ok</p>").unwrap();
    let base = input(&fixture, "report.html", vec![]).await;
    let mut traversal = base.clone();
    traversal.relative_path = "../report.html".into();
    assert!(artifacts::save(&fixture.pool, fixture.task_id, traversal)
        .await
        .is_err());
    let mut internal = base.clone();
    std::fs::create_dir_all(fixture.root.join(".dojang")).unwrap();
    std::fs::write(fixture.root.join(".dojang/hidden.html"), "<p>no</p>").unwrap();
    internal.relative_path = ".dojang/hidden.html".into();
    internal.expected_content = "<p>no</p>".into();
    assert!(artifacts::save(&fixture.pool, fixture.task_id, internal)
        .await
        .is_err());
    let mut normalized_internal = input(&fixture, "report.html", vec![]).await;
    normalized_internal.relative_path = "./.dojang/hidden.html".into();
    normalized_internal.expected_content = "<p>no</p>".into();
    assert!(
        artifacts::save(&fixture.pool, fixture.task_id, normalized_internal)
            .await
            .is_err()
    );
    let mut git_internal = input(&fixture, "report.html", vec![]).await;
    std::fs::write(fixture.root.join(".git/review.html"), "<p>metadata</p>").unwrap();
    git_internal.relative_path = ".git/review.html".into();
    git_internal.expected_content = "<p>metadata</p>".into();
    assert!(
        artifacts::save(&fixture.pool, fixture.task_id, git_internal)
            .await
            .is_err()
    );
    let mut git_resource = input(&fixture, "report.html", vec![]).await;
    git_resource
        .resources
        .push(resource(&fixture, ".git/config"));
    assert!(
        artifacts::save(&fixture.pool, fixture.task_id, git_resource)
            .await
            .is_err()
    );
    #[cfg(unix)]
    {
        let outside =
            temp_root::dir().join(format!("review-artifact-outside-{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("outside.html"), "<p>outside</p>").unwrap();
        std::os::unix::fs::symlink(
            outside.join("outside.html"),
            fixture.root.join("escape.html"),
        )
        .unwrap();
        let mut link = base;
        link.relative_path = "escape.html".into();
        link.expected_content = "<p>outside</p>".into();
        assert!(artifacts::save(&fixture.pool, fixture.task_id, link)
            .await
            .is_err());
    }
}

#[tokio::test]
async fn migration_is_idempotent_and_hashes_frozen_preview_and_resources() {
    let fixture = fixture().await;
    artifacts::migrate(&fixture.pool).await.unwrap();
    std::fs::write(fixture.root.join("report.html"), "<h1>same</h1>").unwrap();
    std::fs::write(fixture.root.join("style.css"), "h1{color:red}").unwrap();
    let first = artifacts::save(
        &fixture.pool,
        fixture.task_id,
        input(
            &fixture,
            "report.html",
            vec![resource(&fixture, "style.css")],
        )
        .await,
    )
    .await
    .unwrap();
    std::fs::write(fixture.root.join("style.css"), "h1{color:blue}").unwrap();
    let second = artifacts::save(
        &fixture.pool,
        fixture.task_id,
        input(
            &fixture,
            "report.html",
            vec![resource(&fixture, "style.css")],
        )
        .await,
    )
    .await
    .unwrap();
    assert_ne!(first.content_hash, second.content_hash);
    assert_eq!(second.version, first.version + 1);
}

struct Fixture {
    pool: sqlx::SqlitePool,
    task_id: i64,
    root: PathBuf,
}

#[tokio::test]
async fn concurrent_saves_keep_distinct_versions() {
    let fixture = fixture().await;
    std::fs::write(fixture.root.join("report.html"), "<p>shared source</p>").unwrap();
    let prepared = input(&fixture, "report.html", vec![]).await;
    let (first, second) = tokio::join!(
        artifacts::save(&fixture.pool, fixture.task_id, prepared.clone()),
        artifacts::save(&fixture.pool, fixture.task_id, prepared),
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(first.content_hash, second.content_hash);
    let mut versions = [first.version, second.version];
    versions.sort();
    assert_eq!(versions, [1, 2]);
}

async fn fixture() -> Fixture {
    let suffix = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = temp_root::dir().join(format!(
        "dojang-review-artifacts-{}-{suffix}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(".gitignore"), "test.sqlite*\n").unwrap();
    for args in [
        ["init", "-q", "-b", "main"].as_slice(),
        ["config", "user.email", "test@example.invalid"].as_slice(),
        ["config", "user.name", "Test"].as_slice(),
        ["add", ".gitignore"].as_slice(),
        ["commit", "-qm", "initial"].as_slice(),
    ] {
        assert!(std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .status()
            .unwrap()
            .success());
    }
    let db_path = root.join("test.sqlite").to_string_lossy().into_owned();
    let pool = db::init_pool(&db_path).await.unwrap();
    let root_text = root.to_string_lossy();
    let task_id = db::insert_task(
        &pool,
        &root_text,
        "praxis/artifacts",
        "main",
        &root_text,
        "review",
        None,
        None,
        "terminal",
        1,
    )
    .await
    .unwrap();
    Fixture {
        pool,
        task_id,
        root,
    }
}

async fn new_task(fixture: &Fixture) -> i64 {
    let root = fixture.root.to_string_lossy();
    db::insert_task(
        &fixture.pool,
        &root,
        "praxis/artifacts-other",
        "main",
        &root,
        "review",
        None,
        None,
        "terminal",
        2,
    )
    .await
    .unwrap()
}

async fn input(
    fixture: &Fixture,
    path: &str,
    resources: Vec<ArtifactResourceInput>,
) -> ArtifactSaveInput {
    let result = praxis_lib::task_results::snapshot(&fixture.pool, fixture.task_id)
        .await
        .unwrap();
    let expected_content = praxis_lib::fsapi::read_file(&fixture.root, path)
        .unwrap()
        .content;
    let preview_html = if path.ends_with(".md") {
        Some("<article>saved</article>".into())
    } else {
        Some("<!doctype html><html><body>saved</body></html>".into())
    };
    ArtifactSaveInput {
        source_id: result.source_id,
        expected_revision: result.result_revision,
        relative_path: path.into(),
        expected_content,
        preview_html,
        unavailable: vec![],
        resources,
    }
}

fn resource(fixture: &Fixture, path: &str) -> ArtifactResourceInput {
    ArtifactResourceInput {
        relative_path: path.into(),
        expected_content: praxis_lib::fsapi::read_file(&fixture.root, path)
            .unwrap()
            .content,
    }
}
