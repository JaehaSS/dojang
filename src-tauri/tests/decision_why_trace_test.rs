#[path = "support/temp_root.rs"]
mod temp_root;

use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::decision::provenance::ApprovalProvenance;
use praxis_lib::{db, decision};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_db(label: &str) -> String {
    let sequence = COUNTER.fetch_add(1, Ordering::SeqCst);
    temp_root::dir()
        .join(format!(
            "praxis-decision-trace-{label}-{}-{sequence}.sqlite",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned()
}

fn provenance(task_id: i64, commit_digit: char) -> ApprovalProvenance {
    ApprovalProvenance {
        task_id,
        instruction_digest: decision::provenance::digest(&format!("instruction-{task_id}")),
        verification_run: None,
        commit_sha: commit_digit.to_string().repeat(40),
    }
}

async fn insert_approval(pool: &sqlx::SqlitePool, task_id: i64, digit: char) -> i64 {
    let mut tx = pool.begin().await.unwrap();
    let id = decision::record::record_approval(&mut tx, &provenance(task_id, digit), 100)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    id
}

#[tokio::test]
async fn trace_is_deterministic_bounded_and_cycle_safe() {
    let path = temp_db("graph");
    let pool = db::init_pool(&path).await.unwrap();
    db::set_setting(&pool, decision::FLAG_KEY, "true")
        .await
        .unwrap();
    let first = insert_approval(&pool, 1, 'a').await;
    let second = insert_approval(&pool, 2, 'b').await;

    let trace = decision::why_trace::why_trace(&pool, first, 2)
        .await
        .unwrap();
    let repeated = decision::why_trace::why_trace(&pool, first, 99)
        .await
        .unwrap();
    assert_eq!(trace, repeated, "depth must clamp to two");
    assert!(!trace.truncated);
    assert!(trace
        .nodes
        .iter()
        .any(|node| node.key == format!("decision:{second}")));
    let unique: HashSet<&str> = trace.nodes.iter().map(|node| node.key.as_str()).collect();
    assert_eq!(unique.len(), trace.nodes.len(), "cycle revisited a node");
    assert!(trace.nodes.len() <= 100);
    drop(pool);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn trace_marks_truncation_at_one_hundred_nodes() {
    let path = temp_db("limit");
    let pool = db::init_pool(&path).await.unwrap();
    db::set_setting(&pool, decision::FLAG_KEY, "true")
        .await
        .unwrap();
    let decision_id = insert_approval(&pool, 7, 'c').await;
    for index in 0..110 {
        sqlx::query(
            "INSERT INTO decision_artifact_links \
             (decision_id, relation, artifact_kind, artifact_ref, created_at) \
             VALUES (?, 'used', 'task_start_receipt', ?, 100)",
        )
        .bind(decision_id)
        .bind(format!("extra-{index:03}"))
        .execute(&pool)
        .await
        .unwrap();
    }

    let trace = decision::why_trace::why_trace(&pool, decision_id, 2)
        .await
        .unwrap();
    assert_eq!(trace.nodes.len(), 100);
    assert!(trace.truncated);
    drop(pool);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn disabled_and_redacted_decisions_are_not_readable() {
    let path = temp_db("visibility");
    let pool = db::init_pool(&path).await.unwrap();
    let id = insert_approval(&pool, 5, 'd').await;

    let disabled = decision::why_trace::why_trace(&pool, id, 2)
        .await
        .unwrap_err();
    assert!(disabled.to_string().contains("disabled"));

    db::set_setting(&pool, decision::FLAG_KEY, "true")
        .await
        .unwrap();
    sqlx::query(
        "UPDATE decision_records SET kind = NULL, outcome = NULL, actor_kind = NULL, \
         task_id = NULL, summary = NULL, status = 'redacted', redacted_at = 200 WHERE id = ?",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    let redacted = decision::why_trace::why_trace(&pool, id, 2)
        .await
        .unwrap_err();
    assert!(redacted.to_string().contains("not found"));
    drop(pool);
    let _ = std::fs::remove_file(path);
}
