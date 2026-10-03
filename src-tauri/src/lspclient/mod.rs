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
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{oneshot, Notify};

pub use server::ServerSpec;

/// 점프 요청 한 건의 상한. rust-analyzer는 최초 인덱싱 중 느리게 답한다.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
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
    stderr_tail: Arc<Mutex<Vec<String>>>,
}

impl LspClient {
    /// 서버를 띄우고 initialize 핸드셰이크까지 마친다.
    async fn spawn(spec: ServerSpec, root: &Path) -> Result<Self, String> {
        // Windows에서 npm으로 깐 언어 서버는 `.cmd` shim이라 맨 이름으로는 실행되지 않는다.
        let program = crate::reviewer::which(spec.command).unwrap_or_else(|| spec.command.to_string());
        let mut command = Command::new(program);
        command
            .args(spec.args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
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

        spawn_reader(
            stdout,
            Arc::clone(&pending),
            Arc::clone(&stdin),
            Arc::clone(&readiness),
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
            stderr_tail,
        };
        client.initialize().await?;
        Ok(client)
    }

    async fn initialize(&self) -> Result<(), String> {
        let params = initialize_params(&self.spec, &self.root);
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
            json!({ "settings": {} }),
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
        let result = self.request(kind.method(), params, REQUEST_TIMEOUT).await?;
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

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            // 리더 스레드가 sender를 떨군다 = 서버가 죽었다.
            Ok(Err(_)) => Err(self.death_detail()),
            Err(_) => {
                self.pending.lock().ok().and_then(|mut p| p.remove(&id));
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
}

impl Drop for LspClient {
    fn drop(&mut self) {
        // exit 노티는 생략한다 — 어차피 프로세스 그룹을 죽이고, 정상 종료를 기다리면
        // 앱 종료가 서버 응답에 묶인다.
        if let Ok(mut child) = self.child.lock() {
            crate::verify::kill_group(child.id());
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// `initialize`의 capability 선언 — **서버별로 다르다.**
///
/// `window.workDoneProgress`를 선언하지 않으면 서버가 `$/progress`를 보낼 수 없어
/// `Progress` 신호가 성립하지 않는다. `hierarchicalDocumentSymbolSupport`를 빼면
/// rust-analyzer가 구형 `SymbolInformation`으로 폴백해 좌표가 본문 시작으로 떨어지고
/// `references`가 0건이 된다(ADR 0099).
fn initialize_params(spec: &ServerSpec, root: &Path) -> Value {
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
                    answer_server_request(&stdin, id, method, &value);
                } else if let Some(params) = value.get("params") {
                    readiness.record(method, params);
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

fn answer_server_request(stdin: &Arc<Mutex<ChildStdin>>, id: i64, method: &str, request: &Value) {
    // 설정은 전부 기본값으로 답한다(항목 수만 맞춰 빈 객체). 나머지 요청은 null로 수락.
    let result = if method == "workspace/configuration" {
        let count = request
            .get("params")
            .and_then(|p| p.get("items"))
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        Value::Array(vec![json!({}); count])
    } else {
        Value::Null
    };
    let _ = write_message(
        stdin,
        &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
    );
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

/// 작업(워크트리) × 언어별 서버 풀.
#[derive(Default)]
pub struct LspPool {
    clients: tokio::sync::Mutex<HashMap<(i64, &'static str), Arc<LspClient>>>,
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
        if let Some(reason) = server::unavailable_reason(&spec, worktree) {
            return Err(reason);
        }

        let client = self.client_for(task_id, worktree, spec).await?;
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
        if server::unavailable_reason(&spec, worktree).is_some() {
            return Ok(None);
        }
        let client = self.client_for(task_id, worktree, spec).await?;
        let uri = protocol::path_to_uri(&abs);
        client.sync_document(&uri, language_id, text)?;
        client.semantic_tokens(&uri).await
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
        if let Some(reason) = server::unavailable_reason(&spec, worktree) {
            return Err(reason);
        }
        let text = std::fs::read_to_string(&abs).map_err(|e| format!("{}: {e}", abs.display()))?;

        let client = self.client_for(task_id, worktree, spec).await?;
        let uri = protocol::path_to_uri(&abs);
        client.sync_document(&uri, language_id, &text)?;
        client.document_symbols(&uri).await
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
        match self.client_for(task_id, worktree, spec).await {
            Ok(client) => client.readiness.wait().await,
            Err(reason) => Readiness::Failed(reason),
        }
    }

    /// 풀에서 살아 있는 클라이언트를 꺼낸다. 죽었으면 버리고 `None`.
    async fn live_client(&self, key: &(i64, &'static str)) -> Option<Arc<LspClient>> {
        let mut clients = self.clients.lock().await;
        let existing = clients.get(key)?;
        if existing.is_alive() {
            return Some(Arc::clone(existing));
        }
        clients.remove(key);
        None
    }

    /// **spawn 중에는 풀 가드를 놓는다.** 풀은 `AppState`에 하나뿐이라 가드를 쥔 채
    /// 기다리면 jdtls JVM 부팅 동안 모든 작업의 goto·semantic tokens가 멈춘다.
    /// 대신 경합하면 프로세스가 둘 뜰 수 있으므로, 늦게 도착한 쪽은 자기 것을 버린다
    /// (`Arc`가 여기서 떨어지고 `Drop`이 프로세스 그룹을 죽인다).
    async fn client_for(
        &self,
        task_id: i64,
        worktree: &Path,
        spec: ServerSpec,
    ) -> Result<Arc<LspClient>, String> {
        let key = (task_id, spec.key);
        if let Some(existing) = self.live_client(&key).await {
            return Ok(existing);
        }
        let client = Arc::new(LspClient::spawn(spec, worktree).await?);
        let mut clients = self.clients.lock().await;
        if let Some(existing) = clients.get(&key) {
            if existing.is_alive() {
                return Ok(Arc::clone(existing));
            }
        }
        clients.insert(key, Arc::clone(&client));
        Ok(client)
    }

    /// 작업이 끝나거나 폐기될 때 그 워크트리의 서버를 모두 정리한다.
    pub async fn shutdown_task(&self, task_id: i64) {
        let mut clients = self.clients.lock().await;
        clients.retain(|(id, _), _| *id != task_id);
    }

    /// 앱 종료 경로.
    pub async fn shutdown_all(&self) {
        self.clients.lock().await.clear();
    }
}

/// 이 파일에서 LSP를 쓸 수 있는지 — 에디터가 탭을 열 때 물어본다.
pub fn status_for(worktree: &Path, rel_path: &str) -> LspStatus {
    let Some((spec, _)) = server::spec_for_path(&worktree.join(rel_path)) else {
        return LspStatus {
            available: false,
            server: None,
            detail: None, // 지원 언어가 아닌 건 오류가 아니다 — 조용히 비활성.
        };
    };
    match server::unavailable_reason(&spec, worktree) {
        Some(detail) => LspStatus {
            available: false,
            server: Some(spec.key.to_string()),
            detail: Some(detail),
        },
        None => LspStatus {
            available: true,
            server: Some(spec.key.to_string()),
            detail: None,
        },
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
            let params = initialize_params(&spec_of(sample), Path::new("/workspace"));
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
        let rust = initialize_params(&spec_of("probe.rs"), Path::new("/workspace"));
        assert_eq!(
            rust.pointer("/capabilities/experimental/serverStatusNotification"),
            Some(&Value::Bool(true))
        );
        for sample in ["a.ts", "a.py", "A.java"] {
            let params = initialize_params(&spec_of(sample), Path::new("/workspace"));
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
