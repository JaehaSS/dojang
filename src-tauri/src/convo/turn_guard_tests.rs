use super::process_cleanup::Survivor;
use super::turn_guard::{
    command_line, completion_system_prompt, guarded_message, shorten_program, SubagentTurnGuard,
    COMMAND_LIMIT, LAST_RESPONSE_MARKER, PROGRAM_LIMIT, TURN_COMPLETION_GUARD,
};
use super::{vendor_command, ConvoEvent, Vendor};

fn survivor(group: u32) -> Survivor {
    Survivor {
        group,
        arguments: None,
    }
}

fn argv(entries: &[&str]) -> Option<Vec<String>> {
    Some(entries.iter().map(|entry| entry.to_string()).collect())
}

fn agent_spawn(id: &str, title: &str) -> ConvoEvent {
    ConvoEvent::ToolUse {
        name: "Agent".into(),
        summary: title.into(),
        tool_id: Some(id.into()),
        parent_id: None,
    }
}

fn bash_call(id: &str, command: &str) -> ConvoEvent {
    ConvoEvent::ToolUse {
        name: "Bash".into(),
        summary: command.into(),
        tool_id: Some(id.into()),
        parent_id: None,
    }
}

fn tool_result(id: &str, summary: &str) -> ConvoEvent {
    ConvoEvent::ToolResult {
        summary: summary.into(),
        is_error: false,
        result_chars: None,
        tool_use_id: Some(id.into()),
        parent_id: None,
    }
}

fn success_result(text: &str) -> ConvoEvent {
    ConvoEvent::Result {
        text: text.into(),
        is_error: false,
        session_id: "session".into(),
        cost_usd: 1.0,
        num_turns: 2,
        tokens_in: 3,
        tokens_out: 4,
    }
}

#[test]
fn async_launch_receipt_remains_pending_until_completion_notification() {
    let mut guard = SubagentTurnGuard::default();
    guard.observe_event(&agent_spawn("tool-agent", "구현 Worker"));
    guard.observe_event(&tool_result(
        "tool-agent",
        "Async agent launched successfully. agentId: internal",
    ));
    assert_eq!(guard.pending_count(), 1);

    let completion = guard
        .observe_raw_line(
        r#"{"type":"user","message":{"role":"user","content":"<task-notification><tool-use-id>tool-agent</tool-use-id><status>completed</status></task-notification>"}}"#,
        )
        .expect("completion event");
    assert_eq!(guard.pending_count(), 0);
    assert!(matches!(
        completion,
        ConvoEvent::ToolResult {
            is_error: false,
            tool_use_id: Some(id),
            ..
        } if id == "tool-agent"
    ));
}

#[test]
fn foreground_agent_result_clears_pending_work() {
    let mut guard = SubagentTurnGuard::default();
    guard.observe_event(&agent_spawn("tool-agent", "조사 Worker"));
    guard.observe_event(&tool_result("tool-agent", "조사 완료"));
    assert_eq!(guard.pending_count(), 0);
}

#[test]
fn success_result_with_pending_worker_becomes_contract_error() {
    let mut guard = SubagentTurnGuard::default();
    guard.observe_event(&agent_spawn("tool-agent", "구현 Worker"));
    guard.observe_event(&tool_result(
        "tool-agent",
        "Async agent launched successfully.",
    ));

    let result = guard.enforce_result(success_result("완료 통지가 오면 검증하겠습니다."));
    match result {
        ConvoEvent::Result { text, is_error, .. } => {
            assert!(is_error);
            assert!(text.contains("완료 회수하기 전에 턴을 종료"));
            assert!(text.contains("구현 Worker"));
            assert!(text.contains("완료 통지가 오면 검증하겠습니다."));
        }
        other => panic!("expected result, got {other:?}"),
    }
}

#[test]
fn async_launch_receipt_is_not_exposed_as_a_completed_tool_result() {
    let guard = SubagentTurnGuard::default();
    let event = tool_result(
        "tool-agent",
        "Async agent launched successfully. agentId: internal",
    );
    assert_eq!(guard.sanitize_event(event), ConvoEvent::Other);
}

#[test]
fn background_shell_stays_pending_until_its_notification_arrives() {
    let mut guard = SubagentTurnGuard::default();
    guard.observe_event(&bash_call("tool-bash", "npm run tauri build"));
    guard.observe_event(&tool_result(
        "tool-bash",
        "Command running in background with ID: b9tplwfcc. Output is being written to: /tmp/out",
    ));
    // Agent만 세던 시절 여기가 0이었다 — 빌드가 통째로 증발해도 경고가 없었다.
    assert_eq!(guard.pending_count(), 1);

    guard
        .observe_raw_line(
            r#"{"type":"user","message":{"role":"user","content":"<task-notification><tool-use-id>tool-bash</tool-use-id><status>stopped</status></task-notification>"}}"#,
        )
        .expect("termination event");
    assert_eq!(guard.pending_count(), 0);
}

#[test]
fn tool_timeout_handoff_to_background_also_stays_pending() {
    let mut guard = SubagentTurnGuard::default();
    guard.observe_event(&bash_call("tool-bash", "cargo test"));
    // 내가 백그라운드를 고른 게 아니라 하니스가 넘긴 경우 — 회수 책임은 똑같다.
    guard.observe_event(&tool_result(
        "tool-bash",
        "Command did not complete within its 600s timeout and was moved to the background (ID: b79exzptf).",
    ));
    assert_eq!(guard.pending_count(), 1);
}

#[test]
fn monitor_stays_pending_but_foreground_shell_does_not() {
    let mut guard = SubagentTurnGuard::default();
    guard.observe_event(&bash_call("tool-fg", "ls"));
    guard.observe_event(&tool_result("tool-fg", "a.rs\nb.rs"));
    assert_eq!(guard.pending_count(), 0);

    guard.observe_event(&ConvoEvent::ToolUse {
        name: "Monitor".into(),
        summary: "빌드 완료 대기".into(),
        tool_id: Some("tool-monitor".into()),
        parent_id: None,
    });
    guard.observe_event(&tool_result(
        "tool-monitor",
        "Monitor started (task bx33prr0x, timeout 3600000ms).",
    ));
    assert_eq!(guard.pending_count(), 1);
}

#[test]
fn background_shell_receipt_stays_visible_unlike_the_agent_one() {
    let guard = SubagentTurnGuard::default();
    // 서브 에이전트 접수는 노이즈라 감춘다.
    assert_eq!(
        guard.sanitize_event(tool_result(
            "tool-agent",
            "Async agent launched successfully."
        )),
        ConvoEvent::Other
    );
    // 셸 접수는 남긴다 — 무엇을 언제 띄웠는지가 사라지면 미완료를 추적할 단서도 없어진다.
    let shell = tool_result(
        "tool-bash",
        "Command running in background with ID: b9tplwfcc.",
    );
    assert_eq!(guard.sanitize_event(shell.clone()), shell);
}

#[test]
fn subagent_model_observation_passes_through_the_guard_untouched() {
    // 가드는 미회수 작업만 센다. 모델 관측은 회수 대상이 아니므로 pending을 흔들지도,
    // 형태가 바뀌지도 않아야 한다 — 여기서 변형되면 카드가 엉뚱한 모델을 이름한다.
    let mut guard = SubagentTurnGuard::default();
    let observation = ConvoEvent::SubagentModel {
        parent_id: "tool-agent".into(),
        model: "claude-opus-5[1m]".into(),
    };
    guard.observe_event(&observation);
    assert_eq!(guard.pending_count(), 0);
    assert_eq!(guard.sanitize_event(observation.clone()), observation);
    assert_eq!(guard.enforce_result(observation.clone()), observation);
}

#[test]
fn completion_guard_uses_system_prompt_for_claude_and_prompt_prefix_for_codex() {
    assert!(TURN_COMPLETION_GUARD
        .contains("Never leave background processes or servers running after the turn."));

    let claude = guarded_message(Vendor::Claude, "사용자 요청");
    assert_eq!(claude.as_ref(), "사용자 요청");
    assert!(completion_system_prompt(Vendor::Claude).is_some());

    let codex = guarded_message(Vendor::Codex, "사용자 요청");
    assert!(codex.starts_with(TURN_COMPLETION_GUARD));
    assert!(codex.ends_with("사용자 요청"));
    assert!(completion_system_prompt(Vendor::Codex).is_none());
}

#[test]
fn vendor_commands_attach_completion_guard_on_every_turn() {
    let claude = vendor_command("claude", Vendor::Claude, "요청", Some("sid"), None);
    let claude_args = claude
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let prompt_index = claude_args
        .iter()
        .position(|arg| arg == "--append-system-prompt")
        .expect("claude system prompt flag");
    assert_eq!(claude_args[prompt_index + 1], TURN_COMPLETION_GUARD);

    let codex = vendor_command("codex", Vendor::Codex, "요청", Some("sid"), None);
    let codex_args = codex
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(codex_args
        .last()
        .unwrap()
        .starts_with(TURN_COMPLETION_GUARD));
}

#[test]
fn surviving_process_fails_the_result_even_when_pending_is_empty() {
    // 하니스가 접수 문구를 남기지 않아 pending에 아무것도 안 잡힌 상황.
    let guard = SubagentTurnGuard::default();
    assert_eq!(guard.pending_count(), 0);

    let event =
        guard.enforce_result_with_survivors(success_result("다 끝냈습니다."), &[survivor(4242)]);

    let ConvoEvent::Result { is_error, text, .. } = event else {
        panic!("Result가 아님");
    };
    assert!(is_error, "생존 프로세스가 있으면 실패로 표시해야 한다");
    assert!(text.contains("4242"), "어떤 그룹이 남았는지 밝혀야 한다");
    assert!(
        text.ends_with(&format!("{LAST_RESPONSE_MARKER}다 끝냈습니다.")),
        "원래 응답은 마커 뒤에 그대로 보존해야 한다 — 대화 뷰가 이 마커에서 자른다"
    );
}

/// 번호만으로는 오탐인지 진탐인지 가릴 수 없다. 명령줄이 경고에 실려야 그 자리에서 갈린다.
#[test]
fn survivor_command_line_lands_in_the_warning() {
    let guard = SubagentTurnGuard::default();

    let event = guard.enforce_result_with_survivors(
        success_result("완료"),
        &[
            Survivor {
                group: 70973,
                arguments: argv(&["node", "/opt/mcp/notion-server.js"]),
            },
            Survivor {
                group: 70980,
                arguments: None,
            },
        ],
    );

    let ConvoEvent::Result { text, .. } = event else {
        panic!("Result가 아님");
    };
    let warning = text.split(LAST_RESPONSE_MARKER).next().unwrap();
    assert!(
        warning.contains("70973 · node /opt/mcp/notion-server.js"),
        "명령줄이 그룹 번호와 함께 나와야 한다: {warning}"
    );
    assert!(
        warning.contains("70980 · (명령줄 확인 불가)"),
        "명령줄을 못 읽어도 그룹은 알려야 한다: {warning}"
    );
    assert!(
        warning.contains("2건"),
        "생존자 수는 그대로 센다: {warning}"
    );
}

#[test]
fn no_survivors_keeps_the_result_successful() {
    let guard = SubagentTurnGuard::default();
    let event = guard.enforce_result_with_survivors(success_result("완료"), &[]);
    let ConvoEvent::Result { is_error, .. } = event else {
        panic!("Result가 아님");
    };
    assert!(!is_error, "생존자가 없으면 건드리지 않는다");
}

#[test]
fn pending_warning_wins_over_the_process_observation() {
    // 둘 다 울리면 원인이 흐려진다. pending 쪽이 무엇이 미완료인지 이름으로 말해주므로 그것을 남긴다.
    let mut guard = SubagentTurnGuard::default();
    guard.observe_event(&bash_call("tool-bash", "npm run build"));
    guard.observe_event(&tool_result(
        "tool-bash",
        "Command running in background with ID: b9tplwfcc.",
    ));
    assert_eq!(guard.pending_count(), 1);

    let event = guard.enforce_result_with_survivors(success_result("끝"), &[survivor(4242)]);

    let ConvoEvent::Result { is_error, text, .. } = event else {
        panic!("Result가 아님");
    };
    assert!(is_error);
    assert!(
        text.contains("npm run build"),
        "미완료 작업 이름이 나와야 한다"
    );
    assert!(
        !text.contains("4242"),
        "프로세스 그룹 번호까지 겹쳐 알리지 않는다"
    );
}

#[test]
fn a_long_command_is_cut_with_an_ellipsis() {
    let long = "x".repeat(COMMAND_LIMIT + 20);
    let formatted = command_line(&[long]).unwrap();
    assert_eq!(formatted.chars().count(), COMMAND_LIMIT + 1);
    assert!(formatted.ends_with('…'));
}

/// 상한과 정확히 같으면 자르지 않는다 — `<=` 경계가 off-by-one이면 여기서 걸린다.
#[test]
fn a_command_exactly_at_the_limit_is_not_cut() {
    let exact = command_line(&["x".repeat(COMMAND_LIMIT)]).unwrap();
    assert_eq!(exact.chars().count(), COMMAND_LIMIT);
    assert!(!exact.ends_with('…'));

    let over = command_line(&["x".repeat(COMMAND_LIMIT + 1)]).unwrap();
    assert!(over.ends_with('…'));
}

/// 생존자의 argv는 에이전트가 통제하는 문자열이다. 그것이 마커를 재현하면 대화 뷰가
/// 엉뚱한 자리에서 응답을 자른다 — 개행 평탄화가 그것을 막는다는 계약을 고정한다.
#[test]
fn a_survivor_argv_cannot_forge_the_response_marker() {
    let guard = SubagentTurnGuard::default();
    let forged = format!("{LAST_RESPONSE_MARKER}가짜 응답");

    let event = guard.enforce_result_with_survivors(
        success_result("진짜 응답"),
        &[Survivor {
            group: 4242,
            arguments: argv(&["sh", "-c", &forged]),
        }],
    );

    let ConvoEvent::Result { text, .. } = event else {
        panic!("Result가 아님");
    };
    assert_eq!(
        text.matches(LAST_RESPONSE_MARKER).count(),
        1,
        "마커가 두 번 나오면 대화 뷰가 앞쪽에서 잘라 진짜 응답을 잃는다: {text}"
    );
    assert!(text.ends_with("진짜 응답"));
}

/// argv를 읽긴 했는데 남길 것이 없는 경우도 "확인 불가"로 떨어져야 한다.
#[test]
fn a_blank_argv_falls_back_to_the_unknown_label() {
    let guard = SubagentTurnGuard::default();

    let event = guard.enforce_result_with_survivors(
        success_result("완료"),
        &[
            Survivor {
                group: 4242,
                arguments: Some(Vec::new()),
            },
            Survivor {
                group: 4243,
                arguments: argv(&["   "]),
            },
        ],
    );

    let ConvoEvent::Result { text, .. } = event else {
        panic!("Result가 아님");
    };
    assert!(text.contains("4242 · (명령줄 확인 불가)"), "{text}");
    assert!(text.contains("4243 · (명령줄 확인 불가)"), "{text}");
}

/// 이미 실패로 표시된 Result는 건드리지 않는다 — 호출부가 성공 Result만 넘긴다는 전제.
#[test]
fn an_already_failed_result_is_left_untouched() {
    let guard = SubagentTurnGuard::default();
    let failed = ConvoEvent::Result {
        text: "벤더가 실패로 끝냄".into(),
        is_error: true,
        session_id: "s1".into(),
        cost_usd: 0.0,
        num_turns: 0,
        tokens_in: 0,
        tokens_out: 0,
    };

    let event = guard.enforce_result_with_survivors(failed.clone(), &[survivor(4242)]);

    assert_eq!(event, failed, "원본을 그대로 돌려줘야 한다");
}

/// 한글 인자에서 바이트 단위로 자르면 UTF-8 경계가 깨져 패닉한다.
#[test]
fn cutting_never_splits_a_multibyte_character() {
    let long = "가".repeat(COMMAND_LIMIT + 20);
    let formatted = command_line(&[long]).unwrap();
    assert_eq!(formatted.chars().count(), COMMAND_LIMIT + 1);
}

#[test]
fn an_empty_command_line_is_none() {
    assert_eq!(command_line(&[]), None);
    assert_eq!(command_line(&["  ".to_string()]), None);
}

/// 개행이 섞이면 그룹당 한 줄인 경고 목록이 무너진다.
#[test]
fn newlines_inside_arguments_are_flattened() {
    let formatted =
        command_line(&["sh".to_string(), "-c".to_string(), "a\nb\tc".to_string()]).unwrap();
    assert_eq!(formatted, "sh -c a b c");
}

/// 긴 인터프리터 경로가 상한을 다 먹으면 정작 무엇을 실행하는지가 잘린다.
#[test]
fn a_long_program_path_shrinks_but_its_arguments_do_not() {
    let formatted = command_line(&[
        "/Applications/Xcode.app/Contents/Developer/Library/Frameworks/Python3.framework/Versions/3.9/Resources/Python.app/Contents/MacOS/Python".to_string(),
        "-c".to_string(),
        "import os; os.setsid()".to_string(),
    ])
    .unwrap();
    assert_eq!(formatted, ".../MacOS/Python -c import os; os.setsid()");
}

/// 짧은 경로는 건드리지 않는다 — 줄여봐야 이득이 없고 어디 있는 바이너리인지만 흐려진다.
#[test]
fn a_short_program_path_is_left_alone() {
    assert_eq!(shorten_program("/usr/bin/python3"), "/usr/bin/python3");
    assert_eq!(shorten_program("/bin/sh"), "/bin/sh");
    assert_eq!(shorten_program("node"), "node");
}

/// 경로가 기형이어도 축약이 패닉하지 않는다 — argv[0]은 프로세스가 정하는 값이라 신뢰할 수 없다.
#[test]
fn a_malformed_program_path_does_not_panic() {
    let long = "/".repeat(PROGRAM_LIMIT + 10);
    assert_eq!(shorten_program(&long), long);
    let trailing = format!("/{}/", "a".repeat(PROGRAM_LIMIT + 10));
    assert_eq!(shorten_program(&trailing), trailing);
    let doubled = format!("/a//b/{}", "c".repeat(PROGRAM_LIMIT));
    assert_eq!(
        shorten_program(&doubled),
        format!(".../b/{}", "c".repeat(PROGRAM_LIMIT))
    );
}

/// 마커는 Rust와 TS에 손으로 복제돼 있고, 계약은 여태 **양쪽 주석으로만** 존재했다.
/// 한쪽만 바꾸면 양쪽 테스트가 전부 초록인 채 사용자 화면에 응답 전문이 두 번 그려진다
/// (이슈 [#119](https://github.com/JaehaSS/praxis/issues/119)).
///
/// 프론트에서 Rust를 읽는 대신 여기서 TS를 읽는다 — 프론트에는 `@types/node`가 없어
/// `node:fs`를 들이면 의존성이 하나 늘고 `tsc --noEmit`이 깨진다. Rust는 `std::fs`가
/// 기본이라 계약을 고정하는 데 아무것도 더 필요하지 않다.
#[test]
fn the_frontend_marker_is_the_same_string() {
    const FRONTEND: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../src/components/ide/ConversationView.tsx"
    );
    let source = std::fs::read_to_string(FRONTEND)
        .unwrap_or_else(|err| panic!("{FRONTEND}를 읽지 못했다: {err}"));

    // 선언은 `export const GUARD_LAST_RESPONSE_MARKER = "...";` 한 줄이다. 이 문자열에는
    // 이스케이프된 따옴표가 없으므로 다음 `"`까지가 리터럴 전체다.
    let literal = source
        .split("GUARD_LAST_RESPONSE_MARKER = \"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("GUARD_LAST_RESPONSE_MARKER 선언을 찾지 못했다 — 이름이 바뀌었나?");

    assert_eq!(
        literal.replace("\\n", "\n"),
        LAST_RESPONSE_MARKER,
        "프론트의 마커가 백엔드와 갈라졌다. 두 상수는 문자 단위로 같아야 하며, \
         다르면 대화 뷰가 가드 경고를 자르지 못해 응답 전문이 중복 노출된다."
    );
}

#[test]
fn claude_question_contract_is_one_stable_system_argument_not_user_history() {
    let mut prompts = Vec::new();
    for resume in [None, Some("existing-session")] {
        let command = super::vendor_command_with_contract(
            "claude", Vendor::Claude, "사용자 요청", resume, None, None, &[], None, None, true,
        );
        let args: Vec<_> = command.get_args().map(|s| s.to_string_lossy().into_owned()).collect();
        assert_eq!(args[1], "사용자 요청");
        assert_eq!(args.iter().filter(|a| *a == "--append-system-prompt").count(), 1);
        let index = args.iter().position(|a| a == "--append-system-prompt").unwrap();
        let prompt = args[index + 1].clone();
        assert_eq!(prompt.matches(TURN_COMPLETION_GUARD).count(), 1);
        assert_eq!(prompt.matches(super::question_local::TOOL_INSTRUCTIONS).count(), 1);
        prompts.push(prompt);
    }
    assert_eq!(prompts[0], prompts[1]);
}

#[test]
fn generic_codex_preserves_configured_developer_instructions() {
    for resume in [None, Some("existing-session")] {
        let command = vendor_command("codex", Vendor::Codex, "사용자 요청", resume, None);
        let args: Vec<_> = command.get_args().map(|s| s.to_string_lossy().into_owned()).collect();
        assert!(!args.iter().any(|s| s.contains("developer_instructions=")));
        assert!(args.last().unwrap().starts_with(TURN_COMPLETION_GUARD));
        assert!(args.last().unwrap().ends_with("사용자 요청"));
    }
}

/// claude 2.1.28x는 종료 알림을 `system/task_notification` 한 줄로 낸다(2026-09-26 실측).
/// 문자열 `<task-notification>`만 읽던 동안 이 형태를 놓쳐, 회수가 끝난 작업이 미완료로 찍혔다.
#[test]
fn structured_task_notification_clears_pending_and_surfaces_completion() {
    let mut guard = SubagentTurnGuard::default();
    guard.observe_event(&agent_spawn("tool-agent", "조사 Worker"));
    guard.observe_event(&tool_result(
        "tool-agent",
        "Async agent launched successfully. agentId: internal",
    ));
    assert_eq!(guard.pending_count(), 1);

    let completion = guard
        .observe_raw_line(
            r#"{"type":"system","subtype":"task_notification","task_id":"a1d772ce","tool_use_id":"tool-agent","status":"completed","output_file":"/tmp/a.output","summary":"Agent \"조사 Worker\" finished"}"#,
        )
        .expect("completion event");
    assert_eq!(guard.pending_count(), 0);
    assert_eq!(guard.cut_count(), 0);
    assert!(matches!(
        completion,
        ConvoEvent::ToolResult {
            is_error: false,
            tool_use_id: Some(id),
            ..
        } if id == "tool-agent"
    ));
    assert!(
        guard.enforce_result(success_result("끝")) == success_result("끝"),
        "회수가 끝났으면 성공 그대로여야 한다"
    );
}

#[test]
fn a_task_stopped_after_the_answer_counts_as_cut_work() {
    // 모델이 답을 마친 뒤 하니스가 종료하면서 끊은 작업 — 결과를 아무도 읽지 못했다.
    let mut guard = SubagentTurnGuard::default();
    guard.observe_event(&bash_call("tool-bash", "npm run tauri build"));
    guard.observe_event(&tool_result(
        "tool-bash",
        "Command running in background with ID: brtxt6n39.",
    ));
    guard.observe_event(&success_result("기다리겠습니다"));
    guard
        .observe_raw_line(
            r#"{"type":"system","subtype":"task_notification","task_id":"brtxt6n39","tool_use_id":"tool-bash","status":"stopped"}"#,
        )
        .expect("termination event");
    assert_eq!(guard.pending_count(), 0);
    assert_eq!(guard.cut_count(), 1);

    let ConvoEvent::Result { is_error, text, .. } = guard.enforce_result(success_result("기다리겠습니다"))
    else {
        panic!("Result가 아님");
    };
    assert!(is_error);
    assert!(text.contains("끊은 작업"));
    assert!(text.contains("npm run tauri build"));
    assert!(text.contains("1건"));
}

#[test]
fn a_task_stopped_by_the_agent_mid_turn_is_not_cut_work() {
    // TaskStop으로 에이전트가 스스로 정리한 경우 — 응답 전이므로 하니스가 끊은 것이 아니다.
    let mut guard = SubagentTurnGuard::default();
    guard.observe_event(&bash_call("tool-bash", "sleep 280; echo timer"));
    guard.observe_event(&tool_result(
        "tool-bash",
        "Command running in background with ID: beibhu7k5.",
    ));
    guard
        .observe_raw_line(
            r#"{"type":"system","subtype":"task_notification","task_id":"beibhu7k5","tool_use_id":"tool-bash","status":"stopped"}"#,
        )
        .expect("termination event");
    assert_eq!(guard.pending_count(), 0);
    assert_eq!(guard.cut_count(), 0);
    assert_eq!(guard.enforce_result(success_result("끝")), success_result("끝"));
}

#[test]
fn a_new_tool_call_after_the_answer_reopens_the_turn() {
    // 알림으로 모델이 다시 불려 도구를 부르면 그 뒤의 stopped는 다시 에이전트의 것이다.
    let mut guard = SubagentTurnGuard::default();
    guard.observe_event(&bash_call("tool-bash", "sleep 280; echo timer"));
    guard.observe_event(&tool_result(
        "tool-bash",
        "Command running in background with ID: beibhu7k5.",
    ));
    guard.observe_event(&success_result("기다리겠습니다"));
    guard.observe_event(&ConvoEvent::ToolUse {
        name: "TaskStop".into(),
        summary: "beibhu7k5".into(),
        tool_id: Some("tool-stop".into()),
        parent_id: None,
    });
    guard
        .observe_raw_line(
            r#"{"type":"system","subtype":"task_notification","task_id":"beibhu7k5","tool_use_id":"tool-bash","status":"stopped"}"#,
        )
        .expect("termination event");
    assert_eq!(guard.cut_count(), 0);
}
