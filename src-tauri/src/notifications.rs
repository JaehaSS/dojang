use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, SqlitePool, Transaction};

const SOURCE_ID_SETTING: &str = "notification_source_id";
const ENABLED_SETTING: &str = "notification_enabled";
const DELIVERY_ERROR_SETTING: &str = "notification_delivery_error";
const PAGE_LIMIT: usize = 100;
const KINDS: [&str; 3] = ["result", "question", "failure"];

#[derive(Clone, Debug, Deserialize, Serialize, sqlx::FromRow)]
pub struct ResultNotice {
    pub sequence: i64,
    pub task_id: i64,
    pub ts: i64,
    pub kind: String,
    pub title: String,
    pub repo: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SourcePage {
    pub source_id: String,
    pub after: Option<i64>,
    pub cursor: i64,
    pub watermark: i64,
    pub results: Vec<ResultNotice>,
}

#[derive(Clone, Debug, Deserialize, Serialize, sqlx::FromRow)]
pub struct InboxItem {
    pub host: String,
    pub source_id: String,
    pub sequence: i64,
    pub task_id: i64,
    pub ts: i64,
    pub kind: String,
    pub title: String,
    pub repo: String,
    pub read_sequence: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, sqlx::FromRow)]
pub struct SourceCursor {
    pub host: String,
    pub source_id: String,
    pub cursor: i64,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub items: Vec<InboxItem>,
    pub sources: Vec<SourceCursor>,
    pub enabled: bool,
    pub delivery_error: Option<String>,
}

pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    for statement in [
        "CREATE TABLE IF NOT EXISTS notification_results (sequence INTEGER PRIMARY KEY AUTOINCREMENT, task_id INTEGER NOT NULL, ts INTEGER NOT NULL, kind TEXT NOT NULL, title TEXT NOT NULL, repo TEXT NOT NULL)",
        "CREATE TABLE IF NOT EXISTS notification_sources (host TEXT PRIMARY KEY, source_id TEXT NOT NULL, cursor INTEGER NOT NULL, warning TEXT)",
        "CREATE TABLE IF NOT EXISTS notification_inbox (host TEXT NOT NULL, source_id TEXT NOT NULL, task_id INTEGER NOT NULL, sequence INTEGER NOT NULL, read_sequence INTEGER NOT NULL DEFAULT 0, ts INTEGER NOT NULL, kind TEXT NOT NULL, title TEXT NOT NULL, repo TEXT NOT NULL, PRIMARY KEY(host, source_id, task_id))",
        "CREATE TABLE IF NOT EXISTS notification_cancels (task_id INTEGER PRIMARY KEY)",
    ] {
        sqlx::query(statement).execute(pool).await?;
    }
    Ok(())
}

pub async fn source_id(pool: &SqlitePool) -> anyhow::Result<String> {
    if let Some(value) = crate::db::get_setting(pool, SOURCE_ID_SETTING).await? {
        return Ok(value);
    }
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes)?;
    let value: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    sqlx::query("INSERT INTO settings(key, value) VALUES (?, ?) ON CONFLICT(key) DO NOTHING")
        .bind(SOURCE_ID_SETTING)
        .bind(value)
        .execute(pool)
        .await?;
    sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(SOURCE_ID_SETTING)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

pub async fn source_page(pool: &SqlitePool, after: Option<i64>) -> anyhow::Result<SourcePage> {
    let mut tx = pool.begin().await?;
    let source_id = source_id_tx(&mut tx).await?;
    let watermark: i64 = sqlx::query_scalar(
        "SELECT COALESCE(seq, 0) FROM sqlite_sequence WHERE name = 'notification_results'",
    )
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(0);
    let results = match after {
        None => Vec::new(),
        Some(after) => sqlx::query_as::<_, ResultNotice>(
            "SELECT sequence, task_id, ts, kind, title, repo FROM notification_results WHERE sequence > ? AND sequence <= ? ORDER BY sequence ASC LIMIT 100",
        )
        .bind(after)
        .bind(watermark)
        .fetch_all(&mut *tx)
        .await?,
    };
    let cursor = if results.len() < PAGE_LIMIT {
        watermark
    } else {
        results
            .last()
            .map(|item| item.sequence)
            .unwrap_or(watermark)
    };
    tx.commit().await?;
    Ok(SourcePage {
        source_id,
        after,
        cursor,
        watermark,
        results,
    })
}

pub async fn snapshot(pool: &SqlitePool) -> anyhow::Result<Snapshot> {
    let items = sqlx::query_as::<_, InboxItem>(
        "SELECT host, source_id, sequence, task_id, ts, kind, title, repo, read_sequence FROM notification_inbox WHERE sequence > read_sequence ORDER BY ts DESC, sequence DESC",
    )
    .fetch_all(pool)
    .await?;
    let sources = sqlx::query_as::<_, SourceCursor>(
        "SELECT host, source_id, cursor, warning FROM notification_sources ORDER BY host",
    )
    .fetch_all(pool)
    .await?;
    let enabled = crate::db::get_setting(pool, ENABLED_SETTING)
        .await?
        .as_deref()
        != Some("false");
    let delivery_error = crate::db::get_setting(pool, DELIVERY_ERROR_SETTING).await?;
    Ok(Snapshot {
        items,
        sources,
        enabled,
        delivery_error,
    })
}

pub async fn ingest(
    pool: &SqlitePool,
    host: &str,
    page: &SourcePage,
) -> anyhow::Result<Vec<ResultNotice>> {
    validate_page(page)?;
    let mut tx = pool.begin().await?;
    let source: Option<SourceCursor> = sqlx::query_as(
        "SELECT host, source_id, cursor, warning FROM notification_sources WHERE host = ?",
    )
    .bind(host)
    .fetch_optional(&mut *tx)
    .await?;
    let inserted = match source {
        None => {
            baseline(&mut tx, host, page, None).await?;
            Vec::new()
        }
        Some(source) if source.source_id != page.source_id => {
            baseline(&mut tx, host, page, Some(source.source_id)).await?;
            Vec::new()
        }
        Some(source) if page.after != Some(source.cursor) => {
            anyhow::bail!("notification page cursor does not match source cursor")
        }
        Some(_) => ingest_page(&mut tx, host, page).await?,
    };
    tx.commit().await?;
    Ok(inserted)
}

pub async fn acknowledge(
    pool: &SqlitePool,
    host: &str,
    source_id: &str,
    task_id: i64,
    through: i64,
) -> anyhow::Result<()> {
    if through < 0 {
        anyhow::bail!("notification acknowledgement sequence must be non-negative");
    }
    let changed = sqlx::query(
        "UPDATE notification_inbox SET read_sequence = MAX(read_sequence, ?) WHERE host = ? AND source_id = ? AND task_id = ? AND ? <= sequence",
    )
    .bind(through).bind(host).bind(source_id).bind(task_id).bind(through).execute(pool).await?;
    if changed.rows_affected() == 0 {
        anyhow::bail!("notification acknowledgement is not valid for the current result");
    }
    Ok(())
}

pub async fn reconcile(pool: &SqlitePool, host: &str, task_ids: &[i64]) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "CREATE TEMP TABLE IF NOT EXISTS notification_known_tasks (task_id INTEGER PRIMARY KEY)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM notification_known_tasks")
        .execute(&mut *tx)
        .await?;
    for task_id in task_ids {
        sqlx::query("INSERT INTO notification_known_tasks(task_id) VALUES (?)")
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("DELETE FROM notification_inbox WHERE host = ? AND task_id NOT IN (SELECT task_id FROM notification_known_tasks)").bind(host).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn set_enabled(pool: &SqlitePool, enabled: bool) -> anyhow::Result<()> {
    crate::db::set_setting(
        pool,
        ENABLED_SETTING,
        if enabled { "true" } else { "false" },
    )
    .await?;
    crate::db::delete_setting(pool, DELIVERY_ERROR_SETTING).await
}

pub async fn set_delivery_error(pool: &SqlitePool, error: Option<&str>) -> anyhow::Result<()> {
    match error {
        Some(error) => crate::db::set_setting(pool, DELIVERY_ERROR_SETTING, error).await,
        None => crate::db::delete_setting(pool, DELIVERY_ERROR_SETTING).await,
    }
}

pub(crate) async fn record_result_tx(
    tx: &mut Transaction<'_, Sqlite>,
    task_id: i64,
    ts: i64,
    kind: &str,
) -> anyhow::Result<()> {
    if !KINDS.contains(&kind) {
        anyhow::bail!("invalid notification result kind");
    }
    let cancelled: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM notification_cancels WHERE task_id = ?)")
            .bind(task_id)
            .fetch_one(&mut **tx)
            .await?;
    if cancelled {
        return Ok(());
    }
    sqlx::query("INSERT INTO notification_results(task_id, ts, kind, title, repo) SELECT id, ?, ?, instruction, repo FROM tasks WHERE id = ?")
        .bind(ts).bind(kind).bind(task_id).execute(&mut **tx).await?;
    Ok(())
}

/// 구조화 질문이 열릴 때 부른다. 질문은 상태 전이가 없어 결과 알림 경로
/// (`finish_running_task_with_notification`)를 타지 않는다 — 같은 원천(`notification_results`)에
/// 실어야 인박스와 OS 알림이 같은 수집기로 전달된다. 취소 의도가 먼저 남았으면 결과 알림과 같은
/// 규칙으로 싣지 않는다.
pub async fn record_question(pool: &SqlitePool, task_id: i64, ts: i64) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    record_result_tx(&mut tx, task_id, ts, "question").await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn cancel_intent(pool: &SqlitePool, task_id: i64) -> anyhow::Result<bool> {
    let result = sqlx::query("INSERT INTO notification_cancels(task_id) SELECT id FROM tasks WHERE id = ? AND state IN ('Created', 'Running') ON CONFLICT(task_id) DO NOTHING")
        .bind(task_id).execute(pool).await?;
    Ok(result.rows_affected() > 0)
}

pub(crate) async fn clear_cancel_tx(
    tx: &mut Transaction<'_, Sqlite>,
    task_id: i64,
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM notification_cancels WHERE task_id = ?")
        .bind(task_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(crate) async fn clear_cancel_if_signal_not_sent(
    pool: &SqlitePool,
    task_id: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "DELETE FROM notification_cancels WHERE task_id = ? AND EXISTS (SELECT 1 FROM tasks WHERE id = ? AND state IN ('Created', 'Running'))",
    )
    .bind(task_id)
    .bind(task_id)
    .execute(pool)
    .await?;
    Ok(())
}

fn validate_page(page: &SourcePage) -> anyhow::Result<()> {
    if page.source_id.is_empty()
        || page.cursor < 0
        || page.watermark < page.cursor
        || page.results.len() > PAGE_LIMIT
        || page
            .after
            .is_some_and(|after| after < 0 || page.cursor < after)
    {
        anyhow::bail!("invalid notification source page");
    }
    if page.after.is_none() && !page.results.is_empty() {
        anyhow::bail!("baseline page must not include results");
    }
    let mut previous = page.after.unwrap_or(0);
    for item in &page.results {
        if item.sequence <= previous
            || item.sequence > page.cursor
            || !KINDS.contains(&item.kind.as_str())
        {
            anyhow::bail!("invalid notification source page results");
        }
        previous = item.sequence;
    }
    if !page.results.is_empty() && page.cursor < previous {
        anyhow::bail!("notification page cursor does not cover results");
    }
    Ok(())
}

async fn source_id_tx(tx: &mut Transaction<'_, Sqlite>) -> anyhow::Result<String> {
    let current: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(SOURCE_ID_SETTING)
        .fetch_optional(&mut **tx)
        .await?;
    if let Some(value) = current {
        return Ok(value);
    }
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes)?;
    let value: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    sqlx::query("INSERT INTO settings(key, value) VALUES (?, ?) ON CONFLICT(key) DO NOTHING")
        .bind(SOURCE_ID_SETTING)
        .bind(&value)
        .execute(&mut **tx)
        .await?;
    sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(SOURCE_ID_SETTING)
        .fetch_one(&mut **tx)
        .await
        .map_err(Into::into)
}

async fn baseline(
    tx: &mut Transaction<'_, Sqlite>,
    host: &str,
    page: &SourcePage,
    old_source: Option<String>,
) -> anyhow::Result<()> {
    if page.after.is_some() {
        anyhow::bail!("new notification source requires a baseline page");
    }
    if old_source.is_some() {
        sqlx::query("DELETE FROM notification_inbox WHERE host = ?")
            .bind(host)
            .execute(&mut **tx)
            .await?;
    }
    let warning =
        old_source.map(|_| "원격 기록이 초기화되어 이전 미확인 결과를 복구할 수 없음".to_string());
    sqlx::query("INSERT INTO notification_sources(host, source_id, cursor, warning) VALUES (?, ?, ?, ?) ON CONFLICT(host) DO UPDATE SET source_id=excluded.source_id,cursor=excluded.cursor,warning=excluded.warning")
        .bind(host).bind(&page.source_id).bind(page.watermark).bind(warning).execute(&mut **tx).await?;
    Ok(())
}

async fn ingest_page(
    tx: &mut Transaction<'_, Sqlite>,
    host: &str,
    page: &SourcePage,
) -> anyhow::Result<Vec<ResultNotice>> {
    // 앱 안 알림 목록("확인" 버튼)은 2026-09-26 제거됐다. 읽음 표시를 전진시킬 손이 없으므로
    // 수집 시점에 읽음(read_sequence = sequence)으로 넣는다 — 새로 들어온 항목은 `inserted`로
    // 돌려보내 OS 푸시는 그대로 나가고, 미확인 목록(snapshot)은 비어 있게 된다.
    let mut inserted = Vec::new();
    for item in &page.results {
        let changed = sqlx::query("INSERT INTO notification_inbox(host, source_id, task_id, sequence, read_sequence, ts, kind, title, repo) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(host, source_id, task_id) DO UPDATE SET sequence=excluded.sequence,read_sequence=excluded.sequence,ts=excluded.ts,kind=excluded.kind,title=excluded.title,repo=excluded.repo WHERE excluded.sequence > notification_inbox.sequence")
            .bind(host).bind(&page.source_id).bind(item.task_id).bind(item.sequence).bind(item.sequence).bind(item.ts).bind(&item.kind).bind(&item.title).bind(&item.repo).execute(&mut **tx).await?;
        if changed.rows_affected() > 0 {
            inserted.push(item.clone());
        }
    }
    sqlx::query("UPDATE notification_sources SET cursor = ? WHERE host = ? AND source_id = ?")
        .bind(page.cursor)
        .bind(host)
        .bind(&page.source_id)
        .execute(&mut **tx)
        .await?;
    Ok(inserted)
}
