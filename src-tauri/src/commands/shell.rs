//! 셸 슬롯·수확(reap) 커맨드.
//!
//! `commands.rs`에서 갈라져 나왔다 — 모든 기능이 한 파일 끝에 줄을 붙이는
//! 구조가 병행 worktree의 머지 충돌을 만들었다.

use std::path::Path;

use base64::{engine::general_purpose::STANDARD, Engine};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::db::{self};
use crate::pty::{OutputCoalescer, PtyEvent, PtySession};
use crate::shellreap::{self, ShellSlot};

use super::{AppState, ExitPayload, OutputPayload, default_shell, pool_of};

/// 워크스페이스 셸 열기 — 작업 워크트리에서 인터랙티브 셸을 띄운다(작업당 1개).
/// 이미 열려 있으면 재사용하고 `true`를 반환한다(프론트는 Ctrl-L로 프롬프트만 재표시).
/// 출력/종료는 작업 PTY(`pty://`)와 분리된 `shell://output/{id}`/`shell://exit/{id}`로 흐른다.
#[tauri::command]
pub async fn shell_open(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    cols: u16,
    rows: u16,
) -> Result<bool, String> {
    if let Some(slot) = state.shells.lock().unwrap().get_mut(&id) {
        slot.attach();
        return Ok(true);
    }
    let pool = pool_of(&state)?;
    let task = db::get_task(&pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    if !Path::new(&task.worktree_path).is_dir() {
        return Err("워크트리가 없습니다 — 종료된 작업일 수 있습니다".into());
    }
    let spec = default_shell();
    let argv: Vec<&str> = spec.args.iter().map(|s| s.as_str()).collect();
    let (session, rx) = PtySession::spawn(&spec.cmd, &argv, Some(&task.worktree_path), cols, rows)
        .map_err(|e| format!("셸 생성 실패: {e}"))?;
    // 종료 이벤트가 늦게 도착해 **다음** 셸을 지우는 것을 막는다 — 유휴 회수가 생기면서
    // 죽자마자 다시 열리는 경우가 실제로 생긴다.
    let session_pid = session.pid();
    {
        // 삽입 직전 재확인(동시 open 레이스) — 진 쪽의 새 세션은 정리하고 기존 것을 쓴다.
        let mut shells = state.shells.lock().unwrap();
        if let Some(slot) = shells.get_mut(&id) {
            session.terminate();
            slot.attach();
            return Ok(true);
        }
        let mut slot = ShellSlot::new(session);
        slot.attach();
        shells.insert(id, slot);
    }
    std::thread::spawn(move || {
        let output_event = format!("shell://output/{id}");
        let exit_event = format!("shell://exit/{id}");
        let mut coalescer = OutputCoalescer::new();
        while let Ok(ev) = rx.recv() {
            // 청크당 emit 대신 창당 emit — 합치는 도중 종료를 만나면 그대로 아래 정리로 넘긴다.
            let code = match ev {
                PtyEvent::Output(b) => {
                    let (bytes, pending_exit) = coalescer.gather(&rx, b);
                    let _ = app.emit(
                        &output_event,
                        OutputPayload {
                            id,
                            data: STANDARD.encode(&bytes),
                        },
                    );
                    match pending_exit {
                        Some(code) => code,
                        None => continue,
                    }
                }
                PtyEvent::Exit(code) => code,
            };
            // 죽은 셸이 맵에 남지 않도록 제거 — 다음 shell_open이 새로 띄운다.
            // 그 자리에 이미 다른 셸이 들어와 있으면 건드리지 않는다(늦은 이벤트).
            let state = app.state::<AppState>();
            let mut shells = state.shells.lock().unwrap_or_else(|e| e.into_inner());
            if shells.get(&id).map(|s| s.session.pid()) == Some(session_pid) {
                shells.remove(&id);
            }
            drop(shells);
            let _ = app.emit(&exit_event, ExitPayload { id, code });
            break;
        }
    });
    Ok(false)
}

/// 워크스페이스 셸 스크롤백 replay(base64) — `shell_open` 직후, 라이브 스트림
/// (`shell://output/{id}`) 구독보다 먼저 호출한다. 열린 셸이 없으면 빈 문자열(에러 아님).
#[tauri::command]
pub async fn shell_replay(state: State<'_, AppState>, id: i64) -> Result<String, String> {
    // 유휴로 회수된 자리면 한 번만 그 사실을 알린다(ADR 0163 결정 4).
    let reaped = state
        .reaped_shells
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&id);
    // base64 인코딩은 락 밖에서 — 입력 경로가 스크롤백 크기만큼 락을 잡지 않게 한다.
    let scrollback = {
        let shells = state.shells.lock().unwrap_or_else(|e| e.into_inner());
        shells.get(&id).map(|slot| slot.session.scrollback_snapshot())
    };
    if !reaped && scrollback.is_none() {
        return Ok(String::new());
    }
    let mut out = Vec::new();
    if reaped {
        out.extend_from_slice(shellreap::REAPED_NOTICE.as_bytes());
    }
    out.extend(scrollback.unwrap_or_default());
    Ok(STANDARD.encode(out))
}

/// 워크스페이스 셸에서 화면이 떨어졌음을 알린다 — 컴포넌트 unmount에서 부른다.
///
/// 셸은 그대로 살아 있다(스크롤백을 지키려면 그래야 한다). 다만 아무도 보고 있지 않다는
/// 사실이 유휴 회수의 첫 조건이므로, 이 신호가 없으면 셸은 영원히 회수되지 않는다.
#[tauri::command]
pub fn shell_detach(state: State<AppState>, id: i64) -> Result<(), String> {
    if let Some(slot) = state.shells.lock().unwrap().get_mut(&id) {
        slot.detach();
    }
    Ok(())
}

/// 유휴 셸 회수 1회분 — `SCAN_INTERVAL`마다 백그라운드에서 부른다.
pub fn reap_idle_shells(app: &AppHandle) {
    let state = app.state::<AppState>();
    let reaped = {
        let mut shells = state.shells.lock().unwrap();
        shellreap::reap(&mut shells, std::time::Instant::now())
    };
    if reaped.is_empty() {
        return;
    }
    let mut marks = state.reaped_shells.lock().unwrap();
    for id in reaped {
        marks.insert(id);
    }
}

/// 워크스페이스 셸 stdin 전달.
#[tauri::command]
pub async fn shell_write(state: State<'_, AppState>, id: i64, data: String) -> Result<(), String> {
    let shells = state.shells.lock().unwrap_or_else(|e| e.into_inner());
    let slot = shells.get(&id).ok_or("열린 셸이 없습니다")?;
    slot.session.write(data.as_bytes()).map_err(|e| e.to_string())
}

/// 워크스페이스 셸 리사이즈.
#[tauri::command]
pub async fn shell_resize(
    state: State<'_, AppState>,
    id: i64,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let shells = state.shells.lock().unwrap_or_else(|e| e.into_inner());
    let slot = shells.get(&id).ok_or("열린 셸이 없습니다")?;
    slot.session.resize(cols, rows).map_err(|e| e.to_string())
}

// ── 에이전트 CLI 액션 셸 ─────────────────────────────────────────────
//
// 인증·업데이트·자유 셸을 앱 안 PTY에서 끝낸다. `shell_*`은 task id로 키잉되고 워크트리를
// 필요로 하지만 이쪽은 작업과 무관하므로, 맵도 이벤트 채널도 분리한다.

