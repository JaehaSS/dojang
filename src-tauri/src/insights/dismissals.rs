//! 발견 카드 무시(설계 2026-09-28 §6.4·§8, T4).
//!
//! 카드 키는 롤링 윈도우 탓에 날마다 바뀐다 — 키로 무시하면 다음 날 같은 신호가
//! 다른 키로 돌아와 무시가 듣지 않는다. 그래서 `(signal, repo-or-empty)` 쌍을 7일
//! 동안만 가라앉힌다. 저장소에 묶이지 않는 신호(S1)는 `repo`가 빈 문자열이다.

use sqlx::SqlitePool;

/// 무시가 유효한 기간(초). 사용자 결정 — 7일.
pub const TTL_SECS: i64 = 7 * 86_400;

const MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS insight_dismissals (
  signal        TEXT NOT NULL,
  repo          TEXT NOT NULL DEFAULT '',
  dismissed_at  INTEGER NOT NULL,
  PRIMARY KEY (signal, repo)
);
"#;

pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(MIGRATION).execute(pool).await?;
    Ok(())
}

/// `(signal, repo)`를 지금 시각으로 무시 처리한다. 이미 있으면 시각만 갱신한다(재무시).
pub async fn dismiss(pool: &SqlitePool, signal: &str, repo: &str, now: i64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO insight_dismissals (signal, repo, dismissed_at) VALUES (?, ?, ?) \
         ON CONFLICT(signal, repo) DO UPDATE SET dismissed_at = excluded.dismissed_at",
    )
    .bind(signal)
    .bind(repo)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// `now - TTL_SECS` 안에 무시된 `(signal, repo)` 쌍 전부. `compute_cards`가 후보를
/// 걸러내기 전에 한 번만 불러 쓴다.
pub async fn active(pool: &SqlitePool, now: i64) -> anyhow::Result<Vec<(String, String)>> {
    let cutoff = now - TTL_SECS;
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT signal, repo FROM insight_dismissals WHERE dismissed_at > ?")
            .bind(cutoff)
            .fetch_all(pool)
            .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static DATABASE_COUNTER: AtomicU32 = AtomicU32::new(0);

    /// 임시 파일 DB인 이유는 `sqlite::memory:`가 풀의 커넥션마다 별개의 빈 DB를 보기 때문이다.
    async fn test_pool() -> SqlitePool {
        let sequence = DATABASE_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = crate::testtmp::dir().join(format!(
            "praxis-dismissals-{}-{sequence}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .unwrap();
        migrate(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn dismiss_then_active_returns_the_pair() {
        let pool = test_pool().await;
        dismiss(&pool, "S1", "", 1_000).await.unwrap();
        dismiss(&pool, "S2", "/repo/a", 1_000).await.unwrap();

        let active = active(&pool, 1_000).await.unwrap();
        assert_eq!(active.len(), 2);
        assert!(active.contains(&("S1".to_string(), "".to_string())));
        assert!(active.contains(&("S2".to_string(), "/repo/a".to_string())));
    }

    #[tokio::test]
    async fn active_excludes_pairs_past_the_ttl() {
        let pool = test_pool().await;
        dismiss(&pool, "S1", "", 1_000).await.unwrap();

        // TTL 안(경계 직전) — 여전히 활성.
        let still_active = active(&pool, 1_000 + TTL_SECS - 1).await.unwrap();
        assert_eq!(still_active, vec![("S1".to_string(), "".to_string())]);

        // TTL을 꽉 채우면(7일 지남) 사라진다 — 같은 신호가 다시 보여야 한다.
        let expired = active(&pool, 1_000 + TTL_SECS).await.unwrap();
        assert!(expired.is_empty());
    }

    #[tokio::test]
    async fn dismissal_is_keyed_by_signal_and_repo_together() {
        let pool = test_pool().await;
        dismiss(&pool, "S2", "/repo/a", 1_000).await.unwrap();

        // 다른 저장소의 같은 신호는 여전히 활성이 아니다 — 쌍으로 구분된다.
        let active = active(&pool, 1_000).await.unwrap();
        assert!(!active.contains(&("S2".to_string(), "/repo/b".to_string())));
        assert!(active.contains(&("S2".to_string(), "/repo/a".to_string())));
    }

    #[tokio::test]
    async fn dismissing_twice_refreshes_the_timestamp() {
        let pool = test_pool().await;
        dismiss(&pool, "S1", "", 1_000).await.unwrap();
        dismiss(&pool, "S1", "", 5_000).await.unwrap();

        // 첫 시각 기준으로는 만료됐어야 하지만, 재무시가 시각을 갱신했으므로 여전히 활성이다.
        let active = active(&pool, 5_000 + TTL_SECS - 1).await.unwrap();
        assert_eq!(active, vec![("S1".to_string(), "".to_string())]);
    }
}
