use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::diffmodel::DiffHunk;
use crate::worktree::Worktree;

use super::patch::group_by_path;
use super::{compose_patch, reject_committed, reject_protected, ApplyOutcome, PartialError};

/// 역패치로 되돌릴 hunk — 고르지 않았고 **아직 커밋되지도 않은** 것.
///
/// 커밋된 hunk를 여기서 빼지 않으면 "고르지 않았다"는 이유만으로 되돌아간다. protected와
/// 정반대다 — 그쪽은 선택을 거부해 자동 폐기시키는 것이 의도된 동작이다.
pub(super) fn hunks_to_revert<'a>(
    all_hunks: &'a [DiffHunk],
    selected_ids: &[String],
) -> Vec<&'a DiffHunk> {
    all_hunks
        .iter()
        .filter(|hunk| !hunk.committed)
        .filter(|hunk| !selected_ids.iter().any(|id| id == &hunk.id))
        .collect()
}

pub fn apply(
    worktree: &Worktree,
    all_hunks: &[DiffHunk],
    selected_ids: &[String],
) -> Result<ApplyOutcome, PartialError> {
    let missing = selected_ids
        .iter()
        .filter(|id| !all_hunks.iter().any(|hunk| &hunk.id == *id))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(PartialError::HunkNotFound(missing));
    }
    let selected = all_hunks
        .iter()
        .filter(|hunk| selected_ids.iter().any(|id| id == &hunk.id))
        .collect::<Vec<_>>();
    reject_protected(&selected)?;
    reject_committed(&selected)?;
    let unselected = hunks_to_revert(all_hunks, selected_ids);
    let checkpoint = worktree
        .checkpoint_commit("dojang: pre-partial")
        .map_err(|error| PartialError::Git(error.to_string()))?;

    let mut failed = Vec::new();
    for (_, group) in group_by_path(&unselected) {
        let patch = compose_patch(&group);
        if apply_reverse_patch(&worktree.path, &patch).is_err() {
            failed.extend(group.iter().map(|hunk| hunk.id.clone()));
        }
    }
    if !failed.is_empty() {
        worktree
            .restore_to_checkpoint(&checkpoint)
            .map_err(|error| PartialError::Git(error.to_string()))?;
        return Err(PartialError::ApplyConflict(failed));
    }
    Ok(ApplyOutcome {
        checkpoint,
        kept_hunk_ids: selected.iter().map(|hunk| hunk.id.clone()).collect(),
        discarded_hunk_ids: unselected.iter().map(|hunk| hunk.id.clone()).collect(),
    })
}

pub fn rollback(worktree: &Worktree, checkpoint: &str) -> Result<(), PartialError> {
    worktree
        .restore_to_checkpoint(checkpoint)
        .map_err(|error| PartialError::Git(error.to_string()))
}

fn apply_reverse_patch(worktree_path: &Path, patch: &str) -> anyhow::Result<()> {
    run_git_apply(
        worktree_path,
        &["apply", "--reverse", "--whitespace=nowarn", "-"],
        patch,
    )
}

pub(crate) fn apply_forward_patch(worktree_path: &Path, patch: &str) -> anyhow::Result<()> {
    run_git_apply(
        worktree_path,
        &["apply", "--3way", "--whitespace=nowarn", "-"],
        patch,
    )
}

fn run_git_apply(worktree_path: &Path, args: &[&str], patch: &str) -> anyhow::Result<()> {
    let mut child = Command::new("git")
        .current_dir(worktree_path)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("git apply stdin 파이프를 열 수 없음"))?
        .write_all(patch.as_bytes())?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git apply 실패: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}
