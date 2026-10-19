use std::path::{Path, PathBuf};

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

pub fn validate_capture_image_paths(
    worktree_path: &Path,
    task_id: i64,
    requested: &[String],
) -> Result<Vec<String>, String> {
    if requested.is_empty() {
        return Ok(Vec::new());
    }
    let capture_roots = super::capture::checked_capture_task_dirs(worktree_path, task_id)
        .into_iter()
        .filter_map(|dir| std::fs::canonicalize(dir).ok())
        .collect::<Vec<_>>();
    if capture_roots.is_empty() {
        return Err("캡처 디렉터리를 찾을 수 없습니다".into());
    }
    requested
        .iter()
        .map(|raw_path| validate_image_path(&capture_roots, raw_path))
        .collect()
}

fn validate_image_path(capture_roots: &[PathBuf], raw_path: &str) -> Result<String, String> {
    let path = std::fs::canonicalize(PathBuf::from(raw_path))
        .map_err(|_| format!("첨부 이미지를 찾을 수 없습니다: {raw_path}"))?;
    if !capture_roots.iter().any(|root| path.starts_with(root)) || !path.is_file() {
        return Err("작업 캡처 디렉터리 밖의 이미지는 첨부할 수 없습니다".into());
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or("이미지 확장자를 확인할 수 없습니다")?;
    if !IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        return Err("지원하지 않는 이미지 형식입니다".into());
    }
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SEQUENCE: AtomicU32 = AtomicU32::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = crate::testtmp::dir().join(format!(
            "praxis-capture-attachment-{tag}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn accepts_images_only_from_the_task_capture_directory() {
        let worktree = temp_dir("inside");
        let capture_dir = super::super::captures_dir(&worktree, 7);
        std::fs::create_dir_all(&capture_dir).unwrap();
        let image = capture_dir.join("editor.png");
        std::fs::write(&image, b"png").unwrap();

        let validated =
            validate_capture_image_paths(&worktree, 7, &[image.to_string_lossy().into_owned()])
                .unwrap();
        assert_eq!(
            validated,
            vec![image.canonicalize().unwrap().to_string_lossy()]
        );
        std::fs::remove_dir_all(worktree).unwrap();
    }

    #[test]
    fn rejects_paths_outside_the_task_capture_directory() {
        let worktree = temp_dir("outside");
        let capture_dir = super::super::captures_dir(&worktree, 7);
        std::fs::create_dir_all(&capture_dir).unwrap();
        let outside = worktree.join("outside.png");
        std::fs::write(&outside, b"png").unwrap();

        let error =
            validate_capture_image_paths(&worktree, 7, &[outside.to_string_lossy().into_owned()])
                .unwrap_err();
        assert!(error.contains("밖"));
        std::fs::remove_dir_all(worktree).unwrap();
    }
}
