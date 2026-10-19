//! Design Mode — 임베디드 웹뷰에서 요소를 클릭해 outerHTML+computed CSS+boundingRect를
//! 캡처한다(local 전용, runner에 노출하지 않음 — designs/0012 §6.8).
//!
//! **스크린샷**: tauri 2.11.3에는 webview 픽셀 캡처 API가 없어, macOS에서는 `screencapture`
//! CLI를 shellout으로 호출해 요소의 스크린 절대좌표 영역만 크롭 캡처한다(`take_screenshot`).
//! Screen Recording TCC 권한이 없으면 명령이 조용히 실패하고 `image_path`는 `None`으로
//! 남는다(best-effort — 사용자는 HTML/CSS 캡처만으로도 계속 작업할 수 있어야 한다). 비-macOS
//! 플랫폼에서는 항상 `None`이다.
//!
//! 새 캡처 파일은 `<worktree>/.dojang/captures/<task_id>/<id>.json`에 저장한다. 기존
//! `.praxis` 캡처도 읽고 정리한다(ADR 0031 경로 정책 — worktree 내부만 허용).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

mod attachments;
mod capture;
mod editor;
mod pasted;
pub use attachments::validate_capture_image_paths;
pub use capture::{
    captures_dir, cleanup_captures, delete_task_captures, list_captures, remove_capture,
    save_capture, save_capture_with_screenshot, screen_capture_rect, viewport_capture_rect,
};
pub use editor::save_editor_capture;
pub use pasted::save_pasted_capture;

/// 자식 웹뷰 `initialization_script`로 주입하는 요소 선택기 — 순수 로직은
/// `inject.test.js`(vitest, jsdom)로 검증한다.
pub const INJECT_JS: &str = include_str!("inject.js");

/// Phase 1 preview action bridge. 선택기와 함께 모든 PreviewMode에 native user script로 주입한다.
pub const PREVIEW_INIT_JS: &str = concat!(
    include_str!("inject.js"),
    "\n",
    include_str!("preview_agent.js"),
    "\n",
    include_str!("exec.js"),
    "\n",
    include_str!("actions.js"),
    "\n",
    include_str!("console.js")
);

/// Design Canvas 전용 초기화 스크립트. raw 선택기 자체는 일반 Preview에도 주입되므로,
/// 네이티브 전달 경로를 열었다는 표식은 이 builder 전용 스크립트에서만 설정한다.
pub const DESIGN_CANVAS_INIT_JS: &str = concat!(
    include_str!("inject.js"),
    "\nwindow.__dojangCanvasNative=true;"
);

/// 레이아웃·색·타이포 중심 화이트리스트 — 프롬프트 비대화 방지(design doc §6.8, ~40속성).
pub const CSS_WHITELIST: &[&str] = &[
    "display",
    "position",
    "top",
    "right",
    "bottom",
    "left",
    "width",
    "height",
    "min-width",
    "min-height",
    "max-width",
    "max-height",
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "padding",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "box-sizing",
    "flex",
    "flex-direction",
    "flex-wrap",
    "align-items",
    "justify-content",
    "gap",
    "grid-template-columns",
    "color",
    "background-color",
    "border",
    "border-radius",
    "border-width",
    "border-style",
    "border-color",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "line-height",
    "letter-spacing",
    "text-align",
    "text-decoration",
    "opacity",
    "box-shadow",
    "z-index",
    "overflow",
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BoundingRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// inject.js가 postMessage(커스텀 스킴 navigation)로 보내는 원본 페이로드.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementCapture {
    pub outer_html: String,
    pub computed_css: BTreeMap<String, String>,
    pub bounding_rect: BoundingRect,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSource {
    #[default]
    Preview,
    Editor,
    /// 컴포저에 클립보드로 붙여넣은 이미지 — HTML/CSS 없이 image_path만 갖는다.
    Paste,
}

/// 프리뷰 웹뷰가 어디에 사는가.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PreviewMode {
    /// 사이드패널 안 자식 웹뷰 — 메인 창에 종속된다.
    Inline,
    /// 독립 OS 창 — 기본값. 사이드패널 폭에 갇히지 않는다.
    #[default]
    Window,
}

impl PreviewMode {
    /// 앱이 이 웹뷰의 위치·크기·표시를 관리하는가.
    /// 창 모드에서는 사용자와 OS가 관리하므로 앱이 bounds를 강제하지 않는다 —
    /// 탭을 떠났다고 창을 숨기면 "따로 띄워 크게 본다"가 성립하지 않는다.
    pub fn owns_geometry(self) -> bool {
        matches!(self, PreviewMode::Inline)
    }
}

#[derive(Debug, Clone)]
pub struct EditorCapture {
    pub file_path: String,
    pub selection_text: Option<String>,
    pub selection_start_line: Option<u32>,
    pub selection_end_line: Option<u32>,
    pub bounding_rect: BoundingRect,
}

/// 저장된 캡처 — Composer 캡처 칩과 프롬프트 주입이 이 형태를 그대로 소비한다.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureRecord {
    pub id: String,
    pub task_id: i64,
    #[serde(default)]
    pub source: CaptureSource,
    pub outer_html: String,
    pub computed_css: BTreeMap<String, String>,
    pub bounding_rect: BoundingRect,
    pub captured_at: i64,
    /// 스크린샷 절대경로 — macOS에서 `screencapture` shellout이 성공하면 채워지고,
    /// TCC 미허용·비-macOS 등 실패 시 `None`(best-effort, HTML/CSS 캡처는 계속 유효).
    pub image_path: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub selection_text: Option<String>,
    #[serde(default)]
    pub selection_start_line: Option<u32>,
    #[serde(default)]
    pub selection_end_line: Option<u32>,
}

#[cfg(test)]
mod tests;
