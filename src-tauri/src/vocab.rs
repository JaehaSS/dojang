//! 영어 표현 단어장 — 번역·⌘J가 짚어 준 표현을 저장하고 간격 반복으로 복습한다.
//!
//! 표현은 `translate::Direction::EnExpressions`가 고른다. 여기는 저장·복습 일정·"알아요"만 맡는다.
//! 대기 퀴즈(`quiz`)와 합치지 않은 이유: 퀴즈는 한 번 답한 문제를 다시 내지 않고 정답을 문자열
//! 일치로 채점한다. 뜻풀이는 문자열로 맞출 수 없고, 복습은 되풀이가 본질이다.
//!
//! 복습은 라이트너 상자다. 기억나면 한 칸 올라가 간격이 늘고, 잊으면 첫 칸으로 돌아간다.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

const DAY: i64 = 24 * 60 * 60;
/// 상자 `n`에 들어간 뒤 다음 복습까지의 간격. 마지막 상자에 머물면 같은 간격을 되풀이한다.
const INTERVALS: [i64; 6] = [DAY, 3 * DAY, 7 * DAY, 14 * DAY, 30 * DAY, 60 * DAY];
/// 표현 추출 프롬프트에 넘길 "아는 표현"의 최대 개수 — 프롬프트가 끝없이 자라지 않게 한다.
pub const KNOWN_LIMIT: i64 = 300;
const MAX_FIELD: usize = 2_000;

pub const MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS vocab_entries (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  phrase      TEXT NOT NULL,
  -- 소문자·공백 정규화. 같은 표현을 두 번 저장하지 않는다.
  phrase_key  TEXT NOT NULL UNIQUE,
  meaning     TEXT NOT NULL,
  note        TEXT,
  -- 표현이 나온 원문 문장. 뜻만 외우면 쓰임을 잊는다.
  example     TEXT,
  -- 'learning' | 'known'. known은 복습·하이라이트에서 빠진다.
  status      TEXT NOT NULL,
  box         INTEGER NOT NULL DEFAULT 0,
  due_at      INTEGER NOT NULL,
  created_at  INTEGER NOT NULL,
  reviewed_at INTEGER
);
CREATE INDEX IF NOT EXISTS vocab_due ON vocab_entries(status, due_at);
"#;

pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::raw_sql(MIGRATION).execute(pool).await?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Learning,
    Known,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Learning => "learning",
            Status::Known => "known",
        }
    }

    fn parse(raw: &str) -> Self {
        if raw == "known" { Status::Known } else { Status::Learning }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub id: i64,
    pub phrase: String,
    pub meaning: String,
    pub note: Option<String>,
    pub example: Option<String>,
    pub status: Status,
    pub box_level: i64,
    pub due_at: i64,
    pub created_at: i64,
    pub reviewed_at: Option<i64>,
}

/// 저장 요청 — 카드에 보이던 그대로 온다.
#[derive(Clone, Debug, Deserialize)]
pub struct NewEntry {
    pub phrase: String,
    pub meaning: String,
    pub note: Option<String>,
    pub example: Option<String>,
}

type Row = (i64, String, String, Option<String>, Option<String>, String, i64, i64, i64, Option<i64>);
const COLUMNS: &str = "id, phrase, meaning, note, example, status, box, due_at, created_at, reviewed_at";

fn entry((id, phrase, meaning, note, example, status, box_level, due_at, created_at, reviewed_at): Row) -> Entry {
    Entry { id, phrase, meaning, note, example, status: Status::parse(&status), box_level, due_at, created_at, reviewed_at }
}

/// 대소문자·공백만 다른 표현은 같은 표현이다.
pub fn phrase_key(phrase: &str) -> String {
    phrase.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn clean(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// 다음 복습 시각과 상자. 기억나면 한 칸 올리고, 잊으면 첫 칸에서 다시 시작한다.
pub fn schedule(box_level: i64, remembered: bool, now: i64) -> (i64, i64) {
    let last = INTERVALS.len() as i64 - 1;
    let next = if remembered { (box_level + 1).min(last) } else { 0 };
    (next, now + INTERVALS[next as usize])
}

async fn by_id(pool: &SqlitePool, id: i64) -> anyhow::Result<Option<Entry>> {
    let row: Option<Row> = sqlx::query_as(&format!("SELECT {COLUMNS} FROM vocab_entries WHERE id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(entry))
}

/// 저장한다. 이미 있는 표현이면 뜻·예문을 새것으로 바꾸고 복습 대상으로 되돌린다 — 모른다고
/// 다시 저장했다는 것은 "알아요"가 틀렸다는 뜻이다. 복습 진도(상자)는 그대로 둔다.
pub async fn save(pool: &SqlitePool, new: NewEntry, now: i64) -> anyhow::Result<Entry> {
    let phrase = new.phrase.trim().to_string();
    let meaning = new.meaning.trim().to_string();
    if phrase.is_empty() || meaning.is_empty() {
        anyhow::bail!("표현과 뜻이 모두 있어야 저장합니다");
    }
    if [&phrase, &meaning].iter().any(|v| v.chars().count() > MAX_FIELD) {
        anyhow::bail!("너무 깁니다");
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO vocab_entries (phrase, phrase_key, meaning, note, example, status, box, due_at, created_at) \
         VALUES (?, ?, ?, ?, ?, 'learning', 0, ?, ?) \
         ON CONFLICT(phrase_key) DO UPDATE SET \
           meaning = excluded.meaning, note = COALESCE(excluded.note, vocab_entries.note), \
           example = COALESCE(excluded.example, vocab_entries.example), \
           status = 'learning', \
           due_at = CASE WHEN vocab_entries.status = 'known' THEN excluded.due_at ELSE vocab_entries.due_at END \
         RETURNING id",
    )
    .bind(&phrase)
    .bind(phrase_key(&phrase))
    .bind(&meaning)
    .bind(clean(new.note))
    .bind(clean(new.example))
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;
    by_id(pool, id).await?.ok_or_else(|| anyhow::anyhow!("저장한 항목을 찾지 못했습니다"))
}

/// "알아요" — 저장하지 않은 표현도 기록해 다음부터 짚지 않는다. 뜻이 없으면 빈 뜻으로 둔다.
pub async fn mark_known(pool: &SqlitePool, phrase: &str, meaning: &str, now: i64) -> anyhow::Result<()> {
    let phrase = phrase.trim();
    if phrase.is_empty() {
        anyhow::bail!("표현이 비어 있습니다");
    }
    sqlx::query(
        "INSERT INTO vocab_entries (phrase, phrase_key, meaning, status, box, due_at, created_at) \
         VALUES (?, ?, ?, 'known', 0, ?, ?) \
         ON CONFLICT(phrase_key) DO UPDATE SET status = 'known'",
    )
    .bind(phrase)
    .bind(phrase_key(phrase))
    .bind(meaning.trim())
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_status(pool: &SqlitePool, id: i64, status: Status, now: i64) -> anyhow::Result<()> {
    // 복습으로 되돌리면 지금부터 다시 센다 — 몇 달 전 due_at이 남아 있으면 한꺼번에 몰려 나온다.
    sqlx::query(
        "UPDATE vocab_entries SET status = ?, due_at = CASE WHEN ? = 'learning' THEN ? ELSE due_at END WHERE id = ?",
    )
    .bind(status.as_str())
    .bind(status.as_str())
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete(pool: &SqlitePool, id: i64) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM vocab_entries WHERE id = ?").bind(id).execute(pool).await?;
    Ok(())
}

/// 최근 저장한 것부터 전부.
pub async fn list(pool: &SqlitePool) -> anyhow::Result<Vec<Entry>> {
    let rows: Vec<Row> = sqlx::query_as(&format!("SELECT {COLUMNS} FROM vocab_entries ORDER BY created_at DESC, id DESC"))
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(entry).collect())
}

/// 지금 복습할 것 — 오래 밀린 것부터.
pub async fn due(pool: &SqlitePool, now: i64, limit: i64) -> anyhow::Result<Vec<Entry>> {
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM vocab_entries WHERE status = 'learning' AND due_at <= ? ORDER BY due_at, id LIMIT ?"
    ))
    .bind(now)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(entry).collect())
}

pub async fn due_count(pool: &SqlitePool, now: i64) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar("SELECT COUNT(*) FROM vocab_entries WHERE status = 'learning' AND due_at <= ?")
        .bind(now)
        .fetch_one(pool)
        .await?)
}

/// 복습 결과를 반영한다. 없는 항목이면 None.
pub async fn review(pool: &SqlitePool, id: i64, remembered: bool, now: i64) -> anyhow::Result<Option<Entry>> {
    let Some(current) = by_id(pool, id).await? else {
        return Ok(None);
    };
    let (box_level, due_at) = schedule(current.box_level, remembered, now);
    sqlx::query("UPDATE vocab_entries SET box = ?, due_at = ?, reviewed_at = ? WHERE id = ?")
        .bind(box_level)
        .bind(due_at)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    by_id(pool, id).await
}

/// 이미 아는 표현 — 추출 프롬프트가 다시 고르지 않게 한다. 최근 것부터 상한까지.
pub async fn known_phrases(pool: &SqlitePool) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar("SELECT phrase FROM vocab_entries WHERE status = 'known' ORDER BY id DESC LIMIT ?")
        .bind(KNOWN_LIMIT)
        .fetch_all(pool)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static DB_COUNTER: AtomicUsize = AtomicUsize::new(0);

    async fn pool() -> SqlitePool {
        let n = DB_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = crate::testtmp::dir().join(format!("praxis-vocab-{}-{n}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let pool = crate::db::init_pool(path.to_str().unwrap()).await.unwrap();
        migrate(&pool).await.unwrap();
        migrate(&pool).await.unwrap();
        pool
    }

    fn new(phrase: &str, meaning: &str) -> NewEntry {
        NewEntry { phrase: phrase.into(), meaning: meaning.into(), note: None, example: Some("We ruled it out.".into()) }
    }

    #[test]
    fn remembering_climbs_boxes_and_forgetting_resets() {
        assert_eq!(schedule(0, true, 0), (1, 3 * DAY));
        assert_eq!(schedule(5, true, 10), (5, 10 + 60 * DAY));
        assert_eq!(schedule(4, false, 10), (0, 10 + DAY));
    }

    #[test]
    fn phrase_key_ignores_case_and_spacing() {
        assert_eq!(phrase_key("  Rule   Out "), "rule out");
    }

    #[tokio::test]
    async fn saved_entry_is_due_now_and_review_pushes_it_out() {
        let pool = pool().await;
        let saved = save(&pool, new("rule out", "배제하다"), 100).await.unwrap();
        assert_eq!(saved.status, Status::Learning);
        assert_eq!(due(&pool, 100, 10).await.unwrap().len(), 1);

        let reviewed = review(&pool, saved.id, true, 200).await.unwrap().unwrap();
        assert_eq!((reviewed.box_level, reviewed.due_at), (1, 200 + 3 * DAY));
        assert_eq!(due_count(&pool, 200).await.unwrap(), 0);
        assert!(review(&pool, 9999, true, 200).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn saving_the_same_phrase_updates_instead_of_duplicating() {
        let pool = pool().await;
        let first = save(&pool, new("Rule out", "배제하다"), 100).await.unwrap();
        review(&pool, first.id, true, 150).await.unwrap();
        let again = save(&pool, new("rule  out", "제외하다"), 200).await.unwrap();
        assert_eq!(again.id, first.id);
        assert_eq!(again.meaning, "제외하다");
        // 복습 진도는 지키고, 예문이 없으면 전 예문을 남긴다.
        assert_eq!(again.box_level, 1);
        assert_eq!(again.example.as_deref(), Some("We ruled it out."));
        assert_eq!(list(&pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn known_phrases_leave_review_and_resaving_brings_them_back() {
        let pool = pool().await;
        mark_known(&pool, "carry forward", "", 100).await.unwrap();
        assert_eq!(known_phrases(&pool).await.unwrap(), vec!["carry forward".to_string()]);
        assert_eq!(due_count(&pool, 100).await.unwrap(), 0);

        let saved = save(&pool, new("Carry forward", "이어 가다"), 500).await.unwrap();
        assert_eq!(saved.status, Status::Learning);
        assert_eq!(saved.due_at, 500);
        assert!(known_phrases(&pool).await.unwrap().is_empty());

        set_status(&pool, saved.id, Status::Known, 600).await.unwrap();
        assert_eq!(due_count(&pool, 9_999_999).await.unwrap(), 0);
        set_status(&pool, saved.id, Status::Learning, 700).await.unwrap();
        assert_eq!(due(&pool, 700, 10).await.unwrap()[0].due_at, 700);

        delete(&pool, saved.id).await.unwrap();
        assert!(list(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn empty_phrase_or_meaning_is_refused() {
        let pool = pool().await;
        assert!(save(&pool, new(" ", "뜻"), 1).await.is_err());
        assert!(save(&pool, new("x", " "), 1).await.is_err());
        assert!(mark_known(&pool, "  ", "", 1).await.is_err());
    }
}
