use std::fs::{self, File, OpenOptions};
use std::io::{Error, ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::TargetPreimage;

pub(super) const MAX_TARGET_BYTES: u64 = 4 * 1024 * 1024;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) fn target_path(worktree: &Path, relative: &str) -> std::io::Result<PathBuf> {
    let mut components = Path::new(relative).components();
    let valid =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if !valid {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "invalid projection target",
        ));
    }
    let root = fs::canonicalize(worktree)?;
    let path = worktree.join(relative);
    if fs::canonicalize(path.parent().unwrap_or(worktree))? != root {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            "target escapes worktree",
        ));
    }
    Ok(path)
}

#[cfg(unix)]
fn open_read(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(windows)]
fn open_read(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

fn unix_mode(metadata: &fs::Metadata) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(metadata.permissions().mode())
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

pub(super) fn new_file_mode() -> Option<u32> {
    #[cfg(unix)]
    {
        Some(0o600)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

pub(super) fn read_preimage(worktree: &Path, relative: &str) -> std::io::Result<TargetPreimage> {
    let path = target_path(worktree, relative)?;
    let mut file = match open_read(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(TargetPreimage {
                relative_path: relative.to_string(),
                content: None,
                readonly: false,
                unix_mode: None,
            });
        }
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_TARGET_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "invalid projection target",
        ));
    }
    let mut content = String::new();
    std::io::Read::by_ref(&mut file)
        .take(MAX_TARGET_BYTES + 1)
        .read_to_string(&mut content)?;
    if content.len() as u64 > MAX_TARGET_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "projection target is too large",
        ));
    }
    Ok(TargetPreimage {
        relative_path: relative.to_string(),
        content: Some(content),
        readonly: metadata.permissions().readonly(),
        unix_mode: unix_mode(&metadata),
    })
}

fn temp_path(target: &Path) -> PathBuf {
    let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("context");
    target.with_file_name(format!(
        ".{name}.dojang-{}-{sequence}.tmp",
        std::process::id()
    ))
}

fn create_temp(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn set_permissions(path: &Path, preimage: &TargetPreimage) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = preimage.unix_mode.unwrap_or(0o600);
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_readonly(preimage.readonly);
        fs::set_permissions(path, permissions)
    }
}

fn write_atomic(target: &Path, preimage: &TargetPreimage, content: &str) -> std::io::Result<()> {
    let temporary = temp_path(target);
    let result = (|| {
        let mut file = create_temp(&temporary)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        drop(file);
        set_permissions(&temporary, preimage)?;
        if fs::symlink_metadata(target).is_ok() {
            let backup = temporary.with_extension("bak");
            fs::rename(target, &backup)?;
            if let Err(error) = fs::rename(&temporary, target) {
                let _ = fs::rename(&backup, target);
                return Err(error);
            }
            if let Err(error) = fs::remove_file(&backup) {
                let _ = fs::remove_file(target);
                let _ = fs::rename(&backup, target);
                return Err(error);
            }
            return Ok(());
        }
        fs::rename(&temporary, target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(super) fn write_update(
    path: &Path,
    preimage: &TargetPreimage,
    update: &Option<String>,
) -> std::io::Result<()> {
    match update {
        Some(content) => write_atomic(path, preimage, content),
        None => fs::remove_file(path).or_else(|error| {
            (error.kind() == ErrorKind::NotFound)
                .then_some(())
                .ok_or(error)
        }),
    }
}
