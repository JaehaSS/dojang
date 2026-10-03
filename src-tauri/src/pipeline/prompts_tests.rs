use super::*;
use crate::multireview::verdict::Severity;
use crate::pipeline::model::Requirement;

fn spec() -> Spec {
    Spec {
        goal: "G".into(),
        requirements: vec![Requirement { id: "R1".into(), text: "do foo".into(), acceptance: "foo passes".into() }],
        out_of_scope: vec!["bar".into()],
    }
}

fn ticket() -> TicketDraft {
    TicketDraft {
        key: "T1".into(),
        title: "Foo".into(),
        body: "implement foo".into(),
        vendor: "codex".into(),
        acceptance_commands: vec!["npm test -- foo".into()],
        allowed_paths: vec!["src/lib/foo.ts".into()],
        covers: vec!["R1".into()],
        ..Default::default()
    }
}

fn finding() -> Finding {
    Finding {
        severity: Severity::Blocking,
        requirement: Some("R1".into()),
        file: Some("a.rs".into()),
        line: Some(7),
        summary: "broken".into(),
        evidence: "line".into(),
    }
}

#[test]
fn ticket_instruction_has_scope_acceptance_and_no_commit() {
    let p = ticket_instruction(&spec(), &ticket());
    for needle in ["src/lib/foo.ts", "npm test -- foo", "커밋", "do foo", "implement foo", "다른 티켓의 범위", "bar"] {
        assert!(p.contains(needle), "{needle}");
    }
}

#[test]
fn fix_prompt_embeds_problem_and_scope() {
    let p = ticket_fix_prompt(&ticket(), "exit 1: boom");
    assert!(p.contains("boom") && p.contains("src/lib/foo.ts") && p.contains("npm test -- foo") && p.contains("커밋"));
}

#[test]
fn conflict_prompt_lists_paths_and_forbids_commit() {
    let p = conflict_prompt("T2", "dojang/t2", &["a.txt".into()], &spec());
    assert!(p.contains("a.txt") && p.contains("dojang/t2") && p.contains("<<<<<<<") && p.contains("git add"));
}

#[test]
fn final_fix_and_findings_text() {
    let p = final_fix_prompt(&spec(), &[finding()]);
    assert!(p.contains("broken") && p.contains("a.rs:7") && p.contains("커밋"));
    assert_eq!(findings_text(&[]), "(지적 없음)");
    let many: Vec<Finding> = (0..2000).map(|_| finding()).collect();
    assert!(findings_text(&many).chars().count() < FINDINGS_MAX_CHARS + 20);
}
