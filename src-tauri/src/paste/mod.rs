//! 클립보드 이미지 붙여넣기 — 공용 검증(mime/크기)과 홈 컴포저 저장 경로.
//!
//! 작업 컴포저의 붙여넣기는 `designmode::save_pasted_capture`가 캡처 레코드로 저장해
//! 칩·프롬프트 주입·`image_paths` 검증·종결 정리 파이프라인을 그대로 탄다. 여기는
//! 작업이 아직 없는 홈 컴포저(작업 생성 전) 경로만 담당한다: `<repo>/.dojang/pasted/`에
//! 저장하고 절대경로를 돌려주면, 프론트가 그 경로를 지시문 텍스트에 삽입한다
//! (에이전트 CLI는 skip-permissions로 실행되므로 절대경로 파일을 직접 읽는다).

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// 바이트 상한(20MB) — 클립보드 실수로 초대형 데이터가 IPC·디스크에 쌓이지 않게.
pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

/// 허용 mime → 확장자. 목록 밖 mime은 거부한다(임의 바이너리 저장 방지).
pub fn extension_for(mime: &str) -> Option<&'static str> {
    match mime {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/bmp" => Some("bmp"),
        _ => None,
    }
}

/// mime·크기 공용 검증 — 통과 시 확장자를 돌려준다.
pub fn validate_image(bytes: &[u8], mime: &str) -> Result<&'static str, String> {
    let ext = extension_for(mime).ok_or("지원하지 않는 이미지 형식입니다")?;
    if bytes.is_empty() {
        return Err("빈 이미지 데이터입니다".into());
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("이미지가 너무 큽니다 (20MB 상한)".into());
    }
    Ok(ext)
}

static PASTE_SEQ: AtomicU32 = AtomicU32::new(0);

/// `paste-<millis>-<seq>.<ext>` — millis 동률(연속 붙여넣기)에서도 seq로 유일하다.
fn next_file_name(ext: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0);
    let sequence = PASTE_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("paste-{millis}-{sequence}.{ext}")
}

/// 작업 컴포저 붙여넣기 — 해당 task의 캡처 디렉터리에 저장해 종결 정리·경로 검증을 공유한다.
pub fn save_task_image(
    worktree: &Path,
    task_id: i64,
    bytes: &[u8],
    mime: &str,
) -> Result<String, String> {
    let ext = validate_image(bytes, mime)?;
    if !worktree.is_dir() {
        return Err("워크트리 경로가 없습니다".into());
    }
    let task_id = task_id.to_string();
    let dir = crate::project_paths::ensure_dir(
        worktree,
        &[crate::project_paths::DIR, "captures", &task_id],
    )?;
    let path = dir.join(next_file_name(ext));
    crate::project_paths::write_new(&path, bytes)?;
    Ok(path.to_string_lossy().into_owned())
}

/// 홈 컴포저 붙여넣기 — 선택된 레포의 `.dojang/pasted/`에 저장하고 절대경로 반환.
pub fn save_repo_image(repo: &Path, bytes: &[u8], mime: &str) -> Result<String, String> {
    let ext = validate_image(bytes, mime)?;
    if !repo.is_dir() {
        return Err("레포 경로가 없습니다".into());
    }
    let dir = crate::project_paths::ensure_dir(repo, &[crate::project_paths::DIR, "pasted"])?;
    let path = dir.join(next_file_name(ext));
    crate::project_paths::write_new(&path, bytes)?;
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = crate::testtmp::dir().join(format!(
            "praxis-paste-test-{tag}-{}-{}",
            std::process::id(),
            PASTE_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_repo_image_writes_into_pasted_dir() {
        let repo = temp_dir("repo");
        let path = save_repo_image(&repo, b"jpg-bytes", "image/jpeg").unwrap();
        assert!(path.contains("pasted"));
        assert!(path.ends_with(".jpg"));
        assert_eq!(std::fs::read(&path).unwrap(), b"jpg-bytes");
        std::fs::remove_dir_all(&repo).unwrap();
    }

    #[test]
    fn save_task_image_writes_into_task_capture_dir() {
        let worktree = temp_dir("task");
        let path = save_task_image(&worktree, 7, b"png-bytes", "image/png").unwrap();
        assert!(path.contains(".dojang/captures/7"));
        assert!(path.ends_with(".png"));
        assert_eq!(std::fs::read(&path).unwrap(), b"png-bytes");
        std::fs::remove_dir_all(&worktree).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn task_and_repo_pastes_refuse_a_symlinked_dojang_root() {
        use std::os::unix::fs::symlink;

        let repo = temp_dir("symlinked-root");
        let outside = temp_dir("outside");
        let sentinel = outside.join("sentinel.txt");
        std::fs::write(&sentinel, "keep").unwrap();
        symlink(&outside, repo.join(crate::project_paths::DIR)).unwrap();

        assert!(save_task_image(&repo, 7, b"png", "image/png").is_err());
        assert!(save_repo_image(&repo, b"png", "image/png").is_err());
        assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "keep");
        std::fs::remove_dir_all(&repo).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn rejects_unknown_mime_empty_bytes_and_missing_repo() {
        let repo = temp_dir("reject");
        assert!(save_repo_image(&repo, b"x", "application/pdf").is_err());
        assert!(save_repo_image(&repo, b"", "image/png").is_err());
        std::fs::remove_dir_all(&repo).unwrap();
        assert!(save_repo_image(&repo, b"x", "image/png").is_err());
    }

    #[test]
    fn consecutive_saves_get_distinct_paths() {
        let repo = temp_dir("seq");
        let a = save_repo_image(&repo, b"a", "image/png").unwrap();
        let b = save_repo_image(&repo, b"b", "image/png").unwrap();
        assert_ne!(a, b);
        std::fs::remove_dir_all(&repo).unwrap();
    }

    #[test]
    fn oversized_image_is_rejected() {
        assert!(validate_image(&vec![0u8; MAX_IMAGE_BYTES + 1], "image/png").is_err());
        assert_eq!(validate_image(b"ok", "image/png"), Ok("png"));
    }
}
