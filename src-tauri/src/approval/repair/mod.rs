//! Isolated, observed approval repair. A candidate never writes the original checkout.
mod execution;
pub mod git;

use crate::{db, review_ops::ReviewClaims, worktree::Worktree};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Check {
    pub command: String,
    pub exit_code: i32,
    pub tail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub task_id: i64,
    pub state: String,
    pub repo: String,
    pub base: String,
    pub source_path: String,
    pub source_branch: String,
    pub source_sha: String,
    pub source_fingerprint: String,
    pub target_sha: String,
    pub snapshot_sha: String,
    pub candidate_path: String,
    pub candidate_branch: String,
    pub candidate_sha: Option<String>,
    pub candidate_fingerprint: Option<String>,
    pub environment_files: Vec<String>,
    pub source_environment_fingerprint: String,
    pub commands: Vec<String>,
    pub attempts: u32,
    pub checks: Vec<Check>,
    pub summary: String,
    pub error: Option<String>,
    pub diff: String,
    pub updated_at: i64,
}

pub async fn latest(pool: &SqlitePool, task_id: i64) -> anyhow::Result<Option<Session>> {
    let raw: Option<String> = sqlx::query_scalar("SELECT detail FROM task_events WHERE task_id=? AND kind='approval_repair' ORDER BY id DESC LIMIT 1")
        .bind(task_id).fetch_optional(pool).await?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(Into::into))
        .transpose()
}

pub async fn observed_status(
    pool: &SqlitePool,
    task_id: i64,
    claims: &ReviewClaims,
) -> anyhow::Result<Option<Session>> {
    let mut session = latest(pool, task_id).await?;
    if let Some(value) = session.as_mut() {
        let task = db::get_task(pool, task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("작업을 찾을 수 없습니다"))?;
        anyhow::ensure!(
            value.task_id == task_id && value.repo == task.repo,
            "해결 세션의 작업 경계가 다릅니다"
        );
        if matches!(value.state.as_str(), "running" | "resolving" | "checking") {
            if let Ok(_claim) = claims.claim_finalization(task_id) {
                if !crate::runner::review_process::task_is_fenced(pool, task_id).await? {
                    if let Ok(_lock) = lock(&git::candidate(value), task_id) {
                        value.state = "needs_attention".into();
                        value.error = Some("자동 해결 실행이 중단됐습니다. 원본과 후보는 보존됐습니다. 새 세션을 준비하세요.".into());
                    }
                }
            }
        }
    }
    Ok(session)
}

async fn save(pool: &SqlitePool, session: &mut Session) -> anyhow::Result<()> {
    session.updated_at = chrono::Utc::now().timestamp();
    db::append_event(
        pool,
        session.task_id,
        "approval_repair",
        Some(&serde_json::to_string(session)?),
        session.updated_at,
    )
    .await
}

pub async fn cancel(pool: &SqlitePool, task_id: i64, session_id: &str) -> anyhow::Result<()> {
    let session = latest(pool, task_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("해결 세션이 없습니다"))?;
    anyhow::ensure!(
        session.id == session_id
            && matches!(session.state.as_str(), "running" | "resolving" | "checking"),
        "중단할 실행이 없습니다"
    );
    db::append_event(
        pool,
        task_id,
        "approval_repair_cancel",
        Some(session_id),
        chrono::Utc::now().timestamp(),
    )
    .await
}

pub(super) async fn cancelled(pool: &SqlitePool, session: &Session) -> anyhow::Result<bool> {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM task_events WHERE task_id=? AND kind='approval_repair_cancel' AND detail=?")
        .bind(session.task_id).bind(&session.id).fetch_one(pool).await?;
    Ok(count > 0)
}

fn lock(worktree: &Worktree, task_id: i64) -> anyhow::Result<File> {
    let root = git::managed_dir(&worktree.repo, &[".praxis", "approval", "repairs"])?;
    let path = root.join(format!("task-{task_id}.lock"));
    if let Ok(meta) = std::fs::symlink_metadata(&path) {
        anyhow::ensure!(!meta.file_type().is_symlink(), "repair lock symlink");
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.try_lock_exclusive()
        .map_err(|_| anyhow::anyhow!("이 작업의 자동 해결이 이미 진행 중입니다"))?;
    Ok(file)
}

async fn reviewable(pool: &SqlitePool, task: &db::Task) -> anyhow::Result<()> {
    let current = db::get_task(pool, task.id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("작업을 찾을 수 없습니다"))?;
    anyhow::ensure!(
        current.worktree_path == task.worktree_path
            && current.branch == task.branch
            && current.state == db::state::AWAITING_REVIEW
            && current.convo_pgid.is_none(),
        "작업이 변경되었거나 실행 중입니다"
    );
    anyhow::ensure!(
        task.state == db::state::AWAITING_REVIEW && task.mode == "conversation",
        "검토 대기 중인 대화 작업에서 실행하세요"
    );
    anyhow::ensure!(
        task.repo != task.worktree_path,
        "격리된 작업에서만 자동 해결할 수 있습니다"
    );
    crate::runner::review_process::assert_task_unfenced(pool, task.id)
        .await
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        !crate::decision::approval_journal::has_incomplete(pool, task.id).await?,
        "기존 승인 복구 저널을 먼저 처리하세요"
    );
    let has_runner: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='runner_finalizations'",
    )
    .fetch_one(pool)
    .await?;
    let pending: i64 = if has_runner > 0 {
        sqlx::query_scalar(
            "SELECT count(*) FROM runner_finalizations WHERE task_id=? AND state != 'completed'",
        )
        .bind(task.id)
        .fetch_one(pool)
        .await?
    } else {
        0
    };
    anyhow::ensure!(pending == 0, "기존 Runner 승인 복구를 먼저 처리하세요");
    Ok(())
}

pub async fn prepare(
    pool: &SqlitePool,
    task: &db::Task,
    worktree: Worktree,
    claims: &ReviewClaims,
) -> anyhow::Result<Session> {
    // A disconnected caller must not release the lock while snapshotting continues.
    let (pool, task, claims) = (pool.clone(), task.clone(), claims.clone());
    tokio::spawn(async move { prepare_owned(&pool, &task, worktree, &claims).await }).await?
}

async fn prepare_owned(
    pool: &SqlitePool,
    task: &db::Task,
    worktree: Worktree,
    claims: &ReviewClaims,
) -> anyhow::Result<Session> {
    let _claim = claims
        .claim_finalization(task.id)
        .map_err(anyhow::Error::msg)?;
    let _lock = lock(&worktree, task.id)?;
    reviewable(pool, task).await?;
    let exclude_mcp = db::has_task_event(pool, task.id, "mcp_generated").await?;
    let task = task.clone();
    let mut session =
        tokio::task::spawn_blocking(move || prepare_git(&task, &worktree, exclude_mcp)).await??;
    save(pool, &mut session).await?;
    Ok(session)
}

fn prepare_git(task: &db::Task, worktree: &Worktree, exclude_mcp: bool) -> anyhow::Result<Session> {
    worktree.validate_isolated_approval()?;
    anyhow::ensure!(
        !git::git(&worktree.path, &["ls-files", "--stage"])?
            .lines()
            .any(|line| line.starts_with("160000 ")),
        "submodule이 있는 작업은 자동 해결 전에 별도 검토가 필요합니다"
    );
    let readiness = worktree.approval_readiness()?;
    anyhow::ensure!(
        !readiness
            .issues
            .iter()
            .any(|issue| matches!(issue.code, "source_merge" | "source_operation")),
        "진행 중인 Git 작업을 먼저 마무리하세요"
    );
    let source_fingerprint = git::fingerprint(&worktree.path)?;
    let id = format!(
        "{}-{}",
        task.id,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let journal = git::managed_dir(&worktree.repo, &[".praxis", "approval", "repairs", &id])?;
    let snapshot = git::snapshot(worktree, &journal, exclude_mcp)?;
    let hooks = git::managed_dir(&journal, &["empty-hooks"])?;
    let root = git::managed_dir(&worktree.repo, &[".praxis", "worktrees"])?;
    let path = root.join(format!("repair-{id}"));
    let branch = format!("dojang/repair-{id}");
    git::git(
        &worktree.repo,
        &[
            "-c",
            &format!("core.hooksPath={}", hooks.display()),
            "worktree",
            "add",
            "-b",
            &branch,
            path.to_str()
                .ok_or_else(|| anyhow::anyhow!("non UTF-8 repair path"))?,
            &snapshot,
        ],
    )?;
    let spec = crate::verify::detect_spec(&worktree.path);
    let mut session = Session {
        id,
        task_id: task.id,
        state: "prepared".into(),
        repo: worktree.repo.to_string_lossy().into_owned(),
        base: worktree.base.clone(),
        source_path: worktree.path.to_string_lossy().into_owned(),
        source_branch: worktree.branch.clone(),
        source_sha: readiness.source_sha,
        source_fingerprint,
        target_sha: readiness.target_sha,
        snapshot_sha: snapshot,
        candidate_path: path.to_string_lossy().into_owned(),
        candidate_branch: branch,
        candidate_sha: None,
        candidate_fingerprint: None,
        environment_files: vec![],
        source_environment_fingerprint: String::new(),
        commands: [spec.build, spec.test].into_iter().flatten().collect(),
        attempts: 0,
        checks: vec![],
        summary: String::new(),
        error: None,
        diff: String::new(),
        updated_at: chrono::Utc::now().timestamp(),
    };
    // Once a candidate exists, persist its location even if preparation fails.
    let finish = (|| -> anyhow::Result<()> {
        let environment = crate::worktree::bootstrap::copy_environment(&worktree.path, &path);
        anyhow::ensure!(
            environment.failed.is_empty(),
            "후보 환경 복사 실패: {}",
            environment.failed.join(", ")
        );
        session.environment_files = environment.copied;
        session.source_environment_fingerprint =
            git::files_fingerprint(&worktree.path, &session.environment_files)?;
        git::assert_original(&session)
    })();
    if let Err(error) = finish {
        session.state = "needs_attention".into();
        session.error = Some(error.to_string());
    }
    Ok(session)
}

pub async fn run(
    pool: SqlitePool,
    task: db::Task,
    claims: ReviewClaims,
    session_id: String,
    model: Option<String>,
) -> Result<Session, String> {
    // Own the claim/lock until children finish even when an HTTP/UI caller disconnects.
    tokio::spawn(async move {
        run_owned(pool, task, claims, session_id, model)
            .await
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn run_owned(
    pool: SqlitePool,
    task: db::Task,
    claims: ReviewClaims,
    session_id: String,
    model: Option<String>,
) -> anyhow::Result<Session> {
    let _claim = claims
        .claim_finalization(task.id)
        .map_err(anyhow::Error::msg)?;
    let mut session = selected(&pool, &task, &session_id).await?;
    let _lock = lock(&git::candidate(&session), task.id)?;
    reviewable(&pool, &task).await?;
    anyhow::ensure!(
        session.state == "prepared",
        "새 해결 세션을 준비하세요. 이전 후보는 보존됩니다."
    );
    git::assert_original(&session)?;
    git::assert_candidate(&session)?;
    session.state = "running".into();
    save(&pool, &mut session).await?;
    let result = execution::run(&pool, &task, &mut session, model).await;
    match result {
        Ok(()) => session.state = "ready".into(),
        Err(error) => {
            session.state = "needs_attention".into();
            session.error = Some(error.to_string().chars().take(16000).collect());
        }
    }
    if let Ok(diff) = git::git(
        Path::new(&session.candidate_path),
        &[
            "diff",
            "--no-ext-diff",
            "--binary",
            &session.snapshot_sha,
            "HEAD",
        ],
    ) {
        session.diff = diff.chars().take(160000).collect();
        if diff.chars().count() > 160000 {
            session
                .diff
                .push_str("\n… 표시 한도 초과. 후보 폴더에서 전체 diff를 확인하세요.");
        }
    }
    save(&pool, &mut session).await?;
    Ok(session)
}

async fn selected(pool: &SqlitePool, task: &db::Task, id: &str) -> anyhow::Result<Session> {
    let session = latest(pool, task.id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("해결 세션을 먼저 준비하세요"))?;
    anyhow::ensure!(
        session.id == id
            && session.task_id == task.id
            && session.base == task.base
            && session.source_path == task.worktree_path
            && session.source_branch == task.branch
            && session.repo == task.repo,
        "작업 또는 해결 세션이 변경됐습니다"
    );
    Ok(session)
}

pub async fn accept(
    pool: &SqlitePool,
    task: &db::Task,
    claims: &ReviewClaims,
    session_id: &str,
) -> anyhow::Result<Session> {
    let (session, _claim) = accept_held(pool, task, claims, session_id).await?;
    Ok(session)
}

/// Desktop keeps finalization claimed until its in-memory worktree is replaced too.
pub(crate) async fn accept_held(
    pool: &SqlitePool,
    task: &db::Task,
    claims: &ReviewClaims,
    session_id: &str,
) -> anyhow::Result<(Session, crate::review_ops::ReviewClaim)> {
    let claim = claims
        .claim_finalization(task.id)
        .map_err(anyhow::Error::msg)?;
    let mut session = selected(pool, task, session_id).await?;
    let _lock = lock(&git::candidate(&session), task.id)?;
    reviewable(pool, task).await?;
    anyhow::ensure!(
        session.state == "ready"
            && !session.commands.is_empty()
            && session.commands.len() == session.checks.len()
            && session
                .commands
                .iter()
                .zip(&session.checks)
                .all(|(command, check)| command == &check.command && check.exit_code == 0),
        "검증을 통과한 후보만 채택할 수 있습니다"
    );
    git::assert_original(&session)?;
    git::assert_candidate(&session)?;
    git::verify_ancestry(&session)?;
    anyhow::ensure!(
        Some(git::revision(Path::new(&session.candidate_path), "HEAD")?) == session.candidate_sha
            && Some(git::candidate_fingerprint(&session)?) == session.candidate_fingerprint,
        "검증 후 후보가 변경됐습니다"
    );
    session.state = "accepted".into();
    session.updated_at = chrono::Utc::now().timestamp();
    let capsule = format!("{}\n자동 해결 후보를 채택했습니다. 원본 보관 경로: {}. 대상 {} / {}. 해결 요약:\n{}\n현재 worktree와 Git 상태를 다시 확인하고 작업을 이어가세요.", task.pending_capsule.as_deref().unwrap_or(""), session.source_path, session.base, session.target_sha, session.summary);
    let mut tx = pool.begin().await?;
    let row = sqlx::query("UPDATE tasks SET worktree_path=?, branch=?, base_revision=?, convo_session_id=NULL, pending_capsule=?, updated_at=? WHERE id=? AND state='AwaitingReview' AND worktree_path=? AND branch=? AND convo_pgid IS NULL AND NOT EXISTS (SELECT 1 FROM review_process_leases WHERE task_id=tasks.id)")
        .bind(&session.candidate_path).bind(&session.candidate_branch).bind(&session.target_sha).bind(capsule).bind(session.updated_at).bind(task.id).bind(&task.worktree_path).bind(&task.branch).execute(&mut *tx).await?;
    anyhow::ensure!(
        row.rows_affected() == 1,
        "작업이 변경되어 후보를 채택하지 않았습니다"
    );
    // Prior Verify evidence belongs to the original checkout, not this candidate.
    sqlx::query("UPDATE evidence SET ready=0 WHERE task_id=?")
        .bind(task.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO task_events(task_id,ts,kind,detail) VALUES(?,?,'approval_repair',?)")
        .bind(task.id)
        .bind(session.updated_at)
        .bind(serde_json::to_string(&session)?)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((session, claim))
}
