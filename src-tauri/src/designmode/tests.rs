use super::*;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEST_SEQUENCE: AtomicU32 = AtomicU32::new(0);

fn tmp() -> PathBuf {
    let seq = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let dir = crate::testtmp::dir().join(format!(
        "praxis-designmode-{}-{}-{}",
        std::process::id(),
        now,
        seq
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn sample() -> ElementCapture {
    let mut css = BTreeMap::new();
    css.insert("color".to_string(), "rgb(0, 0, 0)".to_string());
    css.insert("display".to_string(), "flex".to_string());
    css.insert("__proto__".to_string(), "polluted".to_string());
    ElementCapture {
        outer_html: "<button>Click</button>".to_string(),
        computed_css: css,
        bounding_rect: BoundingRect {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
        },
    }
}

#[test]
fn save_filters_css_to_whitelist() {
    let worktree = tmp();
    let record = save_capture(&worktree, 42, sample(), None).unwrap();
    assert_eq!(record.task_id, 42);
    assert!(record.computed_css.contains_key("color"));
    assert!(record.computed_css.contains_key("display"));
    assert!(!record.computed_css.contains_key("__proto__"));
    assert!(record.image_path.is_none());
    assert_eq!(record.source, CaptureSource::Preview);
    assert!(record.file_path.is_none());
    let _ = fs::remove_dir_all(&worktree);
}

#[test]
fn save_stays_within_worktree() {
    let worktree = tmp();
    save_capture(&worktree, 7, sample(), None).unwrap();
    let dir = captures_dir(&worktree, 7);
    assert!(dir.starts_with(&worktree));
    assert!(dir.is_dir());
    assert!(dir.starts_with(worktree.join(crate::project_paths::DIR)));
    let _ = fs::remove_dir_all(&worktree);
}

#[cfg(unix)]
#[test]
fn save_refuses_a_symlinked_dojang_root() {
    use std::os::unix::fs::symlink;

    let worktree = tmp();
    let outside = tmp();
    let sentinel = outside.join("sentinel.txt");
    fs::write(&sentinel, "keep").unwrap();
    symlink(&outside, worktree.join(crate::project_paths::DIR)).unwrap();

    assert!(save_capture(&worktree, 7, sample(), None).is_err());
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "keep");
    let _ = fs::remove_dir_all(&worktree);
    let _ = fs::remove_dir_all(&outside);
}

#[cfg(unix)]
#[test]
fn record_write_refuses_a_final_file_symlink() {
    use std::os::unix::fs::symlink;

    let worktree = tmp();
    let outside = tmp();
    let sentinel = outside.join("sentinel.json");
    fs::write(&sentinel, "keep").unwrap();
    let record = save_capture(&worktree, 7, sample(), None).unwrap();
    let record_path = captures_dir(&worktree, 7).join(format!("{}.json", record.id));
    fs::remove_file(&record_path).unwrap();
    symlink(&sentinel, &record_path).unwrap();

    assert!(capture::write_record(&captures_dir(&worktree, 7), &record).is_err());
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "keep");
    let _ = fs::remove_dir_all(&worktree);
    let _ = fs::remove_dir_all(&outside);
}

#[cfg(target_os = "macos")]
#[test]
fn screenshot_refuses_a_final_file_symlink_and_existing_image() {
    use std::os::unix::fs::symlink;
    let worktree = tmp();
    let outside = tmp();
    let sentinel = outside.join("sentinel.png");
    fs::write(&sentinel, "keep").unwrap();
    let image = worktree.join("capture.png");
    symlink(&sentinel, &image).unwrap();
    let rect = BoundingRect { x: 0.0, y: 0.0, width: 1.0, height: 1.0 };
    assert!(capture::take_screenshot(&rect, &image).is_none());
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "keep");
    fs::remove_file(&image).unwrap();
    fs::write(&image, "existing").unwrap();
    assert!(capture::take_screenshot(&rect, &image).is_none());
    assert_eq!(fs::read_to_string(&image).unwrap(), "existing");
    let _ = fs::remove_dir_all(worktree);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn list_returns_saved_captures_sorted() {
    let worktree = tmp();
    save_capture(&worktree, 1, sample(), None).unwrap();
    save_capture(&worktree, 1, sample(), None).unwrap();
    let list = list_captures(&worktree, 1).unwrap();
    assert_eq!(list.len(), 2);
    assert!(list[0].captured_at <= list[1].captured_at);
    let _ = fs::remove_dir_all(&worktree);
}

#[test]
fn list_reads_legacy_captures_alongside_new_captures() {
    let worktree = tmp();
    let record = save_capture(&worktree, 1, sample(), None).unwrap();
    let legacy = worktree
        .join(crate::project_paths::LEGACY_DIR)
        .join("captures/1");
    fs::create_dir_all(&legacy).unwrap();
    fs::copy(
        captures_dir(&worktree, 1).join(format!("{}.json", record.id)),
        legacy.join(format!("{}.json", record.id)),
    )
    .unwrap();

    assert_eq!(list_captures(&worktree, 1).unwrap().len(), 2);
    let _ = fs::remove_dir_all(&worktree);
}

#[test]
fn list_missing_dir_is_empty_not_error() {
    let worktree = tmp();
    assert!(list_captures(&worktree, 999).unwrap().is_empty());
    let _ = fs::remove_dir_all(&worktree);
}

#[test]
fn remove_capture_deletes_only_target() {
    let worktree = tmp();
    let first = save_capture(&worktree, 1, sample(), None).unwrap();
    let second = save_capture(&worktree, 1, sample(), None).unwrap();
    remove_capture(&worktree, 1, &first.id).unwrap();
    let list = list_captures(&worktree, 1).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, second.id);
    let _ = fs::remove_dir_all(&worktree);
}

#[test]
fn remove_capture_rejects_path_traversal_id() {
    let worktree = tmp();
    save_capture(&worktree, 1, sample(), None).unwrap();
    let error = remove_capture(&worktree, 1, "../../etc/passwd").unwrap_err();
    assert!(error.contains("유효하지 않은"));
    let _ = fs::remove_dir_all(&worktree);
}

#[test]
fn cleanup_removes_new_and_legacy_task_dirs() {
    let worktree = tmp();
    save_capture(&worktree, 5, sample(), None).unwrap();
    let legacy = worktree
        .join(crate::project_paths::LEGACY_DIR)
        .join("captures/5");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("legacy.json"), "{}").unwrap();
    cleanup_captures(&worktree, 5);
    assert!(!captures_dir(&worktree, 5).exists());
    assert!(!legacy.exists());
    let _ = fs::remove_dir_all(&worktree);
}

#[test]
fn cleanup_missing_dir_is_noop() {
    let worktree = tmp();
    cleanup_captures(&worktree, 123);
    let _ = fs::remove_dir_all(&worktree);
}

#[cfg(unix)]
#[test]
fn cleanup_skips_legacy_root_symlink_without_touching_its_target() {
    use std::os::unix::fs::symlink;

    let worktree = tmp();
    let outside = tmp();
    let outside_task = outside.join("captures/7");
    fs::create_dir_all(&outside_task).unwrap();
    let sentinel = outside_task.join("123-0.json");
    fs::write(&sentinel, "keep").unwrap();
    symlink(&outside, worktree.join(crate::project_paths::LEGACY_DIR)).unwrap();

    remove_capture(&worktree, 7, "123-0").unwrap();
    cleanup_captures(&worktree, 7);
    assert!(sentinel.is_file());
    let _ = fs::remove_dir_all(&worktree);
    let _ = fs::remove_dir_all(&outside);
}

#[test]
fn outer_html_is_truncated_beyond_cap() {
    let worktree = tmp();
    let mut capture = sample();
    capture.outer_html = "x".repeat(capture::MAX_OUTER_HTML + 500);
    let record = save_capture(&worktree, 1, capture, None).unwrap();
    assert!(record.outer_html.len() < capture::MAX_OUTER_HTML + 500);
    assert!(record.outer_html.contains("truncated"));
    let _ = fs::remove_dir_all(&worktree);
}

#[test]
fn viewport_capture_rect_covers_whole_webview_in_screen_coords() {
    // 창 원점(100,200) + 웹뷰 원점(30,50) = 스크린 (130,250), 크기는 웹뷰 그대로.
    let rect = viewport_capture_rect((30.0, 50.0), (600.0, 400.0), (100.0, 200.0)).unwrap();
    assert_eq!(rect.x, 130.0);
    assert_eq!(rect.y, 250.0);
    assert_eq!(rect.width, 600.0);
    assert_eq!(rect.height, 400.0);
}

#[test]
fn viewport_capture_rect_skips_zero_sized_webview() {
    // 아직 레이아웃되지 않은 웹뷰 — 1px 미만이면 screencapture를 부르지 않는다.
    assert!(viewport_capture_rect((0.0, 0.0), (0.0, 400.0), (0.0, 0.0)).is_none());
}

#[test]
fn preview_mode_inline_owns_geometry_but_window_does_not() {
    // 인라인은 앱이 사이드패널 좌표를 강제하지만, 창은 사용자와 OS가 정한다.
    assert!(PreviewMode::Inline.owns_geometry());
    assert!(!PreviewMode::Window.owns_geometry());
}

#[test]
fn preview_mode_defaults_to_window() {
    // 기본이 창인 이유는 사이드패널 폭이 이 기능의 출발점이 된 불만이기 때문이다.
    assert_eq!(PreviewMode::default(), PreviewMode::Window);
}

#[test]
fn preview_mode_deserializes_from_lowercase() {
    let mode: PreviewMode = serde_json::from_str("\"inline\"").unwrap();
    assert_eq!(mode, PreviewMode::Inline);
}

#[test]
fn preview_mode_missing_field_falls_back_to_window() {
    // 프론트가 mode를 안 보내는 경로(구버전 호출)도 창으로 연다.
    #[derive(serde::Deserialize)]
    struct Payload {
        #[serde(default)]
        mode: PreviewMode,
    }
    let p: Payload = serde_json::from_str("{}").unwrap();
    assert_eq!(p.mode, PreviewMode::Window);
}
