use std::path::PathBuf;

use tauri::{AppHandle, Manager, State, Webview};

use crate::design_canvas::{
    apply::{self, DesignCanvasEditRequest, DesignCanvasEditResult},
    DesignCanvasServerStatus,
};

use super::{pool_of, AppState};

fn require_main(webview: &Webview) -> Result<(), String> {
    if webview.label() == "main" && webview.window().label() == "main" {
        Ok(())
    } else {
        Err("Design Canvas는 메인 창에서만 사용할 수 있습니다".into())
    }
}

async fn task_of(state: &AppState, id: i64) -> Result<crate::db::Task, String> {
    let task = crate::db::get_task(&pool_of(state)?, id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
    Ok(task)
}

async fn worktree_of(state: &AppState, id: i64) -> Result<PathBuf, String> {
    let task = task_of(state, id).await?;
    if crate::db::state::is_terminal(&task.state) {
        return Err("종결된 작업에서는 Design Canvas를 시작할 수 없습니다".into());
    }
    let worktree = PathBuf::from(task.worktree_path);
    if !worktree.is_dir() {
        return Err(crate::worktree::missing_worktree_error(
            &worktree.display().to_string(),
        ));
    }
    Ok(worktree)
}

fn task_is_mutable(task: &crate::db::Task) -> bool {
    !crate::db::state::is_terminal(&task.state) && task.state != crate::db::state::FINALIZING
}

fn respond_edit(
    state: &AppState,
    id: i64,
    surface: &str,
    result: DesignCanvasEditResult,
) -> DesignCanvasEditResult {
    let _ = crate::commands::designmode::dispatch_canvas_edit_result(state, id, surface, &result);
    result
}

fn owner_token(owner: String) -> Result<String, String> {
    let owner = owner.trim();
    if owner.is_empty() {
        return Err("Design Canvas 소유자 토큰이 비어 있습니다".into());
    }
    Ok(owner.to_string())
}

#[tauri::command]
pub async fn design_canvas_start(
    webview: Webview,
    state: State<'_, AppState>,
    id: i64,
    owner: String,
) -> Result<DesignCanvasServerStatus, String> {
    require_main(&webview)?;
    let owner = owner_token(owner)?;
    let worktree = worktree_of(&state, id).await?;
    let servers = state.design_canvas.clone();
    tokio::task::spawn_blocking(move || servers.start(id, owner, worktree))
        .await
        .map_err(|error| format!("Design Canvas 시작 작업이 중단되었습니다: {error}"))
}

#[tauri::command]
pub async fn design_canvas_status(
    webview: Webview,
    state: State<'_, AppState>,
    id: i64,
    owner: String,
) -> Result<DesignCanvasServerStatus, String> {
    require_main(&webview)?;
    let owner = owner_token(owner)?;
    let task = task_of(&state, id).await?;
    let terminal = crate::db::state::is_terminal(&task.state);
    let servers = state.design_canvas.clone();
    tokio::task::spawn_blocking(move || {
        if terminal {
            servers.stop_task(id);
        }
        servers.status_owned(id, &owner)
    })
    .await
    .map_err(|error| format!("Design Canvas 상태 확인이 중단되었습니다: {error}"))
}

#[tauri::command]
pub async fn design_canvas_stop(
    webview: Webview,
    state: State<'_, AppState>,
    id: i64,
    owner: String,
) -> Result<(), String> {
    require_main(&webview)?;
    let owner = owner_token(owner)?;
    let servers = state.design_canvas.clone();
    tokio::task::spawn_blocking(move || servers.stop_owned(id, &owner))
        .await
        .map_err(|error| format!("Design Canvas 종료 작업이 중단되었습니다: {error}"))
}

/// Main Design tab만 Canvas의 custom-scheme request를 Tauri command로 전달한다. Canvas
/// 개발 페이지 자체는 app origin이 아니므로 이 명령을 직접 호출할 수 없다.
#[tauri::command]
pub async fn design_canvas_edit(
    webview: Webview,
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    surface: String,
    request: DesignCanvasEditRequest,
) -> Result<DesignCanvasEditResult, String> {
    require_main(&webview)?;
    let request_id = request.request_id.clone();
    if !crate::commands::designmode::canvas_surface_is_current(&state, id, &surface) {
        return Ok(respond_edit(
            &state,
            id,
            &surface,
            DesignCanvasEditResult::rejected(
                request_id,
                "stale_surface",
                "현재 Canvas 표면이 아닙니다",
            ),
        ));
    }
    let pool = match pool_of(&state) {
        Ok(pool) => pool,
        Err(error) => {
            return Ok(respond_edit(
                &state,
                id,
                &surface,
                DesignCanvasEditResult::rejected(request_id, "write_failed", error),
            ))
        }
    };
    let task = match task_of(&state, id).await {
        Ok(task) => task,
        Err(error) => {
            return Ok(respond_edit(
                &state,
                id,
                &surface,
                DesignCanvasEditResult::rejected(request_id, "terminal_task", error),
            ))
        }
    };
    if !task_is_mutable(&task) {
        return Ok(respond_edit(
            &state,
            id,
            &surface,
            DesignCanvasEditResult::rejected(
                request_id,
                "terminal_task",
                "종결 중이거나 종결된 작업은 편집할 수 없습니다",
            ),
        ));
    }
    let worktree = PathBuf::from(&task.worktree_path);
    if !worktree.is_dir() {
        return Ok(respond_edit(
            &state,
            id,
            &surface,
            DesignCanvasEditResult::rejected(
                request_id,
                "path_outside_worktree",
                "작업 폴더가 없습니다",
            ),
        ));
    }
    let expected_root = worktree.clone();
    let guard_app = app.clone();
    let guard_surface = surface.clone();
    let result = match tokio::task::spawn_blocking(move || {
        apply::execute_guarded(id, &worktree, request, move || {
            let guard_state = guard_app.state::<AppState>();
            if !crate::commands::designmode::canvas_surface_is_current(
                &guard_state,
                id,
                &guard_surface,
            ) {
                return Err(apply::EditError::new(
                    "stale_surface",
                    "Canvas 표면이 교체되었습니다",
                ));
            }
            let current = tauri::async_runtime::block_on(crate::db::get_task(&pool, id))
                .map_err(|error| apply::EditError::new("write_failed", error.to_string()))?
                .ok_or_else(|| apply::EditError::new("terminal_task", "작업을 찾을 수 없습니다"))?;
            if !task_is_mutable(&current) || PathBuf::from(current.worktree_path) != expected_root {
                return Err(apply::EditError::new(
                    "terminal_task",
                    "작업의 worktree가 변경되었거나 종결되었습니다",
                ));
            }
            Ok(())
        })
    })
    .await
    {
        Ok(result) => result,
        Err(error) => {
            return Ok(respond_edit(
                &state,
                id,
                &surface,
                DesignCanvasEditResult::rejected(
                    request_id,
                    "write_failed",
                    format!("Design Canvas 편집 작업이 중단되었습니다: {error}"),
                ),
            ))
        }
    };
    // 성공한 HMR 응답도 같은 표면으로만 돌아간다. 늦은 이전 탭은 새 탭의 선택을 덮지 못한다.
    if !crate::commands::designmode::canvas_surface_is_current(&state, id, &surface) {
        return Ok(respond_edit(
            &state,
            id,
            &surface,
            DesignCanvasEditResult::rejected(
                result.request_id,
                "stale_surface",
                "Canvas 표면이 교체되었습니다",
            ),
        ));
    }
    Ok(respond_edit(&state, id, &surface, result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_token_rejects_empty_values() {
        assert!(owner_token(" \n".into()).is_err());
        assert_eq!(owner_token(" effect-1 ".into()).unwrap(), "effect-1");
    }
}
