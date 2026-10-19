use std::collections::BTreeMap;
use std::path::Path;

use super::capture::{
    ensure_captures_dir, next_capture_id, now_millis, take_screenshot, write_record,
};
use super::{CaptureRecord, CaptureSource, EditorCapture};

const MAX_SELECTION_TEXT: usize = 20_000;

fn truncate_selection(text: Option<String>) -> Option<String> {
    let text = text.filter(|value| !value.is_empty())?;
    if text.chars().count() <= MAX_SELECTION_TEXT {
        return Some(text);
    }
    let truncated: String = text.chars().take(MAX_SELECTION_TEXT).collect();
    Some(format!(
        "{truncated}\n…truncated (원본 {}자)",
        text.chars().count()
    ))
}

fn build_editor_record(
    id: String,
    task_id: i64,
    capture: EditorCapture,
    image_path: String,
) -> CaptureRecord {
    CaptureRecord {
        id,
        task_id,
        source: CaptureSource::Editor,
        outer_html: String::new(),
        computed_css: BTreeMap::new(),
        bounding_rect: capture.bounding_rect,
        captured_at: now_millis(),
        image_path: Some(image_path),
        file_path: Some(capture.file_path),
        selection_text: truncate_selection(capture.selection_text),
        selection_start_line: capture.selection_start_line,
        selection_end_line: capture.selection_end_line,
    }
}

pub fn save_editor_capture(
    worktree_path: &Path,
    task_id: i64,
    capture: EditorCapture,
    screen_rect: super::BoundingRect,
) -> Result<CaptureRecord, String> {
    let dir = ensure_captures_dir(worktree_path, task_id)?;
    let id = next_capture_id();
    let image_path = take_screenshot(&screen_rect, &dir.join(format!("{id}.png")))
        .ok_or("에디터 화면 캡처에 실패했습니다. macOS 화면 기록 권한을 확인하세요.")?;
    let record = build_editor_record(id, task_id, capture, image_path);
    write_record(&dir, &record)?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::designmode::BoundingRect;

    fn capture(selection_text: Option<String>) -> EditorCapture {
        EditorCapture {
            file_path: "src/App.tsx".into(),
            selection_text,
            selection_start_line: Some(10),
            selection_end_line: Some(12),
            bounding_rect: BoundingRect {
                x: 1.0,
                y: 2.0,
                width: 300.0,
                height: 200.0,
            },
        }
    }

    #[test]
    fn editor_record_keeps_file_selection_and_image_metadata() {
        let record = build_editor_record(
            "1-0".into(),
            7,
            capture(Some("selected code".into())),
            "/tmp/editor.png".into(),
        );
        assert_eq!(record.source, CaptureSource::Editor);
        assert_eq!(record.file_path.as_deref(), Some("src/App.tsx"));
        assert_eq!(record.selection_text.as_deref(), Some("selected code"));
        assert_eq!(record.selection_start_line, Some(10));
        assert_eq!(record.image_path.as_deref(), Some("/tmp/editor.png"));
    }

    #[test]
    fn editor_record_drops_empty_selection_and_caps_large_selection() {
        let empty = build_editor_record("1-0".into(), 7, capture(Some(String::new())), "x".into());
        assert!(empty.selection_text.is_none());

        let large = build_editor_record(
            "1-1".into(),
            7,
            capture(Some("x".repeat(MAX_SELECTION_TEXT + 10))),
            "x".into(),
        );
        let selection = large.selection_text.expect("truncated selection");
        assert!(selection.contains("truncated"));
        assert!(selection.len() < MAX_SELECTION_TEXT + 100);
    }
}
