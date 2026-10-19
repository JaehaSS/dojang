//! 대화 모드 라이브 검증 — 실제 벤더 CLI를 `run_turn`으로 직접 호출.
//! 인증·비용·비결정성 때문에 기본 `#[ignore]`. 수동 실행:
//!   cargo test --test convo_live_test -- --ignored --nocapture
//! agy 경로는 구조화 스트림이 없어 텍스트 1블록 + 합성 Result로 정규화되는지 확인이 목적(#3).

#![cfg(unix)]

#[path = "support/temp_root.rs"]
mod temp_root;

use praxis_lib::convo::{run_turn, ConvoEvent, Vendor};
use praxis_lib::reviewer::which;

/// 임시 디렉터리(worktree 대용)에서 한 턴 실행 → 수신 이벤트 수집.
fn collect(vendor: Vendor, msg: &str) -> (Result<String, String>, Vec<ConvoEvent>) {
    let bin = which(vendor.bin()).expect("벤더 CLI가 PATH에 있어야 함");
    let cwd = temp_root::dir();
    let cwd = cwd.to_string_lossy().into_owned();
    let mut evs = Vec::new();
    let res = run_turn(
        &cwd,
        msg,
        None,
        90,
        vendor,
        &bin,
        None,
        |_pid| {},
        |ev| evs.push(ev),
    )
    .map(|o| o.session_id); // 관심사는 resume 토큰/이벤트 — 종료 관측은 unit test에서 검증.
    (res, evs)
}

#[test]
#[ignore = "live: agy CLI 호출 (인증 필요)"]
fn agy_conversation_emits_text_and_result() {
    let (res, evs) = collect(
        Vendor::Agy,
        "Run the shell command `echo KIWI` with your terminal tool, then reply with the single word it printed.",
    );
    println!("agy result={res:?}\nevents={evs:#?}");
    let session = res.expect("run_turn 성공해야");
    assert!(
        !session.is_empty() && session != praxis_lib::convo::AGY_CONTINUE,
        "stream-json의 conversation_id를 resume 토큰으로 돌려줘야 함: {session}"
    );
    assert!(
        evs.iter().any(|e| matches!(e, ConvoEvent::ToolUse { .. })),
        "툴 호출이 카드로 흘러야 함: {evs:?}"
    );
    assert!(
        evs.iter().any(|e| matches!(e, ConvoEvent::Text { text, .. } if text.contains("KIWI"))),
        "응답 텍스트가 흘러야 함: {evs:?}"
    );
    assert!(
        evs.iter().any(|e| matches!(e, ConvoEvent::Result { is_error: false, .. })),
        "Result로 턴 종료를 알려야 함: {evs:?}"
    );
}

/// 서브 에이전트 상관관계 라이브 검증 (#99/#100) — 실제 claude 스트림에서
/// Task 스폰(tool_id) → parented 이벤트(parent_id) → 완료(tool_use_id 매칭)가 전부 잡히는지.
#[test]
#[ignore = "live: claude CLI 호출 (인증 필요)"]
fn claude_subagent_events_carry_correlation_ids() {
    let bin = which(Vendor::Claude.bin()).expect("claude가 PATH에 있어야 함");
    let cwd = temp_root::dir().to_string_lossy().into_owned();
    let mut evs = Vec::new();
    let res = run_turn(
        &cwd,
        "Use the Task tool to spawn exactly one subagent (subagent_type: general-purpose, \
         description: 'count files') whose prompt is: run `ls` in the current directory and \
         report only the number of entries. After it returns, reply DONE.",
        None,
        180,
        Vendor::Claude,
        &bin,
        Some("haiku"),
        |_pid| {},
        |ev| evs.push(ev),
    );
    println!("result={res:?}");
    assert!(res.is_ok(), "run_turn 성공해야: {res:?}");

    // 1) 메인 스레드의 Task 스폰 — tool_id가 있어야 패널이 항목을 만든다.
    let spawn_id = evs
        .iter()
        .find_map(|e| match e {
            ConvoEvent::ToolUse {
                name,
                tool_id: Some(id),
                parent_id: None,
                ..
            } if name == "Task" || name == "Agent" => Some(id.clone()),
            _ => None,
        })
        .expect("Task/Agent 스폰 tool_use(tool_id 보유)가 스트림에 있어야 함");

    // 2) 서브 에이전트 내부 활동 — parent_id가 스폰 id를 가리켜야 탭/lastOp가 채워진다.
    let parented = evs
        .iter()
        .filter(|e| {
            matches!(e,
                ConvoEvent::ToolUse { parent_id: Some(p), .. }
                | ConvoEvent::ToolResult { parent_id: Some(p), .. }
                | ConvoEvent::Text { parent_id: Some(p), .. } if *p == spawn_id)
        })
        .count();
    assert!(
        parented > 0,
        "스폰 {spawn_id}에 귀속된 서브 이벤트가 있어야 함: {evs:#?}"
    );

    // 3) 메인 스레드의 Task 완료 — tool_use_id 매칭으로 done 전이가 가능해야 한다.
    assert!(
        evs.iter().any(|e| matches!(e,
            ConvoEvent::ToolResult { tool_use_id: Some(t), parent_id: None, .. } if *t == spawn_id)),
        "Task 결과(tool_use_id={spawn_id})가 있어야 함: {evs:#?}"
    );
    println!("OK: spawn={spawn_id}, 서브 귀속 이벤트 {parented}건, 완료 매칭 확인");
}

#[test]
#[ignore = "live: codex CLI 호출 (인증 필요)"]
fn codex_conversation_emits_session_and_result() {
    let (res, evs) = collect(
        Vendor::Codex,
        "Reply with exactly OK and nothing else. Do not run any commands.",
    );
    println!("codex result={res:?}\nevents={evs:#?}");
    assert!(res.is_ok(), "run_turn 성공해야: {res:?}");
    assert!(
        evs.iter()
            .any(|e| matches!(e, ConvoEvent::SessionInit { .. })),
        "codex thread.started → SessionInit 있어야: {evs:?}"
    );
    assert!(
        evs.iter().any(|e| matches!(e, ConvoEvent::Result { .. })),
        "turn.completed → Result 있어야: {evs:?}"
    );
}
