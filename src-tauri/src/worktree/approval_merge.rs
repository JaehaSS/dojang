use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use sha2::{Digest, Sha256};

use super::{current_branch, local_branch_exists, run_git, Worktree};

pub(super) fn merge(worktree: &Worktree, commit: &str) -> anyhow::Result<()> {
    let _lock = approval_lock(worktree)?;
    if worktree.is_direct() {
        return Ok(());
    }
    let base = base_ref(worktree)?;
    if commit_is_merged(worktree, commit) {
        return cleanup_unlocked(worktree);
    }
    match checkout(worktree, &base)? {
        Checkout::Repository => merge_into(worktree, &worktree.repo, commit),
        Checkout::Temporary(path) => match merge_into(worktree, &path, commit) {
            Ok(()) => cleanup_path(worktree, &path),
            Err(error) => {
                cleanup_path(worktree, &path)?;
                Err(error)
            }
        },
    }
}

pub(super) fn commit_is_merged(worktree: &Worktree, commit: &str) -> bool {
    if worktree.is_direct() {
        return std::process::Command::new("git")
            .current_dir(&worktree.repo)
            .args(["merge-base", "--is-ancestor", commit, "HEAD"])
            .status()
            .is_ok_and(|status| status.success());
    }
    let Ok(base) = base_ref(worktree) else {
        return false;
    };
    std::process::Command::new("git")
        .current_dir(&worktree.repo)
        .args(["merge-base", "--is-ancestor", commit, &base])
        .status()
        .is_ok_and(|status| status.success())
}

pub(super) fn cleanup(worktree: &Worktree) -> anyhow::Result<()> {
    if worktree.is_direct() {
        return Ok(());
    }
    let _lock = approval_lock(worktree)?;
    cleanup_unlocked(worktree)
}

fn cleanup_unlocked(worktree: &Worktree) -> anyhow::Result<()> {
    let path = temporary_path(worktree, false)?;
    let registered = registered(worktree, &path)?;
    match std::fs::symlink_metadata(&path) {
        Ok(_) if registered => cleanup_path(worktree, &path)?,
        Ok(_) => anyhow::bail!("승인용 임시 워크트리 잔여물을 확인해야 합니다: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && registered => anyhow::bail!(
            "승인용 임시 워크트리 등록은 남았지만 경로가 없습니다: {}. 등록 상태를 확인한 뒤 다시 승인하세요.",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

enum Checkout {
    Repository,
    Temporary(PathBuf),
}

pub(super) fn base_ref(worktree: &Worktree) -> anyhow::Result<String> {
    if worktree.is_direct() || !local_branch_exists(&worktree.repo, &worktree.base) {
        anyhow::bail!(
            "저장된 승인 대상 '{}' 브랜치가 없습니다. 현재 HEAD로 대체하지 않았습니다.",
            worktree.base
        );
    }
    Ok(format!("refs/heads/{}", worktree.base))
}

fn checkout(worktree: &Worktree, base: &str) -> anyhow::Result<Checkout> {
    let holders = holders(worktree, base)?;
    if holders
        .first()
        .is_some_and(|path| same_path(path, &worktree.repo))
    {
        ensure_ready(&worktree.repo)?;
        return Ok(Checkout::Repository);
    }
    if let Some(path) = holders.first() {
        if same_path(path, &temporary_path(worktree, false)?) {
            validate_temporary(worktree, path)?;
            ensure_ready(path)?;
            return Ok(Checkout::Temporary(path.clone()));
        }
        anyhow::bail!(
            "승인 대상 '{}' 브랜치는 다른 워크트리에서 사용 중입니다: {}. 작업을 정리한 뒤 다시 승인하세요.",
            worktree.base,
            path.display()
        );
    }
    let path = temporary_path(worktree, false)?;
    if registered(worktree, &path)? {
        validate_temporary(worktree, &path)?;
        ensure_ready(&path)?;
        return Ok(Checkout::Temporary(path));
    }
    match std::fs::symlink_metadata(&path) {
        Ok(_) => anyhow::bail!(
            "승인용 임시 워크트리 잔여물을 확인해야 합니다: {}. 내용을 확인한 뒤 정리하고 다시 승인하세요.",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let path = temporary_path(worktree, true)?;
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("approval worktree path is not UTF-8"))?;
    run_git(
        &worktree.repo,
        &["worktree", "add", "--", path_str, &worktree.base],
    )?;
    validate_temporary(worktree, &path)?;
    Ok(Checkout::Temporary(path))
}

fn merge_into(worktree: &Worktree, target: &Path, commit: &str) -> anyhow::Result<()> {
    ensure_ready(target)?;
    if let Err(error) = run_git(target, &["merge", "--no-edit", commit]) {
        let mut conflicts = Vec::new();
        if merge_open(target)? {
            if let Ok(paths) = run_git(target, &["diff", "--name-only", "--diff-filter=U", "-z"]) {
                conflicts = paths.split('\0').filter(|path| !path.is_empty()).map(str::to_owned).collect();
            }
            let _ = run_git(target, &["merge", "--abort"]);
        }
        if !conflicts.is_empty() {
            // Only an actually attempted merge with unmerged entries opens the resolver.
            // Keep the original Git error after the first-line compatibility prefix.
            anyhow::bail!("MERGE_CONFLICT: {}\n{}", conflicts.join(", "), error);
        }
        return Err(error);
    }
    if !commit_is_merged(worktree, commit) {
        anyhow::bail!("저장된 승인 대상에 머지 커밋이 포함되지 않았습니다");
    }
    Ok(())
}

fn cleanup_path(worktree: &Worktree, path: &Path) -> anyhow::Result<()> {
    validate_temporary(worktree, path)?;
    ensure_ready(path)?;
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("approval worktree path is not UTF-8"))?;
    run_git(&worktree.repo, &["worktree", "remove", path_str])?;
    Ok(())
}

pub(super) fn temporary_path(worktree: &Worktree, create: bool) -> anyhow::Result<PathBuf> {
    let repo = canonical(&worktree.repo)?;
    let name = format!("{:x}", Sha256::digest(worktree.branch.as_bytes()));
    let legacy = repo.join(crate::project_paths::LEGACY_DIR).join("approval").join(&name);
    let current = repo.join(crate::project_paths::DIR).join("approval").join(&name);
    let legacy_present = std::fs::symlink_metadata(&legacy).is_ok() || registered(worktree, &legacy)?;
    let current_present = std::fs::symlink_metadata(&current).is_ok() || registered(worktree, &current)?;
    if legacy_present && current_present {
        anyhow::bail!("승인용 임시 워크트리가 새 경로와 기존 경로에 모두 남아 있습니다");
    }
    // Resume a legacy residual, but never create a new legacy storage directory.
    let dir = if legacy_present {
        crate::project_paths::LEGACY_DIR
    } else {
        crate::project_paths::DIR
    };
    let storage = repo.join(dir);
    managed_dir(&repo, &storage, create && !legacy_present)?;
    let approval = storage.join("approval");
    managed_dir(&repo, &approval, create)?;
    Ok(approval.join(name))
}

pub(super) fn holders(worktree: &Worktree, base: &str) -> anyhow::Result<Vec<PathBuf>> {
    let raw = run_git(&worktree.repo, &["worktree", "list", "--porcelain", "-z"])?;
    let mut paths = Vec::new();
    let mut path = None;
    for field in raw.split('\0') {
        if let Some(value) = field.strip_prefix("worktree ") {
            path = Some(PathBuf::from(value));
        } else if field.strip_prefix("branch ") == Some(base) {
            if let Some(path) = path.take() {
                paths.push(path);
            }
        }
    }
    Ok(paths)
}

fn registered(worktree: &Worktree, wanted: &Path) -> anyhow::Result<bool> {
    let raw = run_git(&worktree.repo, &["worktree", "list", "--porcelain", "-z"])?;
    Ok(raw
        .split('\0')
        .filter_map(|field| field.strip_prefix("worktree "))
        .any(|value| same_path(Path::new(value), wanted)))
}

pub(super) fn validate_temporary(worktree: &Worktree, path: &Path) -> anyhow::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() || !registered(worktree, path)? {
        anyhow::bail!(
            "승인용 임시 워크트리의 정체성을 확인할 수 없습니다: {}",
            path.display()
        );
    }
    let gitfile = std::fs::symlink_metadata(path.join(".git"))?;
    if gitfile.file_type().is_symlink() || !gitfile.is_file() {
        anyhow::bail!(
            "승인용 임시 워크트리 Git 메타데이터가 올바르지 않습니다: {}",
            path.display()
        );
    }
    let root = canonical(path)?;
    if !root.starts_with(canonical(&worktree.repo)?) {
        anyhow::bail!(
            "승인용 임시 워크트리 루트가 저장소 밖을 가리킵니다: {}",
            path.display()
        );
    }
    let actual_root = run_git(path, &["rev-parse", "--show-toplevel"])?;
    if canonical(Path::new(actual_root.trim()))? != root {
        anyhow::bail!(
            "승인용 임시 워크트리 루트가 변경되었습니다: {}",
            path.display()
        );
    }
    let common = run_git(path, &["rev-parse", "--git-common-dir"])?;
    let repo_common = run_git(&worktree.repo, &["rev-parse", "--git-common-dir"])?;
    if canonical_git(path, common.trim())? != canonical_git(&worktree.repo, repo_common.trim())?
        || current_branch(path)? != worktree.base
    {
        anyhow::bail!(
            "승인용 임시 워크트리 정체성이 변경되었습니다: {}",
            path.display()
        );
    }
    Ok(())
}

fn ensure_ready(path: &Path) -> anyhow::Result<()> {
    let changes = super::readiness::changes(path)?;
    if !changes.is_empty() {
        let files = changes.iter().take(5).map(|entry| format!("{} {}", entry.status, entry.path)).collect::<Vec<_>>().join(", ");
        anyhow::bail!(
            "승인 대상에 변경 사항이 있습니다: {} ({}건: {}). 변경을 정리한 뒤 다시 승인하세요.",
            path.display(), changes.len(), files
        );
    }
    if super::readiness::operation_open(path)?
    {
        anyhow::bail!(
            "승인 대상에서 Git 작업이 진행 중입니다: {}. 작업을 마친 뒤 다시 승인하세요.",
            path.display()
        );
    }
    Ok(())
}

fn merge_open(path: &Path) -> anyhow::Result<bool> {
    Ok(git_dir(path)?.join("MERGE_HEAD").exists())
}

fn git_dir(path: &Path) -> anyhow::Result<PathBuf> {
    Ok(PathBuf::from(
        run_git(path, &["rev-parse", "--absolute-git-dir"])?.trim(),
    ))
}

fn canonical(path: &Path) -> anyhow::Result<PathBuf> {
    Ok(std::fs::canonicalize(path)?)
}

fn canonical_git(cwd: &Path, value: &str) -> anyhow::Result<PathBuf> {
    let path = Path::new(value);
    if path.is_absolute() {
        return canonical(path);
    }
    canonical(&cwd.join(path))
}

fn managed_dir(repo: &Path, path: &Path, create: bool) -> anyhow::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            anyhow::bail!(
                "승인용 임시 워크트리 경로가 안전하지 않습니다: {}",
                path.display()
            );
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
            std::fs::create_dir(path)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    if !canonical(path)?.starts_with(repo) {
        anyhow::bail!(
            "승인용 임시 워크트리 경로가 저장소 밖을 가리킵니다: {}",
            path.display()
        );
    }
    Ok(())
}

fn approval_lock(worktree: &Worktree) -> anyhow::Result<Vec<File>> {
    let common = run_git(&worktree.repo, &["rev-parse", "--git-common-dir"])?;
    let common = canonical_git(&worktree.repo, common.trim())?;
    let mut locks = Vec::new();
    for path in super::direct_checkout_lock::branch_lock_paths(&common)? {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        lock.try_lock_exclusive().map_err(|_| {
            anyhow::anyhow!("다른 승인 또는 브랜치 전환이 진행 중입니다. 완료된 뒤 다시 승인하세요.")
        })?;
        locks.push(lock);
    }
    Ok(locks)
}

pub(super) fn same_path(left: &Path, right: &Path) -> bool {
    left == right
        || canonical(left)
            .ok()
            .zip(canonical(right).ok())
            .is_some_and(|(left, right)| left == right)
}

#[cfg(test)]
mod tests;
