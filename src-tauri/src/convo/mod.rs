//! 구조화 대화 (Phase 2) — 에이전트를 **스트리밍 구조화 출력**으로 돌려 raw 터미널 대신
//! 마크다운 답변 + 툴콜 카드로 렌더. 멀티벤더: claude/codex는 구조화 스트림, agy는 텍스트.
//!
//! - `parse_event`: claude stream-json 한 줄 → `ConvoEvent` (순수, cargo test).
//! - `parse_codex_event`: codex `exec --json` JSONL 한 줄 → `ConvoEvent` (실측 fixture 기반).
//! - `run_turn`: 벤더별 한 턴 실행 → 이벤트 콜백. 멀티턴 resume은 벤더별
//!   (claude `--resume <uuid>` / codex `exec resume <uuid>` / agy `--continue`).
//!
//! 보안: 격리 worktree cwd에서 실행. 자율 편집 위해 벤더별 권한 스킵 플래그(ensemble headless와 동일 전제).

mod agy_process;
mod claude_stream;
pub mod app_server;
pub mod cli_runtime;
pub mod interaction;
pub mod native_interaction;
mod tool_output;
pub mod interaction_commands;
pub mod question_local;
pub mod canvas;
pub mod debate;
mod child_reaper;
mod event_frame;
pub mod prompt_metrics;
mod context_observation;
mod model_observation;
mod process_cleanup;
pub mod question;
pub mod tool_cost;
mod turn_failure;
mod turn_guard;
#[cfg(test)]
mod turn_guard_tests;

pub use turn_failure::{FailureCategory, FailureClass, TurnFailure};

use serde::Serialize;

/// 대화 벤더 — `task.agent`로 선택.
/// gemini는 Antigravity(agy)로 라우팅: 개인용 gemini CLI 인증이 폐기됨(IneligibleTierError,
/// Google 안내가 Antigravity 이관). agy는 구조화 스트림이 없어 텍스트 1블록 + `--continue` resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Claude,
    Codex,
    Agy,
}

impl Vendor {
    pub fn from_agent(agent: &str) -> Self {
        match agent.trim() {
            "codex" => Vendor::Codex,
            "agy" | "antigravity" | "gemini" => Vendor::Agy,
            _ => Vendor::Claude,
        }
    }

    pub fn bin(&self) -> &'static str {
        match self {
            Vendor::Claude => "claude",
            Vendor::Codex => "codex",
            Vendor::Agy => "agy",
        }
    }
}

/// agy `--continue` resume 센티널 — 대화 id를 print 모드에서 얻을 수 없어 "직전 대화 이어가기"로 저장.
pub const AGY_CONTINUE: &str = "continue";

/// agy print 모드 자체 타임아웃(`--print-timeout`, 기본 5m0s) 상향값. 기본값은 유휴 워치독
/// (IDE 12h/러너 30min)보다 먼저 발동해, 5분 넘는 턴이 전부 "Error: timeout waiting for
/// response"(exit 1) + 무출력으로 죽는다. 워치독 최대치보다 큰 값을 명시해 kill 주체를
/// Praxis 유휴 워치독으로 일원화한다 (agy 쪽은 행 방지 최후 백스톱으로만 남김).
pub const AGY_PRINT_TIMEOUT: &str = "24h";

/// 작업 계획 한 줄 — TodoWrite todos[] 항목의 필요한 부분만.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanItem {
    pub content: String,
    /// 벤더 원문 그대로(`pending` | `in_progress` | `completed`). 정규화는 렌더가 한다 —
    /// 여기서 접으면 벤더가 새 상태를 늘렸을 때 관측이 먼저 거짓말을 한다.
    pub status: String,
}

/// 토론에서 발화한 자리. 좌측의 원천은 `tasks` 행이고 나머지는 `convo_debate_sides` 행이다.
/// 저장 식별자는 **위치가 아니라 자리의 이름**이다 — 3자에서 `right`는 가운데 열이지만, 이미
/// 저장된 `"speaker":"right"`를 다시 쓰지 않으려고 이름을 유지한다. 순서는 `index()`가 정한다.
/// `None`은 "좌측"이 아니라 **미상**이다 — 토론 이전의 단일 세션 이력이 여기 해당한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
    Third,
}

impl Side {
    pub fn as_str(&self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
            Side::Third => "third",
        }
    }

    /// 턴 순서상의 자리 번호. 좌측이 0이다.
    pub fn index(&self) -> usize {
        match self {
            Side::Left => 0,
            Side::Right => 1,
            Side::Third => 2,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "left" => Some(Side::Left),
            "right" => Some(Side::Right),
            "third" => Some(Side::Third),
            _ => None,
        }
    }
}

/// 토론이 끝난 사유. 상한 도달은 실패가 아니라 정상 종료의 한 종류다(설계 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DebateEndReason {
    Consensus,
    RoundCap,
    Aborted,
    Error,
}

/// 프론트로 보낼 단순화 대화 이벤트.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConvoEvent {
    /// 세션 시작 — session_id 확보(멀티턴 resume용).
    SessionInit { session_id: String },
    /// 어시스턴트 텍스트 블록. `parent_id`가 있으면 서브 에이전트(Task) 소속.
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_id: Option<String>,
    },
    Interaction { interaction_id: String },
    /// A replaceable message snapshot. Only complete snapshots enter history.
    TextUpdate { item_id: String, text: String, complete: bool },
    /// Rich output from Code Mode, native tools, or MCP. Preserved in history.
    ToolOutput { tool_use_id: String, contents: Vec<tool_output::Content> },
    /// 툴 사용 (파일 편집/명령 등) — 이름 + 요약.
    /// `tool_id`: 벤더가 주는 tool_use id(서브 에이전트 Task 상관관계용) — 이벤트 페이로드가
    /// task `id`와 flatten되므로 `id`라는 이름을 피한다. `parent_id`: 이 호출이
    /// 서브 에이전트(부모 tool_use) 소속임을 표시 — claude stream-json `parent_tool_use_id`.
    ToolUse {
        name: String,
        summary: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_id: Option<String>,
    },
    /// 툴 결과 (읽은 내용/명령 출력 등) — 앞부분 요약 + 오류 여부.
    /// `tool_use_id`: 대응하는 tool_use의 id — Task 결과 매칭으로 서브 에이전트 종료 판정.
    ToolResult {
        summary: String,
        is_error: bool,
        /// 절단 전 원문 **문자 수**(코드포인트). 컨텍스트 귀속 분석의 유일한 크기 신호.
        /// 바이트가 아니라 문자인 이유: 한국어 1자 ≈ 3바이트라 바이트로는 언어별로 왜곡된다(#189).
        /// `None`은 0이 아니라 **미상**이다 — 원문 개념이 없는 결과(codex file_change)와
        /// 이 필드 도입 전 기록된 이벤트가 여기 해당한다. 분석기가 둘을 구분해야 한다.
        #[serde(skip_serializing_if = "Option::is_none")]
        result_chars: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_use_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_id: Option<String>,
    },
    /// 서브 에이전트(Task) 실행 모델 관측 — parented assistant의 `message.model`.
    /// 메인 세션 `ModelSnapshot`과 kind를 분리한다: 같은 kind로 흘리면 세션 모델 칩과
    /// ensemble 병합이 서브 에이전트 모델로 덮인다.
    SubagentModel { parent_id: String, model: String },
    /// 실행 모델 관측 — 요청값과 공급자가 실제 사용한 resolved 값을 분리해 비교 재현성을 보존.
    ModelSnapshot {
        #[serde(skip_serializing_if = "Option::is_none")]
        requested: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        resolved: Option<String>,
        source: String,
    },
    /// 턴 종료 — 최종 텍스트 + 오류 여부 + 비용/턴수/토큰(벤더가 주는 만큼: claude=비용+토큰, codex=토큰).
    Result {
        text: String,
        is_error: bool,
        session_id: String,
        cost_usd: f64,
        num_turns: i64,
        tokens_in: i64,
        tokens_out: i64,
    },
    /// 에이전트가 선언한 작업 계획 스냅샷 — 작업 캔버스의 원천(계획 0033 DR-6).
    /// TodoWrite는 매번 **전체 목록**을 다시 보내므로 누적이 아니라 최신본이 정답이다.
    /// claude 전용 — codex/agy에는 대응 툴이 없어 캔버스가 비는 것이 정상이다(C-3의 비대칭).
    Plan {
        items: Vec<PlanItem>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_id: Option<String>,
    },
    /// 컨텍스트 윈도 사용 관측 — claude는 메인 스레드 assistant 메시지 usage의 입력측 합
    /// (input + cache_creation + cache_read = 그 API 호출이 실은 전체 컨텍스트).
    ContextUsage {
        context_tokens: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_window: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        observed_at: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        valid: Option<bool>,
    },
    /// 의도적 컨텍스트 절단 지점 — 벤더가 아니라 우리가 만드는 이벤트다.
    /// 원장에 남아야 재진입·재시작 후에도 경계가 보인다. 프론트가 구분선으로 렌더한다.
    ContextCleared { text: String },
    /// 토론 종료 경계 — 벤더가 아니라 우리가 만드는 이벤트다(`ContextCleared`와 같은 자리).
    /// 원장에 남아야 재진입 후에도 라운드 시퀀스의 끝이 보인다.
    DebateEnded { reason: DebateEndReason },
    /// 그 외(hook/rate_limit/system) — 무시하되 파이프라인 유지.
    Other,
}

/// DB 적재용 이벤트 봉투. `speaker`는 variant 안이 아니라 **루트의 형제 필드**다 —
/// `ConvoEvent`가 내부 태깅(`tag = "kind"`)이라 variant 바깥에만 자리가 있고, 저장·조회가
/// `json_extract(event, '$.kind')`로 루트 키를 직접 본다(설계 §4-4).
#[derive(Serialize)]
struct StoredConvoEvent<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    speaker: Option<Side>,
    #[serde(flatten)]
    event: &'a ConvoEvent,
}

/// 이벤트 JSON 한 줄 — 적재·전송의 유일한 직렬화 지점.
/// `speaker`가 `None`이면 키가 아예 빠져 토론 이전과 **바이트가 같다**.
pub fn stored_event_json(
    event: &ConvoEvent,
    speaker: Option<Side>,
) -> serde_json::Result<String> {
    if let ConvoEvent::TextUpdate { text, complete: true, .. } = event {
        return stored_event_json(&ConvoEvent::Text { text: text.clone(), parent_id: None }, speaker);
    }
    serde_json::to_string(&StoredConvoEvent { speaker, event })
}

/// tool_use 입력을 한 줄 요약 (file_path / command / pattern 등 흔한 키 우선).
fn tool_summary(input: &serde_json::Value) -> String {
    for k in [
        "file_path",
        "path",
        "command",
        "pattern",
        "url",
        "query",
        "description",
    ] {
        if let Some(s) = input.get(k).and_then(|v| v.as_str()) {
            return s.chars().take(120).collect();
        }
    }
    // 폴백: 축약 JSON.
    let s = input.to_string();
    s.chars().take(120).collect()
}

/// TodoWrite `input.todos[]` → 계획 항목. content가 빈 항목은 캔버스에 그릴 것이 없어 버린다.
fn plan_items(input: Option<&serde_json::Value>) -> Vec<PlanItem> {
    let Some(todos) = input
        .and_then(|i| i.get("todos"))
        .and_then(|t| t.as_array())
    else {
        return Vec::new();
    };
    todos
        .iter()
        .filter_map(|t| {
            let content = t.get("content").and_then(|x| x.as_str())?.trim();
            if content.is_empty() {
                return None;
            }
            Some(PlanItem {
                content: content.to_string(),
                status: t
                    .get("status")
                    .and_then(|x| x.as_str())
                    .unwrap_or("pending")
                    .to_string(),
            })
        })
        .collect()
}

/// stream-json 한 줄을 파싱해 유의 이벤트 **전부**를 반환. 무관한 줄은 빈 벡터.
/// 한 줄의 `message.content[]`에 병렬 tool_use/tool_result가 여러 개 실릴 수 있어
/// 첫 블록만 취하면 Task 스폰·결과가 유실된다(서브 에이전트 미표시/영구 running 오표시).
pub fn parse_events(line: &str) -> Vec<ConvoEvent> {
    let line = line.trim();
    if line.is_empty() {
        return Vec::new();
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return Vec::new();
    };
    let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
    // 서브 에이전트(Task) 소속 이벤트 표식 — 메인 스레드 이벤트는 null/부재.
    let parent_id = v
        .get("parent_tool_use_id")
        .and_then(|x| x.as_str())
        .map(str::to_string);
    match ty {
        "system" if v.get("subtype").and_then(|x| x.as_str()) == Some("init") => {
            let Some(session_id) = v.get("session_id").and_then(|x| x.as_str()) else {
                return Vec::new();
            };
            let mut events = vec![ConvoEvent::SessionInit {
                session_id: session_id.to_string(),
            }];
            if let Some(resolved) = v
                .get("model")
                .and_then(|x| x.as_str())
                .and_then(model_observation::normalize_model)
            {
                events.push(ConvoEvent::ModelSnapshot {
                    requested: None,
                    resolved: Some(resolved),
                    source: "claude_stream".into(),
                });
            }
            events
        }
        "assistant" => {
            // message.content[]의 모든 텍스트/툴 블록을 순서대로 방출.
            let content = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array());
            let mut out = Vec::new();
            // parented 턴만 방출한다 — 메인 스레드 모델은 system/init이 이미 관측하고 있고,
            // 여기서 겹쳐 내면 두 관측이 같은 세션을 두고 다투게 된다.
            // 블록이 비어도(툴만 있거나 빈 텍스트) 모델은 관측된 사실이므로 남긴다.
            if let Some(parent) = &parent_id {
                if let Some(model) = v
                    .get("message")
                    .and_then(|m| m.get("model"))
                    .and_then(|x| x.as_str())
                    .and_then(model_observation::normalize_model)
                {
                    out.push(ConvoEvent::SubagentModel {
                        parent_id: parent.clone(),
                        model,
                    });
                }
            }
            if let Some(blocks) = content {
                for b in blocks {
                    match b.get("type").and_then(|x| x.as_str()) {
                        Some("text") => {
                            let t = b.get("text").and_then(|x| x.as_str()).unwrap_or("");
                            if !t.trim().is_empty() {
                                out.push(ConvoEvent::Text {
                                    text: t.to_string(),
                                    parent_id: parent_id.clone(),
                                });
                            }
                        }
                        Some("tool_use") => {
                            let name = b.get("name").and_then(|x| x.as_str()).unwrap_or("tool");
                            let summary = b.get("input").map(tool_summary).unwrap_or_default();
                            out.push(ConvoEvent::ToolUse {
                                name: name.to_string(),
                                summary,
                                tool_id: b.get("id").and_then(|x| x.as_str()).map(str::to_string),
                                parent_id: parent_id.clone(),
                            });
                            // ToolUse를 먼저 밀어넣고 Plan을 덧붙인다 — 활동 표시는 그대로 두고
                            // 캔버스 원천만 추가한다. `tool_summary`는 `todos` 키를 모르므로
                            // 여기서 뽑지 않으면 축약 JSON 120자로 잘려 복원할 수 없다.
                            if name == "TodoWrite" {
                                let items = plan_items(b.get("input"));
                                if !items.is_empty() {
                                    out.push(ConvoEvent::Plan {
                                        items,
                                        parent_id: parent_id.clone(),
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            // 메인 스레드 usage만 방출 — 서브 에이전트(Task) 사이드체인은 별도 컨텍스트라 제외.
            // claude input_tokens는 캐시 제외 값이라 cache_creation/cache_read를 합산해야
            // 실제 컨텍스트 점유가 나온다 — tokens_in만 보면 대화 후반 대부분이 누락된다.
            if parent_id.is_none() {
                let ctx = v
                    .get("message")
                    .and_then(|m| m.get("usage"))
                    .map(|u| {
                        [
                            "input_tokens",
                            "cache_creation_input_tokens",
                            "cache_read_input_tokens",
                        ]
                        .iter()
                        .filter_map(|k| u.get(*k).and_then(|x| x.as_i64()))
                        .sum::<i64>()
                    })
                    .unwrap_or(0);
                if ctx > 0 {
                    out.push(ConvoEvent::ContextUsage {
                        context_tokens: ctx,
                        context_window: None,
                        observed_at: Some(crate::now()),
                        source: Some("claude_message".into()),
                        valid: Some(true),
                    });
                }
            }
            out
        }
        "user" => {
            // tool_result 블록 전부 — content는 문자열 또는 [{type:text, text}].
            let blocks = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array());
            let mut out = Vec::new();
            if let Some(blocks) = blocks {
                for b in blocks {
                    if b.get("type").and_then(|x| x.as_str()) == Some("tool_result") {
                        let is_error = b.get("is_error").and_then(|x| x.as_bool()).unwrap_or(false);
                        let text = match b.get("content") {
                            Some(serde_json::Value::String(s)) => s.clone(),
                            Some(serde_json::Value::Array(arr)) => arr
                                .iter()
                                .filter_map(|x| x.get("text").and_then(|t| t.as_str()))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            _ => String::new(),
                        };
                        // 절단 전 원문 크기 — 절단 후에는 복원할 수 없다.
                        let result_chars = text.chars().count() as i64;
                        // 앞 6줄 + 400자 요약.
                        let summary: String = text
                            .lines()
                            .take(6)
                            .collect::<Vec<_>>()
                            .join("\n")
                            .chars()
                            .take(400)
                            .collect();
                        out.push(ConvoEvent::ToolResult {
                            summary,
                            is_error,
                            result_chars: Some(result_chars),
                            tool_use_id: b
                                .get("tool_use_id")
                                .and_then(|x| x.as_str())
                                .map(str::to_string),
                            parent_id: parent_id.clone(),
                        });
                    }
                }
            }
            out
        }
        "result" => vec![ConvoEvent::Result {
            text: v
                .get("result")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            is_error: v.get("is_error").and_then(|x| x.as_bool()).unwrap_or(false),
            session_id: v
                .get("session_id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            cost_usd: v
                .get("total_cost_usd")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0),
            num_turns: v.get("num_turns").and_then(|x| x.as_i64()).unwrap_or(0),
            tokens_in: v
                .get("usage")
                .and_then(|u| u.get("input_tokens"))
                .and_then(|x| x.as_i64())
                .unwrap_or(0),
            tokens_out: v
                .get("usage")
                .and_then(|u| u.get("output_tokens"))
                .and_then(|x| x.as_i64())
                .unwrap_or(0),
        }],
        _ => Vec::new(),
    }
}

/// 첫 유의 이벤트만 필요할 때의 편의 래퍼 (단일 블록 fixture 테스트용) — 없으면 `Other`.
#[cfg(test)]
fn parse_event(line: &str) -> ConvoEvent {
    parse_events(line)
        .into_iter()
        .next()
        .unwrap_or(ConvoEvent::Other)
}

/// codex file_change의 changes:[{path,kind}]를 "kind path, ..." 한 줄로 요약.
fn codex_changes_summary(item: &serde_json::Value) -> String {
    item.get("changes")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .map(|ch| {
                    let kind = ch.get("kind").and_then(|x| x.as_str()).unwrap_or("edit");
                    let path = ch.get("path").and_then(|x| x.as_str()).unwrap_or("?");
                    format!("{kind} {path}")
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
        .chars()
        .take(160)
        .collect()
}

/// codex 한 줄 → 이벤트 목록. turn.completed usage는 누계이므로 ContextUsage로 변환하지 않는다.
pub fn parse_codex_events(line: &str) -> Vec<ConvoEvent> {
    vec![parse_codex_event(line)]
}

/// codex `exec --json` JSONL 한 줄 파싱 (스키마는 codex-cli 0.141 실측 fixture).
/// thread.started→세션, item.*(command_execution/agent_message)→툴/텍스트, turn.completed→결과.
pub fn parse_codex_event(line: &str) -> ConvoEvent {
    let line = line.trim();
    if line.is_empty() {
        return ConvoEvent::Other;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return ConvoEvent::Other;
    };
    let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
    match ty {
        "thread.started" => match v.get("thread_id").and_then(|x| x.as_str()) {
            Some(tid) => ConvoEvent::SessionInit {
                session_id: tid.to_string(),
            },
            None => ConvoEvent::Other,
        },
        "item.started" | "item.completed" => {
            let Some(item) = v.get("item") else {
                return ConvoEvent::Other;
            };
            let itype = item.get("type").and_then(|x| x.as_str()).unwrap_or("");
            match (ty, itype) {
                ("item.started", "command_execution") => ConvoEvent::ToolUse {
                    name: "shell".into(),
                    summary: item
                        .get("command")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .chars()
                        .take(120)
                        .collect(),
                    tool_id: None,
                    parent_id: None,
                },
                ("item.completed", "command_execution") => {
                    let out = item
                        .get("aggregated_output")
                        .and_then(|x| x.as_str())
                        .unwrap_or("");
                    let summary: String = out
                        .lines()
                        .take(6)
                        .collect::<Vec<_>>()
                        .join("\n")
                        .chars()
                        .take(400)
                        .collect();
                    let failed = item.get("status").and_then(|x| x.as_str()) == Some("failed")
                        || item
                            .get("exit_code")
                            .and_then(|x| x.as_i64())
                            .map(|c| c != 0)
                            .unwrap_or(false);
                    ConvoEvent::ToolResult {
                        summary,
                        is_error: failed,
                        result_chars: Some(out.chars().count() as i64),
                        tool_use_id: None,
                        parent_id: None,
                    }
                }
                ("item.completed", "agent_message") => {
                    let t = item.get("text").and_then(|x| x.as_str()).unwrap_or("");
                    if t.trim().is_empty() {
                        ConvoEvent::Other
                    } else {
                        ConvoEvent::Text {
                            text: t.to_string(),
                            parent_id: None,
                        }
                    }
                }
                // apply_patch 편집 — changes:[{path,kind}] (실측 fixture).
                ("item.started", "file_change") => ConvoEvent::ToolUse {
                    name: "edit".into(),
                    summary: codex_changes_summary(item),
                    tool_id: None,
                    parent_id: None,
                },
                ("item.completed", "file_change") => ConvoEvent::ToolResult {
                    summary: codex_changes_summary(item),
                    is_error: item.get("status").and_then(|x| x.as_str()) == Some("failed"),
                    // 변경 파일 목록에는 "원문"에 해당하는 것이 없다 — 0이 아니라 미상이다.
                    result_chars: None,
                    tool_use_id: None,
                    parent_id: None,
                },
                _ => ConvoEvent::Other, // reasoning, item.started(agent_message) 등
            }
        }
        "turn.completed" => ConvoEvent::Result {
            text: String::new(),
            is_error: false,
            session_id: String::new(),
            cost_usd: 0.0, // codex는 달러 대신 usage 토큰 제공 — 프론트가 토큰 푸터로 렌더.
            num_turns: 0,
            tokens_in: v
                .get("usage")
                .and_then(|u| u.get("input_tokens"))
                .and_then(|x| x.as_i64())
                .unwrap_or(0),
            tokens_out: v
                .get("usage")
                .and_then(|u| u.get("output_tokens"))
                .and_then(|x| x.as_i64())
                .unwrap_or(0),
        },
        "turn.failed" => ConvoEvent::Result {
            text: v
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|x| x.as_str())
                .unwrap_or("codex 턴 실패")
                .to_string(),
            is_error: true,
            session_id: String::new(),
            cost_usd: 0.0,
            num_turns: 0,
            tokens_in: 0,
            tokens_out: 0,
        },
        _ => ConvoEvent::Other, // turn.started 등
    }
}

/// 벤더별 한 턴 실행. 멀티턴이면 `resume`(claude/codex: id, agy: continue 센티널) 전달.
/// 벤더별 실행 인자 구성 — run_turn에서 분리(벤더 분기 격리 + run_turn 축소).
/// resume 규약: claude `--resume <id>`, codex `exec resume <id>`(서브커맨드), agy `--continue`(센티널).
/// 자율 편집 권한 플래그는 ensemble headless와 동일 전제(격리 worktree).
/// `model`: 설정된 벤더 기본 모델(`None`/빈 문자열이면 미주입) — claude `--model`, codex는 `exec` 뒤 `-m`, agy `--model`.
#[cfg(test)]
fn vendor_command(
    bin: &str,
    vendor: Vendor,
    message: &str,
    resume: Option<&str>,
    model: Option<&str>,
) -> std::process::Command {
    vendor_command_with_effort(bin, vendor, message, resume, model, None, &[], None, None)
}

/// `session_name`: claude 세션 레지스트리에 등록될 표시 이름(`agent::headless_args_with_effort`와
/// 같은 근거 — cwd 파생 이름은 같은 worktree의 task끼리 충돌한다).
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn vendor_command_with_effort(
    bin: &str,
    vendor: Vendor,
    message: &str,
    resume: Option<&str>,
    model: Option<&str>,
    reasoning_effort: Option<&str>,
    image_paths: &[String],
    session_name: Option<&str>,
    mcp: Option<&crate::preview_bridge::mcp::McpInjection>,
) -> std::process::Command {
    vendor_command_with_contract(bin, vendor, message, resume, model, reasoning_effort,
        image_paths, session_name, mcp, false)
}

#[allow(clippy::too_many_arguments)]
fn vendor_command_with_contract(
    bin: &str,
    vendor: Vendor,
    message: &str,
    resume: Option<&str>,
    model: Option<&str>,
    reasoning_effort: Option<&str>,
    image_paths: &[String],
    session_name: Option<&str>,
    mcp: Option<&crate::preview_bridge::mcp::McpInjection>,
    questions: bool,
) -> std::process::Command {
    let mut c = std::process::Command::new(bin);
    let model = model.map(str::trim).filter(|m| !m.is_empty());
    let guarded_message = turn_guard::guarded_message(vendor, message);
    match vendor {
        Vendor::Claude => {
            c.args([
                "-p",
                guarded_message.as_ref(),
                "--output-format",
                "stream-json",
                "--verbose",
            ]);
            // 승인이 켜진 턴은 CLI 기본 권한 모드로 돌고, 물어야 할 호출은 주입된
            // `--permission-prompt-tool`(우리 MCP 툴)로 간다.
            if !mcp.is_some_and(|m| m.approvals) {
                c.arg("--dangerously-skip-permissions");
            }
            // 질문 세션은 CLI처럼 글자가 흐르게 한다. 다른 턴에 켜면 델타마다 이벤트가 적재 경로를
            // 타므로(runner) 여기서만 켠다 — 스냅샷 변환은 `claude_stream`이 맡는다.
            if questions {
                c.arg("--include-partial-messages");
            }
            let instructions = prompt_metrics::runtime_instructions(vendor, questions);
            c.args(["--append-system-prompt", &instructions]);
            // `-n`은 claude 전용 — codex/agy에는 대응 플래그가 없어 붙이면 즉사한다.
            if let Some(name) = session_name.map(str::trim).filter(|n| !n.is_empty()) {
                c.args(["-n", name]);
            }
            if let Some(sid) = resume {
                c.args(["--resume", sid]);
            }
            if let Some(m) = model {
                c.args(["--model", m]);
            }
            if let Some([flag, value]) = crate::agent::claude_effort_args(reasoning_effort) {
                c.args([flag, value]);
            }
            if let Some(mcp) = mcp {
                c.args(&mcp.args);
            }
        }
        Vendor::Codex => {
            match resume {
                Some(sid) => {
                    c.args(["exec", "resume", sid]);
                }
                None => {
                    c.arg("exec");
                }
            }
            if let Some(m) = model {
                c.args(["-m", m]);
            }
            if let Some(config) = crate::agent::reasoning_effort_config_arg(reasoning_effort) {
                c.args(["-c", &config]);
            }
            for image_path in image_paths {
                c.args(["--image", image_path]);
            }
            if let Some(mcp) = mcp {
                c.args(&mcp.args);
            }
            c.args([
                "--json",
                "--skip-git-repo-check",
                "--dangerously-bypass-approvals-and-sandbox",
                guarded_message.as_ref(),
            ]);
        }
        Vendor::Agy => {
            c.args([
                "-p",
                guarded_message.as_ref(),
                "--dangerously-skip-permissions",
                "--print-timeout",
                AGY_PRINT_TIMEOUT,
            ]);
            if resume.is_some() {
                c.arg("--continue"); // print 모드에선 대화 id를 못 얻어 "직전 이어가기"로 resume.
            }
            if let Some(m) = model {
                // agy CLI는 `-m` 단축 플래그가 없다(`--model`만 지원) — `-m`이면
                // "flags provided but not defined" 플래그 오류로 무출력 즉사.
                c.args(["--model", m]);
            }
            if let Some([flag, value]) = crate::agent::agy_effort_args(reasoning_effort) {
                c.args([flag, value]);
            }
        }
    }
    if let Some(mcp) = mcp {
        c.envs(mcp.env.iter().cloned());
    }
    c
}

/// `run_turn`의 종료 관측 결과 — result 이벤트 미수신 시 호출측이 사인(死因)을 구분해
/// 표시할 수 있게 한다(인터럽트/유휴 타임아웃/자체 비정상 종료가 전부 같은 메시지로 뭉개지는 것 방지).
#[derive(Debug)]
pub struct TurnOutcome {
    /// resume 토큰(세션 id).
    pub session_id: String,
    /// 유휴 워치독이 프로세스 그룹을 죽였다(무출력 idle_timeout_secs 초과).
    pub timed_out: bool,
    /// 종료 상태 요약 — "exit N" / "signal N" / "unknown".
    pub exit_desc: String,
    /// stderr 마지막 줄들(상한 적용) — 비정상 종료 원인 표면화용(ENOSPC, resume 실패 등).
    pub stderr_tail: String,
    /// 무진전 브레이커가 턴을 끊었다면 그 판정 요약. halt는 스스로 프로세스 그룹을 죽이므로
    /// `exit_desc`만 보면 프로세스 즉사와 구분되지 않는다 — 분류는 이 필드를 먼저 본다.
    pub halted: Option<String>,
}

impl TurnOutcome {
    /// `Result` 이벤트 없이 끝난 턴의 사인(死因) 문구.
    ///
    /// IDE(commands)·러너(runner) 두 호출 경로가 같은 판정을 쓰도록 여기에 둔다 — 러너가 이
    /// 판정을 통째로 누락해, 워치독에 죽거나 비정상 종료한 턴이 "completed"로 마감되던 회귀가
    /// 있었다. 사용자 인터럽트는 호출측만 알 수 있으므로 여기서 다루지 않는다.
    pub fn failure_text(&self, idle_timeout_secs: u64) -> String {
        if self.timed_out {
            return format!(
                "턴이 유휴 타임아웃으로 중단되었습니다 ({} 무출력)",
                humanize_secs(idle_timeout_secs)
            );
        }
        let mut text = format!(
            "에이전트 프로세스가 결과 없이 종료되었습니다 ({})",
            self.exit_desc
        );
        let tail = self.stderr_tail.trim();
        if tail.is_empty() {
            text.push_str(" — 디스크 여유 공간/인증 상태를 확인하세요");
        } else {
            text.push_str("\n\nstderr:\n");
            text.push_str(tail);
        }
        text
    }
}

/// 유휴 상한을 사람이 읽는 단위로. 시간 단위로 나누어떨어지면 "N시간", 아니면 "N분".
fn humanize_secs(secs: u64) -> String {
    if secs >= 3600 && secs % 3600 == 0 {
        format!("{}시간", secs / 3600)
    } else {
        format!("{}분", secs.div_ceil(60))
    }
}

/// stderr 리더가 보관할 마지막 줄 수 / 줄당 문자 상한 — 트랜스크립트 오염 방지용 캡.
const STDERR_TAIL_LINES: usize = 12;
const STDERR_LINE_CHARS: usize = 400;

/// 스폰 직후 `on_spawn(pgid)` — 호출측이 인터럽트(kill_group)용으로 보관.
/// `bin`은 해석된 실행 파일 경로(호출측이 `reviewer::which`로 해석) — core 모듈이 PATH 탐색에
/// 의존하지 않아 스텁 바이너리로 결정적 테스트 가능. 각 이벤트를 `on_event` 콜백. 반환: 종료 관측/에러.
/// `model`: 턴 시작 시점의 설정된 벤더 기본 모델(호출측이 매 턴 최신값을 조회해 전달).
/// `idle_timeout_secs`: 유휴(무출력) 타임아웃 — stdout 라인이 오면 데드라인이 리셋된다. 총 실행
/// 시간은 무제한이라 활동 중인 턴은 긴 툴 실행/생성 구간도 견딘다(고아/행 프로세스 정리용 백스톱).
#[allow(clippy::too_many_arguments)]
pub fn run_turn(
    cwd: &str,
    message: &str,
    resume: Option<&str>,
    idle_timeout_secs: u64,
    vendor: Vendor,
    bin: &str,
    model: Option<&str>,
    on_spawn: impl FnOnce(u32),
    on_event: impl FnMut(ConvoEvent),
) -> Result<TurnOutcome, String> {
    run_turn_with_effort(
        cwd,
        message,
        resume,
        idle_timeout_secs,
        vendor,
        bin,
        model,
        None,
        None,
        &[],
        None,
        None,
        on_spawn,
        on_event,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_turn_with_effort(
    cwd: &str,
    message: &str,
    resume: Option<&str>,
    idle_timeout_secs: u64,
    vendor: Vendor,
    bin: &str,
    model: Option<&str>,
    reasoning_effort: Option<&str>,
    service_tier: Option<&str>,
    image_paths: &[String],
    session_name: Option<&str>,
    mcp: Option<&crate::preview_bridge::mcp::McpInjection>,
    on_spawn: impl FnOnce(u32),
    on_event: impl FnMut(ConvoEvent),
) -> Result<TurnOutcome, String> {
    run_turn_with_contract(cwd, message, resume, idle_timeout_secs, vendor, bin, model,
        reasoning_effort, service_tier, image_paths, session_name, mcp, false, on_spawn, on_event)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_turn_with_contract(
    cwd: &str,
    message: &str,
    resume: Option<&str>,
    idle_timeout_secs: u64,
    vendor: Vendor,
    bin: &str,
    model: Option<&str>,
    reasoning_effort: Option<&str>,
    service_tier: Option<&str>,
    image_paths: &[String],
    session_name: Option<&str>,
    mcp: Option<&crate::preview_bridge::mcp::McpInjection>,
    questions: bool,
    on_spawn: impl FnOnce(u32),
    mut on_event: impl FnMut(ConvoEvent),
) -> Result<TurnOutcome, String> {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    let mut c = vendor_command_with_contract(
        bin,
        vendor,
        message,
        resume,
        model,
        reasoning_effort,
        image_paths,
        session_name,
        mcp,
        questions,
    );
    if vendor == Vendor::Codex {
        crate::agent::service_tier::apply(&mut c, service_tier)?;
    }
    let process_scope = process_cleanup::TurnProcessScope::attach(&mut c);
    c.current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    // 새 프로세스 그룹 — 인터럽트/타임아웃이 자식 트리 전체를 정리(kill_group)하도록.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0); // pgid == pid.
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0000_0200); // CREATE_NEW_PROCESS_GROUP — kill_group(taskkill /T)용.
    }
    // 아래 경로들이 `?`로 이탈해도 자식은 회수된다 — 가드가 없으면 좀비로 남는다.
    let observation_started_at = chrono::Utc::now().timestamp_millis();
    let mut child = match c.spawn() {
        Ok(child) => child_reaper::ReapOnDrop::new(child),
        Err(e) => {
            if vendor == Vendor::Codex {
                on_event(context_observation::invalid_event("codex_session"));
            }
            return Err(format!("{} 실행 실패: {e}", vendor.bin()));
        }
    };
    // `process_group(0)`을 걸었으므로 pgid == pid. 생존자 집계에서 vendor 자신을 빼는 데 쓴다 —
    // Result 시점에 vendor는 당연히 살아 있어서, 빼지 않으면 모든 턴이 실패로 찍힌다.
    let vendor_pgid = child.id();
    let pid = child.id();
    on_spawn(pid); // process_group(0) ⇒ pgid == pid — 인터럽트는 kill_group(pid).
    on_event(ConvoEvent::ModelSnapshot {
        requested: model.and_then(model_observation::normalize_model),
        resolved: None,
        source: "invocation".into(),
    });
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            if vendor == Vendor::Codex {
                on_event(context_observation::invalid_event("codex_session"));
            }
            return Err("stdout 핸들 없음".into());
        }
    };
    let stderr = child.stderr.take();
    if vendor == Vendor::Agy {
        let output = agy_process::capture(&mut child, stdout, stderr, pid, idle_timeout_secs)?;
        let text = output.stdout.trim();
        if text.is_empty() {
            return Err(format!(
                "agy 출력이 없습니다 (인증/모델 설정 확인, {}){}",
                output.exit_desc,
                fmt_stderr(&output.stderr_tail)
            ));
        }
        on_event(ConvoEvent::Text {
            text: text.to_string(),
            parent_id: None,
        });
        on_event(ConvoEvent::Result {
            text: String::new(),
            is_error: false,
            session_id: AGY_CONTINUE.into(),
            cost_usd: 0.0,
            num_turns: 0,
            tokens_in: 0,
            tokens_out: 0,
        });
        return Ok(TurnOutcome {
            session_id: AGY_CONTINUE.into(),
            timed_out: output.timed_out,
            exit_desc: output.exit_desc,
            stderr_tail: output.stderr_tail,
            halted: None,
        });
    }

    // stderr 테일 수집 — 프로세스가 result 없이 죽었을 때 진짜 사인(ENOSPC, resume 실패,
    // 인증 오류 등)을 표면화한다. 마지막 STDERR_TAIL_LINES줄만 보관(폭주 방지).
    let stderr_handle = stderr.map(|stderr| {
        std::thread::spawn(move || {
            let mut tail = std::collections::VecDeque::with_capacity(STDERR_TAIL_LINES);
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                let line: String = line.chars().take(STDERR_LINE_CHARS).collect();
                if tail.len() == STDERR_TAIL_LINES {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
            tail.into_iter().collect::<Vec<_>>().join("\n")
        })
    });

    // 유휴 워치독: stdout 라인 하트비트가 idle_timeout_secs 동안 없으면 프로세스 그룹 kill.
    // 총 실행 시간은 제한하지 않는다 — 활동 중인 턴은 긴 툴 실행/생성 구간을 견딘다.
    enum Beat {
        Line,
        Done,
    }
    let timed_out = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let timed_out_flag = timed_out.clone();
    let (beat_tx, beat_rx) = std::sync::mpsc::channel::<Beat>();
    std::thread::spawn(move || loop {
        match beat_rx.recv_timeout(std::time::Duration::from_secs(idle_timeout_secs.max(1))) {
            Ok(Beat::Line) => {}     // 활동 → 데드라인 리셋
            Ok(Beat::Done) => break, // 정상 종료 → 워치독 해제 (kill 금지 — pid 재사용 방지)
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // 유휴 초과 — 프로세스 그룹 정리. 플래그로 호출측이 "타임아웃"임을 알 수 있게.
                timed_out_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                crate::verify::kill_group(pid);
                break;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                // 송신측 드롭(리더 패닉) — 자식 프로세스 정리(백스톱, 기존 시맨틱 보존).
                crate::verify::kill_group(pid);
                break;
            }
        }
    });

    // 종료 관측 공통화 — wait로 exit 요약을 얻고 stderr 테일 스레드를 회수한다.
    fn finalize(
        child: &mut std::process::Child,
        stderr_handle: Option<std::thread::JoinHandle<String>>,
    ) -> (String, String) {
        let exit_desc = match child.wait() {
            Ok(s) => match s.code() {
                Some(code) => format!("exit {code}"),
                None => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::process::ExitStatusExt;
                        match s.signal() {
                            Some(sig) => format!("signal {sig}"),
                            None => "unknown".to_string(),
                        }
                    }
                    #[cfg(not(unix))]
                    {
                        "unknown".to_string()
                    }
                }
            },
            Err(_) => "unknown".to_string(),
        };
        let stderr_tail = stderr_handle
            .and_then(|h| h.join().ok())
            .unwrap_or_default();
        (exit_desc, stderr_tail)
    }
    // 에러 문자열에 stderr 테일을 덧붙일 때의 표기(비어 있으면 생략).
    fn fmt_stderr(tail: &str) -> String {
        let t = tail.trim();
        if t.is_empty() {
            String::new()
        } else {
            format!("\nstderr: {t}")
        }
    }

    let mut last_session = resume.unwrap_or("").to_string();
    // 스트림에서 실제 모델을 확보했는지 — 못 했을 때만 트랜스크립트 폴백을 탄다.
    let mut stream_model_seen = false;
    let mut subagent_guard = turn_guard::SubagentTurnGuard::default();
    // 마지막 Result를 붙들어 둔다. claude는 백그라운드 작업이 살아 있으면 `result` 뒤에도
    // 프로세스를 끝내지 않고, 알림이 오면 모델을 다시 불러 `result`를 또 낸다 — 턴의 답은
    // 그중 마지막이고, 회수 판정은 프로세스가 끝난 뒤에야 의미가 있다. 본문은 이미 text
    // 이벤트로 흘렀으므로 앞선 Result를 버려도 대화에서 사라지는 것은 없다.
    let mut held_result: Option<ConvoEvent> = None;
    // 마지막 `result` 줄 시점의 생존자 스냅샷. EOF에서만 보면 놓친다 — 탈출한 자식이 vendor의
    // stdout을 물려받고 있으면 그 자식이 죽어야 EOF가 오고, 그때는 이미 잡을 것이 없다.
    let mut survivors_at_result: Vec<process_cleanup::Survivor> = Vec::new();
    let mut stdout = BufReader::new(stdout);
    let mut stream_error = None;
    let mut text_stream = claude_stream::TextStream::default();
    loop {
        let line = match event_frame::read_line(&mut stdout) {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                stream_error = Some(format!("{} 출력 읽기 실패: {error}", vendor.bin()));
                crate::verify::kill_group(pid);
                break;
            }
        };
        let _ = beat_tx.send(Beat::Line);
        if let Some(event) = subagent_guard.observe_raw_line(&line) {
            on_event(event);
        }
        // claude 한 줄에는 병렬 블록이 여러 개 실릴 수 있어 전부 방출. codex는 줄당 1이벤트.
        let evs = match vendor {
            Vendor::Codex => parse_codex_events(&line),
            Vendor::Claude => match text_stream.observe_line(&line) {
                Some(updates) => updates,
                None => parse_events(&line)
                    .into_iter()
                    .filter_map(|ev| text_stream.settle(ev))
                    .collect(),
            },
            _ => parse_events(&line),
        };
        for mut ev in evs {
            subagent_guard.observe_event(&ev);
            ev = subagent_guard.sanitize_event(ev);
            match &ev {
                ConvoEvent::SessionInit { session_id } => last_session = session_id.clone(),
                ConvoEvent::Result { session_id, .. } if !session_id.is_empty() => {
                    last_session = session_id.clone()
                }
                ConvoEvent::ModelSnapshot {
                    resolved: Some(_), ..
                } => stream_model_seen = true,
                _ => {}
            }
            if let ConvoEvent::Result { is_error, .. } = &ev {
                // 생존자 조회는 전체 프로세스를 훑는다(macOS는 pid마다 sysctl). 성공 Result
                // 시점과 EOF, 두 번만 치른다. 실패 Result는 판정 대상이 아니라 조회도 없다.
                if !is_error {
                    survivors_at_result = process_scope.survivors(vendor_pgid, vendor);
                }
                held_result = Some(ev);
                continue;
            }
            on_event(ev);
        }
    }
    let _ = beat_tx.send(Beat::Done);
    let (exit_desc, stderr_tail) = finalize(&mut child, stderr_handle);
    if let Some(error) = stream_error {
        return Err(error);
    }
    if let Some(result) = held_result.take() {
        // 프로세스가 끝난 뒤의 판정 — 이 시점에 남은 pending은 진짜 미회수다. 생존자는
        // 마지막 Result 시점 스냅샷과 EOF 시점 조회를 합친다. 실패 Result는 그대로 지나간다.
        let mut survivors = survivors_at_result;
        if matches!(result, ConvoEvent::Result { is_error: false, .. }) {
            for survivor in process_scope.survivors(vendor_pgid, vendor) {
                if !survivors.iter().any(|seen| seen.group == survivor.group) {
                    survivors.push(survivor);
                }
            }
        }
        on_event(subagent_guard.enforce_result_with_survivors(result, &survivors));
    }
    if vendor == Vendor::Codex {
        // codex는 스트림에 모델을 싣지 않아 트랜스크립트가 유일한 관측원이다.
        if let Some(resolved) = model_observation::codex_model(&last_session) {
            on_event(ConvoEvent::ModelSnapshot {
                requested: None,
                resolved: Some(resolved),
                source: "codex_session".into(),
            });
        }
        let observation = context_observation::codex_context(&last_session, observation_started_at)
            .unwrap_or_else(|| context_observation::invalid_event("codex_session"));
        on_event(observation);
    } else if vendor == Vendor::Claude && !stream_model_seen {
        // claude는 평소 `system/init`이 모델을 준다. 그 경로가 어긋난 턴만 여기로 온다 —
        // 관측이 통째로 사라지느니 덜 구체적인 값이라도 남긴다.
        if let Some(resolved) = model_observation::claude_model(&last_session) {
            on_event(ConvoEvent::ModelSnapshot {
                requested: None,
                resolved: Some(resolved),
                source: "claude_session".into(),
            });
        }
    }
    if last_session.is_empty() {
        // resume 토큰조차 없다 — 첫 턴이 스트림도 못 열고 죽은 경우. 사인을 그대로 노출.
        Err(format!(
            "세션 id를 얻지 못함 ({} 출력 이상, {exit_desc}){}",
            vendor.bin(),
            fmt_stderr(&stderr_tail)
        ))
    } else {
        Ok(TurnOutcome {
            session_id: last_session,
            timed_out: timed_out.load(std::sync::atomic::Ordering::Relaxed),
            exit_desc,
            stderr_tail,
            halted: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// stdout 스트림 없이 stderr만 남기고 즉사하는 스텁 — 프로세스 자체 사망(ENOSPC 등) 재현.
    #[cfg(unix)]
    fn write_stub(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).unwrap();
        let script = dir.join("stub.sh");
        std::fs::write(&script, body).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[test]
    #[cfg(unix)]
    fn oversized_claude_event_fails_even_with_resume_and_reaps_provider() {
        let dir = crate::testtmp::dir().join(format!("praxis-turn-large-{}", std::process::id()));
        let script = write_stub(&dir, "#!/usr/bin/python3\nimport sys,time\nsys.stdout.write('a' * (17 * 1024 * 1024))\nsys.stdout.flush()\ntime.sleep(30)\n");
        let mut pid = 0;
        let result = run_turn(
            dir.to_str().unwrap(), "hi", Some("resume-token"), 10,
            Vendor::Claude, script.to_str().unwrap(), None,
            |id| pid = id, |_| {},
        );
        assert!(result.unwrap_err().contains("이벤트 크기 제한 초과 (16 MiB)"));
        assert_ne!(unsafe { nix::libc::kill(-(pid as i32), 0) }, 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn run_turn_surfaces_exit_and_stderr_on_silent_death() {
        let dir = crate::testtmp::dir().join(format!("praxis-turn-stub-{}", std::process::id()));
        let script = write_stub(&dir, "#!/bin/sh\necho 'boom: no space left' >&2\nexit 3\n");
        // resume 토큰이 있으면 Ok(outcome) — 사인(exit/stderr)이 outcome에 실린다.
        let out = run_turn(
            dir.to_str().unwrap(),
            "hi",
            Some("resume-token"),
            60,
            Vendor::Claude,
            script.to_str().unwrap(),
            None,
            |_| {},
            |_| {},
        )
        .expect("resume 토큰 보유 — Ok(outcome)이어야 함");
        assert_eq!(out.session_id, "resume-token");
        assert!(!out.timed_out);
        assert_eq!(out.exit_desc, "exit 3");
        assert!(out.stderr_tail.contains("no space left"));
        // 첫 턴(resume 없음)이면 Err — 에러 문자열에 사인이 포함된다.
        let err = run_turn(
            dir.to_str().unwrap(),
            "hi",
            None,
            60,
            Vendor::Claude,
            script.to_str().unwrap(),
            None,
            |_| {},
            |_| {},
        )
        .unwrap_err();
        assert!(err.contains("exit 3"), "err={err}");
        assert!(err.contains("no space left"), "err={err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 사인 문구는 IDE·러너가 공유한다 — 유휴 타임아웃과 비정상 종료를 구분하고, 후자는
    /// stderr 테일(없으면 점검 힌트)을 덧붙인다.
    #[test]
    fn failure_text_distinguishes_idle_timeout_from_abnormal_exit() {
        let outcome = |timed_out, exit_desc: &str, stderr_tail: &str| TurnOutcome {
            session_id: "s".into(),
            timed_out,
            exit_desc: exit_desc.into(),
            stderr_tail: stderr_tail.into(),
            halted: None,
        };

        let timed_out = outcome(true, "signal 9", "").failure_text(14_400);
        assert!(timed_out.contains("유휴 타임아웃"), "{timed_out}");
        assert!(timed_out.contains("4시간"), "{timed_out}");

        // 시간으로 나누어떨어지지 않으면 분 단위로 표기한다.
        assert!(outcome(true, "signal 9", "")
            .failure_text(1_800)
            .contains("30분"));

        let with_stderr = outcome(false, "exit 3", "boom: no space left").failure_text(14_400);
        assert!(with_stderr.contains("결과 없이 종료"), "{with_stderr}");
        assert!(with_stderr.contains("exit 3"), "{with_stderr}");
        assert!(with_stderr.contains("boom: no space left"), "{with_stderr}");

        let bare = outcome(false, "exit 1", "  ").failure_text(14_400);
        assert!(bare.contains("디스크 여유 공간/인증 상태"), "{bare}");
    }

    #[test]
    fn parses_session_init() {
        let l = r#"{"type":"system","subtype":"init","session_id":"abc-123"}"#;
        assert_eq!(
            parse_event(l),
            ConvoEvent::SessionInit {
                session_id: "abc-123".into()
            }
        );
    }

    #[test]
    fn parses_assistant_text() {
        let l = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hello"}]}}"#;
        assert_eq!(
            parse_event(l),
            ConvoEvent::Text {
                text: "hello".into(),
                parent_id: None,
            }
        );
    }

    #[test]
    fn parses_tool_use_with_file_path_summary() {
        let l = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_01","name":"Edit","input":{"file_path":"src/main.rs"}}]}}"#;
        assert_eq!(
            parse_event(l),
            ConvoEvent::ToolUse {
                name: "Edit".into(),
                summary: "src/main.rs".into(),
                tool_id: Some("toolu_01".into()),
                parent_id: None,
            }
        );
    }

    #[test]
    fn parses_tool_result_string_content() {
        let l = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_01","content":"hello world","is_error":false}]}}"#;
        assert_eq!(
            parse_event(l),
            ConvoEvent::ToolResult {
                summary: "hello world".into(),
                is_error: false,
                result_chars: Some(11),
                tool_use_id: Some("toolu_01".into()),
                parent_id: None,
            }
        );
    }

    /// TodoWrite는 작업 계획을 통째로 다시 보내는 툴이라 캔버스 노드의 원천이 된다.
    /// ToolUse도 함께 나와야 기존 활동 표시가 깨지지 않는다.
    #[test]
    fn todowrite_becomes_plan_event_alongside_tool_use() {
        let l = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"TodoWrite","input":{"todos":[
            {"content":"스키마 확인","status":"completed","activeForm":"스키마 확인 중"},
            {"content":"주입 경로 수정","status":"in_progress","activeForm":"주입 경로 수정 중"}]}}]}}"#;
        let evs = parse_events(l);
        let items = evs
            .iter()
            .find_map(|e| match e {
                ConvoEvent::Plan { items, .. } => Some(items),
                _ => None,
            })
            .expect("Plan 이벤트가 없다");
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].status, "in_progress");
        assert_eq!(items[1].content, "주입 경로 수정");
        assert!(evs
            .iter()
            .any(|e| matches!(e, ConvoEvent::ToolUse { name, .. } if name == "TodoWrite")));
    }

    /// todos가 없거나 빈 TodoWrite는 Plan을 만들지 않는다 — 빈 캔버스를 그리지 않기 위해서.
    #[test]
    fn todowrite_without_items_emits_no_plan() {
        for input in [r#"{"todos":[]}"#, r#"{"other":1}"#] {
            let l = format!(
                r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"t1","name":"TodoWrite","input":{input}}}]}}}}"#
            );
            assert!(
                !parse_events(&l)
                    .iter()
                    .any(|e| matches!(e, ConvoEvent::Plan { .. })),
                "input={input}"
            );
        }
    }

    /// 요약은 절단되지만 원문 크기는 살아남는다 — 귀속 분석의 유일한 크기 신호라서
    /// 여기서 잃으면 복원할 곳이 없다.
    #[test]
    fn tool_result_records_original_char_count() {
        let big = "가".repeat(5000);
        let l = format!(
            r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"t1","content":"{big}"}}]}}}}"#
        );
        let ConvoEvent::ToolResult {
            result_chars,
            summary,
            ..
        } = parse_event(&l)
        else {
            panic!("tool_result 이벤트가 아니다");
        };
        assert!(summary.chars().count() <= 400);
        assert_eq!(result_chars, Some(5000));
    }

    #[test]
    fn parses_tool_result_array_content_and_error_flag() {
        let l = r#"{"type":"user","message":{"content":[{"type":"tool_result","content":[{"type":"text","text":"boom"}],"is_error":true}]}}"#;
        assert_eq!(
            parse_event(l),
            ConvoEvent::ToolResult {
                summary: "boom".into(),
                is_error: true,
                result_chars: Some(4),
                tool_use_id: None,
                parent_id: None,
            }
        );
    }

    #[test]
    fn parses_subagent_events_with_parent_tool_use_id() {
        // 서브 에이전트(Task) 소속 이벤트 — 최상위 parent_tool_use_id가 실려 온다.
        let use_l = r#"{"type":"assistant","parent_tool_use_id":"toolu_task1","message":{"content":[{"type":"tool_use","id":"toolu_sub1","name":"Read","input":{"file_path":"a.rs"}}]}}"#;
        assert_eq!(
            parse_event(use_l),
            ConvoEvent::ToolUse {
                name: "Read".into(),
                summary: "a.rs".into(),
                tool_id: Some("toolu_sub1".into()),
                parent_id: Some("toolu_task1".into()),
            }
        );
        let res_l = r#"{"type":"user","parent_tool_use_id":"toolu_task1","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_sub1","content":"ok","is_error":false}]}}"#;
        assert_eq!(
            parse_event(res_l),
            ConvoEvent::ToolResult {
                summary: "ok".into(),
                is_error: false,
                result_chars: Some(2),
                tool_use_id: Some("toolu_sub1".into()),
                parent_id: Some("toolu_task1".into()),
            }
        );
        // 서브 에이전트 텍스트 블록도 parent를 실어 별도 뷰에서 트랜스크립트 재구성 가능.
        let text_l = r#"{"type":"assistant","parent_tool_use_id":"toolu_task1","message":{"content":[{"type":"text","text":"조사 결과입니다"}]}}"#;
        assert_eq!(
            parse_event(text_l),
            ConvoEvent::Text {
                text: "조사 결과입니다".into(),
                parent_id: Some("toolu_task1".into()),
            }
        );
        // parent_tool_use_id: null(메인 스레드)은 None으로 정규화.
        let main_l = r#"{"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","id":"t2","name":"Task","input":{"description":"버그 조사"}}]}}"#;
        match parse_event(main_l) {
            ConvoEvent::ToolUse {
                name,
                summary,
                tool_id,
                parent_id,
            } => {
                assert_eq!(name, "Task");
                assert_eq!(summary, "버그 조사");
                assert_eq!(tool_id.as_deref(), Some("t2"));
                assert_eq!(parent_id, None);
            }
            other => panic!("expected tool_use, got {other:?}"),
        }
    }

    #[test]
    fn parses_subagent_model_from_parented_assistant_lines() {
        // 서브 에이전트 카드가 이름할 모델은 그 턴을 실제로 돌린 message.model이다.
        let l = r#"{"type":"assistant","parent_tool_use_id":"toolu_task1","message":{"model":"claude-opus-5[1m]","content":[{"type":"text","text":"조사 결과입니다"}]}}"#;
        let evs = parse_events(l);
        assert_eq!(
            evs,
            vec![
                ConvoEvent::SubagentModel {
                    parent_id: "toolu_task1".into(),
                    model: "claude-opus-5[1m]".into(),
                },
                ConvoEvent::Text {
                    text: "조사 결과입니다".into(),
                    parent_id: Some("toolu_task1".into()),
                },
            ]
        );
        // 메인 스레드 모델은 system/init이 담당한다 — 여기서 겹쳐 내면 세션 칩이 오염된다.
        let main_l = r#"{"type":"assistant","message":{"model":"claude-opus-5","content":[{"type":"text","text":"답변"}]}}"#;
        assert!(!parse_events(main_l)
            .iter()
            .any(|ev| matches!(ev, ConvoEvent::SubagentModel { .. })));
        // <synthetic>은 관측값이 아니라 하니스가 만든 자리표시자다 — normalize가 걸러낸다.
        let synthetic_l = r#"{"type":"assistant","parent_tool_use_id":"toolu_task1","message":{"model":"<synthetic>","content":[{"type":"text","text":"중단"}]}}"#;
        assert!(!parse_events(synthetic_l)
            .iter()
            .any(|ev| matches!(ev, ConvoEvent::SubagentModel { .. })));
    }

    #[test]
    fn parse_events_emits_every_block_in_one_line() {
        // 병렬 도구 호출 — Task 스폰이 첫 블록이 아니어도 유실되면 안 된다(리뷰 지적).
        let multi_use = r#"{"type":"assistant","message":{"content":[
            {"type":"tool_use","id":"t_read","name":"Read","input":{"file_path":"a.rs"}},
            {"type":"tool_use","id":"t_task","name":"Task","input":{"description":"조사"}}
        ]}}"#;
        let evs = parse_events(multi_use);
        assert_eq!(evs.len(), 2);
        assert!(matches!(
            &evs[1],
            ConvoEvent::ToolUse { name, tool_id: Some(id), .. }
                if name == "Task" && id == "t_task"
        ));
        // 배치된 tool_result — Task 결과가 둘째 블록이어도 방출(영구 running 오표시 방지).
        let multi_res = r#"{"type":"user","message":{"content":[
            {"type":"tool_result","tool_use_id":"t_read","content":"ok"},
            {"type":"tool_result","tool_use_id":"t_task","content":"조사 완료"}
        ]}}"#;
        let evs = parse_events(multi_res);
        assert_eq!(evs.len(), 2);
        assert!(matches!(
            &evs[1],
            ConvoEvent::ToolResult { tool_use_id: Some(id), .. } if id == "t_task"
        ));
        // 텍스트 + 툴 혼합 줄도 둘 다 방출.
        let mixed = r#"{"type":"assistant","message":{"content":[
            {"type":"text","text":"먼저 읽겠습니다"},
            {"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"b.rs"}}
        ]}}"#;
        assert_eq!(parse_events(mixed).len(), 2);
        // 무관/노이즈 줄은 빈 벡터 — 파이프라인은 유지된다.
        assert!(parse_events("not json").is_empty());
        assert!(parse_events(r#"{"type":"rate_limit_event"}"#).is_empty());
    }

    #[test]
    fn optional_correlation_fields_are_omitted_from_json_when_none() {
        // 영속 포맷 호환 — None 필드는 직렬화에서 빠져 기존 이벤트 JSON과 동형 유지.
        let ev = ConvoEvent::ToolUse {
            name: "shell".into(),
            summary: "ls".into(),
            tool_id: None,
            parent_id: None,
        };
        let j = serde_json::to_string(&ev).unwrap();
        assert!(!j.contains("\"id\""), "j={j}");
        assert!(!j.contains("parent_id"), "j={j}");
    }

    #[test]
    fn parses_result_with_cost_turns_and_tokens() {
        let l = r#"{"type":"result","subtype":"success","is_error":false,"result":"done","session_id":"s1","total_cost_usd":0.11,"num_turns":3,"usage":{"input_tokens":1200,"output_tokens":45}}"#;
        assert_eq!(
            parse_event(l),
            ConvoEvent::Result {
                text: "done".into(),
                is_error: false,
                session_id: "s1".into(),
                cost_usd: 0.11,
                num_turns: 3,
                tokens_in: 1200,
                tokens_out: 45,
            }
        );
    }

    #[test]
    fn claude_assistant_usage_emits_context_with_cache_tokens_summed() {
        // input은 캐시 제외 값 — cache_creation/cache_read 합산이 실제 컨텍스트 점유.
        let l = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":1200,"cache_creation_input_tokens":300,"cache_read_input_tokens":80000,"output_tokens":45}}}"#;
        let evs = parse_events(l);
        assert!(matches!(
            evs.last(),
            Some(ConvoEvent::ContextUsage {
                context_tokens: 81500,
                source: Some(source),
                valid: Some(true),
                ..
            }) if source == "claude_message"
        ));
    }

    #[test]
    fn subagent_assistant_usage_is_not_context() {
        // 서브 에이전트(Task) 사이드체인은 별도 컨텍스트 — 메인 CTX %를 오염시키면 안 된다.
        let l = r#"{"type":"assistant","parent_tool_use_id":"tu_1","message":{"content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":500,"cache_read_input_tokens":9000}}}"#;
        assert!(parse_events(l)
            .iter()
            .all(|e| !matches!(e, ConvoEvent::ContextUsage { .. })));
    }

    #[test]
    fn codex_turn_completed_does_not_emit_cumulative_context_usage() {
        let l = r#"{"type":"turn.completed","usage":{"input_tokens":17473,"cached_input_tokens":10624,"output_tokens":23}}"#;
        let evs = parse_codex_events(l);
        assert!(evs
            .iter()
            .all(|event| !matches!(event, ConvoEvent::ContextUsage { .. })));
        assert!(matches!(evs.first(), Some(ConvoEvent::Result { .. })));
    }

    #[test]
    fn codex_context_uses_only_current_complete_tail_observation() {
        const ID: &str = "019f205d-33b8-7111-93b8-63fe9eaea592";
        const START: i64 = 1_704_067_200_000;
        let meta = format!(r#"{{"type":"session_meta","payload":{{"id":"{ID}"}}}}"#);
        let token = r#"{"timestamp":"2024-01-01T00:00:05Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":7451604,"last_token_usage":{"total_tokens":147043},"model_context_window":258400}}}"#;
        let path =
            crate::testtmp::dir().join(format!("context-observation-{}", std::process::id()));
        std::fs::write(&path, format!("{meta}\n{token}\n")).unwrap();
        assert!(matches!(
            context_observation::codex_context_in(&path, ID, START, START + 10_000),
            Some(ConvoEvent::ContextUsage {
                context_tokens: 147043,
                context_window: Some(258400),
                valid: Some(true),
                ..
            })
        ));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn codex_context_ignores_head_metrics_and_keeps_exact_tail_lines() {
        const ID: &str = "019f205d-33b8-7111-93b8-63fe9eaea592";
        const START: i64 = 1_704_067_200_000;
        let meta = format!(r#"{{"type":"session_meta","payload":{{"id":"{ID}"}}}}"#);
        let token = r#"{"timestamp":"2024-01-01T00:00:05Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"total_tokens":147043},"model_context_window":258400}}}"#;
        let path = crate::testtmp::dir().join(format!("context-tail-{}", std::process::id()));
        let empty_tail = format!("{}\n", "x".repeat(context_observation::TAIL_BYTES - 1));
        std::fs::write(&path, format!("{meta}\n{token}\n{empty_tail}")).unwrap();
        assert_eq!(
            context_observation::codex_context_in(&path, ID, START, START + 10_000),
            None
        );
        let tail = format!(
            "{token}\n{}\n",
            "x".repeat(context_observation::TAIL_BYTES - token.len() - 2)
        );
        std::fs::write(&path, format!("{meta}\n{tail}")).unwrap();
        assert!(matches!(
            context_observation::codex_context_in(&path, ID, START, START + 10_000),
            Some(ConvoEvent::ContextUsage {
                context_tokens: 147043,
                ..
            })
        ));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn codex_context_rejects_invalid_latest_boundary_and_partial_records() {
        const ID: &str = "019f205d-33b8-7111-93b8-63fe9eaea592";
        const START: i64 = 1_704_067_200_000;
        let meta = format!(r#"{{"type":"session_meta","payload":{{"id":"{ID}"}}}}"#);
        let token = r#"{"timestamp":"2024-01-01T00:00:05Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"total_tokens":147043},"model_context_window":258400}}}"#;
        for (identity, tail) in [
            (meta.as_str(), "{\"type\":\"compacted\"}\n"),
            (meta.as_str(), "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\"}}\n"),
            (meta.as_str(), "{\"timestamp\":\"2023-12-31T23:59:59Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"total_tokens\":147043},\"model_context_window\":258400}}}\n"),
            (meta.as_str(), "{\"timestamp\":\"2024-01-01T00:00:11Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"total_tokens\":147043},\"model_context_window\":258400}}}\n"),
            (r#"{"type":"session_meta","payload":{"id":"other"}}"#, token),
        ] {
            let path = crate::testtmp::dir().join(format!("context-observation-{}-{}", std::process::id(), tail.len()));
            std::fs::write(&path, format!("{identity}\n{token}\n{tail}")).unwrap();
            assert_eq!(context_observation::codex_context_in(&path, ID, START, START + 10_000), None);
            let _ = std::fs::remove_file(path);
        }
        let path = crate::testtmp::dir().join(format!(
            "context-observation-{}-partial",
            std::process::id()
        ));
        std::fs::write(&path, format!("{meta}\n{token}")).unwrap();
        assert_eq!(
            context_observation::codex_context_in(&path, ID, START, START + 10_000),
            None
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn legacy_context_usage_omits_new_optional_fields() {
        let event = ConvoEvent::ContextUsage {
            context_tokens: 1,
            context_window: None,
            observed_at: None,
            source: None,
            valid: None,
        };
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            serde_json::json!({"kind":"context_usage","context_tokens":1})
        );
    }

    #[test]
    fn claude_system_init_emits_session_and_observed_model() {
        let events = parse_events(
            r#"{"type":"system","subtype":"init","session_id":"s1","model":"claude-opus-4-8"}"#,
        );

        assert_eq!(
            events,
            vec![
                ConvoEvent::SessionInit {
                    session_id: "s1".into()
                },
                ConvoEvent::ModelSnapshot {
                    requested: None,
                    resolved: Some("claude-opus-4-8".into()),
                    source: "claude_stream".into(),
                },
            ]
        );
    }

    /// 회귀: 컨텍스트 변형 접미사(`[1m]`)가 이름 화이트리스트에 없어, 그 모델로 돈 턴은
    /// 스냅샷이 아예 방출되지 않았다. 위 테스트가 대괄호 없는 ID만 써서 놓친 경로다.
    #[test]
    fn claude_system_init_observes_context_variant_model() {
        let events = parse_events(
            r#"{"type":"system","subtype":"init","session_id":"s1","model":"claude-opus-5[1m]"}"#,
        );

        assert_eq!(
            events.last(),
            Some(&ConvoEvent::ModelSnapshot {
                requested: None,
                resolved: Some("claude-opus-5[1m]".into()),
                source: "claude_stream".into(),
            })
        );
    }

    #[test]
    fn unknown_or_noise_is_other_not_panic() {
        assert_eq!(parse_event("not json"), ConvoEvent::Other);
        assert_eq!(parse_event(""), ConvoEvent::Other);
        assert_eq!(
            parse_event(r#"{"type":"rate_limit_event"}"#),
            ConvoEvent::Other
        );
        assert_eq!(
            parse_event(r#"{"type":"system","subtype":"hook_started"}"#),
            ConvoEvent::Other
        );
    }

    #[test]
    fn vendor_from_agent_routes_gemini_to_agy() {
        assert_eq!(Vendor::from_agent("codex"), Vendor::Codex);
        assert_eq!(Vendor::from_agent("agy"), Vendor::Agy);
        assert_eq!(Vendor::from_agent("antigravity"), Vendor::Agy);
        assert_eq!(
            Vendor::from_agent("gemini"),
            Vendor::Agy,
            "개인용 gemini CLI 인증 폐기 — agy 라우팅"
        );
        assert_eq!(Vendor::from_agent("claude"), Vendor::Claude);
        assert_eq!(Vendor::from_agent(""), Vendor::Claude);
        assert_eq!(Vendor::from_agent("custom-agent"), Vendor::Claude);
    }

    // codex fixture — codex-cli 0.141.0 `exec --json` 실측 라인.
    #[test]
    fn codex_thread_started_is_session_init() {
        let l = r#"{"type":"thread.started","thread_id":"019f205d-33b8-7111-93b8-63fe9eaea592"}"#;
        assert_eq!(
            parse_codex_event(l),
            ConvoEvent::SessionInit {
                session_id: "019f205d-33b8-7111-93b8-63fe9eaea592".into()
            }
        );
    }

    #[test]
    fn codex_agent_message_is_text() {
        let l = r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"OK"}}"#;
        assert_eq!(
            parse_codex_event(l),
            ConvoEvent::Text {
                text: "OK".into(),
                parent_id: None,
            }
        );
    }

    #[test]
    fn codex_command_execution_maps_to_tool_use_then_result() {
        let started = r#"{"type":"item.started","item":{"id":"item_0","type":"command_execution","command":"/bin/zsh -lc 'echo praxis-smoke'","aggregated_output":"","exit_code":null,"status":"in_progress"}}"#;
        assert_eq!(
            parse_codex_event(started),
            ConvoEvent::ToolUse {
                name: "shell".into(),
                summary: "/bin/zsh -lc 'echo praxis-smoke'".into(),
                tool_id: None,
                parent_id: None,
            }
        );
        let done = r#"{"type":"item.completed","item":{"id":"item_0","type":"command_execution","command":"/bin/zsh -lc 'echo praxis-smoke'","aggregated_output":"praxis-smoke\n","exit_code":0,"status":"completed"}}"#;
        assert_eq!(
            parse_codex_event(done),
            ConvoEvent::ToolResult {
                summary: "praxis-smoke".into(),
                is_error: false,
                // aggregated_output "praxis-smoke\n" — 요약은 개행을 버리지만 크기는 원문 기준.
                result_chars: Some(13),
                tool_use_id: None,
                parent_id: None,
            }
        );
        let failed = r#"{"type":"item.completed","item":{"id":"item_1","type":"command_execution","command":"false","aggregated_output":"","exit_code":1,"status":"failed"}}"#;
        assert_eq!(
            parse_codex_event(failed),
            ConvoEvent::ToolResult {
                summary: "".into(),
                is_error: true,
                result_chars: Some(0),
                tool_use_id: None,
                parent_id: None,
            }
        );
    }

    #[test]
    fn codex_file_change_maps_to_edit_tool_use_then_result() {
        // codex-cli 0.141 apply_patch 실측 fixture (경로만 축약).
        let started = r#"{"type":"item.started","item":{"id":"item_4","type":"file_change","changes":[{"path":"/tmp/x/hello.txt","kind":"update"}],"status":"in_progress"}}"#;
        assert_eq!(
            parse_codex_event(started),
            ConvoEvent::ToolUse {
                name: "edit".into(),
                summary: "update /tmp/x/hello.txt".into(),
                tool_id: None,
                parent_id: None,
            }
        );
        let done = r#"{"type":"item.completed","item":{"id":"item_4","type":"file_change","changes":[{"path":"/tmp/x/hello.txt","kind":"update"},{"path":"/tmp/x/new.rs","kind":"add"}],"status":"completed"}}"#;
        assert_eq!(
            parse_codex_event(done),
            ConvoEvent::ToolResult {
                summary: "update /tmp/x/hello.txt, add /tmp/x/new.rs".into(),
                is_error: false,
                result_chars: None,
                tool_use_id: None,
                parent_id: None,
            }
        );
    }

    #[test]
    fn codex_turn_events_map_to_result_and_noise_to_other() {
        let done = r#"{"type":"turn.completed","usage":{"input_tokens":17473,"cached_input_tokens":10624,"output_tokens":23}}"#;
        assert_eq!(
            parse_codex_event(done),
            ConvoEvent::Result {
                text: "".into(),
                is_error: false,
                session_id: "".into(),
                cost_usd: 0.0,
                num_turns: 0,
                tokens_in: 17473,
                tokens_out: 23,
            }
        );
        assert_eq!(
            parse_codex_event(r#"{"type":"turn.started"}"#),
            ConvoEvent::Other
        );
        assert_eq!(parse_codex_event("not json"), ConvoEvent::Other);
        let failed = r#"{"type":"turn.failed","error":{"message":"quota exceeded"}}"#;
        match parse_codex_event(failed) {
            ConvoEvent::Result {
                is_error: true,
                text,
                ..
            } => assert_eq!(text, "quota exceeded"),
            other => panic!("expected error result, got {other:?}"),
        }
    }

    #[test]
    fn codex_item_started_agent_message_and_reasoning_are_other() {
        // item.started(agent_message)를 Text로 승격하면 조기/중복 텍스트가 뜬다 — Other여야 함.
        let started_msg =
            r#"{"type":"item.started","item":{"id":"x","type":"agent_message","text":"early"}}"#;
        assert_eq!(parse_codex_event(started_msg), ConvoEvent::Other);
        let reasoning =
            r#"{"type":"item.completed","item":{"id":"x","type":"reasoning","text":"..."}}"#;
        assert_eq!(parse_codex_event(reasoning), ConvoEvent::Other);
        let empty_msg =
            r#"{"type":"item.completed","item":{"id":"x","type":"agent_message","text":""}}"#;
        assert_eq!(parse_codex_event(empty_msg), ConvoEvent::Other);
    }

    #[test]
    fn codex_turn_failed_without_message_uses_fallback() {
        for l in [
            r#"{"type":"turn.failed"}"#,
            r#"{"type":"turn.failed","error":{}}"#,
        ] {
            match parse_codex_event(l) {
                ConvoEvent::Result {
                    is_error: true,
                    text,
                    ..
                } => assert_eq!(text, "codex 턴 실패"),
                other => panic!("expected fallback error result, got {other:?}"),
            }
        }
    }

    #[test]
    fn claude_system_init_without_session_is_other() {
        // session_id 없는 init → Other (run_turn이 빈 세션으로 resume 깨지지 않게 Err 유도).
        assert_eq!(
            parse_event(r#"{"type":"system","subtype":"init"}"#),
            ConvoEvent::Other
        );
    }

    #[test]
    fn tool_summary_prefers_common_keys_then_truncates() {
        let v = serde_json::json!({"command": "x".repeat(200)});
        if let ConvoEvent::ToolUse { summary, .. } = parse_event(
            &serde_json::json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input": v}]}}).to_string()
        ) {
            assert!(summary.len() <= 120);
        } else {
            panic!("expected tool_use");
        }
    }

    /// `std::process::Command`의 인자를 문자열 벡터로 뽑아 assert 비교하기 위한 헬퍼.
    fn cmd_args(c: &std::process::Command) -> Vec<String> {
        c.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn vendor_command_model_none_or_empty_is_unchanged() {
        for m in [None, Some(""), Some("   ")] {
            assert_eq!(
                cmd_args(&vendor_command("claude", Vendor::Claude, "hi", None, m)),
                cmd_args(&vendor_command("claude", Vendor::Claude, "hi", None, None))
            );
        }
    }

    #[test]
    fn vendor_command_claude_model_flag() {
        let args = cmd_args(&vendor_command(
            "claude",
            Vendor::Claude,
            "hi",
            None,
            Some("opus"),
        ));
        assert_eq!(args.last(), Some(&"opus".to_string()));
        assert_eq!(args[args.len() - 2], "--model");
    }

    #[test]
    fn vendor_command_codex_model_after_exec_subcommand() {
        let args = cmd_args(&vendor_command(
            "codex",
            Vendor::Codex,
            "hi",
            None,
            Some("o3"),
        ));
        assert_eq!(args[0], "exec");
        assert_eq!(args[1], "-m");
        assert_eq!(args[2], "o3");
    }

    #[test]
    fn vendor_command_codex_resume_model_after_resume_subcommand() {
        let args = cmd_args(&vendor_command(
            "codex",
            Vendor::Codex,
            "hi",
            Some("sid1"),
            Some("o3"),
        ));
        assert_eq!(&args[..3], &["exec", "resume", "sid1"]);
        assert_eq!(args[3], "-m");
        assert_eq!(args[4], "o3");
    }

    #[test]
    fn vendor_command_codex_initial_reasoning_effort_is_a_config_override() {
        let args = cmd_args(&vendor_command_with_effort(
            "codex",
            Vendor::Codex,
            "hi",
            None,
            Some("gpt-5.6-sol"),
            Some("high"),
            &[],
            None,
            None,
        ));
        assert_eq!(
            &args[..5],
            &[
                "exec",
                "-m",
                "gpt-5.6-sol",
                "-c",
                "model_reasoning_effort=\"high\"",
            ]
        );
    }

    fn mcp_fixture() -> crate::preview_bridge::mcp::McpInjection {
        crate::preview_bridge::mcp::McpInjection {
            args: vec!["-c".to_string(), "mcp_servers.praxis_preview.url=\"u\"".to_string()],
            env: vec![("PRAXIS_PREVIEW_TOKEN".to_string(), "tok".to_string())],
            approvals: false,
        }
    }

    /// codex는 마지막 위치인자가 프롬프트다 — 주입 인자가 `--json` 꼬리 앞에 와야 한다.
    #[test]
    fn vendor_command_codex_mcp_args_precede_the_json_tail() {
        let mcp = mcp_fixture();
        let args = cmd_args(&vendor_command_with_effort(
            "codex",
            Vendor::Codex,
            "hi",
            None,
            None,
            None,
            &[],
            None,
            Some(&mcp),
        ));
        let at = args.iter().position(|a| a.starts_with("mcp_servers.")).unwrap();
        let json = args.iter().position(|a| a == "--json").unwrap();
        assert!(at < json);
        assert!(!args.iter().any(|a| a.contains("tok")));
    }

    #[test]
    fn vendor_command_claude_mcp_args_follow_the_effort_flags() {
        let mcp = crate::preview_bridge::mcp::McpInjection {
            args: vec!["--mcp-config".to_string(), "/tmp/a.json".to_string()],
            env: vec![("PRAXIS_PREVIEW_TOKEN".to_string(), "tok".to_string())],
            approvals: false,
        };
        let args = cmd_args(&vendor_command_with_effort(
            "claude",
            Vendor::Claude,
            "hi",
            None,
            Some("opus"),
            None,
            &[],
            None,
            Some(&mcp),
        ));
        assert_eq!(&args[args.len() - 2..], &["--mcp-config", "/tmp/a.json"]);
        assert!(args.iter().position(|a| a == "--model").unwrap() < args.len() - 2);
        assert!(!args.iter().any(|a| a.contains("tok")));
    }

    /// 승인이 켜진 주입은 자동 승인을 빼고 우리 툴을 permission prompt로 건다. 꺼지면 예전 그대로다.
    #[test]
    fn vendor_command_claude_approvals_drop_skip_permissions() {
        let mut mcp = crate::preview_bridge::mcp::McpInjection {
            args: vec![
                "--permission-prompt-tool".to_string(),
                "mcp__praxis-preview__approve".to_string(),
            ],
            env: Vec::new(),
            approvals: true,
        };
        let build = |mcp: &crate::preview_bridge::mcp::McpInjection, questions| {
            cmd_args(&vendor_command_with_contract(
                "claude", Vendor::Claude, "hi", None, None, None, &[], None, Some(mcp), questions,
            ))
        };
        let args = build(&mcp, true);
        assert!(!args.iter().any(|a| a == "--dangerously-skip-permissions"));
        assert!(args.iter().any(|a| a == "--permission-prompt-tool"));
        assert!(args.iter().any(|a| a == "--include-partial-messages"));
        mcp.approvals = false;
        let args = build(&mcp, false);
        assert!(args.iter().any(|a| a == "--dangerously-skip-permissions"));
        assert!(!args.iter().any(|a| a == "--include-partial-messages"));
    }

    /// 주입이 없으면 argv는 예전 그대로다 — 서버가 안 떴을 때의 회귀 0 계약.
    #[test]
    fn vendor_command_without_mcp_is_unchanged() {
        for vendor in [Vendor::Claude, Vendor::Codex, Vendor::Agy] {
            let args = cmd_args(&vendor_command_with_effort(
                "bin", vendor, "hi", None, None, None, &[], None, None,
            ));
            assert!(!args.iter().any(|a| a.contains("mcp")));
        }
    }

    #[test]
    fn vendor_command_codex_resume_reasoning_effort_follows_session_id() {
        let args = cmd_args(&vendor_command_with_effort(
            "codex",
            Vendor::Codex,
            "hi",
            Some("sid1"),
            None,
            Some("xhigh"),
            &[],
            None,
            None,
        ));
        assert_eq!(
            &args[..5],
            &[
                "exec",
                "resume",
                "sid1",
                "-c",
                "model_reasoning_effort=\"xhigh\"",
            ]
        );
    }

    #[test]
    fn vendor_command_claude_initial_reasoning_effort_follows_model_flag() {
        let args = cmd_args(&vendor_command_with_effort(
            "claude",
            Vendor::Claude,
            "hi",
            None,
            Some("opus"),
            Some("high"),
            &[],
            None,
            None,
        ));
        assert_eq!(
            &args[args.len() - 4..],
            &["--model", "opus", "--effort", "high"]
        );
    }

    #[test]
    fn vendor_command_claude_resume_reasoning_effort_follows_session_id() {
        let args = cmd_args(&vendor_command_with_effort(
            "claude",
            Vendor::Claude,
            "hi",
            Some("sid1"),
            None,
            Some("max"),
            &[],
            None,
            None,
        ));
        assert_eq!(
            &args[args.len() - 4..],
            &["--resume", "sid1", "--effort", "max"]
        );
    }

    #[test]
    fn vendor_command_claude_session_name_becomes_dash_n() {
        let args = cmd_args(&vendor_command_with_effort(
            "claude",
            Vendor::Claude,
            "hi",
            Some("sid1"),
            None,
            None,
            &[],
            Some("task-42"),
            None,
        ));
        let at = args.iter().position(|a| a == "-n").expect("-n 누락");
        assert_eq!(args[at + 1], "task-42");
    }

    #[test]
    fn vendor_command_non_claude_never_gets_dash_n() {
        // codex/agy에는 대응 플래그가 없다 — 붙으면 플래그 오류로 즉사한다.
        for (bin, vendor) in [("codex", Vendor::Codex), ("agy", Vendor::Agy)] {
            let args = cmd_args(&vendor_command_with_effort(
                bin,
                vendor,
                "hi",
                None,
                None,
                None,
                &[],
                Some("task-42"),
                None,
            ));
            assert!(!args.iter().any(|a| a == "-n"), "{bin}에 -n이 붙었다");
        }
    }

    #[test]
    fn vendor_command_agy_raises_print_timeout() {
        // agy print 모드 자체 타임아웃 기본 5m — 미주입 시 5분 넘는 턴이 전부
        // "timeout waiting for response"(exit 1) + 무출력으로 죽는다. 첫 턴/resume 모두 주입.
        for resume in [None, Some(AGY_CONTINUE)] {
            let args = cmd_args(&vendor_command("agy", Vendor::Agy, "hi", resume, None));
            let pos = args
                .iter()
                .position(|a| a == "--print-timeout")
                .expect("--print-timeout 주입");
            assert_eq!(args[pos + 1], AGY_PRINT_TIMEOUT);
        }
    }

    #[test]
    fn vendor_command_agy_model_long_flag() {
        // agy CLI는 `-m` 단축 플래그가 없다 — `--model`만 유효.
        let args = cmd_args(&vendor_command(
            "agy",
            Vendor::Agy,
            "hi",
            None,
            Some("gemini-3-pro"),
        ));
        assert_eq!(args.last(), Some(&"gemini-3-pro".to_string()));
        assert_eq!(args[args.len() - 2], "--model");
    }

    #[test]
    fn vendor_command_agy_initial_effort_follows_unchanged_model() {
        let args = cmd_args(&vendor_command_with_effort(
            "agy",
            Vendor::Agy,
            "hi",
            None,
            Some("gemini-3.6-flash-high"),
            Some("high"),
            &[],
            None,
            None,
        ));
        assert_eq!(
            &args[args.len() - 4..],
            &["--model", "gemini-3.6-flash-high", "--effort", "high"]
        );
    }

    #[test]
    fn vendor_command_agy_resume_effort_follows_continue_and_model() {
        let args = cmd_args(&vendor_command_with_effort(
            "agy",
            Vendor::Agy,
            "hi",
            Some(AGY_CONTINUE),
            Some("gemini-3.6-flash-high"),
            Some("medium"),
            &[],
            None,
            None,
        ));
        assert_eq!(
            &args[args.len() - 5..],
            &[
                "--continue",
                "--model",
                "gemini-3.6-flash-high",
                "--effort",
                "medium"
            ]
        );
    }

    #[test]
    fn vendor_command_agy_initial_and_resume_omit_blank_effort() {
        for resume in [None, Some(AGY_CONTINUE)] {
            let args = cmd_args(&vendor_command_with_effort(
                "agy",
                Vendor::Agy,
                "hi",
                resume,
                Some("gemini-3.6-flash-high"),
                Some("  "),
                &[],
                None,
                None,
            ));

            assert!(!args.iter().any(|arg| arg == "--effort"));
        }
    }

    #[test]
    fn vendor_command_codex_attaches_each_image_before_the_prompt() {
        let images = vec![
            "/tmp/editor.png".to_string(),
            "/tmp/preview.png".to_string(),
        ];
        let args = cmd_args(&vendor_command_with_effort(
            "codex",
            Vendor::Codex,
            "이미지를 검토해줘",
            Some("sid1"),
            None,
            None,
            &images,
            None,
            None,
        ));
        let image_flags: Vec<_> = args
            .windows(2)
            .filter(|pair| pair[0] == "--image")
            .map(|pair| pair[1].as_str())
            .collect();
        assert_eq!(image_flags, ["/tmp/editor.png", "/tmp/preview.png"]);
        assert!(args
            .iter()
            .any(|argument| argument.contains("이미지를 검토해줘")));
    }
}
