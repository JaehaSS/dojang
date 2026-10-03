//! Fail-closed Runner task creation and memory projection.

use sqlx::SqlitePool;

use crate::db::{self, state};
use crate::orchestrator::{TaskDraft, TaskService};

use super::worktree_lock::WorktreeLocks;
use super::{config, QueuedTaskRequest};

/// [`create_queued_task`] 실패 사유. HTTP 계층이 상태 코드를 가르는 데 쓴다(설계
/// 2026-09-17 결정 9) — 로컬 사유(`Invalid`)는 400, 세션 해석·인가 실패는 404, 중복 승계는
/// 409(진행 중인 작업 id를 싣는다).
#[derive(Debug)]
pub enum CreateTaskError {
    /// 요청 형식·저장소 인가 등 일반 실패.
    Invalid(String),
    /// 이어받으려는 세션을 찾지 못했거나(없음·모호함) 인가 루트 밖이다 — 존재 열거를 막기 위해
    /// 두 사유를 하나의 얼굴로 합친다.
    SessionUnavailable,
    /// 같은 세션을 이미 물고 있는 살아 있는 작업이 있다. 그 작업 id.
    Conflict(i64),
}

impl std::fmt::Display for CreateTaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CreateTaskError::Invalid(message) => write!(f, "{message}"),
            CreateTaskError::SessionUnavailable => write!(f, "세션을 찾을 수 없습니다"),
            CreateTaskError::Conflict(task_id) => {
                write!(f, "이 세션은 이미 #{task_id} 작업이 이어가고 있습니다")
            }
        }
    }
}

impl std::error::Error for CreateTaskError {}

impl From<String> for CreateTaskError {
    fn from(value: String) -> Self {
        CreateTaskError::Invalid(value)
    }
}

impl From<db::AdoptError> for CreateTaskError {
    fn from(value: db::AdoptError) -> Self {
        match value {
            db::AdoptError::Conflict(task_id) => CreateTaskError::Conflict(task_id),
            db::AdoptError::Db(error) => CreateTaskError::Invalid(error.to_string()),
        }
    }
}

/// Creates an isolated task and exposes it to approval/queue only after projection is applied.
pub async fn create_queued_task(
    config: &config::RunnerConfig,
    pool: &SqlitePool,
    worktree_locks: &WorktreeLocks,
    request: QueuedTaskRequest,
    now: i64,
) -> Result<db::Task, CreateTaskError> {
    validate_request(&request)?;
    let instruction = request.instruction.trim().to_string();
    let agent = request.agent.trim().to_string();
    let repo = super::auth::authorize_repository_path(
        &config.repository_roots,
        std::path::Path::new(&request.repository),
    )
    .map_err(|_| "허용되지 않는 repository 경로입니다".to_string())?;
    // git 서브프로세스 동기 대기 — tokio 워커 점유 방지를 위해 blocking 풀로 분리.
    let worktree = {
        let repo = repo.clone();
        let instruction = instruction.clone();
        let has_goal_contract = request.goal_contract.is_some();
        tokio::task::spawn_blocking(move || {
            create_worktree_or_direct(&repo, &instruction, has_goal_contract)
        })
        .await
        .map_err(|error| error.to_string())??
    };
    let _guard = worktree_locks.acquire(&worktree.path).await;
    let task = create_task(pool, &request, &repo, &worktree, &instruction, &agent, now).await?;
    match project_memory(pool, &task, now).await {
        Ok(true) => {
            let _ = crate::followup_observation::insert_observation_start(pool, task.id, now).await;
        }
        Ok(false) => {}
        Err(error) => {
            return fail_created_task(pool, task, worktree, CreateTaskError::Invalid(error), now)
                .await;
        }
    }
    // 승계는 `project_memory` 뒤·`expose_created_task` 앞에서만 일어난다(설계 결정 2) — 기본
    // 실행 정책이 AlwaysApprove라 노출 즉시 워커가 집어가므로, 그 뒤에 승계하면 첫 턴만 문맥
    // 없이 도는 모양이 된다.
    if let Some(session_id) = request.resume_session.as_deref() {
        if let Err(error) = adopt_resumed_session(config, pool, task.id, request.resume_vendor.as_deref().unwrap_or("claude"), session_id, now).await {
            return fail_created_task(pool, task, worktree, error, now).await;
        }
    }
    if let Err(error) = expose_created_task(config, pool, task.id, now).await {
        return fail_created_task(pool, task, worktree, CreateTaskError::Invalid(error), now).await;
    }
    db::get_task(pool, task.id)
        .await
        .map_err(|error| CreateTaskError::Invalid(error.to_string()))?
        .ok_or_else(|| CreateTaskError::Invalid("생성된 Runner task를 찾을 수 없습니다".to_string()))
}

/// 외부 세션 승계 — `sessionhome::resolve`로 파일을 확정하고, 해석된 cwd(선두·말미 둘 다)가
/// `repository_roots` 인가를 통과할 때만 `db::adopt_external_session`으로 넘긴다.
///
/// 목록 필터만으로는 `POST /v1/tasks`에 세션 id를 직접 실어 인가를 우회할 수 있으므로, 목록과
/// 같은 `sessionhome::authorize_cwd` 판정을 여기서도 그대로 건다(설계 제약 3). 해석 실패(없음·
/// 모호함)와 인가 실패는 같은 [`CreateTaskError::SessionUnavailable`]로 합쳐 존재 열거를
/// 막는다(설계 결정 9).
async fn adopt_resumed_session(
    config: &config::RunnerConfig,
    pool: &SqlitePool,
    task_id: i64,
    vendor: &str,
    session_id: &str,
    now: i64,
) -> Result<(), CreateTaskError> {
    let roots = config.repository_roots.clone();
    let owned_id = session_id.to_string();
    let lookup_id = owned_id.clone();
    let lookup_vendor = vendor.to_string();
    // `resolve`(디렉터리 순회)와 메타 조회(선두/말미 샘플링)는 동기 파일시스템 IO라 blocking
    // 풀로 분리한다 — git worktree 생성과 같은 이유.
    let meta = tokio::task::spawn_blocking(move || resolve_session_meta(&lookup_vendor, &lookup_id))
        .await
        .map_err(|error| CreateTaskError::Invalid(error.to_string()))??;
    let authorized = crate::sessionhome::authorize_cwd(&roots, meta.cwd.as_deref())
        && crate::sessionhome::authorize_cwd(&roots, meta.last_cwd.as_deref());
    if !authorized {
        return Err(CreateTaskError::SessionUnavailable);
    }
    db::adopt_external_session_vendor(pool, task_id, vendor, &owned_id, now)
        .await
        .map_err(CreateTaskError::from)
}

/// 해석 실패 사유(없음·모호함·문법 오류)를 하나의 [`CreateTaskError::SessionUnavailable`]로
/// 뭉갠다 — 존재 열거를 막는다(설계 결정 9). `sessionhome::describe`는 유일성을 확정한 뒤
/// 그 파일 하나만 인덱싱하므로 승계마다 세션홈 전체를 훑지 않는다.
fn resolve_session_meta(vendor: &str, session_id: &str) -> Result<crate::sessionhome::SessionMeta, CreateTaskError> {
    crate::sessionhome::describe_vendor(vendor, session_id).map_err(|_| CreateTaskError::SessionUnavailable)
}

/// 격리 워크트리를 만들되, git 저장소가 아니면 폴더에서 바로 실행하는 직접 모드로 폴백한다.
///
/// 로컬(Tauri) 경로와 같은 규약을 쓴다 — `path == repo`가 직접 모드의 유일한 마커이고,
/// diff·hunk 조회는 이미 `is_git_repository`로 방어되어 빈 결과를 돌려준다. 격리가 가능한
/// (=git) 저장소에서는 언제나 격리를 택하므로 기존 작업의 안전 수준은 그대로다.
fn create_worktree_or_direct(
    repo: &std::path::Path,
    instruction: &str,
    has_goal_contract: bool,
) -> Result<crate::worktree::Worktree, String> {
    if !crate::worktree::is_git_repository(repo) {
        // Goal Contract는 승인 시 `git diff`로 변경 경로를 대조해 강제한다 — git이 없으면
        // 검증할 방법이 없다. 조용히 통과시키는 대신 생성을 막아 계약이 헛돌지 않게 한다.
        if has_goal_contract {
            return Err(
                "Goal Contract는 git 저장소에서만 검증할 수 있습니다 — 폴더를 git으로 초기화하거나 계약 없이 실행하세요"
                    .to_string(),
            );
        }
        let base = crate::worktree::DIRECT_BRANCH.to_string();
        return Ok(crate::worktree::Worktree {
            repo: repo.to_path_buf(),
            path: repo.to_path_buf(),
            branch: base.clone(),
            base,
            // git이 아니면 기준점을 잡을 수 없다 — 레거시 경로로 돈다.
            base_revision: None,
        });
    }
    // Runner는 base를 고르지 않는다 — 러너 머신 레포의 현재 체크아웃에서 분기한다.
    // 최신화하지 않는다(`false`). Runner 기원은 무인 실행이라 자격증명 프롬프트를 받아 줄
    // 사람이 없고, 원격에 못 닿는 환경에서 매 작업마다 30초를 버리게 된다.
    // 러너에는 진행을 보고 있는 사람이 없다 — 단계 콜백은 no-op.
    crate::worktree::create(repo, &branch_name(instruction), None, false, &|_| {})
        .map(|(wt, _)| wt)
        .map_err(|error| error.to_string())
}

pub(super) fn validate_request(request: &QueuedTaskRequest) -> Result<(), String> {
    if request.instruction.trim().is_empty() || request.agent.trim().is_empty() {
        return Err("instruction과 agent는 필수입니다".to_string());
    }
    if request.mode != "terminal" && request.mode != "conversation" {
        return Err("mode는 terminal 또는 conversation이어야 합니다".to_string());
    }
    if let Some(session_id) = request.resume_session.as_deref() {
        if session_id.trim().is_empty() {
            return Err("resume_session이 비어 있습니다".to_string());
        }
        if request.mode != "conversation" {
            return Err("resume_session은 conversation 모드에서만 사용할 수 있습니다".to_string());
        }
        // agy가 저장하는 것은 세션 id가 아니라 "직전 대화"라는 센티널이다(print 모드에서 대화
        // id를 얻을 수 없다) — 물려받아도 새 워크트리에는 직전 대화가 없다. 톤은 기존
        // `task_resume`의 AGY_CONTINUE 가드(`commands.rs`)를 따른다.
        if request.agent.trim() == "agy" {
            return Err(
                "agy 세션은 세션 식별자 대신 '직전 대화' 표시만 가지므로 이어받을 수 없습니다"
                    .to_string(),
            );
        }
    }
    if request.resume_session.is_some() {
        let vendor = request.resume_vendor.as_deref().unwrap_or("claude");
        if !matches!(vendor, "claude" | "codex") || vendor != request.agent.trim() {
            return Err("세션 공급자와 에이전트가 일치해야 합니다".into());
        }
    } else if request.resume_vendor.is_some() {
        return Err("세션 없이 공급자를 지정할 수 없습니다".into());
    }
    crate::agent::normalize_role_or_default(&request.role)?;
    crate::agent::reasoning_effort_override_for_model(
        &request.agent,
        Some(&request.model),
        Some(&request.reasoning_effort),
    )?;
    if let Some(contract) = &request.goal_contract {
        contract.validate()?;
    }
    Ok(())
}

fn branch_name(instruction: &str) -> String {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("dojang/{}-{suffix}", crate::worktree::slugify(instruction))
}

async fn create_task(
    pool: &SqlitePool,
    request: &QueuedTaskRequest,
    repo: &std::path::Path,
    worktree: &crate::worktree::Worktree,
    instruction: &str,
    agent: &str,
    now: i64,
) -> Result<db::Task, String> {
    let draft = TaskDraft {
        repo: repo.to_string_lossy().into_owned(),
        branch: worktree.branch.clone(),
        base: worktree.base.clone(),
        worktree_path: worktree.path.to_string_lossy().into_owned(),
        instruction: instruction.to_string(),
        agent: Some(agent.to_string()),
        role: crate::agent::normalize_role_or_default(&request.role)?.to_string(),
        ensemble: None,
        mode: request.mode.clone(),
        goal_contract: request.goal_contract.clone(),
        // Runner/스케줄 경로는 인터뷰 범위 외 (Plan 0021) — 컴파일 정합만.
        ambiguity: None,
    };
    TaskService::new(pool.clone())
        .create_task(
            draft,
            Some(&request.model),
            Some(&request.reasoning_effort),
            now,
        )
        .await
        .inspect_err(|_| {
            let _ = worktree.discard();
        })
}

/// 파일형 메모리 투영(설계 2026-09-13). Runner는 Tauri가 없어 앱 데이터 디렉터리를
/// 물어볼 곳이 없으므로, 자신이 연 DB 파일의 위치에서 되돌려 얻는다.
///
/// 반환값(`bool`)은 실제 내용이 실렸는지다 — 호출자가 이 값으로 "메모리 안내 후 재설명
/// 없음" 인사이트의 마커를 남긴다.
async fn project_memory(pool: &SqlitePool, task: &db::Task, now: i64) -> Result<bool, String> {
    let targets = crate::projector::project_targets();
    let data_dir = crate::memory::file::data_dir(pool);
    crate::memory::file::project(
        pool,
        &data_dir,
        &task.repo,
        std::path::Path::new(&task.worktree_path),
        &targets,
        task.id,
        now,
    )
    .await
    .map_err(|error| error.to_string())
}

async fn fail_created_task(
    pool: &SqlitePool,
    task: db::Task,
    worktree: crate::worktree::Worktree,
    error: CreateTaskError,
    now: i64,
) -> Result<db::Task, CreateTaskError> {
    let reason = error.to_string();
    // 실패 사유를 이벤트 종류로도 가른다 — 승계 실패를 `memory_projection_failed`로 적으면
    // 나중에 로그만 보는 사람이 원장 투영을 뒤지게 된다.
    let event_kind = match error {
        CreateTaskError::SessionUnavailable | CreateTaskError::Conflict(_) => {
            "session_adoption_failed"
        }
        CreateTaskError::Invalid(_) => "memory_projection_failed",
    };
    // 옛 DB 투영(P2)의 retire는 제거됐다(설계 2026-09-13 §P2 제거) — 파일형 투영은 회수할
    // 원장이 없다. 투영 바이트는 아래 worktree 폐기가 함께 가져간다.
    let _ = db::transition_state_with_runner_event(
        pool,
        task.id,
        state::FAILED,
        now,
        event_kind,
        Some(&reason),
    )
    .await;
    // 직접 모드는 격리 워크트리가 없다 — discard는 메인 체크아웃을 지우려 들므로 건너뛴다.
    if !worktree.is_direct() {
        worktree.discard().map_err(|discard_error| {
            CreateTaskError::Invalid(format!(
                "{reason}; worktree cleanup failed: {discard_error}"
            ))
        })?;
    }
    // 세션 해석·인가 실패(404)와 중복 승계(409)는 원래 얼굴 그대로 위로 올려야 HTTP 계층이
    // 상태 코드를 가를 수 있다(설계 결정 9) — 여기서 문자열로 뭉개지 않는다.
    Err(error)
}

async fn expose_created_task(
    config: &config::RunnerConfig,
    pool: &SqlitePool,
    task_id: i64,
    now: i64,
) -> Result<(), String> {
    let (next_state, event_kind) = match config.execution_policy {
        config::ExecutionPolicy::AlwaysApprove => (state::QUEUED, "queued"),
        config::ExecutionPolicy::RequireApproval => (state::PENDING_APPROVAL, "pending_approval"),
    };
    db::transition_state_with_runner_event(pool, task_id, next_state, now, event_kind, None)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}
