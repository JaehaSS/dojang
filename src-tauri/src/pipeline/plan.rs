//! 드라이버가 틱마다 "다음에 무엇을 할지"를 고르는 순수 함수. DB·프로세스를 만지지 않는다.
//! 실행은 `commands::pipeline` 쪽 드라이버가 맡고, 여기서는 상태 스냅샷만 보고 결정한다.

use super::model::{all_integrated, next_integration, ready_tickets, TicketView};
use super::state::{RunState, TicketState};

/// 실행 한 건의 결정에 필요한 스냅샷.
#[derive(Clone, Debug)]
pub struct RunCtx {
    pub state: RunState,
    pub has_integration: bool,
    pub has_spec: bool,
    pub has_feedback: bool,
    /// awaiting_merge_approval에서 통합 작업의 DB 상태.
    pub integration_task_state: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TicketCtx {
    pub view: TicketView,
    pub has_task: bool,
    /// 티켓 작업의 DB 상태. 작업이 사라졌으면 None.
    pub task_state: Option<String>,
    /// 작업의 에이전트 세션이 이 프로세스에서 아직 살아 있는가.
    pub agent_alive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// 할 일이 없다(외부 이벤트 대기).
    Idle,
    EnsureIntegration,
    Split,
    AdvanceToPlanReview,
    PlanReview,
    /// 사람의 개입이 필요하다. 사유와 함께 실행을 멈춘다.
    Pause(String),
    /// 에이전트가 끝난 티켓을 verifying으로 넘긴다.
    Collect(i64),
    Spawn(Vec<i64>),
    Verify(i64),
    Review(i64),
    Fix(i64),
    Integrate(i64),
    ToFinalReview,
    FinalReview,
    /// 통합 작업의 결말을 실행 상태에 반영한다(Done 또는 Cancelled).
    FinishRun(RunState),
}

/// 작업 상태 중 에이전트가 더 일하지 않는 상태.
fn task_settled(state: &str) -> bool {
    matches!(state, "awaiting_review" | "failed" | "done" | "discarded" | "finalizing")
}

/// Running 티켓의 에이전트가 끝났는가. 작업이 사라졌거나, 끝난 상태이거나, 일하는 상태인데
/// 세션이 죽었으면(앱 재시작) 끝난 것으로 본다. 이 경우 검증이 실패해 수정 경로로 간다.
pub fn agent_finished(t: &TicketCtx) -> bool {
    if !t.has_task {
        return false;
    }
    match t.task_state.as_deref() {
        None => true,
        Some(s) if task_settled(s) => true,
        Some(_) => !t.agent_alive,
    }
}

pub fn plan_next_action(run: &RunCtx, tickets: &[TicketCtx]) -> Action {
    match run.state {
        RunState::Drafting => {
            if !run.has_integration {
                Action::EnsureIntegration
            } else if !run.has_spec || run.has_feedback {
                Action::Split
            } else {
                Action::AdvanceToPlanReview
            }
        }
        RunState::PlanReview => Action::PlanReview,
        RunState::Executing => plan_executing(tickets),
        RunState::FinalReview => Action::FinalReview,
        RunState::AwaitingMergeApproval => match run.integration_task_state.as_deref() {
            Some("done") => Action::FinishRun(RunState::Done),
            Some("discarded") => Action::FinishRun(RunState::Cancelled),
            _ => Action::Idle,
        },
        RunState::AwaitingPlanApproval
        | RunState::Paused
        | RunState::Done
        | RunState::Cancelled
        | RunState::Failed => Action::Idle,
    }
}

fn plan_executing(tickets: &[TicketCtx]) -> Action {
    if let Some(t) = tickets
        .iter()
        .find(|t| t.view.state == TicketState::Escalated)
    {
        return Action::Pause(format!("티켓 {}이(가) 자동 수정 한도에 닿아 사람의 결정이 필요합니다", t.view.key));
    }
    let mut sorted: Vec<&TicketCtx> = tickets.iter().collect();
    sorted.sort_by(|a, b| a.view.key.cmp(&b.view.key));

    if let Some(t) = sorted
        .iter()
        .find(|t| t.view.state == TicketState::Running && agent_finished(t))
    {
        return Action::Collect(t.view.id);
    }
    for state in [TicketState::Verifying, TicketState::Reviewing, TicketState::Fixing] {
        if let Some(t) = sorted.iter().find(|t| t.view.state == state) {
            return match state {
                TicketState::Verifying => Action::Verify(t.view.id),
                TicketState::Reviewing => Action::Review(t.view.id),
                _ => Action::Fix(t.view.id),
            };
        }
    }
    let views: Vec<TicketView> = tickets.iter().map(|t| t.view.clone()).collect();
    if let Some(id) = next_integration(&views) {
        return Action::Integrate(id);
    }
    // 통합 중인 티켓이 남아 있으면(재시작 직후) 같은 통합을 이어 한다.
    if let Some(t) = sorted.iter().find(|t| t.view.state == TicketState::Integrating) {
        return Action::Integrate(t.view.id);
    }
    let mut spawn = ready_tickets(&views);
    // 재배정으로 Running이 되었지만 아직 작업이 없는 티켓도 띄운다.
    spawn.extend(
        sorted
            .iter()
            .filter(|t| t.view.state == TicketState::Running && !t.has_task)
            .map(|t| t.view.id),
    );
    if !spawn.is_empty() {
        return Action::Spawn(spawn);
    }
    if all_integrated(&views) {
        return Action::ToFinalReview;
    }
    // 티켓이 전부 취소돼 통합할 것이 없으면 영원히 Idle로 남지 않도록 멈추고 사용자가 정한다.
    if !views.is_empty() && views.iter().all(|t| t.state == TicketState::Cancelled) {
        return Action::Pause("모든 티켓이 취소되어 통합할 내용이 없습니다. 실행을 취소하거나 계획을 다시 세우세요".into());
    }
    Action::Idle
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tv(id: i64, key: &str, state: TicketState, deps: &[&str]) -> TicketView {
        TicketView {
            id,
            key: key.into(),
            state,
            deps: deps.iter().map(|s| s.to_string()).collect(),
            vendor: "codex".into(),
            reviewer_vendor: Some("claude".into()),
            attempt: 0,
            reassigned: false,
        }
    }

    fn tc(view: TicketView, task_state: Option<&str>, alive: bool) -> TicketCtx {
        TicketCtx {
            has_task: task_state.is_some(),
            task_state: task_state.map(String::from),
            agent_alive: alive,
            view,
        }
    }

    fn run(state: RunState) -> RunCtx {
        RunCtx {
            state,
            has_integration: true,
            has_spec: true,
            has_feedback: false,
            integration_task_state: None,
        }
    }

    #[test]
    fn drafting_orders_integration_split_review() {
        let mut r = run(RunState::Drafting);
        r.has_integration = false;
        assert_eq!(plan_next_action(&r, &[]), Action::EnsureIntegration);
        r.has_integration = true;
        r.has_spec = false;
        assert_eq!(plan_next_action(&r, &[]), Action::Split);
        r.has_spec = true;
        r.has_feedback = true;
        assert_eq!(plan_next_action(&r, &[]), Action::Split);
        r.has_feedback = false;
        assert_eq!(plan_next_action(&r, &[]), Action::AdvanceToPlanReview);
    }

    #[test]
    fn waiting_states_are_idle() {
        for s in [
            RunState::AwaitingPlanApproval,
            RunState::Paused,
            RunState::Done,
            RunState::Cancelled,
            RunState::Failed,
        ] {
            assert_eq!(plan_next_action(&run(s), &[]), Action::Idle, "{s:?}");
        }
    }

    #[test]
    fn merge_approval_follows_integration_task() {
        let mut r = run(RunState::AwaitingMergeApproval);
        assert_eq!(plan_next_action(&r, &[]), Action::Idle);
        r.integration_task_state = Some("awaiting_review".into());
        assert_eq!(plan_next_action(&r, &[]), Action::Idle);
        r.integration_task_state = Some("done".into());
        assert_eq!(plan_next_action(&r, &[]), Action::FinishRun(RunState::Done));
        r.integration_task_state = Some("discarded".into());
        assert_eq!(plan_next_action(&r, &[]), Action::FinishRun(RunState::Cancelled));
    }

    #[test]
    fn executing_spawns_ready_tickets_in_key_order() {
        let t = vec![
            tc(tv(2, "T2", TicketState::Pending, &["T1"]), None, false),
            tc(tv(1, "T1", TicketState::Pending, &[]), None, false),
            tc(tv(3, "T3", TicketState::Pending, &[]), None, false),
        ];
        assert_eq!(plan_next_action(&run(RunState::Executing), &t), Action::Spawn(vec![1, 3]));
    }

    #[test]
    fn escalated_ticket_pauses_the_run() {
        let t = vec![tc(tv(1, "T1", TicketState::Escalated, &[]), Some("failed"), false)];
        assert!(matches!(plan_next_action(&run(RunState::Executing), &t), Action::Pause(_)));
    }

    #[test]
    fn finished_agent_is_collected_before_anything_else() {
        let t = vec![
            tc(tv(1, "T1", TicketState::Running, &[]), Some("awaiting_review"), false),
            tc(tv(2, "T2", TicketState::Pending, &[]), None, false),
        ];
        assert_eq!(plan_next_action(&run(RunState::Executing), &t), Action::Collect(1));
    }

    #[test]
    fn running_agent_is_left_alone() {
        let t = vec![tc(tv(1, "T1", TicketState::Running, &[]), Some("running"), true)];
        assert_eq!(plan_next_action(&run(RunState::Executing), &t), Action::Idle);
    }

    #[test]
    fn dead_session_after_restart_counts_as_finished() {
        let t = vec![tc(tv(1, "T1", TicketState::Running, &[]), Some("running"), false)];
        assert_eq!(plan_next_action(&run(RunState::Executing), &t), Action::Collect(1));
        let gone = TicketCtx {
            view: tv(1, "T1", TicketState::Running, &[]),
            has_task: true,
            task_state: None,
            agent_alive: false,
        };
        assert!(agent_finished(&gone));
    }

    #[test]
    fn pipeline_stages_run_before_integration_and_spawn() {
        let t = vec![
            tc(tv(1, "T1", TicketState::Ready, &[]), Some("awaiting_review"), false),
            tc(tv(2, "T2", TicketState::Fixing, &[]), Some("awaiting_review"), false),
            tc(tv(3, "T3", TicketState::Pending, &[]), None, false),
        ];
        assert_eq!(plan_next_action(&run(RunState::Executing), &t), Action::Fix(2));
        let t = vec![
            tc(tv(1, "T1", TicketState::Ready, &[]), Some("awaiting_review"), false),
            tc(tv(3, "T3", TicketState::Pending, &[]), None, false),
        ];
        assert_eq!(plan_next_action(&run(RunState::Executing), &t), Action::Integrate(1));
    }

    #[test]
    fn integration_is_one_at_a_time() {
        let t = vec![
            tc(tv(1, "T1", TicketState::Integrating, &[]), Some("awaiting_review"), false),
            tc(tv(2, "T2", TicketState::Ready, &[]), Some("awaiting_review"), false),
        ];
        assert_eq!(plan_next_action(&run(RunState::Executing), &t), Action::Integrate(1));
    }

    #[test]
    fn reassigned_running_ticket_without_task_is_respawned() {
        let t = vec![tc(tv(1, "T1", TicketState::Running, &[]), None, false)];
        assert_eq!(plan_next_action(&run(RunState::Executing), &t), Action::Spawn(vec![1]));
    }

    #[test]
    fn all_integrated_moves_to_final_review() {
        let t = vec![
            tc(tv(1, "T1", TicketState::Integrated, &[]), None, false),
            tc(tv(2, "T2", TicketState::Cancelled, &[]), None, false),
        ];
        assert_eq!(plan_next_action(&run(RunState::Executing), &t), Action::ToFinalReview);
    }

    #[test]
    fn cancelled_only_never_reaches_final_review() {
        let t = vec![tc(tv(1, "T1", TicketState::Cancelled, &[]), None, false)];
        assert!(matches!(plan_next_action(&run(RunState::Executing), &t), Action::Pause(_)));
    }
}
