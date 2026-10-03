//! App-owned prompt metadata. Bytes are not tokens or evidence of a provider cache miss.
use super::{turn_guard, Vendor};
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;

/// Shared by the actual runtime and the admission measurement, avoiding a second template.
pub(crate) fn runtime_instructions(vendor: Vendor, questions: bool) -> String {
    match (vendor, questions) {
        (Vendor::Codex, true) => format!(
            "{}\n\n{}",
            turn_guard::TURN_COMPLETION_GUARD,
            super::app_server::TOOL_INSTRUCTIONS
        ),
        (Vendor::Claude, true) => format!(
            "{}\n\n{}",
            turn_guard::TURN_COMPLETION_GUARD,
            super::question_local::TOOL_INSTRUCTIONS
        ),
        (Vendor::Claude, false) => turn_guard::completion_system_prompt(vendor)
            .unwrap_or_default()
            .into(),
        (Vendor::Codex, false) => turn_guard::guarded_message(vendor, "").into_owned(),
        (Vendor::Agy, _) => String::new(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptMetrics {
    pub version: u8,
    pub runtime: String,
    pub resumed: bool,
    pub user_bytes: u64,
    pub sent_user_bytes: u64,
    pub instruction_bytes: u64,
    pub instruction_sha256: String,
    pub skill_added_bytes: u64,
    pub goal_added_bytes: u64,
    pub capsule_added_bytes: u64,
    pub vault_added_bytes: u64,
}

impl PromptMetrics {
    pub(crate) fn set_instructions(&mut self, instructions: &str) {
        self.instruction_bytes = instructions.len() as u64;
        self.instruction_sha256 = format!("{:x}", Sha256::digest(instructions.as_bytes()));
    }

    pub(crate) fn new(vendor: Vendor, questions: bool, resumed: bool, user: &str) -> Self {
        let instructions = runtime_instructions(vendor, questions);
        Self {
            version: 1,
            runtime: match (vendor, questions) {
                (Vendor::Codex, true) => "codex_app_server",
                (Vendor::Codex, false) => "codex_exec",
                (Vendor::Claude, true) => "claude_questions",
                (Vendor::Claude, false) => "claude_print",
                (Vendor::Agy, _) => "agy_print",
            }
            .into(),
            resumed,
            user_bytes: user.len() as u64,
            sent_user_bytes: user.len() as u64,
            instruction_bytes: instructions.len() as u64,
            instruction_sha256: format!("{:x}", Sha256::digest(instructions.as_bytes())),
            skill_added_bytes: 0,
            goal_added_bytes: 0,
            capsule_added_bytes: 0,
            vault_added_bytes: 0,
        }
    }

    pub(crate) fn finish(&mut self, message: &str) {
        self.sent_user_bytes = message.len() as u64;
        // Generic Codex has no safe additive developer-instructions setting: preserve
        // its user-prefix contract rather than overwrite the user's configuration.
        if self.runtime == "codex_exec" {
            self.sent_user_bytes += self.instruction_bytes;
        }
    }
}

pub(crate) fn added_bytes(before: &str, after: &str) -> u64 {
    after.len().saturating_sub(before.len()) as u64
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PromptInjectionSummary {
    pub accepted_turns: u64,
    pub observed_turns: u64,
    pub unknown_turns: u64,
    pub user_bytes: u64,
    pub sent_user_bytes: u64,
    pub instruction_bytes: u64,
    pub skill_added_bytes: u64,
    pub goal_added_bytes: u64,
    pub capsule_added_bytes: u64,
    pub vault_added_bytes: u64,
    pub instruction_changes: u64,
}

/// Includes older rows only as a baseline for fixed-instruction changes, never as
/// current-range byte totals. Existing/rewound events are not backfilled or rewritten.
pub async fn summarize(
    pool: &SqlitePool,
    range: &str,
    now: i64,
) -> Result<PromptInjectionSummary, sqlx::Error> {
    let cutoff = match range {
        "7d" => now.saturating_sub(7 * 86400),
        "30d" => now.saturating_sub(30 * 86400),
        _ => 0,
    };
    let mut rows = sqlx::query("SELECT task_id, ts, CAST(json_extract(event, '$.prompt_metrics') AS TEXT) AS metrics FROM convo_events WHERE rewound_at IS NULL AND json_valid(event) AND json_extract(event, '$.kind')='user' AND ts<=? ORDER BY ts,id")
        .bind(now).fetch(pool);
    let mut summary = PromptInjectionSummary::default();
    let mut previous = HashMap::new();
    while let Some(row) = rows.try_next().await? {
        let task: i64 = row.try_get("task_id")?;
        let ts: i64 = row.try_get("ts")?;
        let encoded: Option<String> = row.try_get("metrics")?;
        let metrics = encoded
            .as_deref()
            .and_then(|s| serde_json::from_str::<PromptMetrics>(s).ok())
            .filter(|m| {
                m.version == 1
                    && m.instruction_sha256.len() == 64
                    && m.instruction_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            });
        let changed = metrics.as_ref().is_some_and(|m| {
            previous
                .insert(task, m.instruction_sha256.clone())
                .is_some_and(|old| old != m.instruction_sha256)
        });
        if ts < cutoff {
            continue;
        }
        summary.accepted_turns += 1;
        let Some(m) = metrics else {
            summary.unknown_turns += 1;
            continue;
        };
        summary.observed_turns += 1;
        summary.instruction_changes += u64::from(changed);
        macro_rules! add { ($($field:ident),+) => { $(summary.$field = summary.$field.saturating_add(m.$field);)+ }; }
        add!(
            user_bytes,
            sent_user_bytes,
            instruction_bytes,
            skill_added_bytes,
            goal_added_bytes,
            capsule_added_bytes,
            vault_added_bytes
        );
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_counts_utf8_bytes_and_hashes_only_fixed_instructions() {
        let mut a = PromptMetrics::new(Vendor::Claude, true, false, "한글");
        let b = PromptMetrics::new(Vendor::Claude, true, true, "different private request");
        assert_eq!(a.user_bytes, 6);
        assert_eq!(a.instruction_sha256, b.instruction_sha256);
        assert!(a.instruction_bytes > 0);
        a.finish("한글 reference");
        assert_eq!(a.sent_user_bytes, "한글 reference".len() as u64);
        assert_eq!(added_bytes("long original", "short"), 0);
        assert!(!serde_json::to_string(&a).unwrap().contains("한글"));
        let mut codex = PromptMetrics::new(Vendor::Codex, false, true, "request");
        codex.finish("request");
        assert_eq!(
            codex.sent_user_bytes,
            turn_guard::guarded_message(Vendor::Codex, "request").len() as u64
        );
    }

    #[tokio::test]
    async fn old_missing_and_future_metrics_do_not_become_current_measurements() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE convo_events(id INTEGER PRIMARY KEY,task_id INTEGER,ts INTEGER,event TEXT,rewound_at INTEGER)").execute(&pool).await.unwrap();
        let now = 20 * 86400;
        let mut a = PromptMetrics::new(Vendor::Claude, true, false, "one");
        a.finish("one");
        let mut b = a.clone();
        b.instruction_sha256 = "a".repeat(64);
        for (id, task, ts, metrics, rewind) in [
            (1, 1, 0, Some(a.clone()), None),
            (2, 1, now - 1, Some(b.clone()), None),
            (3, 2, now - 1, Some(a.clone()), None),
            (4, 1, now, None, None),
            (5, 1, now, Some(a.clone()), Some(now)),
            (6, 1, now + 1, Some(a), None),
        ] {
            let event = serde_json::json!({"kind":"user", "prompt_metrics": metrics}).to_string();
            sqlx::query("INSERT INTO convo_events VALUES(?,?,?,?,?)")
                .bind(id)
                .bind(task)
                .bind(ts)
                .bind(event)
                .bind(rewind)
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO convo_events VALUES(7,1,?, ?,NULL)")
            .bind(now)
            .bind(r#"{"kind":"user","prompt_metrics":7}"#)
            .execute(&pool)
            .await
            .unwrap();
        let summary = summarize(&pool, "7d", now).await.unwrap();
        assert_eq!(
            (
                summary.accepted_turns,
                summary.observed_turns,
                summary.unknown_turns
            ),
            (4, 2, 2)
        );
        assert_eq!(summary.instruction_changes, 1);
        assert_eq!(summary.user_bytes, 6);
        assert_eq!(summary.instruction_bytes, 2 * b.instruction_bytes);
    }
}
