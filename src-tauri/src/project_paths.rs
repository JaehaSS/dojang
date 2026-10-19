//! Project-local storage: new artifacts use Dojang; legacy paths remain readable.

use std::path::{Component, Path};

pub const DIR: &str = ".dojang";
pub const LEGACY_DIR: &str = ".praxis";
pub const DIRS: [&str; 2] = [DIR, LEGACY_DIR];

/// Create app-owned storage without traversing a symlink in its managed suffix.
pub fn ensure_dir(root: &Path, parts: &[&str]) -> Result<std::path::PathBuf, String> {
    fn check(path: &Path) -> Result<(), String> {
        let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(format!(
                "관리 경로가 실제 디렉터리가 아닙니다: {}",
                path.display()
            ));
        }
        Ok(())
    }
    check(root)?;
    let mut path = root.to_path_buf();
    for part in parts {
        let mut components = Path::new(part).components();
        if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
            return Err("관리 경로의 구성 요소가 올바르지 않습니다".into());
        }
        path.push(part);
        match std::fs::create_dir(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
        check(&path)?;
    }
    Ok(path)
}

pub fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())
}

pub fn is_internal_path(relative: &str) -> bool {
    DIRS.contains(&relative.split('/').next().unwrap_or_default())
}

pub fn is_auxiliary_path(relative: &str) -> bool {
    let mut parts = relative.split('/');
    DIRS.contains(&parts.next().unwrap_or_default())
        && matches!(parts.next(), Some("worktrees" | "approval"))
}

/// Missing worktrees can be finalized only at a stored, repository-local location.
pub fn is_worktree_path(repo: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(repo) else {
        return false;
    };
    let mut parts = relative.components();
    let Some(Component::Normal(root)) = parts.next() else {
        return false;
    };
    DIRS.iter().any(|dir| root == *dir)
        && parts.next() == Some(Component::Normal("worktrees".as_ref()))
        && matches!(parts.next(), Some(Component::Normal(_)))
        && parts.all(|part| matches!(part, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn generated_storage_refuses_links_and_never_overwrites_existing_files() {
        use std::os::unix::fs::symlink;
        let root = crate::testtmp::dir().join("dojang-storage-links");
        let outside = crate::testtmp::dir().join("dojang-storage-outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let sentinel = outside.join("sentinel.txt");
        std::fs::write(&sentinel, "preserve").unwrap();
        symlink(&outside, root.join(DIR)).unwrap();
        assert!(ensure_dir(&root, &[DIR, "pasted"]).is_err());
        assert!(!outside.join("pasted").exists());
        std::fs::remove_file(root.join(DIR)).unwrap();
        let storage = ensure_dir(&root, &[DIR, "pasted"]).unwrap();
        let image = storage.join("image.png");
        symlink(&sentinel, &image).unwrap();
        assert!(write_new(&image, b"replacement").is_err());
        assert_eq!(std::fs::read_to_string(sentinel).unwrap(), "preserve");
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn recognizes_both_storage_roots_without_matching_similar_names() {
        for root in DIRS {
            assert!(is_internal_path(&format!("{root}/captures/7/image.png")));
            assert!(is_auxiliary_path(&format!("{root}/worktrees/task/file")));
            assert!(is_auxiliary_path(&format!("{root}/approval/journal")));
            assert!(!is_auxiliary_path(&format!("{root}/captures/7")));
            assert!(!is_internal_path(&format!("{root}-other/file")));
        }
    }

    #[test]
    fn stored_worktrees_must_stay_inside_the_repository() {
        let repo = Path::new("/repo");
        for root in DIRS {
            assert!(is_worktree_path(
                repo,
                &repo.join(root).join("worktrees/task")
            ));
            assert!(!is_worktree_path(repo, &repo.join(root).join("worktrees")));
            assert!(!is_worktree_path(
                repo,
                &repo.join(root).join("worktrees/../../../outside")
            ));
        }
        assert!(!is_worktree_path(
            repo,
            Path::new("/repo-other/.dojang/worktrees/task")
        ));
        assert!(!is_worktree_path(repo, Path::new("/repo/src/task")));
    }
}
