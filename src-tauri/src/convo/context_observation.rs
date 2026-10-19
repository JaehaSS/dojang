use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use super::{model_observation, ConvoEvent};

const HEAD_BYTES: usize = 64 * 1024;
pub(super) const TAIL_BYTES: usize = 1024 * 1024;

pub(super) fn invalid_event(source: &str) -> ConvoEvent {
    ConvoEvent::ContextUsage {
        context_tokens: 0,
        context_window: None,
        observed_at: None,
        source: Some(source.into()),
        valid: Some(false),
    }
}

pub(super) fn codex_context_at_root(root: &Path, session_id: &str, started_at: i64) -> Option<ConvoEvent> {
    if !model_observation::safe_session_id(session_id) { return None; }
    let suffix = format!("{session_id}.jsonl");
    let path = model_observation::find_session_file(root, |name| name.ends_with(&suffix))?;
    codex_context_in(&path, session_id, started_at, chrono::Utc::now().timestamp_millis())
}

pub(super) fn codex_context_in(
    path: &Path,
    session_id: &str,
    started_at: i64,
    read_at: i64,
) -> Option<ConvoEvent> {
    let (head, tail) = bounded_lines(path)?;
    let identity_matches = head
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .any(|event| session_id_of(&event) == Some(session_id));
    let mut latest = None;
    for line in tail {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if is_token_count(&event) {
            latest = token_count(&event, started_at, read_at);
        } else if is_boundary(&event) {
            latest = None;
        }
    }
    identity_matches.then_some(latest).flatten()
}

fn bounded_lines(path: &Path) -> Option<(Vec<String>, Vec<String>)> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let head_len = len.min(HEAD_BYTES as u64) as usize;
    let mut head = vec![0; head_len];
    file.read_exact(&mut head).ok()?;
    let head = complete_lines(&head, false);
    let tail_start = len.saturating_sub(TAIL_BYTES as u64);
    file.seek(SeekFrom::Start(tail_start)).ok()?;
    let mut tail = Vec::new();
    (&mut file)
        .take(TAIL_BYTES as u64)
        .read_to_end(&mut tail)
        .ok()?;
    let starts_partial = tail_start > 0 && previous_byte(&mut file, tail_start) != Some(b'\n');
    Some((head, complete_lines(&tail, starts_partial)))
}

fn previous_byte(file: &mut std::fs::File, offset: u64) -> Option<u8> {
    file.seek(SeekFrom::Start(offset.checked_sub(1)?)).ok()?;
    let mut byte = [0];
    file.read_exact(&mut byte).ok()?;
    Some(byte[0])
}

fn complete_lines(bytes: &[u8], skip_first: bool) -> Vec<String> {
    let Some(end) = bytes.iter().rposition(|byte| *byte == b'\n') else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&bytes[..end]);
    let mut lines = text.lines();
    if skip_first {
        let _ = lines.next();
    }
    lines.map(str::to_string).collect()
}

fn session_id_of(event: &serde_json::Value) -> Option<&str> {
    event
        .get("type")
        .and_then(serde_json::Value::as_str)
        .filter(|t| *t == "session_meta")?;
    event.get("payload")?.get("id")?.as_str()
}

fn event_type(event: &serde_json::Value) -> Option<&str> {
    event
        .get("payload")
        .and_then(|p| p.get("type"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| event.get("type").and_then(serde_json::Value::as_str))
}

fn is_token_count(event: &serde_json::Value) -> bool {
    event.get("type").and_then(serde_json::Value::as_str) == Some("event_msg")
        && event_type(event) == Some("token_count")
}

fn is_boundary(event: &serde_json::Value) -> bool {
    matches!(event_type(event), Some("turn_context" | "compacted"))
}

fn token_count(event: &serde_json::Value, started_at: i64, read_at: i64) -> Option<ConvoEvent> {
    let timestamp = event.get("timestamp")?.as_str()?;
    let observed_at = chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()?
        .timestamp_millis();
    if observed_at < started_at || observed_at > read_at {
        return None;
    }
    let info = event.get("payload")?.get("info")?;
    let tokens = info
        .get("last_token_usage")?
        .get("total_tokens")?
        .as_i64()?;
    let window = info.get("model_context_window")?.as_i64()?;
    if tokens < 0 || window <= 0 {
        return None;
    }
    Some(ConvoEvent::ContextUsage {
        context_tokens: tokens,
        context_window: Some(window),
        observed_at: Some(observed_at / 1000),
        source: Some("codex_session".into()),
        valid: Some(true),
    })
}
