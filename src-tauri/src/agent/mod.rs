//! 리드 에이전트 선택 (pluggable) — Praxis는 **에이전트 비종속** 오케스트레이터다.
//! 각 에이전트의 "인터랙티브 + 초기 프롬프트" 호출법을 매핑하고, 임의 CLI를 위한 커스텀 명령도 지원.
//!
//! 순수 로직(PATH 비의존): bin **이름**만 반환하고, 절대경로 해석은 호출측(`reviewer::which`)이 한다
//! (portable_pty가 프로그램명을 PATH로 해석하지 않으므로 스폰 시 절대경로가 필요).

mod reasoning_effort;
pub mod service_tier;
mod role;

pub(crate) use reasoning_effort::{
    agy_effort_args, claude_effort_args, reasoning_effort_config_arg,
};
pub use reasoning_effort::{reasoning_effort_override, reasoning_effort_override_for_model};
pub use role::{normalize_role, normalize_role_or_default, DEFAULT_ROLE, ROLES};

/// 프리셋 에이전트 키 + 표시명 (프론트 AgentPicker와 동기 유지).
pub const PRESETS: &[(&str, &str)] = &[
    ("claude", "Claude Code"),
    ("codex", "Codex"),
    ("agy", "Antigravity · Gemini"),
];

/// `agent`(trim 후)가 현재 지원하는 프리셋 중 하나인지 — 모델 설정 커맨드의 벤더 검증에 사용.
pub fn is_preset(agent: &str) -> bool {
    let agent = agent.trim();
    PRESETS.iter().any(|(key, _)| *key == agent)
}

/// 프리셋 에이전트별 모델 지정 플래그 (지시문/서브커맨드 앞에 삽입).
/// codex만 `-m`, claude/agy(gemini 별칭 포함)는 `--model`
/// (agy CLI는 `-m` 단축 플래그가 없어 `flags provided but not defined`로 즉사한다).
fn model_flag(agent: &str) -> &'static str {
    match agent {
        "codex" => "-m",
        _ => "--model",
    }
}

/// 에이전트 키(프리셋) 또는 커스텀 명령 문자열 + 지시문 → (실행 bin 이름, 인자).
///
/// - 프리셋: 인터랙티브 세션을 **지시문으로 시드**.
/// - 커스텀: 공백 분할 → 첫 토큰=bin, 나머지=인자. `{prompt}` 자리표시자가 있으면 그 자리에 지시문을
///   치환, 없으면 끝에 append.
/// - `None` = 빈 입력 등으로 결정 불가 → 호출측이 셸로 폴백.
/// - `model`: 프리셋에서만 적용(설정된 벤더 기본 모델). `None`/빈 문자열이면 기존과 동일.
pub fn agent_args(
    agent: &str,
    instruction: &str,
    model: Option<&str>,
) -> Option<(String, Vec<String>)> {
    agent_args_with_effort(agent, instruction, model, None)
}

pub fn agent_args_with_effort(
    agent: &str,
    instruction: &str,
    model: Option<&str>,
    reasoning_effort: Option<&str>,
) -> Option<(String, Vec<String>)> {
    let instr = instruction.trim();
    let agent = agent.trim();
    if agent.is_empty() {
        return None;
    }

    // 프리셋: (bin, 선행 플래그, 지시문 시드 여부)
    let preset: Option<(&str, &[&str], bool)> = match agent {
        "claude" => Some(("claude", &[][..], true)),
        "codex" => Some(("codex", &[][..], true)),
        // gemini는 저장된 기존 작업과의 호환 별칭이며 새 선택지에는 노출하지 않는다.
        "gemini" | "agy" | "antigravity" => Some(("agy", &["-i"][..], true)),
        _ => None,
    };
    if let Some((bin, flags, seed)) = preset {
        let mut args: Vec<String> = flags.iter().map(|s| (*s).to_string()).collect();
        let model = model.map(str::trim).filter(|m| !m.is_empty());
        if let Some(m) = model {
            args.push(model_flag(agent).to_string());
            args.push(m.to_string());
        }
        if agent == "codex" {
            if let Some(config) = reasoning_effort_config_arg(reasoning_effort) {
                args.push("-c".to_string());
                args.push(config);
            }
        }
        if agent == "claude" {
            if let Some([flag, value]) = claude_effort_args(reasoning_effort) {
                args.push(flag);
                args.push(value);
            }
        }
        if matches!(agent, "agy" | "gemini" | "antigravity") {
            if let Some([flag, value]) = agy_effort_args(reasoning_effort) {
                args.push(flag);
                args.push(value);
            }
        }
        if seed && !instr.is_empty() {
            // `-`로 시작하는 지시문이 옵션으로 읽히지 않게 옵션을 끝낸다. agy `-i`는 값을 받는 플래그라 제외.
            if matches!(agent, "claude" | "codex") {
                args.push("--".to_string());
            }
            args.push(instr.to_string());
        }
        return Some((bin.to_string(), args));
    }

    // 커스텀 명령
    let mut tokens = agent.split_whitespace();
    let bin = tokens.next()?.to_string();
    let mut args: Vec<String> = Vec::new();
    let mut placed = false;
    for t in tokens {
        if t == "{prompt}" {
            if !instr.is_empty() {
                args.push(instr.to_string());
            }
            placed = true;
        } else {
            args.push(t.to_string());
        }
    }
    if !placed && !instr.is_empty() {
        args.push(instr.to_string());
    }
    Some((bin, args))
}

/// 헤드리스 **자율 수행** 호출 (앙상블용) — 에이전트가 worktree에서 지시문을 스스로 실행하고
/// 편집/명령을 **자동 승인**한 뒤 종료(→ AwaitingReview, diff 생성). 인터랙티브와 별개 매핑.
///
/// ⚠️ 자동 승인 플래그는 격리 worktree 안에서만 의도된 것.
/// 커스텀은 인터랙티브와 동일 규칙(자율 플래그는 사용자가 명령에 포함).
/// `model`: 프리셋에서만 적용. `None`/빈 문자열이면 기존과 동일.
pub fn headless_args(
    agent: &str,
    instruction: &str,
    model: Option<&str>,
) -> Option<(String, Vec<String>)> {
    headless_args_with_effort(agent, instruction, model, None, None)
}

/// task를 가리키는 claude 세션 이름. 화면에 보이는 task id와 같은 번호를 쓰므로 본 작업을
/// 그대로 `SendMessage`의 수신자로 적을 수 있다. 앙상블 후보는 각자 별도 task row라 여기서
/// 갈린다(같은 `ensemble` 태그를 공유할 뿐 id는 다르다).
pub fn session_name(task_id: i64) -> String {
    format!("task-{task_id}")
}

/// `session_name`: claude 세션 레지스트리(`~/.claude/sessions/<pid>.json`)에 등록될 표시 이름.
/// 미지정이면 claude가 cwd에서 이름을 파생하므로 **같은 worktree의 task가 전부 같은 이름**을
/// 받아 `SendMessage`의 수신자가 모호해진다. task id를 넣어 주소를 유일하게 만든다.
pub fn headless_args_with_effort(
    agent: &str,
    instruction: &str,
    model: Option<&str>,
    reasoning_effort: Option<&str>,
    session_name: Option<&str>,
) -> Option<(String, Vec<String>)> {
    let instr = instruction.trim();
    let agent = agent.trim();
    if agent.is_empty() || instr.is_empty() {
        return None;
    }
    let v = |parts: &[&str]| -> Vec<String> { parts.iter().map(|s| s.to_string()).collect() };
    let model = model.map(str::trim).filter(|m| !m.is_empty());
    let mapped: Option<(&str, Vec<String>)> = match agent {
        "claude" => {
            let mut args = v(&["-p", instr, "--dangerously-skip-permissions"]);
            // `-n`은 claude 전용 — codex/agy에는 대응 플래그가 없어 붙이면 즉사한다.
            if let Some(name) = session_name.map(str::trim).filter(|n| !n.is_empty()) {
                args.extend(v(&["-n", name]));
            }
            if let Some(m) = model {
                args.extend(v(&["--model", m]));
            }
            if let Some([flag, value]) = claude_effort_args(reasoning_effort) {
                args.push(flag);
                args.push(value);
            }
            Some(("claude", args))
        }
        "codex" => {
            let mut args = v(&["exec"]);
            if let Some(m) = model {
                args.extend(v(&["-m", m]));
            }
            if let Some(config) = reasoning_effort_config_arg(reasoning_effort) {
                args.push("-c".to_string());
                args.push(config);
            }
            args.extend(v(&[
                "--skip-git-repo-check",
                "--dangerously-bypass-approvals-and-sandbox",
                instr,
            ]));
            Some(("codex", args))
        }
        // gemini는 저장된 기존 작업과의 호환 별칭이며 Antigravity로 실행한다.
        // agy는 `-m` 단축 플래그가 없다(`--model`만 지원) — `-m`이면 플래그 오류로 즉사.
        // `--print-timeout`: print 모드 자체 타임아웃 기본 5분 — 긴 헤드리스 턴이 무출력으로
        // 죽지 않게 상향(convo와 동일 근거).
        "gemini" | "agy" | "antigravity" => {
            let mut args = v(&[
                "-p",
                instr,
                "--dangerously-skip-permissions",
                "--print-timeout",
                crate::convo::AGY_PRINT_TIMEOUT,
            ]);
            if let Some(m) = model {
                args.extend(v(&["--model", m]));
            }
            if let Some([flag, value]) = agy_effort_args(reasoning_effort) {
                args.push(flag);
                args.push(value);
            }
            Some(("agy", args))
        }
        _ => None,
    };
    if let Some((bin, args)) = mapped {
        return Some((bin.to_string(), args));
    }
    // 커스텀: 인터랙티브와 동일(자율 플래그는 사용자 명령에 포함된다고 가정, model 미적용).
    agent_args_with_effort(agent, instruction, None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_preset_accepts_current_presets_rejects_deprecated_gemini() {
        for (key, _) in PRESETS {
            assert!(is_preset(key), "{key} should be a preset");
        }
        assert!(is_preset("  claude  "), "trim 후 매칭");
        assert!(!is_preset("gemini"));
        assert!(!is_preset("crush"));
        assert!(!is_preset("mybin --foo"));
        assert!(!is_preset(""));
    }

    #[test]
    fn presets_seed_instruction_as_trailing_positional() {
        assert_eq!(
            agent_args("claude", "do x", None),
            Some(("claude".into(), vec!["--".into(), "do x".into()]))
        );
        assert_eq!(
            agent_args("codex", "-x do", None),
            Some(("codex".into(), vec!["--".into(), "-x do".into()]))
        );
    }

    #[test]
    fn deprecated_gemini_and_antigravity_use_agy_interactive_cli() {
        assert_eq!(
            agent_args("gemini", "do x", None),
            Some(("agy".into(), vec!["-i".into(), "do x".into()]))
        );
        assert_eq!(
            agent_args("agy", "do x", None),
            Some(("agy".into(), vec!["-i".into(), "do x".into()]))
        );
        // antigravity 별칭도 agy로.
        assert_eq!(agent_args("antigravity", "y", None).unwrap().0, "agy");
    }

    #[test]
    fn empty_instruction_yields_no_seed_arg() {
        assert_eq!(
            agent_args("claude", "   ", None),
            Some(("claude".into(), vec![]))
        );
    }

    #[test]
    fn custom_command_appends_prompt_at_end() {
        assert_eq!(
            agent_args("crush", "do x", None),
            Some(("crush".into(), vec!["do x".into()]))
        );
        assert_eq!(
            agent_args("mybin --foo", "do x", None),
            Some(("mybin".into(), vec!["--foo".into(), "do x".into()]))
        );
    }

    #[test]
    fn custom_placeholder_is_substituted_in_place() {
        assert_eq!(
            agent_args("mybin -p {prompt} --end", "do x", None),
            Some((
                "mybin".into(),
                vec!["-p".into(), "do x".into(), "--end".into()]
            ))
        );
    }

    #[test]
    fn empty_agent_is_none() {
        assert_eq!(agent_args("   ", "do x", None), None);
    }

    #[test]
    fn headless_uses_autonomous_auto_approve_flags() {
        assert_eq!(
            headless_args("claude", "fix bug", None),
            Some((
                "claude".into(),
                vec![
                    "-p".into(),
                    "fix bug".into(),
                    "--dangerously-skip-permissions".into()
                ]
            ))
        );
        assert_eq!(
            headless_args("codex", "fix bug", None),
            Some((
                "codex".into(),
                vec![
                    "exec".into(),
                    "--skip-git-repo-check".into(),
                    "--dangerously-bypass-approvals-and-sandbox".into(),
                    "fix bug".into()
                ]
            ))
        );
        assert_eq!(
            headless_args("gemini", "fix bug", None),
            Some((
                "agy".into(),
                vec![
                    "-p".into(),
                    "fix bug".into(),
                    "--dangerously-skip-permissions".into(),
                    "--print-timeout".into(),
                    crate::convo::AGY_PRINT_TIMEOUT.into(),
                ]
            ))
        );
        // agy print 모드 기본 타임아웃(5m)이 긴 헤드리스 턴을 죽이지 않게 항상 상향 주입.
        let agy_args = headless_args("agy", "fix bug", None).unwrap().1;
        assert!(agy_args.contains(&"--dangerously-skip-permissions".to_string()));
        assert_eq!(
            &agy_args[agy_args.len() - 2..],
            &["--print-timeout", crate::convo::AGY_PRINT_TIMEOUT]
        );
    }

    #[test]
    fn headless_requires_instruction() {
        // 헤드리스는 시드 지시문이 없으면 의미 없음 → None.
        assert_eq!(headless_args("claude", "  ", None), None);
    }

    #[test]
    fn headless_custom_falls_back_to_interactive_rule() {
        assert_eq!(
            headless_args("crush", "fix bug", None),
            Some(("crush".into(), vec!["fix bug".into()]))
        );
    }

    #[test]
    fn headless_antigravity_alias_maps_to_agy() {
        let (bin, args) = headless_args("antigravity", "fix bug", None).unwrap();
        assert_eq!(bin, "agy");
        assert!(args.contains(&"--dangerously-skip-permissions".to_string()));
    }

    #[test]
    fn model_none_or_empty_yields_unchanged_args() {
        assert_eq!(
            agent_args("claude", "do x", None),
            agent_args("claude", "do x", Some(""))
        );
        assert_eq!(
            agent_args("claude", "do x", Some("  ")),
            agent_args("claude", "do x", None)
        );
        assert_eq!(
            headless_args("codex", "fix bug", None),
            headless_args("codex", "fix bug", Some(""))
        );
    }

    #[test]
    fn agent_args_claude_model_precedes_positional() {
        assert_eq!(
            agent_args("claude", "do x", Some("opus")),
            Some((
                "claude".into(),
                vec!["--model".into(), "opus".into(), "--".into(), "do x".into()]
            ))
        );
    }

    #[test]
    fn headless_claude_model_after_positional() {
        assert_eq!(
            headless_args("claude", "fix bug", Some("opus")),
            Some((
                "claude".into(),
                vec![
                    "-p".into(),
                    "fix bug".into(),
                    "--dangerously-skip-permissions".into(),
                    "--model".into(),
                    "opus".into()
                ]
            ))
        );
    }

    #[test]
    fn headless_codex_model_after_exec_subcommand() {
        assert_eq!(
            headless_args("codex", "fix bug", Some("o3")),
            Some((
                "codex".into(),
                vec![
                    "exec".into(),
                    "-m".into(),
                    "o3".into(),
                    "--skip-git-repo-check".into(),
                    "--dangerously-bypass-approvals-and-sandbox".into(),
                    "fix bug".into()
                ]
            ))
        );
    }

    #[test]
    fn codex_interactive_reasoning_effort_is_a_config_override() {
        assert_eq!(
            agent_args_with_effort("codex", "do x", Some("gpt-5.6-sol"), Some("high")),
            Some((
                "codex".into(),
                vec![
                    "-m".into(),
                    "gpt-5.6-sol".into(),
                    "-c".into(),
                    "model_reasoning_effort=\"high\"".into(),
                    "--".into(),
                    "do x".into(),
                ]
            ))
        );
    }

    #[test]
    fn codex_headless_reasoning_effort_follows_exec_and_model() {
        assert_eq!(
            headless_args_with_effort("codex", "fix bug", Some("gpt-5.6-sol"), Some("xhigh"), None),
            Some((
                "codex".into(),
                vec![
                    "exec".into(),
                    "-m".into(),
                    "gpt-5.6-sol".into(),
                    "-c".into(),
                    "model_reasoning_effort=\"xhigh\"".into(),
                    "--skip-git-repo-check".into(),
                    "--dangerously-bypass-approvals-and-sandbox".into(),
                    "fix bug".into(),
                ]
            ))
        );
    }

    #[test]
    fn claude_interactive_reasoning_effort_follows_model_as_trailing_flag() {
        assert_eq!(
            agent_args_with_effort("claude", "do x", Some("opus"), Some("high")),
            Some((
                "claude".into(),
                vec![
                    "--model".into(),
                    "opus".into(),
                    "--effort".into(),
                    "high".into(),
                    "--".into(),
                    "do x".into(),
                ]
            ))
        );
    }

    #[test]
    fn claude_headless_reasoning_effort_follows_model_flag() {
        assert_eq!(
            headless_args_with_effort("claude", "fix bug", Some("opus"), Some("max"), None),
            Some((
                "claude".into(),
                vec![
                    "-p".into(),
                    "fix bug".into(),
                    "--dangerously-skip-permissions".into(),
                    "--model".into(),
                    "opus".into(),
                    "--effort".into(),
                    "max".into(),
                ]
            ))
        );
    }

    #[test]
    fn claude_headless_session_name_becomes_dash_n() {
        let args = headless_args_with_effort("claude", "fix bug", None, None, Some("task-42"))
            .unwrap()
            .1;
        let at = args.iter().position(|a| a == "-n").expect("-n 누락");
        assert_eq!(args[at + 1], "task-42");
    }

    #[test]
    fn non_claude_headless_never_gets_dash_n() {
        // codex/agy에는 대응 플래그가 없다 — 붙으면 플래그 오류로 즉사한다.
        for agent in ["codex", "agy"] {
            let args = headless_args_with_effort(agent, "fix bug", None, None, Some("task-42"))
                .unwrap()
                .1;
            assert!(!args.iter().any(|a| a == "-n"), "{agent}에 -n이 붙었다");
        }
    }

    #[test]
    fn session_name_follows_task_id() {
        assert_eq!(session_name(42), "task-42");
    }

    #[test]
    fn headless_agy_model_uses_long_flag() {
        // agy CLI는 `-m` 단축 플래그가 없다 — `--model`만 유효.
        assert_eq!(
            headless_args("agy", "fix bug", Some("gemini-3-pro")),
            Some((
                "agy".into(),
                vec![
                    "-p".into(),
                    "fix bug".into(),
                    "--dangerously-skip-permissions".into(),
                    "--print-timeout".into(),
                    crate::convo::AGY_PRINT_TIMEOUT.into(),
                    "--model".into(),
                    "gemini-3-pro".into()
                ]
            ))
        );
    }

    #[test]
    fn interactive_agy_model_uses_long_flag() {
        assert_eq!(
            agent_args("agy", "do x", Some("gemini-3-pro")),
            Some((
                "agy".into(),
                vec![
                    "-i".into(),
                    "--model".into(),
                    "gemini-3-pro".into(),
                    "do x".into()
                ]
            ))
        );
    }

    #[test]
    fn agy_alias_interactive_effort_follows_unchanged_model() {
        for agent in ["agy", "gemini", "antigravity"] {
            assert_eq!(
                agent_args_with_effort(agent, "do x", Some("gemini-3.6-flash-high"), Some("high")),
                Some((
                    "agy".into(),
                    vec![
                        "-i".into(),
                        "--model".into(),
                        "gemini-3.6-flash-high".into(),
                        "--effort".into(),
                        "high".into(),
                        "do x".into(),
                    ]
                ))
            );
        }
    }

    #[test]
    fn agy_alias_headless_effort_follows_unchanged_model() {
        for agent in ["agy", "gemini", "antigravity"] {
            assert_eq!(
                headless_args_with_effort(
                    agent,
                    "fix bug",
                    Some("gemini-3.6-flash-high"),
                    Some("medium"),
                    None,
                ),
                Some((
                    "agy".into(),
                    vec![
                        "-p".into(),
                        "fix bug".into(),
                        "--dangerously-skip-permissions".into(),
                        "--print-timeout".into(),
                        crate::convo::AGY_PRINT_TIMEOUT.into(),
                        "--model".into(),
                        "gemini-3.6-flash-high".into(),
                        "--effort".into(),
                        "medium".into(),
                    ]
                ))
            );
        }
    }

    #[test]
    fn agy_alias_builders_omit_blank_effort() {
        for agent in ["agy", "gemini", "antigravity"] {
            let interactive = agent_args_with_effort(
                agent,
                "do x",
                Some("gemini-3.6-flash-high"),
                Some("  "),
            )
            .unwrap()
            .1;
            let headless = headless_args_with_effort(
                agent,
                "fix bug",
                Some("gemini-3.6-flash-high"),
                None,
                None,
            )
            .unwrap()
            .1;

            assert!(!interactive.iter().any(|arg| arg == "--effort"));
            assert!(!headless.iter().any(|arg| arg == "--effort"));
        }
    }
}
