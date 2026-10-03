//! SQLite Task 영속화 (sqlx, WAL). Tauri 비의존 — `cargo test`로 검증.
//!
//! Phase 1: Task 메타데이터(레포/브랜치/worktree/상태)를 저장해 앱 재시작 시 복원.
//! 타임스탬프는 호출자가 주입(chrono 의존 회피).

use serde::{Deserialize, Serialize};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};

use crate::goal_contract::GoalContract;

mod task_deletion;
pub use task_deletion::delete_task;

/// Task 상태 (DB에는 문자열로 저장).
pub mod state {
    pub const CREATED: &str = "Created";
    pub const QUEUED: &str = "Queued";
    pub const STARTING: &str = "Starting";
    pub const RUNNING: &str = "Running";
    pub const AWAITING_REVIEW: &str = "AwaitingReview";
    /// 승인 또는 폐기가 worktree 부작용을 실행 중인 과도 상태. 같은 작업의 중복 완료 처리를 막는다.
    pub const FINALIZING: &str = "Finalizing";
    pub const DONE: &str = "Done";
    pub const FAILED: &str = "Failed";
    pub const DISCARDED: &str = "Discarded";
    /// 외부기원(봇/크론) 작업의 초기 상태 — 사용자가 UI에서 승인해야 에이전트가 spawn된다.
    pub const PENDING_APPROVAL: &str = "PendingApproval";

    /// 더 이상 워크트리를 필요로 하지 않는 상태 — 승인·폐기가 이미 정리를 마쳤다.
    /// 이 상태에서 워크트리가 없는 것은 고아가 아니라 정상이다.
    pub fn is_terminal(state: &str) -> bool {
        matches!(state, DONE | FAILED | DISCARDED)
    }
}

/// `AwaitingReview`가 왜 대기 중인지에 대한 표시용 주석.
///
/// 별도 state로 나누지 않는 이유: 질문 대기와 결과 검토 대기는 **행동 가능 집합이 동일**하다
/// (승인·폐기·후속 메시지 주입 전부 열림). 상태를 쪼개면 `state != AWAITING_REVIEW` 가드
/// 30여 곳을 모두 고쳐야 하고, 하나만 빠뜨려도 그 작업은 승인 불가로 고착된다.
pub mod awaiting_kind {
    /// 에이전트가 질문을 던지고 답을 기다리는 중. NULL은 통상적인 작업 결과 검토 대기.
    pub const QUESTION: &str = "question";
}

/// `runner_events.kind` 중 **데스크톱이 직접 쓰는** 것들(설계 2026-09-13 D3).
///
/// 러너는 큐가 전이를 기록하지만 데스크톱에는 큐가 없다. 모바일 표면의 라이브 원장
/// (`/v1/events/live`)과 Web Push가 이 테이블 하나만 보므로, 데스크톱도 같은 이름으로
/// 써 넣어야 폰이 깨어난다.
pub mod runner_event_kind {
    /// 검토 대기 진입 — 결과를 볼 사람이 필요하다.
    pub const AWAITING_REVIEW: &str = "awaiting_review";
    /// 에이전트가 질문하고 멈춤 — 답이 없으면 대화가 이어지지 않는다.
    pub const AWAITING_ANSWER: &str = "awaiting_answer";
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Task {
    pub id: i64,
    pub repo: String,
    pub branch: String,
    pub base: String,
    pub worktree_path: String,
    pub instruction: String,
    pub state: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// 이 작업을 수행한 리드 에이전트 (앙상블 후보 구분용). 구버전 행은 NULL.
    pub agent: Option<String>,
    /// 작업 책임 역할. 구버전 행과 역할 미전달 클라이언트는 implementer로 보강된다.
    pub role: String,
    /// 앙상블 그룹 id — 같은 지시문을 여러 에이전트가 수행한 후보들을 묶는다. 단일 작업은 NULL.
    pub ensemble: Option<String>,
    /// 세션(작업) 단위 모델 오버라이드 — 생성 시 선택. NULL이면 설정의 벤더 기본(`model:<agent>`)을 따른다.
    pub model: Option<String>,
    /// Codex 세션 단위 reasoning override. NULL이면 Codex 설정 기본값을 따른다.
    pub reasoning_effort: Option<String>,
    /// Session speed override. NULL preserves the CLI default for existing tasks.
    pub service_tier: Option<String>,
    /// 실행 모드 — "terminal"(PTY) 또는 "conversation"(stream-json). 구버전 행은 'terminal'.
    pub mode: String,
    /// 대화 모드 claude session_id — 앱 재시작 후에도 `--resume`으로 맥락 유지. 터미널 작업은 NULL.
    pub convo_session_id: Option<String>,
    /// 대화 모드 진행 중 turn의 벤더 프로세스 그룹 id(pgid). 스폰 시 기록, 완료/중단 시 NULL.
    /// 앱 재시작 시 생존 확인(신호 0)으로 고아 여부를 판별하는 근거. 비대화/미실행은 NULL.
    pub convo_pgid: Option<i64>,
    /// 명시적으로 생성된 immutable Goal Contract v1. NULL은 레거시 instruction-only 의미를 보존한다.
    pub goal_contract: Option<sqlx::types::Json<GoalContract>>,
    /// 인터뷰 결정화 시점의 모호성 점수 — 생성 시 1회 기록, 이후 UPDATE 없음. 인터뷰 미사용은 NULL.
    pub ambiguity: Option<sqlx::types::Json<crate::interview::AmbiguityScore>>,
    /// 검토 대기의 성격(`awaiting_kind::*`). AwaitingReview가 아닌 상태에서는 항상 NULL —
    /// 턴 시작(Running 전이)에서 지우고 턴 종료에서 다시 쓴다.
    pub awaiting_kind: Option<String>,
    /// 큐에 있으나 지금 시작할 수 없는 이유(`blocked::*`). NULL이면 정상 대기다.
    ///
    /// 상태를 쪼개는 대신 컬럼을 쓰는 이유는 `awaiting_kind`와 같다 — 새 state를 넣으면
    /// `state != …` 가드를 모두 고쳐야 하고 하나만 빠뜨려도 그 작업은 고착된다. 여기서는
    /// lease 쿼리 한 곳만 막으므로, 가드를 빠뜨려도 작업이 죽지 않고 그냥 실행된다.
    /// 차단이 풀리면 컬럼만 비우면 되고 재개 전이를 따로 쓰지 않는다 — 큐를 떠난 적이 없다.
    pub blocked_reason: Option<String>,
    /// diff 기준점의 불변 commit SHA. `base`가 "어디로 머지되나"라면 이쪽은 "어디서 갈라졌나"다.
    ///
    /// 둘을 갈라 두는 이유는 하나가 둘을 겸할 수 없어서다 — 머지 목적지는 브랜치 이름이어야
    /// 하고(`ensure_base_is_checked_out`), diff 기준점은 움직이면 안 된다. 겸하게 두면 base가
    /// 전진할 때 이미 검토한 변경과 거기 붙은 주석이 함께 사라진다.
    ///
    /// 구버전 행과 non-git 직접 실행은 NULL — `diff_base()`가 레거시 경로로 처리한다.
    pub base_revision: Option<String>,
    /// 컨텍스트 절단이 남긴 **미소비 핸드오프 캡슐**. NULL이 정상 상태다.
    ///
    /// 절단은 파일이 아니라 이 칸에 캡슐을 적고, 다음 턴의 프롬프트가 앞에 붙여 읽는다
    /// (ADR 0170). 읽기와 지우기가 갈라져 있다 — 지우는 것은 새 벤더 세션이 확립될 때뿐이다
    /// (`set_convo_session`). 전송 직전에 지우면 벤더 스폰 실패·인터럽트에서 캡슐이 사라지는데,
    /// 그때 세션은 이미 끊긴 뒤라 되돌릴 방법이 없다.
    pub pending_capsule: Option<String>,
    /// 이 작업이 이어받은 원본 작업 id. NULL이 정상이다 — 처음부터 시작한 작업.
    ///
    /// 종결된 대화는 되살리지 않는다. 이어받기는 **새 작업이 옛 벤더 세션을 물려받는 것**이고
    /// (`convo_session_id` 승계), 이 칸은 그 물려받음의 유일한 기록이다. 끊어 두면 새 작업의
    /// 대화창이 첫 턴부터 시작해 보여 — 벤더는 이어가는데 화면만 기억을 잃는다.
    ///
    /// 원본 행은 손대지 않는다. 종결 상태·diff 기준점·주석이 그대로 남아야 무엇을 이어받았는지
    /// 나중에도 확인할 수 있다.
    pub resumed_from: Option<i64>,
    /// 이 작업이 물려받은 **벤더 세션 id**(외부 승계, 세션홈에서 직접 고른 경우). NULL이 정상이다.
    ///
    /// `convo_session_id`도 승계 시점에 같은 값으로 채워지지만, 턴 에필로그가 매 턴 그 칸을
    /// 덮어쓰므로(`set_convo_session`) 시간이 지나면 승계했다는 사실만 남고 원래 물려받은 id는
    /// 사라진다. 이 칸은 덮이지 않는 불변 출처다 — 중복 승계 가드(`adopt_external_session`·
    /// `live_task_with_session`)의 술어가 이 칸도 함께 보는 이유이기도 하다(설계 2026-09-17 결정 5).
    #[sqlx(default)]
    #[serde(default)]
    pub resumed_session: Option<String>,
    /// `worktree_path`가 실재하지 않는 비종료 작업 — DB 컬럼이 아니라 조회 시점의 관측이다.
    ///
    /// 워크트리는 앱 밖에서도 사라진다(수동 `rm`, 다른 도구의 `git worktree prune`). 그래도 DB
    /// 상태는 `AwaitingReview`로 남아 카드가 정상처럼 보이고, 열어서 메시지를 보내는 순간에야
    /// 가드에 걸린다. 목록 조회에서 미리 관측해 두면 열기 전에 알 수 있다.
    /// 종료 상태는 워크트리가 없는 것이 정상이므로 항상 false다.
    #[sqlx(default)]
    #[serde(default)]
    pub worktree_missing: bool,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OpenDirectTask {
    pub id: i64,
    pub repo: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TaskProcessReceipt {
    pub id: i64,
    pub task_id: i64,
    pub pgid: i64,
    pub identity_hash: String,
    pub process_kind: String,
    pub created_at: i64,
}

fn validate_loaded_task(task: Task) -> anyhow::Result<Task> {
    if let Some(contract) = task.goal_contract.as_deref() {
        contract.validate().map_err(anyhow::Error::msg)?;
    }
    Ok(task)
}

fn validate_loaded_tasks(tasks: Vec<Task>) -> anyhow::Result<Vec<Task>> {
    tasks.into_iter().map(validate_loaded_task).collect()
}

const MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS tasks (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  repo          TEXT NOT NULL,
  branch        TEXT NOT NULL,
  base          TEXT NOT NULL,
  worktree_path TEXT NOT NULL,
  instruction   TEXT NOT NULL,
  state         TEXT NOT NULL,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL,
  agent         TEXT,
  role          TEXT NOT NULL DEFAULT 'implementer',
  ensemble      TEXT,
  model         TEXT,
  reasoning_effort TEXT,
  service_tier TEXT CHECK(service_tier IS NULL OR service_tier IN ('default', 'fast')),
  mode          TEXT NOT NULL DEFAULT 'terminal',
  convo_session_id TEXT,
  convo_pgid    INTEGER,
  goal_contract TEXT,
  ambiguity     TEXT,
  awaiting_kind TEXT,
  blocked_reason TEXT
);
"#;

/// WAL 모드 SQLite 풀 초기화 + 마이그레이션.
/// `ALTER TABLE <table> ADD COLUMN <column>` 한 건. 이미 있으면 넘기고, **그 밖의 실패는 올린다.**
///
/// 실패를 통째로 삼키면 컬럼이 빠진 채 앱이 돈다. 그러면 조회가 런타임에야 깨지고, 그 시점에는
/// 원인이 마이그레이션이라는 단서가 아무 데도 남아 있지 않다.
///
/// executor를 제네릭으로 받는 이유는 호출자가 커넥션을 고정할 수 있어야 하기 때문이다 —
/// `CREATE`와 `ALTER`를 같은 커넥션에서 끝내면 그 커넥션의 스키마 뷰가 도중에 갈라지지 않는다.
pub(crate) async fn add_column_if_missing<'e, E>(
    executor: E,
    table: &str,
    column: &str,
) -> anyhow::Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    match sqlx::query(&format!("ALTER TABLE {table} ADD COLUMN {column}"))
        .execute(executor)
        .await
    {
        Ok(_) => Ok(()),
        // "duplicate column name: x" — 이미 채워진 DB다. 무시해도 되는 실패는 이것뿐이다.
        Err(sqlx::Error::Database(db)) if db.message().contains("duplicate column name") => Ok(()),
        Err(e) => Err(anyhow::anyhow!(
            "{table}에 {column} 컬럼을 더하지 못했습니다: {e}"
        )),
    }
}

pub async fn init_pool(db_path: &str) -> anyhow::Result<SqlitePool> {
    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        // WAL + NORMAL: 커밋마다 fsync하지 않는다(체크포인트 시점만). 고빈도 출력 영속화가
        // 커밋당 fsync로 디스크 I/O를 포화시키는 것을 방지 — 전원 단절 시 최근 커밋 일부가
        // 유실될 수 있으나 WAL에서는 DB 파손 없음(출력 로그 성격상 허용).
        .synchronous(SqliteSynchronous::Normal);
    let pool = SqlitePoolOptions::new()
        // 5는 이 앱의 동시성에 비해 너무 좁았다. 세션을 하나 여는 것만으로 `convo_history`·
        // `task_tool_cost`·트리·diff·토론 자리가 함께 나가고, 그 위에 알림 스냅샷(750ms)·
        // 크론 틱·채널 폴링 같은 상시 루프가 얹힌다. 여기에 지식 검색처럼 **초 단위**로
        // 걸리는 조회가 둘만 겹치면 다섯 자리가 모두 차고, 남은 요청은 sqlx 기본
        // acquire_timeout(30초)을 다 기다린 뒤 `pool timed out while waiting for an open
        // connection`으로 죽는다. 죽는 쪽은 원인이 아니라 **줄을 늦게 선 쪽**이라, 증상이
        // 매번 다른 화면에서 나타난다.
        //
        // WAL에서 읽기는 서로 막지 않으므로 읽기 자리를 늘리는 것은 안전하다. 쓰기는 어차피
        // SQLite가 한 번에 하나로 직렬화하고, 못 잡으면 busy_timeout(sqlx 기본 5초)을 기다린다.
        // 커넥션당 스레드가 하나 뜨므로 무한정 늘리지는 않는다.
        .max_connections(16)
        .connect_with(opts)
        .await?;
    sqlx::query(MIGRATION).execute(&pool).await?;
    // 구버전 DB 보강: agent/ensemble 컬럼 추가(이미 있으면 에러 무시 — 멱등).
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN agent TEXT")
        .execute(&pool)
        .await;
    let (role_columns,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name = 'role'")
            .fetch_one(&pool)
            .await?;
    if role_columns == 0 {
        sqlx::query("ALTER TABLE tasks ADD COLUMN role TEXT NOT NULL DEFAULT 'implementer'")
            .execute(&pool)
            .await?;
    }
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN ensemble TEXT")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN model TEXT")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN reasoning_effort TEXT")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN service_tier TEXT CHECK(service_tier IS NULL OR service_tier IN ('default', 'fast'))")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN mode TEXT NOT NULL DEFAULT 'terminal'")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN convo_session_id TEXT")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN convo_pgid INTEGER")
        .execute(&pool)
        .await;
    let (goal_contract_columns,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name = 'goal_contract'",
    )
    .fetch_one(&pool)
    .await?;
    if goal_contract_columns == 0 {
        sqlx::query("ALTER TABLE tasks ADD COLUMN goal_contract TEXT")
            .execute(&pool)
            .await?;
    }
    let (ambiguity_columns,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name = 'ambiguity'")
            .fetch_one(&pool)
            .await?;
    if ambiguity_columns == 0 {
        sqlx::query("ALTER TABLE tasks ADD COLUMN ambiguity TEXT")
            .execute(&pool)
            .await?;
    }
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN awaiting_kind TEXT")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN blocked_reason TEXT")
        .execute(&pool)
        .await;
    // diff 기준점 — `base`(머지 목적지)와 역할이 다르다. 구버전 행은 NULL(레거시 경로).
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN base_revision TEXT")
        .execute(&pool)
        .await;
    // 컨텍스트 절단이 남긴 미소비 캡슐(ADR 0170). CREATE_TABLE에는 넣지 않는다 —
    // `base_revision`과 같이 마이그레이션 한 줄로만 관리한다(두 곳에 적으면 어긋날 자리가 는다).
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN pending_capsule TEXT")
        .execute(&pool)
        .await;
    // 이어받기 원본 — 종결 작업의 벤더 세션을 물려받은 새 작업만 채운다. 구버전 행은 NULL.
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN resumed_from INTEGER")
        .execute(&pool)
        .await;
    // 외부 승계 출처(세션홈에서 직접 고른 벤더 세션 id) — 설계 2026-09-17 결정 5. `convo_session_id`가
    // 매 턴 덮이므로 이 칸이 불변 출처로 남는다. 구버전 행은 NULL.
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN resumed_session TEXT")
        .execute(&pool)
        .await;
    add_column_if_missing(&pool, "tasks", "resumed_vendor TEXT").await?;
    sqlx::query("UPDATE tasks SET resumed_vendor = 'claude' WHERE resumed_session IS NOT NULL AND resumed_vendor IS NULL")
        .execute(&pool).await?;
    // 대화 이벤트 영속화 — 재진입/재시작 시 트랜스크립트 복원 (event = ConvoEvent 또는 user 메시지 JSON).
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS convo_events (\
           id      INTEGER PRIMARY KEY AUTOINCREMENT, \
           task_id INTEGER NOT NULL, \
           ts      INTEGER NOT NULL, \
           event   TEXT NOT NULL)",
    )
    .execute(&pool)
    .await?;
    // 되감기 표시 — 논리 삭제다. 물리 삭제하면 되돌릴 근거가 사라진다.
    // NULL이면 살아 있는 이벤트. 조회 경로는 모두 `rewound_at IS NULL`로 걸러야 한다.
    let _ = sqlx::query("ALTER TABLE convo_events ADD COLUMN rewound_at INTEGER")
        .execute(&pool)
        .await;
    // 형제 테이블(`runner_events`·`task_output`)은 처음부터 `(task_id, sequence)` 인덱스를
    // 가졌는데 여기만 빠져 있었다. 이 테이블의 조회는 **전부** `WHERE task_id = ? ... ORDER BY id`
    // 형태인데 인덱스가 없으면 매번 테이블 전체를 훑는다 — 실사용 DB에서 318,351행·90MB였고,
    // `EXPLAIN QUERY PLAN`이 `SCAN convo_events`를 냈다. 트랜스크립트를 한 번 여는 동작이
    // 커넥션을 초 단위로 붙잡아 풀(5개)을 마르게 하고, 엉뚱한 조회가
    // `pool timed out while waiting for an open connection`으로 죽는다.
    //
    // `rewound_at`을 키에 넣지 않는 이유: task 하나당 평균 400행이라 task_id로 좁힌 뒤의
    // NULL 판정은 공짜에 가깝고, 키가 짧을수록 INSERT(스트림 청크마다 발생)가 싸다.
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_convo_events_task_id ON convo_events(task_id, id)",
    )
    .execute(&pool)
    .await?;
    // 보존 정리(`prune_runner_history`)만 `ts`로 지운다 — 이쪽도 없으면 삭제가 전체 스캔이고,
    // 쓰기 잠금을 쥔 채 그 시간을 다 쓴다.
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_convo_events_ts ON convo_events(ts)")
        .execute(&pool)
        .await?;
    // 대화 체크포인트 — 파일(worktree 커밋)과 대화(이벤트 경계)를 **함께** 잡는다.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS convo_checkpoints (\
           id                 INTEGER PRIMARY KEY AUTOINCREMENT, \
           task_id            INTEGER NOT NULL, \
           label              TEXT    NOT NULL, \
           worktree_commit    TEXT    NOT NULL, \
           convo_event_max_id INTEGER NOT NULL, \
           ts                 INTEGER NOT NULL)",
    )
    .execute(&pool)
    .await?;
    // 토론 세션의 **우측 한 행**. 좌측(`tasks.agent`·`model`·`convo_session_id`)을 이중화하지
    // 않는다 — 두 원천이 생기면 전환·턴 에필로그가 한쪽만 갱신해 어긋난다(설계 §4-1).
    // "토론 중"은 `status` 컬럼이 아니라 **이 행의 존재**로 파생된다. 자리마다 한 행이다
    // (`right`, 3자면 `third`까지) — PK에 `side`를 둔 덕에 3자 토론에 마이그레이션이 없었다.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS convo_debate_sides (\
           task_id           INTEGER NOT NULL, \
           side              TEXT    NOT NULL, \
           agent             TEXT    NOT NULL, \
           model             TEXT, \
           vendor_session_id TEXT, \
           PRIMARY KEY(task_id, side))",
    )
    .execute(&pool)
    .await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
        .execute(&pool)
        .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS task_events (\
           id      INTEGER PRIMARY KEY AUTOINCREMENT, \
           task_id INTEGER NOT NULL, \
           ts      INTEGER NOT NULL, \
           kind    TEXT NOT NULL, \
           detail  TEXT)",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_task_events_followup_observation \
         ON task_events(task_id, kind) \
         WHERE kind IN ('followup_observation_started', 'user_followup_input_observed')",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS runner_events (\
           sequence INTEGER PRIMARY KEY AUTOINCREMENT, \
           task_id  INTEGER NOT NULL, \
           ts       INTEGER NOT NULL, \
           kind     TEXT NOT NULL, \
           detail   TEXT)",
    )
    .execute(&pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_runner_events_task_sequence ON runner_events(task_id, sequence)")
        .execute(&pool)
        .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS task_output (\
           sequence INTEGER PRIMARY KEY AUTOINCREMENT, \
           task_id  INTEGER NOT NULL, \
           ts       INTEGER NOT NULL, \
           data     TEXT NOT NULL)",
    )
    .execute(&pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_task_output_task_sequence ON task_output(task_id, sequence)")
        .execute(&pool)
        .await?;
    // task_start_receipts·task_start_receipt_checks(옛 시작 영수증 원장)는 더는 만들지 않는다 — 검증할 메모리
    // 투영 원장(memory_projection_journal)이 파일형 메모리(P1)로 대체되며 사라졌고,
    // 시작 승격은 이제 영수증 없이 항상 통과한다(runner::queue::verify_and_promote_start).
    // 기존 사용자 DB에 남은 표는 지우지 않는다(ADR: 데이터 삭제는 사용자 명령으로만).
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS task_process_receipts (\
           id INTEGER PRIMARY KEY AUTOINCREMENT, \
           task_id INTEGER NOT NULL, \
           pgid INTEGER NOT NULL, \
           identity_hash TEXT NOT NULL, \
           process_kind TEXT NOT NULL, \
           created_at INTEGER NOT NULL)",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS task_process_receipts_immutable \
         BEFORE UPDATE ON task_process_receipts BEGIN \
           SELECT RAISE(ABORT, 'task process receipt is immutable'); \
         END",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TRIGGER IF NOT EXISTS task_process_receipts_no_delete \
         BEFORE DELETE ON task_process_receipts BEGIN \
           SELECT RAISE(ABORT, 'task process receipt cannot be deleted'); \
         END",
    )
    .execute(&pool)
    .await?;
    sqlx::query("DROP TABLE IF EXISTS challenges")
    .execute(&pool)
    .await?;
        sqlx::query(
        "CREATE TABLE IF NOT EXISTS evidence (\
           task_id    INTEGER PRIMARY KEY, \
           build_cmd  TEXT, build_exit INTEGER, \
           test_cmd   TEXT, test_exit  INTEGER, \
           passed     INTEGER NOT NULL DEFAULT 0, \
           failed     INTEGER NOT NULL DEFAULT 0, \
           ready      INTEGER NOT NULL, \
           created_at INTEGER NOT NULL)",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS preview_commands (\
           id         INTEGER PRIMARY KEY, \
           ts         INTEGER NOT NULL, \
           task_id    INTEGER NOT NULL, \
           op         TEXT NOT NULL, \
           ok         INTEGER NOT NULL, \
           changed    INTEGER, \
           elapsed_ms INTEGER NOT NULL, \
           error      TEXT)",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_preview_commands_task \
         ON preview_commands(task_id, ts)",
    )
    .execute(&pool)
    .await?;
    // 스냅샷 계측 — 이 컬럼들이 생기기 전 행은 NULL로 남는다. 집계가 COUNT(snapshot_bytes)로
    // 스냅샷 건수를 따로 세므로, 옛 행이 평균을 0으로 끌어내리지 않는다.
    for column in [
        "snapshot_bytes INTEGER",
        "snapshot_nodes INTEGER",
        "snapshot_truncated INTEGER",
        "snapshot_shrinks INTEGER",
    ] {
        add_column_if_missing(&pool, "preview_commands", column).await?;
    }
    crate::runner::review_process::migrate(&pool).await?;
    crate::decision::migrate(&pool).await?;
    crate::notifications::migrate(&pool).await?;
    // 크론 스케줄러(schedules) 마이그레이션은 schedule::migrate로 분리 — lib.rs `.setup()`에서 호출.
    // 구 텔레그램 테이블(channels/channel_secrets)은 2026-09-13 제거 — 기존 DB에 남아도 무해.
    crate::convo::interaction::migrate(&pool).await.map_err(anyhow::Error::msg)?;
    // 2026-09-26 제거된 attention(할 일) 기능의 잔재를 걷는다. 트리거는 tasks·interaction
    // 갱신마다 attention_* 테이블에 쓰므로, 테이블만 지우면 이후 모든 작업 갱신이 "no such table"로
    // 죽는다 — 트리거를 먼저, 테이블을 그다음에 지운다. 둘 다 IF EXISTS라 새 DB에서도 무해하다.
    for statement in [
        "DROP TRIGGER IF EXISTS attention_task_after_update",
        "DROP TRIGGER IF EXISTS attention_task_resumed",
        "DROP TRIGGER IF EXISTS attention_interaction_open",
        "DROP TRIGGER IF EXISTS attention_interaction_close",
        "DROP TABLE IF EXISTS attention_dismissals",
        "DROP TABLE IF EXISTS attention_failure_handled",
        "DROP TABLE IF EXISTS attention_baselines",
        "DROP TABLE IF EXISTS attention_occurrences",
        "DROP TABLE IF EXISTS attention_condition_states",
    ] {
        sqlx::query(statement).execute(&pool).await?;
    }
    crate::task_results::migrate(&pool).await?;
    crate::project_plan::migrate(&pool).await?;
    crate::pipeline::db::migrate(&pool).await?;
    crate::task_review::migrate(&pool).await?;
    Ok(pool)
}

/// 작업 이벤트 (append-only ledger) — Capsule 최근활동 + 향후 thrash 보강.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TaskEvent {
    pub id: i64,
    pub task_id: i64,
    pub ts: i64,
    pub kind: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RunnerEvent {
    pub sequence: i64,
    pub task_id: i64,
    pub ts: i64,
    pub kind: String,
    pub detail: Option<String>,
}

/// Runner가 보존한 작업 표준 출력 조각. 전역 sequence로 재접속 replay 경계를 만든다.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TaskOutput {
    pub sequence: i64,
    pub task_id: i64,
    pub ts: i64,
    pub data: String,
}

pub async fn append_runner_event(
    pool: &SqlitePool,
    task_id: i64,
    ts: i64,
    kind: &str,
    detail: Option<&str>,
) -> anyhow::Result<i64> {
    let result =
        sqlx::query("INSERT INTO runner_events (task_id, ts, kind, detail) VALUES (?, ?, ?, ?)")
            .bind(task_id)
            .bind(ts)
            .bind(kind)
            .bind(detail)
            .execute(pool)
            .await?;
    Ok(result.last_insert_rowid())
}

/// 상태 전이와 Runner event를 한 SQLite 트랜잭션으로 기록한다.
///
/// 큐 worker는 이 API를 통해 replay가 상태 변경을 놓치지 않게 한다.
pub async fn transition_state_with_runner_event(
    pool: &SqlitePool,
    task_id: i64,
    new_state: &str,
    ts: i64,
    kind: &str,
    detail: Option<&str>,
) -> anyhow::Result<i64> {
    let mut transaction = pool.begin().await?;
    sqlx::query("UPDATE tasks SET state = ?, updated_at = ? WHERE id = ?")
        .bind(new_state)
        .bind(ts)
        .bind(task_id)
        .execute(&mut *transaction)
        .await?;
    let event =
        sqlx::query("INSERT INTO runner_events (task_id, ts, kind, detail) VALUES (?, ?, ?, ?)")
            .bind(task_id)
            .bind(ts)
            .bind(kind)
            .bind(detail)
            .execute(&mut *transaction)
            .await?;
    transaction.commit().await?;
    Ok(event.last_insert_rowid())
}

/// 아직 Running인 작업만 종료 상태로 전이하고 replay event를 함께 기록한다.
pub async fn finish_running_task(
    pool: &SqlitePool,
    task_id: i64,
    new_state: &str,
    ts: i64,
    kind: &str,
    detail: Option<&str>,
) -> anyhow::Result<bool> {
    let mut transaction = pool.begin().await?;
    let updated = sqlx::query(
        "UPDATE tasks SET state = ?, updated_at = ?, convo_pgid = NULL \
             WHERE id = ? AND state = ?",
    )
    .bind(new_state)
    .bind(ts)
    .bind(task_id)
    .bind(state::RUNNING)
    .execute(&mut *transaction)
    .await?;
    if updated.rows_affected() == 0 {
        transaction.commit().await?;
        return Ok(false);
    }
    sqlx::query("INSERT INTO runner_events (task_id, ts, kind, detail) VALUES (?, ?, ?, ?)")
        .bind(task_id)
        .bind(ts)
        .bind(kind)
        .bind(detail)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(true)
}

/// Running 상태가 실제로 끝난 경우에만 결과 원천도 같은 트랜잭션으로 적재한다.
/// 취소 의도가 먼저 커밋된 경우에는 상태 이벤트만 남기고 알림 결과는 만들지 않는다.
pub async fn finish_running_task_with_notification(
    pool: &SqlitePool,
    task_id: i64,
    new_state: &str,
    ts: i64,
    event_kind: &str,
    detail: Option<&str>,
    notification_kind: &str,
) -> anyhow::Result<bool> {
    let mut transaction = pool.begin().await?;
    let updated = sqlx::query(
        "UPDATE tasks SET state = ?, updated_at = ?, convo_pgid = NULL WHERE id = ? AND state = ?",
    )
    .bind(new_state).bind(ts).bind(task_id).bind(state::RUNNING).execute(&mut *transaction).await?;
    if updated.rows_affected() == 0 { transaction.commit().await?; return Ok(false); }
    sqlx::query("INSERT INTO runner_events (task_id, ts, kind, detail) VALUES (?, ?, ?, ?)")
        .bind(task_id).bind(ts).bind(event_kind).bind(detail).execute(&mut *transaction).await?;
    crate::notifications::record_result_tx(&mut transaction, task_id, ts, notification_kind).await?;
    transaction.commit().await?;
    Ok(true)
}

/// 로컬 실행 종료도 상태 전이와 결과 기록을 분리하지 않는다.
pub async fn mark_awaiting_review_with_notification(
    pool: &SqlitePool,
    id: i64,
    now: i64,
    awaiting: Option<&str>,
    notification_kind: &str,
) -> anyhow::Result<bool> {
    let mut transaction = pool.begin().await?;
    let updated = sqlx::query(
        "UPDATE tasks SET state = ?, updated_at = ?, awaiting_kind = ? WHERE id = ? AND state IN (?, ?)",
    )
    .bind(state::AWAITING_REVIEW).bind(now).bind(awaiting).bind(id)
    .bind(state::RUNNING).bind(state::CREATED).execute(&mut *transaction).await?;
    if updated.rows_affected() > 0 {
        crate::notifications::record_result_tx(&mut transaction, id, now, notification_kind).await?;
    }
    transaction.commit().await?;
    Ok(updated.rows_affected() > 0)
}

/// Signal 이전에 취소 의도를 durable DB에 남긴다. 다음 시작 전이는 이 기록을 지운다.
pub async fn record_notification_cancel_intent(pool: &SqlitePool, task_id: i64) -> anyhow::Result<bool> {
    crate::notifications::cancel_intent(pool, task_id).await
}

pub async fn clear_notification_cancel_if_signal_not_sent(
    pool: &SqlitePool,
    task_id: i64,
) -> anyhow::Result<()> {
    crate::notifications::clear_cancel_if_signal_not_sent(pool, task_id).await
}

/// 스폰 전에 실패한 로컬 작업도 상태 변경과 실패 결과를 함께 남긴다.
pub async fn fail_created_task_with_notification(
    pool: &SqlitePool,
    task_id: i64,
    ts: i64,
) -> anyhow::Result<bool> {
    let mut transaction = pool.begin().await?;
    let updated = sqlx::query("UPDATE tasks SET state = ?, updated_at = ? WHERE id = ? AND state = ?")
        .bind(state::FAILED).bind(ts).bind(task_id).bind(state::CREATED).execute(&mut *transaction).await?;
    if updated.rows_affected() > 0 {
        crate::notifications::record_result_tx(&mut transaction, task_id, ts, "failure").await?;
    }
    transaction.commit().await?;
    Ok(updated.rows_affected() > 0)
}

pub async fn list_runner_events_after(
    pool: &SqlitePool,
    sequence: i64,
    limit: i64,
) -> anyhow::Result<Vec<RunnerEvent>> {
    sqlx::query_as::<_, RunnerEvent>(
        "SELECT sequence, task_id, ts, kind, detail FROM runner_events WHERE sequence > ? ORDER BY sequence ASC LIMIT ?",
    )
    .bind(sequence)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// Runner event replay watermark. 이벤트가 아직 없으면 0이며, client cursor의 초기값으로 쓴다.
pub async fn latest_runner_event_sequence(pool: &SqlitePool) -> anyhow::Result<i64> {
    let (sequence,): (i64,) =
        sqlx::query_as("SELECT COALESCE(MAX(sequence), 0) FROM runner_events")
            .fetch_one(pool)
            .await?;
    Ok(sequence)
}

/// 가장 최근 Runner event의 기록 시각. 이벤트가 없으면 None.
/// 모바일 상태 배너의 "마지막 신호 N분 전"이 이 값을 읽는다 — Runner가 살아 있는지
/// 여부와 별개로 **일을 하고 있는지**를 보여주는 유일한 신호다. (설계 0013 §7.2)
pub async fn latest_runner_event_ts(pool: &SqlitePool) -> anyhow::Result<Option<i64>> {
    let (ts,): (Option<i64>,) = sqlx::query_as("SELECT MAX(ts) FROM runner_events")
        .fetch_one(pool)
        .await?;
    Ok(ts)
}

/// 주어진 상태의 task 수. 상태 문자열은 `state` 상수만 넘긴다.
pub async fn count_tasks_in_state(pool: &SqlitePool, state: &str) -> anyhow::Result<i64> {
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tasks WHERE state = ?")
        .bind(state)
        .fetch_one(pool)
        .await?;
    Ok(count)
}

/// `after < sequence <= watermark` 범위를 오름차순으로 읽는다. WebSocket 구독은 이
/// snapshot을 먼저 보낸 뒤 watermark보다 큰 live event만 전송한다.
pub async fn list_runner_events_through(
    pool: &SqlitePool,
    after: i64,
    watermark: i64,
    limit: i64,
) -> anyhow::Result<Vec<RunnerEvent>> {
    sqlx::query_as::<_, RunnerEvent>(
        "SELECT sequence, task_id, ts, kind, detail FROM runner_events \
         WHERE sequence > ? AND sequence <= ? ORDER BY sequence ASC LIMIT ?",
    )
    .bind(after)
    .bind(watermark)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn append_task_output(
    pool: &SqlitePool,
    task_id: i64,
    ts: i64,
    data: &str,
) -> anyhow::Result<i64> {
    let result = sqlx::query("INSERT INTO task_output (task_id, ts, data) VALUES (?, ?, ?)")
        .bind(task_id)
        .bind(ts)
        .bind(data)
        .execute(pool)
        .await?;
    Ok(result.last_insert_rowid())
}

/// 출력 조각과 replay event를 함께 기록한다.
///
/// `runner_events.sequence`가 Desktop 재접속의 전역 cursor이며, `task_output`은 원시
/// 출력 보존·task별 조회를 맡는다. 둘 중 하나만 남는 상태를 만들지 않는다.
pub async fn append_task_output_with_runner_event(
    pool: &SqlitePool,
    task_id: i64,
    ts: i64,
    data: &str,
) -> anyhow::Result<(i64, i64)> {
    let mut transaction = pool.begin().await?;
    let output = sqlx::query("INSERT INTO task_output (task_id, ts, data) VALUES (?, ?, ?)")
        .bind(task_id)
        .bind(ts)
        .bind(data)
        .execute(&mut *transaction)
        .await?;
    let event = sqlx::query(
        "INSERT INTO runner_events (task_id, ts, kind, detail) VALUES (?, ?, 'output', ?)",
    )
    .bind(task_id)
    .bind(ts)
    .bind(data)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok((output.last_insert_rowid(), event.last_insert_rowid()))
}

/// convo 이벤트 + 원시 출력 + runner replay 이벤트를 한 트랜잭션으로 기록한다.
/// Runner conversation 스트리밍은 라인마다 이 세 기록이 모두 필요한데, 별도 호출로 나누면
/// 라인당 커밋이 2회가 되어 디스크 I/O를 배가시킨다 — 원자성(셋 중 일부만 남는 상태 방지)도 함께 확보.
pub async fn append_convo_event_with_runner_output(
    pool: &SqlitePool,
    task_id: i64,
    event_json: &str,
    ts: i64,
) -> anyhow::Result<()> {
    let mut transaction = pool.begin().await?;
    sqlx::query("INSERT INTO convo_events (task_id, ts, event) VALUES (?, ?, ?)")
        .bind(task_id)
        .bind(ts)
        .bind(event_json)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("INSERT INTO task_output (task_id, ts, data) VALUES (?, ?, ?)")
        .bind(task_id)
        .bind(ts)
        .bind(event_json)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("INSERT INTO runner_events (task_id, ts, kind, detail) VALUES (?, ?, 'output', ?)")
        .bind(task_id)
        .bind(ts)
        .bind(event_json)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

pub async fn list_task_output_after(
    pool: &SqlitePool,
    sequence: i64,
    limit: i64,
) -> anyhow::Result<Vec<TaskOutput>> {
    sqlx::query_as::<_, TaskOutput>(
        "SELECT sequence, task_id, ts, data FROM task_output WHERE sequence > ? ORDER BY sequence ASC LIMIT ?",
    )
    .bind(sequence)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn list_task_output_for_task_after(
    pool: &SqlitePool,
    task_id: i64,
    sequence: i64,
    limit: i64,
) -> anyhow::Result<Vec<TaskOutput>> {
    sqlx::query_as::<_, TaskOutput>(
        "SELECT sequence, task_id, ts, data FROM task_output WHERE task_id = ? AND sequence > ? ORDER BY sequence ASC LIMIT ?",
    )
    .bind(task_id)
    .bind(sequence)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// 검토 대기 conversation task를 후속 턴 메시지로 재큐잉한다.
///
/// instruction을 새 메시지로 교체하고 Queued로 되돌리면 기존 queue worker가
/// `convo_session_id` resume으로 다음 턴을 실행한다. 조건부 UPDATE라
/// 실행 중(Running)이거나 이미 완료 처리된 작업과의 경쟁에서 한 쪽만 성공한다.
pub async fn requeue_conversation_followup(
    pool: &SqlitePool,
    task_id: i64,
    message: &str,
    now: i64,
) -> anyhow::Result<bool> {
    let mut transaction = pool.begin().await?;
    let updated = sqlx::query(
        "UPDATE tasks SET instruction = ?, state = ?, updated_at = ? \
         WHERE id = ? AND mode = 'conversation' AND state = ?",
    )
    .bind(message)
    .bind(state::QUEUED)
    .bind(now)
    .bind(task_id)
    .bind(state::AWAITING_REVIEW)
    .execute(&mut *transaction)
    .await?;
    if updated.rows_affected() == 0 {
        return Ok(false);
    }
    sqlx::query("INSERT INTO runner_events (task_id, ts, kind, detail) VALUES (?, ?, ?, ?)")
        .bind(task_id)
        .bind(now)
        .bind("queued")
        .bind("followup")
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(true)
}

/// Runner receipt acceptance and its actual durable main-turn admission share
/// one transaction. A crash cannot leave an `accepted` receipt without the
/// corresponding queued follow-up (or vice versa).
pub async fn requeue_conversation_followup_receipt(
    pool: &SqlitePool, task_id: i64, request_id: &str, message: &str, now: i64,
) -> anyhow::Result<bool> {
    let mut transaction = pool.begin().await?;
    let updated = sqlx::query("UPDATE tasks SET instruction=?, state=?, updated_at=? WHERE id=? AND mode='conversation' AND state=?")
        .bind(message).bind(state::QUEUED).bind(now).bind(task_id).bind(state::AWAITING_REVIEW).execute(&mut *transaction).await?;
    if updated.rows_affected() == 0 { transaction.commit().await?; return Ok(false); }
    sqlx::query("INSERT INTO runner_events (task_id, ts, kind, detail) VALUES (?, ?, ?, ?)")
        .bind(task_id).bind(now).bind("queued").bind("followup").execute(&mut *transaction).await?;
    sqlx::query("UPDATE conversation_receipts SET status='accepted', error=NULL WHERE task_id=? AND request_id=? AND status='unknown'")
        .bind(task_id).bind(request_id).execute(&mut *transaction).await?;
    transaction.commit().await?;
    Ok(true)
}

pub async fn prune_runner_history(pool: &SqlitePool, before_ts: i64) -> anyhow::Result<u64> {
    let mut transaction = pool.begin().await?;
    let events = sqlx::query("DELETE FROM runner_events WHERE ts < ?")
        .bind(before_ts)
        .execute(&mut *transaction)
        .await?;
    let output = sqlx::query("DELETE FROM task_output WHERE ts < ?")
        .bind(before_ts)
        .execute(&mut *transaction)
        .await?;
    let conversations = sqlx::query("DELETE FROM convo_events WHERE ts < ?")
        .bind(before_ts)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(events.rows_affected() + output.rows_affected() + conversations.rows_affected())
}

/// 이벤트 기록 (best-effort; 실패해도 본 흐름 막지 않음).
pub async fn append_event(
    pool: &SqlitePool,
    task_id: i64,
    kind: &str,
    detail: Option<&str>,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO task_events (task_id, ts, kind, detail) VALUES (?, ?, ?, ?)")
        .bind(task_id)
        .bind(now)
        .bind(kind)
        .bind(detail)
        .execute(pool)
        .await?;
    Ok(())
}

/// 응답에 실린 접근성 스냅샷의 크기. `bytes`는 트리 본문만 잰 것이고, 그것이 에이전트
/// 컨텍스트에 쌓이는 양이다. `shrinks`는 512 KiB 상한에 걸려 상한을 반으로 줄인 횟수(0~2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotMetrics {
    pub bytes: i64,
    pub nodes: i64,
    pub truncated: bool,
    pub shrinks: i64,
}

/// 프리뷰 명령 계측 한 행. `changed`는 해당 op가 변경을 판정하지 않으면 NULL이고,
/// `snapshot`은 스냅샷을 싣지 않는 op(`console`)와 실패 응답에서 None이다.
///
/// 위치 인자가 아니라 구조체로 받는 이유는 필드가 열 개에 가깝고 그중 다섯이 정수·불리언이라,
/// 순서가 어긋나도 타입이 잡아 주지 않기 때문이다.
#[derive(Debug, Clone)]
pub struct PreviewCommandRecord<'a> {
    pub task_id: i64,
    pub op: &'a str,
    pub ok: bool,
    pub changed: Option<bool>,
    pub elapsed_ms: i64,
    pub error: Option<&'a str>,
    pub snapshot: Option<SnapshotMetrics>,
    pub now: i64,
}

pub async fn record_preview_command(
    pool: &SqlitePool,
    record: PreviewCommandRecord<'_>,
) -> anyhow::Result<()> {
    let snapshot = record.snapshot;
    sqlx::query(
        "INSERT INTO preview_commands \
           (ts, task_id, op, ok, changed, elapsed_ms, error, \
            snapshot_bytes, snapshot_nodes, snapshot_truncated, snapshot_shrinks) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(record.now)
    .bind(record.task_id)
    .bind(record.op)
    .bind(i64::from(record.ok))
    .bind(record.changed.map(i64::from))
    .bind(record.elapsed_ms)
    .bind(record.error)
    .bind(snapshot.map(|s| s.bytes))
    .bind(snapshot.map(|s| s.nodes))
    .bind(snapshot.map(|s| i64::from(s.truncated)))
    .bind(snapshot.map(|s| s.shrinks))
    .execute(pool)
    .await?;
    Ok(())
}

/// op 하나에 대한 스냅샷 비용 집계. 평균만 보면 오해한다 — 최댓값과 합계를 함께 둔다.
/// `total_bytes`가 그 task의 대화에 트리가 몇 바이트 쌓였는지이고, 그것이 판단의 기준이다.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PreviewSnapshotCost {
    pub op: String,
    pub calls: i64,
    pub failures: i64,
    pub snapshots: i64,
    pub total_bytes: i64,
    pub max_bytes: i64,
    pub avg_bytes: f64,
    pub avg_nodes: f64,
    pub truncated: i64,
    pub shrinks: i64,
}

/// task 하나의 스냅샷 비용을 op별로 집계한다. 계측 컬럼이 생기기 전 행은 스냅샷 값이 NULL이라
/// `snapshots`에 세어지지 않는다 — `calls`와 `snapshots`가 벌어져 있으면 옛 행이 섞인 것이다.
pub async fn preview_snapshot_cost(
    pool: &SqlitePool,
    task_id: i64,
) -> anyhow::Result<Vec<PreviewSnapshotCost>> {
    let rows = sqlx::query_as::<_, PreviewSnapshotCost>(
        "SELECT op, \
                COUNT(*)                            AS calls, \
                SUM(CASE WHEN ok = 0 THEN 1 ELSE 0 END) AS failures, \
                COUNT(snapshot_bytes)               AS snapshots, \
                COALESCE(SUM(snapshot_bytes), 0)    AS total_bytes, \
                COALESCE(MAX(snapshot_bytes), 0)    AS max_bytes, \
                COALESCE(AVG(snapshot_bytes), 0.0)  AS avg_bytes, \
                COALESCE(AVG(snapshot_nodes), 0.0)  AS avg_nodes, \
                COALESCE(SUM(snapshot_truncated), 0) AS truncated, \
                COALESCE(SUM(snapshot_shrinks), 0)  AS shrinks \
         FROM preview_commands WHERE task_id = ? \
         GROUP BY op ORDER BY total_bytes DESC",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 최근 이벤트 (최신순, limit개).
pub async fn recent_events(
    pool: &SqlitePool,
    task_id: i64,
    limit: i64,
) -> anyhow::Result<Vec<TaskEvent>> {
    // 계측 행(`metric.*`)은 제외한다 — 이 목록은 에이전트 컨텍스트로 들어가고, 거기에
    // 사람이 읽을 일 없는 소요 시간 JSON이 섞이면 맥락만 밀어낸다(ADR 0174).
    let rows = sqlx::query_as::<_, TaskEvent>(
        "SELECT * FROM task_events WHERE task_id = ? AND kind NOT LIKE 'metric.%' \
         ORDER BY id DESC LIMIT ?",
    )
    .bind(task_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 작업에 특정 종류의 이벤트가 한 번이라도 기록됐는지 확인한다.
pub async fn has_task_event(pool: &SqlitePool, task_id: i64, kind: &str) -> anyhow::Result<bool> {
    let (exists,): (bool,) =
        sqlx::query_as("SELECT EXISTS(SELECT 1 FROM task_events WHERE task_id = ? AND kind = ?)")
            .bind(task_id)
            .bind(kind)
            .fetch_one(pool)
            .await?;
    Ok(exists)
}

/// 검증 증거 (작업당 최신 1건).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Evidence {
    pub task_id: i64,
    pub build_cmd: Option<String>,
    pub build_exit: Option<i64>,
    pub test_cmd: Option<String>,
    pub test_exit: Option<i64>,
    pub passed: i64,
    pub failed: i64,
    pub ready: bool,
    pub created_at: i64,
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_evidence(
    pool: &SqlitePool,
    task_id: i64,
    build_cmd: &str,
    build_exit: i64,
    test_cmd: &str,
    test_exit: i64,
    passed: i64,
    failed: i64,
    ready: bool,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO evidence (task_id, build_cmd, build_exit, test_cmd, test_exit, passed, failed, ready, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(task_id) DO UPDATE SET \
           build_cmd=excluded.build_cmd, build_exit=excluded.build_exit, \
           test_cmd=excluded.test_cmd, test_exit=excluded.test_exit, \
           passed=excluded.passed, failed=excluded.failed, ready=excluded.ready, created_at=excluded.created_at",
    )
    .bind(task_id)
    .bind(build_cmd)
    .bind(build_exit)
    .bind(test_cmd)
    .bind(test_exit)
    .bind(passed)
    .bind(failed)
    .bind(ready)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_evidence(pool: &SqlitePool, task_id: i64) -> anyhow::Result<Option<Evidence>> {
    let row = sqlx::query_as::<_, Evidence>("SELECT * FROM evidence WHERE task_id = ?")
        .bind(task_id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// 설정 값 조회 (없으면 None).
pub async fn get_setting(pool: &SqlitePool, key: &str) -> anyhow::Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(v,)| v))
}

/// 설정 값 저장 (upsert).
pub async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

/// 설정 값 삭제 (없으면 무시 — 멱등).
pub async fn delete_setting(pool: &SqlitePool, key: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM settings WHERE key = ?")
        .bind(key)
        .execute(pool)
        .await?;
    Ok(())
}

/// 앱 재시작 복원: Starting/Running Task를 Failed로 마킹 (PRD F-03 R8 토대).
pub async fn mark_stale_running_failed(pool: &SqlitePool, now: i64) -> anyhow::Result<u64> {
    let res = sqlx::query("UPDATE tasks SET state = ?, updated_at = ? WHERE state IN (?, ?)")
        .bind(state::FAILED)
        .bind(now)
        .bind(state::STARTING)
        .bind(state::RUNNING)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// repo 락을 획득한 재시작 복구가 여전히 `Created`인 direct 작업 하나만 실패 처리한다.
pub async fn mark_created_direct_failed(
    pool: &SqlitePool,
    id: i64,
    now: i64,
) -> anyhow::Result<u64> {
    let result = sqlx::query(
        "UPDATE tasks SET state = ?, updated_at = ? \
         WHERE id = ? AND state = ? AND repo = worktree_path",
    )
    .bind(state::FAILED)
    .bind(now)
    .bind(id)
    .bind(state::CREATED)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_task(
    pool: &SqlitePool,
    repo: &str,
    branch: &str,
    base: &str,
    worktree_path: &str,
    instruction: &str,
    agent: Option<&str>,
    ensemble: Option<&str>,
    mode: &str,
    now: i64,
) -> anyhow::Result<i64> {
    insert_task_with_goal_contract(
        pool,
        repo,
        branch,
        base,
        worktree_path,
        instruction,
        agent,
        ensemble,
        None,
        None,
        mode,
        None,
        None,
        now,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_task_with_goal_contract(
    pool: &SqlitePool,
    repo: &str,
    branch: &str,
    base: &str,
    worktree_path: &str,
    instruction: &str,
    agent: Option<&str>,
    ensemble: Option<&str>,
    model: Option<&str>,
    reasoning_effort: Option<&str>,
    mode: &str,
    goal_contract: Option<&GoalContract>,
    ambiguity: Option<&crate::interview::AmbiguityScore>,
    now: i64,
) -> anyhow::Result<i64> {
    insert_task_with_role_and_goal_contract(
        pool,
        repo,
        branch,
        base,
        worktree_path,
        instruction,
        agent,
        crate::agent::DEFAULT_ROLE,
        ensemble,
        model,
        reasoning_effort,
        mode,
        goal_contract,
        ambiguity,
        now,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_task_with_role_and_goal_contract(
    pool: &SqlitePool,
    repo: &str,
    branch: &str,
    base: &str,
    worktree_path: &str,
    instruction: &str,
    agent: Option<&str>,
    role: &str,
    ensemble: Option<&str>,
    model: Option<&str>,
    reasoning_effort: Option<&str>,
    mode: &str,
    goal_contract: Option<&GoalContract>,
    ambiguity: Option<&crate::interview::AmbiguityScore>,
    now: i64,
) -> anyhow::Result<i64> {
    if let Some(contract) = goal_contract {
        contract.validate().map_err(anyhow::Error::msg)?;
    }
    let role = crate::agent::normalize_role_or_default(role).map_err(anyhow::Error::msg)?;
    let id = sqlx::query(
        "INSERT INTO tasks (repo, branch, base, worktree_path, instruction, state, created_at, updated_at, agent, role, ensemble, model, reasoning_effort, mode, goal_contract, ambiguity) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(repo)
    .bind(branch)
    .bind(base)
    .bind(worktree_path)
    .bind(instruction)
    .bind(state::CREATED)
    .bind(now)
    .bind(now)
    .bind(agent)
    .bind(role)
    .bind(ensemble)
    .bind(model)
    .bind(reasoning_effort)
    .bind(mode)
    .bind(goal_contract.cloned().map(sqlx::types::Json))
    .bind(ambiguity.cloned().map(sqlx::types::Json))
    .execute(pool)
    .await?
    .last_insert_rowid();
    Ok(id)
}

/// 기준점이 없는 진행 중 작업 — backfill 대상.
///
/// 종료 상태는 뺀다. 승인·폐기가 워크트리를 이미 정리했으므로 물어볼 git이 없다.
pub async fn tasks_missing_baseline(pool: &SqlitePool) -> anyhow::Result<Vec<Task>> {
    let rows = sqlx::query_as::<_, Task>(
        "SELECT * FROM tasks WHERE base_revision IS NULL \
           AND state NOT IN (?, ?, ?) ORDER BY id ASC",
    )
    .bind(state::DONE)
    .bind(state::FAILED)
    .bind(state::DISCARDED)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// diff 기준점 영속화 — 작업 생성 시 1회 기록하고 이후 바꾸지 않는다.
///
/// 불변인 것이 이 값의 존재 이유다. 갱신 경로를 열어 두면 "지금 기준으로 다시 잡기" 같은
/// 편의 기능이 붙고, 그 순간 이미 검토한 변경과 주석이 사라지는 원래 문제로 되돌아간다.
pub async fn set_base_revision(pool: &SqlitePool, id: i64, revision: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE tasks SET base_revision = ? WHERE id = ?")
        .bind(revision)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 세션 단위 모델 오버라이드 영속화 — 작업 생성 시 1회 기록(빈 값이면 호출 생략, NULL 유지).
pub async fn set_task_model(pool: &SqlitePool, id: i64, model: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE tasks SET model = ? WHERE id = ?")
        .bind(model)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 에이전트가 아직 `expected_agent`일 때만 모델을 바꾸고, 실제 변경이면 컨텍스트 관측을
/// 무효화한다. 에이전트 전환과 동시에 실행된 옛 모델 저장이 새 벤더의 모델을 덮지 않는다.
pub async fn set_task_model_for_agent(
    pool: &SqlitePool,
    id: i64,
    expected_agent: &str,
    model: &str,
    clear_effort: bool,
    ts: i64,
) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await?;
    let previous_model = sqlx::query_scalar::<_, Option<String>>(
        "SELECT model FROM tasks WHERE id = ? AND agent = ?",
    )
    .bind(id)
    .bind(expected_agent)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(previous_model) = previous_model else {
        return Ok(false);
    };
    let changed = previous_model.as_deref().unwrap_or_default() != model;
    let result = sqlx::query(
        "UPDATE tasks SET model = ?, \
         reasoning_effort = CASE WHEN ? THEN NULL ELSE reasoning_effort END, \
         service_tier = CASE WHEN ? AND service_tier IS NOT NULL THEN 'default' ELSE service_tier END \
         WHERE id = ? AND agent = ?",
    )
    .bind(model)
    .bind(clear_effort)
    .bind(changed)
    .bind(id)
    .bind(expected_agent)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() != 1 {
        return Ok(false);
    }
    if changed {
        sqlx::query("INSERT INTO convo_events (task_id, ts, event) VALUES (?, ?, ?)")
            .bind(id)
            .bind(ts)
            .bind(r#"{"kind":"context_usage","context_tokens":0,"source":"model_change","valid":false}"#)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(true)
}


/// Update only the same model/provider that was validated, including after a concurrent switch.
pub async fn set_task_service_tier(pool: &SqlitePool, task: &Task, tier: &str) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "UPDATE tasks SET service_tier = ? \
         WHERE id = ? AND agent IS ? AND model IS ? AND mode = 'conversation' \
         AND (ensemble IS NULL OR ensemble = '') \
         AND NOT EXISTS (SELECT 1 FROM convo_debate_sides WHERE task_id = tasks.id)",
    )
    .bind(tier)
    .bind(task.id)
    .bind(&task.agent)
    .bind(&task.model)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Codex 세션 단위 reasoning override — 작업 생성 시 검증된 명시값만 기록한다.
pub async fn set_task_reasoning_effort(
    pool: &SqlitePool,
    id: i64,
    effort: &str,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE tasks SET reasoning_effort = ? WHERE id = ?")
        .bind(effort)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 대화 작업의 벤더를 바꾸며 새 세션용 핸드오프를 남긴다.
///
/// 에이전트·모델·세션·핸드오프·대화 경계는 한 계약이다. 별도 UPDATE로 나누면 새 벤더가
/// 옛 세션을 resume하거나, 화면에는 절단선이 보이는데 핸드오프가 없는 반쪽 상태가 남는다.
pub async fn switch_convo_agent(
    pool: &SqlitePool,
    id: i64,
    agent: &str,
    model: &str,
    pending_capsule: &str,
    event: &str,
    ts: i64,
) -> anyhow::Result<Task> {
    let mut tx = pool.begin().await?;
    let mut task = sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .map(validate_loaded_task)
        .transpose()?
        .ok_or_else(|| anyhow::anyhow!("작업을 찾을 수 없습니다"))?;
    if task.state != state::AWAITING_REVIEW || task.mode != "conversation" {
        return Err(anyhow::anyhow!("검토 대기 중인 대화 작업만 에이전트를 바꿀 수 있습니다"));
    }
    crate::convo::interaction::rebind_for_agent_tx(&mut tx, id, agent)
        .await.map_err(anyhow::Error::msg)?;
    // 모델 관측은 작업 행의 현재 agent가 아니라 관측 당시의 agent에 귀속돼야 한다. 이미
    // 찍힌 agent는 보존해 여러 번 전환해도 과거 카탈로그가 다시 덮이지 않게 한다.
    sqlx::query(
        "UPDATE convo_events SET event = json_set(event, '$.agent', ?) \
         WHERE task_id = ? AND json_extract(event, '$.kind') = 'model_snapshot' \
         AND json_extract(event, '$.agent') IS NULL",
    )
    .bind(task.agent.as_deref().unwrap_or_default())
    .bind(id)
    .execute(&mut *tx)
    .await?;
    let updated = sqlx::query(
        "UPDATE tasks SET agent = ?, model = ?, reasoning_effort = NULL, service_tier = NULL, \
         convo_session_id = NULL, pending_capsule = ?, updated_at = ? \
         WHERE id = ? AND state = ? AND mode = ?",
    )
    .bind(agent)
    .bind(model)
    .bind(pending_capsule)
    .bind(ts)
    .bind(id)
    .bind(state::AWAITING_REVIEW)
    .bind("conversation")
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(anyhow::anyhow!("작업 상태가 바뀌어 에이전트를 전환하지 못했습니다"));
    }
    sqlx::query("INSERT INTO convo_events (task_id, ts, event) VALUES (?, ?, ?)")
        .bind(id)
        .bind(ts)
        .bind(event)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    task.agent = Some(agent.to_string());
    task.model = Some(model.to_string());
    task.reasoning_effort = None;
    task.service_tier = None;
    task.convo_session_id = None;
    task.pending_capsule = Some(pending_capsule.to_string());
    task.updated_at = ts;
    Ok(task)
}

/// 대화 세션 id 영속화 — 앱 재시작 후에도 `--resume` 근거가 남도록.
///
/// 같은 문장으로 `pending_capsule`을 지운다. **세션이 생겼다는 것이 캡슐이 도착했다는 유일한
/// 증거이기 때문이다**(ADR 0170 §결정3). 두 쓰기로 나누면 그 사이에 죽었을 때 세션은 생겼는데
/// 캡슐이 남아 다음 턴에 또 붙는다. 호출처 넷(데스크톱 턴 에필로그 2 + 러너 2)이 모두 여기를
/// 지나므로 이 한 문장이 넷을 함께 고친다.
pub async fn set_convo_session(pool: &SqlitePool, id: i64, session_id: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE tasks SET convo_session_id = ?, pending_capsule = NULL WHERE id = ?")
        .bind(session_id)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 이어받기 — 새 작업이 원본의 벤더 세션을 물려받는다. 원본 행은 읽기만 한다.
///
/// 두 칸(`resumed_from`·`convo_session_id`)을 **한 문장으로** 쓴다. 나누면 그 사이에 죽었을 때
/// 세션만 물려받고 출처가 없는 작업이 남는다 — 대화창은 첫 턴처럼 보이는데 벤더는 옛 문맥을
/// 이어가므로, 사용자에게는 에이전트가 하지도 않은 말을 기억하는 것으로 보인다.
///
/// 원본에 세션이 없어도(`NULL`) 실패로 보지 않는다. 그 경우 벤더 문맥은 새로 시작하지만
/// 화면상의 대화 이력은 `resumed_from` 체인으로 이어지므로, 이어받기 자체는 성립한다.
/// 승계 여부는 반환값이 알려 준다 — 호출부가 사용자에게 그 차이를 말할 수 있어야 한다.
pub async fn adopt_conversation(
    pool: &SqlitePool,
    id: i64,
    source_id: i64,
    now: i64,
) -> anyhow::Result<bool> {
    if id == source_id {
        anyhow::bail!("작업이 자기 자신을 이어받을 수 없습니다");
    }
    let updated = sqlx::query(
        "UPDATE tasks SET resumed_from = ?1, \
           convo_session_id = (SELECT convo_session_id FROM tasks WHERE id = ?1), \
           service_tier = (SELECT source.service_tier FROM tasks source WHERE source.id = ?1 AND source.agent IS tasks.agent AND COALESCE(source.model, '') = COALESCE(tasks.model, '')), \
           updated_at = ?2 WHERE id = ?3 \
           AND EXISTS (SELECT 1 FROM tasks source WHERE source.id = ?1 AND COALESCE(source.agent, 'claude') = COALESCE(tasks.agent, 'claude')) \
           AND NOT EXISTS (SELECT 1 FROM tasks live JOIN tasks source ON source.id = ?1 \
             WHERE live.id != ?3 AND live.id != ?1 AND live.state NOT IN (?4, ?5, ?6) \
             AND source.convo_session_id IS NOT NULL AND source.convo_session_id != '' \
             AND ((live.convo_session_id = source.convo_session_id AND COALESCE(live.agent, 'claude') = COALESCE(source.agent, 'claude')) \
               OR (live.resumed_session = source.convo_session_id AND live.resumed_vendor = COALESCE(source.agent, 'claude'))))",
    ).bind(source_id).bind(now).bind(id)
    .bind(state::DONE).bind(state::FAILED).bind(state::DISCARDED)
    .execute(pool).await?;
    if updated.rows_affected() == 0 {
        anyhow::bail!("세션 승계 충돌 또는 공급자 불일치");
    }
    let (inherited,): (Option<String>,) =
        sqlx::query_as("SELECT convo_session_id FROM tasks WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await?;
    Ok(inherited.is_some_and(|value| !value.trim().is_empty()))
}

/// 이어받기 체인을 거슬러 올라간 원본 id들 — **오래된 것부터**, 자기 자신은 빼고.
///
/// 대화 이력을 시간순으로 이어 붙이려면 이 순서여야 한다. 깊이를 32로 끊고 방문한 id를
/// 기억하는 이유는 하나다: `resumed_from`은 외래키 제약이 없는 정수 칸이라, 손으로 고친 DB나
/// 앞으로 생길 다른 쓰기 경로가 고리를 만들면 이 함수가 영원히 돌아 앱이 멈춘다. 상한은
/// 사용자가 실제로 만들 체인(한두 번)보다 훨씬 크므로 정상 경로를 자르지 않는다.
pub async fn resume_chain(pool: &SqlitePool, id: i64) -> anyhow::Result<Vec<i64>> {
    const MAX_DEPTH: usize = 32;
    let mut seen = std::collections::HashSet::from([id]);
    let mut chain = Vec::new();
    let mut cursor = id;
    while chain.len() < MAX_DEPTH {
        let row: Option<(Option<i64>,)> = sqlx::query_as("SELECT resumed_from FROM tasks WHERE id = ?")
            .bind(cursor)
            .fetch_optional(pool)
            .await?;
        let Some((Some(parent),)) = row else {
            break;
        };
        if !seen.insert(parent) {
            break;
        }
        chain.push(parent);
        cursor = parent;
    }
    chain.reverse();
    Ok(chain)
}

/// 토론 사이드 한 행 — 좌측이 아닌 자리 하나. 좌측의 원천은 `tasks`다.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct DebateSideRow {
    pub task_id: i64,
    pub side: String,
    pub agent: String,
    pub model: Option<String>,
    pub vendor_session_id: Option<String>,
}

/// 이 작업의 토론 사이드 행 전부, 자리 순서(빈 배열 = 토론 중이 아님).
///
/// 정렬은 SQL이 아니라 `Side::index()`로 한다 — `ORDER BY side`는 알파벳 순이 자리 순과
/// 우연히 맞을 뿐이다. 알 수 없는 자리 값이나 빈틈 있는 조합(`third`만 등)은 **오류다**:
/// 받아들이면 "마지막 자리"가 틀어져 라운드가 오르지 않는다(설계 2026-09-23 D1).
pub async fn debate_sides(pool: &SqlitePool, task_id: i64) -> anyhow::Result<Vec<DebateSideRow>> {
    let rows: Vec<DebateSideRow> =
        sqlx::query_as("SELECT * FROM convo_debate_sides WHERE task_id = ?")
            .bind(task_id)
            .fetch_all(pool)
            .await?;
    let mut seated = rows
        .into_iter()
        .map(|row| {
            crate::convo::Side::parse(&row.side)
                .map(|side| (side, row.clone()))
                .ok_or_else(|| anyhow::anyhow!("알 수 없는 토론 자리: {}", row.side))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    if seated.is_empty() {
        return Ok(Vec::new());
    }
    seated.sort_by_key(|(side, _)| side.index());
    let seats = std::iter::once(crate::convo::Side::Left)
        .chain(seated.iter().map(|(side, _)| *side))
        .collect::<Vec<_>>();
    if !crate::convo::debate::valid_seats(&seats) {
        anyhow::bail!("토론 자리 조합이 어긋났습니다: {seats:?}");
    }
    Ok(seated.into_iter().map(|(_, row)| row).collect())
}

/// 토론 사이드 행들을 한 트랜잭션으로 생성. 벤더 세션은 아직 없다 — 그 자리의 첫 턴이 판다.
/// 같은 `(task_id, side)` 재삽입은 PK 충돌로 **실패하고** 아무 행도 남지 않는다: 이미 도는
/// 토론을 조용히 덮으면 살아 있는 사이드 세션 id가 사라진다.
pub async fn insert_debate_sides(
    pool: &SqlitePool,
    task_id: i64,
    sides: &[(crate::convo::Side, &str, Option<&str>)],
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    for (side, agent, model) in sides {
        sqlx::query(
            "INSERT INTO convo_debate_sides (task_id, side, agent, model, vendor_session_id) \
             VALUES (?, ?, ?, ?, NULL)",
        )
        .bind(task_id)
        .bind(side.as_str())
        .bind(*agent)
        .bind(*model)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// 사이드 벤더 세션 id 영속. **`set_convo_session`을 지나지 않는다** — 그 함수는 세션 확립과
/// `pending_capsule` 삭제를 한 문장으로 묶으므로(ADR 0170), 우측이 지나면 좌측이 받아야 할
/// 캡슐을 우측이 지운다.
pub async fn set_debate_side_session(
    pool: &SqlitePool,
    task_id: i64,
    side: crate::convo::Side,
    session_id: &str,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE convo_debate_sides SET vendor_session_id = ? WHERE task_id = ? AND side = ?")
        .bind(session_id)
        .bind(task_id)
        .bind(side.as_str())
        .execute(pool)
        .await?;
    Ok(())
}

/// 사이드 행 전부 삭제 = 토론 종료. 없어도 성공한다(멱등).
pub async fn delete_debate_sides(pool: &SqlitePool, task_id: i64) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM convo_debate_sides WHERE task_id = ?")
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 진행 중 turn의 pgid 영속(스폰 시 Some, 완료/중단 시 None). 앱 재시작 조정(reconcile)의 근거.
pub async fn set_convo_pgid(pool: &SqlitePool, id: i64, pgid: Option<i64>) -> anyhow::Result<()> {
    sqlx::query("UPDATE tasks SET convo_pgid = ? WHERE id = ?")
        .bind(pgid)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn record_task_process_start(
    pool: &SqlitePool,
    task_id: i64,
    pgid: i64,
    identity_hash: &str,
    process_kind: &str,
    now: i64,
) -> anyhow::Result<()> {
    if pgid <= 0 || !matches!(process_kind, "terminal" | "conversation") {
        anyhow::bail!("invalid task process identity");
    }
    if identity_hash.len() != 64 || !identity_hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("invalid task process identity hash");
    }
    let mut tx = pool.begin().await?;
    let changed = sqlx::query(
        "UPDATE tasks SET convo_pgid = ? \
         WHERE id = ? AND state = ? AND convo_pgid IS NULL",
    )
    .bind(pgid)
    .bind(task_id)
    .bind(state::RUNNING)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed != 1 {
        anyhow::bail!("task left Running before process identity was recorded");
    }
    sqlx::query(
        "INSERT INTO task_process_receipts \
         (task_id, pgid, identity_hash, process_kind, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(task_id)
    .bind(pgid)
    .bind(identity_hash)
    .bind(process_kind)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO runner_events (task_id, ts, kind, detail) \
         VALUES (?, ?, 'process_started', NULL)",
    )
    .bind(task_id)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn task_process_receipt(
    pool: &SqlitePool,
    task_id: i64,
) -> anyhow::Result<Option<TaskProcessReceipt>> {
    sqlx::query_as("SELECT * FROM task_process_receipts WHERE task_id = ? ORDER BY id DESC LIMIT 1")
        .bind(task_id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

/// 대화 이벤트 1건 적재 (JSON 직렬화된 ConvoEvent 또는 `{"kind":"user",...}`).
pub async fn append_convo_event(
    pool: &SqlitePool,
    task_id: i64,
    event_json: &str,
    ts: i64,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO convo_events (task_id, ts, event) VALUES (?, ?, ?)")
        .bind(task_id)
        .bind(ts)
        .bind(event_json)
        .execute(pool)
        .await?;
    Ok(())
}

/// 작업의 **최근** 대화 이벤트 `limit`건을 오래된 순으로 — 텔레그램 `/log` 백엔드.
/// `list_convo_events`(전체 로드)를 쓰지 않는 이유: 긴 작업은 수천 건이라 요약 한 줄을 만들려고
/// 트랜스크립트 전체를 메모리에 올리게 된다. 뒤에서 자르고 표시 직전에 순서를 되돌린다.
pub async fn recent_convo_events(
    pool: &SqlitePool,
    task_id: i64,
    limit: i64,
) -> anyhow::Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT event FROM convo_events WHERE task_id = ? AND rewound_at IS NULL \
         ORDER BY id DESC LIMIT ?",
    )
    .bind(task_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().rev().map(|(e,)| e).collect())
}

/// 에이전트 전환용 최근 사용자·메인 에이전트 대화. 도구·확장 프롬프트·측정 이벤트를
/// LIMIT 전에 제외해야 도구가 많은 턴의 최신 수정 지시가 밀려나지 않는다.
pub async fn recent_handoff_dialogue_events(
    pool: &SqlitePool,
    task_id: i64,
    limit: i64,
) -> anyhow::Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT event FROM convo_events WHERE task_id = ? AND rewound_at IS NULL AND (\
           json_extract(event, '$.kind') = 'user' OR \
           (json_extract(event, '$.kind') = 'text' AND \
            COALESCE(json_extract(event, '$.parent_id'), '') = '') OR \
           (json_extract(event, '$.kind') = 'result' AND \
            COALESCE(json_extract(event, '$.is_error'), 0) = 0)) \
         ORDER BY id DESC LIMIT ?",
    )
    .bind(task_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().rev().map(|(event,)| event).collect())
}

/// 작업의 대화 이벤트 전체 (오래된 순) — 재진입 시 트랜스크립트 복원.
/// 되감긴 이벤트는 빠진다 — 남겨 두면 되감기가 무의미해진다.
pub async fn list_convo_events(pool: &SqlitePool, task_id: i64) -> anyhow::Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT event FROM convo_events WHERE task_id = ? AND rewound_at IS NULL ORDER BY id ASC",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(e,)| e).collect())
}

/// 대화 이벤트의 **마지막 `limit`건**을 시간순으로. 이어받은 이력이 이 조회를 쓴다.
///
/// `list_convo_events`와 갈라 두는 이유는 작업량이다. 이어받기 체인은 원본 대화 전부를 다시
/// 읽는데, 상한을 만든 뒤에 자르면 자를 것까지 전부 읽고 파싱한 뒤에 버리게 된다 — 상한이
/// 화면 크기만 줄이고 세션을 여는 비용은 그대로 둔다. 꼬리를 고르는 이유는 잘라야 할 때
/// 버릴 쪽이 먼 과거이기 때문이다.
pub async fn recent_convo_events_tail(
    pool: &SqlitePool,
    task_id: i64,
    limit: usize,
) -> anyhow::Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT event FROM (\
           SELECT id, event FROM convo_events WHERE task_id = ? AND rewound_at IS NULL \
           ORDER BY id DESC LIMIT ?\
         ) ORDER BY id ASC",
    )
    .bind(task_id)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(e,)| e).collect())
}

/// 되감기지 않은 대화 이벤트 수. 이어받은 이력이 무엇을 얼마나 잘랐는지 세는 데 쓴다.
pub async fn convo_event_count(pool: &SqlitePool, task_id: i64) -> anyhow::Result<usize> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM convo_events WHERE task_id = ? AND rewound_at IS NULL",
    )
    .bind(task_id)
    .fetch_one(pool)
    .await?;
    Ok(count.max(0) as usize)
}

/// 이 벤더 세션을 이미 물고 있는 **살아 있는** 작업. 없으면 `None`.
///
/// 이어받기가 이것을 봐야 하는 이유는 세션이 공유 자원이기 때문이다. 같은 원본을 두 번
/// 이어받으면 서로 다른 워크트리의 두 작업이 같은 세션을 동시에 `--resume`하고, 두 대화의
/// 문맥이 한 세션 파일에서 섞인다. 재시도로도 쉽게 도달한다 — 이어받기는 워크트리 생성까지
/// 포함해 수 초가 걸려서, 느리다고 한 번 더 누르는 것이 곧 두 번째 이어받기가 된다.
///
/// 술어는 `convo_session_id`·`resumed_session` 양쪽을 본다(설계 2026-09-17 결정 5) — 턴
/// 에필로그가 `convo_session_id`를 매 턴 덮어쓰므로(`set_convo_session`), 그것만 보면 외부
/// 승계 작업이 몇 턴 지난 뒤에는 이 가드에서 조용히 빠진다.
pub async fn live_task_with_session(pool: &SqlitePool, session_id: &str) -> anyhow::Result<Option<i64>> {
    live_task_with_session_vendor(pool, "claude", session_id).await
}

pub async fn live_task_with_session_vendor(
    pool: &SqlitePool, vendor: &str, session_id: &str,
) -> anyhow::Result<Option<i64>> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM tasks WHERE ((convo_session_id = ?1 AND COALESCE(agent, 'claude') = ?2) \
         OR (resumed_session = ?1 AND resumed_vendor = ?2)) \
         AND state NOT IN (?3, ?4, ?5) ORDER BY id ASC LIMIT 1",
    ).bind(session_id).bind(vendor)
    .bind(state::DONE).bind(state::FAILED).bind(state::DISCARDED)
    .fetch_optional(pool).await?;
    Ok(row.map(|(id,)| id))
}

/// [`adopt_external_session`] 실패 사유.
#[derive(Debug)]
pub enum AdoptError {
    /// 같은 세션을 이미 물고 있는 살아 있는 작업이 있다 — 그 작업 id(HTTP 409에 싣는다, 설계
    /// 2026-09-17 결정 9).
    Conflict(i64),
    /// 그 밖의 DB 오류.
    Db(anyhow::Error),
}

impl std::fmt::Display for AdoptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdoptError::Conflict(task_id) => {
                write!(f, "이 세션은 이미 #{task_id} 작업이 이어가고 있습니다")
            }
            AdoptError::Db(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for AdoptError {}

impl From<anyhow::Error> for AdoptError {
    fn from(error: anyhow::Error) -> Self {
        AdoptError::Db(error)
    }
}

/// 외부 세션 승계 — 세션홈에서 직접 고른 벤더 세션을 새 작업(`task_id`)이 물려받는다.
///
/// `adopt_conversation`(작업 id 기반 이어받기)과 갈라지는 지점: 원본 작업 행이 없을 수 있다
/// (터미널에서 만든 세션 등 `tasks`에 행이 없는 경우가 정상이다). 그래서 이 함수는 원본을
/// 읽지 않고 대상 행 하나만 **조건부 UPDATE 한 문장**으로 채운다 — "같은 세션을 물고 있는
/// live 작업이 없을 때만" 이라는 조건 자체를 WHERE 절에 넣는다. 사전 조회 → 워크트리 생성
/// (수 초) → UPDATE로 나누면 그 사이 들어온 두 번째 요청이 함께 통과한다(설계 2026-09-17
/// 결정 4 — #476이 기록한 "느려서 한 번 더 누르기"의 재현).
///
/// `convo_session_id`·`resumed_session`을 같은 값으로 함께 쓴다 — 승계 시점에는 둘이 같지만
/// (실측: 벤더 resume이 같은 세션 id를 돌려준다), `convo_session_id`는 턴 에필로그가 매번
/// 덮어쓰므로 `resumed_session`만 불변 출처로 남는다.
///
/// 갱신 행이 0이면(충돌) 그 세션을 물고 있는 작업 id를 다시 조회해 [`AdoptError::Conflict`]로
/// 돌려준다 — 실패가 이미 확정된 뒤의 조회라 TOCTOU가 없다. 그 사이 대상 작업이 끝나는 등
/// 드문 경우로 충돌 행을 못 찾으면 일반 DB 에러로 돌려준다.
pub async fn adopt_external_session(
    pool: &SqlitePool, task_id: i64, session_id: &str, now: i64,
) -> Result<(), AdoptError> {
    adopt_external_session_vendor(pool, task_id, "claude", session_id, now).await
}

pub async fn adopt_external_session_vendor(
    pool: &SqlitePool, task_id: i64, vendor: &str, session_id: &str, now: i64,
) -> Result<(), AdoptError> {
    if !matches!(vendor, "claude" | "codex") {
        return Err(AdoptError::Db(anyhow::anyhow!("지원하지 않는 세션 공급자")));
    }
    let result = sqlx::query(
        "UPDATE tasks SET convo_session_id = ?1, resumed_session = ?1, resumed_vendor = ?7, updated_at = ?2 \
         WHERE id = ?3 AND COALESCE(agent, 'claude') = ?7 AND NOT EXISTS ( \
           SELECT 1 FROM tasks live WHERE live.id != ?3 \
             AND ((live.convo_session_id = ?1 AND COALESCE(live.agent, 'claude') = ?7) \
                  OR (live.resumed_session = ?1 AND live.resumed_vendor = ?7)) \
             AND live.state NOT IN (?4, ?5, ?6))",
    ).bind(session_id).bind(now).bind(task_id)
    .bind(state::DONE).bind(state::FAILED).bind(state::DISCARDED).bind(vendor)
    .execute(pool).await.map_err(|e| AdoptError::Db(e.into()))?;
    if result.rows_affected() > 0 { return Ok(()); }
    match live_task_with_session_vendor(pool, vendor, session_id).await.map_err(AdoptError::Db)? {
        Some(id) => Err(AdoptError::Conflict(id)),
        None => Err(AdoptError::Db(anyhow::anyhow!("세션 승계 대상 또는 공급자가 일치하지 않습니다"))),
    }
}

/// 대화 체크포인트 — 파일(worktree 커밋)과 대화(이벤트 경계)를 함께 잡은 한 점.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ConvoCheckpoint {
    pub id: i64,
    pub task_id: i64,
    pub label: String,
    pub worktree_commit: String,
    pub convo_event_max_id: i64,
    pub ts: i64,
}

/// 이 작업의 현재 대화 이벤트 최대 id. 이벤트가 없으면 0 — 체크포인트가 대화 시작 이전을 가리킨다.
pub async fn max_convo_event_id(pool: &SqlitePool, task_id: i64) -> anyhow::Result<i64> {
    let row: (Option<i64>,) =
        sqlx::query_as("SELECT MAX(id) FROM convo_events WHERE task_id = ?")
            .bind(task_id)
            .fetch_one(pool)
            .await?;
    Ok(row.0.unwrap_or(0))
}

pub async fn insert_convo_checkpoint(
    pool: &SqlitePool,
    task_id: i64,
    label: &str,
    worktree_commit: &str,
    convo_event_max_id: i64,
    ts: i64,
) -> anyhow::Result<ConvoCheckpoint> {
    let id = sqlx::query(
        "INSERT INTO convo_checkpoints (task_id, label, worktree_commit, convo_event_max_id, ts) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(task_id)
    .bind(label)
    .bind(worktree_commit)
    .bind(convo_event_max_id)
    .bind(ts)
    .execute(pool)
    .await?
    .last_insert_rowid();
    Ok(ConvoCheckpoint {
        id,
        task_id,
        label: label.to_string(),
        worktree_commit: worktree_commit.to_string(),
        convo_event_max_id,
        ts,
    })
}

/// 작업의 체크포인트 (최신 순).
pub async fn list_convo_checkpoints(
    pool: &SqlitePool,
    task_id: i64,
) -> anyhow::Result<Vec<ConvoCheckpoint>> {
    Ok(sqlx::query_as(
        "SELECT id, task_id, label, worktree_commit, convo_event_max_id, ts \
         FROM convo_checkpoints WHERE task_id = ? ORDER BY id DESC",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await?)
}

pub async fn get_convo_checkpoint(
    pool: &SqlitePool,
    id: i64,
) -> anyhow::Result<Option<ConvoCheckpoint>> {
    Ok(sqlx::query_as(
        "SELECT id, task_id, label, worktree_commit, convo_event_max_id, ts \
         FROM convo_checkpoints WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

/// 체크포인트 이후의 **살아 있는** 이벤트 (오래된 순) — 요약 생성의 재료.
pub async fn convo_events_after(
    pool: &SqlitePool,
    task_id: i64,
    after_id: i64,
) -> anyhow::Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT event FROM convo_events \
         WHERE task_id = ? AND id > ? AND rewound_at IS NULL ORDER BY id ASC",
    )
    .bind(task_id)
    .bind(after_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(e,)| e).collect())
}

/// 체크포인트 이후를 되감김으로 표시한다(논리 삭제). 표시된 건수를 돌려준다.
pub async fn mark_convo_events_rewound(
    pool: &SqlitePool,
    task_id: i64,
    after_id: i64,
    ts: i64,
) -> anyhow::Result<u64> {
    let result = sqlx::query(
        "UPDATE convo_events SET rewound_at = ? \
         WHERE task_id = ? AND id > ? AND rewound_at IS NULL",
    )
    .bind(ts)
    .bind(task_id)
    .bind(after_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// 벤더 세션을 놓는다 — 다음 턴이 새 세션으로 시작하게 만든다.
/// 벤더 CLI는 컨텍스트 절단을 제공하지 않으므로, 오염된 컨텍스트를 버리는 유일한 방법이다.
pub async fn clear_convo_session(pool: &SqlitePool, task_id: i64) -> anyhow::Result<()> {
    sqlx::query("UPDATE tasks SET convo_session_id = NULL WHERE id = ?")
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 절단이 남긴 캡슐을 적는다 — 다음 턴의 프롬프트가 읽어갈 자리다(ADR 0170).
///
/// 기존 값이 있으면 덮는다. 미소비 캡슐이 있는 상태에서 다시 절단하면 두 번째 캡슐은 진척
/// 없는 상태로 조립되지만, 그것이 그 순간의 사실이므로 옳다 — 마지막 절단만 유효하다.
pub async fn set_pending_capsule(
    pool: &SqlitePool,
    task_id: i64,
    capsule: &str,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE tasks SET pending_capsule = ? WHERE id = ?")
        .bind(capsule)
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 미소비 캡슐을 **읽기만** 한다. 지우지 않는 것이 계약이다.
///
/// 지우기는 `set_convo_session`이 맡는다. 여기서 지우면 벤더 스폰 실패·인터럽트·유휴
/// 타임아웃에서 캡슐이 사라지고, 그때 세션은 이미 끊긴 뒤라 되돌릴 방법이 없다.
pub async fn peek_pending_capsule(
    pool: &SqlitePool,
    task_id: i64,
) -> anyhow::Result<Option<String>> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT pending_capsule FROM tasks WHERE id = ?")
            .bind(task_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|(capsule,)| capsule))
}

/// 한 앙상블 그룹의 후보 작업들 (생성 순). 비교/심판용.
pub async fn tasks_by_ensemble(pool: &SqlitePool, ensemble: &str) -> anyhow::Result<Vec<Task>> {
    let rows = sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE ensemble = ? ORDER BY id ASC")
        .bind(ensemble)
        .fetch_all(pool)
        .await?;
    validate_loaded_tasks(rows)
}

/// 에이전트 종료 시 AwaitingReview로 전이 — 단, 이미 종료 상태(Done/Discarded/Failed)면
/// 덮어쓰지 않는다(승인/폐기와 종료 이벤트의 경쟁 방지).
///
/// `kind`는 대기 성격(`awaiting_kind::*`, None이면 통상 검토)을 **매번 함께 쓴다** —
/// 별도 setter로 두면 직전 턴의 "질문 대기"가 다음 턴 종료 후에도 남는다.
pub async fn mark_awaiting_review(
    pool: &SqlitePool,
    id: i64,
    now: i64,
    kind: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE tasks SET state = ?, updated_at = ?, awaiting_kind = ? WHERE id = ? AND state IN (?, ?)",
    )
    .bind(state::AWAITING_REVIEW)
    .bind(now)
    .bind(kind)
    .bind(id)
    .bind(state::RUNNING)
    .bind(state::CREATED)
    .execute(pool)
    .await?;
    Ok(())
}

/// 검토 대기 → 실행 중 (가드된 전이). 후속 대화 턴 시작 시에만 사용 —
/// blind update_state로 하면 동시 approve/discard가 쓴 Done/Discarded를 되살릴 수 있어(레이스),
/// 현재 상태가 AwaitingReview일 때만 원자적으로 뒤집는다. 반환: 실제 전이 여부.
pub async fn mark_running_from_review(
    pool: &SqlitePool,
    id: i64,
    now: i64,
) -> anyhow::Result<bool> {
    // awaiting_kind는 여기서 지운다 — 대기 주석은 AwaitingReview에서만 의미가 있고,
    // 실행 중 작업에 "질문 대기" 잔상이 남으면 사이드바가 거짓 신호를 낸다.
    let mut transaction = pool.begin().await?;
    let r = sqlx::query(
        "UPDATE tasks SET state = ?, updated_at = ?, awaiting_kind = NULL WHERE id = ? AND state = ? \
         AND NOT EXISTS (SELECT 1 FROM review_process_leases lease WHERE lease.task_id = tasks.id)",
    )
    .bind(state::RUNNING)
    .bind(now)
    .bind(id)
    .bind(state::AWAITING_REVIEW)
    .execute(&mut *transaction)
    .await?;
    if r.rows_affected() > 0 {
        crate::notifications::clear_cancel_tx(&mut transaction, id).await?;
    }
    transaction.commit().await?;
    Ok(r.rows_affected() > 0)
}

/// 이미 검토 대기로 전이된 작업에 대기 성격을 덧쓴다 — 상태 전이가 `finish_running_task`로
/// 일어나는 Runner 경로용. AwaitingReview가 아니면 아무것도 하지 않는다(승인/폐기와 경쟁해
/// 이미 종료된 작업에 주석이 남는 것을 막는다).
pub async fn set_awaiting_kind(
    pool: &SqlitePool,
    id: i64,
    kind: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE tasks SET awaiting_kind = ? WHERE id = ? AND state = ?")
        .bind(kind)
        .bind(id)
        .bind(state::AWAITING_REVIEW)
        .execute(pool)
        .await?;
    Ok(())
}

/// 큐 대기 작업의 차단 사유.
pub mod blocked {
    /// 벤더 CLI 인증이 풀려 시작할 수 없다. 값은 `auth:<vendor>` 형태로 쓴다.
    pub const AUTH_PREFIX: &str = "auth:";

    pub fn auth(vendor: &str) -> String {
        format!("{AUTH_PREFIX}{vendor}")
    }
}

/// 큐 대기 작업을 차단한다. `Queued`가 아니면 아무것도 하지 않는다 — 이미 시작했거나
/// 끝난 작업에 차단 표시가 남으면, 다음에 큐로 돌아왔을 때 이유 없이 멈춘다.
pub async fn set_blocked_reason(
    pool: &SqlitePool,
    id: i64,
    reason: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE tasks SET blocked_reason = ? WHERE id = ? AND state = ?")
        .bind(reason)
        .bind(id)
        .bind(state::QUEUED)
        .execute(pool)
        .await?;
    Ok(())
}

/// lease된(Starting) 작업을 시작하지 않고 큐로 되돌리며 차단 사유를 단다.
///
/// 상태 복귀와 사유 기록을 **한 UPDATE로 묶는다** — 둘로 나누면 그 사이에 다른 워커가
/// 사유 없는 Queued 작업을 다시 집어 같은 실패를 반복한다. 반환: 실제로 되돌렸는지.
pub async fn block_starting_task(
    pool: &SqlitePool,
    id: i64,
    reason: &str,
    now: i64,
) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "UPDATE tasks SET state = ?, blocked_reason = ?, updated_at = ? WHERE id = ? AND state = ?",
    )
    .bind(state::QUEUED)
    .bind(reason)
    .bind(now)
    .bind(id)
    .bind(state::STARTING)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// 한 벤더의 인증 차단을 일괄 해제한다. 반환: 풀려난 작업 수.
///
/// 이것만으로 재개가 끝난다 — 작업은 `Queued`를 떠난 적이 없으므로 다음 lease가 집어간다.
pub async fn clear_auth_block(pool: &SqlitePool, vendor: &str) -> anyhow::Result<u64> {
    let result = sqlx::query("UPDATE tasks SET blocked_reason = NULL WHERE blocked_reason = ?")
        .bind(blocked::auth(vendor))
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// 검토 대기 작업을 완료 처리 전용으로 점유한다. 조건부 UPDATE라 approve/discard 경쟁에서 한 쪽만 성공한다.
pub async fn claim_review_finalization(
    pool: &SqlitePool,
    id: i64,
    now: i64,
) -> anyhow::Result<bool> {
    let result =
        sqlx::query("UPDATE tasks SET state = ?, updated_at = ? WHERE id = ? AND state = ?")
            .bind(state::FINALIZING)
            .bind(now)
            .bind(id)
            .bind(state::AWAITING_REVIEW)
            .execute(pool)
            .await?;
    Ok(result.rows_affected() > 0)
}

/// worktree merge/remove 실패 뒤, 아직 이 finalizer가 점유한 작업만 검토 대기로 되돌린다.
pub async fn restore_awaiting_review(pool: &SqlitePool, id: i64, now: i64) -> anyhow::Result<bool> {
    let result =
        sqlx::query("UPDATE tasks SET state = ?, updated_at = ? WHERE id = ? AND state = ?")
            .bind(state::AWAITING_REVIEW)
            .bind(now)
            .bind(id)
            .bind(state::FINALIZING)
            .execute(pool)
            .await?;
    Ok(result.rows_affected() > 0)
}

/// 가장 오래된 queued 작업 하나를 Runner worker에 lease한다.
///
/// 조건부 UPDATE가 lease 경쟁을 직렬화하고, 성공한 상태 전이만 같은 transaction의
/// `starting_lease` event로 replay에 노출한다. 원본 재검증 뒤에만 Running으로 승격한다.
pub async fn claim_oldest_queued_task(pool: &SqlitePool, now: i64) -> anyhow::Result<Option<Task>> {
    let mut transaction = pool.begin().await?;
    let task = sqlx::query_as::<_, Task>(
        "UPDATE tasks SET state = ?, updated_at = ? \
         WHERE id = (SELECT id FROM tasks WHERE state = ? \
           AND blocked_reason IS NULL \
           AND NOT EXISTS (SELECT 1 FROM review_process_leases lease WHERE lease.task_id = tasks.id) \
           ORDER BY created_at ASC, id ASC LIMIT 1) \
           AND state = ? \
         RETURNING *",
    )
    .bind(state::STARTING)
    .bind(now)
    .bind(state::QUEUED)
    .bind(state::QUEUED)
    .fetch_optional(&mut *transaction)
    .await?;
    if let Some(task) = task {
        let task = validate_loaded_task(task)?;
        sqlx::query(
            "INSERT INTO runner_events (task_id, ts, kind, detail) VALUES (?, ?, 'starting_lease', NULL)",
        )
        .bind(task.id)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        return Ok(Some(task));
    }
    transaction.commit().await?;
    Ok(None)
}

/// Starting lease를 Running으로 승격한다.
///
/// 메모리 정본은 창고 안 파일이다(설계 2026-09-13, P1) — 검증할 투영 원장이 없으므로
/// 시작 영수증 없이 항상 승격한다. 옛 DB 투영(P2) 재검증(`validate_start_receipt`)은
/// `memory_projection_journal`·`memory_evidence`가 사라지며 함께 제거됐다.
pub async fn promote_starting_task(
    pool: &SqlitePool,
    task_id: i64,
    now: i64,
) -> anyhow::Result<bool> {
    let mut transaction = pool.begin().await?;
    let result = sqlx::query(
        "UPDATE tasks SET state = ?, updated_at = ? WHERE id = ? AND state = ? \
         AND NOT EXISTS (SELECT 1 FROM review_process_leases lease WHERE lease.task_id = tasks.id)",
    )
    .bind(state::RUNNING)
    .bind(now)
    .bind(task_id)
    .bind(state::STARTING)
    .execute(&mut *transaction)
    .await?;
    if result.rows_affected() == 0 {
        transaction.commit().await?;
        return Ok(false);
    }
    crate::notifications::clear_cancel_tx(&mut transaction, task_id).await?;
    sqlx::query(
        "INSERT INTO runner_events (task_id, ts, kind, detail) VALUES (?, ?, 'running', NULL)",
    )
    .bind(task_id)
    .bind(now)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(true)
}

pub async fn fail_starting_task(
    pool: &SqlitePool,
    task_id: i64,
    now: i64,
    detail: &str,
) -> anyhow::Result<bool> {
    let mut transaction = pool.begin().await?;
    let result =
        sqlx::query("UPDATE tasks SET state = ?, updated_at = ? WHERE id = ? AND state = ?")
            .bind(state::FAILED)
            .bind(now)
            .bind(task_id)
            .bind(state::STARTING)
            .execute(&mut *transaction)
            .await?;
    if result.rows_affected() == 0 {
        transaction.commit().await?;
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO runner_events (task_id, ts, kind, detail) \
         VALUES (?, ?, 'memory_projection_start_blocked', ?)",
    )
    .bind(task_id)
    .bind(now)
    .bind(detail)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(true)
}

/// 아직 worker가 lease하지 않은 queued 작업만 취소한다. 상태 변경과 replay event는 한 트랜잭션이다.
pub async fn cancel_queued_task(pool: &SqlitePool, task_id: i64, now: i64) -> anyhow::Result<bool> {
    let mut transaction = pool.begin().await?;
    let result =
        sqlx::query("UPDATE tasks SET state = ?, updated_at = ? WHERE id = ? AND state = ?")
            .bind(state::FAILED)
            .bind(now)
            .bind(task_id)
            .bind(state::QUEUED)
            .execute(&mut *transaction)
            .await?;
    if result.rows_affected() == 0 {
        transaction.commit().await?;
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO runner_events (task_id, ts, kind, detail) VALUES (?, ?, 'cancelled', NULL)",
    )
    .bind(task_id)
    .bind(now)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(true)
}

/// 승인 대기 작업을 Runner durable queue에 넣는다. 중복 승인은 상태를 바꾸지 않는다.
pub async fn queue_pending_task(pool: &SqlitePool, task_id: i64, now: i64) -> anyhow::Result<bool> {
    let mut transaction = pool.begin().await?;
    let result =
        sqlx::query("UPDATE tasks SET state = ?, updated_at = ? WHERE id = ? AND state = ?")
            .bind(state::QUEUED)
            .bind(now)
            .bind(task_id)
            .bind(state::PENDING_APPROVAL)
            .execute(&mut *transaction)
            .await?;
    if result.rows_affected() == 0 {
        transaction.commit().await?;
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO runner_events (task_id, ts, kind, detail) VALUES (?, ?, 'queued', NULL)",
    )
    .bind(task_id)
    .bind(now)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(true)
}

/// 승인 대기 작업을 실행 전 폐기 상태로 바꾼다. worktree 정리는 host adapter가 담당한다.
pub async fn reject_pending_task(
    pool: &SqlitePool,
    task_id: i64,
    now: i64,
) -> anyhow::Result<bool> {
    let mut transaction = pool.begin().await?;
    let result =
        sqlx::query("UPDATE tasks SET state = ?, updated_at = ? WHERE id = ? AND state = ?")
            .bind(state::DISCARDED)
            .bind(now)
            .bind(task_id)
            .bind(state::PENDING_APPROVAL)
            .execute(&mut *transaction)
            .await?;
    if result.rows_affected() == 0 {
        transaction.commit().await?;
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO runner_events (task_id, ts, kind, detail) VALUES (?, ?, 'discarded', NULL)",
    )
    .bind(task_id)
    .bind(now)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(true)
}

pub async fn update_state(
    pool: &SqlitePool,
    id: i64,
    new_state: &str,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE tasks SET state = ?, updated_at = ? WHERE id = ?")
        .bind(new_state)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 특정 레포의 Done(승인 완료) Task 수 — L3 자동 트리거 임계 판단용.
pub async fn count_done_tasks(pool: &SqlitePool, repo: &str) -> anyhow::Result<i64> {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tasks WHERE repo = ? AND state = ?")
        .bind(repo)
        .bind(state::DONE)
        .fetch_one(pool)
        .await?;
    Ok(n)
}

pub async fn list_tasks(pool: &SqlitePool) -> anyhow::Result<Vec<Task>> {
    let rows = sqlx::query_as::<_, Task>("SELECT * FROM tasks ORDER BY id DESC")
        .fetch_all(pool)
        .await?;
    validate_loaded_tasks(rows)
}

/// 메인 checkout을 공유하는 비종료 direct 작업만 소형 행으로 조회한다.
pub async fn list_open_direct_tasks(pool: &SqlitePool) -> anyhow::Result<Vec<OpenDirectTask>> {
    let rows = sqlx::query_as::<_, OpenDirectTask>(
        "SELECT id, repo FROM tasks \
         WHERE repo = worktree_path AND state NOT IN (?, ?, ?) ORDER BY id ASC",
    )
    .bind(state::DONE)
    .bind(state::FAILED)
    .bind(state::DISCARDED)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 재시작 복구가 repo 락 아래에서 재확인할 `Created` direct 작업 목록.
pub async fn list_created_direct_tasks(pool: &SqlitePool) -> anyhow::Result<Vec<OpenDirectTask>> {
    let rows = sqlx::query_as::<_, OpenDirectTask>(
        "SELECT id, repo FROM tasks \
         WHERE repo = worktree_path AND state = ? ORDER BY id ASC",
    )
    .bind(state::CREATED)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 현재 Starting/Running 상태 Task 전체 — 앱 재시작 조정 입력.
pub async fn list_running_tasks(pool: &SqlitePool) -> anyhow::Result<Vec<Task>> {
    let rows =
        sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE state IN (?, ?) ORDER BY id ASC")
            .bind(state::STARTING)
            .bind(state::RUNNING)
            .fetch_all(pool)
            .await?;
    validate_loaded_tasks(rows)
}

/// 최근 작업 N개 (최신순) — 텔레그램 봇 `/tasks` 명령용(Phase 2).
pub async fn recent_tasks(pool: &SqlitePool, limit: i64) -> anyhow::Result<Vec<Task>> {
    let rows = sqlx::query_as::<_, Task>("SELECT * FROM tasks ORDER BY id DESC LIMIT ?")
        .bind(limit)
        .fetch_all(pool)
        .await?;
    validate_loaded_tasks(rows)
}

/// 특정 상태의 최근 작업 — 텔레그램 `/tasks <필터>` 백엔드.
pub async fn recent_tasks_by_state(
    pool: &SqlitePool,
    state: &str,
    limit: i64,
) -> anyhow::Result<Vec<Task>> {
    let rows =
        sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE state = ? ORDER BY id DESC LIMIT ?")
            .bind(state)
            .bind(limit)
            .fetch_all(pool)
            .await?;
    validate_loaded_tasks(rows)
}

/// instruction/repo/branch 부분 일치(대소문자 무시, LIKE) 검색 — 최근 수정순. Quick Open(⌘K) 백엔드 소스.
/// query가 빈 문자열이면 전체를 최근순으로 반환(빈 상태에서도 최근 작업 노출).
pub async fn search_tasks(pool: &SqlitePool, query: &str, limit: i64) -> anyhow::Result<Vec<Task>> {
    let pattern = format!("%{}%", escape_like_pattern(query));
    let rows = sqlx::query_as::<_, Task>(
        "SELECT * FROM tasks WHERE instruction LIKE ? ESCAPE '\\' OR repo LIKE ? ESCAPE '\\' \
         OR branch LIKE ? ESCAPE '\\' ORDER BY updated_at DESC LIMIT ?",
    )
    .bind(&pattern)
    .bind(&pattern)
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    validate_loaded_tasks(rows)
}

/// LIKE 와일드카드(`%`,`_`)를 사용자 질의에서 리터럴로 취급하기 위한 이스케이프.
fn escape_like_pattern(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Quick Open(⌘K) 프론트 랭킹 병합(quickopen.ts)의 입력 후보 한 건.
#[derive(Debug, Clone, Serialize)]
pub struct QuickOpenCandidate {
    /// "task"(터미널 모드) | "session"(대화 모드) — mode 컬럼에서 파생.
    pub scope: &'static str,
    pub id: i64,
    pub title: String,
    pub subtitle: String,
    /// 최근성 가중 기준 — epoch초(Task.updated_at과 동일 단위).
    pub updated_at: i64,
}

const QUICKOPEN_TITLE_MAX_CHARS: usize = 80;

/// tasks 테이블에서 검색해 scope(task|session) 부여 + 요청 스코프로 필터.
/// scopes가 비어 있으면 전체 허용. local(Tauri command)·runner(HTTP) 양측이 공유(parity 보장).
pub async fn quickopen_search(
    pool: &SqlitePool,
    query: &str,
    scopes: &[String],
    limit: i64,
) -> anyhow::Result<Vec<QuickOpenCandidate>> {
    let wants = |scope: &str| scopes.is_empty() || scopes.iter().any(|s| s == scope);
    if !wants("task") && !wants("session") {
        return Ok(vec![]);
    }
    let rows = search_tasks(pool, query, limit.max(0)).await?;
    Ok(rows
        .into_iter()
        .filter_map(|task| {
            let scope = if task.mode == "conversation" {
                "session"
            } else {
                "task"
            };
            wants(scope).then(|| QuickOpenCandidate {
                scope,
                id: task.id,
                title: truncate_chars(&task.instruction, QUICKOPEN_TITLE_MAX_CHARS),
                subtitle: format!("{} · {}", task.repo, task.state),
                updated_at: task.updated_at,
            })
        })
        .collect())
}

fn truncate_chars(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        return input.to_string();
    }
    let mut out: String = input.chars().take(max).collect();
    out.push('…');
    out
}

pub async fn get_task(pool: &SqlitePool, id: i64) -> anyhow::Result<Option<Task>> {
    let row = sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    row.map(validate_loaded_task).transpose()
}

/// 관측된 실행 모델 한 건 — 벤더 CLI가 실제로 무엇으로 응답했는지.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ObservedModel {
    pub agent: String,
    pub model: String,
    /// 마지막으로 이 모델이 관측된 시각. 프런트가 최근 사용순으로 보여주는 근거.
    pub last_used_at: i64,
}

/// 상한 — 모델 종류는 원래 몇 개뿐이다. 이벤트가 오염돼 카디널리티가 폭발해도
/// 드롭다운이 수천 줄이 되지 않게 막는 안전장치.
const OBSERVED_MODEL_LIMIT: i64 = 200;

/// 지금까지 관측된 `resolved` 모델을 벤더별로 모은다 (최근 사용순).
///
/// 하드코딩 카탈로그를 보강하는 용도다 — claude/codex CLI 모두 "사용 가능한 모델 목록"을
/// 노출하지 않아, 신모델은 사용자가 자유입력으로 한 번 쓰기 전까지 알 방법이 없다.
/// 그 한 번을 관측해 두면 다음부터는 목록에 뜬다.
pub async fn observed_models(pool: &SqlitePool) -> anyhow::Result<Vec<ObservedModel>> {
    let rows = sqlx::query_as::<_, ObservedModel>(
        "SELECT COALESCE(json_extract(e.event, '$.agent'), t.agent) AS agent, \
                json_extract(e.event, '$.resolved') AS model, \
                MAX(e.ts) AS last_used_at \
         FROM convo_events e JOIN tasks t ON t.id = e.task_id \
         WHERE json_extract(e.event, '$.kind') = 'model_snapshot' \
           AND json_extract(e.event, '$.resolved') IS NOT NULL \
           AND COALESCE(json_extract(e.event, '$.agent'), t.agent) IS NOT NULL \
           AND TRIM(COALESCE(json_extract(e.event, '$.agent'), t.agent)) <> '' \
         GROUP BY COALESCE(json_extract(e.event, '$.agent'), t.agent), \
                  json_extract(e.event, '$.resolved') \
         ORDER BY last_used_at DESC \
         LIMIT ?",
    )
    .bind(OBSERVED_MODEL_LIMIT)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 이미 사용된 repo 경로들(중복 제거) — 외부기원(봇/크론) 작업의 repo 화이트리스트 근거.
pub async fn known_repos(pool: &SqlitePool) -> anyhow::Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT DISTINCT repo FROM tasks")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(r,)| r).collect())
}

// ── 크론 스케줄러(Phase 0) ─────────────────────────────

/// 크론 예약 (task 실행 또는 리마인더 전송). `payload`는 kind별 JSON.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Schedule {
    pub id: i64,
    pub label: String,
    pub cron: String,
    pub kind: String,
    pub payload: String,
    pub enabled: i64,
    pub last_run_at: Option<i64>,
    pub created_at: i64,
    /// 1회성 리마인더의 발화 예정 절대 epoch초. Some이면 1회성, None이면 기존 cron 반복.
    pub run_at: Option<i64>,
    /// 크론 달력 기준 timezone offset (초). 기본값 32400 = KST (+09:00).
    pub tz_offset_secs: i32,
}

pub async fn insert_schedule(
    pool: &SqlitePool,
    label: &str,
    cron: &str,
    kind: &str,
    payload: &str,
    now: i64,
    tz_offset_secs: i32,
) -> anyhow::Result<i64> {
    insert_schedule_with_run_at(pool, label, cron, kind, payload, None, now, tz_offset_secs).await
}

/// `run_at`(1회성 발화 절대 epoch초) 지정 가능한 확장 버전 — 리마인더는 `Some`, 기존
/// cron 반복 스케줄은 `None`을 넘긴다.
pub async fn insert_schedule_with_run_at(
    pool: &SqlitePool,
    label: &str,
    cron: &str,
    kind: &str,
    payload: &str,
    run_at: Option<i64>,
    now: i64,
    tz_offset_secs: i32,
) -> anyhow::Result<i64> {
    let id = sqlx::query(
        "INSERT INTO schedules (label, cron, kind, payload, enabled, last_run_at, created_at, run_at, tz_offset_secs) \
         VALUES (?, ?, ?, ?, 1, NULL, ?, ?, ?)",
    )
    .bind(label)
    .bind(cron)
    .bind(kind)
    .bind(payload)
    .bind(now)
    .bind(run_at)
    .bind(tz_offset_secs)
    .execute(pool)
    .await?
    .last_insert_rowid();
    Ok(id)
}

/// `Schedule`을 `FromRow`로 채우는 데 필요한 컬럼 전부.
///
/// `SELECT *`를 쓰지 않는 이유는 **컬럼 수를 고정하기 위해서**다. `*`가 무엇으로 펼쳐지는지는
/// 준비 시점의 스키마에 달려 있어, `ALTER TABLE ADD COLUMN`과 겹치면 sqlx가 없는 인덱스를 읽고
/// worker 스레드가 패닉한다(이슈 #153). 이름을 적어 두면 컬럼이 없을 때 prepare에서 즉시 실패한다.
const SCHEDULE_COLUMNS: &str = "id, label, cron, kind, payload, enabled, last_run_at, \
     created_at, run_at, tz_offset_secs";

pub async fn list_schedules(pool: &SqlitePool) -> anyhow::Result<Vec<Schedule>> {
    Ok(sqlx::query_as::<_, Schedule>(&format!(
        "SELECT {SCHEDULE_COLUMNS} FROM schedules ORDER BY id"
    ))
    .fetch_all(pool)
    .await?)
}

pub async fn remove_schedule(pool: &SqlitePool, id: i64) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM schedules WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_schedule_enabled(pool: &SqlitePool, id: i64, enabled: bool) -> anyhow::Result<()> {
    sqlx::query("UPDATE schedules SET enabled = ? WHERE id = ?")
        .bind(if enabled { 1 } else { 0 })
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 스케줄 실행 시각 기록 (틱당 1회 실행 보장의 근거 — schedule 모듈이 due 판정에 사용).
pub async fn mark_schedule_ran(pool: &SqlitePool, id: i64, at: i64) -> anyhow::Result<()> {
    sqlx::query("UPDATE schedules SET last_run_at = ? WHERE id = ?")
        .bind(at)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 활성화된 스케줄 전체 — due 판정(cron 파싱)은 schedule 모듈이 담당.
pub async fn list_enabled_schedules(pool: &SqlitePool) -> anyhow::Result<Vec<Schedule>> {
    Ok(sqlx::query_as::<_, Schedule>(&format!(
        "SELECT {SCHEDULE_COLUMNS} FROM schedules WHERE enabled = 1 ORDER BY id"
    ))
    .fetch_all(pool)
    .await?)
}
