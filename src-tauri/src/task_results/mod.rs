//! Durable task-result revisions.  This module deliberately records what was
//! observed; it never infers a successful result from an agent's prose.

mod fingerprint;

use std::path::{Component, Path};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::db;
use crate::verify::{ValidateSpec, VerifyReport};

pub use fingerprint::{fingerprint, fingerprint_with_base, FingerprintObservation};

type LegacyEvidenceRow = (
    i64,
    Option<String>,
    Option<i64>,
    Option<String>,
    Option<i64>,
    i64,
);

const CURRENT: &str = "current";
const UNKNOWN: &str = "unknown";
const LEGACY: &str = "legacy";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TaskResultSnapshot {
    pub task_id: i64,
    pub source_id: String,
    pub result_revision: i64,
    /// The latest result-producing task completion, not a file-change counter.
    pub completion_epoch: i64,
    pub freshness: String,
    /// Versioned SHA-256 source fingerprint.  `None` means observation was incomplete.
    pub fingerprint: Option<String>,
    pub receipts: Vec<VerificationReceipt>,
    pub references: Vec<ResultReference>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct VerificationReceipt {
    pub verify_run_id: String,
    pub source_id: String,
    pub result_revision: i64,
    pub started_at: i64,
    pub finished_at: i64,
    pub command_spec_hash: String,
    pub source_before: Option<String>,
    pub source_after: Option<String>,
    pub outcome: String,
    pub freshness: String,
    pub logs_ref: Option<String>,
    pub observation_scope: String,
    pub report: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ResultReference {
    pub result_revision: i64,
    pub kind: String,
    pub relative_path: String,
    /// `available`, `missing`, or `unavailable`; stored paths are never
    /// silently rewritten when a worktree changes.
    pub availability: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceInput {
    pub expected_source_id: String,
    pub expected_revision: i64,
    pub kind: String,
    pub relative_path: String,
}

/// Additive schema.  Every verification attempt is immutable and the old
/// `evidence` row remains a compatibility projection only.
pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    let source_id = crate::notifications::source_id(pool).await?;
    let mut tx = pool.begin().await?;
    for sql in [
        "CREATE TABLE IF NOT EXISTS task_result_completion_epochs (task_id INTEGER PRIMARY KEY, completion_epoch INTEGER NOT NULL)",
        "CREATE TABLE IF NOT EXISTS task_result_revisions (task_id INTEGER NOT NULL, result_revision INTEGER NOT NULL, source_id TEXT NOT NULL, fingerprint TEXT, completion_epoch INTEGER NOT NULL, created_at INTEGER NOT NULL, PRIMARY KEY(task_id, result_revision))",
        "CREATE INDEX IF NOT EXISTS idx_task_result_revisions_current ON task_result_revisions(task_id, result_revision DESC)",
        "CREATE TABLE IF NOT EXISTS verification_receipts (verify_run_id TEXT PRIMARY KEY, task_id INTEGER NOT NULL, source_id TEXT NOT NULL, result_revision INTEGER NOT NULL, started_at INTEGER NOT NULL, finished_at INTEGER NOT NULL, command_spec_hash TEXT NOT NULL, source_before TEXT, source_after TEXT, outcome TEXT NOT NULL CHECK(outcome IN ('passed','failed','not_run','infrastructure_error','cancelled')), freshness TEXT NOT NULL CHECK(freshness IN ('current','stale','unknown')), logs_ref TEXT, observation_scope TEXT NOT NULL)",
        "CREATE INDEX IF NOT EXISTS idx_verification_receipts_task ON verification_receipts(task_id, result_revision, finished_at)",
        "CREATE TRIGGER IF NOT EXISTS verification_receipts_immutable_update BEFORE UPDATE ON verification_receipts BEGIN SELECT RAISE(ABORT, 'verification receipt is immutable'); END",
        "CREATE TABLE IF NOT EXISTS task_result_references (task_id INTEGER NOT NULL, result_revision INTEGER NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('file','preview','report')), relative_path TEXT NOT NULL, created_at INTEGER NOT NULL, PRIMARY KEY(task_id, result_revision, kind))",
        // Existing task deletion owns retention.  A task-level cleanup trigger
        // keeps the append-only tables from blocking that established path.
        "CREATE TRIGGER IF NOT EXISTS task_results_completion_epoch AFTER UPDATE OF state ON tasks WHEN OLD.state <> 'AwaitingReview' AND NEW.state = 'AwaitingReview' BEGIN INSERT INTO task_result_completion_epochs(task_id,completion_epoch) VALUES(NEW.id,1) ON CONFLICT(task_id) DO UPDATE SET completion_epoch=completion_epoch+1; END",
        "CREATE TRIGGER IF NOT EXISTS task_results_cleanup BEFORE DELETE ON tasks BEGIN DELETE FROM verification_receipts WHERE task_id=OLD.id; DELETE FROM task_result_references WHERE task_id=OLD.id; DELETE FROM task_result_revisions WHERE task_id=OLD.id; DELETE FROM task_result_completion_epochs WHERE task_id=OLD.id; END",
    ] {
        sqlx::query(sql).execute(&mut *tx).await?;
    }
    crate::db::add_column_if_missing(
        &mut *tx,
        "verification_receipts",
        "source_id TEXT NOT NULL DEFAULT ''",
    )
    .await?;
    crate::db::add_column_if_missing(&mut *tx, "verification_receipts", "report_json TEXT").await?;
    // Migration has no historical transition ledger for desktop turns.  Each
    // already-completed review state receives one baseline epoch; subsequent
    // canonical re-entry is counted by the trigger above.
    sqlx::query(
        "INSERT INTO task_result_completion_epochs(task_id,completion_epoch) \
         SELECT id,CASE WHEN state='AwaitingReview' THEN 1 ELSE 0 END FROM tasks WHERE true \
         ON CONFLICT(task_id) DO NOTHING",
    )
    .execute(&mut *tx)
    .await?;

    // A pre-Phase-2 evidence row has no source or command-version binding.  Keep
    // it visible as a receipt but make it ineligible for the current-ready gate.
    let legacy: Vec<LegacyEvidenceRow> =
        sqlx::query_as(
            "SELECT e.task_id,e.build_cmd,e.build_exit,e.test_cmd,e.test_exit,e.created_at FROM evidence e \
             WHERE NOT EXISTS(SELECT 1 FROM verification_receipts r WHERE r.task_id=e.task_id)",
        )
        .fetch_all(&mut *tx)
        .await?;
    for (task_id, build_cmd, build_exit, test_cmd, test_exit, created_at) in legacy {
        let completion_epoch = completion_epoch_tx(&mut tx, task_id).await?;
        let revision = ensure_revision_tx(
            &mut tx,
            task_id,
            &source_id,
            None,
            completion_epoch,
            created_at,
        )
        .await?;
        let hash = command_hash_parts(build_cmd.as_deref(), test_cmd.as_deref(), 0);
        let run_id = format!("legacy:{task_id}:{created_at}");
        sqlx::query("INSERT OR IGNORE INTO verification_receipts(verify_run_id,task_id,source_id,result_revision,started_at,finished_at,command_spec_hash,source_before,source_after,outcome,freshness,logs_ref,observation_scope) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)")
            .bind(run_id).bind(task_id).bind(&source_id).bind(revision).bind(created_at).bind(created_at).bind(hash)
            .bind(None::<String>).bind(None::<String>)
            .bind(if build_exit == Some(0) && test_exit.unwrap_or(0) == 0 { "passed" } else { "failed" })
            .bind(UNKNOWN).bind(None::<String>).bind(LEGACY).execute(&mut *tx).await?;
        sqlx::query("UPDATE evidence SET ready=0 WHERE task_id=?")
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn snapshot(pool: &SqlitePool, task_id: i64) -> Result<TaskResultSnapshot, String> {
    let task = db::get_task(pool, task_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
    let source_id = crate::notifications::source_id(pool)
        .await
        .map_err(|e| e.to_string())?;
    let spec = crate::verify::detect_spec(Path::new(&task.worktree_path));
    let observation = fingerprint_with_base(
        Path::new(&task.worktree_path),
        &spec,
        task.base_revision.as_deref().or(Some(task.base.as_str())),
    );
    let fingerprint = observation.fingerprint();
    let epoch = completion_epoch(pool, task_id).await?;
    let revision = ensure_revision(
        pool,
        task_id,
        &source_id,
        fingerprint.as_deref(),
        epoch,
        now(),
    )
    .await?;
    // Snapshot freshness is only a source-observation statement.  A person may
    // review a current source without a passing check; check outcome remains on
    // the receipt and is what `refresh_evidence` gates on.
    let freshness = if task.state == db::state::RUNNING || fingerprint.is_none() {
        UNKNOWN.into()
    } else {
        CURRENT.into()
    };
    load_snapshot(
        pool,
        task_id,
        source_id,
        revision,
        epoch,
        fingerprint,
        freshness,
        Path::new(&task.worktree_path),
    )
    .await
}

/// Returns true only when the latest evidence can be proved current.  It also
/// clears the legacy compatibility projection when it cannot be proved current.
pub async fn refresh_evidence(pool: &SqlitePool, task_id: i64) -> Result<bool, String> {
    let result = snapshot(pool, task_id).await?;
    let spec = crate::verify::detect_spec(Path::new(
        &db::get_task(pool, task_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?
            .worktree_path,
    ));
    let ready = result.freshness == CURRENT
        && result.receipts.first().is_some_and(|receipt| {
            receipt.result_revision == result.result_revision
                && receipt.outcome == "passed"
                && receipt.freshness == CURRENT
                && receipt.source_id == result.source_id
                && receipt.source_after == result.fingerprint
                && receipt.command_spec_hash == command_hash(&spec)
        });
    sqlx::query("UPDATE evidence SET ready=? WHERE task_id=?")
        .bind(i64::from(ready))
        .bind(task_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ready)
}

/// Database-side CAS for a snapshot already refreshed by the caller.  The
/// caller must obtain a new [`snapshot`] immediately before beginning a review
/// mutation; this fence prevents a reset, completed round, or persisted source
/// observation from being silently accepted between the read and write.
pub async fn validate_snapshot_tx(
    tx: &mut Transaction<'_, Sqlite>,
    task_id: i64,
    snapshot: &TaskResultSnapshot,
) -> Result<(), String> {
    if snapshot.task_id != task_id {
        return Err("결과 스냅샷의 작업이 일치하지 않습니다".into());
    }
    let current: Option<(String, i64, i64, Option<String>, String)> = sqlx::query_as(
        "SELECT r.source_id,r.result_revision,r.completion_epoch,r.fingerprint, \
         COALESCE((SELECT value FROM settings WHERE key='notification_source_id'),'') \
         FROM task_result_revisions r WHERE r.task_id=? ORDER BY r.result_revision DESC LIMIT 1",
    )
    .bind(task_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    match current {
        Some((source, revision, epoch, fingerprint, live_source))
            if source == snapshot.source_id
                && revision == snapshot.result_revision
                && epoch == snapshot.completion_epoch
                && fingerprint == snapshot.fingerprint
                && live_source == snapshot.source_id =>
        {
            let live_epoch: i64 = sqlx::query_scalar(
                        "SELECT COALESCE((SELECT completion_epoch FROM task_result_completion_epochs WHERE task_id=?),0)",
                    )
                    .bind(task_id)
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(|e| e.to_string())?;
            if live_epoch == snapshot.completion_epoch {
                Ok(())
            } else {
                Err("새 실행 회차 결과가 있어 검토를 기록할 수 없습니다 — 새로고침하세요".into())
            }
        }
        _ => Err("결과가 변경되어 검토를 기록할 수 없습니다 — 새로고침하세요".into()),
    }
}

pub async fn add_reference(
    pool: &SqlitePool,
    task_id: i64,
    input: ReferenceInput,
) -> Result<TaskResultSnapshot, String> {
    if !matches!(input.kind.as_str(), "file" | "preview" | "report") {
        return Err("결과 참조 종류가 유효하지 않습니다".into());
    }
    let current = snapshot(pool, task_id).await?;
    if current.source_id != input.expected_source_id
        || current.result_revision != input.expected_revision
    {
        return Err("결과가 변경되어 참조를 등록할 수 없습니다 — 새로고침하세요".into());
    }
    let task = db::get_task(pool, task_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
    let relative = validate_reference(Path::new(&task.worktree_path), &input.relative_path)?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    validate_snapshot_tx(&mut tx, task_id, &current).await?;
    let changed = sqlx::query("INSERT INTO task_result_references(task_id,result_revision,kind,relative_path,created_at) SELECT ?,?,?,?,? WHERE EXISTS(SELECT 1 FROM task_result_revisions WHERE task_id=? AND result_revision=? AND source_id=?) ON CONFLICT(task_id,result_revision,kind) DO UPDATE SET relative_path=excluded.relative_path,created_at=excluded.created_at")
        .bind(task_id).bind(current.result_revision).bind(&input.kind).bind(&relative).bind(now())
        .bind(task_id).bind(current.result_revision).bind(&current.source_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    if changed.rows_affected() != 1 {
        return Err("결과가 변경되어 참조를 등록할 수 없습니다 — 새로고침하세요".into());
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    snapshot(pool, task_id).await
}

pub(crate) async fn persist_verify_receipt_tx(
    tx: &mut Transaction<'_, Sqlite>,
    task_id: i64,
    report: &VerifyReport,
    _root: &Path,
    source_before: &FingerprintObservation,
    source_after: &FingerprintObservation,
    task_state: &str,
    started_at: i64,
    finished_at: i64,
) -> Result<bool, String> {
    let source_id: String =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='notification_source_id'")
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| "unknown".into());
    let epoch = completion_epoch_tx(tx, task_id)
        .await
        .map_err(|e| e.to_string())?;
    let after = source_after.fingerprint();
    let revision = ensure_revision_tx(
        tx,
        task_id,
        &source_id,
        after.as_deref(),
        epoch,
        finished_at,
    )
    .await
    .map_err(|e| e.to_string())?;
    let source_changed = source_before.fingerprint() != after;
    let fresh = if task_state == db::state::RUNNING
        || source_changed
        || !source_before.is_complete()
        || !source_after.is_complete()
    {
        UNKNOWN
    } else {
        CURRENT
    };
    let outcome = if report.build.is_none() && report.test.is_none() {
        "not_run"
    } else if report
        .build
        .iter()
        .chain(report.test.iter())
        .any(|check| check.exit_code < 0)
    {
        "infrastructure_error"
    } else if report.ready {
        "passed"
    } else {
        "failed"
    };
    let run_id = crate::preview_bridge::random_hex_id()?;
    sqlx::query("INSERT INTO verification_receipts(verify_run_id,task_id,source_id,result_revision,started_at,finished_at,command_spec_hash,source_before,source_after,outcome,freshness,logs_ref,observation_scope,report_json) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
        .bind(run_id).bind(task_id).bind(&source_id).bind(revision).bind(started_at).bind(finished_at)
        .bind(command_hash(&report.spec)).bind(source_before.fingerprint()).bind(after)
        .bind(outcome).bind(fresh).bind(None::<String>).bind(source_after.scope()).bind(serde_json::to_string(report).map_err(|e|e.to_string())?)
        .execute(&mut **tx).await.map_err(|e| e.to_string())?;
    Ok(fresh == CURRENT && outcome == "passed")
}

async fn load_snapshot(
    pool: &SqlitePool,
    task_id: i64,
    source_id: String,
    result_revision: i64,
    completion_epoch: i64,
    fingerprint: Option<String>,
    freshness: String,
    root: &Path,
) -> Result<TaskResultSnapshot, String> {
    let mut receipts: Vec<VerificationReceipt> = sqlx::query_as::<_, ReceiptRow>("SELECT verify_run_id,source_id,result_revision,started_at,finished_at,command_spec_hash,source_before,source_after,outcome,freshness,logs_ref,observation_scope,report_json FROM verification_receipts WHERE task_id=? ORDER BY rowid DESC")
        .bind(task_id).fetch_all(pool).await.map_err(|e| e.to_string())?.into_iter().map(Into::into).collect();
    for receipt in &mut receipts {
        if receipt.freshness == CURRENT {
            receipt.freshness = if freshness != CURRENT {
                UNKNOWN
            } else if receipt.source_id != source_id
                || receipt.result_revision != result_revision
                || receipt.source_after != fingerprint
            {
                "stale"
            } else {
                CURRENT
            }
            .into();
        }
    }
    let references = sqlx::query_as::<_, ReferenceRow>("SELECT r.result_revision,r.kind,r.relative_path,r.created_at FROM task_result_references r WHERE r.task_id=? AND NOT EXISTS(SELECT 1 FROM task_result_references newer WHERE newer.task_id=r.task_id AND newer.kind=r.kind AND newer.result_revision>r.result_revision) ORDER BY r.kind")
        .bind(task_id).fetch_all(pool).await.map_err(|e| e.to_string())?.into_iter().map(|row| ResultReference { availability: reference_availability(root, &row.relative_path), ..row.into() }).collect();
    Ok(TaskResultSnapshot {
        task_id,
        source_id,
        result_revision,
        completion_epoch,
        freshness,
        fingerprint,
        receipts,
        references,
    })
}

#[derive(sqlx::FromRow)]
struct ReceiptRow {
    verify_run_id: String,
    source_id: String,
    result_revision: i64,
    started_at: i64,
    finished_at: i64,
    command_spec_hash: String,
    source_before: Option<String>,
    source_after: Option<String>,
    outcome: String,
    freshness: String,
    logs_ref: Option<String>,
    observation_scope: String,
    report_json: Option<String>,
}
impl From<ReceiptRow> for VerificationReceipt {
    fn from(r: ReceiptRow) -> Self {
        Self {
            verify_run_id: r.verify_run_id,
            source_id: r.source_id,
            result_revision: r.result_revision,
            started_at: r.started_at,
            finished_at: r.finished_at,
            command_spec_hash: r.command_spec_hash,
            source_before: r.source_before,
            source_after: r.source_after,
            outcome: r.outcome,
            freshness: r.freshness,
            logs_ref: r.logs_ref,
            observation_scope: r.observation_scope,
            report: r
                .report_json
                .and_then(|raw| serde_json::from_str(&raw).ok()),
        }
    }
}
#[derive(sqlx::FromRow)]
struct ReferenceRow {
    result_revision: i64,
    kind: String,
    relative_path: String,
    created_at: i64,
}
impl From<ReferenceRow> for ResultReference {
    fn from(r: ReferenceRow) -> Self {
        Self {
            result_revision: r.result_revision,
            kind: r.kind,
            relative_path: r.relative_path,
            availability: String::new(),
            created_at: r.created_at,
        }
    }
}

async fn ensure_revision(
    pool: &SqlitePool,
    task_id: i64,
    source_id: &str,
    fingerprint: Option<&str>,
    epoch: i64,
    created_at: i64,
) -> Result<i64, String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let revision = ensure_revision_tx(&mut tx, task_id, source_id, fingerprint, epoch, created_at)
        .await
        .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(revision)
}
async fn ensure_revision_tx(
    tx: &mut Transaction<'_, Sqlite>,
    task_id: i64,
    source_id: &str,
    fingerprint: Option<&str>,
    epoch: i64,
    created_at: i64,
) -> anyhow::Result<i64> {
    let current:Option<(i64,String,Option<String>,i64)>=sqlx::query_as("SELECT result_revision,source_id,fingerprint,completion_epoch FROM task_result_revisions WHERE task_id=? ORDER BY result_revision DESC LIMIT 1").bind(task_id).fetch_optional(&mut **tx).await?;
    if let Some((revision, old_source, old_fingerprint, old_epoch)) = current {
        if old_source == source_id
            && old_fingerprint.as_deref() == fingerprint
            && old_epoch == epoch
        {
            return Ok(revision);
        }
        let next = revision + 1;
        sqlx::query("INSERT INTO task_result_revisions(task_id,result_revision,source_id,fingerprint,completion_epoch,created_at) VALUES(?,?,?,?,?,?)").bind(task_id).bind(next).bind(source_id).bind(fingerprint).bind(epoch).bind(created_at).execute(&mut **tx).await?;
        Ok(next)
    } else {
        sqlx::query("INSERT INTO task_result_revisions(task_id,result_revision,source_id,fingerprint,completion_epoch,created_at) VALUES(?,?,?,?,?,?)").bind(task_id).bind(1_i64).bind(source_id).bind(fingerprint).bind(epoch).bind(created_at).execute(&mut **tx).await?;
        Ok(1)
    }
}
async fn completion_epoch(pool: &SqlitePool, task_id: i64) -> Result<i64, String> {
    sqlx::query_scalar("SELECT COALESCE((SELECT completion_epoch FROM task_result_completion_epochs WHERE task_id=?),0)").bind(task_id).fetch_one(pool).await.map_err(|e|e.to_string())
}
async fn completion_epoch_tx(
    tx: &mut Transaction<'_, Sqlite>,
    task_id: i64,
) -> anyhow::Result<i64> {
    sqlx::query_scalar("SELECT COALESCE((SELECT completion_epoch FROM task_result_completion_epochs WHERE task_id=?),0)").bind(task_id).fetch_one(&mut **tx).await.map_err(Into::into)
}
fn command_hash(spec: &ValidateSpec) -> String {
    command_hash_parts(
        spec.build.as_deref(),
        spec.test.as_deref(),
        spec.timeout_secs,
    )
}
fn command_hash_parts(build: Option<&str>, test: Option<&str>, timeout: u64) -> String {
    let raw = format!(
        "v1\\0{}\\0{}\\0{timeout}",
        build.unwrap_or(""),
        test.unwrap_or("")
    );
    format!("sha256:{:x}", Sha256::digest(raw.as_bytes()))
}
fn validate_reference(root: &Path, relative: &str) -> Result<String, String> {
    let input = Path::new(relative);
    if relative.is_empty()
        || input.is_absolute()
        || input.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("결과 참조는 워크트리 상대 경로여야 합니다".into());
    }
    let root = root
        .canonicalize()
        .map_err(|_| "작업 워크트리를 해석할 수 없습니다".to_string())?;
    let resolved = root
        .join(input)
        .canonicalize()
        .map_err(|_| "결과 참조 파일이 없습니다".to_string())?;
    let safe = resolved
        .strip_prefix(&root)
        .map_err(|_| "결과 참조가 워크트리 밖을 가리킵니다".to_string())?;
    Ok(safe.to_string_lossy().replace("\\", "/"))
}
fn reference_availability(root: &Path, relative: &str) -> String {
    let Ok(root) = root.canonicalize() else {
        return "unavailable".into();
    };
    let resolved = root.join(relative);
    match resolved.canonicalize() {
        Ok(path) if path.starts_with(&root) => "available".into(),
        Ok(_) => "unavailable".into(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "missing".into(),
        Err(_) => "unavailable".into(),
    }
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE tasks(id INTEGER PRIMARY KEY,state TEXT NOT NULL,updated_at INTEGER NOT NULL); CREATE TABLE evidence(task_id INTEGER PRIMARY KEY,build_cmd TEXT,build_exit INTEGER,test_cmd TEXT,test_exit INTEGER,passed INTEGER NOT NULL DEFAULT 0,failed INTEGER NOT NULL DEFAULT 0,ready INTEGER NOT NULL,created_at INTEGER NOT NULL)")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO tasks(id,state,updated_at) VALUES(1,'AwaitingReview',10)")
            .execute(&pool)
            .await
            .unwrap();
        migrate(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn completed_reentry_advances_revision_without_file_changes() {
        let pool = pool().await;
        let source_id = crate::notifications::source_id(&pool).await.unwrap();
        let epoch = completion_epoch(&pool, 1).await.unwrap();
        let first = ensure_revision(&pool, 1, &source_id, Some("git-v1:first"), epoch, 10)
            .await
            .unwrap();
        sqlx::query("UPDATE tasks SET state='Running' WHERE id=1; UPDATE tasks SET state='AwaitingReview' WHERE id=1")
            .execute(&pool).await.unwrap();
        let epoch = completion_epoch(&pool, 1).await.unwrap();
        let second = ensure_revision(&pool, 1, &source_id, Some("git-v1:first"), epoch, 11)
            .await
            .unwrap();
        assert_eq!(first, 1);
        assert_eq!(second, 2);
        assert_eq!(epoch, 2);
    }

    #[tokio::test]
    async fn legacy_evidence_is_migrated_unknown_and_cleared() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE tasks(id INTEGER PRIMARY KEY,state TEXT NOT NULL,updated_at INTEGER NOT NULL); CREATE TABLE evidence(task_id INTEGER PRIMARY KEY,build_cmd TEXT,build_exit INTEGER,test_cmd TEXT,test_exit INTEGER,passed INTEGER NOT NULL DEFAULT 0,failed INTEGER NOT NULL DEFAULT 0,ready INTEGER NOT NULL,created_at INTEGER NOT NULL); INSERT INTO tasks VALUES(1,'AwaitingReview',10); INSERT INTO evidence VALUES(1,'cargo test',0,'',-1,1,0,1,9)")
            .execute(&pool).await.unwrap();
        migrate(&pool).await.unwrap();
        let freshness: String =
            sqlx::query_scalar("SELECT freshness FROM verification_receipts WHERE task_id=1")
                .fetch_one(&pool)
                .await
                .unwrap();
        let ready: i64 = sqlx::query_scalar("SELECT ready FROM evidence WHERE task_id=1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(freshness, UNKNOWN);
        assert_eq!(ready, 0);
    }

    #[test]
    fn reference_paths_cannot_escape_through_a_symlink() {
        let root =
            crate::testtmp::dir().join(format!("task-result-reference-{}", std::process::id()));
        let outside =
            crate::testtmp::dir().join(format!("task-result-outside-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "no").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("escape.txt")).unwrap();
        #[cfg(unix)]
        assert!(validate_reference(&root, "escape.txt").is_err());
    }

    #[tokio::test]
    async fn verification_during_source_change_is_unknown_and_not_ready() {
        let pool = pool().await;
        let root =
            crate::testtmp::dir().join(format!("task-result-verify-race-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for args in [
            ["init", "-q"].as_slice(),
            ["config", "user.email", "test@example.invalid"].as_slice(),
            ["config", "user.name", "Test"].as_slice(),
        ] {
            assert!(std::process::Command::new("git")
                .current_dir(&root)
                .args(args)
                .status()
                .unwrap()
                .success());
        }
        std::fs::write(root.join("file.txt"), "before\n").unwrap();
        assert!(std::process::Command::new("git")
            .current_dir(&root)
            .args(["add", "file.txt"])
            .status()
            .unwrap()
            .success());
        assert!(std::process::Command::new("git")
            .current_dir(&root)
            .args(["commit", "-qm", "initial"])
            .status()
            .unwrap()
            .success());
        let spec = ValidateSpec::default();
        let before = fingerprint(&root, &spec);
        std::fs::write(root.join("file.txt"), "after\n").unwrap();
        let after = fingerprint(&root, &spec);
        let report = VerifyReport {
            spec,
            build: Some(crate::verify::CheckResult {
                command: "true".into(),
                exit_code: 0,
                tail: String::new(),
            }),
            test: None,
            summary: None,
            ready: true,
            checks: vec![("build".into(), true)],
            warnings: vec![],
        };
        let mut tx = pool.begin().await.unwrap();
        assert!(!persist_verify_receipt_tx(
            &mut tx,
            1,
            &report,
            &root,
            &before,
            &after,
            db::state::AWAITING_REVIEW,
            1,
            2
        )
        .await
        .unwrap());
        tx.commit().await.unwrap();
        let freshness: String =
            sqlx::query_scalar("SELECT freshness FROM verification_receipts WHERE task_id=1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(freshness, UNKNOWN);
    }
}
