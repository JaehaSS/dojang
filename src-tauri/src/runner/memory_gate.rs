//! Runner-surface (원격/모바일) 승인 대기 task의 승인·취소.
//!
//! 이름은 옛 P2 투영 재검증에서 왔지만, 그 재검증은 제거됐다(설계 2026-09-13 §P2 제거) —
//! 남은 것은 잠금·상태 전이·워크트리 정리뿐이다.

use sqlx::SqlitePool;

use crate::db;
use crate::runner::worktree_lock::WorktreeLocks;

pub async fn approve_pending_task(
    pool: &SqlitePool,
    worktree_locks: &WorktreeLocks,
    task_id: i64,
    now: i64,
) -> Result<db::Task, String> {
    super::review_process::assert_task_unfenced(pool, task_id).await?;
    let task = db::get_task(pool, task_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "task를 찾을 수 없습니다".to_string())?;
    if task.state != db::state::PENDING_APPROVAL {
        return Err("승인 대기 task만 실행할 수 있습니다".to_string());
    }
    let _guard = worktree_locks
        .acquire(std::path::Path::new(&task.worktree_path))
        .await;
    // 옛 DB 투영(P2) 재검증은 제거됐다(설계 2026-09-13 §P2 제거) — 항상 통과였던 게이트다.
    if !db::queue_pending_task(pool, task_id, now)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("승인 대기 task만 실행할 수 있습니다".to_string());
    }
    db::get_task(pool, task_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "task를 찾을 수 없습니다".to_string())
}

pub async fn cancel_pending_task(
    config: &crate::runner::config::RunnerConfig,
    pool: &SqlitePool,
    worktree_locks: &WorktreeLocks,
    task_id: i64,
    now: i64,
) -> Result<(), String> {
    super::review_process::assert_task_unfenced(pool, task_id).await?;
    let task = db::get_task(pool, task_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "task를 찾을 수 없습니다".to_string())?;
    let _guard = worktree_locks
        .acquire(std::path::Path::new(&task.worktree_path))
        .await;
    let current = db::get_task(pool, task_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "task를 찾을 수 없습니다".to_string())?;
    if current.state != db::state::PENDING_APPROVAL {
        return Err("승인 대기 task만 폐기할 수 있습니다".to_string());
    }
    let repo = super::auth::authorize_repository_path(
        &config.repository_roots,
        std::path::Path::new(&current.repo),
    )
    .map_err(|_| "허용되지 않는 repository 경로입니다".to_string())?;
    let path = super::auth::authorize_repository_path(
        &config.repository_roots,
        std::path::Path::new(&current.worktree_path),
    )
    .map_err(|_| "허용되지 않는 repository 경로입니다".to_string())?;
    crate::worktree::Worktree {
        repo,
        path,
        branch: current.branch,
        base: current.base,
        base_revision: current.base_revision,
    }
    .discard()
    .map_err(|error| error.to_string())?;
    if !db::reject_pending_task(pool, task_id, now)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("pending task changed state during locked cleanup".to_string());
    }
    Ok(())
}
