//! diff 기준점 backfill — worktree(git)와 db를 잇는 얇은 조합 지점.
//!
//! `worktree`는 db를 모르는 순수 git 계층이고 `db`는 git을 모른다. 둘 다 건드려야 하는
//! 이 작업만 여기 둔다.
//!
//! 하는 일은 하나다. 기준점 컬럼이 생기기 전에 만들어진 작업들에 **지금 시점의** merge-base를
//! 한 번 굳힌다. 이미 base가 이동한 작업의 과거 변경은 지금도 보이지 않으므로 되살릴 수는
//! 없고, 앞으로 더 잃지 않게 막는 것이 목적이다(설계 0052 D2).

use sqlx::SqlitePool;

use crate::db;

/// 기준점이 없는 진행 중 작업에 지금 시점 merge-base를 굳힌다. 굳힌 건수를 돌려준다.
///
/// 실패는 건너뛴다 — 워크트리가 이미 사라졌거나 base 브랜치가 없는 작업은 NULL로 남아
/// 레거시 경로로 돈다. 부팅을 막지 않는 것이 계약이다.
pub async fn pin_missing_baselines(pool: &SqlitePool) -> anyhow::Result<usize> {
    let mut pinned = 0;
    for task in db::tasks_missing_baseline(pool).await? {
        let path = std::path::PathBuf::from(&task.worktree_path);
        if !crate::worktree::is_git_repository(&path) {
            continue;
        }
        let Ok(revision) = crate::worktree::freshest_merge_base(&path, &task.base) else {
            continue;
        };
        if revision.is_empty() {
            continue;
        }
        db::set_base_revision(pool, task.id, &revision).await?;
        pinned += 1;
    }
    Ok(pinned)
}

#[cfg(test)]
mod tests;
