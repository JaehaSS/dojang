use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tauri::{Manager, WebviewUrl};

const TOOLBAR_PROBE_WINDOW_LABEL: &str = "preview-window-probe";
const TOOLBAR_PROBE_WEBVIEW_LABEL: &str = "previewbar-probe";
const PAGE_PROBE_WEBVIEW_LABEL: &str = "designmode-probe";
const TOOLBAR_PROBE_RESULT_PATH: &str = "/toolbar-result";

#[derive(Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolbarProbeEvidence {
    toolbar_url: Option<String>,
    page_url: Option<String>,
    toolbar_label: String,
    page_label: String,
    child_labels: Vec<String>,
    root_webview: bool,
    standalone_webview_windows: usize,
    toolbar_app_command_denied: Option<bool>,
    #[serde(skip)]
    native_ready: bool,
    #[serde(skip)]
    emitted: bool,
}

pub fn start_toolbar_probe(app: &tauri::App, url: &str) -> Result<(), String> {
    let page_url = crate::preview_bridge::validate_preview_probe_url(url)?;
    let evidence = Arc::new(Mutex::new(ToolbarProbeEvidence {
        toolbar_label: TOOLBAR_PROBE_WEBVIEW_LABEL.into(),
        page_label: PAGE_PROBE_WEBVIEW_LABEL.into(),
        ..Default::default()
    }));
    let window = tauri::window::WindowBuilder::new(app, TOOLBAR_PROBE_WINDOW_LABEL)
        .title("Dojang toolbar probe")
        .inner_size(720.0, 520.0)
        .build()
        .map_err(|error| error.to_string())?;
    let mut result_url = page_url.clone();
    result_url.set_path(TOOLBAR_PROBE_RESULT_PATH);
    result_url.set_query(None);
    let toolbar_evidence = evidence.clone();
    let toolbar_app = app.handle().clone();
    let toolbar = window
        .add_child(
            tauri::webview::WebviewBuilder::new(
                TOOLBAR_PROBE_WEBVIEW_LABEL,
                WebviewUrl::App(PathBuf::from(
                    "index.html?window=preview-toolbar&task=-4242",
                )),
            )
            .initialization_script(toolbar_probe_result_script(&result_url)?)
            .on_navigation({
                let toolbar_evidence = toolbar_evidence.clone();
                move |url| record_toolbar_probe_result(url, &toolbar_evidence, &toolbar_app)
            })
            .on_page_load(move |webview, payload| {
                record_toolbar_probe(&webview, payload, &toolbar_evidence, true);
            }),
            tauri::LogicalPosition::new(0.0, 0.0),
            tauri::LogicalSize::new(720.0, 120.0),
        )
        .map_err(|error| error.to_string())?;
    let page_evidence = evidence.clone();
    if let Err(error) = window.add_child(
        tauri::webview::WebviewBuilder::new(
            PAGE_PROBE_WEBVIEW_LABEL,
            WebviewUrl::External(page_url),
        )
        .on_page_load(move |webview, payload| {
            record_toolbar_probe(&webview, payload, &page_evidence, false);
        }),
        tauri::LogicalPosition::new(0.0, 120.0),
        tauri::LogicalSize::new(720.0, 400.0),
    ) {
        let _ = toolbar.close();
        let _ = window.destroy();
        return Err(error.to_string());
    }
    let mut probe = evidence.lock().unwrap_or_else(|error| error.into_inner());
    probe.child_labels = window
        .webviews()
        .into_iter()
        .map(|webview| webview.label().into())
        .collect();
    probe.root_webview = app.get_webview_window(TOOLBAR_PROBE_WINDOW_LABEL).is_some();
    probe.standalone_webview_windows = app.webview_windows().len();
    probe.native_ready = true;
    drop(probe);
    emit_toolbar_probe(app.handle(), &evidence);
    Ok(())
}

fn record_toolbar_probe(
    webview: &tauri::Webview,
    payload: tauri::webview::PageLoadPayload<'_>,
    evidence: &Arc<Mutex<ToolbarProbeEvidence>>,
    toolbar: bool,
) {
    if payload.event() != tauri::webview::PageLoadEvent::Finished {
        return;
    }
    let mut probe = evidence.lock().unwrap_or_else(|error| error.into_inner());
    let destination = if toolbar {
        &mut probe.toolbar_url
    } else {
        &mut probe.page_url
    };
    let run_denial_probe = toolbar && destination.is_none();
    *destination = Some(payload.url().to_string());
    drop(probe);
    if run_denial_probe {
        let _ = webview.eval(toolbar_probe_denial_script());
    }
    emit_toolbar_probe(webview.app_handle(), evidence);
}

fn toolbar_probe_result_script(result_url: &tauri::Url) -> Result<String, String> {
    let result_url =
        serde_json::to_string(result_url.as_str()).map_err(|error| error.to_string())?;
    Ok(format!(
        "window.__praxisToolbarProbeResultUrl={result_url};"
    ))
}

fn toolbar_probe_denial_script() -> &'static str {
    "(async()=>{let d=false;try{await window.__TAURI_INTERNALS__.invoke('designmode_close',{id:-4242});}catch(e){d=String(e).toLowerCase().includes('not allowed');}location.href=window.__praxisToolbarProbeResultUrl+'?denied='+d;})()"
}

fn record_toolbar_probe_result(
    url: &tauri::Url,
    evidence: &Arc<Mutex<ToolbarProbeEvidence>>,
    app: &tauri::AppHandle,
) -> bool {
    if url.path() != TOOLBAR_PROBE_RESULT_PATH {
        return true;
    }
    let denied = url
        .query_pairs()
        .any(|(name, value)| name == "denied" && value == "true");
    evidence
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .toolbar_app_command_denied = Some(denied);
    emit_toolbar_probe(app, evidence);
    false
}

fn emit_toolbar_probe(app: &tauri::AppHandle, evidence: &Arc<Mutex<ToolbarProbeEvidence>>) {
    let mut evidence = evidence.lock().unwrap_or_else(|error| error.into_inner());
    if !evidence.native_ready
        || evidence.toolbar_url.is_none()
        || evidence.page_url.is_none()
        || evidence.toolbar_app_command_denied.is_none()
        || evidence.emitted
    {
        return;
    }
    let output = serde_json::to_string(&*evidence).unwrap_or_else(|_| "{}".into());
    evidence.emitted = true;
    drop(evidence);
    println!("{output}");
    app.exit(0);
}
