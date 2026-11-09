#[path = "support/temp_root.rs"]
mod temp_root;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::{
    db,
    task_specs::{self, links::SpecLinkState},
};

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Fixture {
    pool: sqlx::SqlitePool,
    task_id: i64,
    root: PathBuf,
}

#[tokio::test]
async fn validates_inline_reference_and_image_targets() {
    let fixture = fixture().await;
    write(&fixture.root, "docs/guide.md", "# Guide\n");
    write(&fixture.root, "assets/logo.png", "image");
    write(&fixture.root, "spec.md", "[guide]: docs/guide.md\n\n[inline](docs/guide.md)\n[reference][guide]\n![logo](assets/logo%2Epng)\n");
    let document = task_specs::read(&fixture.pool, fixture.task_id, "spec.md")
        .await
        .unwrap();
    let report = task_specs::links::validate(
        &fixture.pool,
        fixture.task_id,
        "spec.md",
        &document.content_hash,
    )
    .await
    .unwrap();
    assert_eq!(report.summary.ok, 3);
    assert_eq!(report.summary.total, 3);
    assert!(report
        .links
        .iter()
        .all(|link| link.state == SpecLinkState::Ok));
    assert_eq!(report.links[0].source_line, 3);
}

#[tokio::test]
async fn reports_missing_anchors_and_external_without_counting_external_as_ok() {
    let fixture = fixture().await;
    write(&fixture.root, "target.md", "# Present heading\n");
    write(
        &fixture.root,
        "spec.md",
        "# Present heading\n\n[same](#present-heading)\n[near](#present-heading!)\n[missing](target.md#nope)\n[absent](gone.md)\n[web](https://example.com/path)\n",
    );
    let document = task_specs::read(&fixture.pool, fixture.task_id, "spec.md")
        .await
        .unwrap();
    let report = task_specs::links::validate(
        &fixture.pool,
        fixture.task_id,
        "spec.md",
        &document.content_hash,
    )
    .await
    .unwrap();
    assert_eq!(report.summary.ok, 1);
    assert_eq!(report.summary.anchor_missing, 2);
    assert_eq!(report.summary.missing, 1);
    assert_eq!(report.summary.external, 1);
    assert_eq!(report.links[4].state, SpecLinkState::External);
}

#[tokio::test]
async fn bounds_the_number_of_checked_links() {
    let fixture = fixture().await;
    write(&fixture.root, "target.md", "# target\n");
    let links = (0..101)
        .map(|_| "[target](target.md)\n")
        .collect::<String>();
    write(&fixture.root, "spec.md", &links);
    let document = task_specs::read(&fixture.pool, fixture.task_id, "spec.md")
        .await
        .unwrap();
    let report = task_specs::links::validate(
        &fixture.pool,
        fixture.task_id,
        "spec.md",
        &document.content_hash,
    )
    .await
    .unwrap();
    assert!(report.truncated);
    assert_eq!(report.links.len(), 100);
    assert_eq!(report.summary.total, 100);
}

#[tokio::test]
async fn rejects_changed_source_hash() {
    let fixture = fixture().await;
    write(&fixture.root, "spec.md", "[missing](nope.md)\n");
    let document = task_specs::read(&fixture.pool, fixture.task_id, "spec.md")
        .await
        .unwrap();
    write(&fixture.root, "spec.md", "[different](other.md)\n");
    let error = task_specs::links::validate(
        &fixture.pool,
        fixture.task_id,
        "spec.md",
        &document.content_hash,
    )
    .await
    .unwrap_err();
    assert!(error.contains("변경"));
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_outside_root_symlink_targets() {
    let fixture = fixture().await;
    let outside = temp_root::dir().join(format!("task-spec-links-outside-{}", std::process::id()));
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("outside.md"), "# outside\n").unwrap();
    std::os::unix::fs::symlink(outside.join("outside.md"), fixture.root.join("linked.md")).unwrap();
    write(&fixture.root, "spec.md", "[bad](linked.md)\n");
    let document = task_specs::read(&fixture.pool, fixture.task_id, "spec.md")
        .await
        .unwrap();
    let report = task_specs::links::validate(
        &fixture.pool,
        fixture.task_id,
        "spec.md",
        &document.content_hash,
    )
    .await
    .unwrap();
    assert_eq!(report.links[0].state, SpecLinkState::OutsideScope);
}

async fn fixture() -> Fixture {
    let suffix = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = temp_root::dir().join(format!(
        "dojang-task-spec-links-{}-{suffix}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let db_path = root.join("test.sqlite").to_string_lossy().into_owned();
    let pool = db::init_pool(&db_path).await.unwrap();
    let root_text = root.to_string_lossy();
    let task_id = db::insert_task(
        &pool,
        &root_text,
        "praxis/task-spec-links",
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
    }
}

fn write(root: &Path, relative: &str, body: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}
