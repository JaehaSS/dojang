//! One stdio connection per turn. Requests and replies never cross execution IDs.
use super::{
    child_reaper::ReapOnDrop, interaction as ledger, process_cleanup::TurnProcessScope, ConvoEvent,
    Side, TurnOutcome,
};
use crate::preview_bridge::mcp::PreviewMcpLease;
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc, Arc, Mutex, OnceLock,
};
use std::time::{Duration, Instant};

const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(50);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
pub const SUPPORTED_VERSION: &str = "codex-cli 0.157.1";
pub const TOOL_INSTRUCTIONS: &str = "For questions to the user, call praxis_ui.ask_user with kind=clarification, 1-3 questions (id, question, options with id/label/description, allow_free_text, is_secret=false). Ask only ordinary clarifications. Never ask for secrets or tool execution approvals. Do not call request_user_input or request_user_input_async. The tool waits for the user's reply; continue only work that does not depend on that reply. Do not treat the answer as a permission grant. All children and commands must finish within this turn.";
const EXPANDED_TOOL_INSTRUCTIONS: &str = "Dojang supports Codex native user input and approval requests. Use the native request_user_input tool for questions when available; the praxis_ui.ask_user dynamic tool is also available for ordinary clarifications only. Never request passwords or credentials in a question. Ordinary answers do not grant execution permissions; use the provider's explicit approval flow. Native nonblocking questions can be answered while independent work continues. Computer Use, browser, Code Mode and installed connector tools are available according to this account, environment and user configuration. Use their actual capabilities and report unavailable tools. Finish all commands, Code Mode cells and delegated work within the current turn; wait for yielded cells before ending. The user can interrupt from Dojang at any time.";

pub(crate) fn instructions(expanded: bool) -> String {
    format!(
        "{}\n\n{}",
        super::turn_guard::TURN_COMPLETION_GUARD,
        if expanded {
            EXPANDED_TOOL_INSTRUCTIONS
        } else {
            TOOL_INSTRUCTIONS
        }
    )
}

pub struct Control {
    pub execution: String,
    /// Debate seat that owns this turn's vendor session. `None` preserves historical sessions.
    pub side: Option<Side>,
    pub cancelled: AtomicBool,
    pub cleanup_failed: AtomicBool,
    /// 로컬 MCP 런타임은 플래그를 읽어줄 이벤트 루프가 없다. 중단은 프로세스 그룹을 직접 죽인다.
    pub local: bool,
    /// `local`일 때만 채워진다. 0은 "아직 spawn 전".
    pub pgid: AtomicU32,
}
impl Control {
    pub fn new(execution: String, local: bool) -> Self {
        Self {
            execution,
            side: None,
            cancelled: AtomicBool::new(false),
            cleanup_failed: AtomicBool::new(false),
            local,
            pgid: AtomicU32::new(0),
        }
    }
}
fn controls() -> &'static Mutex<HashMap<i64, Arc<Control>>> {
    static MAP: OnceLock<Mutex<HashMap<i64, Arc<Control>>>> = OnceLock::new();
    MAP.get_or_init(Default::default)
}
pub fn register(task: i64, execution: String) -> Result<Arc<Control>, String> {
    register_turn(task, execution, false, None)
}
pub fn register_local(task: i64, execution: String) -> Result<Arc<Control>, String> {
    register_turn(task, execution, true, None)
}
pub fn register_turn(
    task: i64,
    execution: String,
    local: bool,
    side: Option<Side>,
) -> Result<Arc<Control>, String> {
    let mut map = controls().lock().unwrap_or_else(|e| e.into_inner());
    if map.contains_key(&task) {
        return Err("이 작업의 질문 실행이 아직 종료되지 않았습니다".into());
    }
    let mut control = Control::new(execution, local);
    control.side = side;
    let c = Arc::new(control);
    map.insert(task, c.clone());
    Ok(c)
}
pub fn execution(task: i64) -> Option<String> {
    controls()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&task)
        .map(|c| c.execution.clone())
}
pub fn unregister(task: i64) {
    controls()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&task);
}
pub fn cancel(task: i64) -> bool {
    let pgid = {
        let map = controls().lock().unwrap_or_else(|e| e.into_inner());
        let Some(c) = map.get(&task) else {
            return false;
        };
        c.cancelled.store(true, Ordering::SeqCst);
        // stdio 런타임은 다음 루프에서 플래그를 읽는다. 로컬 런타임에는 그 루프가 없다.
        if c.local {
            c.pgid.load(Ordering::SeqCst)
        } else {
            0
        }
    };
    if pgid != 0 {
        crate::verify::kill_group(pgid);
    }
    true
}
pub fn active_tasks() -> Vec<i64> {
    controls()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .copied()
        .collect()
}
pub fn cleanup_failed(task: i64) -> bool {
    controls()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&task)
        .is_some_and(|c| c.cleanup_failed.load(Ordering::SeqCst))
}

#[derive(Clone)]
pub struct Context {
    pub pool: SqlitePool,
    pub task_id: i64,
    pub control: Arc<Control>,
    pub changed: Arc<dyn Fn() + Send + Sync>,
}
impl Context {
    pub(super) fn db<T>(
        &self,
        future: impl std::future::Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        tauri::async_runtime::block_on(future)
    }
    pub(crate) fn changed(&self) {
        (self.changed)();
    }
    /// 질문이 열렸음을 알림 원천에 싣는다. 실행은 계속 running이라 상태 전이 알림이 없다 — 여기서
    /// 직접 기록해야 인박스·OS 알림이 온다. 질문은 이미 원장에 열려 있으므로 실패를 전파하지 않는다:
    /// 알림 하나가 빠지는 것이 질문이 사라지는 것보다 낫다.
    pub(super) async fn question_opened(&self) {
        if let Err(error) =
            crate::notifications::record_question(&self.pool, self.task_id, crate::now()).await
        {
            eprintln!(
                "[convo] 질문 알림을 기록하지 못했습니다 (task {}): {error}",
                self.task_id
            );
        }
    }
    fn open(&self, wire: &Value, call: &str, args: &Value) -> Result<ConvoEvent, String> {
        let id = self.db(ledger::open(
            &self.pool,
            &self.control.execution,
            wire,
            call,
            args,
            crate::now(),
        ))?;
        let event = ConvoEvent::Interaction { interaction_id: id };
        self.changed();
        self.db(async {
            self.question_opened().await;
            Ok::<(), String>(())
        })?;
        Ok(event)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn check_version(bin: &str) -> Result<(), String> {
    runtime_version(bin).map(|_| ())
}

pub fn check_exact_version(bin: &str, expected: &str) -> Result<(), String> {
    let actual = runtime_version(bin)?;
    if actual != expected {
        return Err(format!(
            "전용 Codex 버전이 일치하지 않습니다: 요청 {expected}, 확인 {actual}"
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn runtime_version(bin: &str) -> Result<String, String> {
    let mut c = Command::new(bin);
    c.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    let mut child = ReapOnDrop::new(c.spawn().map_err(|_| "Codex 버전을 확인할 수 없습니다")?);
    let deadline = Instant::now() + CLOSE_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            return Err("Codex 버전 확인 시간 초과".into());
        }
        std::thread::sleep(POLL);
    };
    let mut out = String::new();
    child
        .stdout
        .take()
        .ok_or("버전 출력 없음")?
        .take(4096)
        .read_to_string(&mut out)
        .map_err(|e| e.to_string())?;
    let version = out.trim().strip_prefix("codex-cli ").unwrap_or_default();
    if !status.success() || super::cli_runtime::validate_version(version).is_err() {
        return Err(format!(
            "질문 세션은 검증된 {SUPPORTED_VERSION}에서만 실행합니다 (확인된 버전: {}, 종료 상태: {status})", out.trim()
        ));
    }
    Ok(version.to_string())
}

struct Peer {
    input: Option<ChildStdin>,
    rx: mpsc::Receiver<Result<Value, String>>,
    queued: VecDeque<Value>,
    next: u64,
    control: Arc<Control>,
}
impl Peer {
    fn send(&mut self, value: &Value) -> Result<(), String> {
        let input = self
            .input
            .as_mut()
            .ok_or("Codex 입력 연결이 종료되었습니다")?;
        let mut bytes = serde_json::to_vec(value).map_err(|_| "요청 직렬화 실패")?;
        bytes.push(b'\n');
        let deadline = Instant::now() + CLOSE_TIMEOUT;
        let mut offset = 0;
        while offset < bytes.len() {
            match input.write(&bytes[offset..]) {
                Ok(0) => return Err("Codex 입력 연결 종료".into()),
                Ok(n) => offset += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err("Codex 쓰기 시간 초과. 답변을 자동 재전송하지 않습니다".into());
                    }
                    std::thread::sleep(POLL);
                }
                Err(_) => {
                    return Err(
                        "Codex 연결에 쓰지 못했습니다. 답변을 자동 재전송하지 않습니다".into(),
                    )
                }
            }
        }
        Ok(())
    }
    fn rpc(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next += 1;
        let id = format!("praxis-{}", self.next);
        self.send(&json!({"id":id,"method":method,"params":params}))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if self.control.cancelled.load(Ordering::SeqCst) {
                return Err("사용자가 실행을 중단했습니다".into());
            }
            if Instant::now() >= deadline {
                return Err(format!("Codex {method} 응답 시간 초과"));
            }
            match self.rx.recv_timeout(POLL) {
                Ok(Ok(v)) if v.get("id") == Some(&json!(id)) && v.get("method").is_none() => {
                    if v.get("error").is_some() {
                        return Err(format!(
                            "Codex {method} 요청 거절 (프로토콜/설정 확인 필요)"
                        ));
                    }
                    return v
                        .get("result")
                        .cloned()
                        .ok_or_else(|| "Codex 응답 형식 오류".into());
                }
                Ok(Ok(v)) => {
                    if self.queued.len() >= 128 {
                        return Err("Codex 초기 이벤트 한도 초과".into());
                    }
                    self.queued.push_back(v)
                }
                Ok(Err(e)) => return Err(e),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err("Codex 연결 종료".into()),
            }
        }
    }
    fn next(&mut self) -> Result<Option<Value>, String> {
        if let Some(v) = self.queued.pop_front() {
            return Ok(Some(v));
        }
        match self.rx.recv_timeout(POLL) {
            Ok(v) => v.map(Some),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(_) => Err("Codex 연결 종료".into()),
        }
    }
}

#[derive(Default)]
struct Events {
    texts: HashMap<String, String>,
    last_text: String,
    tools: HashSet<String>,
    commands: HashSet<String>,
    agents: HashMap<String, String>,
    started_questions: HashMap<String, Value>,
    unmatched: HashMap<String, (Value, Instant)>,
    raw_calls: HashMap<String, Option<String>>,
    code_cells: HashSet<String>,
    async_answers: HashMap<String, (Value, String)>,
    approval_items: HashMap<String, Value>,
    in_tokens: i64,
    out_tokens: i64,
}
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Codex 이벤트 필드 오류: {key}"))
}
fn owned(v: &Value, thread: &str, turn: &str) -> bool {
    v.get("threadId").and_then(Value::as_str) == Some(thread)
        && v.get("turnId").and_then(Value::as_str) == Some(turn)
}
fn terminal_agent(v: &Value) -> bool {
    matches!(
        v.as_str(),
        Some("completed" | "errored" | "shutdown" | "notFound")
    )
}

fn async_input(item: &Value, thread: &str, turn: &str) -> Result<Value, String> {
    let source = item["questions"].as_array();
    let mut questions = Vec::new();
    if let Some(source) = source {
        for (index, question) in source.iter().enumerate() {
            let options = question["options"]
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|label| json!({"label":label,"description":""}))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            questions.push(json!({"id":format!("question-{index}"),"header":"질문","question":field(question,"title")?,"options":options,"isOther":true,"isSecret":false}));
        }
    }
    if questions.is_empty() {
        questions.push(json!({"id":"question-0","header":"질문","question":field(item,"text")?,"options":[],"isOther":true,"isSecret":false}));
    }
    Ok(
        json!({"threadId":thread,"turnId":turn,"itemId":field(item,"id")?,"isBlocking":false,"questions":questions}),
    )
}

impl Events {
    fn raw_tool(&mut self, item: &Value, emit: &mut impl FnMut(ConvoEvent)) -> Result<(), String> {
        let kind = item["type"].as_str().unwrap_or_default();
        if matches!(kind, "custom_tool_call" | "function_call") {
            let name = item["name"].as_str().unwrap_or_default();
            let namespace = item["namespace"].as_str().unwrap_or_default();
            let code_mode =
                matches!(name, "exec" | "wait") && matches!(namespace, "" | "functions");
            // Computer Use already has public MCP items. Its raw mirror would duplicate
            // screenshots and completion events, so only consume Code Mode wrappers.
            if !code_mode && (kind == "function_call" || name == "apply_patch") {
                return Ok(());
            }
            let call = field(item, "call_id")?.to_string();
            let args = item["arguments"]
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok());
            let cell = args
                .as_ref()
                .and_then(|v| v["cell_id"].as_str())
                .map(str::to_string);
            if self.raw_calls.insert(call.clone(), cell).is_some() {
                return Err("중복 Code Mode 실행 이벤트입니다".into());
            }
            emit(ConvoEvent::ToolUse {
                name: if code_mode {
                    "Code mode".into()
                } else {
                    format!("Codex 도구 · {name}")
                },
                summary: item["input"]
                    .as_str()
                    .or_else(|| item["arguments"].as_str())
                    .unwrap_or(name)
                    .chars()
                    .take(2000)
                    .collect(),
                tool_id: Some(call),
                parent_id: None,
            });
        } else if matches!(kind, "custom_tool_call_output" | "function_call_output") {
            let call = field(item, "call_id")?;
            let Some(wait_cell) = self.raw_calls.remove(call) else {
                return Ok(());
            };
            let contents = super::tool_output::for_item(item);
            let output = contents
                .iter()
                .filter_map(|c| match c {
                    super::tool_output::Content::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            for line in output.lines() {
                if let Some(cell) = line.trim().strip_prefix("Script running with cell ID ") {
                    let cell = cell.trim().trim_end_matches('.');
                    if !cell.is_empty() && cell.len() <= 128 {
                        self.code_cells.insert(cell.to_string());
                    }
                }
            }
            let failed = output.contains("Script failed") || output.contains("Script error");
            if output.contains("Script completed") || output.contains("Script terminated") || failed
            {
                if let Some(cell) = wait_cell {
                    self.code_cells.remove(&cell);
                }
            }
            emit(ConvoEvent::ToolResult {
                summary: output.chars().take(2000).collect(),
                is_error: failed,
                result_chars: Some(output.chars().count() as i64),
                tool_use_id: Some(call.to_string()),
                parent_id: None,
            });
            if !contents.is_empty() {
                emit(ConvoEvent::ToolOutput {
                    tool_use_id: call.to_string(),
                    contents,
                });
            }
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    ctx: &Context,
    cwd: &str,
    message: &str,
    resume: Option<&str>,
    idle_timeout: u64,
    bin: &str,
    model: Option<&str>,
    effort: Option<&str>,
    service_tier: Option<&str>,
    images: &[String],
    mcp: Option<&PreviewMcpLease>,
    on_spawn: impl FnOnce(u32),
    mut on_event: impl FnMut(ConvoEvent),
) -> Result<TurnOutcome, String> {
    let version = runtime_version(bin)?;
    let expanded = version == super::cli_runtime::DEFAULT_VERSION;
    let mut command = Command::new(bin);
    command.args([
        "app-server",
        "--stdio",
        "-c",
        "features.code_mode_host=true",
    ]);
    if expanded {
        // Computer/browser/apps retain the user's configuration and managed policy.
        // Code Mode is the missing execution surface this adapter now understands.
        command.args([
            "-c",
            "features.code_mode=true",
            "-c",
            "features.default_mode_request_user_input=true",
            "-c",
            "features.send_message_to_user_async=true",
        ]);
    } else {
        command.args([
            "-c",
            "features.computer_use=false",
            "-c",
            "features.code_mode=false",
            "-c",
            "features.code_mode_only=false",
        ]);
    }
    crate::agent::service_tier::apply(&mut command, service_tier)?;
    // The host carries ordinary dynamic-tool RPCs even when the code-mode execution tool is off.
    // Historical threads keep their original execution contract.
    if let Some(mcp) = mcp {
        command.args(&mcp.injection().args);
        for (k, v) in &mcp.injection().env {
            command.env(k, v);
        }
    }
    command
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut native_helpers = NativeHelper::installed().into_iter().collect::<Vec<_>>();
    if let Some(home) = std::env::var_os("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| crate::usage::home_dir().map(|h| h.join(".codex")))
    {
        if let Some(helper) =
            NativeHelper::at(&home.join("plugins/.plugin-appserver/codex-code-mode-host"))
        {
            native_helpers.push(helper);
        }
    }
    let scope = TurnProcessScope::attach(&mut command);
    ctx.db(async {
        sqlx::query("UPDATE convo_executions SET process_marker=? WHERE id=?")
            .bind(scope.marker())
            .bind(&ctx.control.execution)
            .execute(&ctx.pool)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    })?;
    let mut child = ReapOnDrop::new(
        command
            .spawn()
            .map_err(|_| "Codex app-server를 시작하지 못했습니다")?,
    );
    let pid = child.id();
    let identity = crate::runner::process_identity::observe_group_leader(pid)
        .map_err(|_| "프로세스 소유권을 확인하지 못했습니다")?
        .ok_or("Codex가 초기화 전에 종료되었습니다")?;
    ctx.db(ledger::spawned(
        &ctx.pool,
        &ctx.control.execution,
        pid,
        &identity,
        scope.marker(),
    ))?;
    on_spawn(pid);
    let mut output = child.stdout.take().ok_or("Codex 출력 연결 없음")?;
    nonblocking(&output)?;
    let input = child.stdin.take().ok_or("Codex 입력 연결 없음")?;
    nonblocking(&input)?;
    let (tx, rx) = mpsc::sync_channel(8);
    let reader_stop = Arc::new(AtomicBool::new(false));
    let stop = reader_stop.clone();
    let reader_thread = std::thread::spawn(move || {
        let mut buffer = super::event_frame::FrameBuffer::default();
        let mut chunk = [0u8; 8192];
        while !stop.load(Ordering::SeqCst) {
            let n = match output.read(&mut chunk) {
                Ok(0) => {
                    let _ = tx.send(Err("Codex 연결 종료".into()));
                    break;
                }
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(POLL);
                    continue;
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    let _ = tx.send(Err("Codex 출력 읽기 실패".into()));
                    break;
                }
            };
            if let Err(error) = buffer.push(&chunk[..n], |line| {
                let value = serde_json::from_slice::<Value>(line)
                    .map_err(|_| "이벤트 JSON 오류".to_string())?;
                tx.send(Ok(value)).map_err(|_| "출력 수신 종료".to_string())
            }) {
                let _ = tx.send(Err(format!("Codex {error}")));
                break;
            }
        }
    });
    let reader = OutputReader {
        stop: reader_stop.clone(),
        handle: Some(reader_thread),
    };
    let mut peer = Peer {
        input: Some(input),
        rx,
        queued: VecDeque::new(),
        next: 0,
        control: ctx.control.clone(),
    };
    let mut last_text_emit = Instant::now() - Duration::from_secs(1);
    let mut events = Events::default();
    let mut thread_id = resume.unwrap_or_default().to_string();
    let mut interrupted = false;
    let mut managed = HashSet::new();
    let result = (|| -> Result<bool, String> {
        peer.rpc("initialize",json!({"clientInfo":{"name":"praxis","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}))?;
        peer.send(&json!({"method":"initialized","params":{}}))?;
        let mut params = json!({"cwd":cwd,"approvalPolicy":"never","sandbox":"danger-full-access","developerInstructions":instructions(expanded)});
        if expanded {
            // 0.157.1 does not project Code Mode calls into ThreadItem. Whitelist only
            // tool calls/outputs below; never retain raw reasoning or auth metadata.
            params["experimentalRawEvents"] = json!(true);
        }
        if let Some(tier) = crate::agent::service_tier::normalize(service_tier)? {
            params["serviceTier"] = json!(tier);
        }
        if let Some(model) = model {
            params["model"] = json!(model);
        }
        let response = if let Some(sid) = resume {
            params["threadId"] = json!(sid);
            // Praxis already owns the displayed transcript. This omits only the RPC
            // history payload, preserving the provider's model context and dynamic tools.
            params["excludeTurns"] = json!(true);
            peer.rpc("thread/resume", params)?
        } else {
            params["dynamicTools"] = json!([ledger::tool_spec()]);
            peer.rpc("thread/start", params)?
        };
        thread_id = field(&response["thread"], "id")?.to_string();
        if resume.is_some_and(|old| old != thread_id) {
            return Err("복원된 Codex 대화 ID가 일치하지 않습니다".into());
        }
        on_event(ConvoEvent::ModelSnapshot {
            requested: model.map(str::to_string),
            resolved: response
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string),
            source: "app_server".into(),
        });
        let mut input = vec![json!({"type":"text","text":message,"text_elements":[]})];
        for image in images {
            input.push(json!({"type":"localImage","path":image}));
        }
        let mut params = json!({"threadId":thread_id,"input":input});
        if let Some(tier) = crate::agent::service_tier::normalize(service_tier)? {
            params["serviceTier"] = json!(tier);
        }
        if let Some(model) = model {
            params["model"] = json!(model);
        }
        if let Some(effort) = effort {
            params["effort"] = json!(effort);
        }
        for (pid, _) in scope.members() {
            if let Some(path) = TurnProcessScope::executable(pid)
                .filter(|p| p.file_name().is_some_and(|n| n == "codex"))
            {
                if let Some(helper) = path
                    .parent()
                    .and_then(|p| NativeHelper::at(&p.join("codex-code-mode-host")))
                {
                    native_helpers.push(helper);
                }
            }
        }
        managed = scope.members().into_iter().collect::<HashSet<_>>();
        let start = peer.rpc("turn/start", params)?;
        let turn = field(&start["turn"], "id")?.to_string();
        ctx.db(ledger::started_on_side(
            &ctx.pool,
            &ctx.control.execution,
            &thread_id,
            &turn,
            ctx.control.side,
        ))?;
        on_event(ConvoEvent::SessionInit {
            session_id: thread_id.clone(),
        });
        ctx.changed();
        let mut last = Instant::now();
        let mut cancel_at = None;
        loop {
            if ctx.control.cancelled.load(Ordering::SeqCst) && cancel_at.is_none() {
                interrupted = true;
                ctx.db(ledger::phase(
                    &ctx.pool,
                    &ctx.control.execution,
                    "cancelling",
                ))?;
                ctx.db(ledger::close_questions(
                    &ctx.pool,
                    &ctx.control.execution,
                    "cancelled",
                ))?;
                ctx.changed();
                peer.send(&json!({"id":"praxis-interrupt","method":"turn/interrupt","params":{"threadId":thread_id,"turnId":turn}}))?;
                cancel_at = Some(Instant::now());
            }
            if cancel_at.is_some_and(|t: Instant| t.elapsed() > CLOSE_TIMEOUT) {
                return Err("Codex 중단 응답 시간 초과".into());
            }
            if cancel_at.is_none() {
                while let Some(answer) = ctx.db(ledger::take_dispatch(
                    &ctx.pool,
                    &ctx.control.execution,
                    crate::now(),
                ))? {
                    if ctx.control.cancelled.load(Ordering::SeqCst) {
                        break;
                    }
                    if answer.native_method.as_deref()
                        == Some(super::native_interaction::ASYNC_MESSAGE)
                    {
                        let rpc_id = format!("dojang-async-answer:{}", answer.answer_id);
                        let reply = format!(
                            "User response to your asynchronous question ({}):\n{}",
                            answer.call_id,
                            answer
                                .native_result
                                .as_ref()
                                .ok_or("비동기 답변 형식 오류")?
                        );
                        peer.send(&json!({"id":rpc_id,"method":"turn/steer","params":{"threadId":thread_id,"expectedTurnId":turn,
                            "input":[{"type":"text","text":reply,"text_elements":[]}]}}))?;
                        events
                            .async_answers
                            .insert(rpc_id, (answer.wire_id.clone(), answer.answer_id.clone()));
                    } else {
                        let result = answer.native_result.unwrap_or_else(|| json!({"success":true,"contentItems":[{"type":"inputText","text":answer.output}]}));
                        peer.send(&json!({"id":answer.wire_id,"result":result}))?;
                    }
                    ctx.db(ledger::written(&ctx.pool, &answer.answer_id))?;
                    ctx.changed();
                }
            }
            let (pending, expired) = ctx.db(ledger::pending(
                &ctx.pool,
                &ctx.control.execution,
                crate::now(),
            ))?;
            if expired {
                return Err("질문이 만료되어 실행을 중단했습니다".into());
            }
            if pending > 0 {
                last = Instant::now();
            } else if last.elapsed() > Duration::from_secs(idle_timeout) {
                return Err("Codex 무출력 시간 초과".into());
            }
            if events
                .unmatched
                .values()
                .any(|(_, at)| at.elapsed() > CLOSE_TIMEOUT)
            {
                return Err("질문 도구의 시작 이벤트를 확인하지 못했습니다".into());
            }
            let Some(v) = peer.next()? else { continue };
            last = Instant::now();
            let method = v.get("method").and_then(Value::as_str).unwrap_or("");
            let p = &v["params"];
            if method.is_empty() {
                if let Some((wire_id, answer_id)) = v["id"]
                    .as_str()
                    .and_then(|id| events.async_answers.remove(id))
                {
                    if v.get("error").is_some() {
                        ctx.db(ledger::reject_native_dispatch(&ctx.pool, &answer_id))?;
                    }
                    ctx.db(ledger::resolve_native(
                        &ctx.pool,
                        &ctx.control.execution,
                        &wire_id,
                    ))?;
                    ctx.changed();
                    continue;
                }
            }
            // Native server requests belong to this exact turn. Elicitation alone permits
            // a missing turnId in the published protocol, but still must name this thread.
            if expanded
                && v.get("id").is_some()
                && matches!(
                    method,
                    "item/tool/requestUserInput"
                        | "item/commandExecution/requestApproval"
                        | "item/fileChange/requestApproval"
                        | "item/permissions/requestApproval"
                        | "mcpServer/elicitation/request"
                )
            {
                if cancel_at.is_some() {
                    continue;
                }
                let scoped = owned(p, &thread_id, &turn)
                    || (method == "mcpServer/elicitation/request"
                        && p["threadId"] == thread_id
                        && p["turnId"].is_null());
                if !scoped {
                    return Err("다른 실행의 사용자 입력 요청입니다".into());
                }
                let mut request = p.clone();
                if let Some(item) = p["itemId"]
                    .as_str()
                    .and_then(|id| events.approval_items.get(id))
                {
                    if method == "item/commandExecution/requestApproval" {
                        for key in ["command", "cwd"] {
                            if request[key].is_null() {
                                request[key] = item[key].clone();
                            }
                        }
                    } else if method == "item/fileChange/requestApproval" {
                        request["changes"] = json!(item["changes"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|c| json!({"path":c["path"],"kind":c["kind"]}))
                            .collect::<Vec<_>>());
                    }
                }
                let id = ctx.db(ledger::open_native(
                    &ctx.pool,
                    &ctx.control.execution,
                    &v["id"],
                    method,
                    &request,
                    crate::now(),
                ))?;
                on_event(ConvoEvent::Interaction { interaction_id: id });
                ctx.changed();
                ctx.db(async {
                    ctx.question_opened().await;
                    Ok::<(), String>(())
                })?;
                continue;
            }
            if expanded && method == "serverRequest/resolved" && p["threadId"] == thread_id {
                if ctx.db(ledger::resolve_native(
                    &ctx.pool,
                    &ctx.control.execution,
                    &p["requestId"],
                ))? {
                    ctx.changed();
                }
                continue;
            }
            if expanded && method == "rawResponseItem/completed" {
                if owned(p, &thread_id, &turn) {
                    events.raw_tool(&p["item"], &mut on_event)?;
                }
                continue;
            }
            if method == "item/tool/call" {
                if cancel_at.is_some() {
                    continue;
                }
                if !owned(p, &thread_id, &turn)
                    || p["namespace"] != "praxis_ui"
                    || p["tool"] != "ask_user"
                {
                    return Err("지원하지 않는 도구 입력 요청입니다".into());
                }
                ledger::validate_questions(&p["arguments"])?;
                let call = field(p, "callId")?.to_string();
                if events
                    .started_questions
                    .get(&call)
                    .is_some_and(|args| args == &p["arguments"])
                {
                    on_event(ctx.open(&v["id"], &call, &p["arguments"])?);
                } else if events.unmatched.len() >= 8
                    || events.unmatched.insert(call, (v, Instant::now())).is_some()
                {
                    return Err("중복 또는 과도한 질문 요청입니다".into());
                }
                continue;
            }
            if v.get("id").is_some() && !method.is_empty() {
                return Err(
                    "지원하지 않는 승인 또는 사용자 입력 요청입니다. 실행을 중단합니다".into(),
                );
            }
            if method == "turn/completed" && p["threadId"] == thread_id && p["turn"]["id"] == turn {
                let blocking = ctx
                    .db(ledger::blocking_pending(
                        &ctx.pool,
                        &ctx.control.execution,
                        crate::now(),
                    ))?
                    .0;
                if blocking > 0
                    || !events.unmatched.is_empty()
                    || !events.started_questions.is_empty()
                {
                    return Err("응답되지 않은 질문이 남은 채 실행이 종료되었습니다".into());
                }
                if !events.commands.is_empty()
                    || !events.tools.is_empty()
                    || events.agents.values().any(|s| !terminal_agent(&json!(s)))
                    || !events.raw_calls.is_empty()
                    || !events.code_cells.is_empty()
                {
                    return Err(
                        "미완료 명령 또는 작업자가 남아 정상 완료로 처리하지 않았습니다".into(),
                    );
                }
                return Ok(p["turn"]["status"] == "completed" && !interrupted);
            }
            if method == "error" && !p["willRetry"].as_bool().unwrap_or(false) {
                return Err("Codex 실행 오류가 발생했습니다 (인증·모델·연결을 확인하세요)".into());
            }
            if method.starts_with("item/") && !owned(p, &thread_id, &turn) {
                continue;
            }
            // Avoid replaying foreign subagent/turn items into this conversation.
            if method.starts_with("item/") && !owned(p, &thread_id, &turn) {
                continue;
            }
            match method {
                "warning" if p["threadId"].is_null() || p["threadId"] == thread_id => {
                    if let Some(message) = p["message"].as_str() {
                        on_event(ConvoEvent::ToolResult {
                            summary: format!(
                                "Codex: {}",
                                message.chars().take(2000).collect::<String>()
                            ),
                            is_error: true,
                            tool_use_id: None,
                            result_chars: None,
                            parent_id: None,
                        });
                    }
                }
                "mcpServer/startupStatus/updated"
                    if p["threadId"] == thread_id && p["status"] == "failed" =>
                {
                    on_event(ConvoEvent::ToolResult {
                        summary: format!(
                            "MCP {} 연결 실패: {}",
                            p["name"].as_str().unwrap_or("도구"),
                            p["error"]
                                .as_str()
                                .unwrap_or("연결 설정과 인증 상태를 확인하세요")
                                .chars()
                                .take(1500)
                                .collect::<String>()
                        ),
                        is_error: true,
                        tool_use_id: None,
                        result_chars: None,
                        parent_id: None,
                    });
                }
                "item/agentMessage/delta" => {
                    let id = field(p, "itemId")?.to_string();
                    let text = events.texts.entry(id.clone()).or_default();
                    text.push_str(field(p, "delta")?);
                    if text.len() > MAX_MESSAGE_BYTES {
                        return Err("메시지 크기 제한 초과".into());
                    }
                    if last_text_emit.elapsed() >= Duration::from_millis(80) {
                        on_event(ConvoEvent::TextUpdate {
                            item_id: id,
                            text: text.clone(),
                            complete: false,
                        });
                        last_text_emit = Instant::now();
                    }
                }
                "item/started" => {
                    let item = &p["item"];
                    let id = field(item, "id")?.to_string();
                    if matches!(
                        item["type"].as_str(),
                        Some("commandExecution" | "fileChange")
                    ) {
                        events.approval_items.insert(id.clone(), item.clone());
                    }
                    match item["type"].as_str() {
                        Some("dynamicToolCall") => {
                            if item["namespace"] != "praxis_ui" || item["tool"] != "ask_user" {
                                return Err("등록되지 않은 동적 도구입니다".into());
                            }
                            ledger::validate_questions(&item["arguments"])?;
                            if events
                                .started_questions
                                .insert(id.clone(), item["arguments"].clone())
                                .is_some()
                            {
                                return Err("중복 질문 시작 이벤트".into());
                            }
                            if let Some((request, _)) = events.unmatched.remove(&id) {
                                if request["params"]["arguments"] != item["arguments"] {
                                    return Err("질문 시작과 요청 내용이 일치하지 않습니다".into());
                                }
                                on_event(ctx.open(
                                    &request["id"],
                                    &id,
                                    &request["params"]["arguments"],
                                )?);
                            }
                        }
                        Some("commandExecution") => {
                            events.commands.insert(id.clone());
                            on_event(ConvoEvent::ToolUse {
                                name: "Bash".into(),
                                summary: item["command"].as_str().unwrap_or("command").into(),
                                tool_id: Some(id),
                                parent_id: None,
                            });
                        }
                        Some(
                            "fileChange"
                            | "mcpToolCall"
                            | "collabAgentToolCall"
                            | "imageGeneration"
                            | "imageView"
                            | "webSearch"
                            | "functionCallOutput"
                            | "sleep",
                        ) => {
                            events.tools.insert(id.clone());
                            on_event(ConvoEvent::ToolUse {
                                name: item["type"].as_str().unwrap_or("tool").into(),
                                summary: item["tool"].as_str().unwrap_or("작업 실행").into(),
                                tool_id: Some(id),
                                parent_id: None,
                            });
                        }
                        Some(kind) if kind.ends_with("Call") || kind.ends_with("Execution") => {
                            return Err("지원하지 않는 실행 도구 이벤트입니다".into())
                        }
                        _ => {}
                    }
                }
                "item/completed" => {
                    let item = &p["item"];
                    let id = field(item, "id")?.to_string();
                    events.approval_items.remove(&id);
                    match item["type"].as_str() {
                        Some("agentMessage") => {
                            let text = field(item, "text")?.to_string();
                            events.last_text = text.clone();
                            events.texts.remove(&id);
                            on_event(ConvoEvent::TextUpdate {
                                item_id: id,
                                text,
                                complete: true,
                            });
                            if expanded && item["delivery"] == "async" {
                                let params = async_input(item, &thread_id, &turn)?;
                                let request_id =
                                    json!(format!("dojang-async:{}", field(item, "id")?));
                                let interaction_id = ctx.db(ledger::open_native(
                                    &ctx.pool,
                                    &ctx.control.execution,
                                    &request_id,
                                    super::native_interaction::ASYNC_MESSAGE,
                                    &params,
                                    crate::now(),
                                ))?;
                                on_event(ConvoEvent::Interaction { interaction_id });
                                ctx.changed();
                                ctx.db(async {
                                    ctx.question_opened().await;
                                    Ok::<(), String>(())
                                })?;
                            }
                        }
                        Some("dynamicToolCall") => {
                            if item["namespace"] != "praxis_ui"
                                || item["tool"] != "ask_user"
                                || events.started_questions.remove(&id).is_none()
                            {
                                return Err("질문 완료 이벤트 소유권 오류".into());
                            }
                            if ctx.db(ledger::acknowledge(
                                &ctx.pool,
                                &ctx.control.execution,
                                &id,
                                &item["contentItems"],
                                item["success"].as_bool() == Some(true),
                            ))? {
                                ctx.changed();
                            }
                        }
                        Some("commandExecution") => {
                            events.commands.remove(&id);
                            on_event(ConvoEvent::ToolResult {
                                summary: item["aggregatedOutput"]
                                    .as_str()
                                    .unwrap_or_default()
                                    .chars()
                                    .take(2000)
                                    .collect(),
                                is_error: item["status"] == "failed"
                                    || item["exitCode"].as_i64().is_some_and(|v| v != 0),
                                tool_use_id: Some(id),
                                result_chars: None,
                                parent_id: None,
                            });
                        }
                        Some(
                            "fileChange"
                            | "mcpToolCall"
                            | "collabAgentToolCall"
                            | "imageGeneration"
                            | "imageView"
                            | "webSearch"
                            | "functionCallOutput"
                            | "sleep",
                        ) => {
                            events.tools.remove(&id);
                            if let Some(agents) = item["agentsStates"].as_object() {
                                for (id, state) in agents {
                                    events.agents.insert(
                                        id.clone(),
                                        state["status"].as_str().unwrap_or("unknown").into(),
                                    );
                                }
                            }
                            on_event(ConvoEvent::ToolResult {
                                summary: super::tool_output::summary(item),
                                is_error: item["status"] == "failed"
                                    || !item["error"].is_null()
                                    || !item["failure"].is_null(),
                                tool_use_id: Some(id.clone()),
                                result_chars: None,
                                parent_id: None,
                            });
                            let contents = super::tool_output::for_item(item);
                            if !contents.is_empty() {
                                on_event(ConvoEvent::ToolOutput {
                                    tool_use_id: id,
                                    contents,
                                });
                            }
                        }
                        Some(kind) if kind.ends_with("Call") || kind.ends_with("Execution") => {
                            return Err("지원하지 않는 실행 도구 이벤트입니다".into())
                        }
                        _ => {}
                    }
                }
                "turn/plan/updated" => {
                    if owned(p, &thread_id, &turn) {
                        let items = p["plan"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .map(|i| super::PlanItem {
                                        content: i["step"].as_str().unwrap_or_default().into(),
                                        status: match i["status"].as_str() {
                                            Some("completed") => "completed",
                                            Some("inProgress") => "in_progress",
                                            _ => "pending",
                                        }
                                        .into(),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        on_event(ConvoEvent::Plan {
                            items,
                            parent_id: None,
                        });
                    }
                }
                "thread/tokenUsage/updated" => {
                    events.in_tokens = p["tokenUsage"]["last"]["inputTokens"].as_i64().unwrap_or(0);
                    events.out_tokens = p["tokenUsage"]["last"]["outputTokens"]
                        .as_i64()
                        .unwrap_or(0);
                    on_event(ConvoEvent::ContextUsage {
                        context_tokens: p["tokenUsage"]["last"]["totalTokens"]
                            .as_i64()
                            .unwrap_or(0),
                        context_window: p["tokenUsage"]["modelContextWindow"].as_i64(),
                        observed_at: Some(chrono::Utc::now().timestamp_millis()),
                        source: Some("app_server".into()),
                        valid: Some(true),
                    });
                }
                _ => {}
            }
        }
    })();
    let finalizing = ctx.db(ledger::phase(
        &ctx.pool,
        &ctx.control.execution,
        "finalizing",
    ));
    ctx.changed();
    let question_close = ctx.db(ledger::close_questions(
        &ctx.pool,
        &ctx.control.execution,
        if interrupted {
            "cancelled"
        } else {
            "turn_ended"
        },
    ));
    let drained = mcp.is_none_or(|m| m.revoke_and_drain(CLOSE_TIMEOUT));
    // Keep evidence before teardown. The app-server's own group is excluded.
    let survivors = scope
        .members()
        .into_iter()
        .filter(|member| {
            !managed.contains(member) && !native_helpers.iter().any(|helper| helper.matches(member))
        })
        .collect::<Vec<_>>();
    let survivor_programs = survivors
        .iter()
        .map(|(pid, _)| TurnProcessScope::member_program(*pid))
        .collect::<Vec<_>>()
        .join(", ");
    peer.input.take();
    let deadline = Instant::now() + CLOSE_TIMEOUT;
    while child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
        std::thread::sleep(POLL);
    }
    if child.try_wait().ok().flatten().is_none() {
        crate::verify::kill_group(pid);
    }
    let descendants_clean = scope.cleanup();
    drop(scope);
    let _ = child.wait();
    reader_stop.store(true, Ordering::SeqCst);
    drop(peer);
    drop(reader);
    let clean = crate::verify::process_group_terminated_checked(pid).unwrap_or(false)
        && drained
        && descendants_clean;
    if !clean || question_close.is_err() || finalizing.is_err() {
        ctx.control.cleanup_failed.store(true, Ordering::SeqCst);
        let _ = ctx.db(ledger::phase(
            &ctx.pool,
            &ctx.control.execution,
            "cleanup_failed",
        ));
        ctx.changed();
        return Err(
            "프로세스 또는 도구 요청 정리를 확인하지 못했습니다. 승인·폐기가 잠겨 있습니다".into(),
        );
    }
    let outcome = result.and_then(|success| {
        if survivors.is_empty() {
            Ok(success)
        } else {
            Err(format!("턴 종료 시 남은 자식 프로세스를 회수했습니다 ({survivor_programs}). 정상 완료로 처리하지 않습니다"))
        }
    });
    let final_error = outcome.as_ref().err().cloned();
    if let Err(e) = ctx.db(ledger::finish(
        &ctx.pool,
        &ctx.control.execution,
        if interrupted {
            "cancelled"
        } else if outcome.as_ref().is_ok_and(|s| *s) {
            "completed"
        } else {
            "failed"
        },
        final_error.as_deref(),
    )) {
        ctx.control.cleanup_failed.store(true, Ordering::SeqCst);
        return Err(e);
    };
    let success = outcome?;
    on_event(ConvoEvent::Result {
        text: if interrupted {
            "사용자가 턴을 중단했습니다".into()
        } else if success {
            events.last_text
        } else {
            "Codex 턴이 실패했습니다".into()
        },
        is_error: !success,
        session_id: thread_id.clone(),
        cost_usd: 0.0,
        num_turns: 1,
        tokens_in: events.in_tokens,
        tokens_out: events.out_tokens,
    });
    ctx.changed();
    Ok(TurnOutcome {
        session_id: thread_id,
        timed_out: false,
        exit_desc: "app-server closed".into(),
        stderr_tail: String::new(),
        halted: None,
    })
}

#[cfg(unix)]
fn nonblocking(pipe: &impl std::os::fd::AsRawFd) -> Result<(), String> {
    use nix::libc::{fcntl, F_GETFL, F_SETFL, O_NONBLOCK};
    let fd = pipe.as_raw_fd();
    // SAFETY: both operations affect only the live pipe descriptor owned by this adapter.
    let flags = unsafe { fcntl(fd, F_GETFL) };
    if flags < 0 || unsafe { fcntl(fd, F_SETFL, flags | O_NONBLOCK) } < 0 {
        return Err("파이프 제한시간 설정 실패".into());
    }
    Ok(())
}

/// Retry only cleanup; a lost stdio request is never replayed or adopted.
pub async fn recover_execution(pool: &SqlitePool, task: i64) -> Result<(), String> {
    use sqlx::Row;
    let rows=sqlx::query("SELECT id,pgid,identity_hash,process_marker FROM convo_executions WHERE task_id=? AND state IN ('starting','running','cancelling','finalizing','cleanup_failed')")
        .bind(task).fetch_all(pool).await.map_err(|e|e.to_string())?;
    for row in rows {
        let execution: String = row.get("id");
        ledger::phase(pool, &execution, "cleanup_failed").await?;
        ledger::close_questions(pool, &execution, "connection_lost").await?;
        let marker: Option<String> = row.get("process_marker");
        // Verify and terminate the provider before marker cleanup. The latter also
        // kills the provider; probing/killing that unreaped zombie again yields
        // EPERM on macOS and incorrectly quarantines an otherwise recovered thread.
        if let Some(pid) = row.get::<Option<i64>, _>("pgid") {
            let identity: String = row
                .get::<Option<String>, _>("identity_hash")
                .ok_or("프로세스 식별 증거가 없습니다")?;
            if crate::runner::process_identity::terminate_if_matches(pid, &identity)
                .await
                .map_err(|e| e.to_string())?
                == crate::runner::process_identity::ProcessTerminationOutcome::IdentityMismatch
            {
                return Err("프로세스 식별 증거가 달라 자동 회수하지 않았습니다".into());
            }
        }
        if let Some(marker) = marker {
            let clean = tauri::async_runtime::spawn_blocking(move || {
                TurnProcessScope::recover(&marker).map(|scope| scope.cleanup())
            })
            .await
            .map_err(|e| e.to_string())??;
            if !clean {
                return Err("소유 자식 프로세스 회수가 끝나지 않았습니다".into());
            }
        }
        ledger::finish(pool, &execution, "failed", Some("connection_lost")).await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn run_selected(
    ctx: Option<&Context>,
    cwd: &str,
    message: &str,
    resume: Option<&str>,
    idle_timeout: u64,
    vendor: super::Vendor,
    bin: &str,
    model: Option<&str>,
    effort: Option<&str>,
    service_tier: Option<&str>,
    images: &[String],
    session_name: Option<&str>,
    mcp: Option<&PreviewMcpLease>,
    on_spawn: impl FnOnce(u32),
    on_event: impl FnMut(ConvoEvent),
) -> Result<TurnOutcome, String> {
    let Some(ctx) = ctx else {
        return super::run_turn_with_effort(
            cwd,
            message,
            resume,
            idle_timeout,
            vendor,
            bin,
            model,
            effort,
            service_tier,
            images,
            session_name,
            mcp.map(|m| m.injection()),
            on_spawn,
            on_event,
        );
    };
    // 벤더가 전송을 고른다. Codex는 app-server stdio, 그 밖은 인앱 MCP 툴로 질문한다.
    let result = if vendor == super::Vendor::Codex {
        run(
            ctx,
            cwd,
            message,
            resume,
            idle_timeout,
            bin,
            model,
            effort,
            service_tier,
            images,
            mcp,
            on_spawn,
            on_event,
        )
    } else {
        super::question_local::run(
            ctx,
            cwd,
            message,
            resume,
            idle_timeout,
            vendor,
            bin,
            model,
            effort,
            service_tier,
            images,
            session_name,
            mcp,
            on_spawn,
            on_event,
        )
    };
    // Also covers initialization failures before the main event loop exists.
    if result.is_err() && !ctx.control.cleanup_failed.load(Ordering::SeqCst) {
        let drained = mcp.is_none_or(|m| m.revoke_and_drain(CLOSE_TIMEOUT));
        if !drained || ctx.db(recover_execution(&ctx.pool, ctx.task_id)).is_err() {
            ctx.control.cleanup_failed.store(true, Ordering::SeqCst);
            let _ = ctx.db(ledger::phase(
                &ctx.pool,
                &ctx.control.execution,
                "cleanup_failed",
            ));
        }
        ctx.changed();
    }
    result
}

pub fn mark_cleanup_failed(task: i64) {
    if let Some(c) = controls()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&task)
    {
        c.cleanup_failed.store(true, Ordering::SeqCst);
    }
}

#[cfg(not(unix))]
fn nonblocking<T>(_: &T) -> Result<(), String> {
    Err("양방향 파이프는 이 플랫폼에서 지원하지 않습니다".into())
}

struct OutputReader {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}
impl Drop for OutputReader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// The installed native client starts lazily even when computer_use is disabled in 0.154.0.
/// Pin its actual executable path and bytes BEFORE spawn. A process name/argv is never proof.
/// All matched helpers still carry this turn marker and are killed/reaped with the provider.
struct NativeHelper {
    path: std::path::PathBuf,
    digest: Vec<u8>,
}
impl NativeHelper {
    fn installed() -> Option<Self> {
        if !cfg!(target_os = "macos") {
            return None;
        }
        let home = std::env::var_os("CODEX_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".codex"))
            })?;
        let path=home.join("computer-use/Codex Computer Use.app/Contents/SharedSupport/SkyComputerUseClient.app/Contents/MacOS/SkyComputerUseClient").canonicalize().ok()?;
        Self::at(&path)
    }
    fn at(path: &std::path::Path) -> Option<Self> {
        let path = path.canonicalize().ok()?;
        let digest = Self::digest(&path)?;
        Some(Self { path, digest })
    }
    fn digest(path: &std::path::Path) -> Option<Vec<u8>> {
        use sha2::{Digest, Sha256};
        let mut input = std::fs::File::open(path).ok()?;
        let mut hash = Sha256::new();
        std::io::copy(&mut input, &mut hash).ok()?;
        Some(hash.finalize().to_vec())
    }
    fn matches(&self, member: &(u32, String)) -> bool {
        let Some(path) = TurnProcessScope::executable(member.0).and_then(|p| p.canonicalize().ok())
        else {
            return false;
        };
        path == self.path
            && Self::digest(&path).as_ref() == Some(&self.digest)
            && crate::runner::process_identity::observe(member.0)
                .ok()
                .flatten()
                .as_ref()
                == Some(&member.1)
    }
}

#[cfg(test)]
#[path = "app_server_tests.rs"]
mod tests;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn check_version(_: &str) -> Result<(), String> {
    Err("질문 세션은 현재 macOS/Linux에서만 지원합니다".into())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn runtime_version(_: &str) -> Result<String, String> {
    Err("질문 세션은 현재 macOS/Linux에서만 지원합니다".into())
}
