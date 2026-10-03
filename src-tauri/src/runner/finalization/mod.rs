//! Durable, idempotent Runner approve/discard finalization.

use sqlx::SqlitePool;

use crate::db;
use crate::runner::config::RunnerConfig;
use crate::runner::worktree_lock::WorktreeLocks;

mod completion;
mod schema;
mod store;

pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    schema::migrate(pool).await
}

pub async fn finalize_task(
    config: &RunnerConfig,
    pool: &SqlitePool,
    worktree_locks: &WorktreeLocks,
    task_id: i64,
    approved: bool,
    now: i64,
) -> Result<(), String> {
    super::review_process::assert_task_unfenced(pool, task_id).await?;
    let pending = db::get_task(pool, task_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
    let _guard = worktree_locks
        .acquire(std::path::Path::new(&pending.worktree_path))
        .await;
    let mut attempt = if approved {
        // Authorize before audit metadata invokes Git on any stored task path.
        worktree(config, &pending).map_err(|e| e.to_string())?;
        crate::task_results::refresh_evidence(pool, task_id).await?;
        crate::task_review::guard_apply(pool, task_id).await?;
        Some(crate::approval::Attempt::start(pool, &pending).await.map_err(|e| e.to_string())?)
    } else { None };
    let result = async {
        let decision = if approved { "approved" } else { "discarded" };
        let task = store::claim(pool, task_id, decision, now)
            .await.map_err(|error| error.to_string())?;
        if let Err(error) = resume_locked(config, pool, &task, now, attempt.as_mut()).await {
            let _ = store::failure(pool, task_id, &error.to_string(), now).await;
            if let Ok(row) = store::load(pool, task_id).await {
                if !matches!(row.state.as_str(), "merged" | "cleaned") {
                    let _ = db::restore_awaiting_review(pool, task_id, now).await;
                }
            }
            return Err(error.to_string());
        }
        Ok(())
    }.await;
    if let Some(attempt) = attempt.as_mut() { attempt.finish(pool, &result).await; }
    result
}

pub async fn reconcile(
    config: &RunnerConfig,
    pool: &SqlitePool,
    worktree_locks: &WorktreeLocks,
    now: i64,
) -> anyhow::Result<u64> {
    let ids = store::finalizing_ids(pool).await?;
    let mut recovered = 0;
    for task_id in &ids {
        if super::review_process::task_is_fenced(pool, *task_id).await? {
            continue;
        }
        let task = db::get_task(pool, *task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("finalizing task disappeared"))?;
        let _guard = worktree_locks
            .acquire(std::path::Path::new(&task.worktree_path))
            .await;
        if let Err(error) = resume_locked(config, pool, &task, now, None).await {
            store::failure(pool, *task_id, &error.to_string(), now).await?;
            return Err(error);
        }
        recovered += 1;
    }
    Ok(recovered)
}

async fn resume_locked(
    config: &RunnerConfig,
    pool: &SqlitePool,
    task: &db::Task,
    now: i64,
    attempt: Option<&mut crate::approval::Attempt>,
) -> anyhow::Result<()> {
    let row = store::load(pool, task.id).await?;
    let worktree = worktree(config, task)?;
    if row.decision == "discarded" {
        return resume_discard(pool, task.id, &worktree, &row.state, now).await;
    }
    resume_approval(pool, task, &worktree, row, now, attempt).await
}

async fn resume_discard(
    pool: &SqlitePool,
    task_id: i64,
    worktree: &crate::worktree::Worktree,
    state: &str,
    now: i64,
) -> anyhow::Result<()> {
    if state == "prepared" {
        crate::memory::file::retire_task(pool, task_id).await?;
        store::stage(pool, task_id, "projection_retired", None, now).await?;
    }
    if matches!(state, "prepared" | "projection_retired") {
        // 직접 모드는 메인 체크아웃 그 자체 — worktree remove/branch -D를 하면 사용자 폴더를
        // 건드리게 된다. 변경사항은 그대로 두고(사용자가 직접 되돌림) 작업 레코드만 정리한다.
        //
        // 격리 워크트리는 폐기 시점 상태를 커밋해 브랜치에 남긴다 — 데스크톱 경로
        // (`commands::discard_task`)와 같은 계약이다. 갈라 두면 한쪽만 고쳐진다(설계 0056).
        let preserved = if worktree.is_direct() {
            None
        } else {
            worktree
                .preserve_and_retire()?
                .map(|evidence| evidence.commit)
        };
        store::stage(pool, task_id, "cleaned", preserved.as_deref(), now).await?;
    }
    if !matches!(state, "prepared" | "projection_retired" | "cleaned") {
        anyhow::bail!("unsupported discard finalization state: {state}");
    }
    completion::complete(pool, task_id, "discarded", now).await
}

async fn resume_approval(
    pool: &SqlitePool,
    task: &db::Task,
    worktree: &crate::worktree::Worktree,
    mut row: store::FinalizationRow,
    now: i64,
    mut attempt: Option<&mut crate::approval::Attempt>,
) -> anyhow::Result<()> {
    set_attempt_stage(&mut attempt, "projection", None);
    if row.state == "prepared" {
        // 파일형 투영에는 원장(journal)이 없다 — 블록만 걷어낸다. 옛 DB 투영(P2)의 retire는
        // 제거됐다(설계 2026-09-13 §P2 제거).
        crate::memory::file::retire_task(pool, task.id).await?;
        store::stage(pool, task.id, "projection_retired", None, now).await?;
        row.state = "projection_retired".to_string();
    }
    // 직접 모드는 커밋할 브랜치도, 머지할 대상도 없다 — 변경은 이미 사용자 체크아웃에 있다.
    // 커밋·머지·정리 단계를 건너뛰고 저널만 종단 상태로 옮긴다.
    if worktree.is_direct() {
        if row.state != "cleaned" {
            store::stage(pool, task.id, "cleaned", None, now).await?;
        }
        set_attempt_stage(&mut attempt, "completion", None);
        return completion::complete(pool, task.id, "approved", now).await;
    }
    if row.state == "projection_retired" {
        set_attempt_stage(&mut attempt, "policy", None);
        enforce_goal_contract(task, worktree)?;
        set_attempt_stage(&mut attempt, "commit", None);
        let commit = worktree.commit_for_approval()?;
        store::stage(pool, task.id, "committed", Some(&commit), now).await?;
        row.state = "committed".to_string();
        row.commit_sha = Some(commit);
    }
    let commit = row
        .commit_sha
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("finalization journal has no commit SHA"))?;
    if row.state == "committed" {
        set_attempt_stage(&mut attempt, "merge", Some(commit));
        if !worktree.commit_is_merged(commit) {
            worktree.validate_recorded_checkout(commit, false)?;
        }
        worktree.merge_for_approval(commit)?;
        store::stage(pool, task.id, "merged", None, now).await?;
        row.state = "merged".to_string();
    }
    if row.state == "merged" {
        set_attempt_stage(&mut attempt, "cleanup", Some(commit));
        if !worktree.commit_is_merged(commit) {
            anyhow::bail!("recorded approval commit is not merged");
        }
        if worktree.path.exists() {
            worktree.validate_recorded_checkout(commit, false)?;
        }
        worktree.cleanup_after_finalization()?;
        store::stage(pool, task.id, "cleaned", None, now).await?;
        row.state = "cleaned".to_string();
    }
    if row.state != "cleaned" {
        anyhow::bail!("unsupported finalization state: {}", row.state);
    }
    set_attempt_stage(&mut attempt, "completion", Some(commit));
    completion::complete(pool, task.id, "approved", now).await
}

fn set_attempt_stage(attempt: &mut Option<&mut crate::approval::Attempt>, stage: &str, commit: Option<&str>) {
    if let Some(attempt) = attempt.as_deref_mut() {
        attempt.stage = stage.into();
        if let Some(commit) = commit { attempt.source_sha = Some(commit.into()); }
    }
}

fn enforce_goal_contract(
    task: &db::Task,
    worktree: &crate::worktree::Worktree,
) -> anyhow::Result<()> {
    let changed = worktree.changed_paths()?;
    let violations = task
        .goal_contract
        .as_deref()
        .map(|contract| {
            crate::goal_contract::protected_path_violations(&contract.protected_paths, &changed)
        })
        .unwrap_or_default();
    if !violations.is_empty() {
        anyhow::bail!(
            "Goal Contract 보호 경로가 변경되어 승인할 수 없습니다: {}",
            violations.join(", ")
        );
    }
    Ok(())
}

fn worktree(config: &RunnerConfig, task: &db::Task) -> anyhow::Result<crate::worktree::Worktree> {
    let repo = super::auth::authorize_repository_path(
        &config.repository_roots,
        std::path::Path::new(&task.repo),
    )
    .map_err(anyhow::Error::msg)?;
    let stored = std::path::PathBuf::from(&task.worktree_path);
    let path = if stored.exists() {
        super::auth::authorize_repository_path(&config.repository_roots, &stored)
            .map_err(anyhow::Error::msg)?
    } else {
        let expected = repo.join(".praxis").join("worktrees");
        if !stored.starts_with(expected) {
            anyhow::bail!("stored worktree path escaped the authorized repository");
        }
        stored
    };
    Ok(crate::worktree::Worktree {
        repo,
        path,
        branch: task.branch.clone(),
        base: task.base.clone(),
        base_revision: task.base_revision.clone(),
    })
}
