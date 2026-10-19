use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{BoundingRect, CaptureRecord, CaptureSource, ElementCapture, CSS_WHITELIST};

const CAPTURES_DIRNAME: &str = "captures";
pub(super) const MAX_OUTER_HTML: usize = 20_000;
static CAPTURE_SEQ: AtomicU32 = AtomicU32::new(0);

pub(super) fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

pub(super) fn next_capture_id() -> String {
    let sequence = CAPTURE_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{}-{sequence}", now_millis())
}

fn valid_capture_id(id: &str) -> bool {
    let Some((millis, sequence)) = id.split_once('-') else {
        return false;
    };
    !millis.is_empty()
        && !sequence.is_empty()
        && millis.chars().all(|character| character.is_ascii_digit())
        && sequence.chars().all(|character| character.is_ascii_digit())
}

pub fn captures_dir(worktree_path: &Path, task_id: i64) -> PathBuf {
    worktree_path
        .join(crate::project_paths::DIR)
        .join(CAPTURES_DIRNAME)
        .join(task_id.to_string())
}

pub(super) fn ensure_captures_dir(worktree_path: &Path, task_id: i64) -> Result<PathBuf, String> {
    let task_id = task_id.to_string();
    crate::project_paths::ensure_dir(
        worktree_path,
        &[crate::project_paths::DIR, CAPTURES_DIRNAME, &task_id],
    )
}

fn truncate_outer_html(html: &str) -> String {
    if html.chars().count() <= MAX_OUTER_HTML {
        return html.to_string();
    }
    let truncated: String = html.chars().take(MAX_OUTER_HTML).collect();
    format!(
        "{truncated}\n<!-- …truncated (원본 {}자) -->",
        html.chars().count()
    )
}

fn whitelist_css(css: BTreeMap<String, String>) -> BTreeMap<String, String> {
    css.into_iter()
        .filter(|(key, _)| CSS_WHITELIST.contains(&key.as_str()))
        .collect()
}

fn build_record(
    id: String,
    task_id: i64,
    capture: ElementCapture,
    image_path: Option<String>,
) -> CaptureRecord {
    CaptureRecord {
        id,
        task_id,
        source: CaptureSource::Preview,
        outer_html: truncate_outer_html(&capture.outer_html),
        computed_css: whitelist_css(capture.computed_css),
        bounding_rect: capture.bounding_rect,
        captured_at: now_millis(),
        image_path,
        file_path: None,
        selection_text: None,
        selection_start_line: None,
        selection_end_line: None,
    }
}

pub(super) fn write_record(dir: &Path, record: &CaptureRecord) -> Result<(), String> {
    let path = dir.join(format!("{}.json", record.id));
    let json = serde_json::to_string_pretty(record).map_err(|error| error.to_string())?;
    crate::project_paths::write_new(&path, json.as_bytes())
}

/// `image_path`는 호출부가 이미 결정한 값을 그대로 기록한다(스크린샷 촬영 자체는
/// `save_capture_with_screenshot` 참고 — id·png 파일명 순서 문제 때문에 이 함수를 거치지 않는다).
pub fn save_capture(
    worktree_path: &Path,
    task_id: i64,
    capture: ElementCapture,
    image_path: Option<String>,
) -> Result<CaptureRecord, String> {
    let dir = ensure_captures_dir(worktree_path, task_id)?;
    let record = build_record(next_capture_id(), task_id, capture, image_path);
    write_record(&dir, &record)?;
    Ok(record)
}

/// 요소 rect(웹뷰 viewport 기준) + 웹뷰/창 오프셋을 합성해 스크린 절대좌표(논리 px)를 만든다.
/// 요소가 웹뷰 밖으로 스크롤된 부분은 웹뷰 크기 안으로 클램프하고, 클램프 후 1px 미만이면
/// 스크린샷을 건너뛴다(`None`). x/y는 절대 0으로 클램프하지 않는다 — 보조 디스플레이의
/// 음수 좌표는 정상값이다.
pub fn screen_capture_rect(
    element: BoundingRect,
    webview_origin: (f64, f64),
    webview_size: (f64, f64),
    window_origin: (f64, f64),
) -> Option<BoundingRect> {
    let clamped_x = element.x.max(0.0).min(webview_size.0);
    let clamped_y = element.y.max(0.0).min(webview_size.1);
    let width = (element.x + element.width).min(webview_size.0) - clamped_x;
    let height = (element.y + element.height).min(webview_size.1) - clamped_y;
    if width < 1.0 || height < 1.0 {
        return None;
    }
    Some(BoundingRect {
        x: window_origin.0 + webview_origin.0 + clamped_x,
        y: window_origin.1 + webview_origin.1 + clamped_y,
        width,
        height,
    })
}

/// 웹뷰 뷰포트 전체의 스크린 절대좌표(논리 px).
///
/// 요소만 크롭하면(`screen_capture_rect`) 결과 이미지가 곧 선택 영역 자체여서, `inject.js`가
/// 그려둔 하이라이트 박스가 이미지 전체를 덮어 "화면 어디"인지를 전달하지 못한다. 주변까지
/// 함께 찍어야 하이라이트가 한 지점을 가리키는 그림이 된다. 요소 자체의 위치·크기는
/// `CaptureRecord.bounding_rect`에 그대로 남으므로 정보 손실은 없다.
pub fn viewport_capture_rect(
    webview_origin: (f64, f64),
    webview_size: (f64, f64),
    window_origin: (f64, f64),
) -> Option<BoundingRect> {
    screen_capture_rect(
        BoundingRect {
            x: 0.0,
            y: 0.0,
            width: webview_size.0,
            height: webview_size.1,
        },
        webview_origin,
        webview_size,
        window_origin,
    )
}

/// `screencapture -x -R x,y,w,h <path>` 인자 목록 — 소수 좌표도 그대로 포맷(screencapture가 허용).
fn screencapture_args(rect: &BoundingRect, path: &Path) -> Vec<String> {
    vec![
        "-x".to_string(),
        "-R".to_string(),
        format!("{},{},{},{}", rect.x, rect.y, rect.width, rect.height),
        path.to_string_lossy().into_owned(),
    ]
}

/// macOS `screencapture` shellout — Screen Recording TCC 미허용 등으로 실패하면 조용히 `None`을
/// 반환한다(사용자가 HTML/CSS 캡처는 계속 쓸 수 있어야 함). 성공 판정은 "exit 성공 + 파일 존재"만 본다.
#[cfg(target_os = "macos")]
pub(super) fn take_screenshot(rect: &BoundingRect, path: &Path) -> Option<String> {
    // Reserve a regular output file before the external command can overwrite it.
    // Existing files and final-file symlinks are rejected by create_new.
    let reserved = fs::OpenOptions::new().write(true).create_new(true).open(path).ok()?;
    drop(reserved);
    let status = std::process::Command::new("screencapture")
        .args(screencapture_args(rect, path))
        .status()
        .ok();
    if !status.is_some_and(|status| status.success())
        || !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
    {
        let _ = fs::remove_file(path);
        return None;
    }
    Some(path.to_string_lossy().into_owned())
}

#[cfg(not(target_os = "macos"))]
pub(super) fn take_screenshot(_rect: &BoundingRect, _path: &Path) -> Option<String> {
    None
}

/// 스크린샷 포함 캡처 저장 — id를 먼저 만들어 `<id>.png`로 촬영한 뒤 그 경로를 record에 기록한다
/// (save_capture는 image_path를 이미 결정된 값으로 받으므로, id·파일명 순서를 맞추려면 여기서
/// 별도로 처리해야 한다). `screen_rect`가 `None`이면(geometry 조회 실패 등) 촬영을 생략한다.
pub fn save_capture_with_screenshot(
    worktree_path: &Path,
    task_id: i64,
    capture: ElementCapture,
    screen_rect: Option<BoundingRect>,
) -> Result<CaptureRecord, String> {
    let dir = ensure_captures_dir(worktree_path, task_id)?;
    let id = next_capture_id();
    let image_path =
        screen_rect.and_then(|rect| take_screenshot(&rect, &dir.join(format!("{id}.png"))));
    let record = build_record(id, task_id, capture, image_path);
    write_record(&dir, &record)?;
    Ok(record)
}

pub fn list_captures(worktree_path: &Path, task_id: i64) -> Result<Vec<CaptureRecord>, String> {
    let mut records = Vec::new();
    for dir in checked_capture_task_dirs(worktree_path, task_id) {
        for entry in fs::read_dir(&dir).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("json")
            {
                continue;
            }
            let content = fs::read_to_string(entry.path()).map_err(|error| error.to_string())?;
            if let Ok(record) = serde_json::from_str::<CaptureRecord>(&content) {
                records.push(record);
            }
        }
    }
    records.sort_by(|left, right| left.captured_at.cmp(&right.captured_at));
    Ok(records)
}

/// 지정 캡처의 json과 동명 이미지(있으면)를 모두 삭제(idempotent).
/// 스크린샷은 png 고정이지만 붙여넣기 캡처는 클립보드 mime에 따라 확장자가 다르다.
pub fn remove_capture(worktree_path: &Path, task_id: i64, capture_id: &str) -> Result<(), String> {
    if !valid_capture_id(capture_id) {
        return Err("유효하지 않은 캡처 id입니다".into());
    }
    for dir in checked_capture_task_dirs(worktree_path, task_id) {
        // 삭제 실패(이미 없음 등)는 결과적으로 무해 — best-effort.
        let _ = fs::remove_file(dir.join(format!("{capture_id}.json")));
        for ext in ["png", "jpg", "jpeg", "gif", "webp", "bmp"] {
            let _ = fs::remove_file(dir.join(format!("{capture_id}.{ext}")));
        }
    }
    Ok(())
}

pub fn cleanup_captures(worktree_path: &Path, task_id: i64) {
    for project_dir in crate::project_paths::DIRS {
        let _ = remove_checked_task_captures(worktree_path, task_id, project_dir);
    }
}

/// 작업 영구 삭제에만 쓰는 checked 정리 — 공유 저장소와 다른 task의 captures는 건드리지 않는다.
pub fn delete_task_captures(worktree_path: &Path, task_id: i64) -> Result<(), String> {
    for project_dir in crate::project_paths::DIRS {
        remove_checked_task_captures(worktree_path, task_id, project_dir)?;
    }
    Ok(())
}

pub(super) fn checked_capture_task_dirs(worktree_path: &Path, task_id: i64) -> Vec<PathBuf> {
    crate::project_paths::DIRS
        .into_iter()
        .filter_map(|project_dir| {
            match checked_capture_task_dir(worktree_path, task_id, project_dir) {
                Ok(Some(dir)) => Some(dir),
                Ok(None) | Err(_) => None,
            }
        })
        .collect()
}

fn checked_capture_task_dir(
    worktree_path: &Path,
    task_id: i64,
    project_dir: &str,
) -> Result<Option<PathBuf>, String> {
    let Some(captures) = checked_captures_dir(worktree_path, project_dir)? else {
        return Ok(None);
    };
    let task_dir = captures.join(task_id.to_string());
    if !checked_directory(&task_dir, "캡처 task")? {
        return Ok(None);
    }
    Ok(Some(task_dir))
}

fn remove_checked_task_captures(
    worktree_path: &Path,
    task_id: i64,
    project_dir: &str,
) -> Result<(), String> {
    let Some(captures) = checked_captures_dir(worktree_path, project_dir)? else {
        return Ok(());
    };
    let task_dir = captures.join(task_id.to_string());
    let metadata = match fs::symlink_metadata(&task_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    if metadata.file_type().is_symlink() {
        return fs::remove_file(task_dir).map_err(|error| error.to_string());
    }
    if !metadata.is_dir() {
        return Err("캡처 task 디렉터리가 아닙니다".into());
    }
    fs::remove_dir_all(task_dir).map_err(|error| error.to_string())
}

fn checked_captures_dir(
    worktree_path: &Path,
    project_dir: &str,
) -> Result<Option<PathBuf>, String> {
    if !checked_directory(worktree_path, "worktree")? {
        return Ok(None);
    }
    let project = worktree_path.join(project_dir);
    if !checked_directory(&project, project_dir)? {
        return Ok(None);
    }
    let captures = project.join(CAPTURES_DIRNAME);
    if !checked_directory(&captures, "captures")? {
        return Ok(None);
    }
    Ok(Some(captures))
}

fn checked_directory(path: &Path, label: &str) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    if metadata.file_type().is_symlink() {
        return Err(format!("{label} 경로가 심볼릭 링크입니다"));
    }
    if !metadata.is_dir() {
        return Err(format!("{label} 경로가 디렉터리가 아닙니다"));
    }
    Ok(true)
}
