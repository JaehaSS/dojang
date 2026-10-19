//! MCP JSON-RPC protocol as pure functions: no transport, no dispatch, no state.

use serde_json::{json, Value};

/// Returned when the client asks for a version we do not recognize.
pub const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";
const SUPPORTED_PROTOCOL_VERSIONS: [&str; 2] = ["2025-06-18", "2025-11-25"];

#[derive(Debug, PartialEq)]
pub enum Request {
    Initialize {
        id: Value,
    },
    Initialized,
    ToolsList {
        id: Value,
    },
    ToolCall {
        id: Value,
        name: String,
        arguments: Value,
    },
    Ping {
        id: Value,
    },
    Unknown {
        id: Value,
        method: String,
    },
}

pub fn classify(request: &Value) -> Request {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match method {
        "initialize" => Request::Initialize { id },
        "notifications/initialized" => Request::Initialized,
        "tools/list" => Request::ToolsList { id },
        "ping" => Request::Ping { id },
        "tools/call" => Request::ToolCall {
            id,
            name: string_param(request, "name"),
            arguments: request
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or_else(|| json!({})),
        },
        other => Request::Unknown {
            id,
            method: other.to_string(),
        },
    }
}

/// `None` means "notification, send no body". `tools/call` is answered by the
/// caller, which classifies first and dispatches to the preview webview.
pub fn handle_request(request: &Value, tools: &Tools) -> Option<Value> {
    match classify(request) {
        Request::Initialized => None,
        Request::Initialize { id } => Some(success(id, initialize_result(request))),
        Request::ToolsList { id } => Some(success(id, json!({ "tools": tools.descriptors }))),
        Request::Ping { id } => Some(success(id, json!({}))),
        Request::ToolCall { id, .. } => Some(tool_error(
            id,
            -32603,
            "tools/call must be dispatched by the caller",
        )),
        Request::Unknown { id, method } => Some(tool_error(
            id,
            -32601,
            format!("method not found: {method}"),
        )),
    }
}

pub fn tool_result(id: Value, text: impl Into<String>) -> Value {
    success(
        id,
        json!({ "content": [{ "type": "text", "text": text.into() }] }),
    )
}

/// A tool that ran and refused: the call succeeded, the action did not. `isError` keeps the
/// reason readable to the agent while marking the result as a failure it may retry.
pub fn tool_failure(id: Value, text: impl Into<String>) -> Value {
    success(
        id,
        json!({ "content": [{ "type": "text", "text": text.into() }], "isError": true }),
    )
}

/// Tool failures are JSON-RPC errors, not `isError` results, so the agent sees them as errors.
pub fn tool_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() },
    })
}

fn success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn string_param(request: &Value, key: &str) -> String {
    request
        .pointer(&format!("/params/{key}"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Echo the client's version when we support it; Task 0 measured both CLIs proceeding on an echo.
fn initialize_result(request: &Value) -> Value {
    let requested = string_param(request, "protocolVersion");
    let version = SUPPORTED_PROTOCOL_VERSIONS
        .iter()
        .find(|supported| **supported == requested)
        .copied()
        .unwrap_or(DEFAULT_PROTOCOL_VERSION);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "praxis-preview", "version": env!("CARGO_PKG_VERSION") },
    })
}

#[derive(Debug, Clone)]
pub struct Tools {
    descriptors: Vec<Value>,
}

impl Tools {
    /// 이 턴에만 존재하는 툴을 얹은 사본. 원본은 그대로 둔다 — `McpState`는 공유물이다.
    pub fn with(&self, extra: Value) -> Self {
        let mut tools = self.clone();
        tools.descriptors.push(extra);
        tools
    }

    /// 리뷰 노트·이동 툴을 얹은 사본. 브라우저 단계와 별개라 `phase_*`에 섞지 않는다 —
    /// 저장소가 있는 서버만 켠다(`McpState::review`가 `NoReview`면 호출이 실패로 돌아간다).
    pub fn with_review(mut self) -> Self {
        self.descriptors.extend(super::review::descriptors());
        self
    }

    pub fn phase_c() -> Self {
        Self {
            descriptors: vec![
                json!({
                    "name": "browser_navigate",
                    "description": "Open this task’s Praxis Preview automatically if absent, otherwise reuse and reveal it, then navigate to a loopback http URL. Returns navigation_started, not page readiness; use browser_wait_for or browser_snapshot to verify the loaded page before interacting. A busy response is retryable; taken_over requires the user to release control.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "url": { "type": "string", "description": "http://localhost:<port>/..." }
                        },
                        "required": ["url"],
                        "additionalProperties": false,
                    },
                }),
                json!({
                    "name": "browser_snapshot",
                    "description": "Return an accessibility snapshot of the preview page. If no_preview is returned, use browser_navigate with the project’s known loopback dev URL to open Praxis Preview automatically.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {},
                        "additionalProperties": false,
                    },
                }),
            ],
        }
    }

    /// Phase C의 둘 뒤에 액션 셋을 잇는다 — 목록 순서가 곧 에이전트가 읽는 순서다.
    pub fn phase_e() -> Self {
        let mut tools = Self::phase_c();
        tools.descriptors.extend([
            json!({
                "name": "browser_click",
                "description": "Click the element named by a ref from browser_snapshot. Returns changed plus a fresh snapshot; every action invalidates refs from earlier snapshots.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "ref": { "type": "string", "description": "Element ref from the latest browser_snapshot, e.g. s1e3." } },
                    "required": ["ref"],
                    "additionalProperties": false,
                },
            }),
            json!({
                "name": "browser_fill",
                "description": "Replace the value of the input, textarea or select named by a ref from browser_snapshot. Returns changed plus a fresh snapshot; every action invalidates refs from earlier snapshots.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "ref": { "type": "string", "description": "Element ref from the latest browser_snapshot, e.g. s1e3." },
                        "text": { "type": "string", "description": "Value to set, up to 64 KiB." },
                    },
                    "required": ["ref", "text"],
                    "additionalProperties": false,
                },
            }),
            json!({
                "name": "browser_press_key",
                "description": "Send a key to the element named by a ref from browser_snapshot, or to the focused element when ref is omitted. Returns changed plus a fresh snapshot; every action invalidates refs from earlier snapshots.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "key": { "type": "string", "description": "KeyboardEvent.key value, e.g. Enter or ArrowDown." },
                        "ref": { "type": "string", "description": "Element ref from the latest browser_snapshot, e.g. s1e3." },
                    },
                    "required": ["key"],
                    "additionalProperties": false,
                },
            }),
        ]);
        tools
    }

    /// Phase E의 다섯 뒤에 대기와 콘솔을 잇는다 — 타이밍과 에러를 다루는 툴들이다.
    pub fn phase_f() -> Self {
        let mut tools = Self::phase_e();
        tools.descriptors.extend([
            json!({
                "name": "browser_wait_for",
                "description": "Wait until text appears on the page, or until gone disappears from it; give at least one of them. Returns satisfied plus a fresh snapshot — running out of time is satisfied: false, not an error.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "text": { "type": "string", "description": "Text to wait for, 1-1024 characters." },
                        "gone": { "type": "string", "description": "Text to wait to disappear, 1-1024 characters." },
                        "timeout": { "type": "integer", "minimum": 1, "maximum": 60, "description": "Seconds to wait before giving up. Default 10." },
                    },
                    "additionalProperties": false,
                },
            }),
            json!({
                "name": "browser_console",
                "description": "Return recent console output and page errors collected since the page loaded, up to 200 entries. Set clear to reset the buffer after reading.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "clear": { "type": "boolean", "description": "Empty the buffer after returning these entries." },
                    },
                    "additionalProperties": false,
                },
            }),
        ]);
        tools
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn initialize_returns_supported_version_and_tools_capability() {
        let req = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}});
        let resp = handle_request(&req, &Tools::phase_c()).unwrap();
        assert_eq!(resp["result"]["protocolVersion"], "2025-11-25");
        assert!(resp["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn unsupported_client_version_falls_back_to_default() {
        let req = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}});
        let resp = handle_request(&req, &Tools::phase_c()).unwrap();
        assert_eq!(resp["result"]["protocolVersion"], "2025-06-18");
    }

    #[test]
    fn initialized_notification_has_no_response() {
        let req = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
        assert!(handle_request(&req, &Tools::phase_c()).is_none());
    }

    #[test]
    fn tools_list_names_phase_c_tools() {
        let req = json!({"jsonrpc":"2.0","id":2,"method":"tools/list"});
        let names: Vec<_> = handle_request(&req, &Tools::phase_c()).unwrap()["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["browser_navigate", "browser_snapshot"]);
    }

    #[test]
    fn tools_list_names_phase_e_tools() {
        let req = json!({"jsonrpc":"2.0","id":2,"method":"tools/list"});
        let tools = handle_request(&req, &Tools::phase_e()).unwrap()["result"]["tools"].clone();
        let names: Vec<_> = tools
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            names,
            vec![
                "browser_navigate",
                "browser_snapshot",
                "browser_click",
                "browser_fill",
                "browser_press_key"
            ]
        );
        assert_eq!(tools[3]["inputSchema"]["required"], json!(["ref", "text"]));
        assert_eq!(tools[4]["inputSchema"]["required"], json!(["key"]));
    }

    #[test]
    fn tools_list_names_phase_f_tools() {
        let req = json!({"jsonrpc":"2.0","id":2,"method":"tools/list"});
        let tools = handle_request(&req, &Tools::phase_f()).unwrap()["result"]["tools"].clone();
        let names: Vec<_> = tools
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            names,
            vec![
                "browser_navigate",
                "browser_snapshot",
                "browser_click",
                "browser_fill",
                "browser_press_key",
                "browser_wait_for",
                "browser_console"
            ]
        );
        assert_eq!(tools[5]["inputSchema"]["properties"]["timeout"]["maximum"], 60);
        assert_eq!(
            tools[6]["inputSchema"]["properties"]["clear"]["type"],
            "boolean"
        );
    }

    #[test]
    fn ping_answers_with_an_empty_result() {
        let req = json!({"jsonrpc":"2.0","id":9,"method":"ping"});
        assert_eq!(
            handle_request(&req, &Tools::phase_c()).unwrap()["result"],
            json!({})
        );
    }

    #[test]
    fn unknown_method_is_minus_32601() {
        let req = json!({"jsonrpc":"2.0","id":3,"method":"server/discover"});
        assert_eq!(
            handle_request(&req, &Tools::phase_c()).unwrap()["error"]["code"],
            -32601
        );
    }

    #[test]
    fn tools_call_is_routed_to_caller_not_answered_here() {
        let req = json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"browser_snapshot","arguments":{}}});
        assert!(
            matches!(classify(&req), Request::ToolCall { id, name, .. } if id == json!(4) && name == "browser_snapshot")
        );
    }
}
