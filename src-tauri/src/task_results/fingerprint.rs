use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::verify::ValidateSpec;

const FILE_LIMIT: u64 = 32 * 1024 * 1024;
const TOTAL_LIMIT: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct FingerprintObservation {
    fingerprint: Option<String>,
    reason: Option<&'static str>,
}
impl FingerprintObservation {
    pub fn fingerprint(&self) -> Option<String> {
        self.fingerprint.clone()
    }
    pub fn is_complete(&self) -> bool {
        self.fingerprint.is_some()
    }
    pub fn scope(&self) -> String {
        match self.reason {
            None => "git-v1:base,head,index,tracked-modes,deletions,untracked,validate".into(),
            Some(reason) => format!("git-v1:unknown:{reason}"),
        }
    }
}

/// Contents are read through the worktree root, including deleted tracked paths
/// and non-ignored untracked files.  Every uncertainty deliberately produces no
/// fingerprint; a partial hash must not be mistaken for a current observation.
pub fn fingerprint(root: &Path, spec: &ValidateSpec) -> FingerprintObservation {
    fingerprint_with_base(root, spec, None)
}

/// The task's immutable base revision is preferred when available.  A generic
/// caller gets HEAD as its base rather than silently omitting that component.
pub fn fingerprint_with_base(
    root: &Path,
    spec: &ValidateSpec,
    base_revision: Option<&str>,
) -> FingerprintObservation {
    let Ok(root) = root.canonicalize() else {
        return unknown("root");
    };
    let Ok(top) = git(&root, ["rev-parse", "--show-toplevel"]) else {
        return unknown("git");
    };
    if PathBuf::from(top.trim()) != root {
        return unknown("not-root");
    }
    let Ok(head) = git(&root, ["rev-parse", "HEAD"]) else {
        return unknown("head");
    };
    let base_ref = base_revision.unwrap_or("HEAD");
    let Ok(base) = git(&root, ["rev-parse", "--verify", base_ref]) else {
        return unknown("base");
    };
    let Ok(index) = git_bytes(&root, ["ls-files", "--stage", "-z"]) else {
        return unknown("index");
    };
    let Ok(tracked) = git_bytes(&root, ["ls-files", "-z"]) else {
        return unknown("tracked");
    };
    let Ok(untracked) = git_bytes(&root, ["ls-files", "--others", "--exclude-standard", "-z"])
    else {
        return unknown("untracked");
    };
    if index
        .split(|b| *b == 0)
        .any(|entry| entry.starts_with(b"160000 "))
    {
        return unknown("submodule");
    }
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    feed(&mut digest, b"task-result-source-v1\0");
    feed(&mut digest, head.trim().as_bytes());
    feed(&mut digest, b"\0base\0");
    feed(&mut digest, base.trim().as_bytes());
    feed(&mut digest, b"\0index\0");
    feed(&mut digest, &index);
    let Ok(mut paths) = parse_paths(&tracked) else {
        return unknown("path");
    };
    let Ok(untracked_paths) = parse_paths(&untracked) else {
        return unknown("path");
    };
    paths.extend(untracked_paths);
    paths.sort();
    paths.dedup();
    for path in paths {
        if hash_path(&root, &path, &mut digest, &mut total).is_err() {
            return unknown("content");
        }
    }
    let validate = root.join(".praxis/validate.toml");
    if validate.exists()
        && hash_path(
            &root,
            Path::new(".praxis/validate.toml"),
            &mut digest,
            &mut total,
        )
        .is_err()
    {
        return unknown("validate");
    }
    feed(
        &mut digest,
        format!(
            "\0commands\0{}\0{}\0{}",
            spec.build.as_deref().unwrap_or(""),
            spec.test.as_deref().unwrap_or(""),
            spec.timeout_secs
        )
        .as_bytes(),
    );
    FingerprintObservation {
        fingerprint: Some(format!("git-v1:sha256:{:x}", digest.finalize())),
        reason: None,
    }
}
fn unknown(reason: &'static str) -> FingerprintObservation {
    FingerprintObservation {
        fingerprint: None,
        reason: Some(reason),
    }
}
fn git<const N: usize>(root: &Path, args: [&str; N]) -> Result<String, ()> {
    String::from_utf8(git_bytes(root, args)?).map_err(|_| ())
}
fn git_bytes<const N: usize>(root: &Path, args: [&str; N]) -> Result<Vec<u8>, ()> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|_| ())?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(())
    }
}
fn parse_paths(raw: &[u8]) -> Result<Vec<PathBuf>, ()> {
    raw.split(|b| *b == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| std::str::from_utf8(entry).map_err(|_| ()))
        .collect::<Result<Vec<_>, _>>()
        .map(|paths| paths.into_iter().map(PathBuf::from).collect())
}
fn hash_path(root: &Path, path: &Path, digest: &mut Sha256, total: &mut u64) -> Result<(), ()> {
    if path.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(());
    }
    let full = root.join(path);
    feed(digest, path.to_string_lossy().as_bytes());
    feed(digest, b"\0");
    match fs::symlink_metadata(&full) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            feed(digest, b"deleted\0");
            Ok(())
        }
        Err(_) => Err(()),
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                let target = fs::read_link(&full).map_err(|_| ())?;
                feed(digest, b"symlink\0");
                feed(digest, target.as_os_str().as_encoded_bytes());
                return Ok(());
            }
            if !meta.is_file() {
                return Err(());
            }
            if meta.len() > FILE_LIMIT || total.saturating_add(meta.len()) > TOTAL_LIMIT {
                return Err(());
            }
            *total += meta.len();
            feed(digest, format!("mode:{:o}\0", file_mode(&meta)).as_bytes());
            let contents = fs::read(&full).map_err(|_| ())?;
            feed(digest, &contents);
            Ok(())
        }
    }
}
fn feed(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value)
}
#[cfg(unix)]
fn file_mode(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o7777
}
#[cfg(not(unix))]
fn file_mode(_meta: &fs::Metadata) -> u32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn fixture(name: &str) -> PathBuf {
        let root = crate::testtmp::dir().join(format!(
            "task-result-fingerprint-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q"]);
        git(&root, &["config", "user.email", "test@example.invalid"]);
        git(&root, &["config", "user.name", "Test"]);
        std::fs::write(root.join("tracked.txt"), "one\n").unwrap();
        git(&root, &["add", "tracked.txt"]);
        git(&root, &["commit", "-qm", "initial"]);
        root
    }

    #[test]
    fn includes_index_worktree_untracked_deleted_and_validate_spec() {
        let root = fixture("content");
        let spec = ValidateSpec::default();
        let original = fingerprint(&root, &spec).fingerprint().unwrap();
        std::fs::write(root.join("tracked.txt"), "staged\n").unwrap();
        git(&root, &["add", "tracked.txt"]);
        let staged = fingerprint(&root, &spec).fingerprint().unwrap();
        assert_ne!(original, staged);
        std::fs::write(root.join("tracked.txt"), "unstaged\n").unwrap();
        let unstaged = fingerprint(&root, &spec).fingerprint().unwrap();
        assert_ne!(staged, unstaged);
        std::fs::write(root.join("new.txt"), "untracked\n").unwrap();
        let untracked = fingerprint(&root, &spec).fingerprint().unwrap();
        assert_ne!(unstaged, untracked);
        std::fs::remove_file(root.join("tracked.txt")).unwrap();
        let deleted = fingerprint(&root, &spec).fingerprint().unwrap();
        assert_ne!(untracked, deleted);
        std::fs::create_dir_all(root.join(".praxis")).unwrap();
        std::fs::write(root.join(".praxis/validate.toml"), "test = \"true\"\n").unwrap();
        let with_validate = fingerprint(&root, &spec).fingerprint().unwrap();
        assert_ne!(deleted, with_validate);
    }

    #[test]
    fn large_file_is_unknown_instead_of_a_partial_hash() {
        let root = fixture("large");
        let file = std::fs::File::create(root.join("large.bin")).unwrap();
        file.set_len(FILE_LIMIT + 1).unwrap();
        assert!(!fingerprint(&root, &ValidateSpec::default()).is_complete());
    }
}
