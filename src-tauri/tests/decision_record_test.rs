#[path = "support/temp_root.rs"]
mod temp_root;

use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::decision::provenance::{ApprovalProvenance, ArtifactKind, Relation};
use praxis_lib::{db, decision};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_db() -> String {
    let sequence = COUNTER.fetch_add(1, Ordering::SeqCst);
    temp_root::dir()
        .join(format!(
            "praxis-decision-record-{}-{sequence}.sqlite",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned()
}

fn approval(instruction: &str) -> ApprovalProvenance {
    ApprovalProvenance {
        task_id: 42,
        instruction_digest: decision::provenance::digest(instruction),
        verification_run: Some(decision::provenance::verification_ref(42, 1_700)),
        commit_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
    }
}

#[test]
fn approval_links_are_typed_deduplicated_and_stable() {
    let links = approval("ship it").links().unwrap();
    let actual: Vec<(&str, &str, &str)> = links
        .iter()
        .map(|link| {
            (
                link.relation.as_str(),
                link.kind.as_str(),
                link.artifact_ref.as_str(),
            )
        })
        .collect();

    // fixed 4 + verification 1
    assert_eq!(actual.len(), 5);
    assert_eq!(actual[0], ("approved_by", "actor", "local-human"));
    assert!(actual.contains(&("derived_from", "task", "42")));
    assert!(actual.contains(&("generated", "git_commit", approval("x").commit_sha.as_str())));
    assert!(actual
        .iter()
        .any(|(_, kind, _)| *kind == "instruction_digest"));
    assert_eq!(Relation::Supersedes.as_str(), "supersedes");
    assert_eq!(Relation::BlockedBy.as_str(), "blocked_by");
    assert_eq!(ArtifactKind::VerificationRun.as_str(), "verification_run");
}

#[tokio::test]
async fn writer_is_idempotent_and_rejects_a_changed_payload() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.unwrap();
    let provenance = approval("RAW_PROMPT_SENTINEL");
    let first = record_twice(&pool, &provenance).await;
    assert_counts_and_privacy(&pool).await;
    assert_payload_conflict(&pool, &provenance).await;
    assert_record_immutable(&pool, first).await;
    drop(pool);
    let _ = std::fs::remove_file(path);
}

async fn record_twice(pool: &sqlx::SqlitePool, provenance: &ApprovalProvenance) -> i64 {
    let mut first_tx = pool.begin().await.unwrap();
    let first = decision::record::record_approval(&mut first_tx, provenance, 2_000)
        .await
        .unwrap();
    first_tx.commit().await.unwrap();

    let mut retry_tx = pool.begin().await.unwrap();
    let retry = decision::record::record_approval(&mut retry_tx, provenance, 2_001)
        .await
        .unwrap();
    retry_tx.commit().await.unwrap();
    assert_eq!(first, retry);
    first
}

async fn assert_counts_and_privacy(pool: &sqlx::SqlitePool) {
    let record_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM decision_records")
        .fetch_one(pool)
        .await
        .unwrap();
    let link_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM decision_artifact_links")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(record_count, 1);
    assert_eq!(link_count, 5);
    let text: String = sqlx::query_scalar(
        "SELECT decision_key_hash || COALESCE(kind, '') || COALESCE(outcome, '') || \
         COALESCE(actor_kind, '') || COALESCE(summary, '') FROM decision_records",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(!text.contains("RAW_PROMPT_SENTINEL"));
}

async fn assert_payload_conflict(pool: &sqlx::SqlitePool, provenance: &ApprovalProvenance) {
    let mut changed = provenance.clone();
    changed.verification_run = Some(decision::provenance::verification_ref(42, 1_701));
    let mut conflict_tx = pool.begin().await.unwrap();
    let error = decision::record::record_approval(&mut conflict_tx, &changed, 2_002)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("payload conflict"));
    conflict_tx.rollback().await.unwrap();
}

async fn assert_record_immutable(pool: &sqlx::SqlitePool, decision_id: i64) {
    let immutable = sqlx::query("UPDATE decision_records SET summary = 'changed' WHERE id = ?")
        .bind(decision_id)
        .execute(pool)
        .await
        .unwrap_err();
    assert!(immutable
        .to_string()
        .contains("decision record is immutable"));
}

#[tokio::test]
async fn database_rejects_free_form_relations_and_artifact_kinds() {
    let path = temp_db();
    let pool = db::init_pool(&path).await.unwrap();
    sqlx::query(
        "INSERT INTO decision_records \
         (decision_key_hash, kind, outcome, actor_kind, task_id, summary, status, created_at) \
         VALUES (?, 'task_approval', 'approved', 'local_human', 1, \
         'Isolated worktree changes approved and merged.', 'active', 1)",
    )
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .execute(&pool)
    .await
    .unwrap();
    let error = sqlx::query(
        "INSERT INTO decision_artifact_links \
         (decision_id, relation, artifact_kind, artifact_ref, created_at) \
         VALUES (1, 'caused', 'prompt', 'raw', 1)",
    )
    .execute(&pool)
    .await
    .unwrap_err();
    assert!(error.to_string().contains("CHECK constraint failed"));
    drop(pool);
    let _ = std::fs::remove_file(path);
}
