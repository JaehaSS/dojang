//! 토론 세션의 DB 계약과 이벤트 직렬화. `cargo test --test convo_debate_test`
//!
//! 라운드 왕복 자체는 벤더 프로세스 둘이 필요하므로 여기서 덮지 않는다 — 판정은
//! `convo::debate` 단위 테스트가, 왕복은 수동 워크스루가 맡는다(계획 0069 §5).

#[path = "support/temp_root.rs"]
mod temp_root;

use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::convo::{ConvoEvent, DebateEndReason, Side};
use praxis_lib::db::{self, state};
use praxis_lib::runner::process;

static COUNTER: AtomicU32 = AtomicU32::new(0);

async fn pool() -> (sqlx::SqlitePool, String) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = temp_root::dir()
        .join(format!("convo-debate-{}-{n}.sqlite", std::process::id()))
        .to_string_lossy()
        .into_owned();
    let pool = db::init_pool(&path).await.expect("init");
    (pool, path)
}

async fn conversation_task(pool: &sqlx::SqlitePool) -> i64 {
    let id = db::insert_task(
        pool,
        "/repo",
        "praxis/debate",
        "main",
        "/repo/.praxis/wt/debate",
        "토론 대상",
        Some("claude"),
        None,
        "conversation",
        1000,
    )
    .await
    .expect("insert");
    db::update_state(pool, id, state::AWAITING_REVIEW, 1001)
        .await
        .expect("state");
    id
}

/// (1) 우측 행은 좌측 원천을 건드리지 않는다 — 세션 id도 미소비 캡슐도 그대로다.
#[tokio::test]
async fn right_row_leaves_the_left_source_untouched() {
    let (pool, path) = pool().await;
    let id = conversation_task(&pool).await;
    db::set_convo_session(&pool, id, "left-session")
        .await
        .expect("session");
    db::set_pending_capsule(&pool, id, "## 캡슐")
        .await
        .expect("capsule");

    db::insert_debate_sides(&pool, id, &[(Side::Right, "codex", Some("gpt-5"))])
        .await
        .expect("insert side");
    db::set_debate_side_session(&pool, id, Side::Right, "right-session")
        .await
        .expect("side session");

    let task = db::get_task(&pool, id).await.unwrap().expect("task");
    assert_eq!(task.convo_session_id.as_deref(), Some("left-session"));
    assert_eq!(task.pending_capsule.as_deref(), Some("## 캡슐"));
    let row = db::debate_sides(&pool, id).await.unwrap().into_iter().next().expect("side");
    assert_eq!(row.side, "right");
    assert_eq!(row.agent, "codex");
    assert_eq!(row.model.as_deref(), Some("gpt-5"));
    assert_eq!(row.vendor_session_id.as_deref(), Some("right-session"));
    let _ = std::fs::remove_file(&path);
}

/// (2) 같은 면을 다시 삽입하면 실패한다 — 조용히 덮으면 살아 있는 우측 세션이 사라진다.
#[tokio::test]
async fn reinserting_the_same_side_conflicts() {
    let (pool, path) = pool().await;
    let id = conversation_task(&pool).await;
    db::insert_debate_sides(&pool, id, &[(Side::Right, "codex", None)])
        .await
        .expect("insert side");
    assert!(db::insert_debate_sides(&pool, id, &[(Side::Right, "agy", None)])
        .await
        .is_err());
    let row = db::debate_sides(&pool, id).await.unwrap().into_iter().next().expect("side");
    assert_eq!(row.agent, "codex");
    let _ = std::fs::remove_file(&path);
}

/// (3) 삭제하면 조회 목록이 빈다 — "토론 중"은 이 행의 존재로만 파생된다.
#[tokio::test]
async fn deleting_the_side_ends_the_debate() {
    let (pool, path) = pool().await;
    let id = conversation_task(&pool).await;
    db::insert_debate_sides(&pool, id, &[(Side::Right, "codex", None)])
        .await
        .expect("insert side");
    db::delete_debate_sides(&pool, id)
        .await
        .expect("delete");
    assert!(db::debate_sides(&pool, id).await.unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
}

/// (4) `speaker`가 없으면 오늘과 **바이트가 같다**. 리플레이·`json_extract` 계약이 그대로다.
#[test]
fn absent_speaker_serializes_byte_identically() {
    let event = ConvoEvent::Text {
        text: "안녕".into(),
        parent_id: None,
    };
    assert_eq!(
        praxis_lib::convo::stored_event_json(&event, None).unwrap(),
        serde_json::to_string(&event).unwrap()
    );
    let result = ConvoEvent::Result {
        text: "끝".into(),
        is_error: false,
        session_id: "s".into(),
        cost_usd: 0.125,
        num_turns: 3,
        tokens_in: 10,
        tokens_out: 20,
    };
    assert_eq!(
        praxis_lib::convo::stored_event_json(&result, None).unwrap(),
        serde_json::to_string(&result).unwrap()
    );
}

/// (5) `speaker`는 루트의 형제다 — `$.kind`가 여전히 루트에서 읽힌다.
#[tokio::test]
async fn speaker_is_a_root_sibling_of_kind() {
    let (pool, path) = pool().await;
    let id = conversation_task(&pool).await;
    let event = ConvoEvent::Text {
        text: "우측 발화".into(),
        parent_id: None,
    };
    let encoded = praxis_lib::convo::stored_event_json(&event, Some(Side::Right)).unwrap();
    db::append_convo_event(&pool, id, &encoded, 1002)
        .await
        .expect("append");

    let row: (String, String) = sqlx::query_as(
        "SELECT json_extract(event, '$.speaker'), json_extract(event, '$.kind') \
         FROM convo_events WHERE task_id = ?",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .expect("extract");
    assert_eq!(row.0, "right");
    assert_eq!(row.1, "text");
    let _ = std::fs::remove_file(&path);
}

/// (6) 종료 이벤트의 모양 — 프론트 유니온이 이 두 키만 본다.
#[test]
fn debate_ended_serializes_kind_and_reason() {
    let event = ConvoEvent::DebateEnded {
        reason: DebateEndReason::Consensus,
    };
    assert_eq!(
        praxis_lib::convo::stored_event_json(&event, None).unwrap(),
        r#"{"kind":"debate_ended","reason":"consensus"}"#
    );
    let capped = ConvoEvent::DebateEnded {
        reason: DebateEndReason::RoundCap,
    };
    assert_eq!(
        praxis_lib::convo::stored_event_json(&capped, None).unwrap(),
        r#"{"kind":"debate_ended","reason":"round_cap"}"#
    );
}

/// T3 (5) 러너는 토론 중인 작업을 거부한다 — 안 그러면 우측이 빠진 채 좌측만 혼잣말을 한다.
#[tokio::test]
async fn runner_refuses_a_task_in_debate() {
    let (pool, path) = pool().await;
    let id = conversation_task(&pool).await;
    db::update_state(&pool, id, state::RUNNING, 1002)
        .await
        .expect("state");
    db::insert_debate_sides(&pool, id, &[(Side::Right, "codex", None)])
        .await
        .expect("insert side");
    let task = db::get_task(&pool, id).await.unwrap().expect("task");

    let error = process::run_conversation_task_with_bin(
        pool.clone(),
        task,
        1003,
        process::active_conversation_tasks(),
        "/bin/echo",
    )
    .await
    .expect_err("토론 중인 작업은 거부");
    assert!(error.contains("토론"), "{error}");
    // 거부는 프롬프트 조립 전이다 — user 이벤트조차 남지 않는다.
    assert!(db::list_convo_events(&pool, id).await.unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
}
