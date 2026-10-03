use super::task_creation::validate_request;
use super::QueuedTaskRequest;

fn request(agent: &str, model: &str, reasoning_effort: &str) -> QueuedTaskRequest {
    QueuedTaskRequest {
        repository: "/repo".to_string(),
        instruction: "ship safely".to_string(),
        agent: agent.to_string(),
        role: "implementer".to_string(),
        model: model.to_string(),
        reasoning_effort: reasoning_effort.to_string(),
        mode: "terminal".to_string(),
        goal_contract: None,
        resume_session: None,
        resume_vendor: None,
    }
}

#[test]
fn runner_rejects_known_unsupported_model_effort_pairs() {
    let luna = validate_request(&request("codex", "gpt-5.6-luna", "ultra")).unwrap_err();
    let legacy = validate_request(&request("codex", "gpt-5.4", "max")).unwrap_err();

    assert!(luna.contains("gpt-5.6-luna"));
    assert!(legacy.contains("gpt-5.4"));
}

#[test]
fn runner_accepts_supported_or_inherited_effort() {
    assert!(validate_request(&request("codex", "gpt-5.6-sol", "ultra")).is_ok());
    assert!(validate_request(&request("codex", "gpt-5.6-luna", "max")).is_ok());
    assert!(validate_request(&request("codex", "gpt-5.4", "xhigh")).is_ok());
    assert!(validate_request(&request("codex", "gpt-5.4", "")).is_ok());
}

#[test]
fn runner_accepts_agy_alias_efforts_and_rejects_invalid_levels() {
    for agent in ["agy", "gemini", "antigravity"] {
        assert!(validate_request(&request(agent, "gemini-3.6-flash-high", "high")).is_ok());
        for effort in ["xhigh", "max", "ultra"] {
            assert!(validate_request(&request(agent, "gemini-3.6-flash-high", effort)).is_err());
        }
    }
}

#[test]
fn runner_defaults_omitted_role_and_rejects_unknown_role() {
    let legacy: QueuedTaskRequest =
        serde_json::from_str(r#"{"repository":"/repo","instruction":"ship","agent":"codex"}"#)
            .unwrap();
    assert_eq!(legacy.role, "implementer");

    let mut invalid = request("codex", "", "");
    invalid.role = "manager".to_string();
    assert!(validate_request(&invalid).unwrap_err().contains("planner"));
}

#[test]
fn resume_session_requires_conversation_mode() {
    let mut terminal_resume = request("claude", "", "");
    terminal_resume.mode = "terminal".to_string();
    terminal_resume.resume_session = Some("11111111-1111-1111-1111-111111111111".to_string());
    assert!(validate_request(&terminal_resume).is_err());

    let mut conversation_resume = request("claude", "", "");
    conversation_resume.mode = "conversation".to_string();
    conversation_resume.resume_session = Some("11111111-1111-1111-1111-111111111111".to_string());
    assert!(validate_request(&conversation_resume).is_ok());
}

#[test]
fn resume_session_rejects_agy() {
    let mut agy_resume = request("agy", "", "");
    agy_resume.mode = "conversation".to_string();
    agy_resume.resume_session = Some("11111111-1111-1111-1111-111111111111".to_string());
    assert!(validate_request(&agy_resume).unwrap_err().contains("agy"));
}

#[test]
fn external_resume_requires_matching_explicit_vendor_for_codex() {
    let mut req = request("codex", "", "");
    req.mode = "conversation".into();
    req.resume_session = Some("11111111-1111-4111-8111-111111111111".into());
    assert!(validate_request(&req).is_err());
    req.resume_vendor = Some("codex".into());
    assert!(validate_request(&req).is_ok());
    req.resume_vendor = Some("claude".into());
    assert!(validate_request(&req).is_err());
    req.resume_session = None;
    assert!(validate_request(&req).is_err());
}
