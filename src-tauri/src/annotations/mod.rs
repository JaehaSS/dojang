//! Durable review annotations and conversation resend formatting.

use serde::{Deserialize, Serialize};

mod rematch;
mod resend;
mod store;

pub use rematch::{rematch, RematchedAnnotation};
pub use resend::{build_resend_items, format_resend, quote_line, ResendItem};
pub use store::{
    create_agent_note, create_draft, list_by_ids, list_by_task, mark_draft, mark_sent, migrate,
    update_draft_body,
};

pub mod status {
    pub const DRAFT: &str = "draft";
    pub const SENT: &str = "sent";
    pub const RESOLVED: &str = "resolved";
}

/// 누가 썼나. 사람 주석은 `human`, 에이전트 노트는 벤더 바이너리 이름(`claude`·`codex`·`agy`)이다.
/// 재전송은 `human`만 보낸다 — 에이전트가 쓴 노트가 에이전트에게 되돌아가지 않게 하는 경계다.
pub mod author {
    pub const HUMAN: &str = "human";

    pub fn is_human(author: &str) -> bool {
        author == HUMAN
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct ReviewAnnotation {
    pub id: String,
    pub task_id: i64,
    pub hunk_id: String,
    pub path: String,
    pub line: i64,
    pub side: String,
    pub body_md: String,
    pub status: String,
    pub created_at: i64,
    /// `author::HUMAN` 또는 벤더 이름. 열이 없던 시절의 행은 기본값 human이다.
    #[serde(default = "default_author")]
    pub author: String,
}

fn default_author() -> String {
    author::HUMAN.to_string()
}

#[cfg(test)]
mod tests;
