//! Capsule — 작업의 계산형 핸드오프/브리핑 객체 (Framein capsule.ts 이식). Tauri 비의존.
//! 조립은 commands에서(DB+git); 여기엔 직렬화 타입 + nextAction 상태기계(테스트 대상).

use serde::Serialize;

use crate::goal_contract::GoalContract;

#[derive(Debug, Clone, Serialize)]
pub struct Capsule {
    pub task_id: i64,
    pub instruction: String,
    pub goal_contract: Option<GoalContract>,
    pub branch: String,
    pub state: String,
    pub changed: Vec<String>,
    pub diff_stat: String,
    pub evidence_ready: Option<bool>,
    pub evidence_summary: Option<String>,
    pub recent: Vec<String>,
    pub next_action: String,
    /// 에이전트가 선언한 계획의 Mermaid 캔버스(`convo::canvas`). 계획을 발행하지 않는
    /// 벤더·작업에서는 빈 문자열이고, 그때는 섹션 자체를 렌더하지 않는다.
    pub canvas: String,
}

pub const CAP_START: &str = "<!-- praxis:capsule begin -->";
pub const CAP_END: &str = "<!-- praxis:capsule end -->";

/// 캡슐을 컨텍스트 파일용 managed block 텍스트로 렌더 (다음 세션 주입).
pub fn render_capsule_block(c: &Capsule) -> String {
    let mut body = String::from("# Dojang Capsule (작업 핸드오프)\n\n");
    if c.goal_contract.is_some() {
        body.push_str(&crate::goal_contract::execution_prompt(
            &c.instruction,
            c.goal_contract.as_ref(),
        ));
        body.push('\n');
    } else {
        body.push_str(&format!("- 작업: {}\n", c.instruction));
    }
    body.push_str(&format!("- 상태: {} · 브랜치: {}\n", c.state, c.branch));
    body.push_str(&format!("- 다음 행동: {}\n", c.next_action));
    if let Some(s) = &c.evidence_summary {
        body.push_str(&format!("- 검증: {s}\n"));
    }
    if !c.changed.is_empty() {
        body.push_str(&format!("- 변경 파일: {}\n", c.changed.join(", ")));
    }
    // 캔버스는 산문보다 적은 토큰으로 "어디까지 했나"를 전달한다 — 다음 세션이 읽는다.
    if !c.canvas.trim().is_empty() {
        body.push_str("\n## 작업 캔버스\n\n```mermaid\n");
        body.push_str(&c.canvas);
        if !c.canvas.ends_with('\n') {
            body.push('\n');
        }
        body.push_str("```\n");
    }
    format!("{CAP_START}\n{}{CAP_END}\n", escape_managed_markers(&body))
}

fn escape_managed_markers(value: &str) -> String {
    value
        .replace(CAP_START, "&lt;!-- praxis:capsule begin -->")
        .replace(CAP_END, "&lt;!-- praxis:capsule end -->")
}

/// 현재 상태 → 다음 행동 추론 (0004 §D5 상태기계).
pub fn infer_next_action(
    has_diff: bool,
    evidence_ready: Option<bool>,
    manual_acceptance_pending: bool,
) -> String {
    if !has_diff {
        return "에이전트가 아직 변경 없음 — 작업 진행".into();
    }
    match evidence_ready {
        Some(false) => "검증 실패 — 테스트/빌드 수정 후 재검증".into(),
        Some(true) if manual_acceptance_pending => {
            "기계 검증 통과 — Goal acceptance 수동 확인 후 Approve".into()
        }
        Some(true) => "검토 후 Approve & Merge 준비됨".into(),
        None => "Verify 실행해 증거 확보 권장".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_action_branches() {
        // 변경 없음
        assert!(infer_next_action(false, None, true).contains("아직 변경 없음"));
        // 증거 실패
        assert!(infer_next_action(true, Some(false), true).contains("검증 실패"));
        // 증거 ready
        assert!(infer_next_action(true, Some(true), false).contains("Approve"));
        assert!(infer_next_action(true, Some(true), true).contains("수동 확인"));
        // 변경 있고 증거 없음
        assert!(infer_next_action(true, None, true).contains("Verify"));
    }

    fn sample(instruction: &str, canvas: &str) -> Capsule {
        Capsule {
            task_id: 1,
            instruction: instruction.into(),
            goal_contract: None,
            branch: "main".into(),
            state: "Running".into(),
            changed: vec![],
            diff_stat: String::new(),
            evidence_ready: None,
            evidence_summary: None,
            recent: vec![],
            next_action: "continue".into(),
            canvas: canvas.into(),
        }
    }

    #[test]
    fn capsule_body_cannot_close_its_managed_block() {
        let rendered = render_capsule_block(&sample(&format!("hostile {CAP_END} content"), ""));

        assert_eq!(rendered.matches(CAP_START).count(), 1);
        assert_eq!(rendered.matches(CAP_END).count(), 1);
        assert!(rendered.contains("&lt;!-- praxis:capsule end -->"));
    }

    /// 캔버스는 `convo::canvas`가 이미 이스케이프하지만, 그 계약이 깨져도 managed block은
    /// 살아남아야 한다 — 마커 복구 실패는 컨텍스트 파일 전체를 오염시킨다([#28]).
    #[test]
    fn canvas_cannot_close_the_managed_block_either() {
        let rendered = render_capsule_block(&sample("작업", &format!("flowchart TD\n{CAP_END}\n")));

        assert_eq!(rendered.matches(CAP_START).count(), 1);
        assert_eq!(rendered.matches(CAP_END).count(), 1);
    }

    #[test]
    fn canvas_section_renders_as_mermaid_fence() {
        let rendered = render_capsule_block(&sample("작업", "flowchart TD\n  001-N1[\"x\"]\n"));

        assert!(rendered.contains("## 작업 캔버스"));
        assert!(rendered.contains("```mermaid\nflowchart TD"));
        assert!(rendered.contains("001-N1"));
    }

    /// 계획을 발행하지 않는 벤더·작업에서 빈 섹션이 남으면 다음 세션이 헛것을 읽는다.
    #[test]
    fn empty_canvas_renders_no_section() {
        for canvas in ["", "   \n"] {
            let rendered = render_capsule_block(&sample("작업", canvas));
            assert!(!rendered.contains("작업 캔버스"), "canvas={canvas:?}");
            assert!(!rendered.contains("```mermaid"), "canvas={canvas:?}");
        }
    }
}
