pub mod actions;
pub mod auth;
pub mod capacity;
pub mod config;
pub mod events;
pub mod file_mutation;
pub mod finalization;
pub mod http;
mod memory_gate;
pub mod mobile_http;
pub mod process;
pub(crate) mod process_identity;
pub mod push;
pub mod queue;
pub mod review_http;
pub mod review_process;
pub mod session;
mod task_creation;
#[cfg(test)]
mod task_creation_tests;
pub mod worktree_lock;

use serde::Deserialize;

use crate::goal_contract::GoalContract;

pub use finalization::finalize_task;
pub use memory_gate::{approve_pending_task, cancel_pending_task};
pub use task_creation::{create_queued_task, CreateTaskError};

pub const RETENTION_DAYS: i64 = 60;

/// Runner HTTP가 받는 비대화형 작업 생성 요청. Runner는 항상 격리 worktree와 durable queue를 쓴다.
#[derive(Debug, Deserialize)]
pub struct QueuedTaskRequest {
    pub repository: String,
    pub instruction: String,
    pub agent: String,
    #[serde(default = "default_agent_role")]
    pub role: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub reasoning_effort: String,
    #[serde(default = "default_task_mode")]
    pub mode: String,
    #[serde(default)]
    pub goal_contract: Option<GoalContract>,
    /// 세션홈에서 고른 벤더 세션을 이 작업이 이어받는다(설계 2026-09-17). `mode`가
    /// `conversation`이어야 하고, agy는 세션 id 대신 "직전 대화" 센티널을 쓰므로 거절된다.
    #[serde(default)]
    pub resume_session: Option<String>,
    pub resume_vendor: Option<String>,
}

fn default_task_mode() -> String {
    "terminal".to_string()
}

fn default_agent_role() -> String {
    crate::agent::DEFAULT_ROLE.to_string()
}

/// Unix epoch 초. 신규 Runner 코드의 공통 시각 소스.
pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
