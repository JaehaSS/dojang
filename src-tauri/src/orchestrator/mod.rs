//! Runner와 Tauri command가 함께 사용하는 task 영속화 경계.
//!
//! 프로세스 spawn·창 이벤트는 각 host adapter가 담당한다. 이 모듈은 task 생성 레코드와
//! review finalization CAS처럼 SQLite만 필요한 상태 전이만 소유한다.

use sqlx::SqlitePool;

use crate::db::{self, Task};
use crate::goal_contract::GoalContract;

/// 작업 생성 기원 — host adapter가 즉시 실행 또는 승인 대기를 결정할 때 사용한다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskOrigin {
    Ui,
    External,
    /// 멀티 벤더 파이프라인 드라이버가 만든 작업. External처럼 최신화·직접 모드·질문 응답을
    /// 쓰지 않지만, 사용자가 이미 계획을 승인했으므로 승인 대기 없이 곧바로 헤드리스로 spawn한다.
    Pipeline,
}

impl TaskOrigin {
    /// 지표와 로그에 남기는 기원 라벨.
    pub fn label(self) -> &'static str {
        match self {
            TaskOrigin::Ui => "ui",
            TaskOrigin::External => "external",
            TaskOrigin::Pipeline => "pipeline",
        }
    }
}

/// host adapter가 정규화한 작업 생성 요청이다.
#[derive(Clone, Debug)]
pub struct CreateTaskParams {
    pub repo: String,
    pub instruction: String,
    pub agent: String,
    pub role: String,
    pub model: String,
    pub reasoning_effort: String,
    pub service_tier: Option<String>,
    pub headless: bool,
    pub ensemble: String,
    pub mode: String,
    pub cmd: String,
    pub args: Vec<String>,
    pub cols: u16,
    pub rows: u16,
    pub origin: TaskOrigin,
    pub goal_contract: Option<GoalContract>,
    /// 인터뷰 결정화 모호성 점수 — 생성 시 1회 기록. 인터뷰 미사용·Runner/스케줄 경로는 None.
    pub ambiguity: Option<crate::interview::AmbiguityScore>,
    /// 로컬 시작 브랜치. 격리는 분기 기준, 직접 실행은 메인 checkout 대상이다.
    /// None이면 레포가 지금 체크아웃한 브랜치를 쓴다(종전 동작).
    /// UI가 고르지 않는 경로(크론·봇·Runner)는 계속 None이다.
    pub base_branch: Option<String>,
    /// 프런트가 이 생성을 식별하는 토큰. Some일 때만 진행 이벤트(`task://creating`)를 보낸다.
    /// 진행을 보는 사람이 없는 경로(크론·봇·Runner·today 착수)는 None이다.
    pub client_ref: Option<String>,
    /// 이어받을 원본 작업 id. Some이면 삽입 직후 그 작업의 벤더 세션을 물려받는다
    /// (`db::adopt_conversation`). 새로 시작하는 모든 경로는 None이다.
    pub resume_from: Option<i64>,
    /// 세션홈에서 고른 외부 벤더 세션 id. Some이면 삽입 직후 `sessionhome::resolve` +
    /// `db::adopt_external_session`으로 승계한다. `resume_from`과 배타 — 둘 다 Some이면
    /// 생성 자체를 거절한다. 새로 시작하는 모든 경로는 None이다.
    pub resume_session: Option<String>,
    pub resume_vendor: Option<String>,
    pub purpose_selection: Option<crate::project_plan::PurposeSelection>,
    /// Claude 질문 세션에서 도구 실행 전 사용자 승인을 받는가. 그 밖의 생성은 false다.
    pub approvals: bool,
}

impl CreateTaskParams {
    pub fn headless_terminal(
        repo: String,
        instruction: String,
        agent: String,
        origin: TaskOrigin,
    ) -> Self {
        Self {
            repo,
            instruction,
            agent,
            role: crate::agent::DEFAULT_ROLE.to_string(),
            model: String::new(),
            reasoning_effort: String::new(),
            service_tier: None,
            headless: true,
            ensemble: String::new(),
            mode: "terminal".to_string(),
            cmd: String::new(),
            args: Vec::new(),
            cols: 80,
            rows: 24,
            origin,
            goal_contract: None,
            ambiguity: None,
            base_branch: None,
            client_ref: None,
            resume_from: None,
            resume_session: None,
            resume_vendor: None,
            purpose_selection: None,
            approvals: false,
        }
    }
}

/// worktree 생성 뒤 SQLite에 기록할 task 메타데이터.
#[derive(Clone, Debug)]
pub struct TaskDraft {
    pub repo: String,
    pub branch: String,
    pub base: String,
    pub worktree_path: String,
    pub instruction: String,
    pub agent: Option<String>,
    pub role: String,
    pub ensemble: Option<String>,
    pub mode: String,
    pub goal_contract: Option<GoalContract>,
    pub ambiguity: Option<crate::interview::AmbiguityScore>,
}

/// SQLite task state transition의 Tauri 비의존 façade.
#[derive(Clone)]
pub struct TaskService {
    pool: SqlitePool,
}

impl TaskService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn create_task(
        &self,
        draft: TaskDraft,
        model: Option<&str>,
        reasoning_effort: Option<&str>,
        now: i64,
    ) -> Result<Task, String> {
        let model = model.map(str::trim).filter(|value| !value.is_empty());
        let reasoning_effort = crate::agent::reasoning_effort_override_for_model(
            draft.agent.as_deref().unwrap_or_default(),
            model,
            reasoning_effort,
        )?;
        let id = db::insert_task_with_role_and_goal_contract(
            &self.pool,
            &draft.repo,
            &draft.branch,
            &draft.base,
            &draft.worktree_path,
            &draft.instruction,
            draft.agent.as_deref(),
            &draft.role,
            draft.ensemble.as_deref(),
            model,
            reasoning_effort.as_deref(),
            &draft.mode,
            draft.goal_contract.as_ref(),
            draft.ambiguity.as_ref(),
            now,
        )
        .await
        .map_err(|error| error.to_string())?;
        db::get_task(&self.pool, id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "생성된 Task를 찾을 수 없음".to_string())
    }

    pub async fn claim_review_finalization(
        &self,
        id: i64,
        convo_active: bool,
        now: i64,
    ) -> Result<Task, String> {
        if convo_active {
            return Err("대화 턴이 아직 종료 처리 중입니다 — 잠시 후 다시 시도하세요".to_string());
        }
        if !db::claim_review_finalization(&self.pool, id, now)
            .await
            .map_err(|error| error.to_string())?
        {
            return Err("검토 대기 상태의 작업만 승인하거나 버릴 수 있습니다".to_string());
        }
        match db::get_task(&self.pool, id).await {
            Ok(Some(task)) => Ok(task),
            Ok(None) => {
                let _ = db::restore_awaiting_review(&self.pool, id, now).await;
                Err("작업을 찾을 수 없습니다".to_string())
            }
            Err(error) => {
                let _ = db::restore_awaiting_review(&self.pool, id, now).await;
                Err(error.to_string())
            }
        }
    }
}
