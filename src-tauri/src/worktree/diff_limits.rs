//! Bound review payloads before decoding or allocating per-line objects.
use std::io::Read;
use std::path::Path;

pub(super) const MAX_PATCH_BYTES: u64 = 1024 * 1024;
pub(super) const LARGE: &str = "Praxis diff omitted: file exceeds 1 MiB";

pub(super) fn omission(path: &Path) -> Option<&'static str> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    // Do not follow symlinks into unrelated files or block on special files.
    if !metadata.file_type().is_file() {
        return None;
    }
    if metadata.len() > MAX_PATCH_BYTES {
        return Some(LARGE);
    }
    let mut prefix = [0; 8000];
    let count = std::fs::File::open(path).ok()?.read(&mut prefix).ok()?;
    let prefix = &prefix[..count];
    if prefix.contains(&0) || prefix.starts_with(b"%PDF-") {
        Some("Binary files differ")
    } else {
        None
    }
}

pub(super) fn marker(path: &str, reason: &str) -> String {
    format!("diff --git a/{path} b/{path}\n{reason}\n")
}
