use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{author, status, ReviewAnnotation};

const MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS review_annotations (
  id         TEXT PRIMARY KEY,
  task_id    INTEGER NOT NULL,
  hunk_id    TEXT NOT NULL,
  path       TEXT NOT NULL,
  line       INTEGER NOT NULL,
  side       TEXT NOT NULL,
  body_md    TEXT NOT NULL,
  status     TEXT NOT NULL DEFAULT 'draft' CHECK(status IN ('draft','sent','resolved')),
  created_at INTEGER NOT NULL,
  author     TEXT NOT NULL DEFAULT 'human'
);
"#;

const COLUMNS: &str = "id, task_id, hunk_id, path, line, side, body_md, status, created_at, author";

pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(MIGRATION).execute(pool).await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_review_annotations_task ON review_annotations(task_id)",
    )
    .execute(pool)
    .await?;
    // 기존 DB에는 author 열이 없다. 사람이 쓴 주석만 있던 시절이므로 기본값 human이 곧 사실이다.
    if !has_column(pool, "author").await? {
        sqlx::query("ALTER TABLE review_annotations ADD COLUMN author TEXT NOT NULL DEFAULT 'human'")
            .execute(pool)
            .await?;
    }
    Ok(())
}

async fn has_column(pool: &SqlitePool, column: &str) -> anyhow::Result<bool> {
    let names: Vec<(String,)> =
        sqlx::query_as("SELECT name FROM pragma_table_info('review_annotations')")
            .fetch_all(pool)
            .await?;
    Ok(names.iter().any(|(name,)| name == column))
}

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn generate_id(task_id: i64, path: &str, line: i64, created_at: i64) -> String {
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let source = format!("{task_id}|{path}|{line}|{created_at}|{sequence}");
    let digest = Sha256::digest(source.as_bytes());
    let hex: String = digest
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("ann-{hex}")
}

/// 사람이 쓰는 초안. 재전송 버튼이 모아 에이전트에게 보내는 쪽이다.
#[allow(clippy::too_many_arguments)]
pub async fn create_draft(
    pool: &SqlitePool,
    task_id: i64,
    hunk_id: &str,
    path: &str,
    line: i64,
    side: &str,
    body_md: &str,
    created_at: i64,
) -> anyhow::Result<ReviewAnnotation> {
    insert(
        pool,
        NewAnnotation {
            task_id,
            hunk_id,
            path,
            line,
            side,
            body_md,
            created_at,
            author: author::HUMAN,
            status: status::DRAFT,
        },
    )
    .await
}

/// 에이전트가 MCP로 남기는 노트. 화면에 보이는 순간 전달이 끝난 것이므로 `sent`로 태어난다 —
/// `draft`로 두면 재전송 묶음에 섞여 에이전트가 쓴 글이 에이전트에게 되돌아간다.
#[allow(clippy::too_many_arguments)]
pub async fn create_agent_note(
    pool: &SqlitePool,
    task_id: i64,
    hunk_id: &str,
    path: &str,
    line: i64,
    side: &str,
    body_md: &str,
    author: &str,
    created_at: i64,
) -> anyhow::Result<ReviewAnnotation> {
    insert(
        pool,
        NewAnnotation {
            task_id,
            hunk_id,
            path,
            line,
            side,
            body_md,
            created_at,
            author,
            status: status::SENT,
        },
    )
    .await
}

struct NewAnnotation<'a> {
    task_id: i64,
    hunk_id: &'a str,
    path: &'a str,
    line: i64,
    side: &'a str,
    body_md: &'a str,
    created_at: i64,
    author: &'a str,
    status: &'a str,
}

async fn insert(pool: &SqlitePool, new: NewAnnotation<'_>) -> anyhow::Result<ReviewAnnotation> {
    let id = generate_id(new.task_id, new.path, new.line, new.created_at);
    sqlx::query(&format!(
        "INSERT INTO review_annotations ({COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    ))
    .bind(&id)
    .bind(new.task_id)
    .bind(new.hunk_id)
    .bind(new.path)
    .bind(new.line)
    .bind(new.side)
    .bind(new.body_md)
    .bind(new.status)
    .bind(new.created_at)
    .bind(new.author)
    .execute(pool)
    .await?;
    Ok(ReviewAnnotation {
        id,
        task_id: new.task_id,
        hunk_id: new.hunk_id.to_string(),
        path: new.path.to_string(),
        line: new.line,
        side: new.side.to_string(),
        body_md: new.body_md.to_string(),
        status: new.status.to_string(),
        created_at: new.created_at,
        author: new.author.to_string(),
    })
}

pub async fn update_draft_body(pool: &SqlitePool, id: &str, body_md: &str) -> anyhow::Result<bool> {
    let result =
        sqlx::query("UPDATE review_annotations SET body_md = ? WHERE id = ? AND status = 'draft'")
            .bind(body_md)
            .bind(id)
            .execute(pool)
            .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn list_by_task(
    pool: &SqlitePool,
    task_id: i64,
) -> anyhow::Result<Vec<ReviewAnnotation>> {
    Ok(sqlx::query_as::<_, ReviewAnnotation>(&format!(
        "SELECT {COLUMNS} FROM review_annotations WHERE task_id = ? ORDER BY created_at ASC"
    ))
    .bind(task_id)
    .fetch_all(pool)
    .await?)
}

pub async fn list_by_ids(
    pool: &SqlitePool,
    task_id: i64,
    ids: &[String],
) -> anyhow::Result<Vec<ReviewAnnotation>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT {COLUMNS} FROM review_annotations \
         WHERE task_id = ? AND id IN ({placeholders}) ORDER BY created_at ASC",
    );
    let mut query = sqlx::query_as::<_, ReviewAnnotation>(&sql).bind(task_id);
    for id in ids {
        query = query.bind(id);
    }
    Ok(query.fetch_all(pool).await?)
}

pub async fn mark_sent(pool: &SqlitePool, task_id: i64, ids: &[String]) -> anyhow::Result<u64> {
    set_status_from(pool, task_id, ids, status::DRAFT, status::SENT).await
}

pub async fn mark_draft(pool: &SqlitePool, task_id: i64, ids: &[String]) -> anyhow::Result<u64> {
    set_status_from(pool, task_id, ids, status::SENT, status::DRAFT).await
}

async fn set_status_from(
    pool: &SqlitePool,
    task_id: i64,
    ids: &[String],
    from: &str,
    to: &str,
) -> anyhow::Result<u64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let mut transaction = pool.begin().await?;
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "UPDATE review_annotations SET status = ? \
         WHERE task_id = ? AND status = ? AND id IN ({placeholders})",
    );
    let mut query = sqlx::query(&sql).bind(to).bind(task_id).bind(from);
    for id in ids {
        query = query.bind(id);
    }
    let result = query.execute(&mut *transaction).await?;
    transaction.commit().await?;
    Ok(result.rows_affected())
}
