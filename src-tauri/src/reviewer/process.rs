use std::ffi::OsString;
use std::io::Write;
use std::process::Stdio;
use std::time::Duration;

use crate::managed_process::{ProcessLease, SharedProcessRegistrar, SpawnedProcess};

/// 호출자가 stdout을 어떻게 쓰는지. 리뷰·판정은 stdout이 결과라 비면 실패이고, 쓰기 에이전트는
/// 파일을 고치는 것이 결과라 stdout이 비어도 정상이다.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Expect {
    /// stdout에 결과(nonce + JSON)가 있어야 한다. 빈 출력은 stderr를 담아 Err.
    Output,
    /// stdout은 참고용. 종료 코드만 본다.
    SideEffect,
}

pub(super) fn run(
    bin: &str,
    args: &[&str],
    prompt: &str,
    timeout_secs: u64,
    registrar: Option<&SharedProcessRegistrar>,
) -> Result<String, String> {
    let tmp = temporary_directory()?;
    let result = run_in_directory(bin, args, prompt, timeout_secs, registrar, &tmp, Expect::Output);
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

pub(super) fn run_in_directory(
    bin: &str, args: &[&str], prompt: &str, timeout_secs: u64,
    registrar: Option<&SharedProcessRegistrar>, cwd: &std::path::Path, expect: Expect,
) -> Result<String, String> {
    let args = args.iter().map(OsString::from).collect::<Vec<_>>();
    let spawned = crate::managed_process::spawn_registered(bin, &args, registrar, |command| {
        command
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    })
    .map_err(|error| format!("{bin} 실행 실패: {error} (설치/PATH 확인)"));
    spawned.and_then(|mut spawned| {
        write_prompt(&mut spawned, prompt)?;
        await_output(spawned, timeout_secs, expect)
    })
}

/// 같은 프로세스에서 병렬로 도는 리뷰가 시계 해상도(macOS는 µs)로 같은 이름을 얻지 않게 하는 순번.
static TEMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn temporary_directory() -> Result<std::path::PathBuf, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    // 순번으로 이름이 겹치지 않게 하고, 다른 프로세스가 같은 이름을 쓴 드문 경우는 순번을 올려 재시도한다.
    let mut last = String::new();
    for _ in 0..16 {
        let seq = TEMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let tmp = std::env::temp_dir().join(format!("praxis-reviewer-{}-{nanos}-{seq}", std::process::id()));
        match std::fs::create_dir(&tmp) {
            Ok(()) => return Ok(tmp),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => last = error.to_string(),
            Err(error) => return Err(format!("temp 생성 실패: {error}")),
        }
    }
    Err(format!("temp 생성 실패: {last}"))
}

fn write_prompt(spawned: &mut SpawnedProcess, prompt: &str) -> Result<(), String> {
    let Some(mut stdin) = spawned.child.stdin.take() else {
        return abort_spawned(spawned, "리뷰어 stdin 핸들 없음".into());
    };
    if let Err(error) = stdin
        .write_all(prompt.as_bytes())
        .and_then(|_| stdin.flush())
    {
        return abort_spawned(spawned, format!("리뷰어 stdin 쓰기 실패: {error}"));
    }
    Ok(())
}

fn abort_spawned(spawned: &mut SpawnedProcess, detail: String) -> Result<(), String> {
    if let Some(lease) = spawned.lease.take() {
        let _ = lease.quarantine(&detail);
    }
    crate::verify::kill_group(spawned.pid);
    let _ = spawned.child.kill();
    let _ = spawned.child.wait();
    Err(detail)
}

fn await_output(mut spawned: SpawnedProcess, timeout_secs: u64, expect: Expect) -> Result<String, String> {
    let lease = spawned.lease.take();
    let pid = spawned.pid;
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(spawned.child.wait_with_output());
    });
    match receiver.recv_timeout(Duration::from_secs(timeout_secs.max(1))) {
        Ok(Ok(output)) => finish_output(output, lease, expect),
        Ok(Err(error)) => {
            quarantine(lease, &format!("reviewer wait failed: {error}"));
            Err(format!("리뷰어 오류: {error}"))
        }
        Err(_) => timeout(receiver, lease, pid, timeout_secs),
    }
}

fn finish_output(
    output: std::process::Output,
    lease: Option<Box<dyn ProcessLease>>,
    expect: Expect,
) -> Result<String, String> {
    complete(lease)?;
    classify_output(output.status.success(), output.status.code(), &output.stdout, &output.stderr, expect)
}

/// 종료 코드와 stdout·stderr로 결과를 가른다. **stdout이 비면 exit 0이어도 실패다** — agy는 헤드리스에서
/// 도구 권한이 거부되면 아무것도 출력하지 않고 0으로 끝나며 사유는 stderr에만 남긴다. 빈 문자열을 Ok로
/// 돌리면 호출자는 "출력에 nonce가 없습니다"만 보고 stderr의 진짜 사유는 사라진다.
fn classify_output(success: bool, code: Option<i32>, stdout: &[u8], stderr: &[u8], expect: Expect) -> Result<String, String> {
    let stderr_text = || {
        let s = String::from_utf8_lossy(stderr);
        let s = s.trim();
        if s.is_empty() { "(stderr 없음)".to_string() } else { s.chars().take(2000).collect() }
    };
    if !success {
        return Err(format!("리뷰어 비정상 종료(code {:?}): {}", code, stderr_text()));
    }
    let out = String::from_utf8_lossy(stdout);
    if expect == Expect::Output && out.trim().is_empty() {
        return Err(format!("리뷰어가 아무 출력 없이 종료했습니다: {}", stderr_text()));
    }
    Ok(out.into_owned())
}

fn timeout(
    receiver: std::sync::mpsc::Receiver<std::io::Result<std::process::Output>>,
    lease: Option<Box<dyn ProcessLease>>,
    pid: u32,
    timeout_secs: u64,
) -> Result<String, String> {
    crate::verify::kill_group(pid);
    match receiver.recv_timeout(Duration::from_secs(2)) {
        Ok(_) => {
            let _ = complete(lease);
        }
        Err(_) => quarantine(lease, "reviewer waiter did not reap after timeout"),
    }
    Err(format!("리뷰어 타임아웃 ({timeout_secs}s)"))
}

fn complete(lease: Option<Box<dyn ProcessLease>>) -> Result<(), String> {
    match lease {
        Some(lease) => lease.complete(),
        None => Ok(()),
    }
}

fn quarantine(lease: Option<Box<dyn ProcessLease>>, detail: &str) {
    if let Some(lease) = lease {
        let _ = lease.quarantine(detail);
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_output, Expect};

    #[test]
    fn empty_stdout_is_an_error_with_stderr_reason() {
        let err = classify_output(true, Some(0), b"  \n", b"Tool run_shell_command denied\n", Expect::Output).unwrap_err();
        assert!(err.contains("아무 출력 없이"), "{err}");
        assert!(err.contains("denied"), "stderr 사유를 보존해야 한다: {err}");
        assert!(classify_output(true, Some(0), b"", b"", Expect::Output).unwrap_err().contains("stderr 없음"));
        // 쓰기 에이전트는 파일이 결과라 빈 stdout이 정상이다.
        assert_eq!(classify_output(true, Some(0), b"", b"", Expect::SideEffect).unwrap(), "");
    }

    #[test]
    fn nonzero_exit_and_normal_output() {
        assert!(classify_output(false, Some(2), b"x", b"bad flag", Expect::SideEffect).unwrap_err().contains("code Some(2)"));
        assert_eq!(classify_output(true, Some(0), b"NONCE\n{}", b"warn", Expect::Output).unwrap(), "NONCE\n{}");
    }
}
