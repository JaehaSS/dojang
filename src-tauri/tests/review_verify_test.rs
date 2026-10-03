#[path = "support/temp_root.rs"]
mod temp_root;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use praxis_lib::db::{self, state};
use praxis_lib::review_ops::{verify, ReviewClaims};

static COUNTER: AtomicU32 = AtomicU32::new(0);

#[tokio::test]
async fn preview_token_rejects_changed_commands_before_spawn() {
    let fixture = fixture("test = \"printf '1 passed'\"\n").await;
    let preview = verify::preview(&fixture.pool, fixture.task_id, &fixture.root)
        .await
        .unwrap();
    write_spec(&fixture.root, "test = \"printf '2 passed'\"\n");

    let error = verify::run(
        fixture.pool.clone(),
        fixture.claims.clone(),
        fixture.task_id,
        fixture.root.clone(),
        preview.preview_token,
    )
    .await
    .unwrap_err();

    assert!(error.contains("미리보기"));
    assert!(db::get_evidence(&fixture.pool, fixture.task_id)
        .await
        .unwrap()
        .is_none());
    assert!(fixture.claims.claim_finalization(fixture.task_id).is_ok());
}

#[tokio::test]
async fn verify_persists_evidence_and_event() {
    let fixture = fixture("test = \"printf '3 passed'\"\n").await;
    let preview = verify::preview(&fixture.pool, fixture.task_id, &fixture.root)
        .await
        .unwrap();

    let report = verify::run(
        fixture.pool.clone(),
        fixture.claims.clone(),
        fixture.task_id,
        fixture.root.clone(),
        preview.preview_token,
    )
    .await
    .unwrap();

    assert!(report.ready);
    assert_eq!(report.summary.unwrap().passed, 3);
    assert!(
        db::get_evidence(&fixture.pool, fixture.task_id)
            .await
            .unwrap()
            .unwrap()
            .ready
    );
    let events = db::recent_events(&fixture.pool, fixture.task_id, 8)
        .await
        .unwrap();
    assert_eq!(events[0].kind, "verify");
}

#[tokio::test]
async fn aborting_request_does_not_release_worker_claim_early() {
    let fixture = fixture("test = \"sleep 1; printf '1 passed'\"\n").await;
    let preview = verify::preview(&fixture.pool, fixture.task_id, &fixture.root)
        .await
        .unwrap();
    let worker = tokio::spawn(verify::run(
        fixture.pool.clone(),
        fixture.claims.clone(),
        fixture.task_id,
        fixture.root.clone(),
        preview.preview_token,
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;
    worker.abort();

    assert!(fixture.claims.claim_finalization(fixture.task_id).is_err());
    // 고정 대기(1,200ms)는 부하에서 곧 실패가 된다 — 하청 명령(`sleep 1`)의 실제 소요가
    // 프로세스 시작 지연만큼 늘어난다. 풀릴 때까지 기다리되 상한을 둔다. 검증하려는 성질은
    // "언제 풀리느냐"가 아니라 "abort가 claim을 앞당겨 풀지 않는다"이고, 그건 위 줄이 본다.
    let released = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if fixture.claims.claim_finalization(fixture.task_id).is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(released.is_ok(), "작업이 끝났는데도 claim이 풀리지 않았다");
    assert!(db::get_evidence(&fixture.pool, fixture.task_id)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn terminal_state_recheck_rejects_persistence() {
    let fixture = fixture("test = \"sleep 1; printf '1 passed'\"\n").await;
    let preview = verify::preview(&fixture.pool, fixture.task_id, &fixture.root)
        .await
        .unwrap();
    let worker = tokio::spawn(verify::run(
        fixture.pool.clone(),
        fixture.claims.clone(),
        fixture.task_id,
        fixture.root.clone(),
        preview.preview_token,
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;
    db::update_state(&fixture.pool, fixture.task_id, state::DONE, 2)
        .await
        .unwrap();

    assert!(worker.await.unwrap().unwrap_err().contains("종료"));
    assert!(db::get_evidence(&fixture.pool, fixture.task_id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn non_git_checks_keep_their_outcome_without_claiming_current_source() {
    let fixture = fixture("test = \"printf '3 passed'\"\n").await;
    std::fs::remove_dir_all(fixture.root.join(".git")).unwrap();
    let preview = verify::preview(&fixture.pool, fixture.task_id, &fixture.root)
        .await
        .unwrap();
    let report = verify::run(
        fixture.pool.clone(),
        fixture.claims.clone(),
        fixture.task_id,
        fixture.root.clone(),
        preview.preview_token,
    )
    .await
    .unwrap();
    assert!(!report.ready);
    assert_eq!(report.summary.unwrap().passed, 3);
    let result = praxis_lib::task_results::snapshot(&fixture.pool, fixture.task_id)
        .await
        .unwrap();
    assert_eq!(result.receipts[0].outcome, "passed");
    assert_eq!(result.receipts[0].freshness, "unknown");
}

struct Fixture {
    pool: sqlx::SqlitePool,
    task_id: i64,
    root: PathBuf,
    claims: ReviewClaims,
}

async fn fixture(spec: &str) -> Fixture {
    let suffix = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = temp_root::dir().join(format!(
        "praxis-review-verify-{}-{suffix}",
        std::process::id()
    ));
    std::fs::create_dir_all(root.join(".praxis")).unwrap();
    std::fs::write(root.join(".gitignore"), "test.sqlite*\n").unwrap();
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.email", "test@example.invalid"],
        vec!["config", "user.name", "Test"],
        vec!["add", ".gitignore"],
        vec!["commit", "-qm", "initial"],
    ] {
        assert!(std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .status()
            .unwrap()
            .success());
    }
    write_spec(&root, spec);
    let db_path = root.join("test.sqlite").to_string_lossy().into_owned();
    let pool = db::init_pool(&db_path).await.unwrap();
    let root_text = root.to_string_lossy();
    let task_id = db::insert_task(
        &pool,
        &root_text,
        "praxis/review",
        "main",
        &root_text,
        "verify",
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
        claims: ReviewClaims::default(),
    }
}

fn write_spec(root: &Path, content: &str) {
    std::fs::write(root.join(".praxis").join("validate.toml"), content).unwrap();
}
