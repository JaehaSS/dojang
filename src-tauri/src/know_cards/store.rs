//! 알아둘 것 카드의 저장 — 카드, 주제(이미 앎·최근), 판정 기록(설계 3.7).
//!
//! 백오프 상태는 따로 저장하지 않는다. 카드의 연속 무시 수와 판정 기록 끝쪽의 연속
//! `skipped` 수에서 매번 계산한다 — 저장하면 카드 동작과 어긋날 두 번째 원천이 생긴다.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::{CardDraft, Outcome};

/// 설정 키. `"0"`이면 꺼짐, 그 밖(없음 포함)은 켜짐.
const ENABLED_KEY: &str = "know_cards:enabled";
/// 세션당 활성 카드 상한(설계 2.6). 넘치면 가장 오래된 활성 카드가 `evicted`.
const MAX_ACTIVE: i64 = 5;
/// 이미 앎·최근 주제 각각의 상한. 둘 다 전역이다.
const MAX_TOPICS: i64 = 50;

pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS know_cards (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           task_id INTEGER NOT NULL,
           kind TEXT NOT NULL,
           topic TEXT NOT NULL,
           title TEXT NOT NULL,
           body TEXT NOT NULL,
           evidence TEXT NOT NULL,
           turn_label TEXT NOT NULL,
           created_at INTEGER NOT NULL,
           seen_at INTEGER,
           asked_at INTEGER,
           action TEXT,
           action_at INTEGER
         )",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_know_cards_task ON know_cards(task_id, id)")
        .execute(pool)
        .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS know_topics (
           topic TEXT PRIMARY KEY,
           state TEXT NOT NULL,
           updated_at INTEGER NOT NULL
         )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS know_card_checks (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           task_id INTEGER NOT NULL,
           created_at INTEGER NOT NULL,
           outcome TEXT NOT NULL,
           duration_ms INTEGER
         )",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_know_card_checks_task ON know_card_checks(task_id, id)")
        .execute(pool)
        .await?;
    Ok(())
}

/// 채널에 보이는 활성 카드 한 장.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct KnowCard {
    pub id: i64,
    pub task_id: i64,
    /// `you_should_know` 또는 `heads_up`.
    pub kind: String,
    pub topic: String,
    pub title: String,
    pub body: String,
    pub evidence: String,
    pub turn_label: String,
    pub created_at: i64,
    pub seen_at: Option<i64>,
    pub asked_at: Option<i64>,
}

/// 카드 버튼 셋 — `자세히`는 카드를 남기고, 나머지 둘은 카드를 뺀다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardAction {
    Asked,
    Known,
    Dismissed,
}

pub async fn enabled(pool: &SqlitePool) -> bool {
    // 설정을 못 읽으면 판정하지 않는다 — 읽지도 못하는 상태에서 모델 호출 비용을 쓰지 않는다.
    matches!(crate::db::get_setting(pool, ENABLED_KEY).await, Ok(value) if value.as_deref() != Some("0"))
}

pub async fn set_enabled(pool: &SqlitePool, on: bool) -> anyhow::Result<()> {
    crate::db::set_setting(pool, ENABLED_KEY, if on { "1" } else { "0" }).await
}

/// 활성 카드, 최신 순.
pub async fn list_active(pool: &SqlitePool, task_id: i64) -> anyhow::Result<Vec<KnowCard>> {
    Ok(sqlx::query_as(
        "SELECT id, task_id, kind, topic, title, body, evidence, turn_label, created_at, seen_at, asked_at
           FROM know_cards WHERE task_id = ? AND action IS NULL ORDER BY id DESC",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await?)
}

/// 처음 본 시각만 남긴다 — 다시 봐도 덮지 않는다.
pub async fn mark_seen(pool: &SqlitePool, card_id: i64, now: i64) -> anyhow::Result<()> {
    sqlx::query("UPDATE know_cards SET seen_at = ? WHERE id = ? AND seen_at IS NULL")
        .bind(now)
        .bind(card_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 버튼 동작을 기록하고 카드가 속한 작업 id를 돌려준다(카드가 없으면 `None`).
///
/// 밀려난(`evicted`) 카드에도 동작을 받는다 — 화면이 늦게 갱신돼 밀려난 카드를 눌렀어도
/// 사용자의 뜻은 분명하다. `이미 앎`이면 카드 상태와 무관하게 주제를 기록한다.
///
/// 처음부터 쓰기 잠금을 잡는다. 읽기로 시작한 지연 트랜잭션은 쓰기로 올라갈 때 busy handler를
/// 거치지 않아, 판정이 카드를 쓰는 순간과 겹치면 기다리지 않고 실패한다.
pub async fn act(pool: &SqlitePool, card_id: i64, action: CardAction, now: i64) -> anyhow::Result<Option<i64>> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let row: Option<(i64, String)> = sqlx::query_as("SELECT task_id, topic FROM know_cards WHERE id = ?")
        .bind(card_id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((task_id, topic)) = row else {
        return Ok(None);
    };
    match action {
        CardAction::Asked => {
            sqlx::query("UPDATE know_cards SET asked_at = COALESCE(asked_at, ?) WHERE id = ?")
                .bind(now)
                .bind(card_id)
                .execute(&mut *tx)
                .await?;
        }
        CardAction::Known | CardAction::Dismissed => {
            let state = if action == CardAction::Known { "known" } else { "dismissed" };
            sqlx::query(
                "UPDATE know_cards SET action = ?, action_at = ?
                  WHERE id = ? AND (action IS NULL OR action = 'evicted')",
            )
            .bind(state)
            .bind(now)
            .bind(card_id)
            .execute(&mut *tx)
            .await?;
        }
    }
    if action == CardAction::Known {
        // 이미 앎이 최근을 덮는다 — 같은 주제가 두 상태로 남지 않는다.
        sqlx::query(
            "INSERT INTO know_topics (topic, state, updated_at) VALUES (?, 'known', ?)
             ON CONFLICT(topic) DO UPDATE SET state = 'known', updated_at = excluded.updated_at",
        )
        .bind(&topic)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        prune_topics(&mut tx, "known").await?;
    }
    tx.commit().await?;
    Ok(Some(task_id))
}

async fn prune_topics(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>, state: &str) -> anyhow::Result<()> {
    sqlx::query(
        "DELETE FROM know_topics WHERE state = ?1 AND topic NOT IN (
           SELECT topic FROM know_topics WHERE state = ?1 ORDER BY updated_at DESC, rowid DESC LIMIT ?2)",
    )
    .bind(state)
    .bind(MAX_TOPICS)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// 프롬프트의 제외 목록 — (이미 앎, 최근), 각각 최신 순 최대 50개.
pub(super) async fn topics(pool: &SqlitePool) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    let list = |state: &'static str| {
        sqlx::query_scalar::<_, String>(
            "SELECT topic FROM know_topics WHERE state = ? ORDER BY updated_at DESC, rowid DESC LIMIT ?",
        )
        .bind(state)
        .bind(MAX_TOPICS)
        .fetch_all(pool)
    };
    Ok((list("known").await?, list("recent").await?))
}

/// 카드 삽입, 5장 상한, 최근 주제 기록을 한 트랜잭션으로 한다. 판정 도중 작업이 지워졌으면
/// 아무것도 쓰지 않고 `false` — 지운 작업의 기록 인용이 카드로 되살아나지 않게 한다.
pub(super) async fn insert_card(
    pool: &SqlitePool,
    task_id: i64,
    draft: &CardDraft,
    turn_label: &str,
    now: i64,
) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await?;
    let inserted = sqlx::query(
        "INSERT INTO know_cards (task_id, kind, topic, title, body, evidence, turn_label, created_at)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8 WHERE EXISTS (SELECT 1 FROM tasks WHERE id = ?1)",
    )
    .bind(task_id)
    .bind(draft.kind)
    .bind(&draft.topic)
    .bind(&draft.title)
    .bind(&draft.body)
    .bind(&draft.evidence)
    .bind(turn_label)
    .bind(now)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Ok(false);
    }
    sqlx::query(
        "UPDATE know_cards SET action = 'evicted', action_at = ?1
          WHERE task_id = ?2 AND action IS NULL AND id NOT IN (
            SELECT id FROM know_cards WHERE task_id = ?2 AND action IS NULL ORDER BY id DESC LIMIT ?3)",
    )
    .bind(now)
    .bind(task_id)
    .bind(MAX_ACTIVE)
    .execute(&mut *tx)
    .await?;
    // 이미 앎인 주제는 최근으로 내리지 않는다 — 판정 도중 사용자가 이미 앎을 눌렀을 수 있다.
    sqlx::query(
        "INSERT INTO know_topics (topic, state, updated_at) VALUES (?, 'recent', ?)
         ON CONFLICT(topic) DO UPDATE SET updated_at = excluded.updated_at WHERE know_topics.state = 'recent'",
    )
    .bind(&draft.topic)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    prune_topics(&mut tx, "recent").await?;
    tx.commit().await?;
    Ok(true)
}

/// 판정 1회의 결과. 기록 실패는 판정을 되돌릴 이유가 아니라 삼킨다.
pub(super) async fn record_check(
    pool: &SqlitePool,
    task_id: i64,
    outcome: Outcome,
    duration_ms: Option<i64>,
    now: i64,
) {
    let _ = sqlx::query(
        "INSERT INTO know_card_checks (task_id, created_at, outcome, duration_ms) VALUES (?, ?, ?, ?)",
    )
    .bind(task_id)
    .bind(now)
    .bind(outcome.as_str())
    .bind(duration_ms)
    .execute(pool)
    .await;
}

/// 최신 카드부터 연속으로 무시된 카드 수. 무시는 판정 시점까지 `자세히`·`이미 앎`·`닫기`를
/// 하나도 받지 못한 카드다 — 밀려난 카드도 여기 든다(설계 2.6).
pub(super) async fn consecutive_ignored(pool: &SqlitePool, task_id: i64) -> usize {
    // `action IN (...)`은 action이 NULL이면 NULL이 된다 — COALESCE로 0/1만 남긴다.
    let acted: Vec<bool> = sqlx::query_scalar(
        "SELECT asked_at IS NOT NULL OR COALESCE(action, '') IN ('known', 'dismissed')
           FROM know_cards WHERE task_id = ? ORDER BY id DESC LIMIT 32",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    acted.iter().take_while(|acted| !**acted).count()
}

/// 판정 기록 끝쪽의 연속 `skipped` 수. `busy`는 판정을 부르지 않은 행이라 세지도 끊지도 않는다.
pub(super) async fn trailing_skips(pool: &SqlitePool, task_id: i64) -> usize {
    let outcomes: Vec<String> = sqlx::query_scalar(
        "SELECT outcome FROM know_card_checks WHERE task_id = ? ORDER BY id DESC LIMIT 64",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    outcomes
        .iter()
        .filter(|outcome| *outcome != Outcome::Busy.as_str())
        .take_while(|outcome| *outcome == Outcome::Skipped.as_str())
        .count()
}
