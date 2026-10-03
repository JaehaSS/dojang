use crate::{db, notifications};

async fn pool(name: &str) -> sqlx::SqlitePool {
    let path = crate::testtmp::dir().join(format!("notifications-{name}.sqlite"));
    db::init_pool(path.to_str().unwrap()).await.unwrap()
}

fn page(
    source_id: &str,
    after: Option<i64>,
    cursor: i64,
    watermark: i64,
    results: Vec<notifications::ResultNotice>,
) -> notifications::SourcePage {
    notifications::SourcePage {
        source_id: source_id.into(),
        after,
        cursor,
        watermark,
        results,
    }
}

fn result(sequence: i64, task_id: i64) -> notifications::ResultNotice {
    notifications::ResultNotice {
        sequence,
        task_id,
        ts: sequence,
        kind: "result".into(),
        title: format!("task {task_id}"),
        repo: "/tmp/repo".into(),
    }
}

#[tokio::test]
async fn notification_source_page_keeps_highwater_after_deleted_rows() {
    let pool = pool("source-page").await;
    for sequence in 1..=3 {
        sqlx::query("INSERT INTO notification_results(task_id, ts, kind, title, repo) VALUES (?, ?, 'result', 'title', 'repo')")
            .bind(sequence).bind(sequence).execute(&pool).await.unwrap();
    }
    sqlx::query("DELETE FROM notification_results WHERE sequence = 3")
        .execute(&pool)
        .await
        .unwrap();
    let page = notifications::source_page(&pool, Some(0)).await.unwrap();
    assert_eq!(page.watermark, 3);
    assert_eq!(page.cursor, 3);
    assert_eq!(
        page.results
            .iter()
            .map(|item| item.sequence)
            .collect::<Vec<_>>(),
        [1, 2]
    );
}

#[tokio::test]
async fn notification_source_page_pages_more_than_one_hundred_without_a_gap() {
    let pool = pool("paging").await;
    for sequence in 1..=101 {
        sqlx::query("INSERT INTO notification_results(task_id, ts, kind, title, repo) VALUES (?, ?, 'result', 'title', 'repo')")
            .bind(sequence).bind(sequence).execute(&pool).await.unwrap();
    }
    let first = notifications::source_page(&pool, Some(0)).await.unwrap();
    assert_eq!(first.results.len(), 100);
    assert_eq!(first.cursor, 100);
    let second = notifications::source_page(&pool, Some(first.cursor))
        .await
        .unwrap();
    assert_eq!(
        second
            .results
            .iter()
            .map(|item| item.sequence)
            .collect::<Vec<_>>(),
        [101]
    );
    assert_eq!(second.cursor, 101);
}

#[tokio::test]
async fn notification_ingest_deduplicates_acknowledges_and_retires_reset_source() {
    let pool = pool("inbox").await;
    let baseline = page("source-a", None, 0, 0, vec![]);
    assert!(notifications::ingest(&pool, "local", &baseline)
        .await
        .unwrap()
        .is_empty());
    let first = page("source-a", Some(0), 1, 1, vec![result(1, 7)]);
    assert_eq!(
        notifications::ingest(&pool, "local", &first)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(notifications::ingest(&pool, "local", &first).await.is_err());
    let second = page("source-a", Some(1), 2, 2, vec![result(2, 7)]);
    notifications::ingest(&pool, "local", &second)
        .await
        .unwrap();
    // 앱 안 목록이 없어 수집 시점에 읽음으로 넣는다 — 미확인 목록은 비고, 행은 최신 sequence를 읽음으로 갖는다.
    let snapshot = notifications::snapshot(&pool).await.unwrap();
    assert!(snapshot.items.is_empty());
    let (sequence, read_sequence): (i64, i64) =
        sqlx::query_as("SELECT sequence, read_sequence FROM notification_inbox WHERE host = 'local' AND task_id = 7")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((sequence, read_sequence), (2, 2));
    // OS 알림 클릭 경로의 확인은 여전히 유효하다(행이 있고 through <= sequence).
    notifications::acknowledge(&pool, "local", "source-a", 7, 1)
        .await
        .unwrap();
    let reset = page("source-b", None, 4, 4, vec![]);
    notifications::ingest(&pool, "local", &reset).await.unwrap();
    let snapshot = notifications::snapshot(&pool).await.unwrap();
    assert!(snapshot.items.is_empty());
    assert!(snapshot.sources[0].warning.is_some());
}

#[tokio::test]
async fn notification_hosts_and_acknowledgements_are_isolated() {
    let pool = pool("hosts").await;
    for host in ["local", "remote"] {
        notifications::ingest(&pool, host, &page("source-a", None, 0, 0, vec![]))
            .await
            .unwrap();
        notifications::ingest(
            &pool,
            host,
            &page("source-a", Some(0), 1, 1, vec![result(1, 7)]),
        )
        .await
        .unwrap();
    }
    assert!(notifications::acknowledge(&pool, "local", "source-b", 7, 1)
        .await
        .is_err());
    assert!(notifications::acknowledge(&pool, "local", "source-a", 7, 2)
        .await
        .is_err());
    notifications::reconcile(&pool, "local", &[]).await.unwrap();
    // 수집 시 자동 읽음이라 미확인 목록은 비어 있다. local의 reconcile(아는 작업 없음)은 local 행만
    // 지우고 remote 행은 읽음 상태 그대로 남는다 — 호스트 격리는 행 단위로 확인한다.
    assert!(notifications::snapshot(&pool).await.unwrap().items.is_empty());
    let rows: Vec<(String, i64, i64)> =
        sqlx::query_as("SELECT host, sequence, read_sequence FROM notification_inbox ORDER BY host")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows, vec![("remote".into(), 1, 1)]);
}

#[tokio::test]
async fn notification_ingest_rejects_invalid_page_without_advancing_cursor() {
    let pool = pool("invalid-page").await;
    notifications::ingest(&pool, "local", &page("source-a", None, 4, 4, vec![]))
        .await
        .unwrap();
    let invalid = page(
        "source-a",
        Some(4),
        5,
        5,
        vec![notifications::ResultNotice {
            kind: "unknown".into(),
            ..result(5, 8)
        }],
    );
    assert!(notifications::ingest(&pool, "local", &invalid)
        .await
        .is_err());
    let snapshot = notifications::snapshot(&pool).await.unwrap();
    assert_eq!(snapshot.sources[0].cursor, 4);
}

#[tokio::test]
async fn cancellation_intent_survives_process_registration_and_suppresses_terminal_result() {
    let pool = pool("cancel-race").await;
    sqlx::query("INSERT INTO tasks(repo, branch, base, worktree_path, instruction, state, created_at, updated_at) VALUES ('repo', 'branch', 'base', 'path', 'task', 'Running', 1, 1)")
        .execute(&pool).await.unwrap();
    assert!(db::record_notification_cancel_intent(&pool, 1)
        .await
        .unwrap());
    db::record_task_process_start(&pool, 1, 42, &"a".repeat(64), "terminal", 2)
        .await
        .unwrap();
    assert!(db::finish_running_task_with_notification(
        &pool,
        1,
        db::state::AWAITING_REVIEW,
        3,
        "completed",
        None,
        "result"
    )
    .await
    .unwrap());
    assert!(notifications::source_page(&pool, Some(0))
        .await
        .unwrap()
        .results
        .is_empty());
    assert!(db::mark_running_from_review(&pool, 1, 4).await.unwrap());
    assert!(db::finish_running_task_with_notification(
        &pool,
        1,
        db::state::AWAITING_REVIEW,
        5,
        "completed",
        None,
        "result"
    )
    .await
    .unwrap());
    assert_eq!(
        notifications::source_page(&pool, Some(0))
            .await
            .unwrap()
            .results
            .len(),
        1
    );
}

#[tokio::test]
async fn created_cancellation_intent_suppresses_local_terminal_exit() {
    let pool = pool("created-cancel").await;
    sqlx::query("INSERT INTO tasks(repo, branch, base, worktree_path, instruction, state, created_at, updated_at) VALUES ('repo', 'branch', 'base', 'path', 'task', 'Created', 1, 1)")
        .execute(&pool).await.unwrap();
    assert!(db::record_notification_cancel_intent(&pool, 1).await.unwrap());
    assert!(db::mark_awaiting_review_with_notification(&pool, 1, 2, None, "result").await.unwrap());
    assert!(notifications::source_page(&pool, Some(0)).await.unwrap().results.is_empty());
}

#[tokio::test]
async fn cancellation_intent_is_removed_when_no_signal_was_sent() {
    let pool = pool("unsent-cancel").await;
    sqlx::query("INSERT INTO tasks(repo, branch, base, worktree_path, instruction, state, created_at, updated_at) VALUES ('repo', 'branch', 'base', 'path', 'task', 'Running', 1, 1)")
        .execute(&pool).await.unwrap();
    assert!(db::record_notification_cancel_intent(&pool, 1).await.unwrap());
    db::clear_notification_cancel_if_signal_not_sent(&pool, 1).await.unwrap();
    assert!(db::finish_running_task_with_notification(&pool, 1, db::state::AWAITING_REVIEW, 2, "completed", None, "result").await.unwrap());
    assert_eq!(notifications::source_page(&pool, Some(0)).await.unwrap().results.len(), 1);
}

#[tokio::test]
async fn question_record_reaches_the_result_source_without_a_state_transition() {
    let pool = pool("question-record").await;
    sqlx::query("INSERT INTO tasks(repo, branch, base, worktree_path, instruction, state, created_at, updated_at) VALUES ('repo', 'branch', 'base', 'path', 'task', 'Running', 1, 1)")
        .execute(&pool).await.unwrap();
    notifications::record_question(&pool, 1, 5).await.unwrap();
    // `after: None`은 커서 초기화용이라 결과를 싣지 않는다 — 첫 순번부터 읽는다.
    let page = notifications::source_page(&pool, Some(0)).await.unwrap();
    assert_eq!(page.results.len(), 1);
    assert_eq!(page.results[0].kind, "question");
    assert_eq!(page.results[0].task_id, 1);
    let state: String = sqlx::query_scalar("SELECT state FROM tasks WHERE id = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state, "Running");
    // 취소 의도가 먼저 남았으면 질문 알림도 싣지 않는다 — 결과 알림과 같은 규칙이다.
    assert!(db::record_notification_cancel_intent(&pool, 1).await.unwrap());
    notifications::record_question(&pool, 1, 6).await.unwrap();
    assert_eq!(notifications::source_page(&pool, Some(0)).await.unwrap().results.len(), 1);
}

#[tokio::test]
async fn init_pool_drops_legacy_attention_triggers_before_tables() {
    // 2026-09-26 제거된 할 일 기능이 남긴 트리거·테이블이 있는 DB를 흉내 낸다.
    let path = crate::testtmp::dir().join("notifications-legacy-attention.sqlite");
    let pool = db::init_pool(path.to_str().unwrap()).await.unwrap();
    for statement in [
        "CREATE TABLE attention_condition_states (source_id TEXT NOT NULL, task_id INTEGER NOT NULL)",
        "CREATE TRIGGER attention_task_after_update AFTER UPDATE OF state ON tasks BEGIN INSERT INTO attention_condition_states(source_id, task_id) VALUES ('legacy', NEW.id); END",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }
    pool.close().await;
    let pool = db::init_pool(path.to_str().unwrap()).await.unwrap();
    let leftovers: Vec<(String,)> =
        sqlx::query_as("SELECT name FROM sqlite_master WHERE name LIKE 'attention_%'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}
