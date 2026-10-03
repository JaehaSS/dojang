//! 로컬 STT 서버(`ohr`) 수명 관리 — 사용자 클릭으로 띄우고, 메인 창과 함께 죽는다.
//!
//! ADR 0103 개정: **앱 시작 시 자동 spawn은 하지 않는다** — 사용자 클릭으로만 띄우고
//! 메인 창이 닫히면 함께 죽는다. 자동 기동은 앱을 켤 때마다 Apple SpeechAnalyzer
//! 프로세스를 하나 물게 하고, 사용자가 서버를 쓸 의사를 밝힌 적이 없는데도 포트를
//! 점유한다. 죽이는 쪽도 같은 이유로 창 닫기에 묶는다 — 고아로 남으면 다음 기동이
//! 포트 선점으로 실패하고, 사용자는 앱 밖에서 그것을 찾아 죽여야 한다.
//!
//! `VoiceManaged` 와 뮤텍스를 나눠 쓰지 않는다. 그쪽은 동기 핫키 핸들러가 만지는
//! 자리라, 기동 대기(최대 5초)를 같은 락에 얹으면 핫키가 그만큼 멈춘다.

use super::VoiceSettings;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 서버 바이너리 이름. 경로 A(runbook `docs/runbooks/voice-stt-server.md`)의 단일 바이너리다.
const BIN: &str = "ohr";
/// `/health` 폴링 상한. ohr 은 모델 다운로드가 없어 즉시 뜬다 — 5초는 넉넉한 여유다.
const READY_TIMEOUT: Duration = Duration::from_secs(5);
const READY_POLL: Duration = Duration::from_millis(200);
/// 포트에 누가 붙어 있는지 보는 대기 상한. 루프백은 붙을 거면 즉시 붙는다 —
/// 핫키 경로라 그 이상 기다리면 사용자가 침묵을 느낀다.
const HOTKEY_PROBE: Duration = Duration::from_millis(300);
/// SIGTERM 이후 SIGKILL 까지의 유예. `pty::Session::terminate` 와 같은 값이다.
const TERM_GRACE: Duration = Duration::from_millis(800);
const TERM_POLL: Duration = Duration::from_millis(50);
/// 로그 꼬리를 읽을 때의 상한. 로그는 계속 자라므로 전체를 메모리에 들이지 않는다.
const LOG_TAIL_BYTES: u64 = 8 * 1024;

const ERR_NOT_INSTALLED: &str =
    "ohr가 설치되어 있지 않습니다 — docs/runbooks/voice-stt-server.md 경로 A";
const ERR_NOT_HTTP: &str = "앱이 띄우는 서버는 http:// 주소여야 합니다";
const ERR_NOT_LOOPBACK: &str = "앱에서 띄우는 서버는 로컬 주소(127.0.0.1)여야 합니다";
const ERR_NO_PORT: &str = "서버 주소에 포트가 없습니다 — 예: http://127.0.0.1:11434/v1";
const ERR_PORT_RANGE: &str = "포트는 1–65535 사이여야 합니다";
const ERR_UNPARSABLE: &str = "서버 주소를 해석할 수 없습니다";

#[derive(Serialize, Clone, Debug)]
pub struct VoiceServerStatus {
    /// 바이너리를 찾았는가. 설정 화면이 "설치 안 됨"과 "꺼져 있음"을 갈라 보여준다.
    pub installed: bool,
    pub binary: Option<String>,
    /// 앱이 띄운 자식이 살아 있는가. 남이 띄운 서버는 여기 잡히지 않는다.
    pub running: bool,
    pub pid: Option<u32>,
    pub port: Option<u16>,
}

struct Running {
    child: Child,
    port: u16,
}

/// Tauri managed state. `Default` 로 등록되고 첫 클릭까지 아무 프로세스도 갖지 않는다.
#[derive(Default)]
pub struct ServerManaged {
    child: Mutex<Option<Running>>,
}

impl ServerManaged {
    /// 이미 살아 있으면 그대로 돌려준다 — 버튼 연타가 프로세스를 늘리지 않는다.
    ///
    /// 검사·spawn·저장을 락 하나 안에서 끝낸다. 나눠 잡으면 두 클릭이 둘 다 "안 떠 있다"를
    /// 보고 각자 spawn 해, 뒤에 뜬 쪽은 포트를 못 잡고 죽으면서 앞의 것을 상태에서 덮어쓴다.
    /// spawn 은 동기라 락 안에서 끝나고, `.await` 는 락을 놓은 뒤에만 온다.
    pub async fn start(&self, settings: &VoiceSettings) -> Result<VoiceServerStatus, String> {
        let log = log_path();
        let port = {
            let mut guard = self.child.lock().unwrap();
            reap_if_exited(&mut guard);
            if let Some(live) = guard.as_ref() {
                return Ok(status_of(Some(live)));
            }
            let bin = resolve_binary(home_dir().as_deref())
                .ok_or_else(|| ERR_NOT_INSTALLED.to_string())?;
            let port = local_port_from_base_url(&settings.base_url)?;
            let child = spawn(&bin, port, settings, log.as_deref())?;
            *guard = Some(Running { child, port });
            port
        };
        self.await_ready(port, log.as_deref()).await
    }

    /// 핫키 경로의 지연 기동. 설정 주소가 로컬이고 그 포트에 아무도 없을 때만 ohr 를 띄운다.
    ///
    /// 앱 시작 시 자동 spawn 금지(ADR 0103)와 다르다 — 핫키를 누른 것이 곧 서버를 쓰겠다는
    /// 의사다. 남이 띄운 서버(수동 ohr, mlx)는 포트 응답으로 알아보고 건드리지 않는다.
    pub async fn ensure_for_hotkey(&self, settings: &VoiceSettings) -> Result<(), String> {
        // 원격·https·포트 없는 주소는 우리가 띄울 대상이 아니다 — transcribe 가 자체 오류를 낸다.
        let Ok(port) = local_port_from_base_url(&settings.base_url) else {
            return Ok(());
        };
        let installed = resolve_binary(home_dir().as_deref()).is_some();
        let plan = plan_hotkey_start(
            Some(port),
            port_answers(port).await,
            installed,
            &settings.language,
        );
        match plan {
            HotkeyStart::Skip => Ok(()),
            HotkeyStart::Start => self.start(settings).await.map(|_| ()),
            HotkeyStart::NotInstalled => Err(format!(
                "로컬 STT 서버(127.0.0.1:{port})가 꺼져 있고 ohr가 설치되어 있지 않습니다 — docs/runbooks/voice-stt-server.md 경로 A"
            )),
            HotkeyStart::BadLanguage => Err(format!(
                "ohr는 언어를 `ko-KR`처럼 지역이 붙은 형식으로 받습니다(현재 '{}') — 설정에서 'ohr 프리셋 적용' 후 저장하세요",
                settings.language
            )),
        }
    }

    /// 프로세스가 실제로 끝날 때까지 기다린 뒤 답한다.
    pub fn stop(&self) -> VoiceServerStatus {
        // 종료 시퀀스가 최대 800ms 라 락을 놓고 기다린다 — 그 사이 status 조회는 막지 않는다.
        let taken = self.child.lock().unwrap().take();
        if let Some(running) = taken {
            terminate_blocking(running.child);
        }
        self.status()
    }

    pub fn status(&self) -> VoiceServerStatus {
        let mut guard = self.child.lock().unwrap();
        // 크래시한 서버가 running 으로 읽히면 사용자는 켤 수도 끌 수도 없게 된다.
        reap_if_exited(&mut guard);
        status_of(guard.as_ref())
    }

    async fn await_ready(
        &self,
        port: u16,
        log: Option<&Path>,
    ) -> Result<VoiceServerStatus, String> {
        let client = reqwest::Client::builder()
            .timeout(READY_POLL)
            .build()
            .map_err(|e| format!("HTTP 클라이언트를 만들 수 없습니다: {e}"))?;
        let url = format!("http://127.0.0.1:{port}/health");
        let deadline = Instant::now() + READY_TIMEOUT;
        while Instant::now() < deadline {
            // 포트 선점이 가장 흔한 실패다 — ohr 가 즉사하므로 5초를 기다리지 않고 사유를 올린다.
            if let Some(exit) = reap_if_exited(&mut self.child.lock().unwrap()) {
                return Err(format!(
                    "서버가 바로 종료됐습니다 ({exit}) — 로그: {}{}",
                    log_label(log),
                    log_tail(log)
                ));
            }
            if let Ok(response) = client.get(&url).send().await {
                if response.status().is_success() {
                    return Ok(self.status());
                }
            }
            tokio::time::sleep(READY_POLL).await;
        }
        // 살려둔다 — 느린 기동일 수 있고, 죽이면 사용자가 로그로 원인을 볼 기회를 잃는다.
        Err(format!(
            "서버가 5초 안에 응답하지 않았습니다 — 프로세스는 살아 있습니다. 로그를 확인한 뒤 중지하세요: {}",
            log_label(log)
        ))
    }
}

#[derive(Debug, PartialEq, Eq)]
enum HotkeyStart {
    Skip,
    Start,
    NotInstalled,
    BadLanguage,
}

/// 판정만 떼어 둔다 — 포트 탐지·파일시스템 없이 네 갈래를 그대로 시험할 수 있다.
fn plan_hotkey_start(
    local_port: Option<u16>,
    port_open: bool,
    installed: bool,
    language: &str,
) -> HotkeyStart {
    // 원격 주소는 우리 소관이 아니고, 포트가 열려 있으면 이미 누군가 서빙 중이다.
    if local_port.is_none() || port_open {
        return HotkeyStart::Skip;
    }
    if !installed {
        return HotkeyStart::NotInstalled;
    }
    // 우리가 띄우는 ohr 에만 걸리는 제약이다 — `ko` 는 unsupported locale 로 500 을 준다.
    let language = language.trim();
    if !language.is_empty() && !language.contains(['-', '_']) {
        return HotkeyStart::BadLanguage;
    }
    HotkeyStart::Start
}

/// 그 포트에 누가 붙어 있는가. `/health` 를 부르지 않는다 — 우리가 아는 서버인지가 아니라
/// 포트가 비었는지만 알면 되고, 남의 서버에 요청을 보내지 않는 쪽이 맞다.
async fn port_answers(port: u16) -> bool {
    let connect = tokio::net::TcpStream::connect(("127.0.0.1", port));
    matches!(tokio::time::timeout(HOTKEY_PROBE, connect).await, Ok(Ok(_)))
}

/// 끝난 자식을 reap 하고 자리를 비운다. zombie 를 남기지 않기 위해 `try_wait` 로 거둔다.
fn reap_if_exited(slot: &mut Option<Running>) -> Option<std::process::ExitStatus> {
    let running = slot.as_mut()?;
    let Ok(Some(exit)) = running.child.try_wait() else {
        return None;
    };
    *slot = None;
    Some(exit)
}

/// 상태 조립을 한 자리에 둔다 — `start` 의 멱등 경로와 `status` 가 같은 모양을 내야 한다.
fn status_of(live: Option<&Running>) -> VoiceServerStatus {
    let binary = resolve_binary(home_dir().as_deref());
    VoiceServerStatus {
        installed: binary.is_some(),
        binary: binary.map(|p| p.to_string_lossy().into_owned()),
        running: live.is_some(),
        pid: live.map(|r| r.child.id()),
        port: live.map(|r| r.port),
    }
}

/// `<home>/.praxis-stt/bin/ohr` 를 먼저 본다 — runbook 이 안내하는 설치 위치다. 없으면 PATH.
pub fn resolve_binary(home: Option<&Path>) -> Option<PathBuf> {
    resolve_with(home, |bin| {
        crate::agenthealth::detect::resolve_bin(bin).map(PathBuf::from)
    })
}

/// PATH 조회를 주입받는다 — 테스트가 프로세스 환경변수를 건드리지 않고 두 갈래를 다 볼 수 있다.
fn resolve_with(
    home: Option<&Path>,
    path_lookup: impl Fn(&str) -> Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(home) = home {
        let installed = home.join(".praxis-stt").join("bin").join(BIN);
        if installed.is_file() {
            return Some(installed);
        }
    }
    path_lookup(BIN)
}

/// 설정의 base_url 에서 우리가 바인드할 포트를 뽑는다.
///
/// `url` 크레이트를 들이지 않고 직접 자른다. 받아야 하는 형태가 loopback http 하나뿐이라
/// 일반 URL 파서의 표면이 필요하지 않다. 여기서 거른 것은 전부 우리가 띄울 수 없는 주소다 —
/// 남의 서버를 가리키는 설정으로 spawn 하면 사용자는 엉뚱한 포트에 뜬 서버를 보게 된다.
pub fn local_port_from_base_url(base_url: &str) -> Result<u16, String> {
    let (scheme, rest) = base_url
        .trim()
        .split_once("://")
        .ok_or_else(|| ERR_UNPARSABLE.to_string())?;
    // ohr 은 TLS 를 끝내지 않는다 — https 를 받아 주면 기동은 되고 전사 요청만 실패한다.
    if !scheme.eq_ignore_ascii_case("http") {
        return Err(ERR_NOT_HTTP.to_string());
    }
    let authority = rest.split('/').next().unwrap_or_default();
    let (host, port) = split_host_port(authority).ok_or_else(|| ERR_UNPARSABLE.to_string())?;
    // `--host 127.0.0.1` 로 IPv4 loopback 에만 바인드하므로 `[::1]` 은 연결되지 않는다.
    if !matches!(
        host.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost"
    ) {
        return Err(ERR_NOT_LOOPBACK.to_string());
    }
    let port = port.ok_or_else(|| ERR_NO_PORT.to_string())?;
    match port.parse::<u16>() {
        // 0 은 "커널이 골라 준다"는 뜻이라 우리가 다시 찾아갈 주소가 되지 못한다.
        Ok(0) => Err(ERR_NO_PORT.to_string()),
        Ok(port) => Ok(port),
        Err(_) => Err(ERR_PORT_RANGE.to_string()),
    }
}

/// 호스트와 포트를 가른다. `[::1]:8080` 의 콜론 때문에 대괄호를 먼저 보고,
/// 없으면 마지막 콜론이 구분자다. 대괄호 호스트는 여기서 갈라진 뒤 호출자가 거절한다.
fn split_host_port(authority: &str) -> Option<(&str, Option<&str>)> {
    if authority.is_empty() {
        return None;
    }
    if authority.starts_with('[') {
        let end = authority.find(']')?;
        let tail = &authority[end + 1..];
        let port = if tail.is_empty() {
            None
        } else {
            Some(tail.strip_prefix(':')?)
        };
        return Some((&authority[..=end], port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => Some((host, Some(port))),
        None => Some((authority, None)),
    }
}

fn spawn(
    bin: &Path,
    port: u16,
    settings: &VoiceSettings,
    log: Option<&Path>,
) -> Result<Child, String> {
    let port = port.to_string();
    let mut cmd = Command::new(bin);
    cmd.args(["--serve", "--host", "127.0.0.1", "--port", port.as_str()]);
    let token = settings.api_key.trim();
    if !token.is_empty() {
        // argv 로 넘기지 않는다 — 같은 사용자로 도는 에이전트 PTY 가 `ps` 로 읽어 간다.
        cmd.env("OHR_TOKEN", token);
    }
    cmd.stdin(Stdio::null())
        .stdout(log_sink(log))
        .stderr(log_sink(log));
    #[cfg(unix)]
    {
        // 프로세스 그룹을 떼어 둔다 — 종료 때 killpg 로 손자까지 한 번에 정리한다.
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn()
        .map_err(|e| format!("ohr를 실행할 수 없습니다: {e}"))
}

fn home_dir() -> Option<PathBuf> {
    crate::usage::home_dir()
}

fn log_path() -> Option<PathBuf> {
    Some(home_dir()?.join(".praxis-stt").join("ohr.log"))
}

/// 로그를 열지 못하면 출력을 버린다 — 기동 자체를 막을 이유가 아니다.
///
/// unix 에서 0600 으로 만든다. 전사 요청의 흔적이 남는 파일이라 같은 머신의 다른
/// 사용자에게 열어 둘 이유가 없다.
fn log_sink(log: Option<&Path>) -> Stdio {
    let Some(log) = log else {
        return Stdio::null();
    };
    if let Some(dir) = log.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(log).map(Stdio::from).unwrap_or(Stdio::null())
}

fn log_label(log: Option<&Path>) -> String {
    log.map(|p| p.display().to_string())
        .unwrap_or_else(|| "없음".to_string())
}

/// 로그 마지막 3줄. 포트 선점 같은 즉사 사유가 대개 여기 한 줄로 찍힌다.
fn log_tail(log: Option<&Path>) -> String {
    let Some(text) = log.and_then(read_tail) else {
        return String::new();
    };
    let mut lines: Vec<&str> = text.lines().rev().take(3).collect();
    if lines.is_empty() {
        return String::new();
    }
    lines.reverse();
    format!("\n{}", lines.join("\n"))
}

/// 파일 끝에서 최대 8KB. 경계가 UTF-8 문자 중간에 떨어질 수 있어 lossy 로 흘려보낸다 —
/// 잘린 첫 줄은 어차피 버려진다.
fn read_tail(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(LOG_TAIL_BYTES)))
        .ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// 프로세스 그룹 단위 SIGTERM → (최대 800ms 폴링) → SIGKILL → reap. 호출자를 블로킹한다.
///
/// 스레드로 넘기지 않는다. 두 가지가 걸린다 — 중지 직후의 재시작이 아직 살아 있는 옛
/// 프로세스 탓에 같은 포트를 잡지 못하고, `CloseRequested` 에서는 앱이 스레드보다 먼저
/// 끝나 SIGKILL 이 아예 실행되지 않는다. 정상 종료는 첫 폴링(50ms) 안에 끝난다.
fn terminate_blocking(mut child: Child) {
    let pid = child.id();
    // pid==0 가드: killpg(0, ...)은 호출자 자신의 프로세스 그룹을 죽인다.
    if pid == 0 {
        return;
    }
    #[cfg(unix)]
    {
        use nix::sys::signal::{killpg, Signal};
        use nix::unistd::Pid;
        let pgid = Pid::from_raw(pid as i32);
        let _ = killpg(pgid, Signal::SIGTERM);
        let deadline = Instant::now() + TERM_GRACE;
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(TERM_POLL);
        }
        let _ = killpg(pgid, Signal::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_port_accepts_loopback_http_forms() {
        assert_eq!(
            local_port_from_base_url("http://127.0.0.1:11434/v1"),
            Ok(11434)
        );
        assert_eq!(local_port_from_base_url("http://localhost:8765"), Ok(8765));
        assert_eq!(
            local_port_from_base_url("http://127.0.0.1:11491/"),
            Ok(11491)
        );
        // 스킴·호스트는 대소문자를 가리지 않는다.
        assert_eq!(
            local_port_from_base_url("HTTP://LOCALHOST:11434/v1"),
            Ok(11434)
        );
    }

    #[test]
    fn local_port_rejects_remote_missing_port_and_garbage() {
        assert_eq!(
            local_port_from_base_url("http://api.openai.com:443/v1"),
            Err(ERR_NOT_LOOPBACK.to_string())
        );
        assert_eq!(
            local_port_from_base_url("http://127.0.0.1/v1"),
            Err(ERR_NO_PORT.to_string())
        );
        assert_eq!(
            local_port_from_base_url("포트가 아니다"),
            Err(ERR_UNPARSABLE.to_string())
        );
    }

    /// https 는 ohr 이 끝내지 못하고, `[::1]` 은 IPv4 바인드에 연결되지 않는다.
    #[test]
    fn local_port_rejects_https_and_ipv6_loopback() {
        assert_eq!(
            local_port_from_base_url("https://127.0.0.1:8080/v1"),
            Err(ERR_NOT_HTTP.to_string())
        );
        assert_eq!(
            local_port_from_base_url("http://[::1]:11491/v1"),
            Err(ERR_NOT_LOOPBACK.to_string())
        );
    }

    #[test]
    fn local_port_rejects_zero_and_out_of_range() {
        assert_eq!(
            local_port_from_base_url("http://127.0.0.1:0/v1"),
            Err(ERR_NO_PORT.to_string())
        );
        assert_eq!(
            local_port_from_base_url("http://127.0.0.1:99999/v1"),
            Err(ERR_PORT_RANGE.to_string())
        );
    }

    /// 원격 주소이거나 포트가 이미 응답하면 손대지 않는다.
    #[test]
    fn hotkey_start_skips_remote_and_live_port() {
        assert_eq!(
            plan_hotkey_start(None, false, true, "ko-KR"),
            HotkeyStart::Skip
        );
        assert_eq!(
            plan_hotkey_start(Some(11434), true, true, "ko-KR"),
            HotkeyStart::Skip
        );
    }

    #[test]
    fn hotkey_start_reports_missing_binary_and_bare_language() {
        assert_eq!(
            plan_hotkey_start(Some(11434), false, false, "ko-KR"),
            HotkeyStart::NotInstalled
        );
        assert_eq!(
            plan_hotkey_start(Some(11434), false, true, "ko"),
            HotkeyStart::BadLanguage
        );
    }

    /// 언어를 비우면 ohr 이 스스로 고른다 — 우리가 막을 이유가 없다.
    #[test]
    fn hotkey_start_launches_for_locale_or_empty_language() {
        assert_eq!(
            plan_hotkey_start(Some(11434), false, true, "ko-KR"),
            HotkeyStart::Start
        );
        assert_eq!(
            plan_hotkey_start(Some(11434), false, true, "  "),
            HotkeyStart::Start
        );
    }

    #[tokio::test]
    async fn port_answers_detects_bound_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(port_answers(port).await);

        drop(listener);
        assert!(!port_answers(port).await);
    }

    /// 설치 위치가 PATH 를 이긴다 — 사용자가 runbook 대로 깔았으면 그것이 앱의 서버다.
    #[test]
    fn resolve_prefers_installed_over_path() {
        let home = std::env::temp_dir().join(format!("praxis-stt-test-{}", std::process::id()));
        let bin = home.join(".praxis-stt").join("bin").join(BIN);
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, b"").unwrap();

        let found = resolve_with(Some(&home), |_| Some(PathBuf::from("/usr/bin/ohr")));
        assert_eq!(found, Some(bin));

        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn resolve_falls_back_to_path_lookup() {
        let empty = std::env::temp_dir().join("praxis-stt-absent");
        assert_eq!(
            resolve_with(Some(&empty), |_| Some(PathBuf::from("/usr/bin/ohr"))),
            Some(PathBuf::from("/usr/bin/ohr"))
        );
        assert_eq!(resolve_with(Some(&empty), |_| None), None);
    }
}
