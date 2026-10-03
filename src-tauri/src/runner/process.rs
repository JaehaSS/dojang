use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sqlx::SqlitePool;

use crate::db::{self, state, Task};
use crate::pty::{PtyEvent, PtySession};

pub type ActiveTerminalTasks = Arc<Mutex<HashMap<i64, Arc<Mutex<PtySession>>>>>;
pub type ActiveConversationTasks = Arc<Mutex<HashMap<i64, u32>>>;

/// 대화 턴 유휴(무출력) 상한. 워치독 하트비트는 stdout 라인 단위라, 에이전트가 단일 툴을
/// 오래 돌리는 구간(빌드·테스트·설치)은 통째로 "무출력"으로 계산된다. 기존 30분은 실제
/// 에이전트 작업 기준으로 짧아 정상 턴이 백그라운드에서만 죽는 원인이었다 — IDE 상한
/// (`CONVO_IDLE_TIMEOUT_SECS`, 12h)과 같은 값까지 갈 필요는 없으나, 고아 프로세스 정리라는
/// 워치독 본래 목적을 해치지 않는 선에서 4시간으로 올린다.
const CONVERSATION_IDLE_TIMEOUT_SECS: u64 = 14_400;

/// PTY 출력 영속화 배칭 윈도우. 청크(최대 8KiB)마다 fsync 동반 커밋을 수행하면 고출력
/// 툴 실행(빌드/설치 로그) 시 디스크 I/O가 작은 동기 쓰기로 포화된다 — 이 윈도우 동안
/// 도착한 청크를 모아 한 트랜잭션으로 기록한다(replay 지연은 최대 이 윈도우만큼).
const OUTPUT_BATCH_WINDOW: Duration = Duration::from_millis(100);
/// 배치 상한 — 폭주 출력이 윈도우 내에서 무한정 메모리에 쌓이지 않게 하는 가드.
const OUTPUT_BATCH_MAX_BYTES: usize = 256 * 1024;

pub fn active_terminal_tasks() -> ActiveTerminalTasks {
    Arc::new(Mutex::new(HashMap::new()))
}

pub fn active_conversation_tasks() -> ActiveConversationTasks {
    Arc::new(Mutex::new(HashMap::new()))
}

pub fn cancel_terminal_task(active: &ActiveTerminalTasks, task_id: i64) -> bool {
    let Some(session) = active
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&task_id)
        .cloned()
    else {
        return false;
    };
    session
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .terminate();
    true
}

/// 실행 중 terminal task의 활성 PTY stdin에 입력을 전달한다. `None` = 활성 세션 없음.
pub fn write_terminal_input(
    active: &ActiveTerminalTasks,
    task_id: i64,
    data: &[u8],
) -> Option<anyhow::Result<()>> {
    let session = active
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&task_id)
        .cloned()?;
    let result = session
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .write(data);
    Some(result)
}

/// Runner terminal task PTY의 현재 스크롤백 스냅샷(메모리 전용) — 기존 event replay
/// 채널(`runner_events`/`task_output`, DB 영속)과 별개로 PTY 스트림 자체에 편승한
/// replay 원천이다. 세션이 없으면(아직 시작 전/이미 종료) `None`.
pub fn terminal_scrollback(active: &ActiveTerminalTasks, task_id: i64) -> Option<Vec<u8>> {
    let session = active
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&task_id)
        .cloned()?;
    let snapshot = session
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .scrollback_snapshot();
    Some(snapshot)
}

pub fn cancel_conversation_task(active: &ActiveConversationTasks, task_id: i64) -> bool {
    let Some(pgid) = active
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&task_id)
        .copied()
    else {
        return false;
    };
    crate::verify::kill_group(pgid);
    true
}

/// Runner terminal task를 기존 headless agent 규칙으로 PTY에서 시작한다.
pub fn spawn_terminal_task(task: &Task) -> Result<(PtySession, Receiver<PtyEvent>), String> {
    if task.mode != "terminal" {
        return Err("Runner process adapter는 terminal 작업만 지원합니다".to_string());
    }
    let agent = task
        .agent
        .as_deref()
        .filter(|agent| !agent.trim().is_empty())
        .ok_or_else(|| "Runner task에 agent가 없습니다".to_string())?;
    let prompt = task_execution_prompt(task);
    let (bin, args) = crate::agent::headless_args_with_effort(
        agent,
        &prompt,
        task.model.as_deref(),
        task.reasoning_effort.as_deref(),
        Some(&crate::agent::session_name(task.id)),
    )
    .ok_or_else(|| "Runner task의 headless 실행 인자를 만들 수 없습니다".to_string())?;
    let command = resolve_command(&bin)
        .ok_or_else(|| format!("Runner agent를 PATH에서 찾을 수 없습니다: {bin}"))?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    PtySession::spawn(&command, &args, Some(&task.worktree_path), 80, 24)
        .map_err(|error| format!("Runner PTY 생성 실패: {error}"))
}

/// PTY 출력은 즉시 durable replay 저장소에 기록하고, 종료 시 조건부 완료 전이를 수행한다.
pub async fn run_terminal_task(
    pool: SqlitePool,
    task: Task,
    now: i64,
    active: ActiveTerminalTasks,
) -> Result<i32, String> {
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || consume_terminal_events(&runtime, pool, task, now, active))
        .await
        .map_err(|error| format!("Runner process worker가 종료되었습니다: {error}"))?
}

/// Runner conversation task를 구조화 이벤트 스트림으로 실행한다.
pub async fn run_conversation_task(
    pool: SqlitePool,
    task: Task,
    now: i64,
    active: ActiveConversationTasks,
) -> Result<(), String> {
    let agent = task
        .agent
        .as_deref()
        .filter(|agent| !agent.trim().is_empty())
        .ok_or_else(|| "Runner task에 agent가 없습니다".to_string())?;
    let vendor = crate::convo::Vendor::from_agent(agent);
    let bin = resolve_command(vendor.bin())
        .ok_or_else(|| format!("Runner agent를 PATH에서 찾을 수 없습니다: {}", vendor.bin()))?;
    run_conversation_task_with_bin(pool, task, now, active, &bin).await
}

/// `bin` 주입을 허용해 실제 Runner와 통합 테스트가 같은 lifecycle을 검증한다.
pub async fn run_conversation_task_with_bin(
    pool: SqlitePool,
    task: Task,
    now: i64,
    active: ActiveConversationTasks,
    bin: &str,
) -> Result<(), String> {
    let runtime = tokio::runtime::Handle::current();
    let bin = bin.to_string();
    tokio::task::spawn_blocking(move || {
        consume_conversation_events(&runtime, pool, task, now, active, &bin, None)
    })
    .await
    .map_err(|error| format!("Runner conversation worker가 종료되었습니다: {error}"))?
}

/// AwaitingReview 대화형 작업에 후속 메시지(주석 재전송 등)를 주입해 재개한다(B-1).
/// `message`가 이번 턴 프롬프트를 그대로 대체한다 — 로컬 `start_convo_turn`의 resume 분기와
/// 동일 계약(Goal Contract 재합성 없음, 새 메시지만 전달).
pub async fn resume_conversation_task(
    pool: SqlitePool,
    task: Task,
    message: String,
    now: i64,
    active: ActiveConversationTasks,
) -> Result<(), String> {
    let agent = task
        .agent
        .as_deref()
        .filter(|agent| !agent.trim().is_empty())
        .ok_or_else(|| "Runner task에 agent가 없습니다".to_string())?;
    let vendor = crate::convo::Vendor::from_agent(agent);
    let bin = resolve_command(vendor.bin())
        .ok_or_else(|| format!("Runner agent를 PATH에서 찾을 수 없습니다: {}", vendor.bin()))?;
    resume_conversation_task_with_bin(pool, task, message, now, active, &bin).await
}

/// `bin` 주입 버전 — 통합 테스트가 스텁 스크립트로 벤더 실행을 대체할 수 있게 한다.
pub async fn resume_conversation_task_with_bin(
    pool: SqlitePool,
    task: Task,
    message: String,
    now: i64,
    active: ActiveConversationTasks,
    bin: &str,
) -> Result<(), String> {
    ensure_resume_running(&pool, task.id, now).await?;
    let runtime = tokio::runtime::Handle::current();
    let bin = bin.to_string();
    tokio::task::spawn_blocking(move || {
        consume_conversation_events(&runtime, pool, task, now, active, &bin, Some(message))
    })
    .await
    .map_err(|error| format!("Runner conversation worker가 종료되었습니다: {error}"))?
}

async fn ensure_resume_running(pool: &SqlitePool, task_id: i64, now: i64) -> Result<(), String> {
    let state = db::get_task(pool, task_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?
        .state;
    if state == state::RUNNING {
        return Ok(());
    }
    if state != state::AWAITING_REVIEW {
        return Err(format!("대화를 재개할 수 없는 작업 상태입니다: {state}"));
    }
    if db::mark_running_from_review(pool, task_id, now)
        .await
        .map_err(|error| error.to_string())?
    {
        return Ok(());
    }
    Err("작업 상태가 동시에 변경되었습니다".into())
}

fn consume_terminal_events(
    runtime: &tokio::runtime::Handle,
    pool: SqlitePool,
    task: Task,
    now: i64,
    active: ActiveTerminalTasks,
) -> Result<i32, String> {
    let (session, events) = match spawn_terminal_task(&task) {
        Ok(value) => value,
        Err(error) => {
            let _ = runtime.block_on(db::finish_running_task_with_notification(
                &pool,
                task.id,
                state::FAILED,
                super::now_secs(),
                "failed",
                Some(&error),
                "failure",
            ));
            return Err(error);
        }
    };
    let identity = session
        .process_identity()
        .ok_or_else(|| "Runner could not establish a secure terminal process identity".to_string());
    let registration = identity.and_then(|identity| {
        runtime
            .block_on(db::record_task_process_start(
                &pool,
                task.id,
                session.pid() as i64,
                identity,
                "terminal",
                now,
            ))
            .map_err(|error| error.to_string())
    });
    if let Err(error) = registration {
        session.terminate();
        let detail = format!("Runner process identity persistence failed: {error}");
        let _ = runtime.block_on(db::finish_running_task_with_notification(
            &pool,
            task.id,
            state::FAILED,
            super::now_secs(),
            "failed",
            Some(&detail),
            "failure",
        ));
        return Err(detail);
    }
    let session = Arc::new(Mutex::new(session));
    active
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(task.id, session);
    while let Ok(event) = events.recv() {
        let exit_code = match event {
            PtyEvent::Output(bytes) => {
                // 배칭 윈도우 동안 후속 청크를 모아 한 트랜잭션으로 기록 — 원본 바이트로
                // 누적한 뒤 마지막에 한 번만 lossy 변환한다(청크 경계의 다중바이트 문자 보존).
                let mut buffer = bytes;
                let mut pending_exit = None;
                let deadline = Instant::now() + OUTPUT_BATCH_WINDOW;
                while buffer.len() < OUTPUT_BATCH_MAX_BYTES {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    match events.recv_timeout(remaining) {
                        Ok(PtyEvent::Output(more)) => buffer.extend_from_slice(&more),
                        Ok(PtyEvent::Exit(code)) => {
                            pending_exit = Some(code);
                            break;
                        }
                        Err(_) => break,
                    }
                }
                let data = String::from_utf8_lossy(&buffer);
                let _ = runtime.block_on(db::append_task_output_with_runner_event(
                    &pool, task.id, now, &data,
                ));
                match pending_exit {
                    Some(code) => code,
                    None => continue,
                }
            }
            PtyEvent::Exit(code) => code,
        };
        let detail = exit_code.to_string();
        let _ = runtime.block_on(db::finish_running_task_with_notification(
            &pool,
            task.id,
            state::AWAITING_REVIEW,
            super::now_secs(),
            "completed",
            Some(&detail),
            if exit_code == 0 { "result" } else { "failure" },
        ));
        active
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&task.id);
        return Ok(exit_code);
    }
    let error = "Runner PTY event stream이 예기치 않게 닫혔습니다";
    let _ = runtime.block_on(db::finish_running_task_with_notification(
        &pool,
        task.id,
        state::FAILED,
        super::now_secs(),
        "failed",
        Some(error),
        "failure",
    ));
    active
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(&task.id);
    Err(error.to_string())
}

fn consume_conversation_events(
    runtime: &tokio::runtime::Handle,
    pool: SqlitePool,
    task: Task,
    now: i64,
    active: ActiveConversationTasks,
    bin: &str,
    message_override: Option<String>,
) -> Result<(), String> {
    if task.mode != "conversation" {
        return Err("Runner conversation adapter는 conversation 작업만 지원합니다".to_string());
    }
    // 원격은 토론을 돌리지 않는다(Q8). 거부하지 않으면 비좌측 면이 조용히 빠진 채 좌측만
    // 혼잣말을 한다 — 원격 토론이 필요해지면 그때 같은 `convo::debate` 함수를 여기서 부른다.
    if !runtime
        .block_on(db::debate_sides(&pool, task.id))
        .map_err(|error| error.to_string())?
        .is_empty()
    {
        return Err("Runner는 토론 중인 작업을 실행할 수 없습니다".to_string());
    }
    let agent = task
        .agent
        .as_deref()
        .filter(|agent| !agent.trim().is_empty())
        .ok_or_else(|| "Runner task에 agent가 없습니다".to_string())?;
    let vendor = crate::convo::Vendor::from_agent(agent);
    let task_id = task.id;
    // 초기 instruction과 후속 메시지 모두 durable transcript와 output replay에 남긴다.
    // 최초 턴의 실제 프롬프트만 Goal Contract를 합성한다.
    let (user_text, prompt) = match message_override {
        Some(message) => (message.clone(), message),
        None => (task.instruction.clone(), task_execution_prompt(&task)),
    };
    // 절단이 남긴 캡슐을 맨 앞에 붙인다(ADR 0170). **데스크톱 경로와 짝이다** —
    // 원격/큐 후속 턴(`POST /tasks/:id/message` → requeue → 이 워커)도 같은 턴이므로
    // 여기를 빠뜨리면 캡슐이 소비되지 않고 남아 엉뚱한 나중 턴에 붙는다.
    // 읽기만 한다. 지우는 것은 세션 확립 시점(`db::set_convo_session`)뿐이다.
    let prompt = match runtime.block_on(db::peek_pending_capsule(&pool, task_id)) {
        Ok(Some(capsule)) => format!("{capsule}\n{prompt}"),
        _ => prompt,
    };
    let user_event = serde_json::json!({ "kind": "user", "text": &user_text }).to_string();
    let _ = runtime.block_on(db::append_convo_event(&pool, task_id, &user_event, now));
    let _ = runtime.block_on(db::append_task_output_with_runner_event(
        &pool,
        task_id,
        now,
        &user_event,
    ));
    let spawn_pool = pool.clone();
    let event_pool = pool.clone();
    // 스트림 종료 후 합성 사인 이벤트를 남길 때 쓴다(`event_pool`은 콜백으로 이동).
    let event_pool_finalize = pool.clone();
    let spawn_error = Arc::new(Mutex::new(None::<String>));
    let spawn_error_callback = spawn_error.clone();
    // 턴이 `Result`로 정상 마감됐는지 관측 — 없으면 워치독 kill이나 프로세스 즉사이므로
    // "completed"로 마감하면 안 된다(사인 없이 조용히 끝나던 회귀).
    let result_seen = Arc::new(AtomicBool::new(false));
    let result_seen_callback = result_seen.clone();
    // 질문 대기 판별 재료 — 데스크톱 경로(commands.rs 턴 에필로그)와 같은 기준을 쓴다.
    let tool_seen = Arc::new(AtomicBool::new(false));
    let tool_seen_callback = tool_seen.clone();
    let result_error = Arc::new(AtomicBool::new(false));
    let result_error_callback = result_error.clone();
    let last_text = Arc::new(Mutex::new(String::new()));
    let last_text_callback = last_text.clone();
    let result = crate::convo::run_turn_with_effort(
        &task.worktree_path,
        &prompt,
        task.convo_session_id.as_deref(),
        CONVERSATION_IDLE_TIMEOUT_SECS,
        vendor,
        bin,
        task.model.as_deref(),
        task.reasoning_effort.as_deref(),
        task.service_tier.as_deref(),
        &[],
        Some(&crate::agent::session_name(task_id)),
        // praxis-runner에는 웹뷰가 없다 — 프리뷰 MCP를 주입할 대상이 없다.
        None,
        |pgid| match register_conversation_process(runtime, &spawn_pool, task_id, pgid, now) {
            Ok(()) => {
                active
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .insert(task_id, pgid);
            }
            Err(error) => {
                crate::verify::kill_group(pgid);
                *spawn_error_callback
                    .lock()
                    .unwrap_or_else(|lock_error| lock_error.into_inner()) = Some(error);
            }
        },
        |event| {
            match &event {
                crate::convo::ConvoEvent::Result { text, is_error, .. } => {
                    result_seen_callback.store(true, Ordering::Relaxed);
                    result_error_callback.store(*is_error, Ordering::Relaxed);
                    // 벤더에 따라 최종 text가 비어 오기도 한다 — 그때는 직전 Text를 쓴다.
                    if !text.trim().is_empty() {
                        *last_text_callback
                            .lock()
                            .unwrap_or_else(|error| error.into_inner()) = text.clone();
                    }
                }
                crate::convo::ConvoEvent::ToolUse { .. } => {
                    tool_seen_callback.store(true, Ordering::Relaxed)
                }
                // 서브 에이전트 텍스트(parent_id)는 사용자에게 던진 말이 아니다.
                crate::convo::ConvoEvent::Text {
                    text,
                    parent_id: None,
                } => {
                    if !text.trim().is_empty() {
                        *last_text_callback
                            .lock()
                            .unwrap_or_else(|error| error.into_inner()) = text.clone();
                    }
                }
                _ => {}
            }
            let event_json = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
            // convo/출력/replay 3건을 한 트랜잭션으로 — 라인당 커밋(fsync) 2회 방지.
            let _ = runtime.block_on(db::append_convo_event_with_runner_output(
                &event_pool,
                task_id,
                &event_json,
                now,
            ));
        },
    );
    active
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(&task_id);
    let registration_error = spawn_error
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    if let Some(error) = registration_error {
        let _ = runtime.block_on(db::finish_running_task_with_notification(
            &pool,
            task_id,
            state::FAILED,
            super::now_secs(),
            "failed",
            Some(&error),
            "failure",
        ));
        return Err(error);
    }
    match result {
        Ok(outcome) => {
            // resume 토큰이 있으면 워치독 kill·비정상 종료도 `Ok(outcome)`으로 돌아온다
            // (사인은 outcome에만 실린다). `Result` 이벤트 없이 끝났으면 완료가 아니다.
            if !result_seen.load(Ordering::Relaxed) {
                let detail = outcome.failure_text(CONVERSATION_IDLE_TIMEOUT_SECS);
                let event = crate::convo::ConvoEvent::Result {
                    text: detail.clone(),
                    is_error: true,
                    session_id: String::new(),
                    cost_usd: 0.0,
                    num_turns: 0,
                    tokens_in: 0,
                    tokens_out: 0,
                };
                if let Ok(event_json) = serde_json::to_string(&event) {
                    let _ = runtime.block_on(db::append_convo_event_with_runner_output(
                        &event_pool_finalize,
                        task_id,
                        &event_json,
                        now,
                    ));
                }
                // 세션 토큰은 남긴다 — 대화 맥락은 유효하므로 사용자가 이어서 재개할 수 있다.
                let _ =
                    runtime.block_on(db::set_convo_session(&pool, task_id, &outcome.session_id));
                let _ = runtime.block_on(db::finish_running_task_with_notification(
                    &pool,
                    task_id,
                    state::FAILED,
                    super::now_secs(),
                    "failed",
                    Some(&detail),
                    "failure",
                ));
                return Err(detail);
            }
            let _ = runtime.block_on(db::set_convo_session(&pool, task_id, &outcome.session_id));
            let worktree_changed = tool_seen.load(Ordering::Relaxed)
                && crate::commands::worktree_from_task(&task).has_changes().unwrap_or(false);
            let question = crate::convo::question::awaits_answer(
                &crate::convo::question::TurnEpilogue {
                    last_text: &last_text.lock().unwrap_or_else(|error| error.into_inner()),
                    worktree_changed,
                    result_seen: true,
                    result_error: result_error.load(Ordering::Relaxed),
                },
            );
            let notification_kind = if result_error.load(Ordering::Relaxed) {
                "failure"
            } else if question {
                "question"
            } else {
                "result"
            };
            let _ = runtime.block_on(db::finish_running_task_with_notification(
                &pool,
                task_id,
                state::AWAITING_REVIEW,
                super::now_secs(),
                "completed",
                None,
                notification_kind,
            ));
            // 전이 직후에 덧쓴다 — Running 진입에서 이미 NULL로 지워졌으므로 잔상 위험은 없고,
            // 상태가 그 사이 승인/폐기로 넘어갔으면 가드가 알아서 무시한다.
            // 워크트리를 실제로 바꿨는지가 판단 기준 — 데스크톱 경로와 같은 함수를 쓴다.
            // 툴 사용 여부로 가르면 조사 후 되묻는 턴이 전부 검토 대기로 넘어간다.
            if question {
                let _ = runtime.block_on(db::set_awaiting_kind(
                    &pool,
                    task_id,
                    Some(db::awaiting_kind::QUESTION),
                ));
            }
            Ok(())
        }
        Err(error) => {
            let _ = runtime.block_on(db::finish_running_task_with_notification(
                &pool,
                task_id,
                state::FAILED,
                super::now_secs(),
                "failed",
                Some(&error),
                "failure",
            ));
            Err(error)
        }
    }
}

fn register_conversation_process(
    runtime: &tokio::runtime::Handle,
    pool: &SqlitePool,
    task_id: i64,
    pgid: u32,
    now: i64,
) -> Result<(), String> {
    let identity = super::process_identity::observe(pgid)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Runner could not establish conversation process identity".to_string())?;
    runtime
        .block_on(db::record_task_process_start(
            pool,
            task_id,
            pgid as i64,
            &identity,
            "conversation",
            now,
        ))
        .map_err(|error| error.to_string())
}

fn task_execution_prompt(task: &Task) -> String {
    crate::goal_contract::execution_prompt(&task.instruction, task.goal_contract.as_deref())
}

fn resolve_command(bin: &str) -> Option<String> {
    if Path::new(bin).is_file() {
        return Some(bin.to_string());
    }
    crate::reviewer::which(bin)
}
