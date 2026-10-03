//! 크론 틱 루프(Phase 3) — 오케스트레이션 계층(AppHandle/State 의존).
//!
//! due 판정(`super::is_due`)은 tauri 비의존 순수 함수로 상위 모듈에 남아있고, 여기서는
//! 틱 스케줄링 + task/reminder 실행 분기만 담당한다.

use tauri::Manager;

use crate::commands::{self, AppState};
use crate::db;
use crate::quiz;

/// 크론 틱 주기.
const TICK_INTERVAL_SECS: u64 = 60;

/// `kind="task"` 스케줄의 payload JSON. `agent`는 비어 있으면 "claude"로 대체.
#[derive(serde::Deserialize)]
struct TaskPayload {
    repo: String,
    instruction: String,
    #[serde(default)]
    agent: String,
    #[serde(default)]
    goal_contract: Option<crate::goal_contract::GoalContract>,
}

/// `kind="reminder"` 스케줄의 payload JSON.
#[derive(serde::Deserialize)]
struct ReminderPayload {
    text: String,
}

/// `kind="quiz"` 스케줄의 payload JSON (설계 0044).
///
/// instruction이 없다 — `quiz::generate`가 종류에 맞춰 조립하기 때문이다. 사용자가 프롬프트를
/// 직접 쓰면 출력 스펙이 어긋나 `inbox` 파싱이 통째로 깨진다.
#[derive(serde::Deserialize)]
struct QuizPayload {
    repo: String,
    /// "domain" | "vocab" | "coding" | "trivia"
    quiz_kind: String,
    /// 한 번에 만들 문제 수. 승인 마찰을 낮추려면 주기를 길게 잡고 이 값을 키운다.
    #[serde(default = "default_quiz_count")]
    count: usize,
    /// 도메인 출제에 쓸 청크 수. 많을수록 프롬프트가 길어진다.
    #[serde(default = "default_source_chunks")]
    source_chunks: i64,
    #[serde(default)]
    agent: String,
}

fn default_quiz_count() -> usize {
    20
}

fn default_source_chunks() -> i64 {
    8
}

/// 크론 틱 루프(Phase 3) — `TICK_INTERVAL_SECS` 주기. `.setup()`에서 1회 spawn, 앱 생명주기
/// 내내 상주.
///
/// M4(불변식): 매 틱마다 스케줄을 한 번씩만 평가·실행하므로 "틱당 스케줄당 최대 1회"가
/// 루프 구조 자체로 보장된다(같은 틱 안에서 동일 스케줄을 재평가하지 않음). 앱이 오래
/// 꺼져 있다 켜져도 `is_due`는 "1회 이상 도래" 여부만 반환하고, 그 1회 실행 후 즉시
/// `mark_schedule_ran`으로 last_run_at을 now로 갱신하므로 과거 누락분이 몰아치지 않는다.
pub async fn tick_loop(app: tauri::AppHandle) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(TICK_INTERVAL_SECS));
    loop {
        interval.tick().await;
        let state = app.state::<AppState>();
        let Some(pool) = state.pool.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
            continue;
        };
        let schedules = match db::list_enabled_schedules(&pool).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("스케줄 목록 조회 실패: {e}");
                continue;
            }
        };
        for sched in schedules {
            run_schedule_if_due(&app, &pool, &sched).await;
        }
    }
}

/// 스케줄 1건을 due 판정 후(참이면) 실행. 실행 여부와 무관하게 파싱/실행 실패는 로그 후 skip
/// (best-effort) — 개별 스케줄 실패가 나머지 스케줄이나 루프를 막지 않는다.
///
/// 순서(M4 중복실행 방지 — 멱등 우선): due 판정 → **부작용 전에 먼저** `mark_schedule_ran` 기록
/// → 성공했을 때만 실제 실행. mark 실패(예: SQLite busy)는 이번 틱 실행을 skip해
/// (last_run_at 미갱신 상태로) 다음 틱 재평가 시 중복 실행되는 것을 막는다.
///
/// `run_at`이 있으면(1회성 리마인더) cron 파싱 없이 절대시각 비교만 하고, 실행 후
/// `set_schedule_enabled(false)`로 자동 비활성화한다(재발화 방지 — 목록엔 남아 "발화됨" 표시).
async fn run_schedule_if_due(
    app: &tauri::AppHandle,
    pool: &sqlx::SqlitePool,
    sched: &db::Schedule,
) {
    let now_ts = crate::now();
    let due = match sched.run_at {
        Some(run_at) => Ok(now_ts >= run_at && sched.last_run_at.is_none()),
        None => {
            let base = sched.last_run_at.unwrap_or(sched.created_at);
            super::is_due(&sched.cron, base, now_ts, sched.tz_offset_secs)
        }
    };
    let due = match due {
        Ok(due) => due,
        Err(e) => {
            eprintln!("스케줄 #{} cron 파싱 실패: {e}", sched.id);
            return;
        }
    };
    if !due {
        return;
    }
    if let Err(e) = db::mark_schedule_ran(pool, sched.id, now_ts).await {
        eprintln!(
            "스케줄 #{} last_run_at 갱신 실패 — 이번 틱 skip(중복실행 방지): {e}",
            sched.id
        );
        return;
    }
    match sched.kind.as_str() {
        "task" => run_task_schedule(app, sched).await,
        "reminder" => run_reminder_schedule(app, sched).await,
        "quiz" => run_quiz_schedule(app, pool, sched).await,
        other => eprintln!("스케줄 #{} 알 수 없는 kind: {other}", sched.id),
    }
    // 1회성(run_at 있음)은 재발화하지 않도록 자동 비활성화 — best-effort.
    if sched.run_at.is_some() {
        if let Err(e) = db::set_schedule_enabled(pool, sched.id, false).await {
            eprintln!("스케줄 #{} 1회성 자동비활성 실패: {e}", sched.id);
        }
    }
}

/// `kind="task"` 실행 — payload 파싱·검증 후 헤드리스 작업 생성(외부기원 — 승인 대기) + OS 알림.
async fn run_task_schedule(app: &tauri::AppHandle, sched: &db::Schedule) {
    let payload: TaskPayload = match serde_json::from_str(&sched.payload) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("스케줄 #{} payload 파싱 실패: {e}", sched.id);
            return;
        }
    };
    let repo = payload.repo.trim().to_string();
    let instruction = payload.instruction.trim().to_string();
    if repo.is_empty() {
        eprintln!("스케줄 #{} payload.repo 비어있음", sched.id);
        return;
    }
    if instruction.is_empty() {
        eprintln!("스케줄 #{} payload.instruction 비어있음", sched.id);
        return;
    }
    let state = app.state::<AppState>();
    let agent = if payload.agent.trim().is_empty() {
        "claude".to_string()
    } else {
        payload.agent
    };
    let mut params = commands::CreateTaskParams::headless_terminal(
        repo.clone(),
        instruction,
        agent,
        commands::TaskOrigin::External,
    );
    params.goal_contract = payload.goal_contract;
    match commands::create_task_internal(app, &state, params).await {
        Ok(task) => {
            use tauri_plugin_notification::NotificationExt;
            let _ = app
                .notification()
                .builder()
                .title(format!("Dojang · 작업 #{} 승인 대기", task.id))
                .body(format!("⏰ {repo} — {}", sched.label))
                .show();
        }
        Err(e) => eprintln!("스케줄 #{} 작업 생성 실패: {e}", sched.id),
    }
}

/// `kind="quiz"` 실행 — 대기 퀴즈 문제를 만들 헤드리스 작업을 생성한다(설계 0044).
///
/// `run_task_schedule`과 두 곳이 다르다.
/// 1. instruction을 payload에서 받지 않고 `quiz::generate`가 조립한다 — 도메인이면 청크를 붙인다.
/// 2. 승인 알림을 띄우지 않는다. 생성 주기가 길어 알림 가치가 낮고, 퀴즈 때문에
///    알림이 울리면 정작 봐야 할 알림이 묻힌다.
///
/// **승인 대기는 그대로 둔다**(`TaskOrigin::External`). 자동으로 뜬 에이전트가 승인을
/// 우회하지 않는다는 원칙은 퀴즈에도 적용된다.
async fn run_quiz_schedule(app: &tauri::AppHandle, pool: &sqlx::SqlitePool, sched: &db::Schedule) {
    let payload: QuizPayload = match serde_json::from_str(&sched.payload) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("스케줄 #{} quiz payload 파싱 실패: {e}", sched.id);
            return;
        }
    };
    let repo = payload.repo.trim().to_string();
    if repo.is_empty() {
        eprintln!("스케줄 #{} payload.repo 비어있음", sched.id);
        return;
    }
    let Some(kind) = quiz::generate::Kind::parse(payload.quiz_kind.trim()) else {
        eprintln!(
            "스케줄 #{} 알 수 없는 quiz_kind: {}",
            sched.id, payload.quiz_kind
        );
        return;
    };

    // 도메인만 청크가 필요하다. 조회 실패는 생성 실패로 이어지므로 여기서 끊는다.
    let chunks = if kind.needs_source() {
        match quiz::pick_source_chunks(pool, payload.source_chunks).await {
            Ok(chunks) => chunks,
            Err(e) => {
                eprintln!("스케줄 #{} 출처 청크 조회 실패: {e}", sched.id);
                return;
            }
        }
    } else {
        Vec::new()
    };

    // inbox는 워크트리가 아니라 앱 데이터 디렉터리 밑이다 — 워크트리는 작업마다 다르고
    // 끝나면 사라질 수 있어, 거기 쓴 결과는 거둘 수가 없다.
    let inbox = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(quiz::generate::INBOX_SUBDIR);

    let Some(instruction) =
        quiz::generate::build_instruction(kind, payload.count, &chunks, &inbox.to_string_lossy())
    else {
        // 도메인인데 Obsidian 청크가 하나도 없는 경우가 대부분이다 — 동기화 전이거나 소스 미연결.
        eprintln!(
            "스케줄 #{} instruction 조립 실패 — {} 재료가 없다",
            sched.id,
            kind.as_str()
        );
        return;
    };

    let state = app.state::<AppState>();
    let agent = if payload.agent.trim().is_empty() {
        "claude".to_string()
    } else {
        payload.agent
    };
    let params = commands::CreateTaskParams::headless_terminal(
        repo,
        instruction,
        agent,
        commands::TaskOrigin::External,
    );
    if let Err(e) = commands::create_task_internal(app, &state, params).await {
        eprintln!("스케줄 #{} 퀴즈 생성 작업 실패: {e}", sched.id);
    }
}

/// `kind="reminder"` 실행 — payload의 text를 로컬 OS 알림으로 띄운다(best-effort).
async fn run_reminder_schedule(app: &tauri::AppHandle, sched: &db::Schedule) {
    let payload: ReminderPayload = match serde_json::from_str(&sched.payload) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("스케줄 #{} payload 파싱 실패: {e}", sched.id);
            return;
        }
    };
    use tauri_plugin_notification::NotificationExt;
    let _ = app
        .notification()
        .builder()
        .title("Dojang 리마인더")
        .body(&payload.text)
        .show();
}
