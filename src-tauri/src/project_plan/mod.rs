//! Durable project purpose and graph annotations.
//!
//! This module deliberately owns only local, descriptive planning data.  It does
//! not schedule tasks or alter GoalContract authority.  Command handlers must
//! resolve a canonical local repository before calling these functions; accepting
//! an arbitrary client path here would make the DB scope ambiguous.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use std::collections::{HashMap, HashSet};
use std::fmt;

pub const SCHEMA_VERSION: u8 = 2;
pub const MAX_PHASES: usize = 32;
pub const MAX_BINDINGS: usize = 10_000;
pub const MAX_DEPENDENCIES: usize = 2_000;
pub const MAX_PLAN_BYTES: usize = 512 * 1024;
pub const MAX_OBJECTIVE_BYTES: usize = 2_000;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProjectPlan {
    pub schema_version: u8,
    /// DB-generation fence.  UI must return this value with a save CAS.
    #[serde(default)]
    pub source_id: String,
    /// Monotonic only within `(source_id, canonical_repo)`.
    pub revision: i64,
    pub repo: String,
    #[serde(default)]
    pub objective: String,
    #[serde(default)]
    pub phases: Vec<Phase>,
    #[serde(default)]
    pub task_bindings: Vec<TaskBinding>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Phase {
    /// Immutable relation key.  Rename changes `name`, never this field.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub objective: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub struct TaskBinding {
    pub task_id: i64,
    pub phase_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub struct Dependency {
    pub from: i64,
    pub to: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, sqlx::FromRow)]
pub struct PurposeSnapshot {
    pub source_id: String,
    pub task_id: i64,
    pub execution_round_id: String,
    pub plan_revision: Option<i64>,
    pub phase_id: Option<String>,
    pub project_objective: String,
    pub phase_objective: String,
    pub captured_at: i64,
}

/// The selection displayed before an initial task starts.  It is a descriptive
/// plan choice, never a substitute for the task's GoalContract.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PurposeSelection {
    pub source_id: String,
    pub canonical_repo: String,
    pub plan_revision: i64,
    pub phase_id: Option<String>,
}

/// Read-only candidate prepared before a conversation admission transaction.
/// Keep this opaque to command callers: [`persist_purpose_tx`] revalidates the
/// plan source/revision when the candidate came from an editable plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedPurpose {
    snapshot: PurposeSnapshot,
    guard: Option<PurposeSelection>,
    consume_refresh: bool,
}

impl PreparedPurpose {
    pub fn snapshot(&self) -> &PurposeSnapshot {
        &self.snapshot
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImportOutcome {
    Imported {
        plan: ProjectPlan,
    },
    AlreadyImported {
        plan: ProjectPlan,
    },
    ExistingPlanConflict {
        plan: ProjectPlan,
        legacy_hash: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
    Invalid(String),
    RevisionConflict { current: Option<ProjectPlan> },
    ImportInvalid(String),
    TaskScope { task_id: i64 },
    PurposeConflict { current: ProjectPlan },
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(f, "project plan is invalid: {message}"),
            Self::RevisionConflict { .. } => write!(f, "project plan revision conflict"),
            Self::ImportInvalid(message) => write!(f, "legacy project graph is invalid: {message}"),
            Self::TaskScope { task_id } => {
                write!(f, "task #{task_id} is outside this local project")
            }
            Self::PurposeConflict { .. } => write!(f, "purpose preview is no longer current"),
        }
    }
}

impl std::error::Error for PlanError {}

/// Additive local schema.  The caller owns migration ordering in `db::init_pool`.
pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    for statement in [
        "CREATE TABLE IF NOT EXISTS project_plans (source_id TEXT NOT NULL, canonical_repo TEXT NOT NULL, revision INTEGER NOT NULL, plan_json TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, PRIMARY KEY(source_id, canonical_repo))",
        "CREATE TABLE IF NOT EXISTS project_plan_imports (source_id TEXT NOT NULL, canonical_repo TEXT NOT NULL, import_hash TEXT NOT NULL, imported_at INTEGER NOT NULL, PRIMARY KEY(source_id, canonical_repo, import_hash))",
        "CREATE TABLE IF NOT EXISTS purpose_snapshots (source_id TEXT NOT NULL, task_id INTEGER NOT NULL, execution_round_id TEXT NOT NULL, plan_revision INTEGER, phase_id TEXT, project_objective TEXT NOT NULL, phase_objective TEXT NOT NULL, captured_at INTEGER NOT NULL, PRIMARY KEY(source_id, task_id, execution_round_id))",
        "CREATE TABLE IF NOT EXISTS purpose_refresh_requests (source_id TEXT NOT NULL, task_id INTEGER NOT NULL, canonical_repo TEXT NOT NULL, expected_plan_revision INTEGER NOT NULL, phase_id TEXT, requested_at INTEGER NOT NULL, PRIMARY KEY(source_id, task_id))",
        "CREATE TABLE IF NOT EXISTS task_purpose_bindings (task_id INTEGER PRIMARY KEY, source_id TEXT NOT NULL, canonical_repo TEXT NOT NULL, plan_revision INTEGER NOT NULL, phase_id TEXT, project_objective TEXT NOT NULL DEFAULT '', phase_objective TEXT NOT NULL DEFAULT '', inherited_from_task_id INTEGER, created_at INTEGER NOT NULL)",
        "CREATE INDEX IF NOT EXISTS idx_purpose_snapshots_task ON purpose_snapshots(source_id, task_id, captured_at DESC)",
    ] {
        sqlx::query(statement).execute(&mut *tx).await?;
    }
    crate::db::add_column_if_missing(
        &mut *tx,
        "task_purpose_bindings",
        "project_objective TEXT NOT NULL DEFAULT ''",
    )
    .await?;
    crate::db::add_column_if_missing(
        &mut *tx,
        "task_purpose_bindings",
        "phase_objective TEXT NOT NULL DEFAULT ''",
    )
    .await?;
    sqlx::query("CREATE TRIGGER IF NOT EXISTS purpose_snapshots_immutable BEFORE UPDATE ON purpose_snapshots BEGIN SELECT RAISE(ABORT,'purpose snapshot is immutable'); END").execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn get(
    pool: &SqlitePool,
    source_id: &str,
    canonical_repo: &str,
) -> anyhow::Result<Option<ProjectPlan>> {
    validate_scope(source_id, canonical_repo)?;
    let row =
        sqlx::query("SELECT plan_json FROM project_plans WHERE source_id=? AND canonical_repo=?")
            .bind(source_id)
            .bind(canonical_repo)
            .fetch_optional(pool)
            .await?;
    row.map(|row| {
        decode_plan(
            &row.get::<String, _>("plan_json"),
            canonical_repo,
            source_id,
        )
    })
    .transpose()
}

/// Save a complete plan by compare-and-swap.  `expected_revision=0` creates the
/// first revision.  Missing tasks retained from the previously saved plan remain
/// historical/dangling; newly introduced task ids must be live in this repository.
pub async fn save(
    pool: &SqlitePool,
    source_id: &str,
    canonical_repo: &str,
    mut proposed: ProjectPlan,
    expected_revision: i64,
    expected_source_id: &str,
    now: i64,
) -> Result<ProjectPlan, PlanError> {
    validate_scope(source_id, canonical_repo)
        .map_err(|error| PlanError::Invalid(error.to_string()))?;
    if expected_source_id != source_id {
        return Err(PlanError::RevisionConflict {
            current: get(pool, source_id, canonical_repo)
                .await
                .map_err(internal)?,
        });
    }
    if expected_revision < 0 {
        return Err(PlanError::Invalid(
            "expected_revision must be non-negative".into(),
        ));
    }
    let mut tx = pool.begin().await.map_err(internal)?;
    let live_source: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='notification_source_id'")
            .fetch_optional(&mut *tx)
            .await
            .map_err(internal)?;
    if live_source.as_deref() != Some(source_id) {
        return Err(PlanError::Invalid("plan source changed".into()));
    }
    let row = sqlx::query(
        "SELECT revision, plan_json FROM project_plans WHERE source_id=? AND canonical_repo=?",
    )
    .bind(source_id)
    .bind(canonical_repo)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?;
    let current = row
        .as_ref()
        .map(|row| {
            decode_plan(
                &row.get::<String, _>("plan_json"),
                canonical_repo,
                source_id,
            )
        })
        .transpose()
        .map_err(internal)?;
    let current_revision = current.as_ref().map(|plan| plan.revision).unwrap_or(0);
    if current_revision != expected_revision {
        return Err(PlanError::RevisionConflict { current });
    }
    proposed.schema_version = SCHEMA_VERSION;
    proposed.source_id = source_id.into();
    proposed.revision = current_revision + 1;
    proposed.repo = canonical_repo.into();
    validate_plan(&proposed).map_err(PlanError::Invalid)?;
    validate_new_task_scope(&mut tx, canonical_repo, &proposed, current.as_ref()).await?;
    let json = serde_json::to_string(&proposed).map_err(internal)?;
    if current_revision == 0 {
        sqlx::query("INSERT INTO project_plans(source_id, canonical_repo, revision, plan_json, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(source_id).bind(canonical_repo).bind(proposed.revision).bind(&json).bind(now).bind(now).execute(&mut *tx).await.map_err(internal)?;
    } else {
        let result = sqlx::query("UPDATE project_plans SET revision=?, plan_json=?, updated_at=? WHERE source_id=? AND canonical_repo=? AND revision=?")
            .bind(proposed.revision).bind(&json).bind(now).bind(source_id).bind(canonical_repo).bind(expected_revision).execute(&mut *tx).await.map_err(internal)?;
        if result.rows_affected() != 1 {
            return Err(PlanError::RevisionConflict {
                current: get(pool, source_id, canonical_repo)
                    .await
                    .map_err(internal)?,
            });
        }
    }
    tx.commit().await.map_err(internal)?;
    Ok(proposed)
}

/// Parse and atomically import the raw v1 browser value.  It never deletes or
/// alters that raw value; callers retain it in localStorage even after success.
pub async fn import_v1(
    pool: &SqlitePool,
    source_id: &str,
    canonical_repo: &str,
    raw: &str,
    now: i64,
) -> Result<ImportOutcome, PlanError> {
    validate_scope(source_id, canonical_repo)
        .map_err(|error| PlanError::ImportInvalid(error.to_string()))?;
    let hash = sha256(raw);
    let candidate = legacy_plan(raw, canonical_repo).map_err(PlanError::ImportInvalid)?;
    let mut tx = pool.begin().await.map_err(internal)?;
    let existing =
        sqlx::query("SELECT plan_json FROM project_plans WHERE source_id=? AND canonical_repo=?")
            .bind(source_id)
            .bind(canonical_repo)
            .fetch_optional(&mut *tx)
            .await
            .map_err(internal)?;
    if let Some(row) = existing {
        let plan = decode_plan(
            &row.get::<String, _>("plan_json"),
            canonical_repo,
            source_id,
        )
        .map_err(internal)?;
        let imported: Option<i64> = sqlx::query_scalar("SELECT 1 FROM project_plan_imports WHERE source_id=? AND canonical_repo=? AND import_hash=?")
            .bind(source_id).bind(canonical_repo).bind(&hash).fetch_optional(&mut *tx).await.map_err(internal)?;
        return if imported.is_some() {
            Ok(ImportOutcome::AlreadyImported { plan })
        } else {
            Ok(ImportOutcome::ExistingPlanConflict {
                plan,
                legacy_hash: hash,
            })
        };
    }
    let mut candidate = candidate;
    candidate.source_id = source_id.into();
    validate_plan(&candidate).map_err(PlanError::ImportInvalid)?;
    validate_new_task_scope(&mut tx, canonical_repo, &candidate, None).await?;
    let json = serde_json::to_string(&candidate).map_err(internal)?;
    sqlx::query("INSERT INTO project_plans(source_id, canonical_repo, revision, plan_json, created_at, updated_at) VALUES (?, ?, 1, ?, ?, ?)")
        .bind(source_id).bind(canonical_repo).bind(&json).bind(now).bind(now).execute(&mut *tx).await.map_err(internal)?;
    sqlx::query("INSERT INTO project_plan_imports(source_id, canonical_repo, import_hash, imported_at) VALUES (?, ?, ?, ?)")
        .bind(source_id).bind(canonical_repo).bind(&hash).bind(now).execute(&mut *tx).await.map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(ImportOutcome::Imported { plan: candidate })
}

/// Request that the next execution round use the selected current plan revision.
/// It has no execution side effect and is intentionally separate from capture.
pub async fn request_refresh(
    pool: &SqlitePool,
    source_id: &str,
    task_id: i64,
    canonical_repo: &str,
    expected_plan_revision: i64,
    phase_id: Option<&str>,
    now: i64,
) -> Result<ProjectPlan, PlanError> {
    let plan = get(pool, source_id, canonical_repo)
        .await
        .map_err(internal)?
        .ok_or_else(|| PlanError::Invalid("no project plan exists".into()))?;
    if plan.revision != expected_plan_revision {
        return Err(PlanError::PurposeConflict { current: plan });
    }
    validate_task_live(pool, canonical_repo, task_id).await?;
    if let Some(id) = phase_id {
        if !plan.phases.iter().any(|phase| phase.id == id) {
            return Err(PlanError::Invalid(
                "phase_id does not belong to this plan".into(),
            ));
        }
    }
    sqlx::query("INSERT INTO purpose_refresh_requests(source_id, task_id, canonical_repo, expected_plan_revision, phase_id, requested_at) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(source_id, task_id) DO UPDATE SET canonical_repo=excluded.canonical_repo, expected_plan_revision=excluded.expected_plan_revision, phase_id=excluded.phase_id, requested_at=excluded.requested_at")
        .bind(source_id).bind(task_id).bind(canonical_repo).bind(expected_plan_revision).bind(phase_id).bind(now).execute(pool).await.map_err(internal)?;
    Ok(plan)
}

/// Associate a newly created task with the user-selected plan revision.  This is
/// metadata only; no prompt text is recorded until an actual execution round is
/// admitted.  A stale preview is rejected before the task can start with it.
pub async fn bind_initial(
    pool: &SqlitePool,
    task_id: i64,
    selection: &PurposeSelection,
    now: i64,
) -> Result<(), PlanError> {
    validate_scope(&selection.source_id, &selection.canonical_repo)
        .map_err(|error| PlanError::Invalid(error.to_string()))?;
    validate_task_live(pool, &selection.canonical_repo, task_id).await?;
    let plan = get(pool, &selection.source_id, &selection.canonical_repo)
        .await
        .map_err(internal)?
        .ok_or_else(|| PlanError::Invalid("no project plan exists".into()))?;
    if plan.revision != selection.plan_revision {
        return Err(PlanError::PurposeConflict { current: plan });
    }
    if let Some(id) = selection.phase_id.as_deref() {
        if !plan.phases.iter().any(|phase| phase.id == id) {
            return Err(PlanError::Invalid(
                "phase_id does not belong to this plan".into(),
            ));
        }
    }
    let phase_objective = selection
        .phase_id
        .as_deref()
        .and_then(|id| plan.phases.iter().find(|phase| phase.id == id))
        .map(|phase| phase.objective.clone())
        .unwrap_or_default();
    let mut next = plan.clone();
    if let Some(phase_id) = selection.phase_id.as_deref() {
        next.task_bindings
            .retain(|binding| binding.task_id != task_id);
        next.task_bindings.push(TaskBinding {
            task_id,
            phase_id: phase_id.into(),
        });
        next.revision += 1;
    }
    let next_json = serde_json::to_string(&next).map_err(internal)?;
    let mut tx = pool.begin().await.map_err(internal)?;
    let current: Option<i64> = sqlx::query_scalar("SELECT revision FROM project_plans WHERE source_id=? AND canonical_repo=? AND source_id=(SELECT value FROM settings WHERE key='notification_source_id')")
        .bind(&selection.source_id).bind(&selection.canonical_repo).fetch_optional(&mut *tx).await.map_err(internal)?;
    if current != Some(selection.plan_revision) {
        return Err(PlanError::RevisionConflict { current: None });
    }
    sqlx::query("INSERT INTO task_purpose_bindings(task_id, source_id, canonical_repo, plan_revision, phase_id, project_objective, phase_objective, inherited_from_task_id, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, NULL, ?) ON CONFLICT(task_id) DO UPDATE SET source_id=excluded.source_id, canonical_repo=excluded.canonical_repo, plan_revision=excluded.plan_revision, phase_id=excluded.phase_id, project_objective=excluded.project_objective, phase_objective=excluded.phase_objective, inherited_from_task_id=NULL, created_at=excluded.created_at")
        .bind(task_id).bind(&selection.source_id).bind(&selection.canonical_repo).bind(selection.plan_revision).bind(&selection.phase_id).bind(&plan.objective).bind(phase_objective).bind(now).execute(&mut *tx).await.map_err(internal)?;
    if selection.phase_id.is_some() {
        let updated = sqlx::query("UPDATE project_plans SET revision=?, plan_json=?, updated_at=? WHERE source_id=? AND canonical_repo=? AND revision=?")
            .bind(next.revision).bind(next_json).bind(now).bind(&selection.source_id).bind(&selection.canonical_repo).bind(selection.plan_revision).execute(&mut *tx).await.map_err(internal)?;
        if updated.rows_affected() != 1 {
            return Err(PlanError::RevisionConflict { current: None });
        }
    }
    tx.commit().await.map_err(internal)?;
    Ok(())
}

/// A resumed task defaults to the source task's latest immutable purpose.  The
/// copied snapshot is written only at its first actual admission.
pub async fn inherit_task(
    pool: &SqlitePool,
    parent_task_id: i64,
    task_id: i64,
    now: i64,
) -> Result<(), PlanError> {
    let parent: Option<(String, String, i64, Option<String>, String, String)> = sqlx::query_as("SELECT source_id, canonical_repo, plan_revision, phase_id, project_objective, phase_objective FROM task_purpose_bindings WHERE task_id=?")
        .bind(parent_task_id).fetch_optional(pool).await.map_err(internal)?;
    let Some((
        source_id,
        canonical_repo,
        plan_revision,
        phase_id,
        project_objective,
        phase_objective,
    )) = parent
    else {
        return Ok(());
    };
    validate_task_live(pool, &canonical_repo, task_id).await?;
    sqlx::query("INSERT INTO task_purpose_bindings(task_id, source_id, canonical_repo, plan_revision, phase_id, project_objective, phase_objective, inherited_from_task_id, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(task_id) DO UPDATE SET source_id=excluded.source_id, canonical_repo=excluded.canonical_repo, plan_revision=excluded.plan_revision, phase_id=excluded.phase_id, project_objective=excluded.project_objective, phase_objective=excluded.phase_objective, inherited_from_task_id=excluded.inherited_from_task_id, created_at=excluded.created_at")
        .bind(task_id).bind(source_id).bind(canonical_repo).bind(plan_revision).bind(phase_id).bind(project_objective).bind(phase_objective).bind(parent_task_id).bind(now).execute(pool).await.map_err(internal)?;
    Ok(())
}

/// Resolve purpose text without writing it.  Call this before prompt composition,
/// then pass the candidate to [`persist_purpose_tx`] in the same transaction that
/// records the actual user/execution admission.
type PurposeBindingRow = (
    String,
    String,
    i64,
    Option<String>,
    String,
    String,
    Option<i64>,
);

pub async fn prepare_purpose(
    pool: &SqlitePool,
    task_id: i64,
    execution_round_id: &str,
    now: i64,
) -> Result<Option<PreparedPurpose>, PlanError> {
    validate_round_id(execution_round_id).map_err(PlanError::Invalid)?;
    let binding: Option<PurposeBindingRow> = sqlx::query_as("SELECT source_id, canonical_repo, plan_revision, phase_id, project_objective, phase_objective, inherited_from_task_id FROM task_purpose_bindings WHERE task_id=?")
        .bind(task_id).fetch_optional(pool).await.map_err(internal)?;
    let Some((
        source_id,
        canonical_repo,
        plan_revision,
        phase_id,
        project_objective,
        phase_objective,
        inherited_from,
    )) = binding
    else {
        return Ok(None);
    };
    if let Some(existing) = purpose_get(pool, &source_id, task_id, execution_round_id)
        .await
        .map_err(internal)?
    {
        return Ok(Some(PreparedPurpose {
            snapshot: existing,
            guard: None,
            consume_refresh: false,
        }));
    }
    let refresh: Option<(i64, Option<String>)> = sqlx::query_as("SELECT expected_plan_revision, phase_id FROM purpose_refresh_requests WHERE source_id=? AND task_id=? AND canonical_repo=?")
        .bind(&source_id).bind(task_id).bind(&canonical_repo).fetch_optional(pool).await.map_err(internal)?;
    if refresh.is_none() {
        if let Some(previous) = latest_snapshot(pool, &source_id, task_id)
            .await
            .map_err(internal)?
        {
            return Ok(Some(PreparedPurpose {
                snapshot: PurposeSnapshot {
                    execution_round_id: execution_round_id.into(),
                    captured_at: now,
                    ..previous
                },
                guard: None,
                consume_refresh: false,
            }));
        }
        if let Some(parent_task_id) = inherited_from {
            let inherited: Option<PurposeSnapshot> = sqlx::query_as("SELECT source_id, task_id, execution_round_id, plan_revision, phase_id, project_objective, phase_objective, captured_at FROM purpose_snapshots WHERE task_id=? AND source_id=? ORDER BY rowid DESC LIMIT 1")
                .bind(parent_task_id).bind(&source_id).fetch_optional(pool).await.map_err(internal)?;
            if let Some(previous) = inherited {
                return Ok(Some(PreparedPurpose {
                    snapshot: PurposeSnapshot {
                        task_id,
                        execution_round_id: execution_round_id.into(),
                        captured_at: now,
                        ..previous
                    },
                    guard: None,
                    consume_refresh: false,
                }));
            }
        }
        return Ok(Some(PreparedPurpose {
            snapshot: PurposeSnapshot {
                source_id,
                task_id,
                execution_round_id: execution_round_id.into(),
                plan_revision: Some(plan_revision),
                phase_id,
                project_objective,
                phase_objective,
                captured_at: now,
            },
            guard: None,
            consume_refresh: false,
        }));
    }
    let (revision, selected_phase, consume_refresh) = refresh
        .map(|(revision, phase)| (revision, phase, true))
        .unwrap_or((plan_revision, phase_id, false));
    prepare_plan_candidate(
        pool,
        task_id,
        execution_round_id,
        PurposeSelection {
            source_id,
            canonical_repo,
            plan_revision: revision,
            phase_id: selected_phase,
        },
        consume_refresh,
        now,
    )
    .await
    .map(Some)
}

pub fn render_purpose(candidate: &PreparedPurpose, text: &str) -> String {
    let context = prompt_context(&candidate.snapshot);
    if candidate.snapshot.project_objective.is_empty()
        && candidate.snapshot.phase_objective.is_empty()
    {
        text.into()
    } else {
        format!("{text}\n\n{context}")
    }
}

/// Persist the prepared purpose in the caller's admission transaction.  A plan
/// candidate rechecks the exact source/revision here, closing the read-to-admit
/// race; inherited snapshots remain valid historical evidence after edits.
pub async fn persist_purpose_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    candidate: &PreparedPurpose,
) -> Result<PurposeSnapshot, PlanError> {
    let live_source: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='notification_source_id'")
            .fetch_optional(&mut **tx)
            .await
            .map_err(internal)?;
    if live_source.as_deref() != Some(candidate.snapshot.source_id.as_str()) {
        return Err(PlanError::Invalid("purpose source changed".into()));
    }
    let existing: Option<PurposeSnapshot> = sqlx::query_as("SELECT source_id, task_id, execution_round_id, plan_revision, phase_id, project_objective, phase_objective, captured_at FROM purpose_snapshots WHERE source_id=? AND task_id=? AND execution_round_id=?")
        .bind(&candidate.snapshot.source_id).bind(candidate.snapshot.task_id).bind(&candidate.snapshot.execution_round_id).fetch_optional(&mut **tx).await.map_err(internal)?;
    if let Some(snapshot) = existing {
        return Ok(snapshot);
    }
    if let Some(guard) = &candidate.guard {
        let raw: Option<String> = sqlx::query_scalar("SELECT plan_json FROM project_plans WHERE source_id=? AND canonical_repo=? AND revision=?")
            .bind(&guard.source_id).bind(&guard.canonical_repo).bind(guard.plan_revision).fetch_optional(&mut **tx).await.map_err(internal)?;
        let Some(raw) = raw else {
            return Err(PlanError::PurposeConflict {
                current: latest_plan_tx(tx, guard).await?,
            });
        };
        let plan = decode_plan(&raw, &guard.canonical_repo, &guard.source_id).map_err(internal)?;
        if guard
            .phase_id
            .as_deref()
            .is_some_and(|id| !plan.phases.iter().any(|phase| phase.id == id))
        {
            return Err(PlanError::PurposeConflict { current: plan });
        }
    }
    insert_snapshot_tx(tx, &candidate.snapshot)
        .await
        .map_err(internal)?;
    if candidate.consume_refresh {
        let guard = candidate
            .guard
            .as_ref()
            .expect("refresh candidates have a plan guard");
        sqlx::query("DELETE FROM purpose_refresh_requests WHERE source_id=? AND task_id=? AND canonical_repo=? AND expected_plan_revision=?")
            .bind(&candidate.snapshot.source_id).bind(candidate.snapshot.task_id).bind(&guard.canonical_repo).bind(guard.plan_revision).execute(&mut **tx).await.map_err(internal)?;
    }
    Ok(candidate.snapshot.clone())
}

/// Capture once for an actual execution admission.  Existing snapshots win, which
/// makes retries idempotent.  If `inherit_from` is set and no refresh request is
/// pending, text/revision are copied from that immutable earlier snapshot.
pub async fn capture_for_execution_round(
    pool: &SqlitePool,
    source_id: &str,
    task_id: i64,
    execution_round_id: &str,
    canonical_repo: &str,
    selected_phase_id: Option<&str>,
    expected_plan_revision: Option<i64>,
    inherit_from: Option<&str>,
    now: i64,
) -> Result<Option<PurposeSnapshot>, PlanError> {
    validate_round_id(execution_round_id).map_err(PlanError::Invalid)?;
    validate_task_live(pool, canonical_repo, task_id).await?;
    if let Some(snapshot) = purpose_get(pool, source_id, task_id, execution_round_id)
        .await
        .map_err(internal)?
    {
        return Ok(Some(snapshot));
    }
    let refresh: Option<(i64, Option<String>)> = sqlx::query_as("SELECT expected_plan_revision, phase_id FROM purpose_refresh_requests WHERE source_id=? AND task_id=? AND canonical_repo=?")
        .bind(source_id).bind(task_id).bind(canonical_repo).fetch_optional(pool).await.map_err(internal)?;
    if refresh.is_none() {
        if let Some(previous_round) = inherit_from {
            if let Some(previous) = purpose_get(pool, source_id, task_id, previous_round)
                .await
                .map_err(internal)?
            {
                let copied = PurposeSnapshot {
                    execution_round_id: execution_round_id.into(),
                    captured_at: now,
                    ..previous
                };
                insert_snapshot(pool, &copied).await.map_err(internal)?;
                return Ok(Some(copied));
            }
        }
    }
    let Some(plan) = get(pool, source_id, canonical_repo)
        .await
        .map_err(internal)?
    else {
        return Ok(None);
    };
    let consume_refresh = refresh.is_some();
    let (requested_revision, requested_phase) = refresh.unwrap_or((
        expected_plan_revision.unwrap_or(plan.revision),
        selected_phase_id.map(str::to_owned),
    ));
    if requested_revision != plan.revision {
        return Err(PlanError::PurposeConflict { current: plan });
    }
    let phase = requested_phase
        .as_deref()
        .map(|id| {
            plan.phases
                .iter()
                .find(|phase| phase.id == id)
                .ok_or_else(|| PlanError::Invalid("phase_id does not belong to this plan".into()))
        })
        .transpose()?;
    let snapshot = PurposeSnapshot {
        source_id: source_id.into(),
        task_id,
        execution_round_id: execution_round_id.into(),
        plan_revision: Some(plan.revision),
        phase_id: requested_phase,
        project_objective: plan.objective,
        phase_objective: phase
            .map(|value| value.objective.clone())
            .unwrap_or_default(),
        captured_at: now,
    };
    insert_snapshot(pool, &snapshot).await.map_err(internal)?;
    if consume_refresh {
        sqlx::query("DELETE FROM purpose_refresh_requests WHERE source_id=? AND task_id=?")
            .bind(source_id)
            .bind(task_id)
            .execute(pool)
            .await
            .map_err(internal)?;
    }
    Ok(Some(snapshot))
}

pub async fn purpose_get(
    pool: &SqlitePool,
    source_id: &str,
    task_id: i64,
    execution_round_id: &str,
) -> anyhow::Result<Option<PurposeSnapshot>> {
    sqlx::query_as::<_, PurposeSnapshot>("SELECT source_id, task_id, execution_round_id, plan_revision, phase_id, project_objective, phase_objective, captured_at FROM purpose_snapshots WHERE source_id=? AND task_id=? AND execution_round_id=?")
        .bind(source_id).bind(task_id).bind(execution_round_id).fetch_optional(pool).await.map_err(Into::into)
}

/// Latest immutable evidence for task-detail UI.  A missing value means no
/// execution round has used a project purpose; callers must not infer one.
pub async fn purpose_latest(
    pool: &SqlitePool,
    source_id: &str,
    task_id: i64,
) -> anyhow::Result<Option<PurposeSnapshot>> {
    latest_snapshot(pool, source_id, task_id).await
}

/// The prompt fragment is descriptive only.  The execution caller must append it
/// after GoalContract and preserve GoalContract's constraints as authoritative.
pub fn prompt_context(snapshot: &PurposeSnapshot) -> String {
    let mut text =
        String::from("## Project purpose (context only; Goal Contract remains authoritative)\n");
    if !snapshot.project_objective.is_empty() {
        text.push_str(&snapshot.project_objective);
        text.push('\n');
    }
    if !snapshot.phase_objective.is_empty() {
        text.push_str("\n## Phase purpose\n");
        text.push_str(&snapshot.phase_objective);
        text.push('\n');
    }
    text
}

async fn insert_snapshot(pool: &SqlitePool, snapshot: &PurposeSnapshot) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO purpose_snapshots(source_id, task_id, execution_round_id, plan_revision, phase_id, project_objective, phase_objective, captured_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(&snapshot.source_id).bind(snapshot.task_id).bind(&snapshot.execution_round_id).bind(snapshot.plan_revision).bind(&snapshot.phase_id).bind(&snapshot.project_objective).bind(&snapshot.phase_objective).bind(snapshot.captured_at).execute(pool).await?;
    Ok(())
}

async fn insert_snapshot_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    snapshot: &PurposeSnapshot,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO purpose_snapshots(source_id, task_id, execution_round_id, plan_revision, phase_id, project_objective, phase_objective, captured_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(&snapshot.source_id).bind(snapshot.task_id).bind(&snapshot.execution_round_id).bind(snapshot.plan_revision).bind(&snapshot.phase_id).bind(&snapshot.project_objective).bind(&snapshot.phase_objective).bind(snapshot.captured_at).execute(&mut **tx).await?;
    Ok(())
}

async fn latest_snapshot(
    pool: &SqlitePool,
    source_id: &str,
    task_id: i64,
) -> anyhow::Result<Option<PurposeSnapshot>> {
    sqlx::query_as("SELECT source_id, task_id, execution_round_id, plan_revision, phase_id, project_objective, phase_objective, captured_at FROM purpose_snapshots WHERE source_id=? AND task_id=? ORDER BY rowid DESC LIMIT 1")
        .bind(source_id).bind(task_id).fetch_optional(pool).await.map_err(Into::into)
}

async fn prepare_plan_candidate(
    pool: &SqlitePool,
    task_id: i64,
    execution_round_id: &str,
    selection: PurposeSelection,
    consume_refresh: bool,
    now: i64,
) -> Result<PreparedPurpose, PlanError> {
    let plan = get(pool, &selection.source_id, &selection.canonical_repo)
        .await
        .map_err(internal)?
        .ok_or_else(|| PlanError::Invalid("no project plan exists".into()))?;
    if plan.revision != selection.plan_revision {
        return Err(PlanError::PurposeConflict { current: plan });
    }
    let phase = selection
        .phase_id
        .as_deref()
        .map(|id| {
            plan.phases
                .iter()
                .find(|phase| phase.id == id)
                .ok_or_else(|| PlanError::Invalid("phase_id does not belong to this plan".into()))
        })
        .transpose()?;
    Ok(PreparedPurpose {
        snapshot: PurposeSnapshot {
            source_id: selection.source_id.clone(),
            task_id,
            execution_round_id: execution_round_id.into(),
            plan_revision: Some(plan.revision),
            phase_id: selection.phase_id.clone(),
            project_objective: plan.objective,
            phase_objective: phase
                .map(|phase| phase.objective.clone())
                .unwrap_or_default(),
            captured_at: now,
        },
        guard: Some(selection),
        consume_refresh,
    })
}

async fn latest_plan_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    selection: &PurposeSelection,
) -> Result<ProjectPlan, PlanError> {
    let raw: Option<String> = sqlx::query_scalar(
        "SELECT plan_json FROM project_plans WHERE source_id=? AND canonical_repo=?",
    )
    .bind(&selection.source_id)
    .bind(&selection.canonical_repo)
    .fetch_optional(&mut **tx)
    .await
    .map_err(internal)?;
    raw.ok_or_else(|| PlanError::Invalid("project plan no longer exists".into()))
        .and_then(|raw| {
            decode_plan(&raw, &selection.canonical_repo, &selection.source_id).map_err(internal)
        })
}

fn internal(error: impl std::fmt::Display) -> PlanError {
    PlanError::Invalid(format!("storage failure: {error}"))
}

fn validate_scope(source_id: &str, canonical_repo: &str) -> anyhow::Result<()> {
    if source_id.is_empty() || source_id.len() > 128 || source_id.chars().any(char::is_control) {
        anyhow::bail!("source_id is invalid");
    }
    if canonical_repo.is_empty()
        || canonical_repo.len() > 4096
        || canonical_repo.chars().any(char::is_control)
    {
        anyhow::bail!("canonical_repo is invalid");
    }
    Ok(())
}

fn validate_plan(plan: &ProjectPlan) -> Result<(), String> {
    if plan.schema_version != SCHEMA_VERSION {
        return Err(format!("schema_version must be {SCHEMA_VERSION}"));
    }
    if plan.revision <= 0 {
        return Err("revision must be positive".into());
    }
    objective_ok(&plan.objective, "project objective")?;
    if plan.phases.len() > MAX_PHASES
        || plan.task_bindings.len() > MAX_BINDINGS
        || plan.dependencies.len() > MAX_DEPENDENCIES
    {
        return Err("plan exceeds its configured limits".into());
    }
    let mut ids = HashSet::new();
    for phase in &plan.phases {
        if !valid_phase_id(&phase.id) || !ids.insert(&phase.id) {
            return Err("phase id is invalid or duplicated".into());
        }
        if phase.name.trim() != phase.name || phase.name.is_empty() || phase.name.len() > 80 {
            return Err("phase name is invalid".into());
        }
        objective_ok(&phase.objective, "phase objective")?;
    }
    let mut bindings = HashSet::new();
    for binding in &plan.task_bindings {
        if binding.task_id <= 0
            || !ids.contains(&binding.phase_id)
            || !bindings.insert(binding.task_id)
        {
            return Err("task binding is invalid or duplicated".into());
        }
    }
    let mut edges = HashSet::new();
    for edge in &plan.dependencies {
        if edge.from <= 0 || edge.to <= 0 || edge.from == edge.to || !edges.insert(edge) {
            return Err("dependency is invalid or duplicated".into());
        }
    }
    if has_cycle(&plan.dependencies) {
        return Err("dependencies contain a cycle".into());
    }
    let encoded = serde_json::to_vec(plan).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_PLAN_BYTES {
        return Err("serialized plan is too large".into());
    }
    Ok(())
}

fn objective_ok(value: &str, name: &str) -> Result<(), String> {
    if value.len() > MAX_OBJECTIVE_BYTES {
        Err(format!("{name} exceeds {MAX_OBJECTIVE_BYTES} UTF-8 bytes"))
    } else {
        Ok(())
    }
}
fn valid_phase_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        && value.as_bytes()[0].is_ascii_alphanumeric()
}
fn has_cycle(edges: &[Dependency]) -> bool {
    let mut next: HashMap<i64, Vec<i64>> = HashMap::new();
    for edge in edges {
        next.entry(edge.from).or_default().push(edge.to);
    }
    fn visit(
        id: i64,
        next: &HashMap<i64, Vec<i64>>,
        active: &mut HashSet<i64>,
        done: &mut HashSet<i64>,
    ) -> bool {
        if !active.insert(id) {
            return true;
        }
        if !done.contains(&id) {
            for to in next.get(&id).into_iter().flatten() {
                if visit(*to, next, active, done) {
                    return true;
                }
            }
            done.insert(id);
        }
        active.remove(&id);
        false
    }
    let mut active = HashSet::new();
    let mut done = HashSet::new();
    next.keys()
        .any(|id| visit(*id, &next, &mut active, &mut done))
}

fn decode_plan(raw: &str, repo: &str, source_id: &str) -> anyhow::Result<ProjectPlan> {
    let plan: ProjectPlan =
        serde_json::from_str(raw).context("stored project plan JSON is corrupt")?;
    if plan.repo != repo {
        anyhow::bail!("stored project plan repo does not match its scope");
    }
    if !plan.source_id.is_empty() && plan.source_id != source_id {
        anyhow::bail!("stored project plan source does not match its scope");
    }
    let mut plan = plan;
    plan.source_id = source_id.into();
    validate_plan(&plan).map_err(anyhow::Error::msg)?;
    Ok(plan)
}

#[derive(Deserialize)]
struct LegacyView {
    version: u8,
    phases: Vec<String>,
    #[serde(rename = "phaseByTask")]
    phase_by_task: HashMap<String, String>,
    dependencies: Vec<Dependency>,
}
fn legacy_plan(raw: &str, repo: &str) -> Result<ProjectPlan, String> {
    if raw.len() > MAX_PLAN_BYTES {
        return Err("legacy JSON is too large".into());
    }
    let legacy: LegacyView =
        serde_json::from_str(raw).map_err(|_| "legacy JSON cannot be parsed".to_string())?;
    if legacy.version != 1 {
        return Err("legacy version must be 1".into());
    }
    let mut phases = Vec::new();
    let mut ids = HashSet::new();
    for (index, name) in legacy.phases.into_iter().enumerate() {
        if name.trim() != name || name.is_empty() || name.len() > 80 || name == "미분류" {
            return Err("legacy phase name is invalid".into());
        }
        if !ids.insert(name.clone()) {
            return Err("legacy phase names are duplicated".into());
        }
        phases.push(Phase {
            id: format!("legacy-{}-{}", index + 1, &sha256(&name)[..12]),
            name,
            objective: String::new(),
        });
    }
    if phases.len() > MAX_PHASES
        || legacy.phase_by_task.len() > MAX_BINDINGS
        || legacy.dependencies.len() > MAX_DEPENDENCIES
    {
        return Err("legacy graph exceeds its configured limits".into());
    }
    let by_name: HashMap<_, _> = phases
        .iter()
        .map(|phase| (phase.name.as_str(), phase.id.as_str()))
        .collect();
    let mut task_bindings = Vec::new();
    for (id, name) in legacy.phase_by_task {
        let task_id = id.parse::<i64>().map_err(|_| "legacy task id is invalid")?;
        let phase_id = by_name
            .get(name.as_str())
            .ok_or("legacy task phase is missing")?;
        task_bindings.push(TaskBinding {
            task_id,
            phase_id: (*phase_id).into(),
        });
    }
    Ok(ProjectPlan {
        schema_version: SCHEMA_VERSION,
        revision: 1,
        source_id: String::new(),
        repo: repo.into(),
        objective: String::new(),
        phases,
        task_bindings,
        dependencies: legacy.dependencies,
    })
}
fn sha256(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

async fn validate_task_live(pool: &SqlitePool, repo: &str, task_id: i64) -> Result<(), PlanError> {
    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM tasks WHERE id=? AND repo=?")
        .bind(task_id)
        .bind(repo)
        .fetch_optional(pool)
        .await
        .map_err(internal)?;
    if exists.is_none() {
        Err(PlanError::TaskScope { task_id })
    } else {
        Ok(())
    }
}
async fn validate_new_task_scope(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    repo: &str,
    proposed: &ProjectPlan,
    previous: Option<&ProjectPlan>,
) -> Result<(), PlanError> {
    let old: HashSet<i64> = previous
        .into_iter()
        .flat_map(|plan| {
            plan.task_bindings
                .iter()
                .map(|binding| binding.task_id)
                .chain(
                    plan.dependencies
                        .iter()
                        .flat_map(|edge| [edge.from, edge.to]),
                )
        })
        .collect();
    let current: HashSet<i64> = proposed
        .task_bindings
        .iter()
        .map(|binding| binding.task_id)
        .chain(
            proposed
                .dependencies
                .iter()
                .flat_map(|edge| [edge.from, edge.to]),
        )
        .collect();
    for id in current.difference(&old) {
        let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM tasks WHERE id=? AND repo=?")
            .bind(*id)
            .bind(repo)
            .fetch_optional(&mut **tx)
            .await
            .map_err(internal)?;
        if exists.is_none() {
            return Err(PlanError::TaskScope { task_id: *id });
        }
    }
    Ok(())
}
fn validate_round_id(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        Err("execution_round_id is invalid".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
