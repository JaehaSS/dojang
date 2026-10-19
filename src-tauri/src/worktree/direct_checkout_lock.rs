//! Direct-mode checkout ownership across async requests and Dojang processes.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use fs2::FileExt;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

#[derive(Clone, Default)]
pub struct DirectCheckoutLocks {
    entries: Arc<Mutex<HashMap<PathBuf, Arc<AsyncMutex<()>>>>>,
}

pub struct DirectCheckoutGuard {
    _process: OwnedMutexGuard<()>,
    _file: Option<DirectCheckoutFileLock>,
}

impl DirectCheckoutLocks {
    pub async fn acquire(&self, repo: &Path) -> anyhow::Result<DirectCheckoutGuard> {
        let key = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
        let lock = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(key.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        let process = lock.lock_owned().await;
        let file = tokio::task::spawn_blocking(move || DirectCheckoutFileLock::acquire(&key))
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))??;
        Ok(DirectCheckoutGuard {
            _process: process,
            _file: file,
        })
    }
}

struct DirectCheckoutFileLock {
    _files: Vec<File>,
}

impl DirectCheckoutFileLock {
    fn acquire(repo: &Path) -> anyhow::Result<Option<Self>> {
        let Some(paths) = direct_checkout_lock_paths(repo)? else {
            return Ok(None);
        };
        let mut files = Vec::new();
        for path in paths {
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;

                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .read(true)
                    .write(true)
                    .mode(0o600)
                    .open(path)?;
                file.lock_exclusive()?;
                files.push(file);
            }
            #[cfg(not(unix))]
            {
                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .read(true)
                    .write(true)
                    .open(path)?;
                file.lock_exclusive()?;
                files.push(file);
            }
        }
        Ok(Some(Self { _files: files }))
    }
}

pub(super) fn branch_lock_paths(common_dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut paths = vec![common_dir.join("dojang-direct-branch.lock")];
    let legacy = common_dir.join("praxis-direct-branch.lock");
    match std::fs::symlink_metadata(&legacy) {
        Ok(metadata) => {
            anyhow::ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "기존 브랜치 잠금 파일이 올바르지 않습니다"
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    // This stable compatibility lock lives in Git metadata, never in `.praxis`.
    // Create it as well so an older process starting later observes the same lock.
    paths.push(legacy);
    Ok(paths)
}

fn direct_checkout_lock_paths(repo: &Path) -> anyhow::Result<Option<Vec<PathBuf>>> {
    if !super::is_git_repository(repo) {
        return Ok(None);
    }
    let common_dir = super::run_git(repo, &["rev-parse", "--git-common-dir"])?;
    let common_dir = Path::new(common_dir.trim());
    let common_dir = if common_dir.is_absolute() {
        common_dir.to_path_buf()
    } else {
        repo.join(common_dir)
    };
    Ok(Some(branch_lock_paths(&common_dir)?))
}
