//! TodoWrite 원문 → Plan 이벤트 → 영속 JSON → 캔버스 → Capsule 블록의 전 경로 확인.
use praxis_lib::capsule::{render_capsule_block, Capsule, CAP_END, CAP_START};
use praxis_lib::convo::{canvas, parse_events};

#[test]
fn todowrite_line_reaches_capsule_block_as_mermaid() {
    let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"TodoWrite","input":{"todos":[
        {"content":"스키마 확인","status":"completed","activeForm":"확인 중"},
        {"content":"주입 경로 수정","status":"in_progress","activeForm":"수정 중"}]}}]}}"#;

    // 파싱 → 영속 형태(JSON 문자열)로 직렬화 — commands.rs의 persist 채널과 같은 경로.
    let persisted: Vec<String> = parse_events(line)
        .iter()
        .map(|e| serde_json::to_string(e).unwrap())
        .collect();

    let mmd = canvas::latest_from_events(&persisted);
    assert!(
        mmd.contains("001-N1[\"status: done<br/>summary: 스키마 확인\"]"),
        "{mmd}"
    );
    assert!(mmd.contains("001-N1 --> 001-N2"), "{mmd}");

    let block = render_capsule_block(&Capsule {
        task_id: 1,
        instruction: "작업".into(),
        goal_contract: None,
        branch: "main".into(),
        state: "Running".into(),
        changed: vec![],
        diff_stat: String::new(),
        evidence_ready: None,
        evidence_summary: None,
        recent: vec![],
        next_action: "continue".into(),
        canvas: mmd,
    });

    assert!(block.contains("# Dojang Capsule (작업 핸드오프)"));
    assert!(block.contains("## 작업 캔버스"));
    assert!(block.contains("```mermaid"));
    assert!(block.contains("주입 경로 수정"));
    // managed block 경계는 정확히 한 쌍이어야 한다 — 캔버스가 끼어들어도.
    assert_eq!(block.matches(CAP_START).count(), 1);
    assert_eq!(block.matches(CAP_END).count(), 1);
}
