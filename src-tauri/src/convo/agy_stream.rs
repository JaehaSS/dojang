//! agy `--output-format stream-json` 한 줄 → `ConvoEvent` (CLI 1.3.1 실측).
//!
//! 순서: `init`(conversation_id) → `step_update`… → `result`. 한 step은 `step_index`로 묶이고
//! ACTIVE가 여러 번, 끝 상태(DONE 등)가 한 번 온다.
//! - `tool`: ACTIVE에서 ToolUse, 끝 상태에서 ToolResult. ACTIVE 없이 끝 상태만 오면 둘 다 낸다.
//! - `agent_response`: `text_delta`를 모았다가 끝 상태에서 Text 한 블록으로 낸다. 러너 경로는
//!   이벤트를 전부 그대로 적재하므로 미완성 스냅샷(`TextUpdate`)을 흘리지 않는다 — codex
//!   `agent_message`와 같은 단위다. 끝 상태의 `usage`는 그 호출이 실은 컨텍스트다.
//! - `result`: 턴 결과. `conversation_id`가 `--conversation` resume 토큰이다.
//!
//! JSON이 아닌 줄은 구버전(텍스트 print) 출력으로 보고 모아 두었다가, 구조화 `result`가 끝내
//! 오지 않았을 때만 `finish`가 텍스트 1블록 + `--continue` 센티널 결과로 낸다.

use super::{ConvoEvent, AGY_CONTINUE};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// 툴 파라미터에서 요약으로 먼저 고를 키 — agy 툴 인자는 PascalCase다.
const SUMMARY_KEYS: &[&str] = &[
    "CommandLine",
    "AbsolutePath",
    "TargetFile",
    "Url",
    "Query",
    "SearchPath",
    "DirectoryPath",
];

#[derive(Default)]
pub(crate) struct AgyStream {
    session_id: Option<String>,
    texts: HashMap<i64, String>,
    open_tools: HashSet<i64>,
    /// 사용자에게 보이는 이벤트(Text/ToolUse)를 낸 적이 있는가 — 503 재시도 판정 재료.
    produced_output: bool,
    result_seen: bool,
    plain: String,
}

impl AgyStream {
    pub(crate) fn observe_line(&mut self, line: &str) -> Vec<ConvoEvent> {
        let line = line.trim();
        if line.is_empty() {
            return Vec::new();
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            self.plain.push_str(line);
            self.plain.push('\n');
            return Vec::new();
        };
        let events = match v.get("event").and_then(Value::as_str) {
            Some("init") => self.init(&v),
            Some("step_update") => v.get("step_update").map(|s| self.step(s)).unwrap_or_default(),
            Some("result") => v.get("result").map(|r| self.result(r)).unwrap_or_default(),
            _ => Vec::new(),
        };
        self.produced_output |= events
            .iter()
            .any(|e| matches!(e, ConvoEvent::Text { .. } | ConvoEvent::ToolUse { .. }));
        events
    }

    /// 프로세스 종료 후 호출. 구조화 결과 없이 텍스트만 나왔다면 그것을 답으로 낸다.
    pub(crate) fn finish(&mut self) -> Vec<ConvoEvent> {
        let text = self.plain.trim();
        if self.result_seen || text.is_empty() {
            return Vec::new();
        }
        self.produced_output = true;
        self.session_id.get_or_insert_with(|| AGY_CONTINUE.to_string());
        vec![
            ConvoEvent::Text { text: text.to_string(), parent_id: None },
            ConvoEvent::Result {
                text: String::new(),
                is_error: false,
                session_id: AGY_CONTINUE.into(),
                cost_usd: 0.0,
                num_turns: 0,
                tokens_in: 0,
                tokens_out: 0,
            },
        ]
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub(crate) fn produced_output(&self) -> bool {
        self.produced_output
    }

    fn remember_session(&mut self, id: Option<&str>) -> Option<String> {
        let id = id.map(str::trim).filter(|id| !id.is_empty())?;
        self.session_id = Some(id.to_string());
        Some(id.to_string())
    }

    fn init(&mut self, v: &Value) -> Vec<ConvoEvent> {
        self.remember_session(v.get("conversation_id").and_then(Value::as_str))
            .map(|session_id| vec![ConvoEvent::SessionInit { session_id }])
            .unwrap_or_default()
    }

    fn step(&mut self, step: &Value) -> Vec<ConvoEvent> {
        let index = step.get("step_index").and_then(Value::as_i64).unwrap_or(-1);
        let done = step.get("state").and_then(Value::as_str) != Some("ACTIVE");
        match step.get("step_type").and_then(Value::as_str) {
            Some("tool") => self.tool(step, index, done),
            Some("agent_response") => self.response(step, index, done),
            _ => Vec::new(),
        }
    }

    fn tool(&mut self, step: &Value, index: i64, done: bool) -> Vec<ConvoEvent> {
        let info = step.get("tool_info");
        let tool_id = format!("agy-{index}");
        let mut out = Vec::new();
        if self.open_tools.insert(index) {
            let name = step
                .get("tool_name")
                .or_else(|| info.and_then(|i| i.get("name")))
                .and_then(Value::as_str)
                .unwrap_or("tool");
            out.push(ConvoEvent::ToolUse {
                name: name.to_string(),
                summary: tool_summary(info.and_then(|i| i.get("parameters"))),
                tool_id: Some(tool_id.clone()),
                parent_id: None,
            });
        }
        if done {
            self.open_tools.remove(&index);
            let error = info.and_then(|i| i.get("error")).and_then(Value::as_str);
            let output = error
                .or_else(|| info.and_then(|i| i.get("output")).and_then(Value::as_str))
                .unwrap_or("");
            let failed = error.is_some()
                || step.get("state").and_then(Value::as_str) != Some("DONE");
            out.push(ConvoEvent::ToolResult {
                summary: output
                    .lines()
                    .take(6)
                    .collect::<Vec<_>>()
                    .join("\n")
                    .chars()
                    .take(400)
                    .collect(),
                is_error: failed,
                result_chars: Some(output.chars().count() as i64),
                tool_use_id: Some(tool_id),
                parent_id: None,
            });
        }
        out
    }

    fn response(&mut self, step: &Value, index: i64, done: bool) -> Vec<ConvoEvent> {
        if let Some(delta) = step.get("text_delta").and_then(Value::as_str) {
            self.texts.entry(index).or_default().push_str(delta);
        }
        if !done {
            return Vec::new();
        }
        let mut out = Vec::new();
        let text = self.texts.remove(&index).unwrap_or_default();
        if !text.trim().is_empty() {
            out.push(ConvoEvent::Text { text: text.trim_end().to_string(), parent_id: None });
        }
        let usage = |key: &str| {
            step.get("usage").and_then(|u| u.get(key)).and_then(Value::as_i64).unwrap_or(0)
        };
        let context_tokens = usage("input_tokens") + usage("cache_read_tokens");
        if context_tokens > 0 {
            out.push(ConvoEvent::ContextUsage {
                context_tokens,
                context_window: None,
                observed_at: Some(crate::now()),
                source: Some("agy_step".into()),
                valid: Some(true),
            });
        }
        out
    }

    fn result(&mut self, result: &Value) -> Vec<ConvoEvent> {
        self.result_seen = true;
        let session_id = self
            .remember_session(result.get("conversation_id").and_then(Value::as_str))
            .unwrap_or_default();
        let status = result.get("status").and_then(Value::as_str).unwrap_or("");
        let is_error = status != "SUCCESS";
        let mut out = Vec::new();
        let response = result.get("response").and_then(Value::as_str).unwrap_or("").trim();
        // 델타 없이 최종 응답만 오는 경우에도 답이 사라지지 않게 한다.
        if !is_error && !self.produced_output && !response.is_empty() {
            out.push(ConvoEvent::Text { text: response.to_string(), parent_id: None });
        }
        let text = if is_error {
            result
                .get("error")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|e| !e.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("agy 턴 실패 ({status})"))
        } else {
            String::new()
        };
        let usage = |key: &str| {
            result.get("usage").and_then(|u| u.get(key)).and_then(Value::as_i64).unwrap_or(0)
        };
        out.push(ConvoEvent::Result {
            text,
            is_error,
            session_id,
            cost_usd: 0.0,
            num_turns: result.get("num_turns").and_then(Value::as_i64).unwrap_or(0),
            tokens_in: usage("input_tokens"),
            tokens_out: usage("output_tokens"),
        });
        out
    }
}

fn tool_summary(params: Option<&Value>) -> String {
    let Some(params) = params else {
        return String::new();
    };
    SUMMARY_KEYS
        .iter()
        .find_map(|key| params.get(*key).and_then(Value::as_str))
        .map(|s| s.chars().take(120).collect())
        .unwrap_or_else(|| super::tool_summary(params))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// agy 1.3.1 실측 스트림(툴 2회 + 응답 델타)에서 필요한 줄만 줄였다.
    const FIXTURE: &str = r#"{"event":"init","conversation_id":"c-1","init":{"cwd":"/tmp/p","tools":["view_file"]}}
{"event":"step_update","step_update":{"conversation_id":"c-1","step_index":0,"state":"DONE","step_type":"user_input"}}
{"event":"step_update","step_update":{"conversation_id":"c-1","step_index":1,"state":"DONE","step_type":"agent_response","usage":{"input_tokens":16289,"output_tokens":212,"cache_read_tokens":0}}}
{"event":"step_update","step_update":{"conversation_id":"c-1","step_index":2,"state":"ACTIVE","step_type":"tool","tool_name":"view_file","tool_info":{"name":"view_file","parameters":{"AbsolutePath":"/tmp/p/a.txt"}}}}
{"event":"step_update","step_update":{"conversation_id":"c-1","step_index":2,"state":"DONE","step_type":"tool","tool_name":"view_file","tool_info":{"name":"view_file","parameters":{"AbsolutePath":"/tmp/p/a.txt"},"output":"2 lines, 12 bytes"}}}
{"event":"step_update","step_update":{"conversation_id":"c-1","step_index":4,"state":"DONE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"ls"},"output":"a.txt\r\n"}}}
{"event":"step_update","step_update":{"conversation_id":"c-1","step_index":5,"state":"ACTIVE","step_type":"agent_response","text_delta":"a.txt에는 \"hell"}}
{"event":"step_update","step_update":{"conversation_id":"c-1","step_index":5,"state":"DONE","step_type":"agent_response","text_delta":"o\"가 있습니다.\n","usage":{"input_tokens":4664,"output_tokens":296,"cache_read_tokens":12222}}}
{"event":"result","result":{"conversation_id":"c-1","status":"SUCCESS","response":"a.txt에는 \"hello\"가 있습니다.\n","num_turns":1,"usage":{"input_tokens":37641,"output_tokens":604}}}"#;

    fn run(lines: &str) -> (AgyStream, Vec<ConvoEvent>) {
        let mut stream = AgyStream::default();
        let mut events: Vec<_> = lines.lines().flat_map(|l| stream.observe_line(l)).collect();
        events.extend(stream.finish());
        (stream, events)
    }

    #[test]
    fn structured_stream_maps_tools_text_and_result() {
        let (stream, events) = run(FIXTURE);
        assert_eq!(stream.session_id(), Some("c-1"));
        assert!(matches!(&events[0], ConvoEvent::SessionInit { session_id } if session_id == "c-1"));
        let tools: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ConvoEvent::ToolUse { name, summary, tool_id, .. } => {
                    Some((name.as_str(), summary.as_str(), tool_id.clone().unwrap()))
                }
                _ => None,
            })
            .collect();
        // ACTIVE 없이 DONE만 온 step 4도 카드가 생긴다. 같은 step의 ACTIVE·DONE은 한 장이다.
        assert_eq!(
            tools,
            vec![
                ("view_file", "/tmp/p/a.txt", "agy-2".to_string()),
                ("run_command", "ls", "agy-4".to_string())
            ]
        );
        assert!(events.iter().any(|e| matches!(e,
            ConvoEvent::ToolResult { summary, is_error: false, tool_use_id: Some(id), .. }
                if summary == "2 lines, 12 bytes" && id == "agy-2")));
        let texts: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ConvoEvent::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["a.txt에는 \"hello\"가 있습니다."], "델타는 한 블록, 결과 응답은 중복 금지");
        assert!(events.iter().any(|e| matches!(e,
            ConvoEvent::ContextUsage { context_tokens: 16886, .. })));
        assert!(matches!(events.last(), Some(ConvoEvent::Result {
            is_error: false, session_id, tokens_in: 37641, tokens_out: 604, num_turns: 1, ..
        }) if session_id == "c-1"));
        assert!(stream.produced_output());
    }

    #[test]
    fn error_result_carries_cli_error_and_no_session() {
        let line = r#"{"event":"result","result":{"conversation_id":"","status":"ERROR","response":"","error":"invalid model selection (--model \"x\")","num_turns":0,"usage":{"input_tokens":0}}}"#;
        let (stream, events) = run(line);
        assert_eq!(stream.session_id(), None);
        assert!(!stream.produced_output());
        assert!(matches!(events.as_slice(), [ConvoEvent::Result { is_error: true, text, .. }]
            if text.starts_with("invalid model selection")));
    }

    #[test]
    fn response_only_in_result_is_not_lost() {
        let line = r#"{"event":"result","result":{"conversation_id":"c-2","status":"SUCCESS","response":"done","usage":{}}}"#;
        let (_, events) = run(line);
        assert!(matches!(&events[0], ConvoEvent::Text { text, .. } if text == "done"));
    }

    #[test]
    fn plaintext_output_falls_back_to_one_block_with_continue_sentinel() {
        let (stream, events) = run("first line\nsecond line");
        assert_eq!(stream.session_id(), Some(AGY_CONTINUE));
        assert!(matches!(&events[0], ConvoEvent::Text { text, .. } if text == "first line\nsecond line"));
        assert!(matches!(&events[1], ConvoEvent::Result { session_id, is_error: false, .. }
            if session_id == AGY_CONTINUE));
    }
}
