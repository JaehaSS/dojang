//! 멀티 벤더 파이프라인 드라이버. 3초마다 활성 실행을 훑어 `plan::plan_next_action`이 고른 일을 한 번에 하나씩 수행한다.
//! 모든 상태는 DB(`pipeline_*`)에 있으므로 앱을 다시 켜도 같은 지점에서 이어진다.
//! 블로킹 작업(git·검증 명령·벤더 CLI)은 전부 `spawn_blocking`에서 돌고, 자식 프로세스는 실행별 등록부에 올라 취소·종료 때 함께 죽는다.
//! 통합 worktree를 쓰는 곳은 이 파일뿐이다(에이전트는 티켓 worktree만 쓴다).
//! 설계 정본: `docs/plans/2026-10-01-multi-vendor-pipeline.md`.

use super::{
    cancel_task_inner, close_designmode_webview_of, close_shell_of, create_task_internal,
    is_cap_reached_error, is_direct_mode, now, pool_of, retire_projection_for_review,
    worktree_from_task, AppState,
};
use crate::db::{self, state as tstate};
use crate::goal_contract::{GoalContract, SCHEMA_VERSION};
use crate::managed_process::SharedProcessRegistrar;
use crate::multireview::verdict::{has_blocking, Finding, ReviewKind, Severity, SynthDecision, Synthesis};
use crate::multireview::{self};
use crate::orchestrator::{CreateTaskParams, TaskDraft, TaskOrigin, TaskService};
use crate::pipeline::db as pdb;
use crate::pipeline::db::{RunRow, StepStart, TicketRow};
use crate::pipeline::model::{self, Spec, SplitOutput, MAX_TICKETS};
use crate::pipeline::plan::{self, Action, RunCtx, TicketCtx};
use crate::pipeline::review_run::{self, ReviewerFn};
use crate::pipeline::state::{RunState, TicketState};
use crate::pipeline::steps::{self, IntegrationRecovery, VerifyOutput};
use crate::pipeline::{checks, git_ops, prompts, registry, split};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

type R<T> = Result<T, String>;

const TICK: Duration = Duration::from_secs(3);
const STARTUP_DELAY: Duration = Duration::from_secs(5);
/// 틱 하나에서 한 실행이 연달아 수행할 수 있는 동작 수의 상한(무한 루프 방어).
const MAX_ACTIONS_PER_TICK: usize = 60;
const SPLIT_TIMEOUT_SECS: u64 = 600;
const REVIEW_TIMEOUT_SECS: u64 = 300;
const FINAL_REVIEW_TIMEOUT_SECS: u64 = 600;
const CHANGED_EVENT: &str = "pipeline://changed";

fn e2s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> R<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(e2s)
}

/// 앱이 쓸 수 있는 벤더(PATH에 CLI가 있는 것). 순서는 claude, codex, agy로 고정이다.
pub(crate) fn available_vendors() -> Vec<String> {
    ["claude", "codex", "agy"]
        .into_iter()
        .filter(|v| crate::reviewer::on_path(v))
        .map(String::from)
        .collect()
}

pub(crate) fn emit_changed(app: &AppHandle, run_id: i64) {
    let _ = app.emit(CHANGED_EVENT, run_id);
}

// ---------- 루프 ----------

fn in_flight() -> &'static Mutex<HashSet<i64>> {
    static SET: OnceLock<Mutex<HashSet<i64>>> = OnceLock::new();
    SET.get_or_init(Default::default)
}

struct FlightGuard(i64);

impl FlightGuard {
    fn acquire(run_id: i64) -> Option<Self> {
        let inserted = in_flight().lock().unwrap().insert(run_id);
        // then_some은 값을 미리 만들어 거절된 경우에도 Drop이 남의 guard 항목을 지운다. 클로저로 지연한다.
        inserted.then(|| Self(run_id))
    }
}

impl Drop for FlightGuard {
    fn drop(&mut self) {
        in_flight().lock().unwrap().remove(&self.0);
    }
}

/// 앱 setup에서 한 번 띄우는 무한 루프.
pub(crate) async fn run_loop(app: AppHandle) {
    tokio::time::sleep(STARTUP_DELAY).await;
    loop {
        tick(&app).await;
        tokio::time::sleep(TICK).await;
    }
}

async fn tick(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Ok(pool) = pool_of(&state) else { return };
    let Ok(runs) = pdb::list_active_runs(&pool).await else { return };
    for run in runs {
        let Some(guard) = FlightGuard::acquire(run.id) else { continue };
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _guard = guard;
            drive_run(&app, run.id).await;
        });
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Progress,
    NoProgress,
}

struct Cx {
    app: AppHandle,
    pool: SqlitePool,
    run_id: i64,
    reg: Arc<registry::RunRegistrar>,
    registrar: SharedProcessRegistrar,
}

impl Cx {
    fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }
    fn emit(&self) {
        emit_changed(&self.app, self.run_id);
    }
    async fn run(&self) -> R<RunRow> {
        pdb::get_run(&self.pool, self.run_id)
            .await
            .map_err(e2s)?
            .ok_or_else(|| format!("파이프라인 실행 {}을(를) 찾을 수 없습니다", self.run_id))
    }
    async fn ticket(&self, id: i64) -> R<TicketRow> {
        pdb::get_ticket(&self.pool, id)
            .await
            .map_err(e2s)?
            .ok_or_else(|| format!("티켓 {id}을(를) 찾을 수 없습니다"))
    }
}

async fn drive_run(app: &AppHandle, run_id: i64) {
    let state = app.state::<AppState>();
    let Ok(pool) = pool_of(&state) else { return };
    let reg = registry::for_run(run_id);
    let cx = Cx {
        app: app.clone(),
        pool,
        run_id,
        registrar: reg.registrar(),
        reg,
    };
    for _ in 0..MAX_ACTIONS_PER_TICK {
        if cx.reg.is_cancelled() {
            return;
        }
        let Ok(run) = cx.run().await else { return };
        let Some(rs) = run.run_state() else { return };
        if rs.is_terminal() {
            return;
        }
        let action = match build_plan(&cx, &run, rs).await {
            Ok(a) => a,
            Err(e) => {
                eprintln!("[pipeline] 계획 수립 실패 run={run_id}: {e}");
                return;
            }
        };
        if action == Action::Idle {
            return;
        }
        match execute(&cx, &run, rs, action.clone()).await {
            Ok(Outcome::Progress) => cx.emit(),
            Ok(Outcome::NoProgress) => return,
            Err(e) => {
                if cx.reg.is_cancelled() {
                    return;
                }
                eprintln!("[pipeline] 동작 실패 run={run_id} action={action:?}: {e}");
                let reason = format!("내부 오류로 멈췄습니다: {e}");
                let _ = pdb::set_run_error(&cx.pool, run_id, Some(&reason), now()).await;
                let _ = pdb::pause_run(&cx.pool, run_id, rs, &reason, now()).await;
                cx.emit();
                return;
            }
        }
    }
}

/// 작업 상태 문자열을 `plan`이 쓰는 snake_case 키로 바꾼다.
fn task_state_key(state: &str) -> String {
    match state {
        tstate::AWAITING_REVIEW => "awaiting_review".into(),
        other => other.to_lowercase(),
    }
}

async fn build_plan(cx: &Cx, run: &RunRow, rs: RunState) -> R<Action> {
    let integration_task_state = match (rs, run.integration_task_id) {
        (RunState::AwaitingMergeApproval, Some(id)) => Some(
            db::get_task(&cx.pool, id)
                .await
                .map_err(e2s)?
                .map(|t| task_state_key(&t.state))
                .unwrap_or_else(|| "discarded".into()),
        ),
        _ => None,
    };
    let rctx = RunCtx {
        state: rs,
        has_integration: run.integration_task_id.is_some() && run.integration_path.is_some(),
        has_spec: run.spec_json.is_some(),
        has_feedback: run.feedback.as_deref().is_some_and(|f| !f.trim().is_empty()),
        integration_task_state,
    };
    let mut tickets = Vec::new();
    if rs == RunState::Executing {
        let alive: HashSet<i64> = cx
            .state()
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .copied()
            .collect();
        for t in pdb::list_tickets(&cx.pool, run.id).await.map_err(e2s)? {
            let Some(view) = t.view() else { continue };
            let task_state = match t.task_id {
                Some(id) => db::get_task(&cx.pool, id)
                    .await
                    .map_err(e2s)?
                    .map(|task| task_state_key(&task.state)),
                None => None,
            };
            tickets.push(TicketCtx {
                has_task: t.task_id.is_some(),
                agent_alive: t.task_id.is_some_and(|id| alive.contains(&id)),
                task_state,
                view,
            });
        }
    }
    Ok(plan::plan_next_action(&rctx, &tickets))
}

async fn execute(cx: &Cx, run: &RunRow, rs: RunState, action: Action) -> R<Outcome> {
    match action {
        Action::Idle => Ok(Outcome::NoProgress),
        Action::EnsureIntegration => ensure_integration(cx, run).await,
        Action::Split => do_split(cx, run).await,
        Action::AdvanceToPlanReview => {
            let avail = available_vendors();
            assign_reviewers(&cx.pool, run.id, &avail).await?;
            cas_run(cx, RunState::Drafting, RunState::PlanReview).await
        }
        Action::PlanReview => do_plan_review(cx, run).await,
        Action::Pause(reason) => {
            pdb::pause_run(&cx.pool, run.id, rs, &reason, now()).await.map_err(e2s)?;
            Ok(Outcome::Progress)
        }
        Action::Collect(id) => collect(cx, id).await,
        Action::Spawn(ids) => spawn_tickets(cx, run, ids).await,
        Action::Verify(id) => verify(cx, run, id).await,
        Action::Review(id) => review_ticket(cx, run, id).await,
        Action::Fix(id) => fix_ticket(cx, run, id).await,
        Action::Integrate(id) => integrate(cx, run, id).await,
        Action::ToFinalReview => cas_run(cx, RunState::Executing, RunState::FinalReview).await,
        Action::FinalReview => do_final_review(cx, run).await,
        Action::FinishRun(end) => {
            if end == RunState::Cancelled {
                pdb::cancel_run(&cx.pool, run.id, now()).await.map_err(e2s)?;
            } else {
                pdb::set_run_state(&cx.pool, run.id, RunState::AwaitingMergeApproval, end, now())
                    .await
                    .map_err(e2s)?;
            }
            Ok(Outcome::Progress)
        }
    }
}

async fn cas_run(cx: &Cx, from: RunState, to: RunState) -> R<Outcome> {
    let ok = pdb::set_run_state(&cx.pool, cx.run_id, from, to, now()).await.map_err(e2s)?;
    Ok(if ok { Outcome::Progress } else { Outcome::NoProgress })
}

async fn pause(cx: &Cx, from: RunState, reason: &str) -> R<Outcome> {
    pdb::pause_run(&cx.pool, cx.run_id, from, reason, now()).await.map_err(e2s)?;
    Ok(Outcome::Progress)
}

async fn fail_run(cx: &Cx, from: RunState, reason: &str) -> R<Outcome> {
    pdb::set_run_error(&cx.pool, cx.run_id, Some(reason), now()).await.map_err(e2s)?;
    pdb::set_run_state(&cx.pool, cx.run_id, from, RunState::Failed, now())
        .await
        .map_err(e2s)?;
    Ok(Outcome::Progress)
}

// ---------- 단계 기록 ----------

enum Step {
    Cached(pdb::StepRow),
    Run(i64),
}

async fn begin(
    cx: &Cx,
    ticket: Option<&TicketRow>,
    kind: &str,
    vendor: Option<&str>,
    token: u32,
    prompt: &str,
) -> R<Step> {
    let key = model::step_key(cx.run_id, kind, ticket.map(|t| t.key.as_str()), token);
    let started = pdb::begin_step(
        &cx.pool,
        cx.run_id,
        ticket.map(|t| t.id),
        kind,
        vendor,
        &key,
        prompt,
        now(),
    )
    .await
    .map_err(e2s)?;
    Ok(match started {
        StepStart::AlreadySucceeded(row) => Step::Cached(row),
        StepStart::Started(id) => Step::Run(id),
    })
}

async fn finish(cx: &Cx, step_id: i64, ok: bool, output: &str) -> R<()> {
    // 취소·앱 종료로 자식이 죽어 생긴 실패를 단계 실패로 기록하면 재시작 때 이어지지 못한다.
    // 단계를 running으로 남기면 begin이 그대로 재사용한다.
    if !ok && cx.reg.is_cancelled() {
        return Err("실행이 취소되었거나 앱이 종료되는 중이라 실패를 기록하지 않습니다".into());
    }
    pdb::finish_step(&cx.pool, step_id, ok, output, now()).await.map_err(e2s)
}

/// 티켓 단계 키의 세대 토큰. 재시도·재배정으로 attempt가 0으로 돌아가도 키가 겹치지 않는다.
fn token(t: &TicketRow) -> u32 {
    (t.gen.max(0) as u32) * 100 + t.attempt.max(0) as u32
}

// ---------- 리뷰 공통 ----------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct ReviewOutcome {
    review_id: i64,
    blocking: bool,
    /// 다음 수정 지시에 넣는 blocking 지적 텍스트.
    findings: String,
    blocking_findings: Vec<Finding>,
}

fn make_runner(registrar: SharedProcessRegistrar) -> Arc<ReviewerFn> {
    Arc::new(move |vendor, prompt, timeout| {
        crate::reviewer::run_reviewer_registered(vendor, prompt, timeout, Some(&registrar))
    })
}

fn blocking_of(s: &Synthesis) -> Vec<Finding> {
    s.findings
        .iter()
        .filter(|f| f.finding.severity == Severity::Blocking)
        .map(|f| f.finding.clone())
        .collect()
}

/// 가용 벤더 전원이 리뷰하고 Claude가 종합한다. 리뷰가 부족하면 `Ok(Err(사유))`를 돌려 호출자가 멈추게 한다.
async fn fanout_review(
    cx: &Cx,
    run: &RunRow,
    kind: ReviewKind,
    step: &str,
    focus: &str,
    content: &str,
    timeout: u64,
) -> R<Result<ReviewOutcome, String>> {
    let vendors = available_vendors();
    if vendors.len() < 2 {
        return Ok(Err(format!("사용 가능한 벤더가 {}개뿐이라 교차 리뷰를 할 수 없습니다", vendors.len())));
    }
    let runner = make_runner(cx.registrar.clone());
    let (f, c) = (focus.to_string(), content.to_string());
    let r2 = runner.clone();
    let (prompt, _nonce, reviews) =
        blocking(move || review_run::run_parallel_reviews(kind, &f, &c, &vendors, timeout, r2)).await?;
    let ok_count = reviews.iter().filter(|r| r.ok).count();
    if ok_count < 2 {
        let errs: Vec<String> = reviews
            .iter()
            .filter(|r| !r.ok)
            .map(|r| format!("{}: {}", r.vendor, r.error.clone().unwrap_or_default()))
            .collect();
        return Ok(Err(format!("성공한 리뷰가 {ok_count}개입니다(2개 이상 필요). {}", errs.join("; "))));
    }
    let (f, rs2) = (focus.to_string(), reviews.clone());
    let synth = blocking(move || review_run::synthesize(kind, &f, &rs2, runner, timeout)).await?;
    let (synth_prompt, _raw, synth) = match synth {
        Ok(v) => v,
        Err(e) => return Ok(Err(format!("리뷰 종합에 실패했습니다: {e}"))),
    };
    let record = review_run::build_review_record(&reviews, Some(&synth), &|_| String::new());
    let review_id = multireview::insert_pipeline_review(
        &cx.pool,
        &run.repo,
        "text",
        &format!("pipeline:{}:{step}", run.id),
        focus,
        &record.result_json,
        record.ok_count,
        record.total,
        content,
        &prompt,
        Some(&synth_prompt),
        &record.model_info_json,
        run.id,
        step,
        now(),
    )
    .await
    .map_err(e2s)?;
    let blocking_findings = blocking_of(&synth);
    Ok(Ok(ReviewOutcome {
        review_id,
        blocking: synth.decision == SynthDecision::Fix,
        findings: prompts::findings_text(&blocking_findings),
        blocking_findings,
    }))
}

fn parse_outcome(row: &pdb::StepRow) -> ReviewOutcome {
    row.output
        .as_deref()
        .and_then(|o| serde_json::from_str(o).ok())
        .unwrap_or_default()
}

// ---------- 초안: 통합 worktree와 분할 ----------

async fn ensure_integration(cx: &Cx, run: &RunRow) -> R<Outcome> {
    // 취소되거나 이미 종결된 실행에는 통합 worktree·작업을 만들지 않는다.
    if cx.reg.is_cancelled() || cx.run().await?.run_state() != Some(RunState::Drafting) {
        return Ok(Outcome::NoProgress);
    }
    let (repo, base, goal) = (run.repo.clone(), run.base_branch.clone(), run.goal.clone());
    let slug: String = crate::worktree::slugify(&goal).chars().take(24).collect();
    let branch = format!("dojang/pipe-{slug}-{}", crate::pipeline::review_run::random_u64() % 1_000_000_000);
    let b2 = branch.clone();
    let created = blocking(move || crate::worktree::create_plain(Path::new(&repo), &b2, Some(&base))).await?;
    let wt = match created {
        Ok(wt) => wt,
        Err(e) => return fail_run(cx, RunState::Drafting, &format!("통합 worktree 생성 실패: {e}")).await,
    };
    let draft = TaskDraft {
        repo: run.repo.clone(),
        branch: wt.branch.clone(),
        base: run.base_branch.clone(),
        worktree_path: wt.path.to_string_lossy().into_owned(),
        instruction: run.goal.clone(),
        agent: Some("claude".into()),
        role: crate::agent::DEFAULT_ROLE.to_string(),
        ensemble: None,
        mode: "terminal".into(),
        goal_contract: None,
        ambiguity: None,
    };
    let task = match TaskService::new(cx.pool.clone()).create_task(draft, None, None, now()).await {
        Ok(t) => t,
        Err(e) => {
            let _ = wt.discard();
            return Err(e);
        }
    };
    if let Some(rev) = &wt.base_revision {
        db::set_base_revision(&cx.pool, task.id, rev).await.map_err(e2s)?;
    }
    let claimed = pdb::claim_run_integration(
        &cx.pool,
        run.id,
        task.id,
        &wt.branch,
        &wt.path.to_string_lossy(),
        now(),
    )
    .await
    .map_err(e2s)?;
    if !claimed {
        // 만드는 동안 취소·종결됐다. 고아 작업과 브랜치를 남기지 않는다.
        close_pipeline_task(&cx.state(), task.id, Closure::Abandon).await;
        return Ok(Outcome::NoProgress);
    }
    Ok(Outcome::Progress)
}

async fn do_split(cx: &Cx, run: &RunRow) -> R<Outcome> {
    let path = PathBuf::from(run.integration_path.clone().unwrap_or_default());
    let avail = available_vendors();
    let mut feedback = run.feedback.clone().filter(|f| !f.trim().is_empty());
    let mut last_errors: Vec<String> = Vec::new();
    let mut result: Option<SplitOutput> = None;
    for attempt in 0..2u32 {
        let nonce = format!("PRAXIS-SPLIT-{:016x}", review_run::random_u64());
        let refs: Vec<&str> = avail.iter().map(String::as_str).collect();
        let prompt = split::build_split_prompt(&run.goal, &run.base_branch, &refs, &nonce, feedback.as_deref());
        let token = (run.spec_revision.max(0) as u32) * 10 + attempt;
        let step_id = match begin(cx, None, "split", Some(review_run::JUDGE_VENDOR), token, &prompt).await? {
            Step::Cached(row) => {
                if let Some(out) = row.output.as_deref().and_then(|o| serde_json::from_str::<SplitOutput>(o).ok()) {
                    result = Some(out);
                    break;
                }
                continue;
            }
            Step::Run(id) => id,
        };
        let head = {
            let p = path.clone();
            blocking(move || git_ops::head(&p)).await?.map_err(e2s)?
        };
        // 부트스트랩이 미추적 파일을 남겼을 수 있으므로 깨끗한지가 아니라 심판 전후가 같은지를 본다.
        let before = {
            let p = path.clone();
            blocking(move || git_ops::status_porcelain(&p)).await?.map_err(e2s)?
        };
        let (p, pr, reg) = (path.clone(), prompt.clone(), cx.registrar.clone());
        let raw = blocking(move || crate::reviewer::run_claude_readonly_in(&p, &pr, SPLIT_TIMEOUT_SECS, Some(&reg))).await?;
        // R4: 읽기 전용 심판이 통합 worktree를 건드렸으면 실행을 실패시킨다.
        let p = path.clone();
        let after = blocking(move || git_ops::status_porcelain(&p)).await?;
        let changed = match &after {
            Ok(after) => steps::tree_changed_during(&before, after),
            Err(_) => true,
        };
        if changed {
            if before.trim().is_empty() {
                // 원래 깨끗했다면 심판이 남긴 것을 모두 되돌린다. 부트스트랩 파일이 있었다면 지우지 않는다.
                let p = path.clone();
                let _ = blocking(move || git_ops::reset_to(&p, &head)).await;
            }
            finish(cx, step_id, false, "분할 심판이 통합 worktree를 변경했습니다").await?;
            return fail_run(cx, RunState::Drafting, "분할 단계에서 읽기 전용 심판이 저장소를 변경해 실행을 중단했습니다").await;
        }
        let raw = match raw {
            Ok(r) => r,
            Err(e) => {
                finish(cx, step_id, false, &e).await?;
                last_errors = vec![format!("분할 호출 실패: {e}")];
                continue;
            }
        };
        let out = match split::parse_split_output(&raw, &nonce) {
            Ok(o) => o,
            Err(e) => {
                finish(cx, step_id, false, &e).await?;
                last_errors = vec![format!("출력 파싱 실패: {e}")];
                feedback = Some(append_errors(feedback.as_deref(), &last_errors));
                continue;
            }
        };
        let refs: Vec<&str> = avail.iter().map(String::as_str).collect();
        match model::validate_split(&out, &refs, MAX_TICKETS) {
            Ok(()) => {
                finish(cx, step_id, true, &serde_json::to_string(&out).map_err(e2s)?).await?;
                result = Some(out);
                break;
            }
            Err(errors) => {
                finish(cx, step_id, false, &errors.join("\n")).await?;
                feedback = Some(append_errors(feedback.as_deref(), &errors));
                last_errors = errors;
            }
        }
    }
    let Some(out) = result else {
        return fail_run(cx, RunState::Drafting, &format!("분할 결과를 얻지 못했습니다: {}", last_errors.join("; "))).await;
    };
    let spec_json = serde_json::to_string(&out.spec).map_err(e2s)?;
    pdb::commit_split(&cx.pool, run.id, &spec_json, &out.tickets, now()).await.map_err(e2s)?;
    Ok(Outcome::Progress)
}

fn append_errors(prev: Option<&str>, errors: &[String]) -> String {
    let list = errors.iter().map(|e| format!("- {e}")).collect::<Vec<_>>().join("\n");
    match prev {
        Some(p) if !p.trim().is_empty() => format!("{p}\n\n직전 분할 출력의 오류(모두 고쳐서 다시 출력하라):\n{list}"),
        _ => format!("직전 분할 출력의 오류(모두 고쳐서 다시 출력하라):\n{list}"),
    }
}

/// 작성자와 같거나 가용하지 않은 리뷰어를 바로잡는다. 이미 올바른 배정은 건드리지 않는다.
pub(crate) async fn assign_reviewers(pool: &SqlitePool, run_id: i64, avail: &[String]) -> R<()> {
    let mut counts = pdb::reviewer_counts(pool, run_id).await.map_err(e2s)?;
    let refs: Vec<&str> = avail.iter().map(String::as_str).collect();
    for t in pdb::list_tickets(pool, run_id).await.map_err(e2s)? {
        if t.ticket_state().is_some_and(|s| s.is_terminal()) {
            continue;
        }
        let valid = t.reviewer_vendor.as_deref().is_some_and(|r| {
            model::normalize_vendor(r) != model::normalize_vendor(&t.vendor) && avail.iter().any(|a| a == r)
        });
        if valid {
            continue;
        }
        if let Some(old) = &t.reviewer_vendor {
            if let Some(c) = counts.get_mut(old) {
                *c = c.saturating_sub(1);
            }
        }
        let pick = model::pick_reviewer(&t.vendor, &refs, &counts);
        if let Some(r) = &pick {
            *counts.entry(r.clone()).or_insert(0) += 1;
        }
        pdb::set_ticket_reviewer(pool, t.id, pick.as_deref(), now()).await.map_err(e2s)?;
    }
    Ok(())
}

// ---------- 계획 리뷰 ----------

fn plan_text(run: &RunRow, tickets: &[TicketRow]) -> R<String> {
    let spec: Spec = run
        .spec_json
        .as_deref()
        .map(|j| serde_json::from_str(j).map_err(e2s))
        .transpose()?
        .unwrap_or_default();
    let out = SplitOutput { spec, tickets: tickets.iter().map(prompts::draft_from_row).collect() };
    Ok(split::render_spec_text(&out))
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

async fn do_plan_review(cx: &Cx, run: &RunRow) -> R<Outcome> {
    let tickets = pdb::list_tickets(&cx.pool, run.id).await.map_err(e2s)?;
    let text = plan_text(run, &tickets)?;
    let focus = format!("목표: {}", clip(&run.goal, 300));
    let step_id = match begin(cx, None, "plan_review", Some(review_run::JUDGE_VENDOR), run.spec_revision.max(0) as u32, "계획 리뷰").await? {
        Step::Cached(row) => return plan_review_decision(cx, run, parse_outcome(&row)).await,
        Step::Run(id) => id,
    };
    match fanout_review(cx, run, ReviewKind::Plan, "plan_review", &focus, &text, REVIEW_TIMEOUT_SECS).await? {
        Err(reason) => {
            finish(cx, step_id, false, &reason).await?;
            pause(cx, RunState::PlanReview, &reason).await
        }
        Ok(outcome) => {
            pdb::set_run_plan_review(&cx.pool, run.id, outcome.review_id, now()).await.map_err(e2s)?;
            finish(cx, step_id, true, &serde_json::to_string(&outcome).map_err(e2s)?).await?;
            plan_review_decision(cx, run, outcome).await
        }
    }
}

async fn plan_review_decision(cx: &Cx, run: &RunRow, outcome: ReviewOutcome) -> R<Outcome> {
    if outcome.blocking && run.plan_fix_used == 0 {
        pdb::set_run_plan_fix_used(&cx.pool, run.id, true, now()).await.map_err(e2s)?;
        pdb::set_run_feedback(
            &cx.pool,
            run.id,
            Some(&format!("계획 리뷰가 아래 blocking 지적을 냈다. 모두 반영해 다시 분할하라.\n{}", outcome.findings)),
            now(),
        )
        .await
        .map_err(e2s)?;
        return cas_run(cx, RunState::PlanReview, RunState::Drafting).await;
    }
    cas_run(cx, RunState::PlanReview, RunState::AwaitingPlanApproval).await
}

// ---------- 실행: 티켓 ----------

fn goal_contract_for(t: &TicketRow) -> Option<GoalContract> {
    let trim = |s: &str| clip(s, 1500);
    let gc = GoalContract {
        schema_version: SCHEMA_VERSION,
        objective: trim(&format!("{}: {}", t.key, t.title)),
        acceptance: t.acceptance().iter().take(30).map(|a| trim(a)).collect(),
        stop_conditions: Vec::new(),
        must_preserve: Vec::new(),
        protected_paths: Vec::new(),
        non_goals: Vec::new(),
    };
    gc.validate().ok().map(|_| gc)
}

async fn spawn_tickets(cx: &Cx, run: &RunRow, ids: Vec<i64>) -> R<Outcome> {
    let spec: Spec = run.spec_json.as_deref().and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default();
    let (Some(branch), Some(path)) = (run.integration_branch.clone(), run.integration_path.clone()) else {
        return Err("통합 worktree가 없습니다".into());
    };
    let mut spawned = 0;
    for id in ids {
        if cx.reg.is_cancelled() {
            break;
        }
        let t = cx.ticket(id).await?;
        let state = t.ticket_state();
        if !matches!(state, Some(TicketState::Pending | TicketState::Running)) {
            continue;
        }
        // 이전 시도가 남긴 작업이 있으면 먼저 치운다(spawn 중 죽은 경우).
        if let Some(old) = t.task_id {
            close_pipeline_task(&cx.state(), old, Closure::Abandon).await;
            pdb::bump_ticket_gen(&cx.pool, t.id, now()).await.map_err(e2s)?;
        }
        let head = {
            let p = PathBuf::from(&path);
            blocking(move || git_ops::head(&p)).await?.map_err(e2s)?
        };
        let draft = prompts::draft_from_row(&t);
        let agent = model::normalize_vendor(&t.vendor);
        let mut params = CreateTaskParams::headless_terminal(
            run.repo.clone(),
            prompts::ticket_instruction(&spec, &draft),
            agent,
            TaskOrigin::Pipeline,
        );
        params.base_branch = Some(branch.clone());
        params.goal_contract = goal_contract_for(&t);
        let created = create_task_internal(&cx.app, &cx.state(), params).await;
        let task = match created {
            Ok(task) => task,
            Err(e) if is_cap_reached_error(&e) => break,
            Err(e) => return Err(format!("티켓 {} 작업 생성 실패: {e}", t.key)),
        };
        if cx.reg.is_cancelled() || cx.run().await.map(|r| r.run_state() != Some(RunState::Executing)).unwrap_or(true) {
            // 생성 중에 실행이 멈추거나 취소됐다. 만들어진 작업을 남기지 않는다.
            close_pipeline_task(&cx.state(), task.id, Closure::Abandon).await;
            break;
        }
        pdb::set_ticket_task(&cx.pool, t.id, Some(task.id), now()).await.map_err(e2s)?;
        pdb::set_ticket_spawn_head(&cx.pool, t.id, Some(&head), now()).await.map_err(e2s)?;
        if state == Some(TicketState::Pending) {
            pdb::set_ticket_state(&cx.pool, t.id, TicketState::Pending, TicketState::Running, now())
                .await
                .map_err(e2s)?;
        }
        spawned += 1;
        cx.emit();
    }
    Ok(if spawned > 0 { Outcome::Progress } else { Outcome::NoProgress })
}

/// 투영된 메모리 블록과 자동 생성된 `.mcp.json`을 걷어낸다 — 둘 다 티켓 변경으로 오인되면 안 된다.
async fn scrub_task_artifacts(pool: &SqlitePool, task_id: i64) {
    let _ = retire_projection_for_review(pool, task_id).await;
    if db::has_task_event(pool, task_id, "mcp_generated").await.unwrap_or(false) {
        if let Ok(Some(task)) = db::get_task(pool, task_id).await {
            let _ = std::fs::remove_file(Path::new(&task.worktree_path).join(".mcp.json"));
        }
    }
}

async fn collect(cx: &Cx, id: i64) -> R<Outcome> {
    let t = cx.ticket(id).await?;
    if let Some(task_id) = t.task_id {
        scrub_task_artifacts(&cx.pool, task_id).await;
        let task = db::get_task(&cx.pool, task_id).await.map_err(e2s)?;
        if task.as_ref().is_none_or(|k| k.state == tstate::FAILED) {
            pdb::set_ticket_error(&cx.pool, t.id, Some("에이전트가 정상 종료하지 못했습니다"), now())
                .await
                .map_err(e2s)?;
        }
    }
    let ok = pdb::set_ticket_state(&cx.pool, t.id, TicketState::Running, TicketState::Verifying, now())
        .await
        .map_err(e2s)?;
    Ok(if ok { Outcome::Progress } else { Outcome::NoProgress })
}

struct TicketDir {
    path: PathBuf,
    branch: String,
}

async fn ticket_dir(cx: &Cx, t: &TicketRow) -> Option<TicketDir> {
    let task = db::get_task(&cx.pool, t.task_id?).await.ok()??;
    let path = PathBuf::from(&task.worktree_path);
    path.is_dir().then_some(TicketDir { path, branch: task.branch })
}

async fn verify(cx: &Cx, _run: &RunRow, id: i64) -> R<Outcome> {
    let t = cx.ticket(id).await?;
    let step_id = match begin(cx, Some(&t), "verify", None, token(&t), "허용 경로 검사와 검증 명령").await? {
        Step::Cached(row) => {
            let out: VerifyOutput = row.output.as_deref().and_then(|o| serde_json::from_str(o).ok()).unwrap_or_default();
            return verify_decision(cx, &t, out).await;
        }
        Step::Run(id) => id,
    };
    let out = match (ticket_dir(cx, &t).await, t.spawn_head.clone()) {
        (Some(dir), Some(fork)) => {
            let (allowed, acceptance, reg) = (t.allowed_paths(), t.acceptance(), cx.registrar.clone());
            // 에이전트 작업을 커밋한 뒤 커밋된 diff로 경로를 판정하고, 검증 산출물은 마지막에 버린다.
            blocking(move || steps::verify_ticket(&dir.path, &fork, &allowed, &acceptance, Some(&reg)))
                .await?
                .unwrap_or_else(|e| VerifyOutput { passed: false, summary: format!("검증을 수행하지 못했습니다: {e}") })
        }
        _ => VerifyOutput { passed: false, summary: "티켓 worktree 또는 기준 커밋을 찾을 수 없습니다".into() },
    };
    finish(cx, step_id, out.passed, &serde_json::to_string(&out).map_err(e2s)?).await?;
    verify_decision(cx, &t, out).await
}

async fn verify_decision(cx: &Cx, t: &TicketRow, out: VerifyOutput) -> R<Outcome> {
    if out.passed {
        pdb::set_ticket_error(&cx.pool, t.id, None, now()).await.map_err(e2s)?;
        pdb::set_ticket_state(&cx.pool, t.id, TicketState::Verifying, TicketState::Reviewing, now())
            .await
            .map_err(e2s)?;
    } else {
        pdb::set_ticket_error(&cx.pool, t.id, Some(&out.summary), now()).await.map_err(e2s)?;
        pdb::set_ticket_state(&cx.pool, t.id, TicketState::Verifying, TicketState::Fixing, now())
            .await
            .map_err(e2s)?;
    }
    Ok(Outcome::Progress)
}

/// 티켓을 에스컬레이션하고 실행을 멈춘다(R13).
async fn escalate(cx: &Cx, t: &TicketRow, from: TicketState, reason: &str) -> R<Outcome> {
    pdb::set_ticket_error(&cx.pool, t.id, Some(reason), now()).await.map_err(e2s)?;
    pdb::set_ticket_state(&cx.pool, t.id, from, TicketState::Escalated, now())
        .await
        .map_err(e2s)?;
    pdb::pause_run(
        &cx.pool,
        cx.run_id,
        RunState::Executing,
        &format!("티켓 {}: {reason}", t.key),
        now(),
    )
    .await
    .map_err(e2s)?;
    Ok(Outcome::Progress)
}

async fn review_ticket(cx: &Cx, run: &RunRow, id: i64) -> R<Outcome> {
    let mut t = cx.ticket(id).await?;
    let (Some(dir), Some(fork)) = (ticket_dir(cx, &t).await, t.spawn_head.clone()) else {
        return escalate(cx, &t, TicketState::Reviewing, "리뷰할 티켓 worktree를 찾을 수 없습니다").await;
    };
    let diff = blocking(move || git_ops::diff_text(&dir.path, &fork)).await?.map_err(e2s)?;
    let content = format!(
        "티켓 {}: {}\n{}\n허용 경로: {}\n수용 명령: {}\n\n{}",
        t.key,
        t.title,
        t.body,
        t.allowed_paths().join(", "),
        t.acceptance().join(" ; "),
        diff
    );
    let focus = format!("티켓 {} 수용 기준 충족 여부", t.key);
    let avail = available_vendors();
    let mut tried: Vec<String> = Vec::new();
    for attempt in 0..2u32 {
        let author = model::normalize_vendor(&t.vendor);
        let reviewer = match t.reviewer_vendor.clone().filter(|r| {
            model::normalize_vendor(r) != author && avail.contains(r) && !tried.contains(r)
        }) {
            Some(r) => r,
            None => {
                let refs: Vec<&str> = avail.iter().map(String::as_str).filter(|v| !tried.iter().any(|x| x == v)).collect();
                match model::pick_reviewer(&author, &refs, &HashMap::new()) {
                    Some(r) => r,
                    None => return escalate(cx, &t, TicketState::Reviewing, "리뷰를 맡을 수 있는 다른 벤더가 없습니다").await,
                }
            }
        };
        if t.reviewer_vendor.as_deref() != Some(&reviewer) {
            pdb::set_ticket_reviewer(&cx.pool, t.id, Some(&reviewer), now()).await.map_err(e2s)?;
            t.reviewer_vendor = Some(reviewer.clone());
        }
        let step_id = match begin(cx, Some(&t), "ticket_review", Some(&reviewer), token(&t) * 10 + attempt, "티켓 리뷰").await? {
            Step::Cached(row) => return review_decision(cx, &t, parse_outcome(&row)).await,
            Step::Run(sid) => sid,
        };
        let runner = make_runner(cx.registrar.clone());
        let (f, c, r) = (focus.clone(), content.clone(), reviewer.clone());
        let (prompt, vr) =
            blocking(move || review_run::single_review(ReviewKind::Ticket, &f, &c, &r, REVIEW_TIMEOUT_SECS, runner)).await?;
        if !vr.ok {
            finish(cx, step_id, false, vr.error.as_deref().unwrap_or("리뷰 실패")).await?;
            tried.push(reviewer);
            continue;
        }
        let record = review_run::build_review_record(std::slice::from_ref(&vr), None, &|_| String::new());
        let review_id = multireview::insert_pipeline_review(
            &cx.pool,
            &run.repo,
            "text",
            &format!("pipeline:{}:ticket_review:{}", run.id, t.key),
            &focus,
            &record.result_json,
            record.ok_count,
            record.total,
            &content,
            &prompt,
            None,
            &record.model_info_json,
            run.id,
            &format!("ticket_review:{}", t.key),
            now(),
        )
        .await
        .map_err(e2s)?;
        let blocking_findings: Vec<Finding> =
            vr.findings.iter().filter(|f| f.severity == Severity::Blocking).cloned().collect();
        let outcome = ReviewOutcome {
            review_id,
            blocking: has_blocking(&vr.findings),
            findings: prompts::findings_text(&blocking_findings),
            blocking_findings,
        };
        finish(cx, step_id, true, &serde_json::to_string(&outcome).map_err(e2s)?).await?;
        return review_decision(cx, &t, outcome).await;
    }
    escalate(cx, &t, TicketState::Reviewing, "리뷰어가 두 번 모두 응답하지 못했습니다").await
}

async fn review_decision(cx: &Cx, t: &TicketRow, outcome: ReviewOutcome) -> R<Outcome> {
    if outcome.blocking {
        pdb::set_ticket_error(&cx.pool, t.id, Some(&outcome.findings), now()).await.map_err(e2s)?;
        pdb::set_ticket_state(&cx.pool, t.id, TicketState::Reviewing, TicketState::Fixing, now())
            .await
            .map_err(e2s)?;
    } else {
        pdb::set_ticket_error(&cx.pool, t.id, None, now()).await.map_err(e2s)?;
        pdb::set_ticket_state(&cx.pool, t.id, TicketState::Reviewing, TicketState::Ready, now())
            .await
            .map_err(e2s)?;
    }
    Ok(Outcome::Progress)
}

async fn fix_ticket(cx: &Cx, _run: &RunRow, id: i64) -> R<Outcome> {
    let t = cx.ticket(id).await?;
    match model::fix_decision(t.attempt.max(0) as u32, t.reassigned != 0) {
        model::FixDecision::RetrySameVendor => {
            let problem = t.last_error.clone().unwrap_or_else(|| "검증 또는 리뷰가 실패했다".into());
            let prompt = prompts::ticket_fix_prompt(&prompts::draft_from_row(&t), &problem);
            let step_id = match begin(cx, Some(&t), "ticket_fix", Some(&t.vendor), token(&t), &prompt).await? {
                Step::Cached(_) => None,
                Step::Run(sid) => Some(sid),
            };
            if let Some(sid) = step_id {
                let result = match ticket_dir(cx, &t).await {
                    Some(dir) => {
                        let (vendor, reg) = (model::normalize_vendor(&t.vendor), cx.registrar.clone());
                        blocking(move || crate::reviewer::run_repair_agent(&dir.path, &vendor, None, None, &prompt, &reg)).await?
                    }
                    None => Err("티켓 worktree를 찾을 수 없습니다".to_string()),
                };
                match &result {
                    Ok(out) => finish(cx, sid, true, &clip(out, 4000)).await?,
                    Err(e) => {
                        finish(cx, sid, false, e).await?;
                        pdb::set_ticket_error(&cx.pool, t.id, Some(&format!("{problem}\n수정 에이전트 오류: {e}")), now())
                            .await
                            .map_err(e2s)?;
                    }
                }
            }
            pdb::set_ticket_attempt(&cx.pool, t.id, t.attempt.max(0) as u32 + 1, now()).await.map_err(e2s)?;
            pdb::set_ticket_state(&cx.pool, t.id, TicketState::Fixing, TicketState::Verifying, now())
                .await
                .map_err(e2s)?;
            Ok(Outcome::Progress)
        }
        model::FixDecision::Reassign => {
            let avail = available_vendors();
            let refs: Vec<&str> = avail.iter().map(String::as_str).collect();
            let mut counts: HashMap<String, u32> = HashMap::new();
            let all = pdb::list_tickets(&cx.pool, t.run_id).await.map_err(e2s)?;
            for o in &all {
                *counts.entry(model::normalize_vendor(&o.vendor)).or_insert(0) += 1;
            }
            let Some(new_vendor) = model::pick_reassign_vendor(&t.vendor, &refs, &counts) else {
                return escalate(cx, &t, TicketState::Fixing, "재배정할 다른 벤더가 없습니다").await;
            };
            if let Ok(Step::Run(sid)) = begin(cx, Some(&t), "ticket_fix", Some(&new_vendor), token(&t), "벤더 재배정").await {
                finish(cx, sid, true, &format!("재배정: {} -> {new_vendor}", t.vendor)).await?;
            }
            if let Some(old) = t.task_id {
                close_pipeline_task(&cx.state(), old, Closure::Abandon).await;
            }
            pdb::set_ticket_vendor(&cx.pool, t.id, &new_vendor, true, 0, now()).await.map_err(e2s)?;
            pdb::bump_ticket_gen(&cx.pool, t.id, now()).await.map_err(e2s)?;
            assign_reviewers(&cx.pool, t.run_id, &avail).await?;
            // Fixing -> Running: 작업이 없는 Running 티켓은 다음 동작에서 다시 spawn된다.
            pdb::set_ticket_state(&cx.pool, t.id, TicketState::Fixing, TicketState::Running, now())
                .await
                .map_err(e2s)?;
            Ok(Outcome::Progress)
        }
        model::FixDecision::Escalate => {
            let reason = format!("자동 수정 한도 초과: {}", clip(t.last_error.as_deref().unwrap_or(""), 600));
            escalate(cx, &t, TicketState::Fixing, &reason).await
        }
    }
}

// ---------- 통합 ----------

fn union_paths(tickets: &[TicketRow], include: impl Fn(&TicketRow) -> bool) -> Vec<String> {
    let mut v: Vec<String> = tickets.iter().filter(|t| include(t)).flat_map(|t| t.allowed_paths()).collect();
    v.sort();
    v.dedup();
    v
}

async fn integrate(cx: &Cx, run: &RunRow, id: i64) -> R<Outcome> {
    let t = cx.ticket(id).await?;
    let spec: Spec = run.spec_json.as_deref().and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default();
    let integration = PathBuf::from(run.integration_path.clone().ok_or("통합 worktree가 없습니다")?);
    let Some(dir) = ticket_dir(cx, &t).await else {
        let from = t.ticket_state().unwrap_or(TicketState::Ready);
        if from == TicketState::Ready {
            pdb::set_ticket_state(&cx.pool, t.id, TicketState::Ready, TicketState::Integrating, now()).await.map_err(e2s)?;
        }
        return escalate(cx, &t, TicketState::Integrating, "통합할 티켓 worktree를 찾을 수 없습니다").await;
    };
    let recovering = t.ticket_state() == Some(TicketState::Integrating);
    // 재시작으로 중단된 통합이면 기록해 둔 통합 전 head로 되돌리고 처음부터 다시 한다.
    let key = model::step_key(run.id, "integrate", Some(&t.key), token(&t));
    let recorded = pdb::list_steps(&cx.pool, run.id)
        .await
        .map_err(e2s)?
        .into_iter()
        .find(|s| s.step_key == key);
    let step_status = recorded.as_ref().map(|s| s.status.clone());
    let recorded_pre = recorded.and_then(|s| s.prompt).and_then(|p| steps::recorded_pre(&p));
    let pre = match steps::integration_recovery(recovering, step_status.as_deref(), recorded_pre) {
        // 통합은 끝났고 티켓 상태 전이만 빠졌다. 합쳐진 결과를 되돌리지 않는다.
        IntegrationRecovery::FinishTransition => return finish_integration(cx, &t).await,
        IntegrationRecovery::ResetTo(p) => {
            let (i, p2) = (integration.clone(), p.clone());
            blocking(move || git_ops::reset_to(&i, &p2)).await?.map_err(e2s)?;
            p
        }
        IntegrationRecovery::FromHead => {
            let i = integration.clone();
            blocking(move || git_ops::head(&i)).await?.map_err(e2s)?
        }
    };
    if !recovering {
        pdb::set_ticket_state(&cx.pool, t.id, TicketState::Ready, TicketState::Integrating, now())
            .await
            .map_err(e2s)?;
    }
    let step_id = match begin(cx, Some(&t), "integrate", None, token(&t), &format!("pre:{pre}")).await? {
        Step::Cached(_) => {
            // 이미 통합이 끝났는데 상태 전이만 빠졌다.
            return finish_integration(cx, &t).await;
        }
        Step::Run(sid) => sid,
    };
    let fail = |reason: String| {
        let (i, pre) = (integration.clone(), pre.clone());
        async move {
            let _ = blocking(move || git_ops::reset_to(&i, &pre)).await;
            reason
        }
    };
    let message = format!("통합: {} {}", t.key, t.title);
    let (i, d, b, m) = (integration.clone(), dir.path.clone(), dir.branch.clone(), message);
    let merged = blocking(move || -> Result<git_ops::MergeOutcome, String> {
        git_ops::commit_all(&d, &format!("{} (파이프라인 티켓)", m)).map_err(e2s)?;
        git_ops::merge_branch(&i, &b, &m).map_err(e2s)
    })
    .await?;
    let merged = match merged {
        Ok(m) => m,
        Err(e) => {
            let reason = fail(format!("merge 실패: {e}")).await;
            finish(cx, step_id, false, &reason).await?;
            return escalate(cx, &t, TicketState::Integrating, &reason).await;
        }
    };
    if let git_ops::MergeOutcome::Conflict { paths } = merged {
        let prompt = prompts::conflict_prompt(&t.key, &dir.branch, &paths, &spec);
        let cstep = begin(cx, Some(&t), "conflict", Some("claude"), token(&t), &prompt).await?;
        let (i, reg) = (integration.clone(), cx.registrar.clone());
        let resolved = blocking(move || -> Result<String, String> {
            crate::reviewer::run_repair_agent(&i, "claude", None, None, &prompt, &reg)?;
            git_ops::finish_merge_after_resolution(&i).map_err(e2s)
        })
        .await?;
        match (resolved, cstep) {
            (Ok(rev), Step::Run(cid)) => finish(cx, cid, true, &rev).await?,
            (Ok(_), Step::Cached(_)) => {}
            (Err(e), cstep) => {
                if let Step::Run(cid) = cstep {
                    finish(cx, cid, false, &e).await?;
                }
                let reason = fail(format!("충돌 해소 실패: {e}")).await;
                finish(cx, step_id, false, &reason).await?;
                return escalate(cx, &t, TicketState::Integrating, &reason).await;
            }
        }
    }
    // 통합으로 바뀐 경로는 이미 통합된 티켓과 이 티켓의 허용 경로 합집합 안에 있어야 한다.
    let all = pdb::list_tickets(&cx.pool, run.id).await.map_err(e2s)?;
    let allowed = union_paths(&all, |o| o.id == t.id || o.state == TicketState::Integrated.as_str());
    let (i, p, acceptance, reg) = (integration.clone(), pre.clone(), t.acceptance(), cx.registrar.clone());
    let verdict = blocking(move || -> Result<(), String> {
        let changed = git_ops::changed_paths(&i, &p).map_err(e2s)?;
        let outside = model::paths_outside_allowed(&changed, &allowed);
        if !outside.is_empty() {
            return Err(format!("통합 결과가 허용 경로 밖을 변경했습니다: {}", outside.join(", ")));
        }
        let outcomes = checks::run_ticket_checks(&i, &acceptance, Some(&reg));
        if !checks::checks_passed(&outcomes) {
            return Err(if outcomes.is_empty() {
                "통합 뒤 실행할 검증 명령이 없습니다".to_string()
            } else {
                format!("통합 뒤 검증 실패:\n{}", checks::failure_summary(&outcomes))
            });
        }
        // 검증이 남긴 추적되지 않는 산출물을 지워 다음 통합의 변경 경로 검사를 깨끗하게 한다.
        let head = git_ops::head(&i).map_err(e2s)?;
        git_ops::reset_to(&i, &head).map_err(e2s)
    })
    .await?;
    if let Err(e) = verdict {
        let reason = fail(e).await;
        finish(cx, step_id, false, &reason).await?;
        return escalate(cx, &t, TicketState::Integrating, &reason).await;
    }
    finish(cx, step_id, true, "통합 완료").await?;
    finish_integration(cx, &t).await
}

async fn finish_integration(cx: &Cx, t: &TicketRow) -> R<Outcome> {
    pdb::set_ticket_state(&cx.pool, t.id, TicketState::Integrating, TicketState::Integrated, now())
        .await
        .map_err(e2s)?;
    if let Some(task_id) = t.task_id {
        // R16: 티켓 브랜치는 base에 머지하지 않고 정리만 한다. 내용은 통합 브랜치에 들어 있다.
        close_pipeline_task(&cx.state(), task_id, Closure::Integrated).await;
    }
    Ok(Outcome::Progress)
}

// ---------- 최종 리뷰 ----------

async fn do_final_review(cx: &Cx, run: &RunRow) -> R<Outcome> {
    let integration = PathBuf::from(run.integration_path.clone().ok_or("통합 worktree가 없습니다")?);
    let task_id = run.integration_task_id.ok_or("통합 작업이 없습니다")?;
    recover_interrupted_final_fix(cx, run, &integration).await?;
    let fork = db::get_task(&cx.pool, task_id)
        .await
        .map_err(e2s)?
        .and_then(|t| t.base_revision)
        .ok_or("통합 작업의 기준 커밋을 알 수 없습니다")?;
    let tickets = pdb::list_tickets(&cx.pool, run.id).await.map_err(e2s)?;
    let spec: Spec = run.spec_json.as_deref().and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default();
    let (i, f) = (integration.clone(), fork.clone());
    let diff = blocking(move || git_ops::diff_text(&i, &f)).await?.map_err(e2s)?;
    let content = format!("{}\n\n{}", plan_text(run, &tickets)?, diff);
    let focus = format!("최종 통합 결과: {}", clip(&run.goal, 300));
    let token = run.auto_fix_used.max(0) as u32;
    let step_id = match begin(cx, None, "final_review", Some(review_run::JUDGE_VENDOR), token, "최종 리뷰").await? {
        Step::Cached(row) => return final_decision(cx, run, &integration, &spec, &tickets, parse_outcome(&row)).await,
        Step::Run(id) => id,
    };
    match fanout_review(cx, run, ReviewKind::Final, "final_review", &focus, &content, FINAL_REVIEW_TIMEOUT_SECS).await? {
        Err(reason) => {
            finish(cx, step_id, false, &reason).await?;
            pause(cx, RunState::FinalReview, &reason).await
        }
        Ok(outcome) => {
            pdb::set_run_final_review(&cx.pool, run.id, outcome.review_id, now()).await.map_err(e2s)?;
            finish(cx, step_id, true, &serde_json::to_string(&outcome).map_err(e2s)?).await?;
            final_decision(cx, run, &integration, &spec, &tickets, outcome).await
        }
    }
}

/// 자동 수정 도중 앱이 죽었다면(단계가 running으로 남음) 통합 worktree를 수정 전 head로 되돌린다.
/// 자동 수정 표시는 수정 전에 이미 저장돼 있으므로 다시 시도하지는 않는다.
async fn recover_interrupted_final_fix(cx: &Cx, run: &RunRow, integration: &Path) -> R<()> {
    if run.auto_fix_used == 0 {
        return Ok(());
    }
    let key = model::step_key(run.id, "final_fix", None, 0);
    let Some(step) = pdb::list_steps(&cx.pool, run.id).await.map_err(e2s)?.into_iter().find(|s| s.step_key == key) else {
        return Ok(());
    };
    if step.status != "running" {
        return Ok(());
    }
    if let Some(pre) = step.prompt.as_deref().and_then(steps::recorded_pre) {
        let i = integration.to_path_buf();
        blocking(move || git_ops::reset_to(&i, &pre)).await?.map_err(e2s)?;
    }
    pdb::finish_step(&cx.pool, step.id, false, "자동 수정이 중단되어 수정 전 상태로 되돌렸습니다", now())
        .await
        .map_err(e2s)?;
    pdb::set_run_error(&cx.pool, run.id, Some("최종 리뷰 자동 수정이 중단되어 되돌렸습니다. 자동 수정은 한 번만 시도합니다."), now())
        .await
        .map_err(e2s)?;
    Ok(())
}

/// 최종 승인 요청(G2)에 붙이는 안내. 남은 blocking 지적을 승인 화면에서 볼 수 있게 한다.
fn blocking_note(prefix: &str, findings: &str) -> String {
    format!("{prefix}\n남은 blocking 지적:\n{findings}")
}

async fn final_decision(
    cx: &Cx,
    run: &RunRow,
    integration: &Path,
    spec: &Spec,
    tickets: &[TicketRow],
    outcome: ReviewOutcome,
) -> R<Outcome> {
    if !(outcome.blocking && run.auto_fix_used == 0) {
        let note = outcome
            .blocking
            .then(|| blocking_note("자동 수정 뒤에도 blocking 지적이 남은 채 승인을 요청합니다.", &outcome.findings));
        return request_final_approval(cx, run, integration, note).await;
    }
    // R17: Claude가 한 번만 자동 수정한다. 시도했다는 표시를 수정 전에 저장해, 실패·중단·재시작 어느 경우에도 두 번 하지 않는다.
    pdb::set_run_auto_fix_used(&cx.pool, run.id, true, now()).await.map_err(e2s)?;
    let pre = {
        let i = integration.to_path_buf();
        blocking(move || git_ops::head(&i)).await?.map_err(e2s)?
    };
    let prompt = prompts::final_fix_prompt(spec, &outcome.blocking_findings);
    let step_id = match begin(cx, None, "final_fix", Some("claude"), 0, &steps::with_pre(&pre, &prompt)).await? {
        // 같은 단계가 이미 끝났다면(이전 판본의 기록) 다시 하지 않고 재리뷰로 간다.
        Step::Cached(_) => return Ok(Outcome::Progress),
        Step::Run(id) => id,
    };
    let acceptance: Vec<String> = {
        let mut v: Vec<String> = tickets.iter().flat_map(|t| t.acceptance()).collect();
        v.sort();
        v.dedup();
        v
    };
    let allowed = union_paths(tickets, |t| t.state == TicketState::Integrated.as_str());
    let (i, reg) = (integration.to_path_buf(), cx.registrar.clone());
    let result = blocking(move || {
        steps::apply_final_fix(&i, &pre, &allowed, &acceptance, Some(&reg), || {
            crate::reviewer::run_repair_agent(&i, "claude", None, None, &prompt, &reg)
        })
    })
    .await?;
    match result {
        Ok(()) => {
            finish(cx, step_id, true, "자동 수정 완료").await?;
            Ok(Outcome::Progress)
        }
        Err(e) => {
            if cx.reg.is_cancelled() {
                // 취소·종료로 끊긴 시도는 소모하지 않는다(워크트리는 이미 되돌려졌다). 다음 시작에서 다시 시도한다.
                pdb::set_run_auto_fix_used(&cx.pool, run.id, false, now()).await.map_err(e2s)?;
                return Err(e);
            }
            finish(cx, step_id, false, &e).await?;
            let note = blocking_note(&format!("최종 리뷰 자동 수정에 실패해 되돌렸습니다: {e}"), &outcome.findings);
            request_final_approval(cx, run, integration, Some(note)).await
        }
    }
}

/// 남은 변경을 커밋하고 통합 작업을 검토 대기로 올려 사용자 승인을 요청한다(G2). 병합은 사용자가 승인한다.
async fn request_final_approval(cx: &Cx, run: &RunRow, integration: &Path, note: Option<String>) -> R<Outcome> {
    let i = integration.to_path_buf();
    blocking(move || git_ops::commit_all(&i, "파이프라인 최종 정리")).await?.map_err(e2s)?;
    if let Some(note) = note {
        pdb::set_run_error(&cx.pool, run.id, Some(&note), now()).await.map_err(e2s)?;
    }
    if let Some(task_id) = run.integration_task_id {
        db::mark_awaiting_review_with_notification(&cx.pool, task_id, now(), None, "result")
            .await
            .map_err(e2s)?;
    }
    cas_run(cx, RunState::FinalReview, RunState::AwaitingMergeApproval).await
}

// ---------- 작업 정리 ----------

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Closure {
    /// 통합된 티켓: 브랜치까지 지우고 Done.
    Integrated,
    /// 버려지는 티켓: 브랜치까지 지우고 Discarded.
    Abandon,
    /// 실행 취소: 커밋해 브랜치를 보존하고 Discarded.
    Preserve,
}

/// 파이프라인이 만든 티켓 작업을 닫는다. 승인/폐기 claim을 쓰지 않으므로 Failed 작업도 닫힌다.
/// 실패는 이벤트로만 남기고 호출 흐름을 막지 않는다.
pub(crate) async fn close_pipeline_task(state: &AppState, id: i64, how: Closure) {
    let Ok(pool) = pool_of(state) else { return };
    let Ok(Some(task)) = db::get_task(&pool, id).await else { return };
    close_shell_of(state, id);
    state.lsp.shutdown_task(id).await;
    let active = state.tasks.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
    let worktree = match active {
        Some(a) => {
            if let Some(s) = &a.session {
                s.terminate();
            }
            a.worktree.clone()
        }
        None => worktree_from_task(&task),
    };
    if !is_direct_mode(&worktree) {
        let w = worktree.clone();
        let result = blocking(move || match how {
            Closure::Preserve => w.retire_preserving_branch(),
            _ => w.discard(),
        })
        .await;
        let failure = match result {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(e.to_string()),
            Err(e) => Some(e),
        };
        if let Some(e) = failure {
            let _ = db::append_event(&pool, id, "pipeline_close_failed", Some(&e), now()).await;
        }
    }
    if !matches!(task.state.as_str(), tstate::DONE | tstate::DISCARDED) {
        let end = if how == Closure::Integrated { tstate::DONE } else { tstate::DISCARDED };
        let _ = db::update_state(&pool, id, end, now()).await;
    }
    let _ = db::append_event(&pool, id, "pipeline_closed", None, now()).await;
    close_designmode_webview_of(state, id);
    crate::designmode::cleanup_captures(&worktree.path, id);
    state.preview_workbench.invalidate(id);
}

/// 취소·재시도처럼 실행 중일 수 있는 티켓 작업을 멈추고 닫는다.
pub(crate) async fn stop_and_close_task(state: &AppState, id: i64, how: Closure) {
    let _ = cancel_task_inner(state, id).await;
    close_pipeline_task(state, id, how).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_state_keys_match_plan_vocabulary() {
        assert_eq!(task_state_key(tstate::AWAITING_REVIEW), "awaiting_review");
        assert_eq!(task_state_key(tstate::FAILED), "failed");
        assert_eq!(task_state_key(tstate::DONE), "done");
        assert_eq!(task_state_key(tstate::DISCARDED), "discarded");
        assert_eq!(task_state_key(tstate::RUNNING), "running");
    }

    #[test]
    fn append_errors_keeps_previous_feedback() {
        let s = append_errors(Some("앞선 피드백"), &["a".into(), "b".into()]);
        assert!(s.starts_with("앞선 피드백"));
        assert!(s.contains("- a") && s.contains("- b"));
        assert!(append_errors(None, &["x".into()]).contains("- x"));
    }

    #[test]
    fn flight_guard_is_exclusive_and_released_on_drop() {
        let a = FlightGuard::acquire(900_001).expect("첫 획득");
        assert!(FlightGuard::acquire(900_001).is_none());
        drop(a);
        assert!(FlightGuard::acquire(900_001).is_some());
    }

    fn ticket(gen: i64, attempt: i64) -> TicketRow {
        TicketRow {
            id: 1,
            run_id: 1,
            key: "T1".into(),
            title: "t".into(),
            body: String::new(),
            vendor: "codex".into(),
            reviewer_vendor: None,
            acceptance_json: "[]".into(),
            allowed_paths_json: "[]".into(),
            deps_json: "[]".into(),
            covers_json: "[]".into(),
            state: "pending".into(),
            task_id: None,
            attempt,
            reassigned: 0,
            last_error: None,
            updated_at: 0,
            gen,
            spawn_head: None,
        }
    }

    #[test]
    fn step_token_never_repeats_across_generations() {
        let mut seen = HashSet::new();
        for gen in 0..3 {
            for attempt in 0..3 {
                assert!(seen.insert(token(&ticket(gen, attempt))));
            }
        }
    }

    #[test]
    fn goal_contract_is_valid_or_dropped() {
        let mut t = ticket(0, 0);
        t.acceptance_json = serde_json::to_string(&vec!["npm test"; 40]).unwrap();
        let gc = goal_contract_for(&t).expect("항목 수 상한을 넘기지 않는다");
        assert!(gc.acceptance.len() <= 30);
    }

    #[test]
    fn union_paths_filters_and_dedups() {
        let mut a = ticket(0, 0);
        a.allowed_paths_json = r#"["src/","docs/"]"#.into();
        let mut b = ticket(0, 0);
        b.id = 2;
        b.allowed_paths_json = r#"["src/","web/"]"#.into();
        let all = [a, b];
        assert_eq!(union_paths(&all, |_| true), vec!["docs/", "src/", "web/"]);
        assert_eq!(union_paths(&all, |t| t.id == 2), vec!["src/", "web/"]);
    }
}
