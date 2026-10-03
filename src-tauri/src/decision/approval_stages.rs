use sqlx::SqlitePool;

use crate::{db, worktree::Worktree};

use super::approval_journal::{self, FailureCode, Journal, Stage};

pub(super) async fn resume(
    pool: &SqlitePool,
    task: &db::Task,
    worktree: &Worktree,
    now: i64,
) -> anyhow::Result<()> {
    let mut journal = approval_journal::load(pool, task.id).await?;
    advance_to_committed(pool, task, worktree, &mut journal, now).await?;
    advance_to_cleaned(pool, task.id, worktree, &mut journal, now).await?;
    complete(pool, task.id, &journal, now).await
}

async fn advance_to_committed(
    pool: &SqlitePool,
    task: &db::Task,
    worktree: &Worktree,
    journal: &mut Journal,
    now: i64,
) -> anyhow::Result<()> {
    if journal.stage()? == Stage::Prepared {
        retire_projection(pool, task.id, now).await?;
        journal.state = Stage::ProjectionRetired.as_str().into();
    }
    if journal.stage()? == Stage::ProjectionRetired {
        let commit = commit(pool, task, worktree, journal, now).await?;
        journal.state = Stage::Committed.as_str().into();
        journal.commit_sha = Some(commit);
    }
    Ok(())
}

async fn advance_to_cleaned(
    pool: &SqlitePool,
    task_id: i64,
    worktree: &Worktree,
    journal: &mut Journal,
    now: i64,
) -> anyhow::Result<()> {
    let commit = journal
        .commit_sha
        .clone()
        .ok_or_else(|| anyhow::anyhow!("local approval journal has no commit SHA"))?;
    if journal.stage()? == Stage::Committed {
        merge(
            pool,
            task_id,
            worktree,
            &commit,
            journal.exclude_generated_mcp,
            now,
        )
        .await?;
        journal.state = Stage::Merged.as_str().into();
    }
    if journal.stage()? == Stage::Merged {
        cleanup(
            pool,
            task_id,
            worktree,
            &commit,
            journal.exclude_generated_mcp,
            now,
        )
        .await?;
        journal.state = Stage::Cleaned.as_str().into();
    }
    Ok(())
}

async fn complete(
    pool: &SqlitePool,
    task_id: i64,
    journal: &Journal,
    now: i64,
) -> anyhow::Result<()> {
    if journal.stage()? != Stage::Cleaned {
        anyhow::bail!("unsupported local approval state: {}", journal.state);
    }
    if let Err(error) = super::approval_completion::complete(pool, task_id, now).await {
        record_failure(pool, task_id, FailureCode::LedgerCommitFailed, now).await;
        return Err(error);
    }
    Ok(())
}

async fn retire_projection(pool: &SqlitePool, task_id: i64, now: i64) -> anyhow::Result<()> {
    // 옛 DB 투영(P2)의 retire는 제거됐다(설계 2026-09-13 §P2 제거) — 파일형 투영 블록만 걷는다.
    // 승인 커밋에 메모리 사본을 남기지 않는다.
    if let Err(error) = crate::memory::file::retire_task(pool, task_id).await {
        record_failure(pool, task_id, FailureCode::ProjectionRetirementFailed, now).await;
        return Err(error);
    }
    if let Err(error) =
        approval_journal::stage(pool, task_id, Stage::ProjectionRetired, None, now).await
    {
        record_failure(pool, task_id, FailureCode::ProjectionRetirementFailed, now).await;
        return Err(error);
    }
    Ok(())
}

async fn commit(
    pool: &SqlitePool,
    task: &db::Task,
    worktree: &Worktree,
    journal: &Journal,
    now: i64,
) -> anyhow::Result<String> {
    if let Err(error) = super::approval_policy::enforce_goal_contract(task, worktree) {
        record_failure(pool, task.id, FailureCode::ProtectedPathChanged, now).await;
        return Err(error);
    }
    let result = if journal.exclude_generated_mcp {
        worktree.commit_for_approval_with_generated_mcp_excluded()
    } else {
        worktree.commit_for_approval()
    };
    let commit = match result {
        Ok(commit) => commit,
        Err(error) => {
            record_failure(pool, task.id, FailureCode::GitCommitFailed, now).await;
            return Err(error);
        }
    };
    if let Err(error) = worktree.validate_decision_commit(&commit, journal.exclude_generated_mcp) {
        record_failure(pool, task.id, FailureCode::GitCommitFailed, now).await;
        return Err(error);
    }
    if let Err(error) =
        approval_journal::stage(pool, task.id, Stage::Committed, Some(&commit), now).await
    {
        record_failure(pool, task.id, FailureCode::GitCommitFailed, now).await;
        return Err(error);
    }
    Ok(commit)
}

async fn merge(
    pool: &SqlitePool,
    task_id: i64,
    worktree: &Worktree,
    commit: &str,
    exclude_generated_mcp: bool,
    now: i64,
) -> anyhow::Result<()> {
    worktree.validate_commit_object(commit)?;
    if !worktree.commit_is_merged(commit) {
        worktree.validate_recorded_checkout(commit, exclude_generated_mcp)?;
    }
    if let Err(error) = worktree.merge_for_approval(commit) {
        record_failure(pool, task_id, FailureCode::GitMergeFailed, now).await;
        return Err(error);
    }
    if let Err(error) = approval_journal::stage(pool, task_id, Stage::Merged, None, now).await {
        record_failure(pool, task_id, FailureCode::GitMergeFailed, now).await;
        return Err(error);
    }
    Ok(())
}

async fn cleanup(
    pool: &SqlitePool,
    task_id: i64,
    worktree: &Worktree,
    commit: &str,
    exclude_generated_mcp: bool,
    now: i64,
) -> anyhow::Result<()> {
    if !worktree.commit_is_merged(commit) {
        record_failure(pool, task_id, FailureCode::GitMergeFailed, now).await;
        anyhow::bail!("recorded approval commit is not merged");
    }
    if worktree.path.exists() {
        worktree.validate_recorded_checkout(commit, exclude_generated_mcp)?;
    }
    if let Err(error) = worktree.cleanup_after_finalization() {
        record_failure(pool, task_id, FailureCode::GitCleanupFailed, now).await;
        return Err(error);
    }
    if let Err(error) = approval_journal::stage(pool, task_id, Stage::Cleaned, None, now).await {
        record_failure(pool, task_id, FailureCode::GitCleanupFailed, now).await;
        return Err(error);
    }
    Ok(())
}

async fn record_failure(pool: &SqlitePool, task_id: i64, code: FailureCode, now: i64) {
    let _ = approval_journal::failure(pool, task_id, code, now).await;
}
