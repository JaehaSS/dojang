//! Read-only, bounded observation of root-correlated Codex child session logs.
use super::{model_observation, ConvoEvent, SubagentMetadata};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
#[path = "codex_subagent_discovery.rs"]
mod discovery;

const DISCOVERY_INTERVAL: Duration = Duration::from_millis(500);
const MAX_ENTRIES: usize = 20_000;
const MAX_CHILDREN: usize = 256;
const MAX_DEPTH: usize = 8;
const MAX_TICK_BYTES: usize = 256 * 1024;
const MAX_FILE_BYTES: usize = 32 * 1024;
const MAX_LINE_BYTES: usize = 64 * 1024;
const MAX_TEXT_CHARS: usize = 8_000;
const MAX_EMITTED: usize = 8192;

#[derive(Clone, Debug)]
struct Child {
    id: String,
    parent: String,
    path: PathBuf,
    nickname: Option<String>,
    title: Option<String>,
    role: Option<String>,
    model: Option<String>,
    turn_id: Option<String>,
    state: &'static str,
    source: &'static str,
    status: &'static str,
    active: bool,
    latest_timestamp: i64,
}
#[derive(Default)]
struct Cursor {
    offset: u64,
    partial: Vec<u8>,
    identity: Option<(u64, u64)>,
    boundary: Vec<u8>,
    dropping_line: bool,
}
/// The execution loop owns this reader; callbacks and persistence remain serial.
pub(super) struct Observer {
    root: Option<String>,
    sessions: Option<PathBuf>,
    started_at: i64,
    execution_id: String,
    next_discovery: Instant,
    next_read: Instant,
    discovery: Option<discovery::Discovery>,
    metas: BTreeMap<String, Meta>,
    children: BTreeMap<String, Child>,
    cursors: BTreeMap<PathBuf, Cursor>,
    emitted: BTreeSet<String>,
    read_index: usize,
}
impl Observer {
    pub(super) fn new(sessions: Option<PathBuf>, started_at: i64) -> Self {
        Self {
            root: None,
            sessions,
            started_at,
            execution_id: format!("codex-exec-{started_at}"),
            next_discovery: Instant::now(),
            next_read: Instant::now(),
            discovery: None,
            metas: BTreeMap::new(),
            children: BTreeMap::new(),
            cursors: BTreeMap::new(),
            emitted: BTreeSet::new(),
            read_index: 0,
        }
    }
    pub(super) fn set_root(&mut self, root: &str) {
        if !model_observation::safe_session_id(root) || self.root.as_deref() == Some(root) {
            return;
        }
        // A changed thread cannot inherit another root's correlation or offsets.
        self.root = Some(root.into());
        self.children.clear();
        self.cursors.clear();
        self.metas.clear();
        self.discovery = self.sessions.clone().map(discovery::Discovery::new);
    }
    pub(super) fn tick(&mut self) -> Vec<ConvoEvent> {
        let mut events = Vec::new();
        if self.root.is_none() {
            return events;
        }
        if Instant::now() >= self.next_discovery {
            self.next_discovery = Instant::now() + DISCOVERY_INTERVAL;
            self.discover_step(false, &mut events);
        }
        if Instant::now() >= self.next_read {
            self.next_read = Instant::now() + Duration::from_millis(100);
            self.read_round(&mut events);
        }
        events
    }
    pub(super) fn observe_stream_line(&mut self, line: &str) -> Vec<ConvoEvent> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        let Some(item) = value.get("item") else {
            return Vec::new();
        };
        if item["type"] != "collab_tool_call" {
            return Vec::new();
        }
        let Some(root) = self.root.clone() else {
            return Vec::new();
        };
        let parent = item
            .get("sender_thread_id")
            .or_else(|| item.get("parent_thread_id"))
            .and_then(Value::as_str);
        // Only a spawn can establish a previously unknown receiver; anonymous wait cannot.
        let spawn = item["tool"] == "spawn_agent";
        if parent != Some(root.as_str()) {
            return Vec::new();
        }
        let mut events = Vec::new();
        for id in item
            .get("receiver_thread_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|id| model_observation::safe_session_id(id))
            .take(MAX_CHILDREN)
        {
            if id == root {
                continue;
            }
            if !self.children.contains_key(id) {
                if !spawn || self.children.len() >= MAX_CHILDREN {
                    continue;
                }
                self.children.insert(
                    id.into(),
                    Child {
                        id: id.into(),
                        parent: root.clone(),
                        path: PathBuf::new(),
                        nickname: None,
                        title: None,
                        role: None,
                        model: None,
                        turn_id: None,
                        state: "unknown",
                        source: "stream",
                        status: "missing",
                        active: true,
                        latest_timestamp: 0,
                    },
                );
            }
            let child = self.children.get_mut(id).unwrap();
            // A log-backed child owns its turn-aware lifecycle. A receipt cannot overwrite it.
            if child.path.as_os_str().is_empty() {
                if let Some(state) = item
                    .get("agents_states")
                    .and_then(|states| states.get(id))
                    .and_then(stream_status)
                    .and_then(stream_state)
                {
                    child.state = state;
                }
                let child = child.clone();
                events.push(self.update(&child));
            }
        }
        events
    }
    pub(super) fn finish(&mut self) -> Vec<ConvoEvent> {
        let mut events = Vec::new();
        // Bounded final drain; no wait for child completion or recursive synchronous full scan.
        for _ in 0..4 {
            self.discover_step(true, &mut events);
            self.read_round(&mut events);
        }
        let incomplete: Vec<_> = self
            .children
            .values()
            .filter(|child| child.active && !child.path.as_os_str().is_empty())
            .filter(|child| {
                fs::metadata(&child.path).ok().is_some_and(|metadata| {
                    self.cursors.get(&child.path).is_none_or(|cursor| {
                        cursor.offset < metadata.len() || !cursor.partial.is_empty()
                    })
                })
            })
            .map(|child| child.path.clone())
            .collect();
        for path in incomplete {
            self.mark_status(&path, "limited", &mut events);
        }
        let mut changed = Vec::new();
        for child in self.children.values_mut() {
            if child.active && child.state == "running" {
                child.state = "unknown";
                changed.push(child.clone());
            }
        }
        for child in changed {
            events.push(self.update(&child));
        }
        events
    }
    fn discover_step(&mut self, force: bool, events: &mut Vec<ConvoEvent>) {
        let (found, limited) = {
            let Some(discovery) = self.discovery.as_mut() else {
                return;
            };
            (discovery.step(force), discovery.limited)
        };
        for meta in found {
            if self.children.values().any(|child| {
                child.path == meta.path
                    && (child.id != meta.id
                        || Some(child.parent.as_str()) != meta.parent.as_deref())
            }) {
                self.invalidate_path(&meta.path, events);
            }
            if self.metas.len() >= MAX_ENTRIES && !self.metas.contains_key(&meta.id) {
                // Keep correlated ancestors while allowing a new active child past old unrelated headers.
                let evict = self
                    .metas
                    .keys()
                    .find(|id| !self.children.contains_key(*id))
                    .cloned();
                if let Some(id) = evict {
                    self.metas.remove(&id);
                }
            }
            if self.metas.len() < MAX_ENTRIES || self.metas.contains_key(&meta.id) {
                self.metas.insert(meta.id.clone(), meta);
            }
        }
        let Some(root) = self.root.clone() else {
            return;
        };
        let mut by_parent: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for meta in self.metas.values() {
            if let Some(parent) = &meta.parent {
                by_parent
                    .entry(parent.clone())
                    .or_default()
                    .push(meta.id.clone());
            }
        }
        let mut pending = vec![(root.clone(), 0usize)];
        let mut accepted = BTreeSet::from([root]);
        while let Some((parent, depth)) = pending.pop() {
            if depth >= MAX_DEPTH {
                continue;
            }
            for id in by_parent.get(&parent).into_iter().flatten() {
                if self.children.len() >= MAX_CHILDREN && !self.children.contains_key(id) {
                    break;
                }
                if !accepted.insert(id.clone()) {
                    continue;
                }
                pending.push((id.clone(), depth + 1));
                let meta = &self.metas[id];
                let status = if limited { "limited" } else { "available" };
                if let Some(existing) = self.children.get_mut(id) {
                    let changed = existing.path != meta.path
                        || existing.nickname != meta.nickname
                        || existing.title != meta.title
                        || existing.role != meta.role;
                    existing.parent = parent.clone();
                    existing.path = meta.path.clone();
                    existing.nickname = meta.nickname.clone();
                    existing.title = meta.title.clone();
                    existing.role = meta.role.clone();
                    existing.source = "session_log";
                    if existing.status == "missing" {
                        existing.status = status;
                    }
                    if changed && existing.active {
                        let child = existing.clone();
                        events.push(self.update(&child));
                    }
                } else {
                    let child = Child {
                        id: id.clone(),
                        parent: parent.clone(),
                        path: meta.path.clone(),
                        nickname: meta.nickname.clone(),
                        title: meta.title.clone(),
                        role: meta.role.clone(),
                        model: None,
                        turn_id: None,
                        state: "unknown",
                        source: "session_log",
                        status,
                        active: meta.observed_at >= self.started_at,
                        latest_timestamp: 0,
                    };
                    if child.active {
                        events.push(self.update(&child));
                    }
                    self.children.insert(id.clone(), child);
                }
                if let Some(dir) = meta.path.parent() {
                    self.discovery.as_mut().unwrap().watch(dir.to_path_buf());
                }
            }
        }
        if limited {
            let paths: Vec<_> = self
                .children
                .values()
                .filter(|child| child.active)
                .map(|child| child.path.clone())
                .collect();
            for path in paths {
                self.mark_status(&path, "limited", events);
            }
        }
    }
    fn read_round(&mut self, events: &mut Vec<ConvoEvent>) {
        let paths: Vec<_> = self
            .children
            .values()
            .filter(|child| !child.path.as_os_str().is_empty())
            .map(|child| child.path.clone())
            .collect();
        if paths.is_empty() {
            return;
        }
        let mut budget = MAX_TICK_BYTES;
        let start = self.read_index % paths.len();
        let mut read = 0;
        for n in 0..paths.len() {
            if budget == 0 {
                break;
            }
            self.read_append(&paths[(start + n) % paths.len()], &mut budget, events);
            read += 1;
        }
        self.read_index = (start + read) % paths.len();
    }
    fn read_append(&mut self, path: &Path, budget: &mut usize, events: &mut Vec<ConvoEvent>) {
        let Ok(mut file) = fs::File::open(path) else {
            self.mark_status(path, "unreadable", events);
            return;
        };
        let Ok(metadata) = file.metadata() else {
            self.mark_status(path, "unreadable", events);
            return;
        };
        let len = metadata.len();
        let identity = file_identity(&metadata);
        let historical = self
            .children
            .values()
            .find(|child| child.path == path)
            .is_some_and(|child| !child.active);
        let rewritten = if let Some(cursor) = self.cursors.get(path) {
            let identity_changed =
                len < cursor.offset || (cursor.identity.is_some() && cursor.identity != identity);
            let boundary_changed = if !cursor.boundary.is_empty() {
                let mut previous = vec![0; cursor.boundary.len()];
                file.seek(SeekFrom::Start(
                    cursor.offset.saturating_sub(previous.len() as u64),
                ))
                .is_err()
                    || file.read_exact(&mut previous).is_err()
                    || previous != cursor.boundary
            } else {
                false
            };
            identity_changed || boundary_changed
        } else {
            false
        };
        if rewritten || !self.cursors.contains_key(path) {
            if file.seek(SeekFrom::Start(0)).is_err() {
                self.mark_status(path, "unreadable", events);
                return;
            }
            // Verify the header on this same file descriptor: the pathname may have been
            // replaced since discovery. Account for proof bytes in the shared read budget.
            let (meta, complete, bytes) =
                read_session_meta(&mut file, (*budget).min(MAX_LINE_BYTES));
            *budget = budget.saturating_sub(bytes);
            let related = meta.as_ref().is_some_and(|meta| {
                self.children.values().any(|child| {
                    child.path == path
                        && child.id == meta.id
                        && Some(child.parent.as_str()) == meta.parent.as_deref()
                })
            });
            if !related {
                if complete {
                    self.invalidate_path(path, events);
                } else {
                    self.mark_status(path, "limited", events);
                }
                return;
            }
            self.cursors.remove(path);
        }
        let (lines, discarded, skipped_history) = {
            let fresh = !self.cursors.contains_key(path);
            let cursor = self.cursors.entry(path.to_path_buf()).or_default();
            let skipped_history = fresh && historical && len > MAX_LINE_BYTES as u64;
            if skipped_history {
                cursor.offset = len.saturating_sub(MAX_LINE_BYTES as u64);
                cursor.dropping_line = true;
            }
            cursor.identity = identity;
            if file.seek(SeekFrom::Start(cursor.offset)).is_err() {
                return;
            }
            let read_limit = len
                .saturating_sub(cursor.offset)
                .min((*budget).min(MAX_FILE_BYTES) as u64);
            let mut bytes = Vec::new();
            if file.take(read_limit).read_to_end(&mut bytes).is_err() {
                return;
            }
            cursor.offset += bytes.len() as u64;
            *budget = budget.saturating_sub(bytes.len());
            if !bytes.is_empty() {
                cursor.boundary.extend_from_slice(&bytes);
                if cursor.boundary.len() > 64 {
                    cursor.boundary.drain(..cursor.boundary.len() - 64);
                }
            }
            let mut lines = Vec::new();
            let mut discarded = false;
            // Split slices once, and discard a rejected long record through its newline.
            for part in bytes.split_inclusive(|byte| *byte == b'\n') {
                let complete = part.last() == Some(&b'\n');
                if !cursor.dropping_line {
                    if cursor.partial.len() + part.len() > MAX_LINE_BYTES {
                        cursor.partial.clear();
                        cursor.dropping_line = true;
                        discarded = true;
                    } else {
                        cursor.partial.extend_from_slice(part);
                    }
                }
                if complete {
                    if !cursor.dropping_line {
                        if let Ok(line) = String::from_utf8(std::mem::take(&mut cursor.partial)) {
                            lines.push(line);
                        } else {
                            discarded = true;
                        }
                    }
                    cursor.dropping_line = false;
                }
            }
            (lines, discarded, skipped_history)
        };
        if discarded || skipped_history {
            self.mark_status(path, "limited", events);
        }
        for line in lines {
            self.observe_line(path, line.trim(), events);
        }
    }
    fn invalidate_path(&mut self, path: &Path, events: &mut Vec<ConvoEvent>) {
        let invalid: Vec<_> = self
            .children
            .values()
            .filter(|child| child.path == path)
            .map(|child| child.id.clone())
            .collect();
        for id in invalid {
            self.metas.remove(&id);
            let child = self.children.get_mut(&id).unwrap();
            child.path.clear();
            child.status = "unsupported";
            if child.state == "running" {
                child.state = "unknown";
            }
            let child = child.clone();
            if child.active {
                events.push(self.update(&child));
            }
        }
        self.cursors.remove(path);
    }
    fn mark_status(&mut self, path: &Path, status: &'static str, events: &mut Vec<ConvoEvent>) {
        if let Some(child) = self.children.values_mut().find(|child| child.path == path) {
            // Missing data remains visible after later lifecycle records and parent shutdown.
            let status = if child.status == "limited" {
                "limited"
            } else {
                status
            };
            if child.status != status {
                child.status = status;
                let child = child.clone();
                if child.active {
                    events.push(self.update(&child));
                }
            }
        }
    }
    fn observe_line(&mut self, path: &Path, line: &str, events: &mut Vec<ConvoEvent>) {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            self.mark_status(path, "unsupported", events);
            return;
        };
        let Some(timestamp) = event
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_timestamp)
        else {
            self.mark_status(path, "unsupported", events);
            return;
        };
        if timestamp < self.started_at {
            return;
        }
        let payload = event.get("payload").unwrap_or(&Value::Null);
        let Some(child_id) = self
            .children
            .values()
            .find(|child| child.path == path)
            .map(|child| child.id.clone())
        else {
            return;
        };
        let Some(mut child) = self.children.remove(&child_id) else {
            return;
        };
        let previous_status = child.status;
        child.active = true;
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        let nested_kind = payload.get("type").and_then(Value::as_str).unwrap_or(kind);
        let turn = payload
            .get("turn_id")
            .and_then(Value::as_str)
            .filter(|turn| !turn.is_empty() && turn.len() <= 240);
        let older_turn = timestamp < child.latest_timestamp;
        let distinct_terminal = turn.is_some()
            && child.turn_id.is_some()
            && turn != child.turn_id.as_deref()
            && !matches!(nested_kind, "task_started")
            && kind != "turn_context";
        let lifecycle_current = !older_turn && !distinct_terminal;
        if lifecycle_current {
            if let Some(turn) = turn {
                child.turn_id = Some(cap_title(turn));
            }
            child.latest_timestamp = child.latest_timestamp.max(timestamp);
        }
        let result_turn = turn
            .or(child.turn_id.as_deref())
            .map(str::to_string)
            .unwrap_or_else(|| timestamp.to_string());
        let key = format!("{}:{result_turn}:{nested_kind}", child.id);
        let parent_id = Some(card_key(self.root.as_deref().unwrap_or(""), &child.id));
        match (kind, nested_kind) {
            ("session_meta", _) => {}
            ("turn_context", _) if lifecycle_current => {
                child.model = payload
                    .get("model")
                    .and_then(Value::as_str)
                    .and_then(model_observation::normalize_model);
                events.push(self.update(&child));
            }
            ("event_msg", "task_started") if lifecycle_current => {
                child.state = "running";
                events.push(self.update(&child));
            }
            ("event_msg", "task_complete") => {
                let explicit_error = payload.get("is_error").and_then(Value::as_bool) == Some(true)
                    || payload
                        .get("error")
                        .is_some_and(|v| !v.is_null() && *v != Value::Bool(false));
                let completion = if explicit_error {
                    "failed"
                } else if let Some(status) = payload.get("status").filter(|value| !value.is_null())
                {
                    stream_status(status)
                        .and_then(stream_state)
                        .unwrap_or_else(|| {
                            if child.status != "limited" {
                                child.status = "unsupported";
                            }
                            "unknown"
                        })
                } else {
                    "done"
                };
                if lifecycle_current {
                    if child.state != "failed" && child.state != "stopped" {
                        child.state = completion;
                    }
                    events.push(self.update(&child));
                }
                if let Some(text) = payload.get("last_agent_message").and_then(Value::as_str) {
                    let source_key = format!("{}:{result_turn}:result", child.id);
                    if !text.trim().is_empty() && self.accept_key(source_key.clone(), &mut child) {
                        events.push(ConvoEvent::SubagentMessage {
                            parent_id: parent_id.clone().unwrap(),
                            source_key,
                            child_turn_id: turn
                                .map(str::to_string)
                                .or_else(|| child.turn_id.clone()),
                            text: cap_public_text(text, MAX_TEXT_CHARS, &mut child),
                            is_error: completion == "failed"
                                || (lifecycle_current && child.state == "failed"),
                        });
                    }
                }
            }
            ("event_msg", "error" | "task_failed" | "turn_failed") => {
                if lifecycle_current {
                    child.state = "failed";
                    events.push(self.update(&child));
                }
                if let Some(text) = payload.get("message").and_then(Value::as_str) {
                    let source_key = format!("{key}:{timestamp}:error");
                    if self.accept_key(source_key.clone(), &mut child) {
                        events.push(ConvoEvent::SubagentMessage {
                            parent_id: parent_id.clone().unwrap(),
                            source_key,
                            child_turn_id: turn
                                .map(str::to_string)
                                .or_else(|| child.turn_id.clone()),
                            text: cap_public_text(text, MAX_TEXT_CHARS, &mut child),
                            is_error: true,
                        });
                    }
                }
            }
            ("event_msg", "turn_aborted" | "task_stopped" | "task_cancelled")
                if lifecycle_current =>
            {
                child.state = "stopped";
                events.push(self.update(&child));
            }
            ("event_msg", "exec_command_begin" | "patch_apply_begin") => {
                if let Some(call) = payload
                    .get("call_id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty() && id.len() <= 240)
                {
                    let id = format!("codex-tool:{}:{call}", child.id);
                    if self.accept_key(format!("{id}:begin"), &mut child) {
                        let (name, summary) = if nested_kind == "exec_command_begin" {
                            ("Bash", public_command(payload.get("command")))
                        } else {
                            ("ApplyPatch", public_paths(payload.get("changes")))
                        };
                        events.push(ConvoEvent::ToolUse {
                            name: name.into(),
                            summary: cap_public_text(&summary, 1000, &mut child),
                            tool_id: Some(id),
                            parent_id: parent_id.clone(),
                        });
                    }
                }
            }
            ("event_msg", "exec_command_end" | "patch_apply_end") => {
                if let Some(call) = payload
                    .get("call_id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty() && id.len() <= 240)
                {
                    let id = format!("codex-tool:{}:{call}", child.id);
                    if self.accept_key(format!("{id}:end"), &mut child) {
                        let output = payload
                            .get("aggregated_output")
                            .or_else(|| payload.get("stdout"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let is_error = payload
                            .get("exit_code")
                            .and_then(Value::as_i64)
                            .is_some_and(|code| code != 0)
                            || payload.get("success").and_then(Value::as_bool) == Some(false);
                        events.push(ConvoEvent::ToolResult {
                            summary: cap_public_text(output, MAX_TEXT_CHARS, &mut child),
                            is_error,
                            result_chars: Some(output.chars().count() as i64),
                            tool_use_id: Some(id),
                            parent_id: parent_id.clone(),
                        });
                    }
                }
            }
            ("turn_context", _)
            | ("event_msg", "task_started" | "turn_aborted" | "task_stopped" | "task_cancelled") => {
            }
            // These contain no lifecycle evidence; private response items are deliberately ignored.
            ("response_item", _)
            | (
                "event_msg",
                "agent_message" | "agent_reasoning" | "token_count" | "user_message"
                | "item_started" | "item_completed" | "context_compacted",
            )
            | (
                "compacted"
                | "world_state"
                | "inter_agent_communication_metadata"
                | "token_usage_record",
                _,
            ) => {}
            _ => {
                if child.status != "limited" {
                    child.status = "unsupported";
                }
                events.push(self.update(&child));
            }
        }
        if child.status != previous_status && child.status == "limited" {
            events.push(self.update(&child));
        }
        self.children.insert(child_id, child);
    }
    fn accept_key(&mut self, key: String, child: &mut Child) -> bool {
        if self.emitted.contains(&key) {
            return false;
        }
        if self.emitted.len() >= MAX_EMITTED {
            child.status = "limited";
            return false;
        }
        self.emitted.insert(key);
        true
    }
    fn update(&self, child: &Child) -> ConvoEvent {
        let root = self.root.as_deref().unwrap_or("");
        ConvoEvent::SubagentUpdate {
            metadata: Box::new(SubagentMetadata {
                vendor: "codex".into(),
                root_thread_id: root.into(),
                parent_thread_id: child.parent.clone(),
                child_thread_id: child.id.clone(),
                execution_id: self.execution_id.clone(),
                child_turn_id: child.turn_id.clone(),
                source_key: format!("session:{}", child.id),
                title: child.title.clone(),
                nickname: child.nickname.clone(),
                agent_role: child.role.clone(),
                model: child.model.clone(),
                state: child.state.into(),
                observation_source: child.source.into(),
                observation_status: child.status.into(),
                parent_id: (child.parent != root).then(|| card_key(root, &child.parent)),
            }),
        }
    }
}
#[derive(Clone)]
struct Meta {
    id: String,
    parent: Option<String>,
    nickname: Option<String>,
    title: Option<String>,
    role: Option<String>,
    path: PathBuf,
    observed_at: i64,
}
impl Meta {
    fn with_path(mut self, path: PathBuf) -> Self {
        self.path = path;
        self
    }
}
fn session_meta(path: &Path, budget: usize) -> (Option<Meta>, bool, usize) {
    let Ok(file) = fs::File::open(path) else {
        return (None, false, 0);
    };
    read_session_meta(file, budget)
}
fn read_session_meta(reader: impl Read, budget: usize) -> (Option<Meta>, bool, usize) {
    let mut bytes = Vec::new();
    if BufReader::with_capacity(1024, reader)
        .take(budget.min(MAX_LINE_BYTES) as u64)
        .read_until(b'\n', &mut bytes)
        .is_err()
    {
        return (None, false, bytes.len());
    }
    let count = bytes.len();
    let Some(end) = bytes.iter().position(|b| *b == b'\n') else {
        return (None, count >= MAX_LINE_BYTES, count);
    };
    let Ok(event) = serde_json::from_slice::<Value>(&bytes[..end]) else {
        return (None, true, count);
    };
    if event["type"] != "session_meta" {
        return (None, true, count);
    }
    let payload = &event["payload"];
    let Some(id) = payload
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| model_observation::safe_session_id(id))
    else {
        return (None, true, count);
    };
    let spawned = payload.pointer("/source/subagent/thread_spawn");
    let parent = spawned
        .and_then(|v| v.get("parent_thread_id"))
        .or_else(|| payload.get("parent_thread_id"))
        .and_then(Value::as_str)
        .filter(|id| model_observation::safe_session_id(id));
    let Some(parent) = parent else {
        return (None, true, count);
    };
    let meta = Meta {
        id: id.into(),
        parent: Some(parent.into()),
        nickname: spawned
            .and_then(|v| v.get("agent_nickname"))
            .or_else(|| payload.get("agent_nickname"))
            .and_then(Value::as_str)
            .map(cap_title),
        title: spawned
            .and_then(|v| v.get("agent_path"))
            .or_else(|| payload.get("agent_path"))
            .and_then(Value::as_str)
            .map(cap_title),
        role: spawned
            .and_then(|v| v.get("agent_role"))
            .and_then(Value::as_str)
            .map(cap_title),
        path: PathBuf::new(),
        observed_at: event
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_timestamp)
            .unwrap_or(0),
    };
    (Some(meta), true, count)
}
/// Match the actual Command's overrides and inherited environment, including relative homes.
pub(super) fn sessions_root_from_command(
    command: &std::process::Command,
    cwd: &str,
) -> Option<PathBuf> {
    let effective = |name: &str| {
        command
            .get_envs()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.map(std::ffi::OsStr::to_os_string))
            .unwrap_or_else(|| std::env::var_os(name))
    };
    let base = effective("CODEX_HOME").map(PathBuf::from).or_else(|| {
        effective("HOME")
            .or_else(|| effective("USERPROFILE"))
            .map(|home| PathBuf::from(home).join(".codex"))
    })?;
    Some(
        if base.is_absolute() {
            base
        } else {
            Path::new(cwd).join(base)
        }
        .join("sessions"),
    )
}
fn card_key(root: &str, child: &str) -> String {
    format!("codex:{root}:{child}")
}
fn parse_timestamp(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|t| t.timestamp_millis())
}
fn cap(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}
fn cap_public_text(value: &str, max: usize, child: &mut Child) -> String {
    let mut chars = value.chars();
    let text: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        child.status = "limited";
    }
    text
}
fn cap_title(value: &str) -> String {
    cap(value, 240)
}
fn public_command(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}
fn public_paths(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_object)
        .map(|m| m.keys().take(32).cloned().collect::<Vec<_>>().join(", "))
        .unwrap_or_default()
}
fn stream_state(value: &str) -> Option<&'static str> {
    match value {
        "running" | "in_progress" => Some("running"),
        "completed" | "done" => Some("done"),
        "failed" | "error" => Some("failed"),
        "stopped" | "cancelled" | "aborted" => Some("stopped"),
        _ => None,
    }
}
fn stream_status(value: &Value) -> Option<&str> {
    value
        .as_str()
        .or_else(|| value.get("status").and_then(Value::as_str))
}
#[cfg(unix)]
fn file_identity(metadata: &fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((metadata.dev(), metadata.ino()))
}
#[cfg(not(unix))]
fn file_identity(_metadata: &fs::Metadata) -> Option<(u64, u64)> {
    None
}
#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "10000000-0000-0000-0000-000000000001";
    const CHILD: &str = "20000000-0000-0000-0000-000000000002";

    fn temporary_directory() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "praxis-codex-observer-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn session_root_honors_command_overrides_relative_paths_and_removed_environment() {
        let mut command = std::process::Command::new("codex");
        command.env("CODEX_HOME", "relative-codex");
        assert_eq!(
            sessions_root_from_command(&command, "/workspace"),
            Some(PathBuf::from("/workspace/relative-codex/sessions"))
        );
        command
            .env_remove("CODEX_HOME")
            .env("HOME", "/fixture-home");
        assert_eq!(
            sessions_root_from_command(&command, "/workspace"),
            Some(PathBuf::from("/fixture-home/.codex/sessions"))
        );
    }

    #[test]
    fn completion_error_fields_are_not_inferred_as_success() {
        let base = temporary_directory();
        fs::create_dir_all(&base).unwrap();
        let path = base.join(format!("rollout-{CHILD}.jsonl"));
        fs::write(&path,format!("{{\"timestamp\":\"2026-10-07T00:00:01Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{CHILD}\",\"parent_thread_id\":\"{ROOT}\"}}}}\n")).unwrap();
        let mut observer = Observer::new(Some(base.clone()), 0);
        observer.set_root(ROOT);
        observer.finish();
        for payload in [
            serde_json::json!({"type":"task_complete","status":"failed"}),
            serde_json::json!({"type":"task_complete","is_error":true}),
            serde_json::json!({"type":"task_complete","status":"completed","is_error":true}),
            serde_json::json!({"type":"task_complete","status":{"status":"failed"}}),
            serde_json::json!({"type":"task_complete","error":{"message":"failed"}}),
        ] {
            let mut events = Vec::new();
            observer.children.get_mut(CHILD).unwrap().state = "running";
            observer.observe_line(&path,&serde_json::json!({"timestamp":"2026-10-07T00:00:02Z","type":"event_msg","payload":payload}).to_string(),&mut events);
            assert_eq!(observer.children[CHILD].state, "failed");
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn replaced_file_must_reprove_its_child_identity_and_parent() {
        let base = temporary_directory();
        fs::create_dir_all(&base).unwrap();
        let path = base.join(format!("rollout-{CHILD}.jsonl"));
        let record = |id: &str, parent: &str, turn: &str, text: &str| {
            [serde_json::json!({"timestamp":"2026-10-07T00:00:01Z","type":"session_meta","payload":{"id":id,"parent_thread_id":parent}}),
             serde_json::json!({"timestamp":"2026-10-07T00:00:02Z","type":"event_msg","payload":{"type":"task_complete","turn_id":turn,"last_agent_message":text}})]
                .iter().map(|value|value.to_string()+"\n").collect::<String>()
        };
        fs::write(&path, record(CHILD, ROOT, "first", "RELATED")).unwrap();
        let mut observer = Observer::new(Some(base.clone()), 0);
        observer.set_root(ROOT);
        observer.finish();
        let replacement = base.join("replacement");
        fs::write(
            &replacement,
            record(
                "30000000-0000-0000-0000-000000000003",
                "40000000-0000-0000-0000-000000000004",
                "foreign",
                "FOREIGN",
            ),
        )
        .unwrap();
        fs::rename(replacement, &path).unwrap();
        let events = observer.finish();
        assert!(!events
            .iter()
            .any(|event| matches!(event,ConvoEvent::SubagentMessage{text,..} if text=="FOREIGN")));
        assert_eq!(observer.children[CHILD].status, "unsupported");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn truncated_public_result_keeps_completion_and_reports_incomplete_details() {
        let base = temporary_directory();
        fs::create_dir_all(&base).unwrap();
        let path = base.join(format!("rollout-{CHILD}.jsonl"));
        let rows = [
            serde_json::json!({"timestamp":"2026-10-07T00:00:01Z","type":"session_meta","payload":{"id":CHILD,"parent_thread_id":ROOT}}),
            serde_json::json!({"timestamp":"2026-10-07T00:00:02Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"large-public","last_agent_message":"가".repeat(MAX_TEXT_CHARS+1)}}),
        ];
        fs::write(
            &path,
            rows.iter()
                .map(|row| row.to_string() + "\n")
                .collect::<String>(),
        )
        .unwrap();
        let mut observer = Observer::new(Some(base.clone()), 0);
        observer.set_root(ROOT);
        let events = observer.finish();
        assert!(events.iter().any(|event|matches!(event,ConvoEvent::SubagentMessage{text,..} if text.chars().count()==MAX_TEXT_CHARS)));
        assert_eq!(observer.children[CHILD].state, "done");
        assert_eq!(observer.children[CHILD].status, "limited");
        assert!(events.iter().any(|event|matches!(event,ConvoEvent::SubagentUpdate{metadata} if metadata.observation_status=="limited")));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn only_related_child_emits_public_completion_once() {
        let base = temporary_directory();
        let sessions = base.join("sessions/2026/10/07");
        fs::create_dir_all(&sessions).unwrap();
        let child = sessions.join(format!("rollout-{CHILD}.jsonl"));
        fs::write(&child, format!(
            "{{\"timestamp\":\"2026-10-07T00:00:01Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{CHILD}\",\"source\":{{\"subagent\":{{\"thread_spawn\":{{\"parent_thread_id\":\"{ROOT}\",\"agent_nickname\":\"fixture\"}}}}}}}}}}\n{{\"timestamp\":\"2026-10-07T00:00:02Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_complete\",\"turn_id\":\"turn\",\"last_agent_message\":\"ok\"}}}}\n" )).unwrap();
        let mut observer = Observer::new(Some(base.join("sessions")), 0);
        observer.set_root(ROOT);
        let events = observer.finish();
        assert!(events.iter().any(|event| matches!(event, ConvoEvent::SubagentUpdate { metadata } if metadata.child_thread_id == CHILD && metadata.state == "done")));
        assert_eq!(events.iter().filter(|event| matches!(event, ConvoEvent::SubagentMessage { text, .. } if text == "ok")).count(), 1);
        let _ = fs::remove_dir_all(base);
    }
}
