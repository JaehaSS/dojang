//! 티켓 검증 명령 실행 — 저장소 verify(build·test) + 티켓 수용 명령. 동기·블로킹이며 명령당 `timeout_secs`까지 걸린다.
//! 실행은 `verify::run_check_registered`(sh -c, 프로세스 그룹 종료, registrar 등록)에 위임한다.

use crate::managed_process::SharedProcessRegistrar;
use serde::Serialize;
use std::path::Path;

const TAIL_MAX_CHARS: usize = 4000;
const SUMMARY_TAIL_CHARS: usize = 1500;
const SUMMARY_MAX_CHARS: usize = 6000;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CheckOutcome {
    pub command: String,
    pub exit_code: i32,
    pub ok: bool,
    /// 출력 끝부분(최대 약 4000자).
    pub tail: String,
    pub timed_out: bool,
}

fn last_chars(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    s.chars().skip(n - max).collect()
}

/// `/bin/sh -c command`를 `dir`에서 실행한다. 타임아웃이면 프로세스 그룹을 종료하고 `timed_out`을 켠다.
pub fn run_shell(
    dir: &Path,
    command: &str,
    timeout_secs: u64,
    registrar: Option<&SharedProcessRegistrar>,
) -> CheckOutcome {
    let r = crate::verify::run_check_registered(dir, command, timeout_secs, registrar);
    let timed_out = r.exit_code == -1 && r.tail.starts_with("타임아웃 (");
    CheckOutcome {
        command: command.to_string(),
        ok: r.exit_code == 0,
        exit_code: r.exit_code,
        tail: last_chars(&r.tail, TAIL_MAX_CHARS),
        timed_out,
    }
}

/// verify 사양의 build → test, 이어서 수용 명령을 실행한다. build가 실패하면 수용 명령은 건너뛴다(결과에 없음).
/// 사양에 build·test가 모두 없으면 수용 명령만 실행한다. 타임아웃은 사양 `timeout_secs`(0이면 기본 600초).
pub fn run_ticket_checks(
    worktree: &Path,
    acceptance_commands: &[String],
    registrar: Option<&SharedProcessRegistrar>,
) -> Vec<CheckOutcome> {
    let spec = crate::verify::detect_spec(worktree);
    let timeout = if spec.timeout_secs == 0 { 600 } else { spec.timeout_secs };
    let mut out = Vec::new();
    let mut build_failed = false;
    for (i, cmd) in [spec.build.as_deref(), spec.test.as_deref()].into_iter().enumerate() {
        let Some(cmd) = cmd.filter(|c| !c.trim().is_empty()) else { continue };
        let r = run_shell(worktree, cmd, timeout, registrar);
        if i == 0 && !r.ok {
            build_failed = true;
        }
        out.push(r);
    }
    if !build_failed {
        for cmd in acceptance_commands.iter().filter(|c| !c.trim().is_empty()) {
            out.push(run_shell(worktree, cmd, timeout, registrar));
        }
    }
    out
}

/// 모두 통과했는가. 결과가 비어 있으면 통과로 보지 않는다(미설정을 통과로 위장 금지).
pub fn checks_passed(outcomes: &[CheckOutcome]) -> bool {
    !outcomes.is_empty() && outcomes.iter().all(|o| o.ok)
}

/// 실패한 명령만 모은 수정 지시용 요약(상한 있음).
pub fn failure_summary(outcomes: &[CheckOutcome]) -> String {
    let mut s = String::new();
    for o in outcomes.iter().filter(|o| !o.ok) {
        let why = if o.timed_out { "타임아웃".to_string() } else { format!("exit {}", o.exit_code) };
        s.push_str(&format!("$ {} ({why})\n{}\n\n", o.command, last_chars(o.tail.trim(), SUMMARY_TAIL_CHARS)));
        if s.chars().count() >= SUMMARY_MAX_CHARS {
            break;
        }
    }
    if s.is_empty() && outcomes.is_empty() {
        return "실행된 검증 명령이 없습니다.".into();
    }
    let t = s.trim_end();
    if t.chars().count() > SUMMARY_MAX_CHARS {
        format!("{}\n[... 생략]", t.chars().take(SUMMARY_MAX_CHARS).collect::<String>())
    } else {
        t.to_string()
    }
}

#[cfg(test)]
#[path = "checks_tests.rs"]
mod tests;
