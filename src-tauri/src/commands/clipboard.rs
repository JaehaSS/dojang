//! 시스템 클립보드 쓰기.
//!
//! WKWebView의 `navigator.clipboard.writeText`는 사용자 활성화가 없으면
//! `NotAllowedError`로 거절한다. 웹 쪽 정책에 기대지 않도록 NSPasteboard에 직접 쓴다.

/// 텍스트를 시스템 클립보드에 쓴다. macOS 외 플랫폼은 프론트가 웹 API로 되돌아간다.
#[tauri::command]
pub fn clipboard_write_text(text: String) -> Result<(), String> {
    write_text(&text)
}

#[cfg(target_os = "macos")]
fn write_text(text: &str) -> Result<(), String> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    use objc2_foundation::NSString;

    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    // SAFETY: AppKit이 내보내는 불변 상수다.
    let kind = unsafe { NSPasteboardTypeString };
    if board.setString_forType(&NSString::from_str(text), kind) {
        Ok(())
    } else {
        Err("클립보드에 쓰지 못했습니다".into())
    }
}

#[cfg(not(target_os = "macos"))]
fn write_text(_text: &str) -> Result<(), String> {
    Err("이 플랫폼에는 네이티브 클립보드 쓰기가 없습니다".into())
}
