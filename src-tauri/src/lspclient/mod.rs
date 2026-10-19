//! IDE 에디터용 LSP 클라이언트 — 워크트리별로 언어 서버를 띄우고 "정의로 이동"에 답한다.
//!
//! 구조: 서버 프로세스 하나당 stdout 리더 스레드 하나가 붙어 응답을 id로 라우팅하고,
//! 요청자는 oneshot을 기다린다. 문서 동기화는 **점프 직전에 에디터 버퍼 전문을 통째로**
//! 밀어넣는 방식이다 — 키 입력마다 didChange를 보내지 않아도 저장 안 한 편집이 반영되고,
//! 점프는 초당 수십 번 일어나는 동작이 아니라 이 비용이 문제되지 않는다.

pub mod protocol;
pub mod server;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{oneshot, Notify};

pub use server::ServerSpec;

/// 점프 요청 한 건의 상한. rust-analyzer는 최초 인덱싱 중 느리게 답한다.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// 대형 Java 프로젝트의 최초 사용처 검색은 import 직후 20초를 넘길 수 있다.
const REFERENCES_TIMEOUT: Duration = Duration::from_secs(120);
const IDLE_TTL: Duration = Duration::from_secs(120);
const REAPER_INTERVAL: Duration = Duration::from_secs(15);
const MAX_CACHED_IDLE: usize = 2;
/// 진단용으로 붙잡아 둘 stderr 마지막 줄 수.
const STDERR_TAIL: usize = 20;

/// 아직 답을 못 받은 요청들 — 리더 스레드가 응답 id로 찾아 깨운다.
type PendingResponses = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>>;

/// 의미 분석 준비 대기의 결과. `Failed`가 `Err`가 아닌 이유는 설계 0065 DR-3이다 —
/// 한 서버의 실패가 런 전체를 죽이면 안 된다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    /// 기다릴 신호가 없다. 사유는 그대로 사용자에게 보인다.
    Unsupported(String),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReadyState {
    Pending,
    Ready,
    Failed(String),
}

/// 준비 신호 **래치** — `Pending`에서 한 번만 전이하고 되돌아오지 않는다.
///
/// 네 신호가 전부 에지 트리거 알림이라 최신값을 덮어쓰는 폴링형으로는 못 받는다.
/// tsserver의 워크스페이스 진행 토큰은 세션당 1개이고 end가 기동 294~762ms에 오는데,
/// 그때는 심볼 수집이 한참 남은 시점이다. 나중에 리셋하면 이미 도착한 end가 사라지고
/// 다시 오지 않는 알림을 상한까지 기다리다 심볼까지 전부 잃는다(설계 0065 DR-2b).
/// 그래서 리셋 API가 없다 — 래치의 수명은 클라이언트의 수명이다.
struct SemanticLatch {
    spec: ServerSpec,
    state: Mutex<ReadyState>,
    /// 상한 초과 사유에 담을 마지막 관측 — 없으면 알림이 하나도 오지 않은 것이다.
    last_seen: Mutex<Option<String>>,
    changed: Notify,
}

/// 준비 래치와 별개인 프로젝트 import 상태.
///
/// jdtls는 import가 끝나면 `language/status`의 `ProjectStatus`를 `OK` 또는 `WARNING`으로
/// 보낸다. **`WARNING`은 import 실패가 아니다** — `ProjectsManager.reportProjectsStatus`가
/// 워크스페이스의 최대 문제 심각도가 ERROR면 `WARNING`, 아니면 `OK`를 고르는 것뿐이라,
/// Lombok·미해결 의존성으로 컴파일 오류 마커가 있는 프로젝트는 거의 항상 `WARNING`이고
/// 그 뒤에도 정의·사용처 조회는 정상 동작한다. 실패로 다루면 멀쩡한 서버가 "언어 서버
/// 오류"로 굳어 사용처 조회가 막힌다. 그래서 사유를 `warning`에만 남기고 상태는 바꾸지 않는다.
///
/// 진짜 실패 신호는 둘이다 — jdtls의 `language/status` `Error`(InitHandler가 초기화 실패 때
/// 보낸다)와 rust-analyzer의 `experimental/serverStatus health=error`. `window/logMessage`·
/// `window/showMessage`는 판정에 쓰지 않는다. jdtls의 LogHandler는 플랫폼 로그의 ERROR
/// 항목을 전부 `type=1`로 흘려보내므로(JavaBuilder의 리소스 복사 예외 등) 하나만 받아도
/// 서버가 살아 있는 채로 영구히 실패로 굳는다.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ProjectHealth {
    Starting,
    Analyzing,
    Ready,
    Failed(String),
}

struct ProjectHealthState {
    state: Mutex<ProjectHealth>,
    /// 서버는 쓸 수 있지만 결과가 온전하지 않을 수 있는 사유 — 상태 표시의 보조 설명.
    warning: Mutex<Option<String>>,
}

impl ProjectHealthState {
    fn new() -> Self {
        Self {
            state: Mutex::new(ProjectHealth::Starting),
            warning: Mutex::new(None),
        }
    }

    fn record(&self, spec: &ServerSpec, method: &str, params: &Value) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => return,
        };
        match method {
            "language/status" if spec.key == "jdtls" => {
                let kind = params
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let message = params
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match kind {
                    "ProjectStatus" => {
                        if !message.eq_ignore_ascii_case("ok") {
                            self.note_warning(format!(
                                "jdtls가 프로젝트의 컴파일 오류를 보고했습니다 ({message}) — 해결되지 않은 심볼의 참조는 결과에서 빠질 수 있습니다"
                            ));
                        }
                        // 준비 뒤에 다시 오는 ProjectStatus(빌드 파일 변경 등)로 되돌리지 않는다 —
                        // ServiceReady는 다시 오지 않으므로 "분석 중"에 영영 머문다.
                        if *state == ProjectHealth::Starting {
                            *state = ProjectHealth::Analyzing;
                        }
                    }
                    "Error" => {
                        let reason = if message.is_empty() {
                            "jdtls 초기화 실패".to_string()
                        } else {
                            format!("jdtls 초기화 실패: {message}")
                        };
                        *state = ProjectHealth::Failed(reason);
                    }
                    "ServiceReady" if !matches!(*state, ProjectHealth::Failed(_)) => {
                        *state = ProjectHealth::Ready;
                    }
                    _ => {}
                }
            }
            "experimental/serverStatus" => {
                let quiescent = params
                    .get("quiescent")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let health = params
                    .get("health")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if health == "error" {
                    let message = params
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("LSP 서버 오류");
                    *state = ProjectHealth::Failed(message.into());
                } else if health == "ok" && quiescent {
                    *state = ProjectHealth::Ready;
                } else {
                    *state = ProjectHealth::Analyzing;
                }
            }
            "$/progress" if spec.readiness == server::ReadinessSignal::Progress => {
                if matches!(*state, ProjectHealth::Failed(_)) {
                    return;
                }
                if protocol::parse_progress_kind(params).as_deref() == Some("end") {
                    *state = ProjectHealth::Ready;
                } else if !matches!(*state, ProjectHealth::Failed(_)) {
                    *state = ProjectHealth::Analyzing;
                }
            }
            _ => {}
        }
    }

    fn phase(&self) -> ProjectHealth {
        self.state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_else(|_| ProjectHealth::Failed("LSP 프로젝트 상태 잠금 실패".into()))
    }

    fn note_warning(&self, detail: String) {
        if let Ok(mut warning) = self.warning.lock() {
            *warning = Some(detail);
        }
    }

    fn warning(&self) -> Option<String> {
        self.warning.lock().ok().and_then(|warning| warning.clone())
    }
}

impl SemanticLatch {
    fn new(spec: ServerSpec) -> Self {
        Self {
            spec,
            state: Mutex::new(ReadyState::Pending),
            last_seen: Mutex::new(None),
            changed: Notify::new(),
        }
    }

    /// 리더 스레드가 부른다. 이 클라이언트가 쓰지 않는 신호는 버린다 — pyright는
    /// `references` 요청마다 진행 토큰을 보내 90초에 861개가 온다.
    fn record(&self, method: &str, params: &Value) {
        match (self.spec.readiness, method) {
            (server::ReadinessSignal::ServerStatus, "experimental/serverStatus") => {
                self.record_server_status(params)
            }
            (server::ReadinessSignal::LanguageStatus, "language/status") => {
                self.record_language_status(params)
            }
            (server::ReadinessSignal::Progress, "$/progress") => self.record_progress(params),
            _ => {}
        }
    }

    fn record_server_status(&self, params: &Value) {
        let Some(status) = protocol::parse_server_status(params) else {
            return;
        };
        if status.is_ready() {
            return self.latch(ReadyState::Ready);
        }
        if status.health == "error" {
            let reason = status
                .message
                .clone()
                .unwrap_or_else(|| format!("{}이(가) 오류 상태를 보고했습니다", self.spec.key));
            return self.latch(ReadyState::Failed(reason));
        }
        self.note(format!(
            "health={}, quiescent={}",
            status.health, status.quiescent
        ));
    }

    fn record_language_status(&self, params: &Value) {
        let Some(status) = protocol::parse_language_status(params) else {
            return;
        };
        if status.is_ready() {
            return self.latch(ReadyState::Ready);
        }
        if status.kind == "Error" {
            // InitHandler가 초기화 실패 때 보낸다 — 여기서 끝내지 않으면 오지 않을
            // ServiceReady를 상한까지 기다린다.
            let reason = match status.message.as_deref() {
                Some(message) if !message.is_empty() => format!("jdtls 초기화 실패: {message}"),
                _ => "jdtls 초기화 실패".to_string(),
            };
            return self.latch(ReadyState::Failed(reason));
        }
        self.note(format!("language/status={}", status.kind));
    }

    fn record_progress(&self, params: &Value) {
        let Some(kind) = protocol::parse_progress_kind(params) else {
            return;
        };
        if kind == "end" {
            return self.latch(ReadyState::Ready);
        }
        self.note(format!("$/progress={kind}"));
    }

    /// `Pending`에서만 전이한다.
    fn latch(&self, next: ReadyState) {
        if let Ok(mut state) = self.state.lock() {
            if *state != ReadyState::Pending {
                return;
            }
            *state = next;
        }
        self.changed.notify_waiters();
    }

    fn note(&self, detail: String) {
        if let Ok(mut last) = self.last_seen.lock() {
            *last = Some(detail);
        }
    }

    async fn wait(&self) -> Readiness {
        let timeout = Duration::from_secs(self.spec.ready_timeout_secs);
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let changed = self.changed.notified();
            match self.state.lock().map(|s| s.clone()) {
                Ok(ReadyState::Ready) => return Readiness::Ready,
                Ok(ReadyState::Failed(reason)) => return Readiness::Failed(reason),
                Ok(ReadyState::Pending) => {}
                Err(_) => return Readiness::Failed("LSP 준비 상태 잠금 실패".to_string()),
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if tokio::time::timeout(remaining, changed).await.is_err() {
                return Readiness::Failed(self.timeout_detail(timeout));
            }
        }
    }

    fn timeout_detail(&self, timeout: Duration) -> String {
        let last = self.last_seen.lock().ok().and_then(|s| s.clone());
        match last {
            Some(detail) => format!(
                "{} 의미 분석이 {}초 안에 준비되지 않았습니다 (마지막 상태: {detail})",
                self.spec.key,
                timeout.as_secs()
            ),
            None => format!(
                "{}이(가) {}초 안에 준비 알림을 보내지 않았습니다",
                self.spec.key,
                timeout.as_secs()
            ),
        }
    }
}

/// 기다릴 신호가 없는 서버의 사유 — 심볼은 온전하고 엣지만 없다.
fn unsupported_reason(spec: &ServerSpec) -> Option<String> {
    if spec.readiness != server::ReadinessSignal::Unsupported {
        return None;
    }
    Some(format!(
        "{}는 의미 분석 완료를 알리는 신호가 없어 참조 관계를 만들지 않았습니다",
        spec.key
    ))
}

fn server_settings(spec: &ServerSpec, root: &Path) -> Result<Value, String> {
    if spec.key != "jdtls" {
        return Ok(json!({}));
    }
    let jdk = server::resolve_java_import_jdk(root)?;
    Ok(json!({
        "java": {
            "import": { "gradle": { "java": { "home": jdk.home } } }
        }
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigStamp {
    name: &'static str,
    exists: bool,
    length: u64,
    modified_nanos: u128,
}

#[derive(Debug, Clone)]
struct ConfigCacheEntry {
    stamps: Vec<ConfigStamp>,
    content_hash: String,
}

async fn config_identity(
    inner: &LspPoolInner,
    spec: &ServerSpec,
    root: &Path,
) -> Result<String, String> {
    let settings = server_settings(spec, root)?;
    let stamps = config_stamps(spec, root);
    let cache_key = (root.to_path_buf(), spec.key);
    let content_hash = {
        let cache = inner.config_cache.lock().await;
        cache
            .get(&cache_key)
            .filter(|entry| entry.stamps == stamps)
            .map(|entry| entry.content_hash.clone())
    };
    let content_hash = match content_hash {
        Some(hash) => hash,
        None => {
            let hash = config_content_hash(spec, root)?;
            inner.config_cache.lock().await.insert(
                cache_key,
                ConfigCacheEntry {
                    stamps,
                    content_hash: hash.clone(),
                },
            );
            hash
        }
    };
    let mut hasher = Sha256::new();
    hasher.update(spec.key.as_bytes());
    hasher.update(root.to_string_lossy().as_bytes());
    hasher.update(settings.to_string().as_bytes());
    hasher.update(content_hash.as_bytes());
    Ok(format!("{:x}", hasher.finalize()))
}

fn config_stamps(spec: &ServerSpec, root: &Path) -> Vec<ConfigStamp> {
    spec.config_files
        .iter()
        .map(|name| {
            let metadata = std::fs::metadata(root.join(name)).ok();
            let modified_nanos = metadata
                .as_ref()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |duration| duration.as_nanos());
            ConfigStamp {
                name,
                exists: metadata.is_some(),
                length: metadata.as_ref().map_or(0, std::fs::Metadata::len),
                modified_nanos,
            }
        })
        .collect()
}

fn config_content_hash(spec: &ServerSpec, root: &Path) -> Result<String, String> {
    let mut hasher = Sha256::new();
    for name in spec.config_files {
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        let path = root.join(name);
        match std::fs::read(&path) {
            Ok(bytes) => {
                hasher.update([1]);
                hasher.update((bytes.len() as u64).to_le_bytes());
                hasher.update(bytes);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hasher.update([0]);
            }
            Err(error) => {
                return Err(format!(
                    "LSP 설정 파일을 읽지 못했습니다 ({}): {error}",
                    path.display()
                ))
            }
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn jdtls_data_dir(root: &Path, task_id: i64, config_identity: &str) -> Result<PathBuf, String> {
    let cache = std::env::var_os("DOJANG_LSP_JAVA_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(lsp_cache_root);
    let path = jdtls_data_path(&cache, root, task_id, config_identity);
    std::fs::create_dir_all(&path).map_err(|e| {
        format!(
            "jdtls 캐시 디렉터리를 만들지 못했습니다 ({}): {e}",
            path.display()
        )
    })?;
    Ok(path)
}

fn jdtls_data_path(cache: &Path, root: &Path, task_id: i64, config_identity: &str) -> PathBuf {
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    hasher.update(task_id.to_le_bytes());
    hasher.update(config_identity.as_bytes());
    cache.join("jdtls").join(format!("{:x}", hasher.finalize()))
}

fn lsp_cache_root() -> PathBuf {
    if let Some(cache) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(cache).join("praxis/lsp");
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join("Library/Caches/Praxis/lsp");
    }
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("Praxis/lsp");
    }
    std::env::temp_dir().join("praxis/lsp")
}

/// 사용자가 요청한 이동 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GotoKind {
    Definition,
    Implementation,
    References,
}

impl GotoKind {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "definition" => Ok(Self::Definition),
            "implementation" => Ok(Self::Implementation),
            "references" => Ok(Self::References),
            other => Err(format!("알 수 없는 이동 종류입니다: {other}")),
        }
    }

    fn method(self) -> &'static str {
        match self {
            Self::Definition => "textDocument/definition",
            Self::Implementation => "textDocument/implementation",
            Self::References => "textDocument/references",
        }
    }
}

/// 프론트로 돌려주는 이동 대상. 좌표는 **Monaco 기준(1-based)** 으로 변환해 넘긴다 —
/// 변환을 한곳(여기)에 몰아 두어야 UI에서 off-by-one이 재발하지 않는다.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LspTarget {
    /// 워크트리 상대 경로. 워크트리 밖이면 `None`이고 `abs_path`만 채워진다.
    pub path: Option<String>,
    pub abs_path: String,
    pub line: u32,
    pub column: u32,
    /// 워크트리 밖(의존성·표준 라이브러리) 여부 — 탭으로 못 열고 안내만 한다.
    pub external: bool,
}

/// 현재 파일에 대해 LSP를 쓸 수 있는지.
#[derive(Debug, Clone, Serialize)]
pub struct LspStatus {
    pub available: bool,
    /// 서버 표기명 (없으면 지원 언어가 아님).
    pub server: Option<String>,
    /// 설치/연결/분석 상태. available은 설치 가능 여부만 뜻한다.
    pub phase: String,
    /// 사용 불가 사유 — UI가 그대로 보여준다.
    pub detail: Option<String>,
}

/// 시맨틱 토큰 legend — 타입·수식자 이름의 **순서**가 곧 인덱스다.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct SemanticLegend {
    pub token_types: Vec<String>,
    pub token_modifiers: Vec<String>,
}

/// 시맨틱 토큰 응답 — 5-tuple 델타 인코딩된 `data`를 **디코딩하지 않고 그대로** 넘긴다.
/// Monaco의 `DocumentSemanticTokensProvider`가 같은 인코딩을 기대하므로 중간에서 풀면
/// 다시 감아야 한다.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct SemanticTokens {
    pub data: Vec<u32>,
    pub legend: SemanticLegend,
}

pub struct LspClient {
    spec: ServerSpec,
    root: PathBuf,
    child: Mutex<Child>,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: PendingResponses,
    next_id: AtomicI64,
    /// uri → 마지막으로 보낸 문서 버전.
    docs: Mutex<HashMap<String, i64>>,
    /// 서버가 `initialize` 응답에서 선언한 시맨틱 토큰 legend.
    ///
    /// **응답에서만 얻을 수 있다** — 토큰 데이터는 legend 배열의 **인덱스**로 오므로,
    /// 이것이 없으면 숫자 배열을 해석할 방법이 없다. 서버가 지원하지 않으면 `None`이고,
    /// 그때는 프론트가 프로바이더를 아예 등록하지 않아야 한다(빈 프로바이더는 Monarch
    /// 결과까지 덮어 색이 오히려 사라진다).
    semantic_legend: Mutex<Option<SemanticLegend>>,
    readiness: Arc<SemanticLatch>,
    health: Arc<ProjectHealthState>,
    settings: Value,
    last_used: Mutex<Instant>,
    stderr_tail: Arc<Mutex<Vec<String>>>,
}

impl LspClient {
    /// 서버를 띄우고 initialize 핸드셰이크까지 마친다.
    async fn spawn(
        task_id: i64,
        spec: ServerSpec,
        root: &Path,
        config_identity: &str,
    ) -> Result<Self, String> {
        // Windows에서 npm으로 깐 언어 서버는 `.cmd` shim이라 맨 이름으로는 실행되지 않는다.
        let program =
            crate::reviewer::which(spec.command).unwrap_or_else(|| spec.command.to_string());
        let mut command = Command::new(program);
        command
            .args(spec.args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let settings = server_settings(&spec, root)?;
        if spec.key == "jdtls" {
            let runtime = server::resolve_jdtls_runtime()?;
            let data = jdtls_data_dir(root, task_id, config_identity)?;
            command
                .arg("--java-executable")
                .arg(runtime)
                .arg("--jvm-arg=-Xms128m")
                .arg("-data")
                .arg(data);
        }
        set_process_group(&mut command);

        let mut child = command
            .spawn()
            .map_err(|e| format!("{} 실행 실패: {e}", spec.command))?;

        let stdin = child.stdin.take().ok_or("LSP stdin을 열지 못했습니다")?;
        let stdout = child.stdout.take().ok_or("LSP stdout을 열지 못했습니다")?;
        let stderr = child.stderr.take().ok_or("LSP stderr을 열지 못했습니다")?;

        let stdin = Arc::new(Mutex::new(stdin));
        let pending: PendingResponses = Arc::new(Mutex::new(HashMap::new()));
        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        // 래치는 initialize 이전에 리더 스레드에 물린다 — 그래야 기동 직후 오는
        // 에지 트리거 알림을 하나도 놓치지 않는다.
        let readiness = Arc::new(SemanticLatch::new(spec));
        let health = Arc::new(ProjectHealthState::new());

        spawn_reader(
            stdout,
            Arc::clone(&pending),
            Arc::clone(&stdin),
            Arc::clone(&readiness),
            Arc::clone(&health),
            settings.clone(),
        );
        spawn_stderr_drain(stderr, Arc::clone(&stderr_tail));

        let client = Self {
            spec,
            root: root.to_path_buf(),
            child: Mutex::new(child),
            stdin,
            pending,
            next_id: AtomicI64::new(1),
            docs: Mutex::new(HashMap::new()),
            semantic_legend: Mutex::new(None),
            readiness,
            health,
            settings,
            last_used: Mutex::new(Instant::now()),
            stderr_tail,
        };
        client.initialize().await?;
        Ok(client)
    }

    async fn initialize(&self) -> Result<(), String> {
        let params = initialize_params(&self.spec, &self.root, &self.settings);
        let timeout = Duration::from_secs(self.spec.init_timeout_secs);
        let result = self.request("initialize", params, timeout).await?;
        // legend는 이 응답에만 있다. 놓치면 토큰 데이터의 숫자를 해석할 방법이 사라진다.
        if let Some(legend) = result
            .get("capabilities")
            .and_then(|c| c.get("semanticTokensProvider"))
            .and_then(|p| p.get("legend"))
        {
            let names = |key: &str| -> Vec<String> {
                legend
                    .get(key)
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let parsed = SemanticLegend {
                token_types: names("tokenTypes"),
                token_modifiers: names("tokenModifiers"),
            };
            // 타입이 하나도 없는 legend는 없는 것과 같다 — 그대로 두면 프론트가
            // 쓸모없는 프로바이더를 등록해 Monarch 색까지 덮는다.
            if !parsed.token_types.is_empty() {
                if let Ok(mut slot) = self.semantic_legend.lock() {
                    *slot = Some(parsed);
                }
            }
        }
        self.notify("initialized", json!({}))?;
        // Pyright는 이 알림 뒤에 workspace/configuration을 요청해 프로젝트 설정을 읽는다.
        self.notify(
            "workspace/didChangeConfiguration",
            json!({ "settings": self.settings }),
        )
    }

    /// 에디터의 현재 버퍼를 서버에 반영한다. 처음 보는 문서면 didOpen, 아니면 full didChange.
    fn sync_document(&self, uri: &str, language_id: &str, text: &str) -> Result<(), String> {
        let version = {
            let mut docs = self.docs.lock().map_err(|_| "LSP 문서 상태 잠금 실패")?;
            let entry = docs.entry(uri.to_string()).or_insert(0);
            *entry += 1;
            *entry
        };
        if version == 1 {
            return self.notify(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": uri,
                        "languageId": language_id,
                        "version": version,
                        "text": text,
                    }
                }),
            );
        }
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [{ "text": text }],
            }),
        )
    }

    async fn goto(
        &self,
        uri: &str,
        kind: GotoKind,
        line: u32,
        character: u32,
    ) -> Result<Vec<protocol::RawLocation>, String> {
        let mut params = json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
        });
        if kind == GotoKind::References {
            // 선언 자체는 빼야 "사용처"가 된다 — 넣으면 결과 첫 줄이 늘 제자리다.
            params["context"] = json!({ "includeDeclaration": false });
        }
        let timeout = if kind == GotoKind::References {
            REFERENCES_TIMEOUT
        } else {
            REQUEST_TIMEOUT
        };
        let result = self.request(kind.method(), params, timeout).await?;
        Ok(protocol::parse_locations(&result))
    }

    /// 파일 하나의 심볼 목록.
    ///
    /// `goto`와 달리 **위치 인자가 없다** — 커서가 아니라 파일이 질의 단위다. 그래서
    /// `GotoKind`("커서 위치 → 이동 대상" 모델)에 넣지 않고 별도 경로로 둔다.
    async fn document_symbols(&self, uri: &str) -> Result<Vec<protocol::RawSymbol>, String> {
        let result = self
            .request(
                "textDocument/documentSymbol",
                json!({ "textDocument": { "uri": uri } }),
                REQUEST_TIMEOUT,
            )
            .await?;
        Ok(protocol::parse_document_symbols(&result))
    }

    /// `textDocument/semanticTokens/full`. legend가 없으면(서버 미지원) `None`.
    async fn semantic_tokens(&self, uri: &str) -> Result<Option<SemanticTokens>, String> {
        let Some(legend) = self.semantic_legend.lock().ok().and_then(|g| g.clone()) else {
            return Ok(None);
        };
        let result = self
            .request(
                "textDocument/semanticTokens/full",
                json!({ "textDocument": { "uri": uri } }),
                REQUEST_TIMEOUT,
            )
            .await?;
        let data = result
            .get("data")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u32))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Some(SemanticTokens { data, legend }))
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().map_err(|_| "LSP 요청 큐 잠금 실패")?;
            pending.insert(id, tx);
        }
        let payload = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(e) = write_message(&self.stdin, &payload) {
            self.pending.lock().ok().and_then(|mut p| p.remove(&id));
            return Err(e);
        }

        let pending = PendingRequest::new(id, Arc::clone(&self.pending), Arc::clone(&self.stdin));
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => {
                pending.complete();
                result
            }
            // 리더 스레드가 sender를 떨군다 = 서버가 죽었다.
            Ok(Err(_)) => Err(self.death_detail()),
            Err(_) => {
                pending.cancel();
                Err(format!(
                    "{} 응답이 {}초 안에 오지 않았습니다 — 인덱싱 중일 수 있습니다",
                    self.spec.key,
                    timeout.as_secs()
                ))
            }
        }
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        let payload = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        write_message(&self.stdin, &payload)
    }

    /// 서버가 죽었을 때 stderr 꼬리를 붙여 원인을 보여준다 (빈손 에러보다 훨씬 낫다).
    fn death_detail(&self) -> String {
        let tail = self
            .stderr_tail
            .lock()
            .map(|t| t.join("\n"))
            .unwrap_or_default();
        if tail.trim().is_empty() {
            return format!("{}이(가) 종료됐습니다", self.spec.key);
        }
        format!("{}이(가) 종료됐습니다:\n{tail}", self.spec.key)
    }

    fn is_alive(&self) -> bool {
        self.child
            .lock()
            .map(|mut c| matches!(c.try_wait(), Ok(None)))
            .unwrap_or(false)
    }

    /// 명시적 worktree/window 종료는 Arc 대여 중이어도 즉시 프로세스를 끊는다. 대기 중인
    /// 요청은 stdout EOF로 깨우며, 이후 Arc가 떨어질 때까지 기다리지 않는다.
    fn shutdown(&self) {
        if let Ok(mut child) = self.child.lock() {
            crate::verify::kill_group(child.id());
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn touch(&self) {
        if let Ok(mut used) = self.last_used.lock() {
            *used = Instant::now();
        }
    }

    fn idle_for(&self) -> Duration {
        self.last_used
            .lock()
            .map(|used| used.elapsed())
            .unwrap_or_default()
    }

    async fn wait_java_ready(&self) -> Result<(), String> {
        if self.spec.key != "jdtls" {
            return Ok(());
        }
        match self.readiness.wait().await {
            Readiness::Ready => match self.health.phase() {
                ProjectHealth::Failed(reason) => Err(reason),
                _ => Ok(()),
            },
            Readiness::Failed(reason) => Err(reason),
            Readiness::Unsupported(reason) => Err(reason),
        }
    }
}

struct PendingRequest {
    id: i64,
    pending: PendingResponses,
    stdin: Arc<Mutex<ChildStdin>>,
    active: AtomicBool,
}

impl PendingRequest {
    fn new(id: i64, pending: PendingResponses, stdin: Arc<Mutex<ChildStdin>>) -> Self {
        Self {
            id,
            pending,
            stdin,
            active: AtomicBool::new(true),
        }
    }

    fn cancel(&self) {
        if self.active.swap(false, Ordering::SeqCst) {
            self.pending
                .lock()
                .ok()
                .and_then(|mut p| p.remove(&self.id));
            let _ = write_message(
                &self.stdin,
                &json!({
                    "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": self.id }
                }),
            );
        }
    }

    fn complete(&self) {
        self.active.store(false, Ordering::SeqCst);
    }
}

impl Drop for PendingRequest {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        // exit 노티는 생략한다 — 어차피 프로세스 그룹을 죽이고, 정상 종료를 기다리면
        // 앱 종료가 서버 응답에 묶인다.
        self.shutdown();
    }
}

/// `initialize`의 capability 선언 — **서버별로 다르다.**
///
/// `window.workDoneProgress`를 선언하지 않으면 서버가 `$/progress`를 보낼 수 없어
/// `Progress` 신호가 성립하지 않는다. `hierarchicalDocumentSymbolSupport`를 빼면
/// rust-analyzer가 구형 `SymbolInformation`으로 폴백해 좌표가 본문 시작으로 떨어지고
/// `references`가 0건이 된다(ADR 0099).
fn initialize_params(spec: &ServerSpec, root: &Path, settings: &Value) -> Value {
    let uri = protocol::path_to_uri(root);
    let mut capabilities = json!({
        "window": { "workDoneProgress": true },
        "textDocument": {
            "synchronization": { "didSave": false, "dynamicRegistration": false },
            "definition": { "linkSupport": true },
            "implementation": { "linkSupport": true },
            "references": {},
            "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
            "semanticTokens": {
                "requests": { "full": true, "range": false },
                "tokenTypes": [],
                "tokenModifiers": [],
                "formats": ["relative"],
                "overlappingTokenSupport": false,
                "multilineTokenSupport": false,
            },
        },
        "workspace": { "workspaceFolders": true, "configuration": true },
    });
    if spec.readiness == server::ReadinessSignal::ServerStatus {
        capabilities["experimental"] = json!({ "serverStatusNotification": true });
    }
    json!({
        "processId": std::process::id(),
        "rootUri": uri,
        "workspaceFolders": [{
            "uri": uri,
            "name": root.file_name().and_then(|n| n.to_str()).unwrap_or("workspace"),
        }],
        "initializationOptions": { "settings": settings },
        "capabilities": capabilities,
    })
}

fn write_message(stdin: &Arc<Mutex<ChildStdin>>, payload: &Value) -> Result<(), String> {
    let encoded = protocol::encode_message(&payload.to_string());
    let mut guard = stdin.lock().map_err(|_| "LSP stdin 잠금 실패")?;
    guard
        .write_all(&encoded)
        .and_then(|_| guard.flush())
        .map_err(|e| format!("LSP 요청 전송 실패: {e}"))
}

/// stdout 리더 — 응답은 id로 깨우고, 서버가 보낸 **요청**에는 반드시 답한다.
fn spawn_reader(
    stdout: std::process::ChildStdout,
    pending: PendingResponses,
    stdin: Arc<Mutex<ChildStdin>>,
    readiness: Arc<SemanticLatch>,
    health: Arc<ProjectHealthState>,
    settings: Value,
) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        while let Ok(Some(buf)) = protocol::read_message(&mut reader) {
            let Ok(value) = serde_json::from_slice::<Value>(&buf) else {
                continue;
            };
            let id = value.get("id").and_then(Value::as_i64);
            if let Some(method) = value.get("method").and_then(Value::as_str) {
                // method + id = 서버→클라이언트 요청. 답하지 않으면 pyright 등은
                // workspace/configuration 응답을 기다리다 초기화가 멈춘다.
                if let Some(id) = id {
                    answer_server_request(&stdin, id, method, &value, &settings);
                } else if let Some(params) = value.get("params") {
                    readiness.record(method, params);
                    health.record(&readiness.spec, method, params);
                }
                continue;
            }
            let Some(id) = id else { continue };
            let Some(tx) = pending.lock().ok().and_then(|mut p| p.remove(&id)) else {
                continue;
            };
            let outcome = match value.get("error") {
                Some(error) => Err(lsp_error_message(error)),
                None => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
            };
            let _ = tx.send(outcome);
        }
        // 스트림이 끝났다 = 서버 종료. 대기 중인 요청의 sender를 떨궈 즉시 깨운다.
        if let Ok(mut p) = pending.lock() {
            p.clear();
        }
    });
}

fn answer_server_request(
    stdin: &Arc<Mutex<ChildStdin>>,
    id: i64,
    method: &str,
    request: &Value,
    settings: &Value,
) {
    // 일부 서버는 initialize 설정을 읽지 않고 이 요청에서만 workspace 설정을 조회한다.
    let result = if method == "workspace/configuration" {
        let items = request
            .get("params")
            .and_then(|p| p.get("items"))
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        Value::Array(
            items
                .iter()
                .map(|item| {
                    item.get("section")
                        .and_then(Value::as_str)
                        .and_then(|section| setting_for_section(settings, section))
                        .unwrap_or_else(|| settings.clone())
                })
                .collect(),
        )
    } else {
        Value::Null
    };
    let _ = write_message(
        stdin,
        &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
    );
}

fn setting_for_section(settings: &Value, section: &str) -> Option<Value> {
    let mut cursor = settings;
    for component in section.split('.') {
        cursor = cursor.get(component)?;
    }
    Some(cursor.clone())
}

fn lsp_error_message(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("LSP 서버가 오류를 반환했습니다")
        .to_string()
}

/// stderr을 계속 비운다 — 안 읽으면 파이프가 차서 서버가 write에서 멈춘다.
fn spawn_stderr_drain(stderr: std::process::ChildStderr, tail: Arc<Mutex<Vec<String>>>) {
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Ok(mut tail) = tail.lock() {
                tail.push(line);
                let overflow = tail.len().saturating_sub(STDERR_TAIL);
                if overflow > 0 {
                    tail.drain(0..overflow);
                }
            }
        }
    });
}

fn set_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ClientKey {
    task_id: i64,
    root: PathBuf,
    server: &'static str,
    config_identity: String,
}

#[derive(Default)]
struct LspPoolInner {
    clients: tokio::sync::Mutex<HashMap<ClientKey, Arc<LspClient>>>,
    starts: tokio::sync::Mutex<HashMap<ClientKey, Arc<tokio::sync::Mutex<()>>>>,
    starting: tokio::sync::Mutex<HashMap<ClientKey, Arc<StartingState>>>,
    failures: tokio::sync::Mutex<HashMap<ClientKey, String>>,
    config_cache: tokio::sync::Mutex<HashMap<(PathBuf, &'static str), ConfigCacheEntry>>,
    /// shutdown과 세대 확인·cache 삽입만 짧게 직렬화한다. JVM 부팅은 이 잠금을 잡지 않는다.
    lifecycle: tokio::sync::Mutex<()>,
    task_epochs: tokio::sync::Mutex<HashMap<i64, u64>>,
    all_epoch: AtomicI64,
    terminal_shutdown: AtomicBool,
    #[cfg(test)]
    before_first_cache_read: tokio::sync::Mutex<Option<Arc<TestCacheReadGate>>>,
    reaper_active: AtomicBool,
}

#[cfg(test)]
struct TestCacheReadGate {
    entered: tokio::sync::Barrier,
    release: tokio::sync::Barrier,
}

struct StartingState {
    cancelled: AtomicBool,
    cancelled_changed: Notify,
    completed: AtomicBool,
    completed_changed: Notify,
}

impl StartingState {
    fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            cancelled_changed: Notify::new(),
            completed: AtomicBool::new(false),
            completed_changed: Notify::new(),
        }
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.cancelled_changed.notify_waiters();
    }

    fn complete(&self) {
        self.completed.store(true, Ordering::SeqCst);
        self.completed_changed.notify_waiters();
    }

    async fn wait_complete(&self) {
        loop {
            let notified = self.completed_changed.notified();
            if self.completed.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
    }
}

struct StartingCompletion {
    state: Arc<StartingState>,
}

impl Drop for StartingCompletion {
    fn drop(&mut self) {
        // outer client_for future가 취소되어도 initialize future와 LspClient가 먼저 drop된 뒤
        // shutdown waiter를 깨운다.
        self.state.complete();
    }
}

/// 작업 × 실제 언어 프로젝트 × 설정별 서버 풀. 같은 서버 프로세스를 공유하는 것은
/// 동일 worktree와 동일 설정뿐이므로 다른 dirty 버퍼가 섞이지 않는다.
#[derive(Clone, Default)]
pub struct LspPool {
    inner: Arc<LspPoolInner>,
}

impl LspPool {
    /// 정의/구현/사용처 조회. `text`는 에디터의 현재 버퍼(미저장 편집 포함)다.
    ///
    /// `line`/`column`은 Monaco 좌표(1-based)로 받아 LSP 좌표(0-based)로 낮췄다가,
    /// 결과를 다시 Monaco 좌표로 올려 돌려준다.
    #[allow(clippy::too_many_arguments)]
    pub async fn goto(
        &self,
        task_id: i64,
        worktree: &Path,
        rel_path: &str,
        text: &str,
        line: u32,
        column: u32,
        kind: GotoKind,
    ) -> Result<Vec<LspTarget>, String> {
        let abs = worktree.join(rel_path);
        let (spec, language_id) =
            server::spec_for_path(&abs).ok_or("이 파일 형식은 정의 이동을 지원하지 않습니다")?;
        let root = server::resolve_root(&spec, worktree, &abs)?;
        if let Some(reason) = server::unavailable_reason(&spec, &root) {
            return Err(reason);
        }

        let client = self.client_for(task_id, &root, spec).await?;
        client.touch();
        let result = async {
            client.wait_java_ready().await?;
            let uri = protocol::path_to_uri(&abs);
            client.sync_document(&uri, language_id, text)?;
            let locations = client
                .goto(&uri, kind, line.saturating_sub(1), column.saturating_sub(1))
                .await?;
            Ok(locations
                .into_iter()
                .filter_map(|loc| to_target(&loc, worktree))
                .collect())
        }
        .await;
        client.touch();
        drop(client);
        self.reap().await;
        result
    }

    /// 파일 하나의 시맨틱 토큰 — Monaco의 `DocumentSemanticTokensProvider`가 그대로 먹는다.
    ///
    /// `goto`와 마찬가지로 **에디터의 현재 버퍼**를 동기화한 뒤 묻는다. 디스크를 읽으면
    /// 저장 전 편집에서 색이 한 박자 늦는다.
    ///
    /// 서버가 지원하지 않으면 `Ok(None)`이다 — 에러가 아니다. 언어 서버가 안 붙은 환경은
    /// 정상이고, 그때는 Monarch 색으로 후퇴하면 된다.
    pub async fn semantic_tokens(
        &self,
        task_id: i64,
        worktree: &Path,
        rel_path: &str,
        text: &str,
    ) -> Result<Option<SemanticTokens>, String> {
        let abs = worktree.join(rel_path);
        let Some((spec, language_id)) = server::spec_for_path(&abs) else {
            return Ok(None);
        };
        let Ok(root) = server::resolve_root(&spec, worktree, &abs) else {
            return Ok(None);
        };
        if server::unavailable_reason(&spec, &root).is_some() {
            return Ok(None);
        }
        let client = self.client_for(task_id, &root, spec).await?;
        client.touch();
        let result = async {
            let uri = protocol::path_to_uri(&abs);
            client.sync_document(&uri, language_id, text)?;
            client.semantic_tokens(&uri).await
        }
        .await;
        client.touch();
        drop(client);
        self.reap().await;
        result
    }

    /// 파일 하나의 심볼 목록 — 코드 그래프 인덱싱의 수집 경로.
    ///
    /// `goto`와 셋이 다르다. ① 위치 인자가 없다 ② 좌표를 **LSP 원본(0-based)** 그대로 돌려준다
    /// — 그래프에 그대로 저장되고 UI 경계에서만 올린다(변환을 두 곳에서 하면 off-by-one이
    /// 재발한다) ③ `text`를 인자로 받지 않고 디스크에서 읽는다. 인덱싱은 에디터 버퍼가 아니라
    /// 워크트리의 현재 상태를 대상으로 하기 때문이다.
    pub async fn document_symbols(
        &self,
        task_id: i64,
        worktree: &Path,
        rel_path: &str,
    ) -> Result<Vec<protocol::RawSymbol>, String> {
        let abs = worktree.join(rel_path);
        let (spec, language_id) =
            server::spec_for_path(&abs).ok_or("이 파일 형식은 심볼 수집을 지원하지 않습니다")?;
        let root = server::resolve_root(&spec, worktree, &abs)?;
        if let Some(reason) = server::unavailable_reason(&spec, &root) {
            return Err(reason);
        }
        let text = std::fs::read_to_string(&abs).map_err(|e| format!("{}: {e}", abs.display()))?;

        let client = self.client_for(task_id, &root, spec).await?;
        client.touch();
        let result = async {
            let uri = protocol::path_to_uri(&abs);
            client.sync_document(&uri, language_id, &text)?;
            client.document_symbols(&uri).await
        }
        .await;
        client.touch();
        drop(client);
        self.reap().await;
        result
    }

    /// 이 서버의 의미 분석이 준비될 때까지 기다린다.
    ///
    /// 기다릴 신호가 없는 서버는 **띄우지도 않고** 즉시 `Unsupported`를 돌려준다 —
    /// 상한까지 기다려 봐야 오지 않을 알림이고, 그 시간에 엣지뿐 아니라 심볼까지 잃는다.
    pub async fn wait_semantic_ready(
        &self,
        task_id: i64,
        worktree: &Path,
        spec: ServerSpec,
    ) -> Readiness {
        if let Some(reason) = unsupported_reason(&spec) {
            return Readiness::Unsupported(reason);
        }
        let root = match server::resolve_root(&spec, worktree, worktree) {
            Ok(root) => root,
            Err(reason) => return Readiness::Failed(reason),
        };
        match self.client_for(task_id, &root, spec).await {
            Ok(client) => client.readiness.wait().await,
            Err(reason) => Readiness::Failed(reason),
        }
    }

    /// 풀에서 살아 있는 클라이언트를 꺼낸다. 죽었으면 버리고 `None`.
    async fn live_client(&self, key: &ClientKey) -> Option<Arc<LspClient>> {
        let mut clients = self.inner.clients.lock().await;
        let existing = clients.get(key)?;
        if existing.is_alive() {
            return Some(Arc::clone(existing));
        }
        clients.remove(key);
        None
    }

    /// 동일 키의 시작은 합치되, JVM 부팅 동안 전역 풀 가드는 잡지 않는다.
    async fn client_for(
        &self,
        task_id: i64,
        worktree: &Path,
        spec: ServerSpec,
    ) -> Result<Arc<LspClient>, String> {
        let root = worktree
            .canonicalize()
            .unwrap_or_else(|_| worktree.to_path_buf());
        // 종료 세대는 config I/O보다 먼저 고정한다. shutdown 뒤에는 stale 조회가 기존
        // cache client를 돌려주거나 새 JVM을 시작할 수 없다.
        let (task_epoch, all_epoch) = {
            let _lifecycle = self.inner.lifecycle.lock().await;
            if self.inner.terminal_shutdown.load(Ordering::SeqCst) {
                return Err("앱 종료로 LSP 서버 시작을 중단했습니다".into());
            }
            self.generation_locked(task_id).await
        };
        #[cfg(test)]
        self.pause_before_first_cache_read().await;
        let config_identity = config_identity(&self.inner, &spec, &root).await?;
        let key = ClientKey {
            task_id,
            root: root.clone(),
            server: spec.key,
            config_identity,
        };
        {
            let _lifecycle = self.inner.lifecycle.lock().await;
            if !self
                .generation_matches_locked(task_id, task_epoch, all_epoch)
                .await
            {
                return Err("LSP 범위가 종료되어 서버 시작을 취소했습니다".into());
            }
            if let Some(existing) = self.live_client(&key).await {
                return Ok(existing);
            }
        }
        let start = {
            let mut starts = self.inner.starts.lock().await;
            Arc::clone(
                starts
                    .entry(key.clone())
                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
            )
        };
        let _start_guard = start.lock().await;
        {
            let _lifecycle = self.inner.lifecycle.lock().await;
            if self
                .generation_matches_locked(task_id, task_epoch, all_epoch)
                .await
            {
                if let Some(existing) = self.live_client(&key).await {
                    return Ok(existing);
                }
            } else {
                return Err("LSP 범위가 종료되어 서버 시작을 취소했습니다".into());
            }
        }
        let starting = Arc::new(StartingState::new());
        {
            let _lifecycle = self.inner.lifecycle.lock().await;
            if !self
                .generation_matches_locked(task_id, task_epoch, all_epoch)
                .await
            {
                return Err("LSP 범위가 종료되어 서버 시작을 취소했습니다".into());
            }
            self.inner
                .starting
                .lock()
                .await
                .insert(key.clone(), Arc::clone(&starting));
        }
        let _completion = StartingCompletion {
            state: Arc::clone(&starting),
        };
        let client = match self
            .spawn_until_cancelled(task_id, spec, &root, &key.config_identity, &starting)
            .await
        {
            Ok(client) => Arc::new(client),
            Err(error) => {
                let _lifecycle = self.inner.lifecycle.lock().await;
                if self
                    .generation_matches_locked(task_id, task_epoch, all_epoch)
                    .await
                {
                    self.inner
                        .failures
                        .lock()
                        .await
                        .insert(key.clone(), error.clone());
                }
                self.finish_start(&key, &start, &starting).await;
                return Err(error);
            }
        };
        let _lifecycle = self.inner.lifecycle.lock().await;
        if !self
            .generation_matches_locked(task_id, task_epoch, all_epoch)
            .await
        {
            client.shutdown();
            self.finish_start(&key, &start, &starting).await;
            return Err("LSP 범위가 종료되어 서버 시작을 취소했습니다".into());
        }
        self.inner.failures.lock().await.remove(&key);
        self.inner
            .clients
            .lock()
            .await
            .insert(key.clone(), Arc::clone(&client));
        drop(_lifecycle);
        self.finish_start(&key, &start, &starting).await;
        self.ensure_reaper();
        self.reap().await;
        Ok(client)
    }

    async fn generation_locked(&self, task_id: i64) -> (u64, i64) {
        let task_epoch = self
            .inner
            .task_epochs
            .lock()
            .await
            .get(&task_id)
            .copied()
            .unwrap_or_default();
        (task_epoch, self.inner.all_epoch.load(Ordering::SeqCst))
    }

    #[cfg(test)]
    async fn pause_before_first_cache_read(&self) {
        let gate = self.inner.before_first_cache_read.lock().await.clone();
        if let Some(gate) = gate {
            gate.entered.wait().await;
            gate.release.wait().await;
        }
    }

    async fn generation_matches_locked(
        &self,
        task_id: i64,
        task_epoch: u64,
        all_epoch: i64,
    ) -> bool {
        self.generation_locked(task_id).await == (task_epoch, all_epoch)
    }

    async fn spawn_until_cancelled(
        &self,
        task_id: i64,
        spec: ServerSpec,
        root: &Path,
        config_identity: &str,
        starting: &StartingState,
    ) -> Result<LspClient, String> {
        let cancelled = starting.cancelled_changed.notified();
        tokio::pin!(cancelled);
        cancelled.as_mut().enable();
        if starting.cancelled.load(Ordering::SeqCst) {
            return Err("LSP 범위가 종료되어 서버 시작을 취소했습니다".into());
        }
        let spawn = LspClient::spawn(task_id, spec, root, config_identity);
        tokio::pin!(spawn);
        tokio::select! {
            result = &mut spawn => result,
            _ = &mut cancelled => Err("LSP 범위가 종료되어 서버 시작을 취소했습니다".into()),
        }
    }

    async fn finish_start(
        &self,
        key: &ClientKey,
        start: &Arc<tokio::sync::Mutex<()>>,
        starting: &Arc<StartingState>,
    ) {
        let mut starts = self.inner.starts.lock().await;
        if starts
            .get(key)
            .is_some_and(|current| Arc::ptr_eq(current, start))
        {
            starts.remove(key);
        }
        drop(starts);
        let mut starting_entries = self.inner.starting.lock().await;
        if starting_entries
            .get(key)
            .is_some_and(|current| Arc::ptr_eq(current, starting))
        {
            starting_entries.remove(key);
        }
        starting.complete();
    }

    /// 작업이 끝나거나 폐기될 때 그 워크트리의 서버를 모두 정리한다.
    pub async fn shutdown_task(&self, task_id: i64) {
        let (clients, starting) = {
            let _lifecycle = self.inner.lifecycle.lock().await;
            let mut epochs = self.inner.task_epochs.lock().await;
            *epochs.entry(task_id).or_default() += 1;
            let mut clients = self.inner.clients.lock().await;
            let keys = clients
                .keys()
                .filter(|key| key.task_id == task_id)
                .cloned()
                .collect::<Vec<_>>();
            let clients = keys
                .into_iter()
                .filter_map(|key| clients.remove(&key))
                .collect::<Vec<_>>();
            let starting = {
                let mut entries = self.inner.starting.lock().await;
                let keys = entries
                    .keys()
                    .filter(|key| key.task_id == task_id)
                    .cloned()
                    .collect::<Vec<_>>();
                keys.into_iter()
                    .filter_map(|key| entries.remove(&key))
                    .collect::<Vec<_>>()
            };
            for state in &starting {
                state.cancel();
            }
            self.inner
                .starts
                .lock()
                .await
                .retain(|key, _| key.task_id != task_id);
            self.inner
                .failures
                .lock()
                .await
                .retain(|key, _| key.task_id != task_id);
            (clients, starting)
        };
        for client in clients {
            client.shutdown();
        }
        for state in starting {
            state.wait_complete().await;
        }
    }

    /// 앱 종료 경로.
    pub async fn shutdown_all(&self) {
        let (clients, starting) = {
            let _lifecycle = self.inner.lifecycle.lock().await;
            self.inner.terminal_shutdown.store(true, Ordering::SeqCst);
            self.inner.all_epoch.fetch_add(1, Ordering::SeqCst);
            let clients = std::mem::take(&mut *self.inner.clients.lock().await);
            let starting = std::mem::take(&mut *self.inner.starting.lock().await)
                .into_values()
                .collect::<Vec<_>>();
            for state in &starting {
                state.cancel();
            }
            self.inner.starts.lock().await.clear();
            self.inner.failures.lock().await.clear();
            (clients, starting)
        };
        for (_, client) in clients {
            client.shutdown();
        }
        for state in starting {
            state.wait_complete().await;
        }
    }

    pub async fn status_for(&self, task_id: i64, worktree: &Path, rel_path: &str) -> LspStatus {
        let abs = worktree.join(rel_path);
        let Some((spec, _)) = server::spec_for_path(&abs) else {
            return unavailable_status(None, None);
        };
        let root = match server::resolve_root(&spec, worktree, &abs) {
            Ok(root) => root,
            Err(detail) => return unavailable_status(Some(spec.key), Some(detail)),
        };
        if let Some(detail) = server::unavailable_reason(&spec, &root) {
            return unavailable_status(Some(spec.key), Some(detail));
        }
        let root = root.canonicalize().unwrap_or(root);
        let config_identity = match config_identity(&self.inner, &spec, &root).await {
            Ok(identity) => identity,
            Err(detail) => {
                return LspStatus {
                    available: true,
                    server: Some(spec.key.into()),
                    phase: "error".into(),
                    detail: Some(detail),
                }
            }
        };
        let key = ClientKey {
            task_id,
            root,
            server: spec.key,
            config_identity,
        };
        if let Some(detail) = self.inner.failures.lock().await.get(&key).cloned() {
            return LspStatus {
                available: true,
                server: Some(spec.key.into()),
                phase: "error".into(),
                detail: Some(detail),
            };
        }
        let completed_start = self
            .inner
            .starting
            .lock()
            .await
            .get(&key)
            .is_some_and(|state| state.completed.load(Ordering::SeqCst));
        if completed_start {
            self.inner.starting.lock().await.remove(&key);
            self.inner.starts.lock().await.remove(&key);
        }
        if self.inner.starts.lock().await.contains_key(&key) {
            return LspStatus {
                available: true,
                server: Some(spec.key.into()),
                phase: "starting".into(),
                detail: None,
            };
        }
        let client = self.inner.clients.lock().await.get(&key).cloned();
        match client {
            None => LspStatus {
                available: true,
                server: Some(spec.key.into()),
                phase: "idle".into(),
                detail: None,
            },
            Some(client) if !client.is_alive() => LspStatus {
                available: true,
                server: Some(spec.key.into()),
                phase: "error".into(),
                detail: Some(client.death_detail()),
            },
            Some(client) => match client.health.phase() {
                ProjectHealth::Starting => LspStatus {
                    available: true,
                    server: Some(spec.key.into()),
                    phase: "starting".into(),
                    detail: None,
                },
                ProjectHealth::Analyzing => LspStatus {
                    available: true,
                    server: Some(spec.key.into()),
                    phase: "analyzing".into(),
                    detail: None,
                },
                ProjectHealth::Ready => LspStatus {
                    available: true,
                    server: Some(spec.key.into()),
                    phase: "ready".into(),
                    // 준비는 됐지만 결과가 온전하지 않을 수 있는 사유(컴파일 오류 등).
                    detail: client.health.warning(),
                },
                ProjectHealth::Failed(detail) => LspStatus {
                    available: true,
                    server: Some(spec.key.into()),
                    phase: "error".into(),
                    detail: Some(detail),
                },
            },
        }
    }

    fn ensure_reaper(&self) {
        if self.inner.reaper_active.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(REAPER_INTERVAL).await;
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                if reap_inner(&inner).await {
                    return;
                }
            }
        });
    }

    async fn reap(&self) {
        let _ = reap_inner(&self.inner).await;
    }
}

/// 이 파일에서 LSP를 쓸 수 있는지 — 에디터가 탭을 열 때 물어본다.
pub fn status_for(worktree: &Path, rel_path: &str) -> LspStatus {
    let Some((spec, _)) = server::spec_for_path(&worktree.join(rel_path)) else {
        return unavailable_status(None, None);
    };
    let root = match server::resolve_root(&spec, worktree, &worktree.join(rel_path)) {
        Ok(root) => root,
        Err(detail) => return unavailable_status(Some(spec.key), Some(detail)),
    };
    match server::unavailable_reason(&spec, &root) {
        Some(detail) => unavailable_status(Some(spec.key), Some(detail)),
        None => LspStatus {
            available: true,
            server: Some(spec.key.to_string()),
            phase: "idle".into(),
            detail: None,
        },
    }
}

fn unavailable_status(server: Option<&str>, detail: Option<String>) -> LspStatus {
    LspStatus {
        available: false,
        server: server.map(str::to_string),
        phase: "unavailable".into(),
        detail,
    }
}

async fn reap_inner(inner: &Arc<LspPoolInner>) -> bool {
    let mut clients = inner.clients.lock().await;
    clients.retain(|_, client| {
        client.is_alive() && (Arc::strong_count(client) > 1 || client.idle_for() < IDLE_TTL)
    });
    let mut idle = clients
        .iter()
        .filter(|(_, client)| Arc::strong_count(client) == 1)
        .map(|(key, client)| (key.clone(), client.idle_for()))
        .collect::<Vec<_>>();
    idle.sort_by_key(|(_, duration)| std::cmp::Reverse(*duration));
    for (key, _) in idle {
        if clients.len() <= MAX_CACHED_IDLE {
            break;
        }
        clients.remove(&key);
    }
    if clients.is_empty() {
        inner.reaper_active.store(false, Ordering::SeqCst);
        true
    } else {
        false
    }
}

/// LSP 위치 → 프론트 타겟. 워크트리 안이면 상대 경로를, 밖이면 external로 표시한다.
fn to_target(loc: &protocol::RawLocation, worktree: &Path) -> Option<LspTarget> {
    let abs = protocol::uri_to_path(&loc.uri)?;
    let rel = match (abs.canonicalize(), worktree.canonicalize()) {
        // macOS의 /var → /private/var 별칭과 워크트리 안의 심링크를 모두 올바르게 분류한다.
        (Ok(canonical_abs), Ok(canonical_worktree)) => canonical_abs
            .strip_prefix(canonical_worktree)
            .ok()
            .and_then(|p| p.to_str())
            .map(str::to_string),
        // 아직 생성되지 않은 LSP 대상은 canonicalize할 수 없으므로, 기존 경로 비교를 유지한다.
        _ => abs
            .strip_prefix(worktree)
            .ok()
            .and_then(|p| p.to_str())
            .map(str::to_string),
    };
    Some(LspTarget {
        path: rel.clone(),
        abs_path: abs.to_string_lossy().into_owned(),
        line: loc.line + 1,
        column: loc.character + 1,
        external: rel.is_none(),
    })
}

/// DR-1의 Unknown 해소 — 이 저장소 자신을 상대로 `documentSymbol` 응답 시간을 잰다.
///
/// 기본 실행에서는 건너뛴다. rust-analyzer 설치가 필요하고, 첫 요청은 프로젝트 인덱싱을
/// 기다리느라 수십 초가 걸린다 — CI에 두면 느리고 불안정하다.
///
/// ```text
/// cargo test --lib -- --ignored lsp_symbol_timing --nocapture
/// ```
#[cfg(test)]
mod timing {
    use super::*;

    /// 워밍업을 뺀 나머지의 중앙값이 이 값을 넘으면 파일 단위 배치를 재검토한다(계획 0037 Task 2).
    const BATCH_THRESHOLD_MS: u128 = 200;

    fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    #[tokio::test]
    #[ignore = "실측: rust-analyzer 설치 필요"]
    async fn lsp_symbol_timing() {
        let worktree = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        rust_sources(&worktree.join("src"), &mut files);
        files.sort();
        assert!(!files.is_empty(), "측정할 .rs 파일이 없다");

        let pool = LspPool::default();
        let mut samples: Vec<(u128, usize, String)> = Vec::new();
        let mut failures = 0usize;

        for path in &files {
            let rel = path.strip_prefix(&worktree).unwrap().to_string_lossy();
            let started = std::time::Instant::now();
            match pool.document_symbols(0, &worktree, &rel).await {
                Ok(symbols) => samples.push((
                    started.elapsed().as_millis(),
                    symbols.len(),
                    rel.into_owned(),
                )),
                Err(error) => {
                    failures += 1;
                    eprintln!("  실패 {rel}: {error}");
                }
            }
        }
        pool.shutdown_task(0).await;

        assert!(
            !samples.is_empty(),
            "성공한 측정이 없다 (실패 {failures}건)"
        );
        // 첫 요청은 프로젝트 인덱싱을 기다린다 — 나머지와 성격이 달라 따로 본다.
        let (warmup, rest) = samples.split_first().unwrap();
        let mut durations: Vec<u128> = rest.iter().map(|(ms, _, _)| *ms).collect();
        durations.sort_unstable();
        let median = durations.get(durations.len() / 2).copied().unwrap_or(0);
        let total: u128 = samples.iter().map(|(ms, _, _)| ms).sum();
        let symbols: usize = samples.iter().map(|(_, n, _)| n).sum();

        println!("\n=== documentSymbol 실측 ({}) ===", worktree.display());
        println!("파일 {} · 실패 {failures} · 심볼 {symbols}", files.len());
        println!("워밍업(첫 파일 {}): {} ms", warmup.2, warmup.0);
        println!("총 {total} ms · 워밍업 제외 중앙값 {median} ms");
        if let Some((ms, n, path)) = rest.iter().max_by_key(|(ms, _, _)| *ms) {
            println!("최장: {path} — {ms} ms ({n} 심볼)");
        }
        println!(
            "판정: 중앙값 {median} ms {} {BATCH_THRESHOLD_MS} ms → 파일 단위 배치 {}",
            if median <= BATCH_THRESHOLD_MS {
                "≤"
            } else {
                ">"
            },
            if median <= BATCH_THRESHOLD_MS {
                "유지"
            } else {
                "재검토"
            },
        );
    }

    #[tokio::test]
    #[ignore = "실측: rust-analyzer 설치 필요"]
    async fn rust_analyzer_status_reaches_ready() {
        let worktree = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let (spec, _) = server::spec_for_path(Path::new("probe.rs")).unwrap();
        let pool = LspPool::default();
        let symbols = pool.document_symbols(1, &worktree, "src/lib.rs").await;
        let readiness = pool.wait_semantic_ready(1, &worktree, spec).await;
        pool.shutdown_task(1).await;

        assert!(!symbols.unwrap().is_empty(), "src/lib.rs 심볼이 비었습니다");
        assert_eq!(
            readiness,
            Readiness::Ready,
            "공식 serverStatus ready 알림을 받지 못했습니다"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    const FAKE_LSP_SCRIPT: &str = r#"
import json, sys, time
def read_message():
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line: return None
        if line in (b'\n', b'\r\n'): break
        key, value = line.decode().split(':', 1)
        headers[key.lower()] = value.strip()
    return json.loads(sys.stdin.buffer.read(int(headers['content-length'])))
def send(value):
    data = json.dumps(value).encode()
    sys.stdout.buffer.write(b'Content-Length: ' + str(len(data)).encode() + b'\r\n\r\n' + data)
    sys.stdout.buffer.flush()
while True:
    request = read_message()
    if request is None: break
    method = request.get('method')
    if method == 'initialize':
        if 'slow' in json.dumps(request): time.sleep(.35)
        send({'jsonrpc':'2.0', 'id':request['id'], 'result':{'capabilities':{}}})
    elif method == 'sleep':
        time.sleep(10)
    elif 'id' in request:
        send({'jsonrpc':'2.0', 'id':request['id'], 'result':None})
"#;

    #[cfg(unix)]
    const FAKE_SPEC: ServerSpec = ServerSpec {
        key: "fake-lsp",
        command: "python3",
        args: &["-u", "-c", FAKE_LSP_SCRIPT],
        root_markers: &["fixture"],
        config_files: &[],
        readiness: server::ReadinessSignal::Unsupported,
        init_timeout_secs: 2,
        ready_timeout_secs: 2,
    };

    #[cfg(unix)]
    const CONFIG_FAKE_SPEC: ServerSpec = ServerSpec {
        key: "fake-config-lsp",
        command: "python3",
        args: &["-u", "-c", FAKE_LSP_SCRIPT],
        root_markers: &["fixture"],
        config_files: &["fixture.toml"],
        readiness: server::ReadinessSignal::Unsupported,
        init_timeout_secs: 2,
        ready_timeout_secs: 2,
    };

    #[cfg(unix)]
    fn fake_root(name: &str) -> PathBuf {
        let root = crate::testtmp::dir().join(format!("lsp-pool-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("fixture"), "\n").unwrap();
        root
    }

    fn spec_of(sample: &str) -> ServerSpec {
        server::spec_for_path(Path::new(sample)).unwrap().0
    }

    /// 상한을 밀리초로 줄인 래치 — 도달하지 못하는 경우를 테스트가 60초 기다리지 않는다.
    fn latch(sample: &str) -> SemanticLatch {
        let mut spec = spec_of(sample);
        spec.ready_timeout_secs = 0;
        SemanticLatch::new(spec)
    }

    #[test]
    fn initialize_declares_work_done_progress_for_every_server() {
        for sample in ["probe.rs", "a.ts", "a.py", "A.java"] {
            let params = initialize_params(&spec_of(sample), Path::new("/workspace"), &json!({}));
            assert_eq!(
                params.pointer("/capabilities/window/workDoneProgress"),
                Some(&Value::Bool(true)),
                "{sample}: 선언하지 않으면 서버가 $/progress를 보낼 수 없다"
            );
            assert_eq!(
                params.pointer(
                    "/capabilities/textDocument/documentSymbol/hierarchicalDocumentSymbolSupport"
                ),
                Some(&Value::Bool(true)),
                "{sample}: 빠지면 구형 SymbolInformation으로 폴백한다"
            );
        }
    }

    #[test]
    fn server_status_notification_goes_only_to_rust_analyzer() {
        let rust = initialize_params(&spec_of("probe.rs"), Path::new("/workspace"), &json!({}));
        assert_eq!(
            rust.pointer("/capabilities/experimental/serverStatusNotification"),
            Some(&Value::Bool(true))
        );
        for sample in ["a.ts", "a.py", "A.java"] {
            let params = initialize_params(&spec_of(sample), Path::new("/workspace"), &json!({}));
            assert!(
                params.pointer("/capabilities/experimental").is_none(),
                "{sample}: rust-analyzer 전용 capability를 보내면 안 된다"
            );
        }
    }

    #[tokio::test]
    async fn server_status_latch_needs_ok_and_quiescent() {
        let latch = latch("probe.rs");
        latch.record(
            "experimental/serverStatus",
            &json!({ "health": "ok", "quiescent": false }),
        );
        assert!(
            matches!(latch.wait().await, Readiness::Failed(_)),
            "분석 중 상태는 준비로 통과하면 안 된다"
        );

        latch.record(
            "experimental/serverStatus",
            &json!({ "health": "ok", "quiescent": true }),
        );
        assert_eq!(latch.wait().await, Readiness::Ready);
    }

    #[tokio::test]
    async fn ready_latch_never_goes_back() {
        let latch = latch("probe.rs");
        latch.record(
            "experimental/serverStatus",
            &json!({ "health": "ok", "quiescent": true }),
        );
        // 도달 후에 오는 어떤 알림도 상태를 되돌리지 못한다.
        latch.record(
            "experimental/serverStatus",
            &json!({ "health": "error", "quiescent": false, "message": "cargo 실패" }),
        );
        assert_eq!(latch.wait().await, Readiness::Ready);
    }

    #[tokio::test]
    async fn progress_end_latches_once_and_ignores_the_rest() {
        let latch = latch("a.ts");
        latch.record(
            "$/progress",
            &json!({ "token": "1", "value": { "kind": "begin" } }),
        );
        assert!(matches!(latch.wait().await, Readiness::Failed(_)));

        latch.record(
            "$/progress",
            &json!({ "token": "1", "value": { "kind": "end" } }),
        );
        latch.record(
            "$/progress",
            &json!({ "token": "2", "value": { "kind": "begin" } }),
        );
        assert_eq!(latch.wait().await, Readiness::Ready);
    }

    #[tokio::test]
    async fn language_status_latches_on_service_ready_only() {
        let latch = latch("A.java");
        latch.record(
            "language/status",
            &json!({ "type": "ProjectStatus", "message": "OK" }),
        );
        latch.record(
            "language/status",
            &json!({ "type": "Started", "message": "Ready" }),
        );
        assert!(
            matches!(latch.wait().await, Readiness::Failed(_)),
            "Started의 message가 Ready인 것에 속으면 안 된다"
        );

        latch.record("language/status", &json!({ "type": "ServiceReady" }));
        assert_eq!(latch.wait().await, Readiness::Ready);
    }

    /// `ProjectStatus: WARNING`은 "프로젝트에 컴파일 오류 마커가 있다"는 뜻이지 import 실패가
    /// 아니다. Lombok을 쓰는 실제 프로젝트가 그렇고, 그 뒤에도 정의·사용처 조회는 동작한다.
    #[test]
    fn java_project_problems_keep_health_ready_with_a_warning() {
        let health = ProjectHealthState::new();
        let spec = spec_of("A.java");
        health.record(
            &spec,
            "language/status",
            &json!({ "type": "ProjectStatus", "message": "WARNING" }),
        );
        assert_eq!(health.phase(), ProjectHealth::Analyzing);
        health.record(&spec, "language/status", &json!({ "type": "ServiceReady" }));
        assert_eq!(health.phase(), ProjectHealth::Ready);
        assert!(
            health.warning().is_some_and(|w| w.contains("WARNING")),
            "사유는 보조 설명으로만 남는다"
        );
    }

    /// jdtls의 LogHandler는 플랫폼 로그의 ERROR 항목을 전부 `window/logMessage type=1`로
    /// 보낸다(JavaBuilder 예외 등). 살아 있는 서버를 그것으로 실패 처리하면 안 된다.
    #[test]
    fn java_error_log_messages_do_not_fail_health() {
        let health = ProjectHealthState::new();
        let spec = spec_of("A.java");
        health.record(
            &spec,
            "language/status",
            &json!({ "type": "ProjectStatus", "message": "OK" }),
        );
        health.record(&spec, "language/status", &json!({ "type": "ServiceReady" }));
        health.record(
            &spec,
            "window/logMessage",
            &json!({ "type": 1, "message": "JavaBuilder handling ImageBuilderInternalException while building: core" }),
        );
        health.record(
            &spec,
            "window/showMessage",
            &json!({ "type": 1, "message": "Build path problems" }),
        );
        assert_eq!(health.phase(), ProjectHealth::Ready);
    }

    /// 진짜 초기화 실패는 `language/status`의 `Error`로 온다 — 상태와 래치가 함께 끝나야
    /// 오지 않을 ServiceReady를 상한까지 기다리지 않는다.
    #[tokio::test]
    async fn java_init_error_fails_health_and_latch() {
        let health = ProjectHealthState::new();
        let spec = spec_of("A.java");
        health.record(
            &spec,
            "language/status",
            &json!({ "type": "Error", "message": "Failed to initialize" }),
        );
        health.record(&spec, "language/status", &json!({ "type": "ServiceReady" }));
        assert!(matches!(
            health.phase(),
            ProjectHealth::Failed(reason) if reason.contains("Failed to initialize")
        ));

        let latch = latch("A.java");
        latch.record(
            "language/status",
            &json!({ "type": "Error", "message": "Failed to initialize" }),
        );
        assert!(matches!(
            latch.wait().await,
            Readiness::Failed(reason) if reason.contains("Failed to initialize")
        ));
    }

    /// 준비 뒤에 다시 오는 ProjectStatus(빌드 파일 변경 재import)로 "분석 중"에 되돌아가지 않는다.
    #[test]
    fn java_project_status_after_ready_does_not_regress() {
        let health = ProjectHealthState::new();
        let spec = spec_of("A.java");
        health.record(
            &spec,
            "language/status",
            &json!({ "type": "ProjectStatus", "message": "OK" }),
        );
        health.record(&spec, "language/status", &json!({ "type": "ServiceReady" }));
        health.record(
            &spec,
            "language/status",
            &json!({ "type": "ProjectStatus", "message": "WARNING" }),
        );
        assert_eq!(health.phase(), ProjectHealth::Ready);
    }

    #[test]
    fn pool_key_isolates_task_root_and_configuration() {
        let base = ClientKey {
            task_id: 1,
            root: PathBuf::from("/workspace/a"),
            server: "jdtls",
            config_identity: "jdk17".into(),
        };
        let other_task = ClientKey {
            task_id: 2,
            ..base.clone()
        };
        let other_root = ClientKey {
            root: PathBuf::from("/workspace/b"),
            ..base.clone()
        };
        let other_config = ClientKey {
            config_identity: "jdk21".into(),
            ..base.clone()
        };
        assert_ne!(base, other_task);
        assert_ne!(base, other_root);
        assert_ne!(base, other_config);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn same_key_start_is_coalesced_and_idle_cache_is_capped() {
        let pool = LspPool::default();
        let root = fake_root("coalesce");
        let (first, second) = tokio::join!(
            pool.client_for(31, &root, FAKE_SPEC),
            pool.client_for(31, &root, FAKE_SPEC),
        );
        let first = first.unwrap();
        let second = second.unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "같은 키의 시작은 한 프로세스를 공유해야 한다"
        );
        drop((first, second));

        for name in ["cap-a", "cap-b", "cap-c"] {
            let client = pool
                .client_for(31, &fake_root(name), FAKE_SPEC)
                .await
                .unwrap();
            drop(client);
        }
        pool.reap().await;
        assert!(pool.inner.clients.lock().await.len() <= MAX_CACHED_IDLE);
        pool.shutdown_all().await;
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn configuration_content_changes_client_and_jdtls_cache_identity() {
        let pool = LspPool::default();
        let root = fake_root("config-identity");
        let config = root.join("fixture.toml");
        let missing = config_identity(&pool.inner, &CONFIG_FAKE_SPEC, &root)
            .await
            .unwrap();
        std::fs::write(&config, "missing").unwrap();
        let present = config_identity(&pool.inner, &CONFIG_FAKE_SPEC, &root)
            .await
            .unwrap();
        assert_ne!(
            missing, present,
            "없는 파일과 내용이 missing인 파일은 달라야 한다"
        );
        let stable = config_identity(&pool.inner, &CONFIG_FAKE_SPEC, &root)
            .await
            .unwrap();
        assert_eq!(
            present, stable,
            "변하지 않은 설정은 같은 identity를 써야 한다"
        );
        std::fs::write(&config, "changed").unwrap();
        let changed = config_identity(&pool.inner, &CONFIG_FAKE_SPEC, &root)
            .await
            .unwrap();
        assert_ne!(present, changed);
        assert_ne!(
            jdtls_data_path(Path::new("/cache"), &root, 41, &present),
            jdtls_data_path(Path::new("/cache"), &root, 41, &changed),
            "설정이 달라지면 jdtls data 경로도 달라야 한다"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shutdown_before_first_cache_read_cannot_return_stale_client() {
        let pool = LspPool::default();
        let root = fake_root("shutdown-before-cache");
        let client = pool.client_for(36, &root, FAKE_SPEC).await.unwrap();
        drop(client);
        let gate = Arc::new(TestCacheReadGate {
            entered: tokio::sync::Barrier::new(2),
            release: tokio::sync::Barrier::new(2),
        });
        *pool.inner.before_first_cache_read.lock().await = Some(Arc::clone(&gate));
        let starter_pool = pool.clone();
        let starter_root = root.clone();
        let starter =
            tokio::spawn(
                async move { starter_pool.client_for(36, &starter_root, FAKE_SPEC).await },
            );
        gate.entered.wait().await;
        pool.shutdown_task(36).await;
        gate.release.wait().await;
        assert!(starter.await.unwrap().is_err());
        assert!(pool.inner.clients.lock().await.is_empty());
        *pool.inner.before_first_cache_read.lock().await = None;
        pool.shutdown_all().await;
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shutdown_interrupts_pending_request_and_cancels_start_generation() {
        let pool = LspPool::default();
        let root = fake_root("pending");
        let client = pool.client_for(32, &root, FAKE_SPEC).await.unwrap();
        let request_client = Arc::clone(&client);
        let pending = tokio::spawn(async move {
            request_client
                .request("sleep", json!({}), Duration::from_secs(5))
                .await
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        pool.shutdown_task(32).await;
        assert!(tokio::time::timeout(Duration::from_secs(1), pending)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        assert!(pool.inner.clients.lock().await.is_empty());
        drop(client);

        let slow_root = fake_root("slow");
        let starting_pool = pool.clone();
        let starting_root = slow_root.clone();
        let starting = tokio::spawn(async move {
            starting_pool
                .client_for(33, &starting_root, FAKE_SPEC)
                .await
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        pool.shutdown_task(33).await;
        assert!(
            starting.await.unwrap().is_err(),
            "종료 중 시작한 서버는 풀에 들어가면 안 된다"
        );
        assert!(pool.inner.clients.lock().await.is_empty());
        let terminal_pool = LspPool::default();
        let terminal_root = fake_root("slow-terminal");
        let terminal_start_pool = terminal_pool.clone();
        let terminal_start_root = terminal_root.clone();
        let terminal_start = tokio::spawn(async move {
            terminal_start_pool
                .client_for(34, &terminal_start_root, FAKE_SPEC)
                .await
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        let stopped = Instant::now();
        terminal_pool.shutdown_all().await;
        assert!(
            stopped.elapsed() < Duration::from_secs(1),
            "종료는 initialize 상한을 기다리면 안 된다"
        );
        assert!(terminal_start.await.unwrap().is_err());
        assert!(terminal_pool.inner.clients.lock().await.is_empty());
        assert!(terminal_pool.inner.starting.lock().await.is_empty());

        let aborted_pool = LspPool::default();
        let aborted_root = fake_root("slow-aborted");
        let aborted_start_pool = aborted_pool.clone();
        let aborted_start_root = aborted_root.clone();
        let aborted = tokio::spawn(async move {
            aborted_start_pool
                .client_for(35, &aborted_start_root, FAKE_SPEC)
                .await
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        aborted.abort();
        assert!(aborted.await.is_err());
        assert!(
            tokio::time::timeout(Duration::from_secs(1), aborted_pool.shutdown_task(35))
                .await
                .is_ok()
        );
        assert!(aborted_pool.inner.starting.lock().await.is_empty());
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(slow_root);
        let _ = std::fs::remove_dir_all(terminal_root);
        let _ = std::fs::remove_dir_all(aborted_root);
    }

    #[tokio::test]
    async fn latch_ignores_signals_other_servers_use() {
        let latch = latch("probe.rs");
        latch.record(
            "$/progress",
            &json!({ "token": "1", "value": { "kind": "end" } }),
        );
        latch.record("language/status", &json!({ "type": "ServiceReady" }));
        assert!(
            matches!(latch.wait().await, Readiness::Failed(_)),
            "다른 서버의 신호로 준비 판정을 내리면 안 된다"
        );
    }

    #[tokio::test]
    async fn unsupported_server_returns_immediately_without_spawning() {
        let pool = LspPool::default();
        let started = std::time::Instant::now();
        let readiness = pool
            .wait_semantic_ready(0, Path::new("/nonexistent"), spec_of("a.py"))
            .await;
        assert!(
            matches!(readiness, Readiness::Unsupported(reason) if reason.contains("pyright")),
            "기다릴 신호가 없는 서버는 사유와 함께 즉시 돌아와야 한다"
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "상한까지 기다리면 심볼까지 잃는다"
        );
    }

    #[test]
    fn goto_kind_parses_known_values_only() {
        assert_eq!(GotoKind::parse("definition").unwrap(), GotoKind::Definition);
        assert_eq!(
            GotoKind::parse("implementation").unwrap(),
            GotoKind::Implementation
        );
        assert!(GotoKind::parse("rename").is_err());
    }

    #[test]
    fn target_converts_zero_based_lsp_to_one_based_monaco() {
        let loc = protocol::RawLocation {
            uri: protocol::path_to_uri(Path::new("/w/src/main.rs")),
            line: 0,
            character: 0,
        };
        let target = to_target(&loc, Path::new("/w")).unwrap();
        // LSP의 (0,0) = 파일 첫 글자 = Monaco의 (1,1).
        assert_eq!((target.line, target.column), (1, 1));
        assert_eq!(target.path.as_deref(), Some("src/main.rs"));
        assert!(!target.external);
    }

    #[cfg(unix)]
    #[test]
    fn target_is_internal_when_worktree_is_a_canonical_alias() {
        use std::os::unix::fs::symlink;

        let nonce = format!("{}-{:?}", std::process::id(), std::thread::current().id());
        let root = crate::testtmp::dir().join(format!("lsp-target-worktree-{nonce}"));
        let alias = crate::testtmp::dir().join(format!("lsp-target-alias-{nonce}"));
        let inside = root.join("src/inside.rs");
        std::fs::create_dir_all(inside.parent().unwrap()).unwrap();
        std::fs::write(&inside, "fn inside() {}\n").unwrap();
        symlink(&root, &alias).unwrap();

        let target = to_target(
            &protocol::RawLocation {
                uri: protocol::path_to_uri(&inside),
                line: 0,
                character: 0,
            },
            &alias,
        )
        .unwrap();
        assert_eq!(target.path.as_deref(), Some("src/inside.rs"));

        let _ = std::fs::remove_file(&alias);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn target_is_external_when_worktree_symlink_escapes() {
        use std::os::unix::fs::symlink;

        let nonce = format!("{}-{:?}", std::process::id(), std::thread::current().id());
        let root = crate::testtmp::dir().join(format!("lsp-target-worktree-{nonce}"));
        let outside = crate::testtmp::dir().join(format!("lsp-target-outside-{nonce}"));
        let escaped = root.join("escaped.rs");
        let outside_file = outside.join("outside.rs");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(&outside_file, "fn outside() {}\n").unwrap();
        symlink(&outside_file, &escaped).unwrap();

        let target = to_target(
            &protocol::RawLocation {
                uri: protocol::path_to_uri(&escaped),
                line: 0,
                character: 0,
            },
            &root,
        )
        .unwrap();
        assert!(target.external);

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn target_outside_worktree_is_external() {
        let loc = protocol::RawLocation {
            uri: protocol::path_to_uri(Path::new("/usr/lib/rust/core.rs")),
            line: 9,
            character: 3,
        };
        let target = to_target(&loc, Path::new("/w")).unwrap();
        assert!(target.external);
        assert_eq!(target.path, None);
        assert_eq!(target.abs_path, "/usr/lib/rust/core.rs");
    }

    #[test]
    fn unsupported_scheme_is_dropped_not_crashed() {
        let loc = protocol::RawLocation {
            uri: "untitled:Untitled-1".into(),
            line: 0,
            character: 0,
        };
        assert!(to_target(&loc, Path::new("/w")).is_none());
    }

    #[test]
    fn status_is_quiet_for_unsupported_languages() {
        let status = status_for(Path::new("/w"), "README.md");
        assert!(!status.available);
        assert!(status.server.is_none());
        assert!(status.detail.is_none());
    }
}
