//! Design Canvas용 Vite 개발 서버의 작업별 수명 주기.
//!
//! 서버는 앱의 프리뷰 웹뷰와 달리 외부 프로세스이므로, 소유자 토큰으로 늦게 도착한
//! React effect cleanup이 새 서버를 죽이지 않게 한다. 잠금은 상태 교체와 `try_wait`에만
//! 쓰고, spawn·포트 대기·프로세스 회수는 모두 잠금 밖에서 수행한다.

use serde::Serialize;
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub mod selection;
pub mod apply;

const RESERVED_APP_PORT: u16 = 1420;
const LOG_TAIL_BYTES: u64 = 8 * 1024;
const TERM_GRACE: Duration = Duration::from_millis(800);
const TERM_POLL: Duration = Duration::from_millis(50);
const STARTUP_DEADLINE: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DesignCanvasServerPhase {
    ServerStarting,
    Ready,
    ServerFailed,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignCanvasServerStatus {
    pub status: DesignCanvasServerPhase,
    pub url: Option<String>,
    pub port: Option<u16>,
    pub error: Option<String>,
    pub log_path: Option<String>,
}

#[derive(Clone)]
pub struct DesignCanvasServers {
    state: Arc<Mutex<CanvasState>>,
    launch_done: Arc<Condvar>,
}

struct CanvasState {
    slots: HashMap<i64, ServerSlot>,
    launches: HashMap<i64, std::collections::HashSet<String>>,
    terminal_tasks: std::collections::HashSet<i64>,
    accepting_starts: bool,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            slots: HashMap::new(),
            launches: HashMap::new(),
            terminal_tasks: Default::default(),
            accepting_starts: true,
        }
    }
}

impl Default for DesignCanvasServers {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(CanvasState::default())),
            launch_done: Arc::new(Condvar::new()),
        }
    }
}

struct LaunchGuard {
    state: Arc<Mutex<CanvasState>>,
    launch_done: Arc<Condvar>,
    task_id: i64,
    owner: String,
}

impl Drop for LaunchGuard {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(owners) = state.launches.get_mut(&self.task_id) {
            owners.remove(&self.owner);
            if owners.is_empty() {
                state.launches.remove(&self.task_id);
            }
        }
        self.launch_done.notify_all();
    }
}

struct ServerSlot {
    owner: String,
    phase: DesignCanvasServerPhase,
    port: Option<u16>,
    error: Option<String>,
    log_path: Option<PathBuf>,
    child: Option<Child>,
    pgid: Option<u32>,
    started_at: Option<Instant>,
}

impl ServerSlot {
    fn starting(owner: String) -> Self {
        Self {
            owner,
            phase: DesignCanvasServerPhase::ServerStarting,
            port: None,
            error: None,
            log_path: None,
            child: None,
            pgid: None,
            started_at: None,
        }
    }

    fn view(&self) -> DesignCanvasServerStatus {
        DesignCanvasServerStatus {
            status: self.phase,
            url: self.port.map(canvas_url),
            port: self.port,
            error: self.error.clone(),
            log_path: self
                .log_path
                .as_ref()
                .map(|path| path.display().to_string()),
        }
    }
}

impl DesignCanvasServers {
    /// 현재 작업의 준비된 Design Canvas URL만 허용한다. 외부 Preview URL은 이 경로에서
    /// 선택 결과를 전달할 수 없다.
    pub fn is_active_canvas_url(&self, task_id: i64, url: &tauri::Url) -> bool {
        let mut state = self.lock();
        let Some(slot) = state.slots.get_mut(&task_id) else {
            return false;
        };
        refresh_process(slot);
        slot.phase == DesignCanvasServerPhase::Ready
            && slot.port.is_some_and(|port| canvas_url_matches(port, url))
    }
    /// 같은 owner의 재호출은 현재 세션을 반환한다. 다른 owner는 이전 슬롯을 먼저
    /// 맵에서 떼어 내므로, 이전 start가 늦게 끝나도 새 슬롯을 덮어쓸 수 없다.
    pub fn start(
        &self,
        task_id: i64,
        owner: String,
        worktree: PathBuf,
    ) -> DesignCanvasServerStatus {
        if let Err(error) = ensure_design_entry(&worktree) {
            return failed_status(error);
        }
        let displaced = {
            let mut state = self.lock();
            if !state.accepting_starts || state.terminal_tasks.contains(&task_id) {
                return terminated_status();
            }
            if let Some(slot) = state.slots.get_mut(&task_id) {
                refresh_process(slot);
                if slot.owner == owner {
                    return slot.view();
                }
            }
            let displaced = state.slots.remove(&task_id);
            state
                .slots
                .insert(task_id, ServerSlot::starting(owner.clone()));
            state
                .launches
                .entry(task_id)
                .or_default()
                .insert(owner.clone());
            displaced
        };
        let _launch = LaunchGuard {
            state: self.state.clone(),
            launch_done: self.launch_done.clone(),
            task_id,
            owner: owner.clone(),
        };
        if let Some(mut slot) = displaced {
            terminate_slot(&mut slot);
        }

        let port = match vacant_loopback_port() {
            Ok(port) => port,
            Err(error) => return self.fail_if_owner(task_id, &owner, error),
        };
        let log_path = log_path(task_id, port);
        let log = match open_log(&log_path) {
            Ok(log) => log,
            Err(error) => {
                return self.fail_if_owner(
                    task_id,
                    &owner,
                    format!("개발 서버 로그를 열 수 없습니다: {error}"),
                );
            }
        };
        if !self.set_launch_details(task_id, &owner, port, log_path.clone()) {
            return self.status_owned(task_id, &owner);
        }

        let child = match spawn_vite(&worktree, port, log) {
            Ok(child) => child,
            Err(error) => return self.fail_if_owner(task_id, &owner, error),
        };
        let mut child = Some(child);
        let status = {
            let mut state = self.lock();
            match state
                .slots
                .get_mut(&task_id)
                .filter(|slot| slot.owner == owner)
            {
                Some(slot) => {
                    slot.pgid = Some(child.as_ref().expect("spawned child").id());
                    slot.child = child.take();
                    slot.view()
                }
                None => DesignCanvasServerStatus {
                    status: DesignCanvasServerPhase::ServerFailed,
                    url: None,
                    port: None,
                    error: Some("디자인 캔버스 시작이 더 새 세션으로 대체되었습니다".into()),
                    log_path: None,
                },
            }
        };
        if let Some(mut stale) = child {
            terminate_child(&mut stale);
        }
        status
    }

    pub fn status_owned(&self, task_id: i64, owner: &str) -> DesignCanvasServerStatus {
        let (port, started_at) = {
            let mut state = self.lock();
            let Some(slot) = state.slots.get_mut(&task_id) else {
                return absent_status();
            };
            if slot.owner != owner {
                return owner_mismatch_status();
            }
            refresh_process(slot);
            if slot.phase != DesignCanvasServerPhase::ServerStarting {
                return slot.view();
            }
            if slot
                .started_at
                .is_some_and(|at| at.elapsed() >= STARTUP_DEADLINE)
            {
                let mut expired = ServerSlot {
                    owner: String::new(),
                    phase: DesignCanvasServerPhase::ServerFailed,
                    port: None,
                    error: None,
                    log_path: None,
                    child: slot.child.take(),
                    pgid: slot.pgid.take(),
                    started_at: None,
                };
                slot.phase = DesignCanvasServerPhase::ServerFailed;
                slot.error = Some("디자인 캔버스 서버 시작 시간이 초과되었습니다".into());
                let status = slot.view();
                drop(state);
                terminate_slot(&mut expired);
                return status;
            }
            (slot.port, slot.started_at)
        };
        if port.is_some_and(design_page_ready) {
            let mut state = self.lock();
            if let Some(slot) = state.slots.get_mut(&task_id) {
                if slot.owner == owner
                    && slot.phase == DesignCanvasServerPhase::ServerStarting
                    && slot.port == port
                    && slot.started_at == started_at
                {
                    refresh_process(slot);
                    if slot.phase == DesignCanvasServerPhase::ServerStarting {
                        slot.phase = DesignCanvasServerPhase::Ready;
                    }
                }
                return if slot.owner == owner {
                    slot.view()
                } else {
                    owner_mismatch_status()
                };
            }
        }
        let state = self.lock();
        state
            .slots
            .get(&task_id)
            .filter(|slot| slot.owner == owner)
            .map(ServerSlot::view)
            .unwrap_or_else(owner_mismatch_status)
    }

    /// owner가 일치할 때만 제거한다. Strict Mode의 이전 cleanup은 새 effect의 서버를
    /// 관측할 수는 있어도 종료하지 못한다.
    pub fn stop_owned(&self, task_id: i64, owner: &str) {
        let slot = {
            let mut state = self.lock();
            if state
                .slots
                .get(&task_id)
                .is_some_and(|slot| slot.owner == owner)
            {
                state.slots.remove(&task_id)
            } else {
                None
            }
        };
        if let Some(mut slot) = slot {
            terminate_slot(&mut slot);
        }
        self.wait_for_launch(task_id, Some(owner));
    }

    /// 작업 종결과 앱 종료는 owner 없이도 확정적으로 제거한다.
    pub fn stop_task(&self, task_id: i64) {
        let slot = {
            let mut state = self.lock();
            state.terminal_tasks.insert(task_id);
            state.slots.remove(&task_id)
        };
        if let Some(mut slot) = slot {
            terminate_slot(&mut slot);
        }
        self.wait_for_launch(task_id, None);
    }

    pub fn stop_all(&self) {
        let slots: Vec<_> = {
            let mut state = self.lock();
            state.accepting_starts = false;
            state.slots.drain().map(|(_, slot)| slot).collect()
        };
        for mut slot in slots {
            terminate_slot(&mut slot);
        }
        self.wait_for_all_launches();
    }

    fn set_launch_details(&self, task_id: i64, owner: &str, port: u16, log_path: PathBuf) -> bool {
        let mut state = self.lock();
        let Some(slot) = state
            .slots
            .get_mut(&task_id)
            .filter(|slot| slot.owner == owner)
        else {
            return false;
        };
        slot.port = Some(port);
        slot.log_path = Some(log_path);
        slot.started_at = Some(Instant::now());
        true
    }

    fn fail_if_owner(&self, task_id: i64, owner: &str, error: String) -> DesignCanvasServerStatus {
        let mut state = self.lock();
        let Some(slot) = state
            .slots
            .get_mut(&task_id)
            .filter(|slot| slot.owner == owner)
        else {
            return absent_status();
        };
        slot.phase = DesignCanvasServerPhase::ServerFailed;
        slot.error = Some(error);
        slot.view()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CanvasState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn wait_for_launch(&self, task_id: i64, owner: Option<&str>) {
        let mut state = self.lock();
        while state
            .launches
            .get(&task_id)
            .is_some_and(|owners| owner.map_or(!owners.is_empty(), |owner| owners.contains(owner)))
        {
            state = self
                .launch_done
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    fn wait_for_all_launches(&self) {
        let mut state = self.lock();
        while !state.launches.is_empty() {
            state = self
                .launch_done
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

fn absent_status() -> DesignCanvasServerStatus {
    DesignCanvasServerStatus {
        status: DesignCanvasServerPhase::ServerFailed,
        url: None,
        port: None,
        error: Some("디자인 캔버스 서버가 실행 중이 아닙니다".into()),
        log_path: None,
    }
}

fn owner_mismatch_status() -> DesignCanvasServerStatus {
    DesignCanvasServerStatus {
        status: DesignCanvasServerPhase::ServerFailed,
        url: None,
        port: None,
        error: Some("Design Canvas 소유자가 현재 세션과 일치하지 않습니다".into()),
        log_path: None,
    }
}

fn terminated_status() -> DesignCanvasServerStatus {
    DesignCanvasServerStatus {
        status: DesignCanvasServerPhase::ServerFailed,
        url: None,
        port: None,
        error: Some("종결된 작업에서는 Design Canvas를 시작할 수 없습니다".into()),
        log_path: None,
    }
}

fn failed_status(error: String) -> DesignCanvasServerStatus {
    DesignCanvasServerStatus {
        status: DesignCanvasServerPhase::ServerFailed,
        url: None,
        port: None,
        error: Some(error),
        log_path: None,
    }
}

fn refresh_process(slot: &mut ServerSlot) {
    if matches!(slot.phase, DesignCanvasServerPhase::ServerFailed) {
        return;
    }
    let Some(child) = slot.child.as_mut() else {
        return;
    };
    match child.try_wait() {
        Ok(Some(exit)) => {
            slot.child = None;
            slot.phase = DesignCanvasServerPhase::ServerFailed;
            slot.error = Some(exit_error(exit.code(), slot.log_path.as_deref()));
        }
        Ok(None) => {}
        Err(error) => {
            slot.child = None;
            slot.phase = DesignCanvasServerPhase::ServerFailed;
            slot.error = Some(format!("디자인 캔버스 서버 상태 확인 실패: {error}"));
        }
    }
}

fn ensure_design_entry(worktree: &Path) -> Result<(), String> {
    for path in [
        worktree.join("design.html"),
        worktree.join("src/design/main.tsx"),
    ] {
        if !path.is_file() {
            return Err(format!(
                "Design Canvas 진입점이 없습니다: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn design_page_ready(port: u16) -> bool {
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(100)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
    if std::io::Write::write_all(
        &mut stream,
        b"GET /design.html HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    )
    .is_err()
    {
        return false;
    }
    let mut response = [0_u8; 64];
    match std::io::Read::read(&mut stream, &mut response) {
        Ok(read) => {
            response[..read].starts_with(b"HTTP/1.1 200")
                || response[..read].starts_with(b"HTTP/1.0 200")
        }
        Err(_) => false,
    }
}

fn spawn_vite(worktree: &Path, port: u16, log: File) -> Result<Child, String> {
    let stdout = log.try_clone().map_err(|error| error.to_string())?;
    let args = [
        OsString::from("--no-install"),
        OsString::from("vite"),
        OsString::from("--mode"),
        OsString::from("design"),
        OsString::from("--host"),
        OsString::from("127.0.0.1"),
        OsString::from("--port"),
        OsString::from(port.to_string()),
        OsString::from("--strictPort"),
    ];
    crate::managed_process::spawn_registered("npx", &args, None, |process| {
        process
            .current_dir(worktree)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(log));
    })
    .map(|spawned| spawned.child)
    .map_err(|error| format!("npx vite를 실행할 수 없습니다: {error}"))
}

fn vacant_loopback_port() -> Result<u16, String> {
    for _ in 0..16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|error| error.to_string())?;
        let port = listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        drop(listener);
        if port != RESERVED_APP_PORT {
            return Ok(port);
        }
    }
    Err("디자인 캔버스용 빈 포트를 찾을 수 없습니다".into())
}

fn canvas_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/design.html")
}

fn canvas_url_matches(port: u16, url: &tauri::Url) -> bool {
    url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.port_or_known_default() == Some(port)
        && url.path() == "/design.html"
}

fn log_path(task_id: i64, port: u16) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir()
        .join("dojang-design-canvas")
        .join(format!("task-{task_id}-{port}-{nonce}.log"))
}

fn open_log(path: &Path) -> std::io::Result<File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.create_new(true).append(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn exit_error(code: Option<i32>, log_path: Option<&Path>) -> String {
    let code = code.map_or_else(|| "signal".to_string(), |code| code.to_string());
    let tail = log_path.and_then(log_tail).unwrap_or_default();
    if tail.is_empty() {
        format!("디자인 캔버스 서버가 종료되었습니다 (exit {code})")
    } else {
        format!("디자인 캔버스 서버가 종료되었습니다 (exit {code}): {tail}")
    }
}

fn log_tail(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(LOG_TAIL_BYTES)))
        .ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let tail = text.lines().rev().take(3).collect::<Vec<_>>();
    (!tail.is_empty()).then(|| tail.into_iter().rev().collect::<Vec<_>>().join("\n"))
}

fn terminate_slot(slot: &mut ServerSlot) {
    if let Some(pgid) = slot.pgid {
        terminate_group(pgid, slot.child.as_mut());
    } else if let Some(child) = slot.child.as_mut() {
        terminate_child(child);
    }
    slot.child = None;
}

fn terminate_group(pgid: u32, mut child: Option<&mut Child>) {
    if pgid == 0 {
        return;
    }
    #[cfg(unix)]
    {
        use nix::sys::signal::{killpg, Signal};
        use nix::unistd::Pid;
        let group = Pid::from_raw(pgid as i32);
        let _ = killpg(group, Signal::SIGTERM);
        let deadline = Instant::now() + TERM_GRACE;
        while Instant::now() < deadline {
            if let Some(child) = child.as_deref_mut() {
                let _ = child.try_wait();
            }
            std::thread::sleep(TERM_POLL);
        }
        let _ = killpg(group, Signal::SIGKILL);
        // parent가 먼저 끝난 뒤 남은 손자는 우리에게 wait handle이 없다. killpg 성공만으로
        // 즉시 반환하면 다음 기동이 같은 포트를 잡는 경합이 남으므로 그룹 소멸까지 짧게 확인한다.
        let reap_deadline = Instant::now() + Duration::from_secs(2);
        while crate::verify::process_group_alive(pgid) && Instant::now() < reap_deadline {
            std::thread::sleep(TERM_POLL);
        }
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pgid.to_string(), "/T", "/F"])
            .output();
    }
    if let Some(child) = child {
        let _ = child.wait();
    }
}

fn terminate_child(child: &mut Child) {
    let pid = child.id();
    if pid == 0 {
        return;
    }
    #[cfg(unix)]
    {
        use nix::sys::signal::{killpg, Signal};
        use nix::unistd::Pid;
        let group = Pid::from_raw(pid as i32);
        let _ = killpg(group, Signal::SIGTERM);
        let deadline = std::time::Instant::now() + TERM_GRACE;
        while std::time::Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(TERM_POLL);
        }
        let _ = killpg(group, Signal::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    static TEST_NONCE: AtomicU64 = AtomicU64::new(0);

    fn test_log(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "dojang-design-canvas-{name}-{}-{}.log",
            std::process::id(),
            TEST_NONCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn sleeper() -> Child {
        crate::managed_process::spawn_registered(
            "/bin/sh",
            &[OsString::from("-c"), OsString::from("sleep 30")],
            None,
            |_| {},
        )
        .unwrap()
        .child
    }

    #[test]
    fn port_selection_never_uses_tauri_dev_port() {
        for _ in 0..32 {
            assert_ne!(vacant_loopback_port().unwrap(), RESERVED_APP_PORT);
        }
    }

    fn wait_for_failed_status(
        servers: &DesignCanvasServers,
        task_id: i64,
        owner: &str,
    ) -> DesignCanvasServerStatus {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let status = servers.status_owned(task_id, owner);
            if status.status == DesignCanvasServerPhase::ServerFailed || Instant::now() >= deadline {
                return status;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn start_rejects_worktree_without_the_design_entry() {
        let root =
            crate::testtmp::dir().join(format!("design-canvas-empty-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let status = DesignCanvasServers::default().start(7, "owner".into(), root.clone());
        assert_eq!(status.status, DesignCanvasServerPhase::ServerFailed);
        assert!(status.error.unwrap().contains("진입점"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn status_reports_log_tail_after_server_failure() {
        let log = test_log("failed");
        std::fs::write(&log, "first\nVite failed to start\nport busy\n").unwrap();
        let servers = DesignCanvasServers::default();
        servers.lock().slots.insert(
            7,
            ServerSlot {
                owner: "owner".into(),
                phase: DesignCanvasServerPhase::ServerStarting,
                port: Some(45678),
                error: None,
                log_path: Some(log.clone()),
                child: Some(
                    crate::managed_process::spawn_registered(
                        "/bin/sh",
                        &[OsString::from("-c"), OsString::from("exit 17")],
                        None,
                        |_| {},
                    )
                    .unwrap()
                    .child,
                ),
                pgid: None,
                started_at: Some(Instant::now()),
            },
        );
        let state = wait_for_failed_status(&servers, 7, "owner");
        servers.stop_owned(7, "owner");
        assert_eq!(state.status, DesignCanvasServerPhase::ServerFailed);
        assert!(state.error.unwrap().contains("port busy"));
        assert_eq!(
            state.log_path.as_deref(),
            Some(log.to_string_lossy().as_ref())
        );
        let _ = std::fs::remove_file(log);
    }

    #[test]
    fn stale_owner_cannot_stop_new_server() {
        let servers = DesignCanvasServers::default();
        servers
            .lock()
            .slots
            .insert(7, ServerSlot::starting("new-owner".into()));
        servers.stop_owned(7, "old-owner");
        assert_eq!(servers.lock().slots.get(&7).unwrap().owner, "new-owner");
    }

    #[test]
    fn stop_reaps_the_entire_server_process_group() {
        let servers = DesignCanvasServers::default();
        let child = sleeper();
        let pid = child.id() as i32;
        servers.lock().slots.insert(
            7,
            ServerSlot {
                owner: "owner".into(),
                phase: DesignCanvasServerPhase::ServerStarting,
                port: None,
                error: None,
                log_path: None,
                child: Some(child),
                pgid: Some(pid as u32),
                started_at: Some(Instant::now()),
            },
        );
        servers.stop_owned(7, "owner");
        #[cfg(unix)]
        assert_eq!(unsafe { nix::libc::kill(pid, 0) }, -1);
    }

    #[cfg(unix)]
    #[test]
    fn stop_reaps_descendants_after_the_server_parent_exits() {
        let child = crate::managed_process::spawn_registered(
            "/bin/sh",
            &[
                OsString::from("-c"),
                OsString::from("(trap '' TERM; sleep 30) & exit 0"),
            ],
            None,
            |_| {},
        )
        .unwrap()
        .child;
        let pgid = child.id() as i32;
        let servers = DesignCanvasServers::default();
        servers.lock().slots.insert(
            7,
            ServerSlot {
                owner: "owner".into(),
                phase: DesignCanvasServerPhase::Ready,
                port: None,
                error: None,
                log_path: None,
                child: Some(child),
                pgid: Some(pgid as u32),
                started_at: Some(Instant::now()),
            },
        );
        let status = wait_for_failed_status(&servers, 7, "owner");
        let descendants_alive = unsafe { nix::libc::kill(-pgid, 0) };
        // Always reap the test group before assertions, including a failed timeout.
        servers.stop_owned(7, "owner");
        assert_eq!(status.status, DesignCanvasServerPhase::ServerFailed);
        assert_eq!(descendants_alive, 0);
        assert_eq!(unsafe { nix::libc::kill(-pgid, 0) }, -1);
    }

    #[test]
    fn ready_status_uses_only_its_reserved_loopback_port() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let response = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 512];
            let _ = std::io::Read::read(&mut stream, &mut request);
            std::io::Write::write_all(
                &mut stream,
                b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        });
        let servers = DesignCanvasServers::default();
        servers.lock().slots.insert(
            7,
            ServerSlot {
                owner: "owner".into(),
                phase: DesignCanvasServerPhase::ServerStarting,
                port: Some(port),
                error: None,
                log_path: None,
                child: Some(sleeper()),
                pgid: None,
                started_at: Some(Instant::now()),
            },
        );
        let state = servers.status_owned(7, "owner");
        assert_eq!(state.status, DesignCanvasServerPhase::Ready);
        assert_eq!(state.url.as_deref(), Some(canvas_url(port).as_str()));
        servers.stop_task(7);
        response.join().unwrap();
    }

    #[test]
    fn active_canvas_url_requires_ready_slot_loopback_port_and_design_entry() {
        let servers = DesignCanvasServers::default();
        servers.lock().slots.insert(
            7,
            ServerSlot {
                owner: "owner".into(),
                phase: DesignCanvasServerPhase::Ready,
                port: Some(45123),
                error: None,
                log_path: None,
                child: None,
                pgid: None,
                started_at: None,
            },
        );
        assert!(servers.is_active_canvas_url(
            7,
            &tauri::Url::parse("http://127.0.0.1:45123/design.html?screen=card").unwrap()
        ));
        for url in [
            "http://127.0.0.1:45124/design.html",
            "http://localhost:45123/design.html",
            "http://127.0.0.1:45123/other.html",
            "https://127.0.0.1:45123/design.html",
        ] {
            assert!(
                !servers.is_active_canvas_url(7, &tauri::Url::parse(url).unwrap()),
                "{url}"
            );
        }
    }

    #[test]
    fn active_canvas_url_rejects_a_stale_exited_server_handle() {
        let child = crate::managed_process::spawn_registered(
            "/bin/sh",
            &[OsString::from("-c"), OsString::from("exit 0")],
            None,
            |_| {},
        )
        .unwrap()
        .child;
        let servers = DesignCanvasServers::default();
        servers.lock().slots.insert(
            7,
            ServerSlot {
                owner: "owner".into(),
                phase: DesignCanvasServerPhase::Ready,
                port: Some(45123),
                error: None,
                log_path: None,
                child: Some(child),
                pgid: None,
                started_at: None,
            },
        );
        let status = wait_for_failed_status(&servers, 7, "owner");
        servers.stop_owned(7, "owner");
        assert_eq!(status.status, DesignCanvasServerPhase::ServerFailed);
        assert!(!servers.is_active_canvas_url(
            7,
            &tauri::Url::parse("http://127.0.0.1:45123/design.html").unwrap()
        ));
    }

    #[test]
    fn ready_server_exit_is_observed_on_the_next_status_poll() {
        let servers = DesignCanvasServers::default();
        servers.lock().slots.insert(
            7,
            ServerSlot {
                owner: "owner".into(),
                phase: DesignCanvasServerPhase::Ready,
                port: None,
                error: None,
                log_path: None,
                child: Some(
                    crate::managed_process::spawn_registered(
                        "/bin/sh",
                        &[OsString::from("-c"), OsString::from("exit 9")],
                        None,
                        |_| {},
                    )
                    .unwrap()
                    .child,
                ),
                pgid: None,
                started_at: Some(Instant::now()),
            },
        );
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(
            servers.status_owned(7, "owner").status,
            DesignCanvasServerPhase::ServerFailed
        );
    }

    #[test]
    fn startup_deadline_reaps_late_server_and_terminal_stop_blocks_restart() {
        let servers = DesignCanvasServers::default();
        let child = sleeper();
        let pid = child.id() as i32;
        servers.lock().slots.insert(
            7,
            ServerSlot {
                owner: "owner".into(),
                phase: DesignCanvasServerPhase::ServerStarting,
                port: None,
                error: None,
                log_path: None,
                child: Some(child),
                pgid: Some(pid as u32),
                started_at: Some(Instant::now() - STARTUP_DEADLINE),
            },
        );
        assert_eq!(
            servers.status_owned(7, "owner").status,
            DesignCanvasServerPhase::ServerFailed
        );
        #[cfg(unix)]
        assert_eq!(unsafe { nix::libc::kill(pid, 0) }, -1);

        servers.stop_task(8);
        assert_eq!(
            servers
                .start(8, "owner".into(), std::env::temp_dir())
                .status,
            DesignCanvasServerPhase::ServerFailed
        );
    }

    #[test]
    fn status_requires_current_owner_and_serializes_log_path_in_camel_case() {
        let servers = DesignCanvasServers::default();
        let log = test_log("serialization");
        servers.lock().slots.insert(
            7,
            ServerSlot {
                owner: "current".into(),
                phase: DesignCanvasServerPhase::ServerFailed,
                port: None,
                error: Some("failed".into()),
                log_path: Some(log.clone()),
                child: None,
                pgid: None,
                started_at: None,
            },
        );
        assert_eq!(
            servers.status_owned(7, "stale").error.as_deref(),
            owner_mismatch_status().error.as_deref()
        );
        let json = serde_json::to_value(servers.status_owned(7, "current")).unwrap();
        assert_eq!(json["logPath"], log.display().to_string());
        assert!(json.get("log_path").is_none());
    }

    #[test]
    fn task_stop_waits_for_an_inflight_start_reservation() {
        let servers = DesignCanvasServers::default();
        let task_id = 77;
        let owner = "starting-owner".to_string();
        servers
            .lock()
            .launches
            .entry(task_id)
            .or_default()
            .insert(owner.clone());
        let launch = LaunchGuard {
            state: servers.state.clone(),
            launch_done: servers.launch_done.clone(),
            task_id,
            owner,
        };
        let done = Arc::new(AtomicBool::new(false));
        let stopped = servers.clone();
        let done_in_thread = done.clone();
        let waiter = std::thread::spawn(move || {
            stopped.stop_task(task_id);
            done_in_thread.store(true, Ordering::Release);
        });
        std::thread::sleep(Duration::from_millis(30));
        assert!(!done.load(Ordering::Acquire));
        drop(launch);
        waiter.join().unwrap();
        assert!(done.load(Ordering::Acquire));
    }

    #[test]
    fn manager_starts_vite_from_the_worktree_and_serves_design_html() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        assert!(root.join("design.html").is_file());
        let servers = DesignCanvasServers::default();
        let owner = "vite-smoke";
        let starting = servers.start(91_001, owner.into(), root);
        assert_eq!(starting.status, DesignCanvasServerPhase::ServerStarting);
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut current = starting;
        while Instant::now() < deadline && current.status == DesignCanvasServerPhase::ServerStarting
        {
            std::thread::sleep(Duration::from_millis(100));
            current = servers.status_owned(91_001, owner);
        }
        let error = current.error.clone();
        servers.stop_owned(91_001, owner);
        assert_eq!(current.status, DesignCanvasServerPhase::Ready, "{error:?}");
    }
}
