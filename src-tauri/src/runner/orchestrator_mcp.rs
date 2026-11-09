//! 상시 비서 전용 내장 관제 MCP 서버(`POST /v1/mcp/orchestrator`) 및 `.mcp.json` 동기화.
//!
//! `claude`, `codex`, `agy` 어느 벤더로 전환하더라도 동일한 MCP 도구 7종으로 코딩 작업을
//! 관제·위임하고, 일정·리마인더를 관리하며, 파괴적 작업에 대한 사용자 인라인 승인을 요청한다.

use std::path::Path;

use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::db;
use crate::preview_bridge::mcp::protocol;
use crate::runner::actions::SharedTaskActions;
use crate::runner::assistant;
use crate::runner::config::RunnerConfig;
use crate::runner::http::RunnerHttpState;
use crate::runner::QueuedTaskRequest;

const PROTOCOL_VERSION: &str = "2024-11-05";
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

#[derive(Debug, Deserialize)]
struct RpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Deserialize)]
struct CallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

/// `config.assistant_workspace`(또는 첫 번째 `repository_roots`)의 `.mcp.json`에
/// `dojang-orchestrator` HTTP MCP 엔드포인트와 `assistant_mcp_config` 외부 서버들을 병합한다.
pub fn sync_assistant_mcp_config(config: &RunnerConfig) -> Result<(), String> {
    let Some(workspace) = config
        .assistant_workspace
        .as_deref()
        .or_else(|| config.repository_roots.first().map(|p| p.as_path()))
    else {
        return Ok(());
    };
    sync_assistant_mcp_config_at(workspace, config)
}

pub fn sync_assistant_mcp_config_at(workspace: &Path, config: &RunnerConfig) -> Result<(), String> {
    if !workspace.is_dir() {
        return Ok(());
    }
    let mcp_path = workspace.join(".mcp.json");
    let mut root = std::fs::read_to_string(&mcp_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .filter(|val| val.is_object())
        .unwrap_or_else(|| json!({ "mcpServers": {} }));

    if !root.get("mcpServers").is_some_and(|v| v.is_object()) {
        root["mcpServers"] = json!({});
    }

    if let Some(ext_path) = config.assistant_mcp_config.as_deref() {
        if let Ok(ext_raw) = std::fs::read_to_string(ext_path) {
            if let Ok(ext_json) = serde_json::from_str::<Value>(&ext_raw) {
                if let Some(ext_servers) = ext_json.get("mcpServers").and_then(|v| v.as_object()) {
                    if let Some(target) = root["mcpServers"].as_object_mut() {
                        for (name, spec) in ext_servers {
                            target.insert(name.clone(), spec.clone());
                        }
                    }
                }
            }
        }
    }

    let token = std::fs::read_to_string(&config.pairing_token_file)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let mut server_entry = json!({
        "type": "http",
        "url": format!("http://{}/v1/mcp/orchestrator", config.bind),
    });
    if let Some(bearer) = token {
        server_entry["headers"] = json!({
            "Authorization": format!("Bearer {bearer}"),
        });
    }
    if let Some(target) = root["mcpServers"].as_object_mut() {
        target.insert("dojang-orchestrator".to_string(), server_entry);
    }

    if let Ok(serialized) = serde_json::to_string_pretty(&root) {
        let _ = std::fs::write(&mcp_path, serialized);
    }
    Ok(())
}

pub async fn handle_mcp(
    State(state): State<RunnerHttpState>,
    Extension(actions): Extension<SharedTaskActions>,
    body: Bytes,
) -> Response {
    let request: RpcRequest = match serde_json::from_slice(&body) {
        Ok(req) => req,
        Err(error) => {
            return (
                StatusCode::OK,
                Json(protocol::tool_error(
                    Value::Null,
                    PARSE_ERROR,
                    format!("invalid JSON: {error}"),
                )),
            )
                .into_response();
        }
    };

    if request.jsonrpc != "2.0" {
        return (
            StatusCode::OK,
            Json(protocol::tool_error(
                request.id.unwrap_or(Value::Null),
                INVALID_REQUEST,
                "jsonrpc must be '2.0'",
            )),
        )
            .into_response();
    }

    let Some(id) = request.id else {
        return StatusCode::ACCEPTED.into_response();
    };

    match request.method.as_str() {
        "initialize" => (
            StatusCode::OK,
            Json(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "dojang-orchestrator",
                        "version": env!("CARGO_PKG_VERSION"),
                    }
                }
            })),
        )
            .into_response(),
        "ping" => (
            StatusCode::OK,
            Json(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {}
            })),
        )
            .into_response(),
        "tools/list" => (
            StatusCode::OK,
            Json(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": tool_definitions()
                }
            })),
        )
            .into_response(),
        "tools/call" => {
            let call: CallParams = match serde_json::from_value(request.params) {
                Ok(params) => params,
                Err(error) => {
                    return (
                        StatusCode::OK,
                        Json(protocol::tool_error(
                            id,
                            INVALID_PARAMS,
                            format!("invalid tool call params: {error}"),
                        )),
                    )
                        .into_response();
                }
            };
            match execute_tool(&state, &actions, &call.name, call.arguments).await {
                Ok(value) => (
                    StatusCode::OK,
                    Json(protocol::tool_result(id, value.to_string())),
                )
                    .into_response(),
                Err(error) => (StatusCode::OK, Json(protocol::tool_failure(id, error))).into_response(),
            }
        }
        other => (
            StatusCode::OK,
            Json(protocol::tool_error(
                id,
                METHOD_NOT_FOUND,
                format!("unsupported method: {other}"),
            )),
        )
            .into_response(),
    }
}

async fn execute_tool(
    state: &RunnerHttpState,
    actions: &SharedTaskActions,
    name: &str,
    args: Value,
) -> Result<Value, String> {
    let now = crate::runner::now_secs();
    match name {
        "dojang_overview" => {
            let summary = assistant::overview(&state.pool).await?;
            let tasks = db::recent_tasks(&state.pool, 30)
                .await
                .map_err(|error| error.to_string())?;
            let compact: Vec<Value> = tasks
                .into_iter()
                .map(|t| {
                    json!({
                        "id": t.id,
                        "repo": t.repo,
                        "branch": t.branch,
                        "instruction": t.instruction,
                        "state": t.state,
                        "agent": t.agent,
                        "model": t.model,
                        "awaiting_kind": t.awaiting_kind,
                        "blocked_reason": t.blocked_reason,
                        "updated_at": t.updated_at,
                    })
                })
                .collect();
            Ok(json!({
                "overview": summary,
                "tasks": compact,
            }))
        }
        "dojang_task_inspect" => {
            let task_id = args
                .get("task_id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| "task_id(정수)가 필요합니다".to_string())?;
            let limit = args
                .get("output_limit")
                .and_then(|v| v.as_i64())
                .unwrap_or(20)
                .clamp(1, 100);
            let task = db::get_task(&state.pool, task_id)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("작업 #{task_id}을(를) 찾을 수 없습니다"))?;
            let output = db::list_task_output_for_task_after(&state.pool, task_id, 0, limit)
                .await
                .map_err(|error| error.to_string())?;
            Ok(json!({
                "task": task,
                "output": output,
            }))
        }
        "dojang_task_dispatch" => {
            let repository = args
                .get("repository")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "repository 경로가 필요합니다".to_string())?
                .to_string();
            let instruction = args
                .get("instruction")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "instruction이 필요합니다".to_string())?
                .to_string();
            let agent = args
                .get("agent")
                .and_then(|v| v.as_str())
                .unwrap_or("claude")
                .to_string();
            let role = args
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or(crate::agent::DEFAULT_ROLE)
                .to_string();
            let model = args
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let reasoning_effort = args
                .get("reasoning_effort")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mode = args
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("conversation")
                .to_string();

            let created = actions
                .create(
                    state,
                    QueuedTaskRequest {
                        repository,
                        instruction,
                        agent,
                        role,
                        model,
                        reasoning_effort,
                        mode,
                        goal_contract: None,
                        resume_session: None,
                        resume_vendor: None,
                    },
                )
                .await
                .map_err(|(_, msg)| msg)?;
            Ok(json!({ "task": created }))
        }
        "dojang_task_message" => {
            let task_id = args
                .get("task_id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| "task_id(정수)가 필요합니다".to_string())?;
            let message = args
                .get("message")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "비어 있지 않은 message가 필요합니다".to_string())?;
            actions
                .message(state, task_id, message)
                .await
                .map_err(|(_, msg)| msg)?;
            Ok(json!({ "ok": true, "task_id": task_id }))
        }
        "dojang_schedule_list" => {
            crate::schedule::migrate(&state.pool)
                .await
                .map_err(|error| error.to_string())?;
            let schedules = db::list_schedules(&state.pool)
                .await
                .map_err(|error| error.to_string())?;
            Ok(json!({ "schedules": schedules }))
        }
        "dojang_schedule_create" => {
            crate::schedule::migrate(&state.pool)
                .await
                .map_err(|error| error.to_string())?;
            if let Some(delay_minutes) = args.get("delay_minutes").and_then(|v| v.as_i64()) {
                if delay_minutes <= 0 {
                    return Err("delay_minutes는 0보다 커야 합니다".to_string());
                }
                let text = args
                    .get("text")
                    .or_else(|| args.get("instruction"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| "리마인더 내용(text)이 필요합니다".to_string())?;
                let label: String = text.chars().take(20).collect();
                let run_at = now + delay_minutes * 60;
                let payload = json!({ "text": text }).to_string();
                let id = db::insert_schedule_with_run_at(
                    &state.pool,
                    &label,
                    "",
                    "reminder",
                    &payload,
                    Some(run_at),
                    now,
                    32400,
                )
                .await
                .map_err(|error| error.to_string())?;
                return Ok(json!({ "id": id, "kind": "reminder", "run_at": run_at }));
            }

            let cron = args
                .get("cron")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "cron 식 또는 delay_minutes가 필요합니다".to_string())?;
            std::str::FromStr::from_str(cron)
                .map(|_: cron::Schedule| ())
                .map_err(|error: cron::error::Error| format!("유효하지 않은 cron 식: {error}"))?;
            let kind = args
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("task");
            if !matches!(kind, "task" | "reminder") {
                return Err("지원하는 스케줄 kind는 task 또는 reminder입니다".to_string());
            }
            let label = args
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or("비서 등록 스케줄");
            let payload = if kind == "reminder" {
                let text = args
                    .get("text")
                    .or_else(|| args.get("instruction"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "reminder 스케줄에는 text가 필요합니다".to_string())?;
                json!({ "text": text }).to_string()
            } else {
                let repo = args
                    .get("repo")
                    .or_else(|| args.get("repository"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "task 스케줄에는 repo 경로가 필요합니다".to_string())?;
                let instruction = args
                    .get("instruction")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "task 스케줄에는 instruction이 필요합니다".to_string())?;
                let agent = args
                    .get("agent")
                    .and_then(|v| v.as_str())
                    .unwrap_or("claude");
                json!({
                    "repo": repo,
                    "instruction": instruction,
                    "agent": agent,
                })
                .to_string()
            };
            let id = db::insert_schedule(&state.pool, label, cron, kind, &payload, now, 32400)
                .await
                .map_err(|error| error.to_string())?;
            Ok(json!({ "id": id, "kind": kind, "cron": cron }))
        }
        "dojang_request_approval" => {
            let action_kind = args
                .get("action_kind")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "action_kind가 필요합니다".to_string())?;
            let target_task_id = args.get("target_task_id").and_then(|v| v.as_i64());
            let summary = args
                .get("summary")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "summary가 필요합니다".to_string())?;
            let payload_json = args
                .get("payload")
                .map(|v| v.to_string())
                .unwrap_or_else(|| "{}".to_string());
            let assistant_task =
                assistant::ensure_assistant_task(&state.pool, &state.config, now).await?;
            let approval = assistant::create_approval(
                &state.pool,
                assistant_task.id,
                target_task_id,
                action_kind,
                summary,
                &payload_json,
                now,
            )
            .await?;
            Ok(json!({ "approval": approval }))
        }
        other => Err(format!("알 수 없는 도구입니다: {other}")),
    }
}

/// `praxis-runner` 데몬 틱에서 도래한 스케줄·리마인더를 평가해 실행하고 Web Push 이벤트를 발행한다.
pub async fn run_due_schedules(
    state: &RunnerHttpState,
    actions: &SharedTaskActions,
    now: i64,
) {
    if crate::schedule::migrate(&state.pool).await.is_err() {
        return;
    }
    let Ok(schedules) = db::list_enabled_schedules(&state.pool).await else {
        return;
    };
    for sched in schedules {
        let due = match sched.run_at {
            Some(run_at) => Ok(now >= run_at && sched.last_run_at.is_none()),
            None => {
                let base = sched.last_run_at.unwrap_or(sched.created_at);
                crate::schedule::is_due(&sched.cron, base, now, sched.tz_offset_secs)
            }
        };
        let Ok(true) = due else {
            continue;
        };
        if db::mark_schedule_ran(&state.pool, sched.id, now)
            .await
            .is_err()
        {
            continue;
        }
        if sched.run_at.is_some() {
            let _ = db::set_schedule_enabled(&state.pool, sched.id, false).await;
        }
        match sched.kind.as_str() {
            "reminder" => {
                let text = serde_json::from_str::<Value>(&sched.payload)
                    .ok()
                    .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_string))
                    .unwrap_or_else(|| sched.label.clone());
                if let Ok(assistant_task) =
                    assistant::ensure_assistant_task(&state.pool, &state.config, now).await
                {
                    let event_json = json!({
                        "kind": "text",
                        "text": format!("[리마인더] {text}"),
                    })
                    .to_string();
                    let _ = db::append_convo_event_with_runner_output(
                        &state.pool,
                        assistant_task.id,
                        &event_json,
                        now,
                    )
                    .await;
                    let _ = db::append_runner_event(
                        &state.pool,
                        assistant_task.id,
                        now,
                        db::runner_event_kind::REMINDER_FIRED,
                        Some(&text),
                    )
                    .await;
                }
            }
            "task" => {
                let Ok(payload) = serde_json::from_str::<Value>(&sched.payload) else {
                    continue;
                };
                let Some(repository) = payload.get("repo").and_then(|v| v.as_str()) else {
                    continue;
                };
                let Some(instruction) = payload.get("instruction").and_then(|v| v.as_str()) else {
                    continue;
                };
                let agent = payload
                    .get("agent")
                    .and_then(|v| v.as_str())
                    .unwrap_or("claude");
                let _ = actions
                    .create(
                        state,
                        QueuedTaskRequest {
                            repository: repository.to_string(),
                            instruction: instruction.to_string(),
                            agent: agent.to_string(),
                            role: crate::agent::DEFAULT_ROLE.to_string(),
                            model: String::new(),
                            reasoning_effort: String::new(),
                            mode: "conversation".to_string(),
                            goal_contract: None,
                            resume_session: None,
                            resume_vendor: None,
                        },
                    )
                    .await;
            }
            _ => {}
        }
    }
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "dojang_overview",
            "description": "현재 호스트의 코딩 작업 상태 요약(실행 중, 대기열, 검토 대기, 질문 대기, 승인 대기)과 최근 작업 목록을 조회합니다.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "dojang_task_inspect",
            "description": "특정 코딩 작업의 상세 메타데이터와 최근 출력 로그를 조회합니다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task_id": { "type": "integer", "description": "조회할 작업 ID" },
                    "output_limit": { "type": "integer", "description": "출력 줄 상한 (기본 20, 최대 100)" }
                },
                "required": ["task_id"],
                "additionalProperties": false
            }
        },
        {
            "name": "dojang_task_dispatch",
            "description": "허용된 저장소 루트 하위에 새 코딩 작업을 지정된 벤더(claude, codex, agy)로 생성·위임합니다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "repository": { "type": "string", "description": "대상 저장소 절대 경로" },
                    "instruction": { "type": "string", "description": "에이전트에게 전달할 작업 지시문" },
                    "agent": { "type": "string", "enum": ["claude", "codex", "agy"], "description": "실행할 벤더" },
                    "role": { "type": "string", "description": "역할 (기본 implementer)" },
                    "model": { "type": "string", "description": "모델 식별자 (선택)" },
                    "reasoning_effort": { "type": "string", "description": "추론 강도 (선택)" },
                    "mode": { "type": "string", "enum": ["conversation", "terminal"], "description": "실행 모드 (기본 conversation)" }
                },
                "required": ["repository", "instruction", "agent"],
                "additionalProperties": false
            }
        },
        {
            "name": "dojang_task_message",
            "description": "검토 대기 또는 질문 대기 중인 대화형 작업에 후속 답변·지시를 전달해 재개합니다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task_id": { "type": "integer", "description": "대상 작업 ID" },
                    "message": { "type": "string", "description": "전달할 후속 메시지" }
                },
                "required": ["task_id", "message"],
                "additionalProperties": false
            }
        },
        {
            "name": "dojang_schedule_list",
            "description": "등록된 정기 스케줄(cron)과 1회성 리마인더 목록을 조회합니다.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "dojang_schedule_create",
            "description": "정기 코딩/리마인더 스케줄(cron) 또는 상대 시각 1회성 리마인더(delay_minutes)를 등록합니다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "label": { "type": "string", "description": "스케줄 표시 이름" },
                    "cron": { "type": "string", "description": "6필드 또는 5필드 cron 표현식 (정기 스케줄용)" },
                    "delay_minutes": { "type": "integer", "description": "현재 시각 기준 분 단위 지연 (1회성 리마인더용)" },
                    "kind": { "type": "string", "enum": ["task", "reminder"], "description": "스케줄 종류" },
                    "text": { "type": "string", "description": "리마인더 알림 문구" },
                    "repo": { "type": "string", "description": "정기 작업 실행 저장소 경로" },
                    "instruction": { "type": "string", "description": "정기 작업 지시문" },
                    "agent": { "type": "string", "enum": ["claude", "codex", "agy"], "description": "정기 작업 벤더" }
                },
                "additionalProperties": false
            }
        },
        {
            "name": "dojang_request_approval",
            "description": "머지 승인(approve_task), 작업 폐기(discard_task), 외부기원 실행 허가(run_pending_task) 등 파괴적 동작에 대해 사용자 승인 카드를 생성하고 푸시 알림을 발송합니다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action_kind": {
                        "type": "string",
                        "enum": ["approve_task", "discard_task", "run_pending_task", "custom_command"],
                        "description": "승인 요청 동작 종류"
                    },
                    "target_task_id": { "type": "integer", "description": "대상 작업 ID" },
                    "summary": { "type": "string", "description": "사용자에게 보여줄 한 줄 승인 요약" },
                    "payload": { "type": "object", "description": "추가 메타데이터 JSON" }
                },
                "required": ["action_kind", "summary"],
                "additionalProperties": false
            }
        }
    ])
}
