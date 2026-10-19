use std::collections::BTreeMap;
use std::path::Path;

use super::capture::{ensure_captures_dir, next_capture_id, now_millis, write_record};
use super::{BoundingRect, CaptureRecord, CaptureSource};

/// 클립보드 붙여넣기 이미지 → 캡처 레코드. Design Mode/에디터 캡처와 같은 디렉터리에
/// `<id>.<ext>` + `<id>.json`으로 저장해 칩 표시·전송 시 프롬프트 주입·`image_paths`
/// 검증(`validate_capture_image_paths`)·작업 종결 정리(`cleanup_captures`)를 그대로 탄다.
pub fn save_pasted_capture(
    worktree_path: &Path,
    task_id: i64,
    bytes: &[u8],
    mime: &str,
) -> Result<CaptureRecord, String> {
    let ext = crate::paste::validate_image(bytes, mime)?;
    let dir = ensure_captures_dir(worktree_path, task_id)?;
    let id = next_capture_id();
    let image_path = dir.join(format!("{id}.{ext}"));
    crate::project_paths::write_new(&image_path, bytes)?;
    let record = CaptureRecord {
        id,
        task_id,
        source: CaptureSource::Paste,
        outer_html: String::new(),
        computed_css: BTreeMap::new(),
        // 클립보드 이미지에는 화면 좌표가 없다 — 칩/프롬프트 어느 쪽도 rect를 쓰지 않는다.
        bounding_rect: BoundingRect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        },
        captured_at: now_millis(),
        image_path: Some(image_path.to_string_lossy().into_owned()),
        file_path: None,
        selection_text: None,
        selection_start_line: None,
        selection_end_line: None,
    };
    write_record(&dir, &record)?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SEQUENCE: AtomicU32 = AtomicU32::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = crate::testtmp::dir().join(format!(
            "praxis-pasted-capture-{tag}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn saves_image_and_record_into_captures_dir() {
        let worktree = temp_dir("save");
        let record = save_pasted_capture(&worktree, 7, b"png-bytes", "image/png").unwrap();
        assert_eq!(record.source, CaptureSource::Paste);
        let image_path = record.image_path.as_deref().unwrap();
        assert!(image_path.ends_with(".png"));
        assert_eq!(std::fs::read(image_path).unwrap(), b"png-bytes");
        // json 레코드가 함께 남아 list/remove 파이프라인이 일반 캡처처럼 다룬다.
        let listed = super::super::list_captures(&worktree, 7).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, record.id);
        std::fs::remove_dir_all(&worktree).unwrap();
    }

    #[test]
    fn pasted_image_passes_attachment_validation() {
        let worktree = temp_dir("validate");
        let record = save_pasted_capture(&worktree, 7, b"png", "image/png").unwrap();
        let validated = super::super::validate_capture_image_paths(
            &worktree,
            7,
            &[record.image_path.clone().unwrap()],
        )
        .unwrap();
        assert_eq!(validated.len(), 1);
        std::fs::remove_dir_all(&worktree).unwrap();
    }

    #[test]
    fn remove_capture_deletes_non_png_image_too() {
        let worktree = temp_dir("remove");
        let record = save_pasted_capture(&worktree, 7, b"jpg", "image/jpeg").unwrap();
        let image_path = PathBuf::from(record.image_path.as_deref().unwrap());
        assert!(image_path.is_file());
        super::super::remove_capture(&worktree, 7, &record.id).unwrap();
        assert!(!image_path.is_file());
        assert!(super::super::list_captures(&worktree, 7)
            .unwrap()
            .is_empty());
        std::fs::remove_dir_all(&worktree).unwrap();
    }

    #[test]
    fn rejects_unsupported_mime() {
        let worktree = temp_dir("mime");
        assert!(save_pasted_capture(&worktree, 7, b"x", "application/pdf").is_err());
        std::fs::remove_dir_all(&worktree).unwrap();
    }
}
