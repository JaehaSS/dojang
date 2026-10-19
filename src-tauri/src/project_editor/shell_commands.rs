use tauri::{AppHandle, Runtime, State, WebviewWindow};

use super::registry::display_root;
use super::shell::{forward_events, ProjectShell, ShellOpen, ShellSnapshot};
use super::ProjectEditorState;

#[tauri::command]
pub fn project_editor_shell_open<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, ProjectEditorState>,
    window: WebviewWindow<R>,
    cols: u16,
    rows: u16,
    root: Option<String>,
) -> Result<ShellOpen, String> {
    let root = state.root_for_request(window.label(), root.as_deref())?;
    if let Some(open) = current_shell(&state, window.label()) {
        if state.root_is_current(window.label(), &root) {
            return Ok(open);
        }
    }
    let session = state.next_session();
    let launch = state
        .registry
        .lock()
        .unwrap()
        .launch_for_label(window.label());
    let (shell, events) =
        ProjectShell::spawn(&display_root(&root), session, cols, rows, launch.as_ref())?;
    let Some(installed) =
        super::insert_shell_if_registered(&state, window.label(), &root, shell.clone())
    else {
        shell.lock().unwrap().terminate();
        return Err("프로젝트 창이 닫혔습니다".into());
    };
    if installed != session {
        shell.lock().unwrap().terminate();
        return Ok(ShellOpen {
            session: installed,
            existed: true,
        });
    }
    forward_events(app, window.label().to_string(), shell, events);
    Ok(ShellOpen {
        session,
        existed: false,
    })
}

fn current_shell(state: &ProjectEditorState, label: &str) -> Option<ShellOpen> {
    let shell = state.shells.lock().unwrap().get(label).cloned()?;
    let shell = shell.lock().unwrap();
    (!shell.exited()).then_some(ShellOpen {
        session: shell.session,
        existed: true,
    })
}

#[tauri::command]
pub fn project_editor_shell_snapshot<R: Runtime>(
    state: State<'_, ProjectEditorState>,
    window: WebviewWindow<R>,
    session: u64,
    root: Option<String>,
) -> Result<ShellSnapshot, String> {
    state.root_for_request(window.label(), root.as_deref())?;
    shell_for(&state, window.label())?
        .lock()
        .unwrap()
        .snapshot(session)
}

#[tauri::command]
pub fn project_editor_shell_write<R: Runtime>(
    state: State<'_, ProjectEditorState>,
    window: WebviewWindow<R>,
    session: u64,
    data: String,
    root: Option<String>,
) -> Result<(), String> {
    state.root_for_request(window.label(), root.as_deref())?;
    shell_for(&state, window.label())?
        .lock()
        .unwrap()
        .write(session, &data)
}

#[tauri::command]
pub fn project_editor_shell_resize<R: Runtime>(
    state: State<'_, ProjectEditorState>,
    window: WebviewWindow<R>,
    session: u64,
    cols: u16,
    rows: u16,
    root: Option<String>,
) -> Result<(), String> {
    state.root_for_request(window.label(), root.as_deref())?;
    shell_for(&state, window.label())?
        .lock()
        .unwrap()
        .resize(session, cols, rows)
}

#[tauri::command]
pub fn project_editor_shell_close<R: Runtime>(
    state: State<'_, ProjectEditorState>,
    window: WebviewWindow<R>,
    session: u64,
    root: Option<String>,
) -> Result<(), String> {
    state.root_for_request(window.label(), root.as_deref())?;
    let shell = shell_for(&state, window.label())?;
    let result = shell.lock().unwrap().close(session);
    result
}

fn shell_for(
    state: &ProjectEditorState,
    label: &str,
) -> Result<std::sync::Arc<std::sync::Mutex<ProjectShell>>, String> {
    state
        .shells
        .lock()
        .unwrap()
        .get(label)
        .cloned()
        .ok_or("열린 프로젝트 셸이 없습니다".into())
}
