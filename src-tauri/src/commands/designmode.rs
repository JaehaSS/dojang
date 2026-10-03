//! 디자인 모드 프리뷰 창 — 배치·캡처·내비게이션.
//!
//! `commands.rs`에서 갈라져 나왔다 — 모든 기능이 한 파일 끝에 줄을 붙이는
//! 구조가 병행 worktree의 머지 충돌을 만들었다.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager, State};

use crate::db::{self};
use crate::designmode;

use super::{
    close_preview_surface, pool_of, AppState, DesignBounds, DesignCapturePayload,
    DesignModeScreenGeometry, EditorCaptureRequest, PreviewHandle,
};

mod activation;
pub use activation::*;

const DESIGNMODE_CAPTURE_SCHEME: &str = "praxis-designmode";

/// 프리뷰 창·웹뷰 라벨 — **같은 작업이라도 열 때마다 새 라벨을 쓴다.**
///
/// 창을 파괴해도 tauri의 창 맵에서 라벨이 빠지는 것은 이벤트 루프가 `Destroyed`를 처리한
/// 뒤다. 라벨이 고정이면 그 사이에 다시 연 창이 `WindowLabelAlreadyExists`로 튕기고,
/// 사용자에게는 "프리뷰가 다시는 안 열린다"로만 보인다 — 원인이 화면에 드러나지 않는다.
/// 라벨을 유일하게 만들면 그 창이 아예 생기지 않는다.
fn next_preview_generation() -> u64 {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

fn designmode_label(id: i64, generation: u64) -> String {
    format!("designmode-{id}-{generation}")
}

fn toolbar_label(id: i64, generation: u64) -> String {
    format!("previewbar-{id}-{generation}")
}

/// bounds가 창 밖으로 나가지 않도록 보정한다. 프론트 stale 좌표(리사이즈 이동 미감지 등)나
/// 잘못된 입력에 대한 최후 방어선 — 음수 좌표는 0으로 당기고, 넘치는 만큼 **크기를 줄인다.**
///
/// 원점은 절대 위로/왼쪽으로 옮기지 않는다. 네이티브 자식 웹뷰는 z-index와 무관하게 메인
/// 웹뷰의 DOM 위에 그려지므로, 넘침을 위치 이동으로 흡수하면 웹뷰가 컨테이너를 벗어나 위쪽
/// UI(프리뷰 툴바의 URL 입력줄)를 덮어버린다. 그리고 넘침은 잘못된 입력에서만 오는 게 아니다 —
/// `window.inner_size()`(창 클라이언트 좌표계)와 `getBoundingClientRect()`(메인 웹뷰 뷰포트
/// 좌표계)는 같다는 보장이 없고, 리사이즈 도중에는 프론트 레이아웃이 한 프레임 뒤처진다.
/// 그런 정상 경로에서 UI가 먹히면 안 된다 — 아래가 조금 잘려 보이는 편이 낫다.
fn clamp_bounds(bounds: DesignBounds, win_width: f64, win_height: f64) -> DesignBounds {
    let x = bounds.x.max(0.0);
    let y = bounds.y.max(0.0);
    let width = bounds.width.min((win_width - x).max(0.0)).max(1.0);
    let height = bounds.height.min((win_height - y).max(0.0)).max(1.0);
    DesignBounds {
        x,
        y,
        width,
        height,
    }
}

/// 창 모드 프리뷰의 기본 크기(논리 px). 자리가 좁으면 폭만 줄어든다.
const PREVIEW_WINDOW_SIZE: (f64, f64) = (1280.0, 860.0);

/// 공통 툴바의 최소 폭. 창 높이는 최대 툴바 160px와 페이지 320px를 확보한다.
const MIN_PREVIEW_WINDOW_WIDTH: f64 = 320.0;

/// 창 모드 프리뷰가 앉을 자리 — **조작면을 덮지 않는 곳**. 모두 논리 px·스크린 좌표계.
///
/// 창 모드여도 조작면은 여전히 메인 창 안에 있다. URL 입력줄도 선택 버튼도 프리뷰 탭 툴바에
/// 있고, 사용자는 주소를 넣고 Enter를 친 **뒤에** 그것들을 다시 쓴다. 그런데 예전에는
/// `center()`로 열어, 1280×860 창이 메인 창 한복판에 겹쳐 그 툴바를 그대로 덮었다 —
/// 렌더링에 성공한 순간 다음 조작을 할 곳이 사라지는 셈이다.
///
/// 그래서 패널이 차지한 가로 구간(`panel` = 왼쪽 x·폭)의 좌우 중 **더 넓은 쪽**에 앉힌다.
/// 겹치지 않고 들어가면 그 자리를 그대로 쓰고, 좁으면 그 공간에 맞춰 폭을 줄인다.
/// 세로는 보지 않는다 — 패널은 창 높이를 거의 다 쓰므로 위아래로 비켜 줄 자리가 없다.
fn preview_window_placement(
    work_area: (f64, f64, f64, f64),
    panel: (f64, f64),
    preferred: (f64, f64),
) -> (f64, f64, f64, f64) {
    let (work_x, work_y, work_w, work_h) = work_area;
    let (panel_x, panel_w) = panel;
    let (pref_w, pref_h) = preferred;

    let left = (panel_x - work_x).max(0.0);
    let right = (work_x + work_w - (panel_x + panel_w)).max(0.0);
    let put_left = left > right;

    let space = if put_left { left } else { right };
    let width = pref_w.min(space.max(MIN_PREVIEW_WINDOW_WIDTH)).min(work_w);
    let height = pref_h.min(work_h).max(1.0);
    let y = work_y + ((work_h - height) / 2.0).max(0.0);
    let x = if put_left {
        // 패널 왼쪽에 오른쪽 정렬 — 붙여 두면 시선 이동이 짧고, 넘치면 작업영역이 막아선다.
        (panel_x - width).max(work_x)
    } else {
        (panel_x + panel_w).min(work_x + work_w - width).max(work_x)
    };
    (x, y, width, height)
}

/// 실제 창·모니터를 읽어 [`preview_window_placement`]에 넘긴다. 하나라도 읽지 못하면
/// `None` — 호출부가 예전처럼 화면 중앙으로 떨어진다(자리를 못 고르는 것이 못 여는 것보다 낫다).
fn preview_window_frame(app: &AppHandle, panel: DesignBounds) -> Option<(f64, f64, f64, f64)> {
    let window = app.get_window("main")?;
    let scale = window.scale_factor().ok()?;
    let origin = window.inner_position().ok()?.to_logical::<f64>(scale);
    let monitor = window.current_monitor().ok()??;
    let monitor_scale = monitor.scale_factor();
    let work = monitor.work_area();
    let work_position = work.position.to_logical::<f64>(monitor_scale);
    let work_size = work.size.to_logical::<f64>(monitor_scale);
    // 프리뷰 탭은 자기 자리를 메인 창 뷰포트 좌표로 보낸다 — 창 원점을 더해 스크린으로 옮긴다.
    Some(preview_window_placement(
        (
            work_position.x,
            work_position.y,
            work_size.width,
            work_size.height,
        ),
        (origin.x + panel.x, panel.width),
        PREVIEW_WINDOW_SIZE,
    ))
}

/// 창 핸들에서 논리 크기(width, height)를 얻는다. 조회 실패 시 클램프를 생략할 수 있도록 `None`을 반환한다.
fn window_logical_size(window: &tauri::Window) -> Option<(f64, f64)> {
    let size = window.inner_size().ok()?;
    let scale = window.scale_factor().ok()?;
    let logical = size.to_logical::<f64>(scale);
    Some((logical.width, logical.height))
}

/// 자식 웹뷰 y 보정값 — 창 프레임 높이와 클라이언트 높이의 차(macOS에서는 타이틀바 높이).
///
/// wry 0.55는 macOS 자식 웹뷰의 y를 뒤집을 때(`window_position`) 부모 뷰 높이로 **창 프레임
/// 높이**를 쓴다. 우리가 넘기는 y는 `getBoundingClientRect()` — 즉 메인 웹뷰 뷰포트(=클라이언트
/// 영역) 기준이라, 그대로 주면 웹뷰가 두 좌표계의 차이만큼 **위로** 올라가 프리뷰 툴바의 URL
/// 입력줄을 덮는다. 네이티브 웹뷰는 DOM 위에 그려지므로 입력 자체가 불가능해진다.
///
/// 실측(1800×1075 창): 프론트가 y=100·height=919를 보냈는데 웹뷰는 y=72에 앉았다.
/// `1075 - 100 - 919 = 56`을 클라이언트 높이 1047로 되돌리면 정확히 72 — 어긋난 값이 28px,
/// 곧 타이틀바 높이다. 그만큼 y를 내려 상쇄한다.
///
/// 전체화면처럼 타이틀바가 없으면 두 높이가 같아 보정이 저절로 0이 된다.
fn titlebar_offset(outer_height: f64, inner_height: f64) -> f64 {
    (outer_height - inner_height).max(0.0)
}

/// 창에서 [`titlebar_offset`]을 읽는다. macOS 외 플랫폼은 자식 웹뷰가 클라이언트 좌표를
/// 그대로 쓰므로 보정하지 않는다. 조회 실패 시 0(보정 생략)으로 떨어진다.
fn window_titlebar_offset(window: &tauri::Window) -> f64 {
    #[cfg(target_os = "macos")]
    {
        let Ok(scale) = window.scale_factor() else {
            return 0.0;
        };
        let (Ok(outer), Ok(inner)) = (window.outer_size(), window.inner_size()) else {
            return 0.0;
        };
        titlebar_offset(
            outer.to_logical::<f64>(scale).height,
            inner.to_logical::<f64>(scale).height,
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        0.0
    }
}

/// 프론트가 보낸 뷰포트 좌표를 네이티브 자식 웹뷰가 받는 좌표로 옮긴다.
/// 클램프는 뷰포트 좌표계에서 먼저 하고(그래야 창 밖 판정이 맞다), 보정은 마지막에 더한다.
fn native_child_bounds(window: &tauri::Window, bounds: DesignBounds) -> DesignBounds {
    let bounds = match window_logical_size(window) {
        Some((win_width, win_height)) => clamp_bounds(bounds, win_width, win_height),
        None => bounds, // 창 크기 조회 실패 — best-effort로 원본 bounds 그대로 적용.
    };
    DesignBounds {
        y: bounds.y + window_titlebar_offset(window),
        ..bounds
    }
}

fn window_toolbar_height(toolbar_height: &Mutex<f64>) -> f64 {
    *toolbar_height
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn set_window_children(
    window: &tauri::Window,
    toolbar: &tauri::Webview,
    page: &tauri::Webview,
    toolbar_height: f64,
) {
    let (width, height) = window_logical_size(window).unwrap_or(PREVIEW_WINDOW_SIZE);
    let toolbar_height = toolbar_height.clamp(96.0, 160.0);
    let offset = window_titlebar_offset(window);
    let _ = toolbar.set_bounds(tauri::Rect {
        position: tauri::Position::Logical(tauri::LogicalPosition::new(0.0, offset)),
        size: tauri::Size::Logical(tauri::LogicalSize::new(width, toolbar_height)),
    });
    let _ = page.set_bounds(tauri::Rect {
        position: tauri::Position::Logical(tauri::LogicalPosition::new(
            0.0,
            offset + toolbar_height,
        )),
        size: tauri::Size::Logical(tauri::LogicalSize::new(
            width,
            (height - toolbar_height).max(320.0),
        )),
    });
}

pub(crate) fn set_window_toolbar_height(
    state: &AppState,
    task_id: i64,
    label: &str,
    generation: u64,
    height: f64,
) -> Result<(), String> {
    let handles = state
        .designmode_webviews
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let handle = handles.get(&task_id).ok_or("프리뷰가 닫혔습니다")?;
    if handle.generation != generation {
        return Err("툴바 창 세대가 만료되었습니다".into());
    }
    let toolbar = handle.toolbar.as_ref().ok_or("별도 창 툴바가 없습니다")?;
    if toolbar.label() != label {
        return Err("툴바 작업이 일치하지 않습니다".into());
    }
    let toolbar_height = handle
        .toolbar_height
        .as_ref()
        .ok_or("프리뷰 툴바 높이가 없습니다")?;
    let height = height.clamp(96.0, 160.0);
    *toolbar_height
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = height;
    let window = handle.window.as_ref().ok_or("프리뷰 창이 없습니다")?;
    set_window_children(window, toolbar, &handle.webview, height);
    Ok(())
}

fn set_webview_bounds(webview: &tauri::Webview, bounds: DesignBounds) -> Result<(), String> {
    let bounds = native_child_bounds(&webview.window(), bounds);
    webview
        .set_bounds(tauri::Rect {
            position: tauri::Position::Logical(tauri::LogicalPosition::new(bounds.x, bounds.y)),
            size: tauri::Size::Logical(tauri::LogicalSize::new(bounds.width, bounds.height)),
        })
        .map_err(|e| e.to_string())
}

/// 웹뷰/창의 현재 geometry(논리 px) — 스크린샷 영역 합성에 필요한 값만 값 복사로 가져온다.
/// `intercept_capture_navigation`은 동기 반환해야 하므로, 락을 쥔 채로든 스레드로 넘기지 않기
/// 위해 이 함수에서 필요한 값만 뽑아 반환한다.
fn designmode_screen_geometry(app: &AppHandle, task_id: i64) -> Option<DesignModeScreenGeometry> {
    let state = app.state::<AppState>();
    let webviews = state.designmode_webviews.lock().ok()?;
    let webview = &webviews.get(&task_id)?.webview;
    let window = webview.window();
    let scale = window.scale_factor().ok()?;
    let webview_origin = webview.position().ok()?.to_logical::<f64>(scale);
    let webview_size = webview.size().ok()?.to_logical::<f64>(scale);
    let window_origin = window.inner_position().ok()?.to_logical::<f64>(scale);
    Some((
        (webview_origin.x, webview_origin.y),
        (webview_size.width, webview_size.height),
        (window_origin.x, window_origin.y),
    ))
}

/// `on_navigation` 훅 — 커스텀 스킴이면 캡처로 소비하고 항상 navigate를 취소(`false`)한다.
/// 그 외 모든 URL(=dev 서버 자체 내비게이션)은 통과시킨다. 스크린샷 shellout은 블로킹이므로
/// geometry만 여기서 값 복사로 조회하고, 저장·촬영·emit은 별도 스레드로 넘겨 즉시 반환한다.
fn intercept_capture_navigation(
    app: &AppHandle,
    bridge: &crate::preview_bridge::PreviewBridge,
    task_id: i64,
    worktree_path: &Path,
    url: &tauri::Url,
) -> bool {
    if url.scheme() != DESIGNMODE_CAPTURE_SCHEME {
        let _ = bridge.observe_navigation(task_id);
        return true;
    }
    let Some(encoded) = url
        .query_pairs()
        .find(|(k, _)| k == "data")
        .map(|(_, v)| v.into_owned())
    else {
        return false;
    };
    let Ok(capture) = serde_json::from_str::<designmode::ElementCapture>(&encoded) else {
        return false; // 실패해도 조용히 무시 — 사용자가 다시 클릭해 재시도할 수 있다.
    };
    // 요소 rect가 아니라 웹뷰 뷰포트 전체를 찍는다 — 클릭 시점의 하이라이트 박스가 주변 화면과
    // 함께 담겨야 이미지만으로 "어디를 고칠지"가 전달된다(요소 rect는 record에 그대로 남는다).
    let screen_rect = designmode_screen_geometry(app, task_id).and_then(
        |(webview_origin, webview_size, window_origin)| {
            designmode::viewport_capture_rect(webview_origin, webview_size, window_origin)
        },
    );
    let app = app.clone();
    let worktree_path = worktree_path.to_path_buf();
    std::thread::spawn(move || {
        if let Ok(record) =
            designmode::save_capture_with_screenshot(&worktree_path, task_id, capture, screen_rect)
        {
            let _ = app.emit(
                "designmode://capture",
                DesignCapturePayload { task_id, record },
            );
        }
    });
    false
}

/// URL policy is centralized: reject https, username userinfo, a missing port(), and any host
/// other than localhost or 127.0.0.1 before creating the remote preview webview.
pub fn start_preview_probe(app: &tauri::App, url: &str) -> Result<(), String> {
    let parsed = crate::preview_bridge::validate_preview_probe_url(url)?;
    let preview_bridge = app.state::<AppState>().preview_bridge.clone();
    let session_id = crate::preview_bridge::new_session_id()?;
    let command_id = crate::preview_bridge::new_command_id()?;
    let label = crate::preview_bridge::PREVIEW_PROBE_WEBVIEW_LABEL.to_string();
    preview_bridge
        .register(crate::preview_bridge::SessionRegistration::new(
            crate::preview_bridge::PREVIEW_PROBE_TASK_ID,
            &label,
            &session_id,
            0,
        ))
        .map_err(|reason| format!("probe registration rejected: {reason:?}"))?;
    let pending = crate::preview_bridge::PendingAction::new(
        crate::preview_bridge::PREVIEW_PROBE_TASK_ID,
        &session_id,
        0,
        &command_id,
    );
    let begun = preview_bridge.begin(pending);
    let _waiter = begun.map_err(|reason| format!("probe pending rejected: {reason:?}"))?;
    preview_bridge.start_probe();
    build_preview_probe(app, parsed, label, session_id, command_id, preview_bridge)
}

fn build_preview_probe(
    app: &tauri::App,
    url: tauri::Url,
    label: String,
    session_id: String,
    command_id: String,
    preview_bridge: crate::preview_bridge::PreviewBridge,
) -> Result<(), String> {
    let submitted = Arc::new(AtomicBool::new(false));
    let script = preview_probe_script(&session_id, &command_id)?;
    let builder = tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::External(url))
        .visible(false)
        .on_page_load(move |webview, payload| {
            if payload.event() != tauri::webview::PageLoadEvent::Finished
                || submitted.swap(true, Ordering::SeqCst)
            {
                return;
            }
            if let Err(error) = webview.eval(script.clone()) {
                eprintln!("preview IPC probe eval failed: {error}");
            }
        });
    if let Err(error) = builder.build() {
        preview_bridge.close(crate::preview_bridge::PREVIEW_PROBE_TASK_ID);
        return Err(error.to_string());
    }
    Ok(())
}

fn preview_probe_script(session_id: &str, command_id: &str) -> Result<String, String> {
    let session = serde_json::to_string(session_id).map_err(|error| error.to_string())?;
    let command = serde_json::to_string(command_id).map_err(|error| error.to_string())?;
    Ok(format!(
        "(async()=>{{const i=window.__TAURI_INTERNALS__,p=window.__praxisStrictCspProbe;if(!i||!p)throw Error('probe unavailable');try{{await i.invoke('designmode_close',{{id:{}}});throw Error('remote app command allowed')}}catch(e){{if(!String(e).toLowerCase().includes('not allowed'))throw e;}}const b=new TextEncoder().encode(p.payload),d=await crypto.subtle.digest('SHA-256',b),h=[...new Uint8Array(d)].map(x=>x.toString(16).padStart(2,'0')).join('');await i.invoke('plugin:preview-bridge|submit_result',{{result:{{taskId:{},sessionId:{},generation:0,commandId:{},sha256:h,body:p.payload}}}});}})();",
        crate::preview_bridge::PREVIEW_PROBE_TASK_ID,
        crate::preview_bridge::PREVIEW_PROBE_TASK_ID,
        session,
        command
    ))
}

/// 프리뷰 진입 — 독립 창(기본) 또는 사이드패널 자식 웹뷰를 만든다. 이미 있으면 navigate만.
/// local 전용.
#[tauri::command]
pub async fn designmode_open(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    url: String,
    bounds: DesignBounds,
    // 생략하면 창 모드. 커맨드 인자에는 `#[serde(default)]`를 붙일 수 없어 Option으로 받는다.
    mode: Option<designmode::PreviewMode>,
) -> Result<(), String> {
    let generation = next_preview_generation();
    let opening = state
        .preview_openings
        .begin(id, generation)
        .map_err(|error| error.as_str().to_string())?;
    let mode = mode.unwrap_or_default();
    let pool = pool_of(&state)?;
    let task = db::get_task(&pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    let worktree_path = PathBuf::from(&task.worktree_path);
    let parsed = tauri::Url::parse(&url).map_err(|e| e.to_string())?;

    let existing = state.designmode_webviews.lock().unwrap().get(&id).cloned();
    if let Some(handle) = existing {
        let preview_bridge = state.preview_bridge.clone();
        preview_bridge
            .prepare_navigation(id)
            .map_err(|reason| format!("NAVIGATION_CANCELLED: {reason:?}"))?;
        if let Err(error) = handle.webview.navigate(parsed) {
            let _ = preview_bridge.cancel_prepared_navigation(id);
            return Err(format!("NAVIGATION_CANCELLED: {error}"));
        }
        if handle.mode.owns_geometry() {
            set_webview_bounds(&handle.webview, bounds)?;
        }
        return match &handle.window {
            Some(window) => window.show().map_err(|error| error.to_string()),
            None => handle.webview.show().map_err(|error| error.to_string()),
        };
    }

    create_preview(
        &app,
        &state,
        id,
        parsed,
        bounds,
        mode,
        worktree_path,
        &opening,
    )
}

fn create_preview(
    app: &AppHandle,
    state: &AppState,
    id: i64,
    parsed: tauri::Url,
    bounds: DesignBounds,
    mode: designmode::PreviewMode,
    worktree_path: PathBuf,
    opening: &crate::preview_control::openings::Opening,
) -> Result<(), String> {
    let generation = opening.generation;
    let label = designmode_label(id, generation);
    let bridge_label = label.clone();
    let (webview, toolbar, toolbar_height, container) = match mode {
        designmode::PreviewMode::Window => {
            let nav_app = app.clone();
            let nav_bridge = state.preview_bridge.clone();
            let builder =
                tauri::window::WindowBuilder::new(app, format!("preview-window-{id}-{generation}"))
                    .title(format!("Dojang Preview — task #{id}"))
                    .min_inner_size(MIN_PREVIEW_WINDOW_WIDTH, 480.0);
            let builder = match preview_window_frame(app, bounds) {
                Some((x, y, width, height)) => builder.position(x, y).inner_size(width, height),
                None => builder
                    .inner_size(PREVIEW_WINDOW_SIZE.0, PREVIEW_WINDOW_SIZE.1)
                    .center(),
            };
            let window = builder.build().map_err(|e| e.to_string())?;
            let (width, height) = window_logical_size(&window).unwrap_or(PREVIEW_WINDOW_SIZE);
            let toolbar_height = Arc::new(Mutex::new(96.0));
            let toolbar = match window.add_child(
                tauri::webview::WebviewBuilder::new(
                    toolbar_label(id, generation),
                    tauri::WebviewUrl::App(
                        format!("index.html?window=preview-toolbar&task={id}").into(),
                    ),
                ),
                tauri::LogicalPosition::new(0.0, window_titlebar_offset(&window)),
                tauri::LogicalSize::new(width, window_toolbar_height(&toolbar_height)),
            ) {
                Ok(toolbar) => toolbar,
                Err(error) => {
                    let _ = window.destroy();
                    return Err(error.to_string());
                }
            };
            let page = tauri::webview::WebviewBuilder::new(
                label.clone(),
                tauri::WebviewUrl::External(parsed),
            )
            .initialization_script(designmode::PREVIEW_INIT_JS)
            .on_page_load(activation::page_loaded)
            .on_navigation(move |nav_url| {
                intercept_capture_navigation(&nav_app, &nav_bridge, id, &worktree_path, nav_url)
            });
            let page_toolbar_height = window_toolbar_height(&toolbar_height);
            let page = match window.add_child(
                page,
                tauri::LogicalPosition::new(
                    0.0,
                    page_toolbar_height + window_titlebar_offset(&window),
                ),
                tauri::LogicalSize::new(width, (height - page_toolbar_height).max(320.0)),
            ) {
                Ok(page) => page,
                Err(error) => {
                    let _ = window.destroy();
                    return Err(error.to_string());
                }
            };
            let resize_window = window.clone();
            let resize_toolbar = toolbar.clone();
            let resize_page = page.clone();
            let resize_toolbar_height = toolbar_height.clone();

            // 창이 사라지면 웹뷰도 함께 죽는다 — 맵에 죽은 핸들을 남기면 다음 open이 그것을
            // 재사용해 아무 일도 일어나지 않는다. 프론트에도 알려 빈 상태로 되돌린다.
            //
            // **그 자리가 아직 이 창의 것일 때만** 정리한다. 라벨이 열 때마다 달라지므로,
            // 이미 다음 프리뷰가 자리를 넘겨받았으면 옛 창의 뒤늦은 이벤트가 그것을 지우지 못한다.
            // `Destroyed`도 함께 듣는다 — 창이 어떤 경로로 죽든 맵에 유령이 남지 않게.
            let close_app = app.clone();
            let close_label = label.clone();
            window.on_window_event(move |event| {
                if matches!(
                    event,
                    tauri::WindowEvent::Resized(_) | tauri::WindowEvent::ScaleFactorChanged { .. }
                ) {
                    set_window_children(
                        &resize_window,
                        &resize_toolbar,
                        &resize_page,
                        window_toolbar_height(&resize_toolbar_height),
                    );
                    return;
                }
                if !matches!(
                    event,
                    tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
                ) {
                    return;
                }
                let state = close_app.state::<AppState>();
                state.preview_openings.cancel_generation(id, generation);
                let removed = state.preview_openings.close_if_current(id, || {
                    let mut webviews = state.designmode_webviews.lock().unwrap();
                    let mine = webviews.get(&id).is_some_and(|handle| {
                        handle.webview.label() == close_label && handle.generation == generation
                    });
                    if !mine {
                        return false;
                    }
                    webviews.remove(&id);
                    drop(webviews);
                    state.preview_bridge.close(id);
                    true
                });
                if removed {
                    let _ = close_app.emit("designmode://closed", id);
                }
            });

            (page, Some(toolbar), Some(toolbar_height), Some(window))
        }
        designmode::PreviewMode::Inline => {
            let nav_app = app.clone();
            let nav_bridge = state.preview_bridge.clone();
            let window = app.get_window("main").ok_or("메인 창을 찾을 수 없습니다")?;
            let builder =
                tauri::webview::WebviewBuilder::new(label, tauri::WebviewUrl::External(parsed))
                    .initialization_script(designmode::PREVIEW_INIT_JS)
                    .on_page_load(activation::page_loaded)
                    .on_navigation(move |nav_url| {
                        intercept_capture_navigation(
                            &nav_app,
                            &nav_bridge,
                            id,
                            &worktree_path,
                            nav_url,
                        )
                    });
            let bounds = native_child_bounds(&window, bounds);
            let page = window
                .add_child(
                    builder,
                    tauri::LogicalPosition::new(bounds.x, bounds.y),
                    tauri::LogicalSize::new(bounds.width, bounds.height),
                )
                .map_err(|e| e.to_string())?;
            (page, None, None, None)
        }
    };
    let session_id = match crate::preview_bridge::new_session_id() {
        Ok(session_id) => session_id,
        Err(error) => {
            if let Some(window) = &container {
                let _ = window.destroy();
            } else {
                let _ = webview.close();
            }
            return Err(error);
        }
    };
    let handle = PreviewHandle {
        webview,
        toolbar,
        toolbar_height,
        window: container,
        generation,
        mode,
    };
    let published = opening.publish(|| {
        state
            .preview_bridge
            .register(crate::preview_bridge::SessionRegistration::new(
                id,
                bridge_label,
                session_id,
                generation,
            ))
            .map_err(|reason| format!("preview bridge registration rejected: {reason:?}"))?;
        state
            .designmode_webviews
            .lock()
            .unwrap()
            .insert(id, handle.clone());
        Ok(())
    });
    if let Err(error) = published {
        super::destroy_preview(&handle);
        return Err(error);
    }
    let _ = app.emit("designmode://changed", id);
    Ok(())
}

/// 탭 컨테이너 리사이즈/스크롤 시 위치·크기 동기화.
#[tauri::command]
pub fn designmode_set_bounds(
    state: State<'_, AppState>,
    id: i64,
    bounds: DesignBounds,
) -> Result<(), String> {
    let webviews = state.designmode_webviews.lock().unwrap();
    let handle = webviews.get(&id).ok_or("프리뷰 웹뷰가 없습니다")?;
    if !handle.mode.owns_geometry() {
        return Ok(()); // 창 모드 — 크기·위치는 사용자와 OS가 정한다.
    }
    set_webview_bounds(&handle.webview, bounds)
}

/// 기존 프리뷰 탭 재활성화 — 현재 문서를 다시 navigate하지 않고 bounds와 visibility만 복원한다.
/// 웹뷰가 아직 생성되지 않았으면 `false`를 반환해 프론트가 빈 상태를 유지하게 한다.
#[tauri::command]
pub fn designmode_show(
    state: State<'_, AppState>,
    id: i64,
    bounds: DesignBounds,
) -> Result<bool, String> {
    let webviews = state.designmode_webviews.lock().unwrap();
    let Some(handle) = webviews.get(&id) else {
        return Ok(false);
    };
    if !handle.mode.owns_geometry() {
        return Ok(true); // 창은 이미 떠 있다 — 프론트가 loaded 상태를 유지하게 true.
    }
    set_webview_bounds(&handle.webview, bounds)?;
    handle.webview.show().map_err(|e| e.to_string())?;
    Ok(true)
}

/// URL 바 이동 — 새로고침도 동일 URL로 재호출.
#[tauri::command]
pub fn designmode_navigate(state: State<'_, AppState>, id: i64, url: String) -> Result<(), String> {
    designmode_navigate_inner(&state, id, &url)
}

/// 에이전트 디스패처도 같은 경로로 옮긴다 — 세대 준비·롤백이 하나여야 회수 판정이 어긋나지 않는다.
pub(crate) fn designmode_navigate_inner(
    state: &AppState,
    id: i64,
    url: &str,
) -> Result<(), String> {
    let parsed = tauri::Url::parse(url).map_err(|e| e.to_string())?;
    let webview = state
        .designmode_webviews
        .lock()
        .unwrap()
        .get(&id)
        .ok_or("프리뷰 웹뷰가 없습니다")?
        .webview
        .clone();
    let preview_bridge = state.preview_bridge.clone();
    preview_bridge
        .prepare_navigation(id)
        .map_err(|reason| format!("NAVIGATION_CANCELLED: {reason:?}"))?;
    if let Err(error) = webview.navigate(parsed) {
        let _ = preview_bridge.cancel_prepared_navigation(id);
        return Err(format!("NAVIGATION_CANCELLED: {error}"));
    }
    Ok(())
}

/// 요소 선택 모드 토글 — inject.js의 하이라이트/클릭 캡처 리스너를 켜고 끈다.
#[tauri::command]
pub fn designmode_set_selection_mode(
    state: State<'_, AppState>,
    id: i64,
    enabled: bool,
) -> Result<(), String> {
    let webviews = state.designmode_webviews.lock().unwrap();
    let handle = webviews.get(&id).ok_or("프리뷰 웹뷰가 없습니다")?;
    let js =
        format!("window.__praxisDesignMode && window.__praxisDesignMode.setEnabled({enabled});");
    handle.webview.eval(js).map_err(|e| e.to_string())
}

/// 다른 중앙 탭으로 전환 시 프리뷰 웹뷰를 숨긴다(파괴하지 않음 — 되돌아오면 상태 유지).
#[tauri::command]
pub fn designmode_hide(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    match state.designmode_webviews.lock().unwrap().get(&id) {
        Some(handle) if handle.mode.owns_geometry() => {
            handle.webview.hide().map_err(|e| e.to_string())
        }
        // 창 모드거나 없음 — 탭을 떠나도 창은 남는다. 그게 별도 창의 요점이다.
        _ => Ok(()),
    }
}

/// 프리뷰 탭을 완전히 닫을 때(작업 전환 등) 웹뷰 자체를 해제.
#[tauri::command]
pub fn designmode_close(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    close_preview_surface(&state, id);
    Ok(())
}

/// 사용자가 프리뷰를 되찾는다 — 대기 중이던 에이전트 명령은 그 자리에서 끊긴다.
#[tauri::command]
pub fn preview_take_over(
    state: State<'_, AppState>,
    app: AppHandle,
    id: i64,
) -> Result<(), String> {
    state.preview_bridge.take_over(id);
    crate::preview_control::emit_control_state(&app, &state, id, "take_over", false);
    Ok(())
}

/// 회수 해제 — 다시 에이전트가 몰 수 있다.
#[tauri::command]
pub fn preview_release(state: State<'_, AppState>, app: AppHandle, id: i64) -> Result<(), String> {
    state.preview_bridge.release(id);
    crate::preview_control::emit_control_state(&app, &state, id, "release", true);
    Ok(())
}

/// Composer 캡처 칩의 × 제거.
#[tauri::command]
pub async fn designmode_remove_capture(
    state: State<'_, AppState>,
    id: i64,
    capture_id: String,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    let task = db::get_task(&pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    designmode::remove_capture(&PathBuf::from(&task.worktree_path), id, &capture_id)
}

/// 현재 메인 WebView의 에디터 DOM 영역을 PNG로 캡처하고 Composer 칩용 레코드를 반환한다.
#[tauri::command]
pub async fn designmode_capture_editor(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    capture: EditorCaptureRequest,
) -> Result<designmode::CaptureRecord, String> {
    let pool = pool_of(&state)?;
    let task = db::get_task(&pool, id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    let window = app.get_window("main").ok_or("메인 창을 찾을 수 없습니다")?;
    let scale = window.scale_factor().map_err(|error| error.to_string())?;
    let window_origin = window
        .inner_position()
        .map_err(|error| error.to_string())?
        .to_logical::<f64>(scale);
    let window_size = window
        .inner_size()
        .map_err(|error| error.to_string())?
        .to_logical::<f64>(scale);
    let screen_rect = designmode::screen_capture_rect(
        designmode::BoundingRect {
            x: capture.bounds.x,
            y: capture.bounds.y,
            width: capture.bounds.width,
            height: capture.bounds.height,
        },
        (0.0, 0.0),
        (window_size.width, window_size.height),
        (window_origin.x, window_origin.y),
    )
    .ok_or("에디터 캡처 영역이 화면에 보이지 않습니다")?;
    let worktree_path = PathBuf::from(task.worktree_path);
    let editor_capture = designmode::EditorCapture {
        file_path: capture.file_path,
        selection_text: capture.selection_text,
        selection_start_line: capture.selection_start_line,
        selection_end_line: capture.selection_end_line,
        bounding_rect: designmode::BoundingRect {
            x: capture.bounds.x,
            y: capture.bounds.y,
            width: capture.bounds.width,
            height: capture.bounds.height,
        },
    };
    tauri::async_runtime::spawn_blocking(move || {
        designmode::save_editor_capture(&worktree_path, id, editor_capture, screen_rect)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod clamp_bounds_tests {
    use super::*;

    fn bounds(x: f64, y: f64, width: f64, height: f64) -> DesignBounds {
        DesignBounds {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn bounds_inside_window_are_unchanged() {
        let b = clamp_bounds(bounds(10.0, 20.0, 300.0, 200.0), 800.0, 600.0);
        assert_eq!((b.x, b.y, b.width, b.height), (10.0, 20.0, 300.0, 200.0));
    }

    #[test]
    fn bounds_overflowing_right_or_bottom_are_shrunk_not_moved() {
        // 오른쪽/아래로 벗어남 -> 원점은 그대로, 넘치는 만큼 크기만 줄어든다.
        let b = clamp_bounds(bounds(700.0, 500.0, 300.0, 200.0), 800.0, 600.0);
        assert_eq!((b.x, b.y), (700.0, 500.0));
        assert_eq!((b.width, b.height), (100.0, 100.0));
    }

    #[test]
    fn negative_origin_is_clamped_to_zero() {
        let b = clamp_bounds(bounds(-50.0, -30.0, 200.0, 150.0), 800.0, 600.0);
        assert_eq!((b.x, b.y), (0.0, 0.0));
        assert_eq!((b.width, b.height), (200.0, 150.0));
    }

    #[test]
    fn bounds_larger_than_window_are_shrunk_to_fit() {
        let b = clamp_bounds(bounds(0.0, 0.0, 1000.0, 900.0), 800.0, 600.0);
        assert_eq!((b.x, b.y), (0.0, 0.0));
        assert_eq!((b.width, b.height), (800.0, 600.0));
    }

    /// 회귀: 프리뷰 웹뷰가 툴바(URL 입력줄) 아래에 앉아 있는데 아래로 넘치면, 예전 clamp는
    /// 크기를 유지한 채 y를 위로 당겨 웹뷰가 툴바를 덮었다. 네이티브 웹뷰는 DOM 위에 그려지므로
    /// 이 순간 URL 입력줄이 화면에서 사라진다. y는 절대 올라가면 안 된다.
    #[test]
    fn webview_never_rises_above_its_container_top() {
        let toolbar_bottom = 92.0;
        // 창 클라이언트 높이가 뷰포트보다 작게 잡히면(좌표계 불일치·리사이즈 지연) 넘침이 생긴다.
        let b = clamp_bounds(bounds(722.0, toolbar_bottom, 834.0, 929.0), 1557.0, 990.0);
        assert_eq!(b.y, toolbar_bottom, "웹뷰 상단이 툴바 아래를 유지해야 한다");
        assert_eq!(b.height, 898.0, "넘친 만큼만 아래가 줄어든다");
    }

    /// 원점이 창 밖으로 완전히 나가도 크기는 양수여야 한다(네이티브가 음수 크기를 받지 않도록).
    #[test]
    fn origin_outside_window_still_yields_positive_size() {
        let b = clamp_bounds(bounds(900.0, 700.0, 300.0, 200.0), 800.0, 600.0);
        assert_eq!((b.x, b.y), (900.0, 700.0));
        assert_eq!((b.width, b.height), (1.0, 1.0));
    }

    /// wry 0.55가 macOS 자식 웹뷰를 앉히는 계산 — y를 **창 프레임 높이** 기준으로 뒤집은 뒤
    /// (`window_position`), 화면에서는 클라이언트 좌표로 읽힌다. 우리가 보낸 y가 화면 어디에
    /// 떨어지는지를 이 왕복으로 확인한다.
    fn wry_placed_y(sent_y: f64, height: f64, frame_height: f64, client_height: f64) -> f64 {
        let origin_from_bottom = frame_height - sent_y - height;
        client_height - origin_from_bottom - height
    }

    #[test]
    fn titlebar_offset_is_frame_minus_client_height() {
        assert_eq!(titlebar_offset(1075.0, 1047.0), 28.0);
    }

    /// 전체화면·비장식 창은 타이틀바가 없어 두 높이가 같다 — 보정도 0이어야 한다.
    #[test]
    fn titlebar_offset_is_zero_without_a_titlebar() {
        assert_eq!(titlebar_offset(1047.0, 1047.0), 0.0);
        // 조회가 뒤집혀 들어와도 y를 위로 끌어올리지는 않는다.
        assert_eq!(titlebar_offset(1000.0, 1047.0), 0.0);
    }

    /// 회귀: 프리뷰 웹뷰가 타이틀바 높이만큼 위로 올라가 URL 입력줄을 덮었다.
    /// 실측(1800×1075 창, 클라이언트 1047) — 프론트가 y=100·height=919를 보냈는데 웹뷰는 y=72에
    /// 앉아 툴바를 먹었다. 보정을 더하면 의도한 100에 정확히 떨어져야 한다.
    #[test]
    fn titlebar_offset_lands_the_webview_on_its_container() {
        let (frame, client) = (1075.0, 1047.0);
        let (container_y, height) = (100.0, 919.0);

        let uncorrected = wry_placed_y(container_y, height, frame, client);
        assert_eq!(
            uncorrected, 72.0,
            "보정 전에는 타이틀바 높이만큼 위로 올라간다"
        );

        let corrected = wry_placed_y(
            container_y + titlebar_offset(frame, client),
            height,
            frame,
            client,
        );
        assert_eq!(corrected, container_y, "보정 후에는 컨테이너 상단에 앉는다");
    }

    /// 프리뷰 패널이 메인 창 오른쪽에 있는 보통의 배치 — 왼쪽이 더 넓으니 그쪽에 앉고,
    /// 패널을 조금도 덮지 않는다.
    #[test]
    fn preview_window_takes_the_wider_side_of_the_panel() {
        let (x, y, width, height) = preview_window_placement(
            (0.0, 25.0, 1920.0, 1055.0),
            (1120.0, 800.0),
            (1280.0, 860.0),
        );
        assert!(x + width <= 1120.0, "프리뷰 창이 패널을 덮으면 안 된다");
        assert_eq!((x, width), (0.0, 1120.0), "왼쪽 공간을 꽉 채운다");
        assert_eq!(height, 860.0);
        assert_eq!(y, 25.0 + (1055.0 - 860.0) / 2.0, "세로는 작업영역 중앙");
    }

    /// 패널이 왼쪽에 붙어 있으면 오른쪽에 앉는다 — 선호 폭이 들어가면 그대로 쓴다.
    #[test]
    fn preview_window_sits_right_of_a_left_hand_panel() {
        let (x, _, width, _) =
            preview_window_placement((0.0, 0.0, 2560.0, 1400.0), (200.0, 500.0), (1280.0, 860.0));
        assert_eq!((x, width), (700.0, 1280.0), "패널 오른쪽 끝에 붙는다");
    }

    /// 회귀: `center()`로 열던 시절 프리뷰 창이 조작면(URL 입력줄·선택 버튼) 위에 겹쳤다.
    /// 자리가 좁아도 폭을 줄여 패널 밖에 머문다.
    #[test]
    fn preview_window_shrinks_instead_of_covering_the_toolbar() {
        let (x, _, width, _) =
            preview_window_placement((0.0, 0.0, 1440.0, 900.0), (740.0, 700.0), (1280.0, 860.0));
        assert_eq!((x, width), (0.0, 740.0), "남은 폭에 맞춰 줄어든다");
        assert!(x + width <= 740.0);
    }

    /// 양옆이 모두 최소 폭보다 좁으면 겹침을 감수한다 — 200px짜리 프리뷰는 프리뷰가 아니다.
    /// 그래도 작업영역 밖으로는 나가지 않는다.
    #[test]
    fn preview_window_keeps_a_usable_width_on_a_cramped_screen() {
        let (x, _, width, _) =
            preview_window_placement((0.0, 0.0, 1000.0, 800.0), (200.0, 800.0), (1280.0, 860.0));
        assert_eq!(width, MIN_PREVIEW_WINDOW_WIDTH);
        assert_eq!(
            x, 0.0,
            "왼쪽 공간이 넓은 쪽 — 작업영역 밖으로 밀리지 않는다"
        );
    }

    #[test]
    fn resized_window_uses_the_latest_toolbar_height() {
        let toolbar_height = Mutex::new(96.0);
        *toolbar_height.lock().unwrap() = 128.0;
        assert_eq!(window_toolbar_height(&toolbar_height), 128.0);
    }

    /// 작업영역보다 큰 선호 크기는 작업영역에 갇힌다(작은 노트북 화면).
    #[test]
    fn preview_window_never_exceeds_the_work_area() {
        let (x, y, width, height) =
            preview_window_placement((0.0, 25.0, 1440.0, 775.0), (1040.0, 400.0), (1280.0, 860.0));
        assert_eq!(height, 775.0, "높이는 작업영역까지");
        assert_eq!(y, 25.0);
        assert!(x >= 0.0 && x + width <= 1440.0);
    }
}
