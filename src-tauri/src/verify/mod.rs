//! 검증 게이트 코어 (Framein evidence/gate 이식) — Tauri 비의존, `cargo test`.
//!
//! - `detect_spec`: `.dojang/validate.toml` 우선, 레거시 설정이 없으면 마커 파일로 빌드/테스트 명령 자동탐지.
//! - `run_check`: worktree에서 명령 실행 → exit code + 출력 tail (timeout).
//! - `parse_test_summary`: 빌드/테스트 출력에서 passed/failed 추출.
//! - `gate`: 증거 → ready 판정 (체크 0개면 not-ready: 미설정을 통과로 위장 금지).

use std::path::{Path, PathBuf};

use serde::Serialize;

mod managed_check;
pub use managed_check::{run_check, run_check_registered};

/// 검증 명령 사양.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ValidateSpec {
    pub build: Option<String>,
    pub test: Option<String>,
    pub timeout_secs: u64,
}

/// 단일 명령 실행 결과.
#[derive(Debug, Clone, Serialize, Default)]
pub struct CheckResult {
    pub command: String,
    pub exit_code: i32,
    pub tail: String,
}

/// 테스트 요약.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct TestSummary {
    pub passed: u32,
    pub failed: u32,
}

/// 증거 번들.
#[derive(Debug, Clone, Serialize, Default)]
pub struct EvidenceBundle {
    pub build: Option<CheckResult>,
    pub tests: Option<CheckResult>,
    pub test_summary: Option<TestSummary>,
    pub changed_files: Vec<String>,
    pub created_at: i64,
}

/// 게이트 판정.
#[derive(Debug, Clone, Serialize)]
pub struct GateResult {
    pub ready: bool,
    pub checks: Vec<(String, bool)>,
    pub warnings: Vec<String>,
}

/// 프론트 반환용 — 사용한 명령 + 결과 + 게이트 + 경고(미추적 포함).
#[derive(Debug, Clone, Serialize)]
pub struct VerifyReport {
    pub spec: ValidateSpec,
    pub build: Option<CheckResult>,
    pub test: Option<CheckResult>,
    pub summary: Option<TestSummary>,
    pub ready: bool,
    pub checks: Vec<(String, bool)>,
    pub warnings: Vec<String>,
}

const DEFAULT_TIMEOUT: u64 = 600;

/// `key = "value"` 한 줄 파서 (zero-dep, validate.toml 용).
fn toml_str(txt: &str, key: &str) -> Option<String> {
    for line in txt.lines() {
        let line = line.trim();
        // 주석/섹션 라인 스킵 (flat key=value만 지원; W4).
        if line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        if let Some(rest) = line.strip_prefix(key) {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix('=') {
                let v = rest.trim().trim_matches('"').trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// 선택된 검증 설정 경로. 새 `.dojang` 설정이 존재하면 레거시 설정보다 우선한다.
///
/// 새 설정이 비어 있거나 읽히지 않으면 호출자는 레거시로 내려가지 않고 자동탐지를 사용해야 한다.
pub fn validate_config_path(root: &Path) -> Option<PathBuf> {
    crate::project_paths::DIRS
        .into_iter()
        .map(|dir| root.join(dir).join("validate.toml"))
        .find(|path| path.exists())
}

/// 검증 명령 결정: `.dojang/validate.toml` 우선, 레거시 `.praxis` 설정, 마커 파일 자동탐지.
pub fn detect_spec(root: &Path) -> ValidateSpec {
    // (1) 레포 설정 오버라이드
    if let Some(config_path) = validate_config_path(root) {
        if let Ok(txt) = std::fs::read_to_string(config_path) {
            let build = toml_str(&txt, "build");
            let test = toml_str(&txt, "test");
            let timeout_secs = toml_str(&txt, "timeout_secs")
                .and_then(|s| s.parse().ok())
                .unwrap_or(DEFAULT_TIMEOUT);
            if build.is_some() || test.is_some() {
                return ValidateSpec {
                    build,
                    test,
                    timeout_secs,
                };
            }
        }
    }

    // (2) 자동탐지 (첫 매칭 1개; 혼합 레포는 validate.toml 권장)
    let has = |f: &str| root.join(f).exists();
    let pm = if has("pnpm-lock.yaml") {
        "pnpm"
    } else if has("yarn.lock") {
        "yarn"
    } else if has("bun.lockb") {
        "bun"
    } else {
        "npm"
    };
    let (mut build, mut test) = (None, None);
    if has("Cargo.toml") {
        build = Some("cargo build".into());
        test = Some("cargo test".into());
    } else if has("package.json") {
        let pkg = std::fs::read_to_string(root.join("package.json")).unwrap_or_default();
        if pkg.contains("\"build\"") {
            build = Some(format!("{pm} run build"));
        }
        if pkg.contains("\"test\"") {
            test = Some(format!("{pm} test"));
        }
    } else if has("go.mod") {
        build = Some("go build ./...".into());
        test = Some("go test ./...".into());
    } else if has("pyproject.toml") || has("setup.py") || has("tox.ini") {
        // 시스템 python이 아닌 프로젝트 고정 환경(.python-version 등)으로 돌려야 하므로 락 파일로 러너를 고른다.
        test = Some(
            if has("uv.lock") {
                "uv run pytest"
            } else if has("poetry.lock") {
                "poetry run pytest"
            } else {
                "pytest"
            }
            .into(),
        );
    } else if has("pom.xml") {
        build = Some("mvn -q -DskipTests package".into());
        test = Some("mvn -q test".into());
    } else if has("build.gradle") || has("build.gradle.kts") {
        build = Some("./gradlew build -x test".into());
        test = Some("./gradlew test".into());
    } else if has("Makefile") {
        build = Some("make build".into());
        test = Some("make test".into());
    }
    ValidateSpec {
        build,
        test,
        timeout_secs: DEFAULT_TIMEOUT,
    }
}

fn num_before(line: &str, kw: &str) -> Option<u32> {
    let idx = line.find(kw)?;
    let pre = line[..idx].trim_end();
    let digits: String = pre
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits.parse().ok()
}

/// 빌드/테스트 출력에서 passed/failed 추출 (cargo/node:test/jest/vitest/pytest 공통).
/// 마지막 'passed' 포함 라인을 요약으로 본다. 못 잡으면 None (exit code 폴백).
pub fn parse_test_summary(out: &str) -> Option<TestSummary> {
    let line = out.lines().rev().find(|l| l.contains("passed"))?;
    let passed = num_before(line, "passed")?;
    let failed = num_before(line, "failed").unwrap_or(0);
    Some(TestSummary { passed, failed })
}

/// 증거 → ready 판정. **체크가 0개면 ready=false** (미설정을 통과로 위장 금지).
pub fn gate(e: &EvidenceBundle) -> GateResult {
    let mut checks: Vec<(String, bool)> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    if let Some(b) = &e.build {
        checks.push(("build".into(), b.exit_code == 0));
    }
    if let Some(t) = &e.tests {
        let ok = t.exit_code == 0 && e.test_summary.map(|s| s.failed == 0).unwrap_or(true);
        checks.push(("tests".into(), ok));
    }
    if checks.is_empty() {
        warnings.push("검증 명령이 없습니다 (.dojang/validate.toml 설정 권장)".into());
    }
    let ready = !checks.is_empty() && checks.iter().all(|(_, ok)| *ok);
    GateResult {
        ready,
        checks,
        warnings,
    }
}

/// opt-in Approve 차단 판정: 설정 ON이고 (증거 없음 또는 not-ready)면 차단.
pub fn approve_blocked(block_on: bool, evidence_ready: Option<bool>) -> bool {
    block_on && !evidence_ready.unwrap_or(false)
}

fn tail_str(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut start = s.len() - n;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    s[start..].to_string()
}

/// 타임아웃 시 프로세스 그룹 전체 강제 종료 (cargo→rustc 등 자식까지).
#[cfg(unix)]
pub(crate) fn kill_group(pid: u32) {
    let _ = kill_group_checked(pid);
}

#[cfg(unix)]
pub(crate) fn kill_group_checked(pid: u32) -> std::io::Result<()> {
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;
    classify_group_signal(killpg(Pid::from_raw(pid as i32), Signal::SIGKILL))
}
#[cfg(windows)]
pub(crate) fn kill_group(pid: u32) {
    let _ = kill_group_checked(pid);
}

/// Windows에는 프로세스 그룹 신호가 없다 — `taskkill /T`로 자식 트리까지 종료한다.
/// taskkill은 대상이 이미 사라진 경우에도 실패 코드를 내므로, 실패를 그대로 오류로
/// 올리지 않고 tasklist 재조회로 부재를 확인한다(unix의 ESRCH 허용과 같은 의미).
#[cfg(windows)]
pub(crate) fn kill_group_checked(pid: u32) -> std::io::Result<()> {
    let output = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F", "/T"])
        .output()?;
    if output.status.success() || !process_group_alive_checked(pid)? {
        return Ok(());
    }
    Err(std::io::Error::other(format!(
        "taskkill failed for pid {pid}: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

/// 프로세스 그룹 생존 확인 — 신호 0(존재 검사) / Windows tasklist.
/// 앱 재시작 후 대화(convo) 고아 프로세스 판별용(Plan 0012). 재부팅 후 pid 재사용 오탐은
/// reaper 재검 + 워치독 상한이 완화(정밀 `comm` 대조는 YAGNI로 보류).
#[cfg(unix)]
pub(crate) fn process_group_alive(pid: u32) -> bool {
    process_group_alive_checked(pid).unwrap_or(true)
}

#[cfg(unix)]
pub(crate) fn process_group_alive_checked(pid: u32) -> std::io::Result<bool> {
    use nix::sys::signal::killpg;
    use nix::unistd::Pid;
    // 신호 0: 그룹에 살아있는 프로세스가 하나라도 있으면 Ok, 전무하면 Err(ESRCH).
    classify_group_probe(killpg(Pid::from_raw(pid as i32), None))
}
#[cfg(windows)]
pub(crate) fn process_group_alive(pid: u32) -> bool {
    process_group_alive_checked(pid).unwrap_or(false)
}

/// CSV 출력으로 조회해 PID 열만 대조한다 — 기본 표 형식은 메모리 사용량 등 다른 열의
/// 숫자에 pid가 우연히 걸릴 수 있다. 일치가 없으면 tasklist는 안내 문구만 찍고 0으로 끝난다.
#[cfg(windows)]
pub(crate) fn process_group_alive_checked(pid: u32) -> std::io::Result<bool> {
    let output = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "tasklist failed for pid {pid}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\"")))
}

/// SIGKILL을 보낸 **직후** 종료를 확인하는 전용 경로.
///
/// macOS는 아직 부모가 수확(wait)하지 않은 좀비에 신호를 보내면 `ESRCH`가 아니라 `EPERM`을
/// 돌려준다. 방금 우리가 그 그룹에 kill을 성공시킨 뒤이므로 여기서의 `EPERM`은 "권한 없음"이
/// 아니라 "이미 죽어 수확 대기 중"으로 읽는 것이 맞다. 일반 생존 확인
/// (`process_group_alive_checked`)은 남의 프로세스를 죽었다고 오판하면 안 되므로 `EPERM`을
/// 계속 오류로 남겨 둔다 — 두 경로를 분리하는 이유다.
///
/// 리눅스는 같은 상태를 `EPERM`이 아니라 **성공**으로 알린다. 좀비 판정은 플랫폼마다 다른
/// 신호를 읽어야 하므로 `group_has_only_zombies`로 갈라 둔다.
#[cfg(unix)]
pub(crate) fn process_group_terminated_checked(pid: u32) -> std::io::Result<bool> {
    use nix::sys::signal::killpg;
    use nix::unistd::Pid;
    match killpg(Pid::from_raw(pid as i32), None) {
        Ok(()) => Ok(group_has_only_zombies(pid)),
        Err(nix::errno::Errno::ESRCH | nix::errno::Errno::EPERM) => Ok(true),
        Err(error) => Err(std::io::Error::from_raw_os_error(error as i32)),
    }
}

/// 리눅스에는 macOS의 `EPERM`에 해당하는 신호가 없다 — 좀비도 프로세스 테이블에 남아 있어
/// 신호 0이 그대로 **성공**한다. 그래서 killpg만으로는 "이미 죽었고 수확만 남은" 그룹을
/// 종료로 읽을 수 없고, `/proc`을 훑어 구성원의 상태를 직접 봐야 한다.
///
/// 그룹을 통째로 물어보는 시스템 콜이 없으므로 전체 순회 외의 방법이 없다. 부담은 kill 직후
/// 판정 루프(최대 20회)에 한정된다. 좀비가 아닌 구성원이 하나라도 보이면 아직 살아 있다.
#[cfg(target_os = "linux")]
fn group_has_only_zombies(pgid: u32) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        // 훑는 동안 사라진 프로세스는 그룹에 없는 것과 같다.
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{name}/stat")) else {
            continue;
        };
        if matches!(parse_proc_stat_group(&stat), Some((member, state)) if member == pgid && state != 'Z')
        {
            return false;
        }
    }
    true
}

/// macOS·BSD는 좀비를 `EPERM`으로 알려 주므로 순회가 필요 없다 — killpg의 성공은 살아 있는
/// 구성원이 있다는 뜻 그대로다.
#[cfg(all(unix, not(target_os = "linux")))]
fn group_has_only_zombies(_pgid: u32) -> bool {
    false
}

/// `/proc/<pid>/stat`에서 (pgrp, state)를 뽑는다.
///
/// comm 필드는 공백과 괄호를 담을 수 있어 앞에서 필드를 세면 어긋난다 — **마지막** `)`
/// 뒤부터 읽는 것이 커널 문서가 지정한 파싱 방법이다. 그 뒤 순서는 state, ppid, pgrp다.
#[cfg(any(target_os = "linux", all(test, unix)))]
fn parse_proc_stat_group(stat: &str) -> Option<(u32, char)> {
    let (_, tail) = stat.rsplit_once(')')?;
    let mut fields = tail.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let _ppid = fields.next()?;
    let pgrp = fields.next()?.parse().ok()?;
    Some((pgrp, state))
}

/// Windows에는 좀비(수확 대기) 상태가 없어 unix처럼 EPERM을 종료로 읽어 줄 필요가 없다 —
/// 생존 확인의 반대면 그대로 종료다.
#[cfg(windows)]
pub(crate) fn process_group_terminated_checked(pid: u32) -> std::io::Result<bool> {
    process_group_alive_checked(pid).map(|alive| !alive)
}

#[cfg(unix)]
fn classify_group_probe(result: Result<(), nix::errno::Errno>) -> std::io::Result<bool> {
    match result {
        Ok(()) => Ok(true),
        Err(nix::errno::Errno::ESRCH) => Ok(false),
        Err(error) => Err(std::io::Error::from_raw_os_error(error as i32)),
    }
}

#[cfg(unix)]
fn classify_group_signal(result: Result<(), nix::errno::Errno>) -> std::io::Result<()> {
    match result {
        Ok(()) | Err(nix::errno::Errno::ESRCH) => Ok(()),
        Err(error) => Err(std::io::Error::from_raw_os_error(error as i32)),
    }
}

/// 프로세스(그룹 리더 pid)의 실행 파일 이름이 `needle`을 포함하는지 best-effort 확인.
/// 재시작 후 pgid 재사용 오탐으로 **무관한 프로세스를 kill하지 않도록** 워치독 kill 직전 게이트(Plan 0012 DR-5 완화).
/// 확신이 없으면(조회 실패/불일치) `false` — 보수적으로 kill을 건너뛴다(무고한 프로세스 보호 우선).
#[cfg(unix)]
pub(crate) fn process_group_matches(pid: u32, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    match std::process::Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .to_lowercase()
            .contains(&needle.to_lowercase()),
        _ => false,
    }
}
#[cfg(windows)]
pub(crate) fn process_group_matches(pid: u32, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    match std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .to_lowercase()
            .contains(&needle.to_lowercase()),
        Err(_) => false,
    }
}

#[cfg(all(test, unix))]
mod alive_tests {
    use super::*;

    #[test]
    fn process_group_probe_distinguishes_absent_from_uncertain() {
        assert!(classify_group_probe(Ok(())).unwrap());
        assert!(!classify_group_probe(Err(nix::errno::Errno::ESRCH)).unwrap());
        assert!(classify_group_probe(Err(nix::errno::Errno::EPERM)).is_err());
        assert!(classify_group_signal(Err(nix::errno::Errno::EPERM)).is_err());
    }

    /// comm에 공백·괄호가 들어가도 pgrp와 state를 놓치지 않아야 한다 — 이 파싱이 어긋나면
    /// 리눅스에서 남의 프로세스를 그룹 구성원으로 잘못 세거나 좀비를 놓친다.
    #[test]
    fn proc_stat_fields_are_read_after_the_last_paren() {
        assert_eq!(
            parse_proc_stat_group("42 (sleep) Z 7 42 42 0 -1 4194560"),
            Some((42, 'Z'))
        );
        assert_eq!(
            parse_proc_stat_group("42 (odd ) name) S 7 41 41 0 -1 4194560"),
            Some((41, 'S'))
        );
        assert_eq!(parse_proc_stat_group("42 sleep Z 7 42"), None);
    }

    #[test]
    fn process_group_alive_tracks_live_then_dead() {
        use std::os::unix::process::CommandExt;
        // 독립 프로세스 그룹으로 sleep 스폰 → pgid == pid.
        let mut child = {
            let mut c = std::process::Command::new("sleep");
            c.arg("30");
            c.process_group(0);
            c.spawn().expect("spawn sleep")
        };
        let pid = child.id();
        assert!(process_group_alive(pid), "스폰 직후엔 살아있어야 함");
        // 실행 파일 이름 대조: "sleep"은 매칭, 무관 이름/빈 문자열은 불매칭(보수적).
        assert!(
            process_group_matches(pid, "sleep"),
            "comm이 sleep을 포함해야 함"
        );
        assert!(
            !process_group_matches(pid, "nonexistent-binary"),
            "무관 이름은 불매칭"
        );
        assert!(!process_group_matches(pid, ""), "빈 needle은 항상 false");
        kill_group(pid);
        let _ = child.wait(); // 좀비 회수 → 그룹 소멸 보장.
        assert!(!process_group_alive(pid), "kill 후엔 죽어있어야 함");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp() -> PathBuf {
        let d = crate::testtmp::dir().join(format!(
            "praxis-verify-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn parses_cargo_node_pytest() {
        assert_eq!(
            parse_test_summary("test result: ok. 12 passed; 0 failed; 1 ignored"),
            Some(TestSummary {
                passed: 12,
                failed: 0
            })
        );
        assert_eq!(
            parse_test_summary("Tests  3 passed (3)"),
            Some(TestSummary {
                passed: 3,
                failed: 0
            })
        );
        assert_eq!(
            parse_test_summary("=== 5 passed, 2 failed in 1.2s ==="),
            Some(TestSummary {
                passed: 5,
                failed: 2
            })
        );
        assert_eq!(parse_test_summary("no recognizable summary"), None);
    }

    #[test]
    fn gate_requires_checks_and_green() {
        let none = EvidenceBundle::default();
        assert!(!gate(&none).ready, "체크 0개면 not-ready");

        let mut e = EvidenceBundle::default();
        e.build = Some(CheckResult {
            command: "b".into(),
            exit_code: 0,
            tail: String::new(),
        });
        e.tests = Some(CheckResult {
            command: "t".into(),
            exit_code: 0,
            tail: String::new(),
        });
        e.test_summary = Some(TestSummary {
            passed: 3,
            failed: 0,
        });
        assert!(gate(&e).ready);

        e.test_summary = Some(TestSummary {
            passed: 3,
            failed: 1,
        });
        assert!(!gate(&e).ready, "failed>0이면 not-ready");
    }

    #[test]
    fn detect_python_runner_by_lockfile() {
        let d = tmp();
        std::fs::write(d.join("pyproject.toml"), "").unwrap();
        assert_eq!(detect_spec(&d).test.as_deref(), Some("pytest"));
        std::fs::write(d.join("poetry.lock"), "").unwrap();
        assert_eq!(detect_spec(&d).test.as_deref(), Some("poetry run pytest"));
        std::fs::write(d.join("uv.lock"), "").unwrap();
        assert_eq!(detect_spec(&d).test.as_deref(), Some("uv run pytest"), "uv.lock이 poetry.lock보다 우선");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn detect_auto_by_marker() {
        let d = tmp();
        std::fs::write(d.join("Cargo.toml"), "").unwrap();
        let s = detect_spec(&d);
        assert_eq!(s.test.as_deref(), Some("cargo test"));
        assert_eq!(s.build.as_deref(), Some("cargo build"));
        std::fs::remove_dir_all(&d).ok();

        let d2 = tmp();
        std::fs::write(d2.join("go.mod"), "").unwrap();
        assert_eq!(detect_spec(&d2).build.as_deref(), Some("go build ./..."));
        std::fs::remove_dir_all(&d2).ok();

        let d3 = tmp();
        let s3 = detect_spec(&d3);
        assert!(
            s3.build.is_none() && s3.test.is_none(),
            "마커 없으면 명령 0개"
        );
        std::fs::remove_dir_all(&d3).ok();
    }

    #[test]
    fn config_overrides_auto() {
        let d = tmp();
        std::fs::write(d.join("Cargo.toml"), "").unwrap();
        std::fs::create_dir_all(d.join(".praxis")).unwrap();
        std::fs::write(
            d.join(".praxis").join("validate.toml"),
            "build = \"make b\"\ntest = \"make t\"\ntimeout_secs = 120\n",
        )
        .unwrap();
        let s = detect_spec(&d);
        assert_eq!(s.build.as_deref(), Some("make b"));
        assert_eq!(s.test.as_deref(), Some("make t"));
        assert_eq!(s.timeout_secs, 120);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn new_config_has_priority_and_empty_new_config_falls_back_to_markers() {
        let d = tmp();
        std::fs::write(d.join("Cargo.toml"), "").unwrap();
        std::fs::create_dir_all(d.join(crate::project_paths::LEGACY_DIR)).unwrap();
        std::fs::write(
            d.join(crate::project_paths::LEGACY_DIR)
                .join("validate.toml"),
            "build = \"legacy build\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(d.join(crate::project_paths::DIR)).unwrap();
        let new_path = d.join(crate::project_paths::DIR).join("validate.toml");
        std::fs::write(&new_path, "test = \"new test\"\n").unwrap();

        assert_eq!(validate_config_path(&d), Some(new_path.clone()));
        let selected = detect_spec(&d);
        assert_eq!(selected.test.as_deref(), Some("new test"));
        assert!(selected.build.is_none());

        std::fs::write(&new_path, "# intentionally empty\n").unwrap();
        let fallback = detect_spec(&d);
        assert_eq!(fallback.build.as_deref(), Some("cargo build"));
        assert_eq!(fallback.test.as_deref(), Some("cargo test"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn approve_block_gate() {
        assert!(!approve_blocked(false, None), "토글 OFF면 차단 안 함");
        assert!(!approve_blocked(false, Some(false)), "OFF면 항상 통과");
        assert!(approve_blocked(true, None), "ON+증거없음 → 차단");
        assert!(approve_blocked(true, Some(false)), "ON+not-ready → 차단");
        assert!(!approve_blocked(true, Some(true)), "ON+ready → 통과");
    }

    #[test]
    fn run_check_captures_exit() {
        let d = tmp();
        assert_eq!(run_check(&d, "exit 0", 30).exit_code, 0);
        assert_eq!(run_check(&d, "exit 3", 30).exit_code, 3);
        std::fs::remove_dir_all(&d).ok();
    }

    #[cfg(unix)]
    #[test]
    fn run_check_times_out() {
        let d = tmp();
        let start = std::time::Instant::now();
        let r = run_check(&d, "sleep 30", 1);
        assert_eq!(r.exit_code, -1, "타임아웃은 exit -1");
        // 1s 타임아웃 → 30s sleep을 끝까지 기다리지 않고 즉시 반환(부하 여유로 10s).
        assert!(start.elapsed().as_secs() < 10, "타임아웃 후 즉시 반환");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn toml_skips_comments_and_sections() {
        let d = tmp();
        std::fs::create_dir_all(d.join(".praxis")).unwrap();
        std::fs::write(
            d.join(".praxis").join("validate.toml"),
            "# 주석\n[meta]\nbuild = \"make b\"\n# test = \"wrong\"\ntest = \"make t\"\n",
        )
        .unwrap();
        let s = detect_spec(&d);
        assert_eq!(s.build.as_deref(), Some("make b"));
        assert_eq!(s.test.as_deref(), Some("make t"));
        std::fs::remove_dir_all(&d).ok();
    }
}
