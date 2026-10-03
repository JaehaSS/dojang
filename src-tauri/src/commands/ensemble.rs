//! 앙상블 판정·인터뷰·그릴링 커맨드.
//!
//! `commands.rs`에서 갈라져 나왔다 — 모든 기능이 한 파일 끝에 줄을 붙이는
//! 구조가 병행 worktree의 머지 충돌을 만들었다.

use std::path::{Path, PathBuf};

use tauri::State;

use crate::db::{self, state as tstate, Task};

use super::{AppState, compose_kpi_detail, now, pool_of, rand_u64, task_hunks, worktree_from_task};

/// 앙상블 그룹의 후보 작업들 (비교 뷰).
#[tauri::command]
pub async fn ensemble_list(
    state: State<'_, AppState>,
    ensemble: String,
) -> Result<Vec<Task>, String> {
    let pool = pool_of(&state)?;
    db::tasks_by_ensemble(&pool, &ensemble)
        .await
        .map_err(|e| e.to_string())
}

/// 같은 ensemble 후보의 저장된 대화 이벤트와 memory usage를 읽기 전용으로 집계한다.
#[tauri::command]
pub async fn ensemble_metrics(
    state: State<'_, AppState>,
    ensemble: String,
) -> Result<Vec<crate::ensemble::CandidateBenchmarkMetrics>, String> {
    let pool = pool_of(&state)?;
    crate::ensemble::candidate_metrics(&pool, &ensemble)
        .await
        .map_err(|error| error.to_string())
}

/// 최근 비교 실행에서 실제 승인된 후보와 메모리 사용의 상관을 읽기 전용으로 집계한다.
#[tauri::command]
pub async fn ensemble_feedback_history(
    state: State<'_, AppState>,
) -> Result<crate::ensemble::EnsembleFeedbackHistory, String> {
    let pool = pool_of(&state)?;
    crate::ensemble::feedback_history(&pool)
        .await
        .map_err(|error| error.to_string())
}

/// 앙상블 교차검증 — 후보 diff들을 **독립 심판(후보에 없는 모델 우선)**이 비교해 최선 추천.
/// 후보 diff는 검토 콘텐츠일 뿐(인젝션 가드 + nonce 스푸핑 차단). 헤드리스 run_reviewer.
#[tauri::command]
pub async fn ensemble_judge(
    state: State<'_, AppState>,
    ensemble: String,
    judge_pref: String,
) -> Result<crate::ensemble::Judgment, String> {
    let pool = pool_of(&state)?;
    let all = db::tasks_by_ensemble(&pool, &ensemble)
        .await
        .map_err(|e| e.to_string())?;
    // 완료된 후보만 비교 — 실패/폐기/진행중은 심판 대상 아님(심판이 그것을 winner로 못 고르게).
    let candidates: Vec<db::Task> = all
        .into_iter()
        .filter(|t| matches!(t.state.as_str(), tstate::AWAITING_REVIEW | tstate::DONE))
        .collect();
    if candidates.len() < 2 {
        return Err(
            "비교할 완료된 후보가 2개 이상이어야 합니다 (자율수행 완료 대기 중일 수 있음)".into(),
        );
    }
    let instruction = candidates[0].instruction.clone();
    // 각 후보의 diff 텍스트 수집 (worktree에서 — 활성/비활성 무관, DB 레코드로 재구성).
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut valid_agents: Vec<String> = Vec::new();
    for t in &candidates {
        let label = t.agent.clone().unwrap_or_else(|| t.branch.clone());
        let wt = worktree_from_task(t);
        let diff = if wt.path.is_dir() {
            wt.diff_detailed()
                .map(|files| {
                    files
                        .iter()
                        .map(|f| f.patch.clone())
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
        valid_agents.push(label.clone());
        pairs.push((label, diff));
    }
    // 심판 모델: 지정값은 **알려진 리뷰어 모델 화이트리스트**에 있고 PATH에 있을 때만 채택
    // (임의 바이너리를 심판으로 못 쓰게). 아니면 후보에 없는 벤더 우선(자기심판 편향 회피).
    const JUDGE_MODELS: [&str; 3] = ["codex", "agy", "claude"];
    let pref = judge_pref.trim();
    let judge = if JUDGE_MODELS.contains(&pref) && crate::reviewer::on_path(pref) {
        pref.to_string()
    } else {
        let cand: std::collections::HashSet<&str> = candidates
            .iter()
            .filter_map(|t| t.agent.as_deref())
            .collect();
        JUDGE_MODELS
            .iter()
            .find(|m| !cand.contains(**m) && crate::reviewer::on_path(m))
            .map(|s| s.to_string())
            .unwrap_or_else(|| crate::reviewer::detect_reviewer(""))
    };
    // nonce: OS 난수 기반(추측 차단) — 악성 diff가 nonce를 맞혀 가짜 심판결과를 스푸핑 못하게.
    let nonce = format!("PRAXIS-JUDGE-{}-{:016x}", std::process::id(), rand_u64());
    let prompt = crate::ensemble::build_judge_prompt(&instruction, &pairs, &nonce);
    let nonce_c = nonce.clone();
    let raw = tauri::async_runtime::spawn_blocking(move || {
        crate::reviewer::run_reviewer(&judge, &prompt, 180)
    })
    .await
    .map_err(|e| e.to_string())??;
    crate::ensemble::parse_judgment(&raw, &nonce_c, &valid_agents)
        .ok_or_else(|| "심판 결과 파싱 실패 (nonce 이후 유효 JSON 없음)".to_string())
}

/// 인터뷰 1차 — 레포 요약을 근거로 지시문 명확도를 채점하고 부족한 차원의 질문을 생성한다.
/// 전 차원이 명확하고 종합 점수가 낮으면 질문을 비워 반환하고, 프론트가 즉시
/// `interview_crystallize`를 연쇄 호출한다(Plan 0021 DR-P2). 소프트 게이트 — 작업 생성을 막지 않는다.
#[tauri::command]
pub async fn interview_start(
    repo: String,
    instruction: String,
    agent: Option<String>,
) -> Result<crate::interview::InterviewAssessment, String> {
    let vendor = crate::reviewer::detect_reviewer(agent.as_deref().unwrap_or(""));
    let ctx = crate::interview::collect_repo_context(Path::new(&repo))?;
    // nonce: OS 난수 기반 — 레포 콘텐츠(README 등)에 심긴 위조 JSON 스푸핑 차단.
    let nonce = format!(
        "PRAXIS-INTERVIEW-{}-{:016x}",
        std::process::id(),
        rand_u64()
    );
    let prompt = crate::interview::build_assessment_prompt(&ctx, &instruction, &nonce);
    let raw = tauri::async_runtime::spawn_blocking(move || {
        crate::reviewer::run_reviewer(&vendor, &prompt, crate::interview::INTERVIEW_TIMEOUT_SECS)
    })
    .await
    .map_err(|e| e.to_string())??;
    crate::interview::finalize_assessment(&raw, &nonce)
}

/// 인터뷰 2차 — 답변을 반영해 재채점하고 Goal Contract 초안 필드를 결정화한다.
/// 결과는 편집 가능한 드래프트일 뿐이며 계약 저장·검증은 기존 `task_create` 경로가 수행한다.
#[tauri::command]
pub async fn interview_crystallize(
    repo: String,
    instruction: String,
    answers: Vec<crate::interview::InterviewAnswer>,
    agent: Option<String>,
) -> Result<crate::interview::CrystallizeResult, String> {
    let vendor = crate::reviewer::detect_reviewer(agent.as_deref().unwrap_or(""));
    let ctx = crate::interview::collect_repo_context(Path::new(&repo))?;
    let nonce = format!(
        "PRAXIS-INTERVIEW-{}-{:016x}",
        std::process::id(),
        rand_u64()
    );
    let prompt = crate::interview::build_crystallize_prompt(&ctx, &instruction, &answers, &nonce);
    let raw = tauri::async_runtime::spawn_blocking(move || {
        crate::reviewer::run_reviewer(&vendor, &prompt, crate::interview::INTERVIEW_TIMEOUT_SECS)
    })
    .await
    .map_err(|e| e.to_string())??;
    crate::interview::parse_crystallize(&raw, &nonce)
}

/// 발산 인터뷰 한 라운드(설계 0026). 질문 1개 + 추천 답 + 미해결 논점을 돌려준다.
/// transcript 길이가 상한(11)에 닿으면 파서가 모델 판정과 무관하게 질문을 끊는다.
#[tauri::command]
pub async fn grill_round(
    repo: String,
    instruction: String,
    transcript: Vec<crate::interview::grill::GrillTurn>,
    agent: Option<String>,
) -> Result<crate::interview::grill::GrillRound, String> {
    let vendor = crate::reviewer::detect_reviewer(agent.as_deref().unwrap_or(""));
    let ctx = crate::interview::collect_repo_context(Path::new(&repo))?;
    let nonce = format!("PRAXIS-GRILL-{}-{:016x}", std::process::id(), rand_u64());
    let len = transcript.len();
    let prompt =
        crate::interview::grill::build_round_prompt(&ctx, &instruction, &transcript, &nonce);
    let raw = tauri::async_runtime::spawn_blocking(move || {
        crate::reviewer::run_reviewer(&vendor, &prompt, crate::interview::INTERVIEW_TIMEOUT_SECS)
    })
    .await
    .map_err(|e| e.to_string())??;
    crate::interview::grill::parse_round(&raw, &nonce, len)
}

/// 인터뷰 종료 후 노트 생성. 호출 상한 12회(라운드 11 + 노트 1) 중 마지막 1회다.
#[tauri::command]
pub async fn grill_note(
    repo: String,
    instruction: String,
    transcript: Vec<crate::interview::grill::GrillTurn>,
    agent: Option<String>,
) -> Result<crate::interview::grill::GrillNote, String> {
    let vendor = crate::reviewer::detect_reviewer(agent.as_deref().unwrap_or(""));
    let ctx = crate::interview::collect_repo_context(Path::new(&repo))?;
    let nonce = format!("PRAXIS-GRILL-{}-{:016x}", std::process::id(), rand_u64());
    let prompt =
        crate::interview::grill::build_note_prompt(&ctx, &instruction, &transcript, &nonce);
    let raw = tauri::async_runtime::spawn_blocking(move || {
        crate::reviewer::run_reviewer(&vendor, &prompt, crate::interview::INTERVIEW_TIMEOUT_SECS)
    })
    .await
    .map_err(|e| e.to_string())??;
    crate::interview::grill::parse_note(&raw, &nonce)
}

/// 노트를 레포에 저장한다. LLM을 호출하지 않으므로 12회 상한과 무관하다.
/// 저장은 사용자가 버튼을 눌렀을 때만 일어난다 — 자동 저장은 사용자 레포에 대한 부작용이다.
/// `date`를 프론트에서 받는 이유: 백엔드가 시스템 시계를 읽으면 결과가 시각에 의존한다.
#[tauri::command]
pub async fn grill_save_note(
    repo: String,
    slug: String,
    markdown: String,
    date: String,
) -> Result<String, String> {
    let repo_path = PathBuf::from(&repo);
    let target = crate::interview::grill::resolve_note_path(&repo_path, &date, &slug)?;
    std::fs::write(&target, markdown).map_err(|e| format!("노트 저장 실패: {e}"))?;
    // 표시는 레포 상대 경로로 — 절대 경로는 UI에서 길기만 하다.
    let shown = repo_path
        .canonicalize()
        .ok()
        .and_then(|root| target.strip_prefix(root).ok().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| target.clone());
    Ok(shown.to_string_lossy().to_string())
}

/// ensemble 후보×파일×hunk 매트릭스(B-3) — 서로 다른 후보의 겹치는 hunk를 배타 그룹으로 묶어
/// 반환한다(그룹 내 최대 1개만 선택 가능). UI 겹침 선택 차단·`ensemble_compose` 이중 방어의 기반.
#[tauri::command]
pub async fn ensemble_matrix(
    state: State<'_, AppState>,
    ensemble: String,
) -> Result<crate::ensemble::EnsembleMatrix, String> {
    let pool = pool_of(&state)?;
    let tasks = db::tasks_by_ensemble(&pool, &ensemble)
        .await
        .map_err(|e| e.to_string())?;
    let mut candidates = Vec::new();
    for t in &tasks {
        candidates.push((t.id, task_hunks(t)?));
    }
    Ok(crate::ensemble::matrix(&candidates))
}

/// ensemble 조합 병합(B-3): 심판 추천 후보(`winner_task_id`) worktree를 베이스로 타 후보의 선택
/// hunk를 체크포인트(`dojang: pre-compose`) 후 순차 forward 적용(`git apply --3way`, `ensemble::compose`
/// 재사용)한다. 충돌 시 실패 hunk 목록과 함께 fail-closed(체크포인트 원복). 커밋은 만들지 않는다
/// (승인은 기존 `task_approve` 경로 재사용). 성공 시 체크포인트를 영속해 기존 `partial_rollback`
/// 경로로 되돌릴 수 있게 한다(DR-P1: 롤백 재사용).
#[tauri::command]
pub async fn ensemble_compose(
    state: State<'_, AppState>,
    ensemble: String,
    winner_task_id: i64,
    selections: Vec<crate::ensemble::HunkRef>,
) -> Result<crate::ensemble::ComposeOutcome, String> {
    let pool = pool_of(&state)?;
    let tasks = db::tasks_by_ensemble(&pool, &ensemble)
        .await
        .map_err(|e| e.to_string())?;
    let winner = tasks
        .iter()
        .find(|t| t.id == winner_task_id)
        .ok_or("추천 후보를 찾을 수 없습니다")?;
    if winner.state != tstate::AWAITING_REVIEW {
        return Err("검토 대기 중인 후보에만 조합을 적용할 수 있습니다".into());
    }
    let mut candidates = Vec::new();
    for t in &tasks {
        candidates.push((t.id, task_hunks(t)?));
    }
    let worktree = worktree_from_task(winner);
    let result = crate::ensemble::compose(winner_task_id, &worktree, &candidates, &selections);
    let _ = db::append_event(
        &pool,
        winner_task_id,
        "ensemble_compose",
        Some(&compose_kpi_detail(&result)),
        now(),
    )
    .await;
    let outcome = result.map_err(|e| e.to_string())?;
    crate::partial::save_checkpoint(&pool, winner_task_id, &outcome.checkpoint, now())
        .await
        .map_err(|e| e.to_string())?;
    Ok(outcome)
}

