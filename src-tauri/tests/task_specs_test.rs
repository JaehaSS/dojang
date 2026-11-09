#[path = "support/temp_root.rs"]
mod temp_root;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::db;
use praxis_lib::task_specs::{self, SpecSaveInput};

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Fixture {
    pool: sqlx::SqlitePool,
    task_id: i64,
    root: PathBuf,
    db_path: String,
}

#[tokio::test]
async fn search_and_read_return_only_project_markdown() {
    let fixture = fixture().await;
    write(&fixture.root, "spec.md", "# Product spec\nneedle in body\n");
    write(&fixture.root, "notes.txt", "needle should not appear\n");
    write(&fixture.root, ".hidden.md", "needle should not appear\n");
    write(
        &fixture.root,
        ".dojang/private.md",
        "needle should not appear\n",
    );
    write(
        &fixture.root,
        "ignored/skip.md",
        "needle should not appear\n",
    );
    write(&fixture.root, ".gitignore", "ignored/\n");
    let page = task_specs::search(&fixture.pool, fixture.task_id, "needle")
        .await
        .unwrap();
    assert_eq!(page.hits.len(), 1);
    assert_eq!(page.hits[0].relative_path, "spec.md");
    assert_eq!(page.hits[0].title, "Product spec");
    assert_eq!(page.hits[0].line, 2);
    write(
        &fixture.root,
        "auth.md",
        "# Access guide\nno body keyword\n",
    );
    let path_page = task_specs::search(&fixture.pool, fixture.task_id, "auth")
        .await
        .unwrap();
    assert_eq!(path_page.hits.len(), 1);
    assert_eq!(path_page.hits[0].relative_path, "auth.md");
    assert_eq!(path_page.hits[0].snippet, "auth.md");
    let document = task_specs::read(&fixture.pool, fixture.task_id, "./spec.md")
        .await
        .unwrap();
    assert_eq!(document.relative_path, "spec.md");
    assert_eq!(document.title, "Product spec");
    assert_eq!(document.content, "# Product spec\nneedle in body\n");
    assert_eq!(document.content_hash.len(), 64);
    assert_eq!(document.byte_length, document.content.len() as u64);
    assert!(
        task_specs::read(&fixture.pool, fixture.task_id, "notes.txt")
            .await
            .is_err()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_symlinks_and_git_metadata_aliases() {
    let fixture = fixture().await;
    write(&fixture.root, "ok.md", "# okay\n");
    let outside = temp_root::dir().join(format!("task-spec-outside-{}", std::process::id()));
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("outside.md"), "# outside\n").unwrap();
    std::os::unix::fs::symlink(outside.join("outside.md"), fixture.root.join("link.md")).unwrap();
    write(&fixture.root, ".git/config.md", "# metadata\n");
    write(&fixture.root, ".dojang/private.md", "# internal\n");
    std::os::unix::fs::symlink(fixture.root.join(".git"), fixture.root.join("git-alias")).unwrap();
    std::os::unix::fs::symlink(fixture.root.join(".dojang"), fixture.root.join("app-alias"))
        .unwrap();
    assert!(task_specs::read(&fixture.pool, fixture.task_id, "link.md")
        .await
        .is_err());
    assert!(
        task_specs::read(&fixture.pool, fixture.task_id, ".git/config.md")
            .await
            .is_err()
    );
    assert!(
        task_specs::read(&fixture.pool, fixture.task_id, "git-alias/config.md")
            .await
            .is_err()
    );
    assert!(
        task_specs::read(&fixture.pool, fixture.task_id, "app-alias/private.md")
            .await
            .is_err()
    );
    assert!(
        task_specs::read(&fixture.pool, fixture.task_id, "./.dojang/private.md")
            .await
            .is_err()
    );
    assert!(task_specs::read(
        &fixture.pool,
        fixture.task_id,
        &format!("ok.md{}bad", char::from(0))
    )
    .await
    .is_err());
}

#[tokio::test]
async fn save_uses_hash_cas_and_freezes_content_across_reopen() {
    let mut fixture = fixture().await;
    write(&fixture.root, "spec.md", "# first\nold body\n");
    let document = task_specs::read(&fixture.pool, fixture.task_id, "spec.md")
        .await
        .unwrap();
    let stale = SpecSaveInput {
        relative_path: "spec.md".into(),
        expected_hash: "0".repeat(64),
    };
    assert!(task_specs::save(&fixture.pool, fixture.task_id, stale)
        .await
        .is_err());
    let saved = task_specs::save(
        &fixture.pool,
        fixture.task_id,
        SpecSaveInput {
            relative_path: "spec.md".into(),
            expected_hash: document.content_hash.clone(),
        },
    )
    .await
    .unwrap();
    write(&fixture.root, "spec.md", "# second\nnew body\n");
    assert_eq!(
        task_specs::get(&fixture.pool, fixture.task_id, saved.id)
            .await
            .unwrap()
            .content,
        "# first\nold body\n"
    );
    assert!(
        sqlx::query("UPDATE task_spec_snapshots SET content='changed' WHERE id=?")
            .bind(saved.id)
            .execute(&fixture.pool)
            .await
            .is_err()
    );
    fixture.pool.close().await;
    fixture.pool = db::init_pool(&fixture.db_path).await.unwrap();
    assert_eq!(
        task_specs::get(&fixture.pool, fixture.task_id, saved.id)
            .await
            .unwrap()
            .content_hash,
        document.content_hash
    );
}

#[tokio::test]
async fn snapshots_are_task_scoped_and_delete_with_the_task() {
    let fixture = fixture().await;
    write(&fixture.root, "spec.md", "# body\n");
    let doc = task_specs::read(&fixture.pool, fixture.task_id, "spec.md")
        .await
        .unwrap();
    let saved = task_specs::save(
        &fixture.pool,
        fixture.task_id,
        SpecSaveInput {
            relative_path: "spec.md".into(),
            expected_hash: doc.content_hash,
        },
    )
    .await
    .unwrap();
    let other = add_task(&fixture).await;
    assert!(task_specs::get(&fixture.pool, other, saved.id)
        .await
        .is_err());
    let cross_token = format!("[[dojang-spec:{}:{}]]", saved.id, saved.content_hash);
    assert!(
        task_specs::expand_message(&fixture.pool, other, &cross_token)
            .await
            .is_err()
    );
    db::delete_task(&fixture.pool, fixture.task_id)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_spec_snapshots WHERE id=?")
        .bind(saved.id)
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn expansion_is_fixed_to_the_snapshot_and_nonrecursive() {
    let fixture = fixture().await;
    write(
        &fixture.root,
        "spec.md",
        "# frozen\ninner [[dojang-spec:999:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa]]\n",
    );
    let document = task_specs::read(&fixture.pool, fixture.task_id, "spec.md")
        .await
        .unwrap();
    let saved = task_specs::save(
        &fixture.pool,
        fixture.task_id,
        SpecSaveInput {
            relative_path: "spec.md".into(),
            expected_hash: document.content_hash,
        },
    )
    .await
    .unwrap();
    write(&fixture.root, "spec.md", "# live source must not be read\n");
    let token = format!("[[dojang-spec:{}:{}]]", saved.id, saved.content_hash);
    let expanded = task_specs::expand_message(
        &fixture.pool,
        fixture.task_id,
        &format!("before {token} after"),
    )
    .await
    .unwrap();
    assert!(expanded.contains("path: spec.md"));
    assert!(expanded.contains("# frozen"));
    assert!(expanded.contains("inner [[dojang-spec:999:"));
    assert!(expanded.starts_with("before "));
    assert!(expanded.ends_with(" after"));
    assert!(
        task_specs::expand_message(&fixture.pool, fixture.task_id, "ordinary message")
            .await
            .is_ok_and(|message| message == "ordinary message")
    );
    assert!(
        task_specs::expand_message(&fixture.pool, fixture.task_id, "[[dojang-spec:bad]]")
            .await
            .is_err()
    );
    let wrong_hash = format!("[[dojang-spec:{}:{}]]", saved.id, "f".repeat(64));
    assert!(
        task_specs::expand_message(&fixture.pool, fixture.task_id, &wrong_hash)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn search_and_expansion_enforce_bounds() {
    let fixture = fixture().await;
    for index in 0..45 {
        write(
            &fixture.root,
            &format!("many/{index}.md"),
            "# repeated\nneedle\n",
        );
    }
    write(&fixture.root, "large.md", &"a".repeat(64 * 1024 + 1));
    let page = task_specs::search(&fixture.pool, fixture.task_id, "needle")
        .await
        .unwrap();
    assert_eq!(page.hits.len(), 40);
    assert!(page.truncated);
    assert!(page.scanned_files <= 1_000);
    assert!(task_specs::read(&fixture.pool, fixture.task_id, "large.md")
        .await
        .is_err());
    assert!(
        task_specs::search(&fixture.pool, fixture.task_id, &"x".repeat(201))
            .await
            .is_err()
    );
    let unchanged = "x".repeat(128 * 1024 + 1);
    assert_eq!(
        task_specs::expand_message(&fixture.pool, fixture.task_id, &unchanged)
            .await
            .unwrap(),
        unchanged
    );
}

#[tokio::test]
async fn expansion_limits_reference_count_and_actual_frozen_body_bytes() {
    let fixture = fixture().await;
    write(&fixture.root, "small.md", "# Small\nrequirement\n");
    let small = task_specs::read(&fixture.pool, fixture.task_id, "small.md")
        .await
        .unwrap();
    let saved = task_specs::save(
        &fixture.pool,
        fixture.task_id,
        SpecSaveInput {
            relative_path: small.relative_path,
            expected_hash: small.content_hash,
        },
    )
    .await
    .unwrap();
    let token = format!("[[dojang-spec:{}:{}]]", saved.id, saved.content_hash);
    assert!(
        task_specs::expand_message(&fixture.pool, fixture.task_id, &token.repeat(4))
            .await
            .is_ok()
    );
    let too_many = task_specs::expand_message(&fixture.pool, fixture.task_id, &token.repeat(5))
        .await
        .unwrap_err();
    assert!(too_many.contains("4개"));
    let oversized_input = format!("{}{token}", "한".repeat(128 * 1024 / 3));
    assert!(
        task_specs::expand_message(&fixture.pool, fixture.task_id, &oversized_input)
            .await
            .unwrap_err()
            .contains("128KiB")
    );
    assert!(task_specs::expand_message(
        &fixture.pool,
        fixture.task_id,
        "[[dojang-spec:1:missing-close"
    )
    .await
    .is_err());

    write(&fixture.root, "large.md", &"a".repeat(64 * 1024));
    let large = task_specs::read(&fixture.pool, fixture.task_id, "large.md")
        .await
        .unwrap();
    let saved = task_specs::save(
        &fixture.pool,
        fixture.task_id,
        SpecSaveInput {
            relative_path: large.relative_path,
            expected_hash: large.content_hash,
        },
    )
    .await
    .unwrap();
    let token = format!("[[dojang-spec:{}:{}]]", saved.id, saved.content_hash);
    assert!(
        task_specs::expand_message(&fixture.pool, fixture.task_id, &token)
            .await
            .is_ok()
    );
    assert!(
        task_specs::expand_message(&fixture.pool, fixture.task_id, &token.repeat(2))
            .await
            .unwrap_err()
            .contains("128KiB")
    );
}

async fn fixture() -> Fixture {
    let suffix = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = temp_root::dir().join(format!("dojang-task-specs-{}-{suffix}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let db_path = root.join("test.sqlite").to_string_lossy().into_owned();
    let pool = db::init_pool(&db_path).await.unwrap();
    let root_text = root.to_string_lossy();
    let task_id = db::insert_task(
        &pool,
        &root_text,
        "praxis/task-specs",
        "main",
        &root_text,
        "test",
        None,
        None,
        "conversation",
        1,
    )
    .await
    .unwrap();
    Fixture {
        pool,
        task_id,
        root,
        db_path,
    }
}

async fn add_task(fixture: &Fixture) -> i64 {
    let root = fixture.root.to_string_lossy();
    db::insert_task(
        &fixture.pool,
        &root,
        "praxis/task-specs-other",
        "main",
        &root,
        "other",
        None,
        None,
        "conversation",
        2,
    )
    .await
    .unwrap()
}

fn write(root: &std::path::Path, relative: &str, body: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}
