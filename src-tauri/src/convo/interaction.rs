//! Durable, task-scoped question receipts. A committed dispatch is never replayed.
use super::native_interaction::NativeRequestInfo;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use std::collections::HashSet;

pub const RUNTIME: &str = "codex_questions_v1";
/// 인앱 MCP 툴로 질문하는 런타임. Codex의 dynamicTools와 입력 스키마를 공유한다.
pub const RUNTIME_LOCAL: &str = "claude_questions_v1";
/// 두 표면이 함께 쓰는 툴 이름. Claude에는 `mcp__praxis-preview__ask_user`로 보인다.
pub const TOOL_NAME: &str = "ask_user";
pub const CREATE_MODE: &str = "conversation_questions";
/// Claude `--permission-prompt-tool`이 부르는 호스트 툴. 에이전트가 아니라 CLI가 부른다.
pub const APPROVAL_TOOL: &str = "approve";
pub const QUESTION_TTL: i64 = 30 * 60;
pub const MAX_BYTES: usize = 16 * 1024;
const ACTIVE: &str = "('starting','running','cancelling','finalizing','cleanup_failed')";

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn input_validation(e: impl std::fmt::Display) -> String {
    format!("INPUT_VALIDATION: {e}")
}
pub fn id() -> Result<String, String> {
    crate::preview_bridge::random_hex_id()
}
fn hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OptionItem {
    pub id: String,
    pub label: String,
    pub description: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Question {
    pub id: String,
    pub question: String,
    pub options: Vec<OptionItem>,
    pub allow_free_text: bool,
    pub is_secret: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Questions {
    pub kind: String,
    pub questions: Vec<Question>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub question_id: String,
    pub option_id: Option<String>,
    pub text: Option<String>,
}

pub fn validate_questions(value: &Value) -> Result<Questions, String> {
    if value.to_string().len() > MAX_BYTES {
        return Err("질문 크기 제한을 초과했습니다".into());
    }
    let q: Questions =
        serde_json::from_value(value.clone()).map_err(|_| "지원하지 않는 질문 형식입니다")?;
    if q.kind != "clarification" || !(1..=3).contains(&q.questions.len()) {
        return Err("일반 확인 질문만 지원합니다".into());
    }
    let mut ids = HashSet::new();
    for item in &q.questions {
        if item.is_secret {
            return Err("비밀 입력은 지원하지 않습니다".into());
        }
        if !valid_id(&item.id)
            || !ids.insert(&item.id)
            || item.question.trim().is_empty()
            || item.question.chars().count() > 2000
            || item.options.len() > 3
            || (item.options.is_empty() && !item.allow_free_text)
        {
            return Err("질문 항목이 올바르지 않습니다".into());
        }
        let mut options = HashSet::new();
        for option in &item.options {
            if !valid_id(&option.id)
                || !options.insert(&option.id)
                || option.label.trim().is_empty()
                || option.label.chars().count() > 200
                || option.description.chars().count() > 1000
            {
                return Err("질문 선택지가 올바르지 않습니다".into());
            }
        }
    }
    Ok(q)
}
fn normalize_answers(
    q: &Questions,
    answers: &[Answer],
    complete: bool,
) -> Result<Vec<Answer>, String> {
    if serde_json::to_vec(answers).map_err(err)?.len() > MAX_BYTES {
        return Err("답변 크기 제한을 초과했습니다".into());
    }
    let mut seen = HashSet::new();
    for a in answers {
        let item = q
            .questions
            .iter()
            .find(|q| q.id == a.question_id)
            .ok_or("다른 질문의 답변입니다")?;
        if !seen.insert(&a.question_id) {
            return Err("질문 ID가 중복되었습니다".into());
        }
        match (&a.option_id, &a.text) {
            (Some(option), None) if item.options.iter().any(|o| &o.id == option) => {}
            (None, Some(text))
                if item.allow_free_text
                    && text.chars().count() <= 4000
                    && (!complete || !text.trim().is_empty()) => {}
            (None, None) if !complete => {}
            _ => return Err("답변 또는 선택지가 올바르지 않습니다".into()),
        }
    }
    if complete && answers.len() != q.questions.len() {
        return Err("모든 질문에 답해주세요".into());
    }
    let mut normalized = answers.to_vec();
    normalized.sort_by(|a, b| a.question_id.cmp(&b.question_id));
    Ok(normalized)
}
pub fn answer_output(answers: &[Answer]) -> String {
    json!({"schema_version":1,"answers":answers}).to_string()
}
pub fn tool_spec() -> Value {
    serde_json::from_str(include_str!("interaction-tool.json")).expect("embedded tool schema")
}
/// MCP `tools/list` 서술자. **입력 스키마는 위 파일이 유일한 원천이다** — 두 벌을 두면 어긋난다.
pub fn mcp_tool_spec() -> Value {
    let spec = tool_spec();
    let tool = &spec["tools"][0];
    json!({
        "name": TOOL_NAME,
        "description": tool["description"],
        "inputSchema": tool["inputSchema"],
    })
}
fn schema_hash(runtime: &str) -> String {
    match runtime {
        RUNTIME_LOCAL => hash(&mcp_tool_spec().to_string()),
        _ => hash(&tool_spec().to_string()),
    }
}
/// 에이전트가 고르는 런타임. 여기 없는 에이전트는 질문 세션을 열 수 없다.
pub fn runtime_for_agent(agent: &str) -> Option<&'static str> {
    match agent {
        "codex" => Some(RUNTIME),
        "claude" => Some(RUNTIME_LOCAL),
        _ => None,
    }
}
pub async fn migrate(pool: &SqlitePool) -> Result<(), String> {
    for sql in [
        "CREATE TABLE IF NOT EXISTS convo_runtime_bindings(task_id INTEGER PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,runtime_kind TEXT NOT NULL,tool_schema_hash TEXT NOT NULL)",
        "CREATE TABLE IF NOT EXISTS convo_executions(id TEXT PRIMARY KEY,task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,state TEXT NOT NULL,thread_id TEXT,turn_id TEXT,pgid INTEGER,identity_hash TEXT,process_marker TEXT,created_at INTEGER NOT NULL,error TEXT)",
        "CREATE UNIQUE INDEX IF NOT EXISTS convo_one_execution ON convo_executions(task_id) WHERE state IN ('starting','running','cancelling','finalizing','cleanup_failed')",
        "CREATE TABLE IF NOT EXISTS convo_interactions(id TEXT PRIMARY KEY,execution_id TEXT NOT NULL REFERENCES convo_executions(id) ON DELETE CASCADE,wire_id TEXT NOT NULL,call_id TEXT NOT NULL,questions TEXT NOT NULL,native_method TEXT,native_params TEXT,native_request TEXT,state TEXT NOT NULL DEFAULT 'pending',reason TEXT,revision INTEGER NOT NULL DEFAULT 1,created_at INTEGER NOT NULL,expires_at INTEGER NOT NULL,UNIQUE(execution_id,wire_id),UNIQUE(execution_id,call_id))",
        "CREATE TABLE IF NOT EXISTS convo_interaction_answers(id TEXT PRIMARY KEY,interaction_id TEXT NOT NULL REFERENCES convo_interactions(id) ON DELETE CASCADE,payload TEXT NOT NULL,payload_hash TEXT NOT NULL,state TEXT NOT NULL,created_at INTEGER NOT NULL)",
        "CREATE UNIQUE INDEX IF NOT EXISTS convo_one_answer ON convo_interaction_answers(interaction_id)",
        "CREATE TABLE IF NOT EXISTS convo_interaction_drafts(interaction_id TEXT PRIMARY KEY REFERENCES convo_interactions(id) ON DELETE CASCADE,payload TEXT NOT NULL,revision INTEGER NOT NULL)",
    ] { sqlx::query(sql).execute(pool).await.map_err(err)?; }
    let mut tx = pool.begin().await.map_err(err)?;
    let exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('convo_runtime_bindings') WHERE name='cli_version'",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(err)?;
    if exists == 0 {
        sqlx::query("ALTER TABLE convo_runtime_bindings ADD COLUMN cli_version TEXT")
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        // Historical question threads could only run with this exact adapter version.
        sqlx::query("UPDATE convo_runtime_bindings SET cli_version=? WHERE runtime_kind=?")
            .bind(super::cli_runtime::LEGACY_VERSION)
            .bind(RUNTIME)
            .execute(&mut *tx)
            .await
            .map_err(err)?;
    }
    let approvals: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('convo_runtime_bindings') WHERE name='approvals'",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(err)?;
    if approvals == 0 {
        sqlx::query("ALTER TABLE convo_runtime_bindings ADD COLUMN approvals INTEGER NOT NULL DEFAULT 0")
            .execute(&mut *tx)
            .await
            .map_err(err)?;
    }
    for column in ["native_method", "native_params", "native_request"] {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('convo_interactions') WHERE name=?",
        )
        .bind(column)
        .fetch_one(&mut *tx)
        .await
        .map_err(err)?;
        if exists == 0 {
            sqlx::query(&format!(
                "ALTER TABLE convo_interactions ADD COLUMN {column} TEXT"
            ))
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        }
    }
    tx.commit().await.map_err(err)?;
    Ok(())
}
/// 실행 전 승인은 Claude 질문 세션의 생성 옵션이다. 공급자를 바꿔도 값은 남고, Codex 차례는 읽지 않는다.
pub async fn set_approvals(pool: &SqlitePool, task: i64, on: bool) -> Result<(), String> {
    let updated = sqlx::query("UPDATE convo_runtime_bindings SET approvals=? WHERE task_id=?")
        .bind(on)
        .bind(task)
        .execute(pool)
        .await
        .map_err(err)?
        .rows_affected();
    if updated != 1 {
        return Err("질문 실행 계약이 없어 실행 전 승인을 켤 수 없습니다".into());
    }
    Ok(())
}
pub async fn approvals_of(pool: &SqlitePool, task: i64) -> Result<bool, String> {
    let on: Option<bool> =
        sqlx::query_scalar("SELECT approvals FROM convo_runtime_bindings WHERE task_id=?")
            .bind(task)
            .fetch_optional(pool)
            .await
            .map_err(err)?;
    Ok(on.unwrap_or(false))
}
/// `approve`의 MCP 서술자. 입력은 CLI가 정한 permission prompt 형태다(`tool_name`·`input`).
pub fn approval_tool_spec() -> Value {
    json!({
        "name": APPROVAL_TOOL,
        "description": "Host permission prompt used by the Claude CLI. Do not call this tool directly.",
        "inputSchema": {
            "type": "object",
            "required": ["tool_name", "input"],
            "properties": {
                "tool_name": {"type": "string"},
                "input": {"type": "object"},
                "tool_use_id": {"type": "string"}
            }
        }
    })
}
pub async fn bind(pool: &SqlitePool, task: i64) -> Result<(), String> {
    bind_runtime(pool, task, RUNTIME).await
}
pub async fn bind_runtime(pool: &SqlitePool, task: i64, runtime: &str) -> Result<(), String> {
    sqlx::query("INSERT INTO convo_runtime_bindings(task_id,runtime_kind,tool_schema_hash,cli_version) VALUES(?,?,?,?)")
        .bind(task)
        .bind(runtime)
        .bind(schema_hash(runtime))
        .bind((runtime == RUNTIME).then_some(super::cli_runtime::DEFAULT_VERSION))
        .execute(pool)
        .await
        .map_err(err)?;
    Ok(())
}
fn validate_binding(kind: &str, schema: &str, version: Option<&str>) -> Result<(), String> {
    match kind {
        RUNTIME if schema == schema_hash(RUNTIME) => super::cli_runtime::validate_version(
            version.ok_or("대화의 Codex 버전 기록이 없습니다")?,
        ),
        RUNTIME_LOCAL if schema == schema_hash(RUNTIME_LOCAL) => {
            // Claude가 질문을 주도해도 토론의 Codex 차례는 이 작업의 고정 런타임을 쓴다.
            // 옛 로컬 바인딩은 이 값이 없을 수 있어 `ensure_codex_version`만 채울 수 있다.
            if let Some(version) = version {
                super::cli_runtime::validate_version(version)
            } else {
                Ok(())
            }
        }
        _ => Err("이 질문 세션의 실행 계약을 지원하지 않습니다".into()),
    }
}
/// 바인딩이 없으면 `None`. 있는데 우리가 모르는 계약이면 **에러다** — 조용히 평범한 턴으로
/// 흘려보내면 열린 질문이 영영 응답되지 않는다.
pub async fn runtime_of(pool: &SqlitePool, task: i64) -> Result<Option<String>, String> {
    let row: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT runtime_kind,tool_schema_hash,cli_version FROM convo_runtime_bindings WHERE task_id=?",
    )
    .bind(task)
    .fetch_optional(pool)
    .await
    .map_err(err)?;
    match row {
        None => Ok(None),
        Some((kind, schema, version)) => {
            validate_binding(&kind, &schema, version.as_deref())?;
            Ok(Some(kind))
        }
    }
}
/// Never infer a thread's version from the currently installed global CLI.
pub async fn cli_version_of(pool: &SqlitePool, task: i64) -> Result<String, String> {
    let row: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT runtime_kind,tool_schema_hash,cli_version FROM convo_runtime_bindings WHERE task_id=?",
    )
    .bind(task)
    .fetch_optional(pool)
    .await
    .map_err(err)?;
    let (kind, schema, version) = row.ok_or("질문 실행 계약이 없습니다")?;
    validate_binding(&kind, &schema, version.as_deref())?;
    let version = version.ok_or("대화의 Codex 버전 기록이 없습니다")?;
    Ok(version)
}
/// Return the task's pinned Codex runtime. A historical Claude binding may not have needed a
/// Codex pin; it is filled exactly once when a Codex debate turn needs one.
pub async fn ensure_codex_version(pool: &SqlitePool, task: i64) -> Result<String, String> {
    let mut tx = pool.begin().await.map_err(err)?;
    let row: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT runtime_kind,tool_schema_hash,cli_version FROM convo_runtime_bindings WHERE task_id=?",
    )
    .bind(task)
    .fetch_optional(&mut *tx)
    .await
    .map_err(err)?;
    let (kind, schema, version) = row.ok_or("질문 실행 계약이 없습니다")?;
    validate_binding(&kind, &schema, version.as_deref())?;
    let version = match (kind.as_str(), version) {
        (_, Some(version)) => version,
        (RUNTIME_LOCAL, None) => {
            let version = super::cli_runtime::DEFAULT_VERSION.to_string();
            let updated = sqlx::query(
                "UPDATE convo_runtime_bindings SET cli_version=? WHERE task_id=? AND runtime_kind=? AND cli_version IS NULL",
            )
            .bind(&version)
            .bind(task)
            .bind(RUNTIME_LOCAL)
            .execute(&mut *tx)
            .await
            .map_err(err)?
            .rows_affected();
            if updated != 1 {
                return Err("대화의 Codex 버전 기록을 고정하지 못했습니다".into());
            }
            version
        }
        (RUNTIME, None) => return Err("대화의 Codex 버전 기록이 없습니다".into()),
        _ => return Err("이 질문 세션의 실행 계약을 지원하지 않습니다".into()),
    };
    tx.commit().await.map_err(err)?;
    Ok(version)
}
/// Rebind an existing question contract while its caller updates the task's selected agent.
/// Keeping this inside that transaction prevents the task row and tool contract from drifting.
pub async fn rebind_for_agent_tx(
    tx: &mut Transaction<'_, Sqlite>,
    task: i64,
    agent: &str,
) -> Result<(), String> {
    let row: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT runtime_kind,tool_schema_hash,cli_version FROM convo_runtime_bindings WHERE task_id=?",
    )
    .bind(task)
    .fetch_optional(&mut **tx)
    .await
    .map_err(err)?;
    let Some((old_kind, old_schema, old_version)) = row else {
        return Ok(());
    };
    validate_binding(&old_kind, &old_schema, old_version.as_deref())?;
    let new_kind = runtime_for_agent(agent)
        .ok_or_else(|| format!("질문 응답은 Codex·Claude 사이의 전환을 지원합니다: {agent}"))?;
    let active: i64 = sqlx::query_scalar(&format!(
        "SELECT EXISTS(SELECT 1 FROM convo_executions WHERE task_id=? AND state IN {ACTIVE})"
    ))
    .bind(task)
    .fetch_one(&mut **tx)
    .await
    .map_err(err)?;
    if active != 0 {
        return Err("진행 중인 질문 실행이 있어 에이전트를 바꿀 수 없습니다".into());
    }
    let version = match (old_kind.as_str(), new_kind, old_version) {
        (RUNTIME_LOCAL, RUNTIME, None) => Some(super::cli_runtime::DEFAULT_VERSION.to_string()),
        (_, _, version) => version,
    };
    let updated = sqlx::query(
        "UPDATE convo_runtime_bindings SET runtime_kind=?,tool_schema_hash=?,cli_version=? WHERE task_id=?",
    )
    .bind(new_kind)
    .bind(schema_hash(new_kind))
    .bind(version)
    .bind(task)
    .execute(&mut **tx)
    .await
    .map_err(err)?
    .rows_affected();
    if updated != 1 {
        return Err("질문 실행 계약을 전환하지 못했습니다".into());
    }
    Ok(())
}
pub async fn is_bound(pool: &SqlitePool, task: i64) -> Result<bool, String> {
    Ok(runtime_of(pool, task).await?.is_some())
}
pub async fn begin(pool: &SqlitePool, task: i64, now: i64) -> Result<String, String> {
    let execution = id()?;
    sqlx::query(
        "INSERT INTO convo_executions(id,task_id,state,created_at) VALUES(?,?,'starting',?)",
    )
    .bind(&execution)
    .bind(task)
    .bind(now)
    .execute(pool)
    .await
    .map_err(err)?;
    Ok(execution)
}
pub async fn phase(pool: &SqlitePool, execution: &str, state: &str) -> Result<(), String> {
    sqlx::query("UPDATE convo_executions SET state=? WHERE id=?")
        .bind(state)
        .bind(execution)
        .execute(pool)
        .await
        .map_err(err)?;
    Ok(())
}
pub async fn spawned(
    pool: &SqlitePool,
    execution: &str,
    pgid: u32,
    identity: &str,
    marker: &str,
) -> Result<(), String> {
    sqlx::query("UPDATE convo_executions SET pgid=?,identity_hash=?,process_marker=? WHERE id=?")
        .bind(i64::from(pgid))
        .bind(identity)
        .bind(marker)
        .bind(execution)
        .execute(pool)
        .await
        .map_err(err)?;
    Ok(())
}
pub async fn started(
    pool: &SqlitePool,
    execution: &str,
    thread: &str,
    turn: &str,
) -> Result<(), String> {
    started_on_side(pool, execution, thread, turn, None).await
}
/// Accept a started turn and persist its vendor session in the owner for that debate side.
/// `None` is the historical single-session path; `Left` deliberately shares that task owner.
pub async fn started_on_side(
    pool: &SqlitePool,
    execution: &str,
    thread: &str,
    turn: &str,
    side: Option<super::Side>,
) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(err)?;
    let affected=sqlx::query("UPDATE convo_executions SET state='running',thread_id=?,turn_id=? WHERE id=? AND state='starting'")
        .bind(thread).bind(turn).bind(execution).execute(&mut *tx).await.map_err(err)?.rows_affected();
    if affected != 1 {
        return Err("실행 시작 상태가 변경되었습니다".into());
    }
    let persisted = match side {
        Some(side @ (super::Side::Right | super::Side::Third)) => sqlx::query(
            "UPDATE convo_debate_sides SET vendor_session_id=? WHERE task_id=(SELECT task_id FROM convo_executions WHERE id=?) AND side=?",
        )
        .bind(thread)
        .bind(execution)
        .bind(side.as_str())
        .execute(&mut *tx)
        .await
        .map_err(err)?
        .rows_affected(),
        None | Some(super::Side::Left) => sqlx::query(
            "UPDATE tasks SET convo_session_id=?,pending_capsule=NULL WHERE id=(SELECT task_id FROM convo_executions WHERE id=?)",
        )
        .bind(thread)
        .bind(execution)
        .execute(&mut *tx)
        .await
        .map_err(err)?
        .rows_affected(),
    };
    if persisted != 1 {
        return Err("대화 세션을 저장할 토론 자리를 찾지 못했습니다".into());
    }
    tx.commit().await.map_err(err)?;
    Ok(())
}
/// 로컬 MCP 런타임의 시작. 스레드 id 없이 상태만 올린다 — Claude의 세션 id는 `system/init`에서야
/// 오는데 `ask_user`는 그보다 먼저 올 수 있고, `open`은 실행이 `running`이어야 받아준다.
pub async fn started_local(pool: &SqlitePool, execution: &str, turn: &str) -> Result<(), String> {
    let affected = sqlx::query(
        "UPDATE convo_executions SET state='running',turn_id=? WHERE id=? AND state='starting'",
    )
    .bind(turn)
    .bind(execution)
    .execute(pool)
    .await
    .map_err(err)?
    .rows_affected();
    if affected != 1 {
        return Err("실행 시작 상태가 변경되었습니다".into());
    }
    Ok(())
}
/// 세션 id가 늦게 도착했을 때 실행 행에 이어 붙인다. 작업의 `convo_session_id`는 턴 경로가 따로 쓴다.
pub async fn thread_bound(pool: &SqlitePool, execution: &str, thread: &str) -> Result<(), String> {
    sqlx::query("UPDATE convo_executions SET thread_id=? WHERE id=?")
        .bind(thread)
        .bind(execution)
        .execute(pool)
        .await
        .map_err(err)?;
    Ok(())
}
/// 프로세스 마커가 없는 spawn 기록. 회수는 pgid와 신원 대조만으로 한다.
pub async fn spawned_local(
    pool: &SqlitePool,
    execution: &str,
    pgid: u32,
    identity: &str,
) -> Result<(), String> {
    sqlx::query("UPDATE convo_executions SET pgid=?,identity_hash=? WHERE id=?")
        .bind(i64::from(pgid))
        .bind(identity)
        .bind(execution)
        .execute(pool)
        .await
        .map_err(err)?;
    Ok(())
}
pub async fn open(
    pool: &SqlitePool,
    execution: &str,
    wire_id: &Value,
    call: &str,
    args: &Value,
    now: i64,
) -> Result<String, String> {
    let q = validate_questions(args)?;
    if !valid_id(call) || !(wire_id.is_string() || wire_id.is_i64() || wire_id.is_u64()) {
        return Err("질문 요청 ID가 올바르지 않습니다".into());
    }
    let pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM convo_interactions WHERE execution_id=? AND state='pending'",
    )
    .bind(execution)
    .fetch_one(pool)
    .await
    .map_err(err)?;
    if pending >= 8 {
        return Err("미응답 질문 한도를 초과했습니다".into());
    }
    let interaction = id()?;
    sqlx::query("INSERT INTO convo_interactions(id,execution_id,wire_id,call_id,questions,created_at,expires_at) SELECT ?,id,?,?,?,?,? FROM convo_executions WHERE id=? AND state='running'")
        .bind(&interaction).bind(wire_id.to_string()).bind(call).bind(serde_json::to_string(&q).map_err(err)?).bind(now).bind(now+QUESTION_TTL).bind(execution).execute(pool).await.map_err(err)?.rows_affected().eq(&1).then_some(()).ok_or("질문 실행이 종료되었습니다")?;
    Ok(interaction)
}
/// Open an app-server-originated request. Native calls retain their original protocol envelope
/// privately; the public interaction shape stays the established clarification shape.
pub async fn open_native(
    pool: &SqlitePool,
    execution: &str,
    wire_id: &Value,
    method: &str,
    params: &Value,
    now: i64,
) -> Result<String, String> {
    if !(wire_id.is_string() || wire_id.is_i64() || wire_id.is_u64()) {
        return Err("native 요청 ID가 올바르지 않습니다".into());
    }
    if serde_json::to_vec(params).map_err(err)?.len() > MAX_BYTES {
        return Err("native 요청 크기 제한을 초과했습니다".into());
    }
    let native = super::native_interaction::normalize(method, params)?;
    let pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM convo_interactions WHERE execution_id=? AND state='pending'",
    )
    .bind(execution)
    .fetch_one(pool)
    .await
    .map_err(err)?;
    if pending >= 8 {
        return Err("미응답 질문 한도를 초과했습니다".into());
    }
    let interaction = id()?;
    // A single provider item can produce several approvals. The JSON-RPC request id is the only
    // protocol correlation key, so preserve it verbatim and namespace its local call id.
    let call = format!("native:{}", wire_id);
    let inserted = sqlx::query("INSERT INTO convo_interactions(id,execution_id,wire_id,call_id,questions,native_method,native_params,native_request,created_at,expires_at) SELECT ?,id,?,?,?,?,?,?,?,? FROM convo_executions WHERE id=? AND state='running'")
        .bind(&interaction)
        .bind(wire_id.to_string())
        .bind(call)
        .bind(serde_json::to_string(&native.questions).map_err(err)?)
        .bind(method)
        .bind(serde_json::to_string(params).map_err(err)?)
        .bind(serde_json::to_string(&native.info).map_err(err)?)
        .bind(now)
        .bind(now + QUESTION_TTL)
        .bind(execution)
        .execute(pool)
        .await
        .map_err(err)?
        .rows_affected();
    if inserted != 1 {
        return Err("질문 실행이 종료되었거나 이미 받은 native 요청입니다".into());
    }
    Ok(interaction)
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Receipt {
    pub request_id: String,
    pub state: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct Interaction {
    pub id: String,
    pub execution_id: String,
    pub call_id: String,
    pub questions: Questions,
    pub state: String,
    pub reason: Option<String>,
    pub revision: i64,
    pub expires_at: i64,
    pub draft: Vec<Answer>,
    pub draft_revision: i64,
    pub receipt: Option<Receipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<NativeRequestInfo>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub enabled: bool,
    pub execution_id: Option<String>,
    pub phase: String,
    pub items: Vec<Interaction>,
}
pub async fn snapshot(pool: &SqlitePool, task: i64) -> Result<Snapshot, String> {
    let enabled = is_bound(pool, task).await?;
    let active: Option<(String,String)> = sqlx::query_as(&format!("SELECT id,state FROM convo_executions WHERE task_id=? AND state IN {ACTIVE} ORDER BY created_at DESC LIMIT 1")).bind(task).fetch_optional(pool).await.map_err(err)?;
    let rows=sqlx::query("SELECT i.*,d.payload AS draft,d.revision AS draft_revision,a.id AS answer_id,a.payload AS answer,a.state AS answer_state FROM convo_interactions i JOIN convo_executions e ON e.id=i.execution_id LEFT JOIN convo_interaction_drafts d ON d.interaction_id=i.id LEFT JOIN convo_interaction_answers a ON a.interaction_id=i.id WHERE e.task_id=? ORDER BY i.created_at,i.rowid").bind(task).fetch_all(pool).await.map_err(err)?;
    let mut items = Vec::new();
    for r in rows {
        let payload: Option<String> = r.get::<Option<String>, _>("answer").or(r.get("draft"));
        let receipt = r.get::<Option<String>, _>("answer_id").map(|id| Receipt {
            request_id: id,
            state: r.get::<String, _>("answer_state"),
        });
        items.push(Interaction {
            id: r.get("id"),
            execution_id: r.get("execution_id"),
            call_id: r.get("call_id"),
            questions: serde_json::from_str(r.get("questions")).map_err(err)?,
            state: r.get("state"),
            reason: r.get("reason"),
            revision: r.get("revision"),
            expires_at: r.get("expires_at"),
            draft: payload
                .map(|s| serde_json::from_str(&s))
                .transpose()
                .map_err(err)?
                .unwrap_or_default(),
            draft_revision: r.get::<Option<i64>, _>("draft_revision").unwrap_or(0),
            receipt,
            request: r
                .get::<Option<String>, _>("native_request")
                .map(|value| serde_json::from_str(&value))
                .transpose()
                .map_err(err)?,
        });
    }
    Ok(Snapshot {
        enabled,
        execution_id: active.as_ref().map(|v| v.0.clone()),
        phase: active.map(|v| v.1).unwrap_or_else(|| "idle".into()),
        items,
    })
}
async fn question_for(
    pool: &SqlitePool,
    task: i64,
    execution: &str,
    interaction: &str,
    now: i64,
) -> Result<Questions, String> {
    let q: Option<String> = sqlx::query_scalar("SELECT i.questions FROM convo_interactions i JOIN convo_executions e ON e.id=i.execution_id WHERE e.task_id=? AND e.id=? AND i.id=? AND e.state='running' AND i.state='pending' AND i.expires_at>?").bind(task).bind(execution).bind(interaction).bind(now).fetch_optional(pool).await.map_err(err)?;
    serde_json::from_str(&q.ok_or("이 질문은 더 이상 답변을 받지 않습니다")?).map_err(err)
}
pub async fn draft(
    pool: &SqlitePool,
    task: i64,
    execution: &str,
    interaction: &str,
    answers: &[Answer],
    revision: i64,
    now: i64,
) -> Result<i64, String> {
    let q = question_for(pool, task, execution, interaction, now).await?;
    let payload = serde_json::to_string(&normalize_answers(&q, answers, false)?).map_err(err)?;
    let changed=sqlx::query("INSERT INTO convo_interaction_drafts(interaction_id,payload,revision) SELECT ?,?,1 WHERE ?=0 AND EXISTS(SELECT 1 FROM convo_interactions i JOIN convo_executions e ON e.id=i.execution_id WHERE i.id=? AND i.state='pending' AND e.state='running') AND NOT EXISTS(SELECT 1 FROM convo_interaction_answers WHERE interaction_id=?) ON CONFLICT(interaction_id) DO UPDATE SET payload=excluded.payload,revision=convo_interaction_drafts.revision+1 WHERE convo_interaction_drafts.revision=?")
      .bind(interaction).bind(payload).bind(revision).bind(interaction).bind(interaction).bind(revision).execute(pool).await.map_err(err)?.rows_affected();
    // Existing rows need a CAS update; the insert's predicate intentionally rejects nonzero revisions.
    if changed == 0 && revision > 0 {
        let payload =
            serde_json::to_string(&normalize_answers(&q, answers, false)?).map_err(err)?;
        let n=sqlx::query("UPDATE convo_interaction_drafts SET payload=?,revision=revision+1 WHERE interaction_id=? AND revision=? AND EXISTS(SELECT 1 FROM convo_interactions i JOIN convo_executions e ON e.id=i.execution_id WHERE i.id=? AND i.state='pending' AND e.state='running') AND NOT EXISTS(SELECT 1 FROM convo_interaction_answers WHERE interaction_id=?)")
          .bind(payload).bind(interaction).bind(revision).bind(interaction).bind(interaction).execute(pool).await.map_err(err)?.rows_affected();
        if n == 1 {
            return Ok(revision + 1);
        }
    }
    if changed == 1 {
        Ok(revision + 1)
    } else {
        Err("초안이 변경되었거나 질문이 종료되었습니다".into())
    }
}
pub async fn receipt(pool: &SqlitePool, task: i64, request: &str) -> Result<Receipt, String> {
    let state: Option<String>=sqlx::query_scalar("SELECT a.state FROM convo_interaction_answers a JOIN convo_interactions i ON i.id=a.interaction_id JOIN convo_executions e ON e.id=i.execution_id WHERE e.task_id=? AND a.id=?").bind(task).bind(request).fetch_optional(pool).await.map_err(err)?;
    Ok(Receipt {
        request_id: request.into(),
        state: state.unwrap_or_else(|| "not_found".into()),
    })
}
pub async fn submit(
    pool: &SqlitePool,
    task: i64,
    execution: &str,
    interaction: &str,
    request: &str,
    answers: &[Answer],
    now: i64,
) -> Result<Receipt, String> {
    if !valid_id(request) {
        return Err("답변 접수 ID가 올바르지 않습니다".into());
    }
    let mut sorted = answers.to_vec();
    sorted.sort_by(|a, b| a.question_id.cmp(&b.question_id));
    let payload = serde_json::to_string(&sorted).map_err(err)?;
    let fingerprint = hash(&payload);
    let old:Option<(String,String,String,String,i64)>=sqlx::query_as("SELECT a.payload_hash,a.state,a.interaction_id,e.id,e.task_id FROM convo_interaction_answers a JOIN convo_interactions i ON i.id=a.interaction_id JOIN convo_executions e ON e.id=i.execution_id WHERE a.id=?").bind(request).fetch_optional(pool).await.map_err(err)?;
    if let Some((old, state, owner, exec, tid)) = old {
        if old != fingerprint || owner != interaction || exec != execution || tid != task {
            return Err("같은 접수 ID에 다른 답변을 보낼 수 없습니다".into());
        }
        return Ok(Receipt {
            request_id: request.into(),
            state,
        });
    }
    let q = question_for(pool, task, execution, interaction, now).await?;
    // Validate the protocol-shaped result before claiming the receipt. In particular, this keeps
    // malformed MCP numbers and schema-bound strings from becoming an undeliverable dispatch.
    let native: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT native_method,native_params FROM convo_interactions WHERE id=? AND execution_id=?",
    )
    .bind(interaction)
    .bind(execution)
    .fetch_optional(pool)
    .await
    .map_err(err)?;
    let negative_form = match native.as_ref() {
        Some((Some(method), Some(params))) => {
            let params = serde_json::from_str(params).map_err(err)?;
            super::native_interaction::is_negative_form_response(method, &params, &sorted)
                .map_err(input_validation)?
        }
        _ => false,
    };
    if !negative_form {
        normalize_answers(&q, &sorted, true).map_err(input_validation)?;
    } else {
        // A declined/cancelled MCP form deliberately omits its fields, but it is still subject to
        // the normal size, question-id, duplicate, and option validation rules.
        normalize_answers(&q, &sorted, false).map_err(input_validation)?;
    }
    match native {
        Some((Some(method), Some(params))) => {
            let params = serde_json::from_str(&params).map_err(err)?;
            super::native_interaction::response(&method, &params, &sorted)
                .map_err(input_validation)?;
        }
        Some((None, None)) => {}
        Some(_) => return Err("native 요청 기록이 손상되었습니다".into()),
        None => return Err("이 질문은 더 이상 답변을 받지 않습니다".into()),
    }
    let mut tx = pool.begin().await.map_err(err)?;
    let n=sqlx::query("UPDATE convo_interactions SET revision=revision+1 WHERE id=? AND execution_id=? AND state='pending' AND expires_at>? AND EXISTS(SELECT 1 FROM convo_executions WHERE id=? AND task_id=? AND state='running') AND NOT EXISTS(SELECT 1 FROM convo_interaction_answers WHERE interaction_id=?)")
      .bind(interaction).bind(execution).bind(now).bind(execution).bind(task).bind(interaction).execute(&mut *tx).await.map_err(err)?.rows_affected();
    if n != 1 {
        tx.rollback().await.map_err(err)?;
        let duplicate:Option<(String,String)>=sqlx::query_as("SELECT payload_hash,state FROM convo_interaction_answers WHERE id=? AND interaction_id=?").bind(request).bind(interaction).fetch_optional(pool).await.map_err(err)?;
        if let Some((old, state)) = duplicate {
            if old == fingerprint {
                return Ok(Receipt {
                    request_id: request.into(),
                    state,
                });
            }
        }
        return Err("이미 답변을 접수했거나 질문이 종료되었습니다".into());
    }
    sqlx::query("INSERT INTO convo_interaction_answers VALUES(?,?,?,?,'claimed',?)")
        .bind(request)
        .bind(interaction)
        .bind(payload)
        .bind(fingerprint)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(err)?;
    sqlx::query("DELETE FROM convo_interaction_drafts WHERE interaction_id=?")
        .bind(interaction)
        .execute(&mut *tx)
        .await
        .map_err(err)?;
    tx.commit().await.map_err(err)?;
    Ok(Receipt {
        request_id: request.into(),
        state: "claimed".into(),
    })
}
#[derive(Debug)]
pub struct Dispatch {
    pub answer_id: String,
    pub interaction_id: String,
    pub wire_id: Value,
    pub call_id: String,
    pub output: String,
    /// Native app-server requests receive their protocol response here. `output` remains the
    /// legacy dynamic-tool result for Claude/Codex clarification calls.
    pub native_result: Option<Value>,
    /// `Some` identifies a native request. Async host messages use this to route their result via
    /// `turn/steer` instead of replying to an invented JSON-RPC id.
    pub native_method: Option<String>,
}
pub async fn take_dispatch(
    pool: &SqlitePool,
    execution: &str,
    now: i64,
) -> Result<Option<Dispatch>, String> {
    take_dispatch_where(pool, execution, None, now).await
}
/// interaction 하나에 묶인 변종. 로컬 런타임은 질문마다 **각자의 호출이 대기**하므로, 실행 단위로
/// 집으면 먼저 깨어난 대기자가 남의 답을 가져간다.
pub async fn take_dispatch_for(
    pool: &SqlitePool,
    execution: &str,
    interaction: &str,
    now: i64,
) -> Result<Option<Dispatch>, String> {
    take_dispatch_where(pool, execution, Some(interaction), now).await
}
async fn take_dispatch_where(
    pool: &SqlitePool,
    execution: &str,
    interaction: Option<&str>,
    now: i64,
) -> Result<Option<Dispatch>, String> {
    let mut tx = pool.begin().await.map_err(err)?;
    let row=sqlx::query("SELECT a.id,a.payload,i.id AS interaction_id,i.wire_id,i.call_id,i.native_method,i.native_params FROM convo_interaction_answers a JOIN convo_interactions i ON i.id=a.interaction_id JOIN convo_executions e ON e.id=i.execution_id WHERE e.id=? AND e.state='running' AND i.state='pending' AND a.state='claimed' AND i.expires_at>? AND (? IS NULL OR i.id=?) ORDER BY a.created_at LIMIT 1").bind(execution).bind(now).bind(interaction).bind(interaction).fetch_optional(&mut *tx).await.map_err(err)?;
    let Some(r) = row else { return Ok(None) };
    let answer_id: String = r.get("id");
    sqlx::query(
        "UPDATE convo_interaction_answers SET state='dispatching' WHERE id=? AND state='claimed'",
    )
    .bind(&answer_id)
    .execute(&mut *tx)
    .await
    .map_err(err)?;
    tx.commit().await.map_err(err)?;
    let answers: Vec<Answer> = serde_json::from_str(r.get("payload")).map_err(err)?;
    let native_result = match (
        r.get::<Option<String>, _>("native_method"),
        r.get::<Option<String>, _>("native_params"),
    ) {
        (Some(method), Some(params)) => Some(super::native_interaction::response(
            &method,
            &serde_json::from_str(&params).map_err(err)?,
            &answers,
        )?),
        (None, None) => None,
        _ => return Err("native 요청 기록이 손상되었습니다".into()),
    };
    Ok(Some(Dispatch {
        answer_id,
        interaction_id: r.get("interaction_id"),
        wire_id: serde_json::from_str(r.get("wire_id")).map_err(err)?,
        call_id: r.get("call_id"),
        output: answer_output(&answers),
        native_result,
        native_method: r.get("native_method"),
    }))
}
pub async fn written(pool: &SqlitePool, answer: &str) -> Result<(), String> {
    sqlx::query(
        "UPDATE convo_interaction_answers SET state='written' WHERE id=? AND state='dispatching'",
    )
    .bind(answer)
    .execute(pool)
    .await
    .map_err(err)?;
    Ok(())
}
/// The host attempted to relay a native response but the app-server rejected it. Keep a durable
/// receipt distinct from delivery, and let `serverRequest/resolved` close the interaction.
pub async fn reject_native_dispatch(pool: &SqlitePool, answer: &str) -> Result<(), String> {
    sqlx::query(
        "UPDATE convo_interaction_answers SET state='rejected' WHERE id=? AND state='dispatching'",
    )
    .bind(answer)
    .execute(pool)
    .await
    .map_err(err)?;
    Ok(())
}
pub async fn acknowledge(
    pool: &SqlitePool,
    execution: &str,
    call: &str,
    contents: &Value,
    success: bool,
) -> Result<bool, String> {
    let row:Option<(String,String)>=sqlx::query_as("SELECT a.id,a.payload FROM convo_interaction_answers a JOIN convo_interactions i ON i.id=a.interaction_id JOIN convo_executions e ON e.id=i.execution_id WHERE e.id=? AND e.state='running' AND i.call_id=? AND i.state='pending' AND a.state='written'").bind(execution).bind(call).fetch_optional(pool).await.map_err(err)?;
    let Some((answer, payload)) = row else {
        return Ok(false);
    };
    let answers: Vec<Answer> = serde_json::from_str(&payload).map_err(err)?;
    if !success || *contents != json!([{"type":"inputText","text":answer_output(&answers)}]) {
        return Ok(false);
    }
    let mut tx = pool.begin().await.map_err(err)?;
    sqlx::query("UPDATE convo_interaction_answers SET state='acknowledged' WHERE id=?")
        .bind(answer)
        .execute(&mut *tx)
        .await
        .map_err(err)?;
    sqlx::query("UPDATE convo_interactions SET state='closed',reason='answered',revision=revision+1 WHERE execution_id=? AND call_id=?").bind(execution).bind(call).execute(&mut *tx).await.map_err(err)?;
    tx.commit().await.map_err(err)?;
    Ok(true)
}
/// 로컬 런타임의 종결. Codex의 `acknowledge`가 모델이 돌려준 내용을 대조하는 자리에서, 여기서는
/// **호스트가 직접 결과를 돌려주므로** 대조할 상대가 없다 — 쓴 즉시 닫는다.
pub async fn settle(pool: &SqlitePool, execution: &str, call: &str) -> Result<bool, String> {
    let mut tx = pool.begin().await.map_err(err)?;
    let answered=sqlx::query("UPDATE convo_interaction_answers SET state='acknowledged' WHERE state='written' AND interaction_id IN (SELECT i.id FROM convo_interactions i WHERE i.execution_id=? AND i.call_id=?)").bind(execution).bind(call).execute(&mut *tx).await.map_err(err)?.rows_affected();
    sqlx::query("UPDATE convo_interactions SET state='closed',reason='answered',revision=revision+1 WHERE execution_id=? AND call_id=? AND state='pending'").bind(execution).bind(call).execute(&mut *tx).await.map_err(err)?;
    tx.commit().await.map_err(err)?;
    Ok(answered == 1)
}
/// 대기 중인 호출이 계속 기다려도 되는가 — 실행이 살아 있고, 질문이 열려 있고, 만료 전인가.
pub async fn interaction_open(
    pool: &SqlitePool,
    interaction: &str,
    now: i64,
) -> Result<bool, String> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM convo_interactions i JOIN convo_executions e ON e.id=i.execution_id WHERE i.id=? AND i.state='pending' AND e.state='running' AND i.expires_at>?)")
        .bind(interaction)
        .bind(now)
        .fetch_one(pool)
        .await
        .map_err(err)
}
pub async fn pending(pool: &SqlitePool, execution: &str, now: i64) -> Result<(i64, bool), String> {
    let (count,expired):(i64,i64)=sqlx::query_as("SELECT COUNT(*),COALESCE(MAX(expires_at<=?),0) FROM convo_interactions WHERE execution_id=? AND state='pending'").bind(now).bind(execution).fetch_one(pool).await.map_err(err)?;
    Ok((count, expired != 0))
}
/// Native non-blocking input may remain visible while a turn finishes. Legacy question calls
/// were always blocking, so they remain in this count.
pub async fn blocking_pending(
    pool: &SqlitePool,
    execution: &str,
    now: i64,
) -> Result<(i64, bool), String> {
    let (count, expired): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*),COALESCE(MAX(expires_at<=?),0) FROM convo_interactions WHERE execution_id=? AND state='pending' AND (native_request IS NULL OR json_extract(native_request,'$.blocking')=1)",
    )
    .bind(now)
    .bind(execution)
    .fetch_one(pool)
    .await
    .map_err(err)?;
    Ok((count, expired != 0))
}
/// `serverRequest/resolved` confirms that the app-server stopped waiting for this request. It is
/// not an acknowledgement that the model consumed a response, so answer receipt state is kept.
pub async fn resolve_native(
    pool: &SqlitePool,
    execution: &str,
    wire_id: &Value,
) -> Result<bool, String> {
    if !(wire_id.is_string() || wire_id.is_i64() || wire_id.is_u64()) {
        return Ok(false);
    }
    let changed = sqlx::query("UPDATE convo_interactions SET state='closed',reason='resolved',revision=revision+1 WHERE execution_id=? AND wire_id=? AND native_method IS NOT NULL AND state='pending'")
        .bind(execution)
        .bind(wire_id.to_string())
        .execute(pool)
        .await
        .map_err(err)?
        .rows_affected();
    Ok(changed == 1)
}
pub async fn close_questions(
    pool: &SqlitePool,
    execution: &str,
    reason: &str,
) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(err)?;
    sqlx::query("UPDATE convo_interactions SET state='closed',reason=?,revision=revision+1 WHERE execution_id=? AND state='pending'").bind(reason).bind(execution).execute(&mut *tx).await.map_err(err)?;
    sqlx::query("UPDATE convo_interaction_answers SET state='unknown' WHERE state IN ('claimed','dispatching') AND interaction_id IN (SELECT id FROM convo_interactions WHERE execution_id=?)").bind(execution).execute(&mut *tx).await.map_err(err)?;
    tx.commit().await.map_err(err)?;
    Ok(())
}
pub async fn finish(
    pool: &SqlitePool,
    execution: &str,
    state: &str,
    reason: Option<&str>,
) -> Result<(), String> {
    close_questions(pool, execution, reason.unwrap_or("turn_ended")).await?;
    sqlx::query("UPDATE convo_executions SET state=?,error=?,pgid=NULL,identity_hash=NULL,process_marker=NULL WHERE id=?").bind(state).bind(reason).bind(execution).execute(pool).await.map_err(err)?;
    Ok(())
}
/// 답변을 기다리는 질문이 열린 작업 id 목록. 사이드바가 질문 점·라벨을 겹치는 근거다.
/// 조건은 `question_for`와 같다 — 실행이 `running`이고 질문이 `pending`이며 만료 전이어야 한다.
/// 구조화 질문은 `tasks.state`를 바꾸지 않으므로 여기 말고는 드러날 곳이 없다.
pub async fn pending_tasks(pool: &SqlitePool, now: i64) -> Result<Vec<i64>, String> {
    sqlx::query_scalar("SELECT DISTINCT e.task_id FROM convo_interactions i JOIN convo_executions e ON e.id=i.execution_id WHERE e.state='running' AND i.state='pending' AND i.expires_at>? ORDER BY e.task_id").bind(now).fetch_all(pool).await.map_err(err)
}
pub async fn blocked(pool: &SqlitePool, task: i64) -> Result<bool, String> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM convo_executions WHERE task_id=? AND state='cleanup_failed')",
    )
    .bind(task)
    .fetch_one(pool)
    .await
    .map_err(err)
}
/// Admission paths that would change a debate or runtime contract require no live execution.
pub async fn ensure_idle(pool: &SqlitePool, task: i64) -> Result<(), String> {
    let active: i64 = sqlx::query_scalar(&format!(
        "SELECT EXISTS(SELECT 1 FROM convo_executions WHERE task_id=? AND state IN {ACTIVE})"
    ))
    .bind(task)
    .fetch_one(pool)
    .await
    .map_err(err)?;
    if active != 0 {
        Err("진행 중인 질문 실행이 있어 이 작업을 변경할 수 없습니다".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "interaction_tests.rs"]
mod tests;
