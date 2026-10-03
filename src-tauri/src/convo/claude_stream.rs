//! claude `--include-partial-messages`의 텍스트 델타를 교체형 스냅샷(`TextUpdate`)으로 옮긴다.
//!
//! 순서는 2.1.283 실측이다: `content_block_start(text)` → `text_delta`… → 그 블록 하나만 실은
//! `assistant` 줄 → `content_block_stop`. 완성본(`assistant`)이 stop보다 **먼저** 온다. 그래서
//! 열린 텍스트 블록이 있는 동안 들어온 메인 스레드 `Text`는 그 블록의 완성본이고, 같은 `item_id`의
//! `complete: true`로 바꿔 스트리밍 말풍선을 제자리에서 닫는다. 적재는 complete만 한다
//! (`stored_event_json`이 `Text`로 되돌린다). 순서가 뒤집혀 stop이 먼저 닫았다면, 뒤이어 온 같은
//! 완성본은 흡수한다 — 그대로 흘리면 같은 말이 두 번 적재된다.
//!
//! 서브 에이전트(`parent_tool_use_id`) 델타는 다루지 않는다 — 그쪽은 기존 `Text`가 그대로 간다.

use super::ConvoEvent;
use serde_json::Value;

#[derive(Default)]
pub(crate) struct TextStream {
    message: String,
    open: Option<Block>,
    /// stop으로 먼저 닫힌 블록의 텍스트. 다음 블록이 열리기 전의 같은 완성본은 이미 보낸 것이다.
    closed: Option<String>,
}

struct Block {
    item_id: String,
    text: String,
}

impl TextStream {
    /// `stream_event` 줄이면 그 결과를 돌려준다. 아니면 `None` — 호출자가 평소대로 파싱한다.
    pub(crate) fn observe_line(&mut self, line: &str) -> Option<Vec<ConvoEvent>> {
        let v: Value = serde_json::from_str(line.trim()).ok()?;
        if v.get("type").and_then(Value::as_str) != Some("stream_event") {
            return None;
        }
        if v.get("parent_tool_use_id").is_some_and(|p| !p.is_null()) {
            return Some(Vec::new());
        }
        let Some(event) = v.get("event") else {
            return Some(Vec::new());
        };
        let index = event.get("index").and_then(Value::as_i64).unwrap_or(0);
        let out = match event.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                self.message = event
                    .pointer("/message/id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.open = None;
                self.closed = None;
                Vec::new()
            }
            Some("content_block_start")
                if event.pointer("/content_block/type").and_then(Value::as_str) == Some("text") =>
            {
                self.closed = None;
                self.open = Some(Block {
                    item_id: format!("{}:{index}", self.message),
                    text: String::new(),
                });
                Vec::new()
            }
            Some("content_block_delta")
                if event.pointer("/delta/type").and_then(Value::as_str) == Some("text_delta") =>
            {
                let delta = event.pointer("/delta/text").and_then(Value::as_str).unwrap_or("");
                match self.open.as_mut() {
                    Some(block) if !delta.is_empty() => {
                        block.text.push_str(delta);
                        vec![ConvoEvent::TextUpdate {
                            item_id: block.item_id.clone(),
                            text: block.text.clone(),
                            complete: false,
                        }]
                    }
                    _ => Vec::new(),
                }
            }
            // 완성본이 오지 않은 채 닫히면(빈 텍스트 등) 스트리밍 말풍선을 남긴 텍스트로 닫는다.
            Some("content_block_stop") => match self.open.take() {
                Some(block) if !block.text.trim().is_empty() => {
                    self.closed = Some(block.text.clone());
                    vec![ConvoEvent::TextUpdate {
                        item_id: block.item_id,
                        text: block.text,
                        complete: true,
                    }]
                }
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        Some(out)
    }

    /// 열린 블록의 완성본이면 같은 스트림 id로 닫는다. 이미 닫힌 블록의 완성본이면 버린다.
    /// 그 외 이벤트는 그대로 돌려준다.
    pub(crate) fn settle(&mut self, event: ConvoEvent) -> Option<ConvoEvent> {
        match event {
            ConvoEvent::Text {
                text,
                parent_id: None,
            } if self.open.is_some() => {
                let block = self.open.take().expect("checked above");
                Some(ConvoEvent::TextUpdate {
                    item_id: block.item_id,
                    text,
                    complete: true,
                })
            }
            ConvoEvent::Text {
                ref text,
                parent_id: None,
            } if self.closed.as_deref() == Some(text.as_str()) => {
                self.closed = None;
                None
            }
            other => Some(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(stream: &mut TextStream, lines: &[&str]) -> Vec<ConvoEvent> {
        let mut out = Vec::new();
        for line in lines {
            match stream.observe_line(line) {
                Some(events) => out.extend(events),
                None => out.extend(
                    super::super::parse_events(line)
                        .into_iter()
                        .filter_map(|event| stream.settle(event)),
                ),
            }
        }
        out
    }

    const START: &str = r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"msg_1"}},"parent_tool_use_id":null}"#;
    const TEXT_START: &str = r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}},"parent_tool_use_id":null}"#;
    const DELTA_A: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"안녕"}},"parent_tool_use_id":null}"#;
    const DELTA_B: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"하세요"}},"parent_tool_use_id":null}"#;
    const ASSISTANT: &str = r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"text","text":"안녕하세요"}]},"parent_tool_use_id":null}"#;
    const STOP: &str = r#"{"type":"stream_event","event":{"type":"content_block_stop","index":1},"parent_tool_use_id":null}"#;

    #[test]
    fn deltas_grow_one_snapshot_and_the_assistant_line_closes_it() {
        let mut stream = TextStream::default();
        let events = feed(&mut stream, &[START, TEXT_START, DELTA_A, DELTA_B, ASSISTANT, STOP]);
        let update = |text: &str, complete| ConvoEvent::TextUpdate {
            item_id: "msg_1:1".into(),
            text: text.into(),
            complete,
        };
        assert_eq!(
            events,
            vec![
                update("안녕", false),
                update("안녕하세요", false),
                update("안녕하세요", true),
            ]
        );
    }

    #[test]
    fn a_block_closed_without_its_assistant_line_still_completes() {
        let mut stream = TextStream::default();
        let events = feed(&mut stream, &[START, TEXT_START, DELTA_A, STOP]);
        assert_eq!(
            events.last(),
            Some(&ConvoEvent::TextUpdate {
                item_id: "msg_1:1".into(),
                text: "안녕".into(),
                complete: true,
            })
        );
    }

    #[test]
    fn an_assistant_line_after_stop_is_not_stored_twice() {
        let mut stream = TextStream::default();
        let events = feed(&mut stream, &[START, TEXT_START, DELTA_A, DELTA_B, STOP, ASSISTANT]);
        assert_eq!(events.len(), 3);
        assert!(matches!(
            events.last(),
            Some(ConvoEvent::TextUpdate { complete: true, .. })
        ));
    }

    #[test]
    fn text_outside_an_open_block_and_subagent_text_pass_through() {
        let mut stream = TextStream::default();
        // 스트리밍 없이 온 완성본(partial이 꺼진 턴)은 평소의 Text다.
        assert_eq!(
            feed(&mut stream, &[ASSISTANT]),
            vec![ConvoEvent::Text {
                text: "안녕하세요".into(),
                parent_id: None
            }]
        );
        // 서브 에이전트 델타는 스냅샷을 만들지 않고, 열린 메인 블록도 건드리지 않는다.
        let sub = DELTA_A.replace(r#""parent_tool_use_id":null"#, r#""parent_tool_use_id":"toolu_1""#);
        let events = feed(&mut stream, &[START, TEXT_START, &sub]);
        assert!(events.is_empty());
        let sub_text = r#"{"type":"assistant","message":{"id":"msg_2","content":[{"type":"text","text":"sub"}]},"parent_tool_use_id":"toolu_1"}"#;
        assert!(matches!(
            feed(&mut stream, &[sub_text]).as_slice(),
            [ConvoEvent::Text { parent_id: Some(_), .. }]
        ));
    }

    #[test]
    fn non_text_blocks_emit_nothing() {
        let mut stream = TextStream::default();
        let thinking = r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"thinking"}},"parent_tool_use_id":null}"#;
        let json_delta = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{"}},"parent_tool_use_id":null}"#;
        assert!(feed(&mut stream, &[START, thinking, json_delta]).is_empty());
    }
}
