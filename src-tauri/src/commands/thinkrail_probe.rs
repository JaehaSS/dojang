//! Explicit debug-only isolated real-WebView probe. Never initializes the user's data directory.
use super::AppState;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Duration,
};
use tauri::{Manager, WebviewWindow};
struct Probe {
    nonce: String,
    agent: String,
    ids: Vec<i64>,
}
fn probe() -> &'static Mutex<Option<Probe>> {
    static P: OnceLock<Mutex<Option<Probe>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(None))
}
pub fn enabled() -> bool {
    cfg!(debug_assertions) && std::env::var_os("DOJANG_NATIVE_PROBE_ROOT").is_some()
}
pub fn start(app: &tauri::App) -> Result<(), String> {
    let root =
        PathBuf::from(std::env::var_os("DOJANG_NATIVE_PROBE_ROOT").ok_or("probe root missing")?)
            .canonicalize()
            .map_err(|e| e.to_string())?;
    let nonce =
        std::fs::read_to_string(root.join("dojang-probe-owned")).map_err(|e| e.to_string())?;
    if nonce.len() < 16 || nonce.len() > 128 || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid probe marker".into());
    }
    let repo = root.join("repo");
    if !repo.join(".git").is_dir() {
        return Err("probe git fixture missing".into());
    }
    let agent = std::env::var("DOJANG_NATIVE_AGENT").unwrap_or_else(|_| "claude".into());
    if !matches!(agent.as_str(), "claude" | "codex") {
        return Err("unsupported probe agent".into());
    }
    let handle = app.handle().clone();
    let ids = tauri::async_runtime::block_on(async {
        let pool = crate::db::init_pool(root.join("probe.sqlite").to_str().ok_or("probe path")?)
            .await
            .map_err(|e| e.to_string())?;
        crate::memory::migrate(&pool)
            .await
            .map_err(|e| e.to_string())?;
        crate::knowledge::migrate(&pool)
            .await
            .map_err(|e| e.to_string())?;
        crate::knowledge::vault::migrate(&pool)
            .await
            .map_err(|e| e.to_string())?;
        crate::github::migrate(&pool)
            .await
            .map_err(|e| e.to_string())?;
        let path = repo.to_str().ok_or("probe repo path")?;
        let missing = root.join("missing");
        let first = crate::db::insert_task(
            &pool,
            path,
            "native-probe",
            "main",
            missing.to_str().ok_or("missing path")?,
            "Native archive record",
            Some("claude"),
            None,
            "conversation",
            1,
        )
        .await
        .map_err(|e| e.to_string())?;
        crate::db::update_state(&pool, first, crate::db::state::AWAITING_REVIEW, 2)
            .await
            .map_err(|e| e.to_string())?;
        crate::db::append_convo_event(
            &pool,
            first,
            r#"{"kind":"text","text":"Native preserved answer"}"#,
            2,
        )
        .await
        .map_err(|e| e.to_string())?;
        let second = crate::db::insert_task(
            &pool,
            path,
            "native-probe",
            "main",
            path,
            "Native vendor record",
            Some(agent.as_str()),
            None,
            "conversation",
            3,
        )
        .await
        .map_err(|e| e.to_string())?;
        crate::db::update_state(&pool, second, crate::db::state::AWAITING_REVIEW, 4)
            .await
            .map_err(|e| e.to_string())?;
        *handle.state::<AppState>().pool.lock().unwrap() = Some(pool);
        Ok::<_, String>(vec![first, second])
    })?;
    *probe().lock().map_err(|e| e.to_string())? = Some(Probe { nonce, agent, ids });
    if let Some(main) = handle.get_webview_window("main") {
        page_loaded(&main);
    }
    let app = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(100)).await;
        finish_inner(&app, json!({"error":"native probe deadline"})).await;
    });
    Ok(())
}
pub fn page_loaded(window: &WebviewWindow) {
    if !enabled() || window.label() != "main" {
        return;
    }
    let guard = probe().lock().unwrap_or_else(|e| e.into_inner());
    let Some(p) = guard.as_ref() else {
        return;
    };
    let cfg = json!({"nonce":p.nonce,"agent":p.agent,"archiveId":p.ids[0],"vendorId":p.ids[1],"liveVendor":std::env::var("DOJANG_NATIVE_LIVE_VENDOR").as_deref()==Ok("1")});
    let script = format!(
        "window.__DOJANG_NATIVE_PROBE__={cfg};\n{}",
        include_str!("../../../scripts/tests/thinkrail-native-driver.js")
    );
    if let Err(e) = window.eval(&script) {
        eprintln!("native probe eval: {e}");
    }
}
#[tauri::command]
pub async fn thinkrail_probe_finish(
    app: tauri::AppHandle,
    window: WebviewWindow,
    nonce: String,
    evidence: Value,
) -> Result<(), String> {
    if !enabled() || window.label() != "main" {
        return Err("probe disabled".into());
    }
    let matches = probe()
        .lock()
        .map_err(|e| e.to_string())?
        .as_ref()
        .is_some_and(|p| p.nonce == nonce);
    if !matches {
        return Err("invalid probe nonce".into());
    }
    finish_inner(&app, evidence).await;
    Ok(())
}
async fn finish_inner(app: &tauri::AppHandle, evidence: Value) {
    let ids = probe()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .map(|p| p.ids)
        .unwrap_or_default();
    if ids.is_empty() {
        return;
    }
    let state = app.state::<AppState>();
    for id in ids {
        let active = state
            .convo_active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&id);
        if active {
            let _ = super::cancel_task_inner(&state, id).await;
        }
    }
    let still_active = !state
        .convo_active
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_empty();
    let success = evidence.get("error").is_none() && !still_active;
    println!(
        "{}",
        json!({"probe":"thinkrail-native","passed":success,"active_after_cleanup":still_active,"evidence":evidence})
    );
    app.exit(if success { 0 } else { 1 });
}
