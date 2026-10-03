//! Durable receipt and lease ledger for Runner-owned review children.

mod ledger;
pub(crate) use ledger::resolve;
pub use ledger::{lease, quarantine, register, register_observed};
mod fence;
#[cfg(test)]
mod ledger_tests;
pub use fence::{assert_task_not_quarantined, assert_task_unfenced, task_is_fenced};
mod schema;
pub use schema::migrate;
mod supervisor;
pub use supervisor::registrar;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewOperation {
    Verify,
    Repair,
    /// Challenge 리뷰 기능은 제거됐지만(#75) variant는 남긴다 — `recovery::parse_operation`이
    /// DB에 이미 적힌 `operation` 문자열을 파싱하므로, 지우면 기존 원장의 challenge lease를
    /// 복구할 수 없다. 새 레코드를 만드는 프로덕션 경로는 더 이상 없다.
    Challenge,
}

impl ReviewOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verify => "verify",
            Self::Repair => "repair",
            Self::Challenge => "challenge",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewPhase {
    VerifyBuild,
    VerifyTest,
    Reviewer,
    RepairAgent,
    RepairCheck,
}

impl ReviewPhase {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::VerifyBuild => "verify_build",
            Self::VerifyTest => "verify_test",
            Self::Reviewer => "reviewer",
            Self::RepairAgent => "repair_agent",
            Self::RepairCheck => "repair_check",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewProcessState {
    Active,
    Quarantined,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct ReviewProcessQuarantine {
    pub receipt_id: i64,
    pub task_id: i64,
    pub operation: String,
    pub phase: String,
    pub pgid: i64,
    pub state: &'static str,
    pub reason: String,
    pub detail: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewProcessRepairStatus {
    ResolvedAbsent,
    ResolvedTerminated,
    StillQuarantined,
    AlreadyResolved,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct ReviewProcessRepairResult {
    pub receipt_id: i64,
    pub status: ReviewProcessRepairStatus,
    pub reason: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug)]
pub enum ReviewProcessRepairError {
    ReceiptNotFound,
    NotQuarantined,
    StaleReceipt,
    Internal(anyhow::Error),
}

impl std::fmt::Display for ReviewProcessRepairError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReceiptNotFound => write!(formatter, "review process receipt not found"),
            Self::NotQuarantined => write!(formatter, "review process receipt is not quarantined"),
            Self::StaleReceipt => write!(formatter, "review process receipt is stale"),
            Self::Internal(error) => write!(formatter, "{error}"),
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ReviewProcessReceipt {
    pub id: i64,
    pub task_id: i64,
    pub operation: String,
    pub phase: String,
    pub pgid: i64,
    pub identity_hash: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewProcessLease {
    pub task_id: i64,
    pub operation: String,
    pub receipt_id: i64,
    pub state: ReviewProcessState,
    pub detail: Option<String>,
    pub updated_at: i64,
}

pub(super) fn validate_registration(
    operation: ReviewOperation,
    phase: ReviewPhase,
    pgid: i64,
    identity_hash: &str,
) -> anyhow::Result<()> {
    let valid_phase = matches!(
        (operation, phase),
        (
            ReviewOperation::Verify,
            ReviewPhase::VerifyBuild | ReviewPhase::VerifyTest
        ) | (ReviewOperation::Challenge, ReviewPhase::Reviewer)
          | (ReviewOperation::Repair, ReviewPhase::RepairAgent | ReviewPhase::RepairCheck)
    );
    if !valid_phase || !(1..=i64::from(i32::MAX)).contains(&pgid) || identity_hash.len() != 64 {
        anyhow::bail!("invalid review process registration");
    }
    Ok(())
}
