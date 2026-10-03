use super::*;

fn answer(id: &str, option_id: Option<&str>, text: Option<&str>) -> Answer {
    Answer {
        question_id: id.into(),
        option_id: option_id.map(Into::into),
        text: text.map(Into::into),
    }
}

#[test]
fn native_questions_keep_labels_as_wire_answers_and_allow_many_choices() {
    let params = json!({"isBlocking":false,"questions":[{"id":"colour","header":"Colour","question":"Choose","isOther":true,"isSecret":false,"options":[{"label":"red","description":""},{"label":"blue","description":""},{"label":"green","description":""},{"label":"violet","description":""}]}]});
    let normalized = normalize(TOOL_INPUT, &params).unwrap();
    assert!(!normalized.info.blocking);
    assert_eq!(normalized.questions.questions[0].options.len(), 4);
    assert_eq!(
        response(
            TOOL_INPUT,
            &params,
            &[answer("colour", Some("option:0:3"), None)]
        )
        .unwrap(),
        json!({"answers":{"colour":{"answers":["violet"]}}})
    );
}

#[test]
fn native_questions_without_options_are_free_text_even_without_is_other() {
    let params = json!({"isBlocking":false,"questions":[{"id":"note","header":"Note","question":"Explain","options":null}]});
    let normalized = normalize(TOOL_INPUT, &params).unwrap();
    assert!(normalized.questions.questions[0].allow_free_text);
    assert_eq!(
        response(
            TOOL_INPUT,
            &params,
            &[answer("note", None, Some("details"))]
        )
        .unwrap(),
        json!({"answers":{"note":{"answers":["details"]}}})
    );
    let empty_options = json!({"isBlocking":false,"questions":[{"id":"note","header":"Note","question":"Explain","options":[]}]});
    assert!(
        normalize(TOOL_INPUT, &empty_options)
            .unwrap()
            .questions
            .questions[0]
            .allow_free_text
    );
    assert_eq!(
        response(
            TOOL_INPUT,
            &empty_options,
            &[answer("note", None, Some("details"))]
        )
        .unwrap(),
        json!({"answers":{"note":{"answers":["details"]}}})
    );
}

#[test]
fn denials_and_permissions_never_create_session_grants() {
    let command = json!({"itemId":"i","threadId":"t","turnId":"u","startedAtMs":1});
    assert_eq!(
        response(
            COMMAND_APPROVAL,
            &command,
            &[answer("decision", Some("decline"), None)]
        )
        .unwrap(),
        json!({"decision":"decline"})
    );
    let permissions = json!({"itemId":"i","threadId":"t","turnId":"u","startedAtMs":1,"cwd":"/tmp","permissions":{"network":{"enabled":true}}});
    assert_eq!(
        response(
            PERMISSIONS_APPROVAL,
            &permissions,
            &[answer("decision", Some("accept"), None)]
        )
        .unwrap(),
        json!({"permissions":{"network":{"enabled":true}},"scope":"turn"})
    );
    assert_eq!(
        response(
            PERMISSIONS_APPROVAL,
            &permissions,
            &[answer("decision", Some("decline"), None)]
        )
        .unwrap(),
        json!({"permissions":{},"scope":"turn"})
    );
}

#[test]
fn command_approval_intersects_server_offered_decisions() {
    let params = json!({"availableDecisions":["decline"]});
    let normalized = normalize(COMMAND_APPROVAL, &params).unwrap();
    assert_eq!(normalized.questions.questions[0].options.len(), 1);
    assert_eq!(normalized.questions.questions[0].options[0].id, "decline");
    assert!(response(
        COMMAND_APPROVAL,
        &params,
        &[answer("decision", Some("accept"), None)]
    )
    .is_err());
    assert_eq!(
        response(
            COMMAND_APPROVAL,
            &params,
            &[answer("decision", Some("decline"), None)]
        )
        .unwrap(),
        json!({"decision":"decline"})
    );
    assert_eq!(
        normalize(COMMAND_APPROVAL, &json!({"availableDecisions":null}))
            .unwrap()
            .questions
            .questions[0]
            .options
            .len(),
        3
    );
}

#[test]
fn approval_details_show_the_exact_requested_scope() {
    let command = normalize(
        COMMAND_APPROVAL,
        &json!({"command":"git status","cwd":"/repo","reason":"inspect"}),
    )
    .unwrap();
    assert!(command.info.details.contains("명령: \"git status\""));
    assert!(command.info.details.contains("작업 폴더: \"/repo\""));
    let permissions = normalize(
        PERMISSIONS_APPROVAL,
        &json!({"permissions":{"fileSystem":{"write":["/repo"]}}}),
    )
    .unwrap();
    assert!(permissions.info.details.contains("\"/repo\""));
}

#[test]
fn malformed_or_secret_native_requests_are_rejected() {
    assert!(normalize(
        TOOL_INPUT,
        &json!({"isBlocking":true,"questions":[{"id":"x","question":"secret","isSecret":true}]})
    )
    .is_err());
    assert!(normalize(MCP_ELICITATION, &json!({"mode":"form","message":"x","requestedSchema":{"type":"object","properties":{"password":{"type":"string","format":"password"}}}})).is_err());
    assert!(normalize(MCP_ELICITATION, &json!({"mode":"form","message":"x","requestedSchema":{"type":"object","properties":{"nested":{"type":"object"}}}})).is_err());
}

#[test]
fn mcp_form_coerces_only_supported_primitive_schema() {
    let params = json!({"mode":"form","message":"Configure","requestedSchema":{"type":"object","required":["count","enabled"],"properties":{"count":{"type":"integer"},"enabled":{"type":"boolean"},"style":{"type":"string","enum":["small","large"]},"note":{"type":"string"}}}});
    let response = response(
        MCP_ELICITATION,
        &params,
        &[
            answer("count", None, Some("3")),
            answer("enabled", Some("true"), None),
            answer("style", Some("enum:1"), None),
            answer("note", Some("skip"), None),
            answer(FORM_ACTION_ID, Some("accept"), None),
        ],
    )
    .unwrap();
    assert_eq!(
        response,
        json!({"action":"accept","content":{"count":3,"enabled":true,"style":"large"}})
    );
}

#[test]
fn mcp_form_decline_or_cancel_needs_only_the_reserved_action() {
    let params = json!({"mode":"form","message":"Configure","requestedSchema":{"type":"object","required":["value"],"properties":{"value":{"type":"string"}}}});
    let normalized = normalize(MCP_ELICITATION, &params).unwrap();
    assert!(normalized
        .questions
        .questions
        .iter()
        .any(|question| question.id == FORM_ACTION_ID));
    let declined = vec![answer(FORM_ACTION_ID, Some("decline"), None)];
    assert!(is_negative_form_response(MCP_ELICITATION, &params, &declined).unwrap());
    assert_eq!(
        response(MCP_ELICITATION, &params, &declined).unwrap(),
        json!({"action":"decline","content":null})
    );
}
