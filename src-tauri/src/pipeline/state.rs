//! 실행(run)·티켓 상태와 전이 규칙. 허용 전이 표는 `RUN_TRANSITIONS`·`TICKET_TRANSITIONS`가 정본이다.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Drafting,
    PlanReview,
    AwaitingPlanApproval,
    Executing,
    FinalReview,
    AwaitingMergeApproval,
    Done,
    Paused,
    Cancelled,
    Failed,
}

impl RunState {
    pub const ALL: [RunState; 10] = [
        RunState::Drafting,
        RunState::PlanReview,
        RunState::AwaitingPlanApproval,
        RunState::Executing,
        RunState::FinalReview,
        RunState::AwaitingMergeApproval,
        RunState::Done,
        RunState::Paused,
        RunState::Cancelled,
        RunState::Failed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RunState::Drafting => "drafting",
            RunState::PlanReview => "plan_review",
            RunState::AwaitingPlanApproval => "awaiting_plan_approval",
            RunState::Executing => "executing",
            RunState::FinalReview => "final_review",
            RunState::AwaitingMergeApproval => "awaiting_merge_approval",
            RunState::Done => "done",
            RunState::Paused => "paused",
            RunState::Cancelled => "cancelled",
            RunState::Failed => "failed",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            RunState::Done | RunState::Cancelled | RunState::Failed
        )
    }
}

impl FromStr for RunState {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        RunState::ALL
            .iter()
            .copied()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| format!("알 수 없는 실행 상태: {s}"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketState {
    Pending,
    Running,
    Verifying,
    Reviewing,
    Fixing,
    Ready,
    Integrating,
    Integrated,
    Escalated,
    Cancelled,
}

impl TicketState {
    pub const ALL: [TicketState; 10] = [
        TicketState::Pending,
        TicketState::Running,
        TicketState::Verifying,
        TicketState::Reviewing,
        TicketState::Fixing,
        TicketState::Ready,
        TicketState::Integrating,
        TicketState::Integrated,
        TicketState::Escalated,
        TicketState::Cancelled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TicketState::Pending => "pending",
            TicketState::Running => "running",
            TicketState::Verifying => "verifying",
            TicketState::Reviewing => "reviewing",
            TicketState::Fixing => "fixing",
            TicketState::Ready => "ready",
            TicketState::Integrating => "integrating",
            TicketState::Integrated => "integrated",
            TicketState::Escalated => "escalated",
            TicketState::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, TicketState::Integrated | TicketState::Cancelled)
    }
}

impl FromStr for TicketState {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        TicketState::ALL
            .iter()
            .copied()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| format!("알 수 없는 티켓 상태: {s}"))
    }
}

use RunState as R;
use TicketState as T;

/// 실행 상태 허용 전이 표(비종결 → Cancelled는 별도 규칙으로 항상 허용).
/// Paused는 멈춘 지점으로만 복귀하며, 복귀 대상 일치는 DB(`paused_from`)가 보장한다.
pub const RUN_TRANSITIONS: &[(RunState, &[RunState])] = &[
    (R::Drafting, &[R::PlanReview, R::Failed, R::Paused]),
    (
        R::PlanReview,
        &[R::AwaitingPlanApproval, R::Drafting, R::Failed, R::Paused],
    ),
    (R::AwaitingPlanApproval, &[R::Executing, R::Drafting]),
    (R::Executing, &[R::FinalReview, R::Paused]),
    (R::FinalReview, &[R::AwaitingMergeApproval, R::Paused]),
    (R::AwaitingMergeApproval, &[R::Done, R::Paused]),
    (
        R::Paused,
        &[
            R::Executing,
            R::FinalReview,
            R::AwaitingMergeApproval,
            R::PlanReview,
            R::Drafting,
        ],
    ),
    (R::Done, &[]),
    (R::Cancelled, &[]),
    (R::Failed, &[]),
];

/// 티켓 상태 허용 전이 표(비종결 → Cancelled는 별도 규칙으로 항상 허용).
pub const TICKET_TRANSITIONS: &[(TicketState, &[TicketState])] = &[
    (T::Pending, &[T::Running]),
    (T::Running, &[T::Verifying]),
    (T::Verifying, &[T::Reviewing, T::Fixing, T::Escalated]),
    (T::Reviewing, &[T::Ready, T::Fixing, T::Escalated]),
    (T::Fixing, &[T::Verifying, T::Running, T::Escalated]),
    (T::Ready, &[T::Integrating]),
    (T::Integrating, &[T::Integrated, T::Escalated]),
    (T::Escalated, &[T::Pending, T::Running, T::Cancelled]),
    (T::Integrated, &[]),
    (T::Cancelled, &[]),
];

pub fn run_transition_allowed(from: RunState, to: RunState) -> bool {
    if from.is_terminal() {
        return false;
    }
    if to == R::Cancelled {
        return true;
    }
    RUN_TRANSITIONS
        .iter()
        .any(|(f, tos)| *f == from && tos.contains(&to))
}

pub fn ticket_transition_allowed(from: TicketState, to: TicketState) -> bool {
    if from.is_terminal() {
        return false;
    }
    if to == T::Cancelled {
        return true;
    }
    TICKET_TRANSITIONS
        .iter()
        .any(|(f, tos)| *f == from && tos.contains(&to))
}
