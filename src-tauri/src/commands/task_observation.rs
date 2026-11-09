use super::{pool_of, require_main_notification_window, AppState};
use tauri::{State, WebviewWindow};

#[tauri::command]
pub async fn task_observation_get(
    state: State<'_, AppState>,
    window: WebviewWindow,
    id: i64,
) -> Result<crate::task_observation::Observation, String> {
    require_main_notification_window(&window)?;
    let live = live_ownership(&state, id);
    crate::task_observation::snapshot(&pool_of(&state)?, id, live, crate::now()).await
}

fn live_ownership(state: &AppState, id: i64) -> bool {
    let terminal_live = state
        .tasks
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .and_then(|task| task.session.as_ref())
        .is_some_and(|session| {
            crate::verify::process_group_alive_checked(session.pid()).ok() == Some(true)
        });
    terminal_live
        || state
            .convo_active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&id)
        || crate::convo::app_server::execution(id).is_some()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn terminal_ownership_tracks_real_pty_exit() {
        let state = AppState::default();
        let (session, events) =
            crate::pty::PtySession::spawn("/bin/sh", &["-c", "read line"], None, 80, 24).unwrap();
        let path = crate::testtmp::dir();
        state.tasks.lock().unwrap().insert(
            987654,
            super::super::ActiveTask {
                worktree: crate::worktree::Worktree {
                    repo: path.clone(),
                    path,
                    branch: "fixture".into(),
                    base: "main".into(),
                    base_revision: None,
                },
                session: Some(session),
                preview_mcp: None,
            },
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !live_ownership(&state, 987654) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(live_ownership(&state, 987654));
        state
            .tasks
            .lock()
            .unwrap()
            .get(&987654)
            .unwrap()
            .session
            .as_ref()
            .unwrap()
            .write(b"done\n")
            .unwrap();
        while live_ownership(&state, 987654) && std::time::Instant::now() < deadline {
            let _ = events.recv_timeout(std::time::Duration::from_millis(20));
        }
        assert!(!live_ownership(&state, 987654));
    }
}
