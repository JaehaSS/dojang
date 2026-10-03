//! 리뷰어 헤드리스 호출 (Framein delegate 이식) — Tauri 비의존.
//!
//! 보안: 리뷰어를 **빈 temp 디렉터리**에서 실행(worktree 아님)해 리뷰어가 툴을 써도
//! worktree를 못 건드리게 한다. 프롬프트는 기본적으로 stdin으로 전달하지만, agy는 CLI 계약상
//! `-p <prompt>` 값으로 전달한다. 타임아웃 시 프로세스 그룹 kill.

use crate::managed_process::SharedProcessRegistrar;

mod process;

/// PATH에서 실행 파일의 **절대 경로**를 찾는다 (간이 which).
/// PTY 스폰(portable_pty)은 프로그램명을 PATH로 해석하지 않으므로 절대경로가 필요하다.
///
/// Windows에서는 `.exe` → `.cmd` → 확장자 없음 순으로 찾는다. npm 전역 shim은 확장자 없는
/// POSIX 스크립트를 `.cmd` 옆에 함께 까는데, CreateProcess는 그것을 실행하지 못한다.
pub fn which(bin: &str) -> Option<String> {
    let path = std::env::var("PATH").ok()?;
    #[cfg(windows)]
    let candidates = [format!("{bin}.exe"), format!("{bin}.cmd"), bin.to_string()];
    #[cfg(not(windows))]
    let candidates = [bin.to_string()];
    for dir in std::env::split_paths(&path) {
        for candidate in &candidates {
            let p = dir.join(candidate);
            if p.is_file() {
                return Some(p.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// PATH에 실행 파일이 있는지(간이 which).
pub(crate) fn on_path(bin: &str) -> bool {
    which(bin).is_some()
}

/// 리드와 **다른 벤더**를 우선(교차모델), 없으면 claude 폴백.
pub fn detect_reviewer(lead: &str) -> String {
    for cand in ["codex", "agy", "claude"] {
        if cand != lead && on_path(cand) {
            return cand.to_string();
        }
    }
    "claude".to_string()
}

struct ReviewerInvocation<'a> {
    bin: &'static str,
    args: Vec<String>,
    stdin_prompt: Option<&'a str>,
}

fn invocation<'a>(model: &str, prompt: &'a str) -> ReviewerInvocation<'a> {
    let args = |values: &[&str]| values.iter().map(|value| value.to_string()).collect();
    match model {
        "codex" => ReviewerInvocation {
            bin: "codex",
            args: args(&["exec", "--skip-git-repo-check"]),
            stdin_prompt: Some(prompt),
        },
        // agy 1.1.8의 `-p`는 stdin 스위치가 아니라 프롬프트 값을 요구하는 플래그다.
        "gemini" | "agy" | "antigravity" => ReviewerInvocation {
            bin: "agy",
            args: args(&["-p", prompt]),
            stdin_prompt: None,
        },
        _ => ReviewerInvocation {
            bin: "claude",
            args: args(&["-p", "--output-format", "text"]),
            stdin_prompt: Some(prompt),
        },
    }
}

/// 벤더별 실행 커맨드라인 문자열 생성 (모델 정보 기록용).
pub fn describe_invocation(vendor: &str) -> String {
    let invocation = invocation(vendor, "<prompt>");
    let mut cmd = invocation.bin.to_string();
    for arg in invocation.args {
        cmd.push(' ');
        cmd.push_str(&arg);
    }
    cmd
}

/// 리뷰어를 헤드리스로 실행 → stdout 텍스트. 프롬프트는 stdin, cwd는 빈 temp.
pub fn run_reviewer(model: &str, prompt: &str, timeout_secs: u64) -> Result<String, String> {
    run_reviewer_registered(model, prompt, timeout_secs, None)
}

pub fn run_reviewer_registered(
    model: &str,
    prompt: &str,
    timeout_secs: u64,
    registrar: Option<&SharedProcessRegistrar>,
) -> Result<String, String> {
    let invocation = invocation(model, prompt);
    let args = invocation
        .args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    process::run(
        invocation.bin,
        &args,
        invocation.stdin_prompt.unwrap_or_default(),
        timeout_secs,
        registrar,
    )
}

/// Uses the task's established headless agent invocation in an app-owned candidate.
pub fn run_repair_agent(
    cwd: &std::path::Path, agent: &str, model: Option<&str>, effort: Option<&str>,
    prompt: &str, registrar: &SharedProcessRegistrar,
) -> Result<String, String> {
    if !crate::agent::is_preset(agent) && !matches!(agent, "gemini" | "antigravity") {
        return Err("자동 해결은 등록된 에이전트에서 지원합니다".into());
    }
    let (bin, args) = crate::agent::headless_args_with_effort(agent, prompt, model, effort, None)
        .ok_or_else(|| "에이전트 실행 설정이 없습니다".to_string())?;
    let bin = which(&bin).ok_or_else(|| format!("{bin} 실행 파일을 찾을 수 없습니다"))?;
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    process::run_in_directory(&bin, &args, "", 600, Some(registrar), cwd)
}

/// claude를 read-only 도구(Read,Grep,Glob)만 허용해 `cwd`에서 실행한다. (bin, args) — 단위 테스트용 순수 헬퍼.
fn readonly_claude_command() -> (&'static str, Vec<String>) {
    let args = ["-p", "--output-format", "text", "--tools", "Read,Grep,Glob"];
    ("claude", args.iter().map(|a| a.to_string()).collect())
}

/// 도구 제한으로만 read-only를 보장한다(Bash/Edit 비허용). 호출측은 실행 뒤 [`worktree_is_clean`]으로
/// worktree가 그대로인지 반드시 다시 확인해야 한다. 프롬프트는 stdin.
pub fn run_claude_readonly_in(
    cwd: &std::path::Path, prompt: &str, timeout_secs: u64,
    registrar: Option<&SharedProcessRegistrar>,
) -> Result<String, String> {
    let (bin, args) = readonly_claude_command();
    let bin = which(bin).ok_or_else(|| "claude 실행 파일을 찾을 수 없습니다".to_string())?;
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    process::run_in_directory(&bin, &args, prompt, timeout_secs, registrar, cwd)
}

/// `git status --porcelain`이 비어 있으면 true (추적·미추적 변경 없음).
pub fn worktree_is_clean(dir: &std::path::Path) -> Result<bool, String> {
    let out = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("git status 실행 실패: {e}"))?;
    if !out.status.success() {
        return Err(format!("git status 실패: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout.iter().all(|b| b.is_ascii_whitespace()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invocation_agy_reserves_print_flag_value_for_prompt() {
        // agy 1.1.8의 `-p`는 stdin 스위치가 아니라 프롬프트 값을 받는 플래그다.
        for vendor in ["agy", "antigravity", "gemini"] {
            let invocation = invocation(vendor, "prompt");
            assert_eq!(invocation.bin, "agy");
            assert_eq!(invocation.args, vec!["-p", "prompt"]);
            assert_eq!(invocation.stdin_prompt, None);
        }
    }

    #[test]
    fn invocation_known_vendors() {
        let codex = invocation("codex", "prompt");
        assert_eq!(codex.bin, "codex");
        assert_eq!(codex.stdin_prompt, Some("prompt"));
        // 미지/기본은 claude 폴백.
        assert_eq!(invocation("claude", "prompt").bin, "claude");
        assert_eq!(invocation("anything-else", "prompt").bin, "claude");
    }

    #[test]
    fn readonly_claude_args_restrict_tools() {
        let (bin, args) = readonly_claude_command();
        assert_eq!(bin, "claude");
        let i = args.iter().position(|a| a == "--tools").unwrap();
        assert_eq!(args[i + 1], "Read,Grep,Glob");
        assert!(args.contains(&"-p".to_string()));
    }

    #[test]
    fn worktree_clean_detects_changes() {
        let dir = std::env::temp_dir().join(format!("praxis-clean-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let git = |a: &[&str]| std::process::Command::new("git").args(a).current_dir(&dir).output().unwrap();
        git(&["init", "-q"]);
        assert_eq!(worktree_is_clean(&dir), Ok(true));
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        assert_eq!(worktree_is_clean(&dir), Ok(false));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
