//! 토론 커맨드의 게이트와 라운드 상한 설정. 라운드 루프 자체는 벤더 프로세스가 필요하므로
//! 여기서 덮지 않는다 — 판정은 `convo::debate`가, 왕복은 수동 워크스루가 맡는다.
//!
//! `commands`가 크레이트 밖에 보이지 않아 `tests/`가 아니라 여기 산다.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::*;
use crate::convo::Side;

async fn pool(name: &str) -> SqlitePool {
    let path = crate::testtmp::dir().join(format!("commands-debate-{name}.sqlite"));
    db::init_pool(path.to_str().unwrap()).await.unwrap()
}

async fn conversation_task(pool: &SqlitePool, state: &str) -> i64 {
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
    .unwrap();
    db::update_state(pool, id, state, 1001).await.unwrap();
    id
}

fn active() -> ActiveConvos {
    Arc::new(Mutex::new(HashMap::new()))
}

fn opponent(agent: &str, model: &str) -> DebateOpponent {
    DebateOpponent {
        agent: agent.into(),
        model: Some(model.into()),
    }
}

/// (1) 시작 게이트는 에이전트 전환과 같다 — 대화 모드 + 검토 대기.
#[tokio::test]
async fn debate_start_needs_a_conversation_awaiting_review() {
    let pool = pool("start-gate").await;
    let id = conversation_task(&pool, db::state::RUNNING).await;
    let codex = [opponent("codex", "gpt-5")];
    let error = debate_start_checked(&pool, active(), id, &codex)
        .await
        .expect_err("실행 중에는 시작할 수 없다");
    assert!(error.contains("검토 대기"), "{error}");
    assert!(db::debate_sides(&pool, id).await.unwrap().is_empty());

    db::update_state(&pool, id, db::state::AWAITING_REVIEW, 1002)
        .await
        .unwrap();
    debate_start_checked(&pool, active(), id, &codex)
        .await
        .expect("시작");
    let rows = db::debate_sides(&pool, id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].side, "right");
    assert_eq!(rows[0].agent, "codex");
    // 벤더 세션은 첫 우측 턴이 판다.
    assert!(rows[0].vendor_session_id.is_none());

    // 이미 도는 토론은 다시 열지 않는다.
    let error = debate_start_checked(&pool, active(), id, &codex)
        .await
        .expect_err("이미 토론 중");
    assert!(error.contains("이미 토론 중"), "{error}");
}

/// (1b) 상대 둘이면 right·third 두 행이 자리 순서대로 생긴다(설계 2026-09-23 A1).
#[tokio::test]
async fn debate_start_with_two_opponents_seats_right_and_third() {
    let pool = pool("start-three").await;
    let id = conversation_task(&pool, db::state::AWAITING_REVIEW).await;
    debate_start_checked(
        &pool,
        active(),
        id,
        &[opponent("codex", "gpt-5"), DebateOpponent { agent: "agy".into(), model: None }],
    )
    .await
    .expect("3자 시작");
    let rows = db::debate_sides(&pool, id).await.unwrap();
    let seats: Vec<_> = rows
        .iter()
        .map(|row| (row.side.as_str(), row.agent.as_str(), row.model.as_deref()))
        .collect();
    assert_eq!(
        seats,
        vec![("right", "codex", Some("gpt-5")), ("third", "agy", None)]
    );
}

/// (1c) 상대 목록은 1~2명, 서로 다르고 현재 에이전트와도 달라야 한다 — 실패하면 행이 하나도 남지 않는다.
#[tokio::test]
async fn debate_start_rejects_invalid_opponent_lists() {
    let pool = pool("start-invalid").await;
    let id = conversation_task(&pool, db::state::AWAITING_REVIEW).await;
    let cases: [(&[DebateOpponent], &str); 4] = [
        (&[], "1~2명"),
        (
            &[opponent("codex", "a"), opponent("agy", "b"), opponent("claude", "c")],
            "1~2명",
        ),
        (&[opponent("codex", "a"), opponent("codex", "b")], "두 번"),
        (&[opponent("codex", "a"), opponent("claude", "b")], "현재 에이전트"),
    ];
    for (opponents, expected) in cases {
        let error = debate_start_checked(&pool, active(), id, opponents)
            .await
            .expect_err("거부");
        assert!(error.contains(expected), "{expected}: {error}");
        assert!(db::debate_sides(&pool, id).await.unwrap().is_empty());
    }
}

/// (1d) 저장된 자리 집합이 좌측부터 빈틈없이 이어지지 않으면 읽기가 실패한다 — 셋째만 남은 행으로
/// 라운드를 돌리면 좌측 다음 차례가 없다(설계 2026-09-23 D1).
#[tokio::test]
async fn debate_sides_rejects_a_seat_set_with_a_gap() {
    let pool = pool("seat-gap").await;
    let id = conversation_task(&pool, db::state::AWAITING_REVIEW).await;
    db::insert_debate_sides(&pool, id, &[(Side::Third, "agy", None)])
        .await
        .unwrap();
    assert!(db::debate_sides(&pool, id).await.is_err());
}

/// (2) 사이드 행이 있으면 전환이 거부된다 — 점유가 없어도 그렇다(원격·재시도 경로가 여기를 지난다).
#[tokio::test]
async fn agent_switch_is_refused_during_a_debate() {
    let pool = pool("switch-refusal").await;
    let id = conversation_task(&pool, db::state::AWAITING_REVIEW).await;
    db::insert_debate_sides(&pool, id, &[(Side::Right, "codex", None)])
        .await
        .unwrap();
    let error = switch_task_agent_checked(&pool, active(), id, "agy", "gemini-3")
        .await
        .expect_err("토론 중 전환 거부");
    assert_eq!(
        error,
        "토론 중에는 에이전트를 바꿀 수 없습니다 — 먼저 토론을 끝내세요"
    );
}

/// (3) 끝내면 사이드 행이 전부 사라지고 종료 경계가 원장에 남는다(설계 2026-09-23 A7).
#[tokio::test]
async fn debate_end_removes_the_row_and_records_the_boundary() {
    let pool = pool("end").await;
    let id = conversation_task(&pool, db::state::AWAITING_REVIEW).await;
    db::insert_debate_sides(
        &pool,
        id,
        &[(Side::Right, "codex", None), (Side::Third, "agy", None)],
    )
    .await
    .unwrap();
    // 진행 중인 시퀀스가 점유를 쥐고 있으면 수동 종료는 물러난다.
    let held = active();
    held.lock().unwrap().insert(
        id,
        ActiveConvo {
            pgid: None,
            vendor_bin: String::new(),
            started_at: 1,
            last_event_at: 1,
            last_operation: None,
            interrupted: false,
        },
    );
    debate_end_checked(&pool, held.clone(), id)
        .await
        .expect_err("점유 중에는 끝낼 수 없다");
    assert_eq!(db::debate_sides(&pool, id).await.unwrap().len(), 2);

    debate_end_checked(&pool, active(), id).await.expect("끝내기");

    assert!(db::debate_sides(&pool, id).await.unwrap().is_empty());
    let events = db::list_convo_events(&pool, id).await.unwrap();
    assert_eq!(
        events.last().map(String::as_str),
        Some(r#"{"kind":"debate_ended","reason":"aborted"}"#)
    );
}

/// (4) 중단 플래그는 pgid와 무관하게 선다 — 라운드 사이가 정확히 그 구간이다.
#[test]
fn interrupt_sets_the_flag_without_a_pgid() {
    let active = active();
    active.lock().unwrap().insert(
        7,
        ActiveConvo {
            pgid: None,
            vendor_bin: "claude".into(),
            started_at: 1,
            last_event_at: 1,
            last_operation: None,
            interrupted: false,
        },
    );
    interrupt_convo_entry(&active, 7).expect("pgid가 없어도 중단은 성립한다");
    assert!(active.lock().unwrap().get(&7).unwrap().interrupted);
    // 진행 중인 턴이 없으면 여전히 거절한다 — 잔류 pgid에 신호를 보내지 않기 위한 가드다.
    assert!(interrupt_convo_entry(&active, 8).is_err());
}

/// T4 — 미설정은 기본 3, 범위 밖은 거부(클램프 없음), 범위 안은 왕복한다.
#[tokio::test]
async fn round_cap_setting_rejects_out_of_range_values() {
    let pool = pool("round-cap").await;
    assert_eq!(
        debate_round_cap(&pool).await,
        crate::convo::debate::DEFAULT_ROUND_CAP
    );

    let error = set_debate_round_cap(&pool, 7).await.expect_err("범위 밖");
    assert!(error.contains("2~5"), "{error}");
    assert_eq!(db::get_setting(&pool, DEBATE_ROUND_CAP_KEY).await.unwrap(), None);

    set_debate_round_cap(&pool, 4).await.expect("저장");
    assert_eq!(debate_round_cap(&pool).await, 4);
}
