use std::path::PathBuf;
use std::time::Duration;

use tauri::Manager;

use super::commands::write_if_current;
use super::open_path::open_verified;
use super::registry::{
    canonical_root, OpenAction, ProjectEditorInfo, ProjectLaunch, ProjectRegistry,
};
use super::shell::ProjectShell;
use super::ProjectEditorState;

fn root(name: &str) -> PathBuf {
    let root = crate::testtmp::dir().join(name);
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn registry_normalizes_roots_and_keeps_projects_isolated() {
    let first = root("project-editor-registry-first");
    let second = root("project-editor-registry-second");
    let first = canonical_root(first.join(".").to_str().unwrap()).unwrap();
    let second = canonical_root(second.to_str().unwrap()).unwrap();
    let mut registry = ProjectRegistry::default();
    let (first_label, first_opening) = create(&mut registry, first.clone());
    let (second_label, _) = create(&mut registry, second.clone());
    assert_ne!(first_label, second_label);
    assert_eq!(registry.root_for_label(&first_label), Some(first));
    assert_eq!(registry.root_for_label(&second_label), Some(second));
    first_opening.complete(Ok(ProjectEditorInfo {
        root: "first".into(),
        label: first_label,
        launch: false,
    }));
}

#[tokio::test]
async fn opening_retains_completed_result_for_all_waiters() {
    let root = root("project-editor-opening-waiters");
    let mut registry = ProjectRegistry::default();
    let (label, opening) = create(&mut registry, root);
    let info = ProjectEditorInfo {
        root: "root".into(),
        label,
        launch: false,
    };
    opening.complete(Ok(info.clone()));
    let (first, second) = tokio::join!(opening.wait(), opening.wait());
    assert_eq!(first.unwrap(), info);
    assert_eq!(second.unwrap(), info);
}

#[tokio::test]
async fn opening_cleanup_returns_the_same_error_to_waiters() {
    let root = root("project-editor-opening-cleanup");
    let mut registry = ProjectRegistry::default();
    let (label, opening) = create(&mut registry, root);
    let removed = registry.remove_label(&label).unwrap();
    removed.complete(Err("프로젝트 창이 닫혔습니다".into()));
    assert_eq!(
        opening.wait().await.unwrap_err(),
        "프로젝트 창이 닫혔습니다"
    );
}

#[test]
fn write_requires_current_text_and_rejects_escaped_paths() {
    let root = root("project-editor-write-cas");
    let file = root.join("note.txt");
    std::fs::write(&file, "before").unwrap();
    write_if_current(&root, "note.txt", "after", "before").unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "after");
    assert!(write_if_current(&root, "note.txt", "lost", "before").is_err());
    assert!(write_if_current(&root, "../outside.txt", "lost", "").is_err());
    assert!(write_if_current(&root, "missing.txt", "new", "").is_err());
}

#[cfg(unix)]
#[test]
fn shell_tokens_reject_stale_calls_and_cleanup_the_child() {
    let root = root("project-editor-shell-cleanup");
    let (shell, events) = ProjectShell::spawn(root.to_str().unwrap(), 7, 80, 24, None).unwrap();
    assert!(shell.lock().unwrap().snapshot(6).is_err());
    shell.lock().unwrap().close(7).unwrap();
    assert!(shell.lock().unwrap().exited());
    let exit = std::iter::from_fn(|| events.recv_timeout(Duration::from_secs(2)).ok())
        .find(|event| matches!(event, crate::pty::PtyEvent::Exit(_)));
    assert!(exit.is_some(), "project shell must exit after cleanup");
}

#[cfg(unix)]
#[test]
fn pty_starts_in_the_project_root() {
    let root = root("project-editor-pty-cwd");
    let (pty, events) = crate::pty::PtySession::spawn(
        "/bin/sh",
        &["-c", "pwd"],
        Some(root.to_str().unwrap()),
        80,
        24,
    )
    .unwrap();
    let output = std::iter::from_fn(|| events.recv_timeout(Duration::from_secs(2)).ok())
        .filter_map(|event| match event {
            crate::pty::PtyEvent::Output(bytes) => {
                Some(String::from_utf8_lossy(&bytes).into_owned())
            }
            crate::pty::PtyEvent::Exit(_) => None,
        })
        .collect::<String>();
    pty.terminate();
    assert!(output.contains(root.to_str().unwrap()), "{output:?}");
}

#[cfg(unix)]
#[test]
fn launch_replaces_the_default_shell_of_the_first_pty() {
    let root = root("project-editor-shell-launch");
    let launch = ProjectLaunch {
        bin: "/bin/echo".into(),
        args: vec!["hello".into()],
    };
    let (shell, events) =
        ProjectShell::spawn(root.to_str().unwrap(), 3, 80, 24, Some(&launch)).unwrap();
    let mut output = String::new();
    let mut code = None;
    while let Ok(event) = events.recv_timeout(Duration::from_secs(2)) {
        match event {
            crate::pty::PtyEvent::Output(bytes) => {
                output.push_str(&String::from_utf8_lossy(&bytes))
            }
            crate::pty::PtyEvent::Exit(value) => code = Some(value),
        }
    }
    shell.lock().unwrap().terminate();
    assert!(output.contains("hello"), "{output:?}");
    assert_eq!(code, Some(0));
}

#[test]
fn mock_ipc_uses_the_injected_caller_for_project_access() {
    let app = tauri::test::mock_builder()
        .manage(ProjectEditorState::default())
        .invoke_handler(tauri::generate_handler![
            super::project_editor_open,
            super::editor_project_prepare,
            super::editor_project_activate,
            super::project_editor_info
        ])
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let request = |cmd: &str, body: serde_json::Value| tauri::webview::InvokeRequest {
        cmd: cmd.into(),
        callback: tauri::ipc::CallbackFn(0),
        error: tauri::ipc::CallbackFn(1),
        url: "tauri://localhost".parse().unwrap(),
        body: body.into(),
        headers: Default::default(),
        invoke_key: tauri::test::INVOKE_KEY.into(),
    };
    let project = tauri::WebviewWindowBuilder::new(&app, "project-editor-evil", Default::default())
        .build()
        .unwrap();
    let open = tauri::test::get_ipc_response(
        &project,
        request("project_editor_open", serde_json::json!({ "root": "/" })),
    );
    assert!(format!("{open:?}").contains("메인 창에서만"));
    let main = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let info =
        tauri::test::get_ipc_response(&main, request("project_editor_info", serde_json::json!({})));
    assert!(format!("{info:?}").contains("등록되지 않은 프로젝트 창"));

    let shared_root = canonical_root(root("project-editor-shared-ipc").to_str().unwrap())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let prepared = tauri::test::get_ipc_response(
        &project,
        request(
            "editor_project_prepare",
            serde_json::json!({ "root": shared_root }),
        ),
    );
    assert!(format!("{prepared:?}").contains("메인 창에서만"));
    let prepared = tauri::test::get_ipc_response(
        &main,
        request(
            "editor_project_prepare",
            serde_json::json!({ "root": shared_root }),
        ),
    );
    let prepared = format!("{prepared:?}");
    assert!(
        prepared.contains("label") && prepared.contains("editor"),
        "{prepared}"
    );
    assert!(app.get_webview_window("editor").is_none());

    let editor = tauri::WebviewWindowBuilder::new(&app, "editor", Default::default())
        .build()
        .unwrap();
    let activate_from_main = tauri::test::get_ipc_response(
        &main,
        request(
            "editor_project_activate",
            serde_json::json!({ "root": shared_root }),
        ),
    );
    assert!(format!("{activate_from_main:?}").contains("공유 에디터 창에서만"));
    let activate = tauri::test::get_ipc_response(
        &editor,
        request(
            "editor_project_activate",
            serde_json::json!({ "root": shared_root }),
        ),
    );
    let active = app.state::<ProjectEditorState>().root_for_window("editor");
    assert_eq!(
        active,
        Ok(canonical_root(&shared_root).unwrap()),
        "{activate:?}"
    );
}

#[test]
fn shared_editor_accepts_only_prepared_roots_and_clears_for_task_sessions() {
    let first = canonical_root(root("project-editor-shared-first").to_str().unwrap()).unwrap();
    let second = canonical_root(root("project-editor-shared-second").to_str().unwrap()).unwrap();
    let state = ProjectEditorState::default();

    assert!(state
        .activate_shared_root(Some(first.to_str().unwrap()))
        .is_err());
    state.prepare_shared_root(first.clone());
    assert!(state
        .activate_shared_root(Some(second.to_str().unwrap()))
        .is_err());
    assert!(state.root_for_window("editor").is_err());
    assert!(state
        .activate_shared_root(Some(first.to_str().unwrap()))
        .unwrap());
    assert_eq!(state.root_for_window("editor").unwrap(), first);
    assert!(!state
        .activate_shared_root(Some(first.to_str().unwrap()))
        .unwrap());
    assert!(state.activate_shared_root(None).unwrap());
    assert!(state.root_for_window("editor").is_err());
}

#[test]
fn shared_editor_rejects_stale_root_scoped_file_access() {
    let first =
        canonical_root(root("project-editor-shared-stale-first").to_str().unwrap()).unwrap();
    let second =
        canonical_root(root("project-editor-shared-stale-second").to_str().unwrap()).unwrap();
    std::fs::write(first.join("same.txt"), "first").unwrap();
    std::fs::write(second.join("same.txt"), "second").unwrap();
    let state = ProjectEditorState::default();
    state.prepare_shared_root(first.clone());
    state.prepare_shared_root(second.clone());
    state
        .activate_shared_root(Some(first.to_str().unwrap()))
        .unwrap();

    state
        .activate_shared_root(Some(second.to_str().unwrap()))
        .unwrap();
    assert!(state
        .root_for_request("editor", Some(first.to_str().unwrap()))
        .is_err());
    assert_eq!(
        std::fs::read_to_string(second.join("same.txt")).unwrap(),
        "second"
    );
    assert_eq!(
        state
            .root_for_request("editor", Some(second.to_str().unwrap()))
            .unwrap(),
        second
    );
}

#[cfg(unix)]
#[test]
fn shared_editor_rejects_a_prepared_path_replaced_by_an_authorized_symlink() {
    let first = canonical_root(
        root("project-editor-shared-symlink-first")
            .to_str()
            .unwrap(),
    )
    .unwrap();
    let second = canonical_root(
        root("project-editor-shared-symlink-second")
            .to_str()
            .unwrap(),
    )
    .unwrap();
    let state = ProjectEditorState::default();
    state.prepare_shared_root(first.clone());
    state.prepare_shared_root(second.clone());
    state
        .activate_shared_root(Some(second.to_str().unwrap()))
        .unwrap();

    std::fs::remove_dir_all(&first).unwrap();
    std::os::unix::fs::symlink(&second, &first).unwrap();
    assert!(state
        .root_for_request("editor", Some(first.to_str().unwrap()))
        .is_err());
    assert!(state
        .activate_shared_root(Some(first.to_str().unwrap()))
        .is_err());
}

#[cfg(unix)]
#[test]
fn shared_root_switch_or_clear_terminates_its_previous_shell() {
    let first =
        canonical_root(root("project-editor-shared-shell-first").to_str().unwrap()).unwrap();
    let second =
        canonical_root(root("project-editor-shared-shell-second").to_str().unwrap()).unwrap();
    let state = ProjectEditorState::default();
    state.prepare_shared_root(first.clone());
    state.prepare_shared_root(second.clone());
    state
        .activate_shared_root(Some(first.to_str().unwrap()))
        .unwrap();
    let (shell, events) = ProjectShell::spawn(first.to_str().unwrap(), 13, 80, 24, None).unwrap();
    assert_eq!(
        super::insert_shell_if_registered(&state, "editor", &first, shell),
        Some(13)
    );
    assert!(!state
        .activate_shared_root(Some(first.to_str().unwrap()))
        .unwrap());
    assert!(state.shells.lock().unwrap().contains_key("editor"));

    state
        .activate_shared_root(Some(second.to_str().unwrap()))
        .unwrap();
    assert!(state.shells.lock().unwrap().is_empty());
    let exit = std::iter::from_fn(|| events.recv_timeout(Duration::from_secs(2)).ok())
        .find(|event| matches!(event, crate::pty::PtyEvent::Exit(_)));
    assert!(
        exit.is_some(),
        "root switch must terminate the old shared shell"
    );

    let (shell, events) = ProjectShell::spawn(second.to_str().unwrap(), 14, 80, 24, None).unwrap();
    assert_eq!(
        super::insert_shell_if_registered(&state, "editor", &second, shell),
        Some(14)
    );
    state.activate_shared_root(None).unwrap();
    assert!(state.shells.lock().unwrap().is_empty());
    let exit = std::iter::from_fn(|| events.recv_timeout(Duration::from_secs(2)).ok())
        .find(|event| matches!(event, crate::pty::PtyEvent::Exit(_)));
    assert!(
        exit.is_some(),
        "clearing the root must terminate the shared shell"
    );
}

#[cfg(unix)]
#[test]
fn destroyed_shared_editor_clears_authorization_and_terminates_its_shell() {
    let root = canonical_root(root("project-editor-shared-destroyed").to_str().unwrap()).unwrap();
    let state = ProjectEditorState::default();
    state.prepare_shared_root(root.clone());
    state
        .activate_shared_root(Some(root.to_str().unwrap()))
        .unwrap();
    let (shell, events) = ProjectShell::spawn(root.to_str().unwrap(), 15, 80, 24, None).unwrap();
    assert_eq!(
        super::insert_shell_if_registered(&state, "editor", &root, shell),
        Some(15)
    );

    super::cleanup_window(&state, "editor");
    assert!(state.root_for_window("editor").is_err());
    assert!(state
        .activate_shared_root(Some(root.to_str().unwrap()))
        .is_err());
    let (late_shell, _) = ProjectShell::spawn(root.to_str().unwrap(), 16, 80, 24, None).unwrap();
    assert_eq!(
        super::insert_shell_if_registered(&state, "editor", &root, late_shell.clone()),
        None
    );
    late_shell.lock().unwrap().terminate();
    let exit = std::iter::from_fn(|| events.recv_timeout(Duration::from_secs(2)).ok())
        .find(|event| matches!(event, crate::pty::PtyEvent::Exit(_)));
    assert!(
        exit.is_some(),
        "destroyed editor must terminate its shared shell"
    );
}

#[test]
fn capability_is_limited_to_project_editor_windows() {
    let capability = include_str!("../../capabilities/project-editor.json");
    assert!(capability.contains("\"windows\": [\"project-editor-*\"]"));
    assert!(capability.contains("\"core:window:allow-destroy\""));
}

#[test]
fn native_open_path_accepts_only_regular_root_files() {
    let root = root("project-editor-open-path");
    let file = root.join("note.txt");
    std::fs::write(&file, "ok").unwrap();
    let mut opened = None;
    open_verified(&root, "note.txt", |path| {
        opened = Some(path.to_path_buf());
        Ok(())
    })
    .unwrap();
    assert_eq!(opened, Some(file.canonicalize().unwrap()));
    assert!(open_verified(&root, "../outside.txt", |_| Ok(())).is_err());
    assert!(open_verified(&root, "", |_| Ok(())).is_err());
}

#[test]
fn main_shell_cleanup_preserves_live_project_registration() {
    let root = canonical_root(root("project-editor-main-close").to_str().unwrap()).unwrap();
    let state = ProjectEditorState::default();
    let (label, _) = create(&mut state.registry.lock().unwrap(), root.clone());
    super::cleanup_all(&state);
    assert_eq!(state.root_for_window(&label).unwrap(), root);
    super::cleanup_window(&state, &label);
    assert!(state.root_for_window(&label).is_err());
}

#[cfg(unix)]
#[test]
fn destroyed_window_removes_its_registration_and_terminates_the_owned_pty() {
    let root = canonical_root(root("project-editor-destroyed").to_str().unwrap()).unwrap();
    let state = ProjectEditorState::default();
    let (label, _) = create(&mut state.registry.lock().unwrap(), root.clone());
    let (shell, events) = ProjectShell::spawn(root.to_str().unwrap(), 11, 80, 24, None).unwrap();
    assert_eq!(
        super::insert_shell_if_registered(&state, &label, &root, shell),
        Some(11)
    );
    super::cleanup_window(&state, &label);
    assert!(state.shells.lock().unwrap().is_empty());
    assert!(state.root_for_window(&label).is_err());
    let exit = std::iter::from_fn(|| events.recv_timeout(Duration::from_secs(2)).ok())
        .find(|event| matches!(event, crate::pty::PtyEvent::Exit(_)));
    assert!(
        exit.is_some(),
        "destroyed window must terminate its owned PTY"
    );
}

fn create(
    registry: &mut ProjectRegistry,
    root: PathBuf,
) -> (String, std::sync::Arc<super::registry::Opening>) {
    match registry.begin_open(root, None) {
        OpenAction::Create { label, opening } => (label, opening),
        _ => panic!("first open must reserve a window"),
    }
}
