use std::io::Read;
use std::process::{Child, ChildStderr, ChildStdout, ExitStatus};
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(20);
const STDERR_TAIL_LINES: usize = 12;
const STDERR_LINE_CHARS: usize = 400;

/// 서버 일시 장애(503 UNAVAILABLE)로 출력 없이 끝난 턴의 재시도 대기. 자격 확인은 시작
/// 직후 실패하므로 대기만 합쳐도 사용자가 느끼는 지연은 수십 초 이내다.
pub const TRANSIENT_RETRY_DELAYS: [Duration; 3] =
    [Duration::from_secs(2), Duration::from_secs(5), Duration::from_secs(10)];

/// agy가 출력 없이 끝났을 때 stderr가 서버 일시 장애를 가리키는가.
/// 쿼터 소진(`Individual quota reached`)·인증 실패·타임아웃은 기다려도 풀리지 않으므로 넣지 않는다.
pub fn is_transient_failure(stderr_tail: &str) -> bool {
    stderr_tail.contains("UNAVAILABLE") || stderr_tail.contains("code 503")
}

pub struct AgyProcessOutput {
    pub timed_out: bool,
    pub exit_desc: String,
    pub stderr_tail: String,
}

#[cfg(unix)]
pub fn capture(
    child: &mut Child,
    mut stdout: ChildStdout,
    mut stderr: Option<ChildStderr>,
    pid: u32,
    idle_timeout_secs: u64,
    mut on_line: impl FnMut(&str),
) -> Result<AgyProcessOutput, String> {
    set_nonblocking(&stdout)?;
    if let Some(pipe) = stderr.as_ref() {
        set_nonblocking(pipe)?;
    }
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let mut last_output = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if read_available(&mut stdout, &mut stdout_bytes)? {
            last_output = Instant::now();
            emit_lines(&mut stdout_bytes, &mut on_line);
        }
        if let Some(pipe) = stderr.as_mut() {
            read_available(pipe, &mut stderr_bytes)?;
        }
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            read_available(&mut stdout, &mut stdout_bytes)?;
            if let Some(pipe) = stderr.as_mut() {
                read_available(pipe, &mut stderr_bytes)?;
            }
            break status;
        }
        if !timed_out && last_output.elapsed() >= Duration::from_secs(idle_timeout_secs.max(1)) {
            timed_out = true;
            crate::verify::kill_group(pid);
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    emit_lines(&mut stdout_bytes, &mut on_line);
    flush_line(stdout_bytes, &mut on_line);
    Ok(output(stderr_bytes, timed_out, status))
}

#[cfg(unix)]
fn set_nonblocking(pipe: &impl std::os::fd::AsRawFd) -> Result<(), String> {
    let fd = pipe.as_raw_fd();
    let flags = unsafe { nix::libc::fcntl(fd, nix::libc::F_GETFL) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let result = unsafe { nix::libc::fcntl(fd, nix::libc::F_SETFL, flags | nix::libc::O_NONBLOCK) };
    if result < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn read_available(reader: &mut impl Read, bytes: &mut Vec<u8>) -> Result<bool, String> {
    let mut read_any = false;
    let mut chunk = [0_u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => return Ok(read_any),
            Ok(count) => {
                bytes.extend_from_slice(&chunk[..count]);
                read_any = true;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(read_any),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.to_string()),
        }
    }
}

#[cfg(not(unix))]
pub fn capture(
    child: &mut Child,
    stdout: ChildStdout,
    stderr: Option<ChildStderr>,
    pid: u32,
    idle_timeout_secs: u64,
    mut on_line: impl FnMut(&str),
) -> Result<AgyProcessOutput, String> {
    let stdout_rx = read_in_background(stdout);
    let stderr_rx = stderr.map(read_in_background);
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let mut last_output = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if drain_channel(&stdout_rx, &mut stdout_bytes) {
            last_output = Instant::now();
            emit_lines(&mut stdout_bytes, &mut on_line);
        }
        if let Some(receiver) = stderr_rx.as_ref() {
            drain_channel(receiver, &mut stderr_bytes);
        }
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            std::thread::sleep(POLL_INTERVAL);
            drain_channel(&stdout_rx, &mut stdout_bytes);
            if let Some(receiver) = stderr_rx.as_ref() {
                drain_channel(receiver, &mut stderr_bytes);
            }
            break status;
        }
        if !timed_out && last_output.elapsed() >= Duration::from_secs(idle_timeout_secs.max(1)) {
            timed_out = true;
            crate::verify::kill_group(pid);
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    emit_lines(&mut stdout_bytes, &mut on_line);
    flush_line(stdout_bytes, &mut on_line);
    Ok(output(stderr_bytes, timed_out, status))
}

#[cfg(not(unix))]
fn read_in_background(
    mut reader: impl Read + Send + 'static,
) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut chunk = [0_u8; 8192];
        while let Ok(count) = reader.read(&mut chunk) {
            if count == 0 || sender.send(chunk[..count].to_vec()).is_err() {
                break;
            }
        }
    });
    receiver
}

#[cfg(not(unix))]
fn drain_channel(receiver: &std::sync::mpsc::Receiver<Vec<u8>>, bytes: &mut Vec<u8>) -> bool {
    let mut read_any = false;
    while let Ok(chunk) = receiver.try_recv() {
        bytes.extend_from_slice(&chunk);
        read_any = true;
    }
    read_any
}

/// 버퍼에서 완성된 줄(`\n` 종결)을 모두 꺼내 넘긴다. 미완성 꼬리는 다음 읽기를 기다린다.
fn emit_lines(bytes: &mut Vec<u8>, on_line: &mut impl FnMut(&str)) {
    while let Some(end) = bytes.iter().position(|byte| *byte == b'\n') {
        let line: Vec<u8> = bytes.drain(..=end).collect();
        on_line(String::from_utf8_lossy(&line[..end]).trim_end_matches('\r'));
    }
}

/// 프로세스가 끝난 뒤 개행 없이 남은 마지막 줄.
fn flush_line(bytes: Vec<u8>, on_line: &mut impl FnMut(&str)) {
    if !bytes.is_empty() {
        on_line(String::from_utf8_lossy(&bytes).trim_end_matches('\r'));
    }
}

fn output(
    stderr: Vec<u8>,
    timed_out: bool,
    status: ExitStatus,
) -> AgyProcessOutput {
    let stderr = String::from_utf8_lossy(&stderr);
    let stderr_tail = stderr
        .lines()
        .rev()
        .take(STDERR_TAIL_LINES)
        .map(|line| line.chars().take(STDERR_LINE_CHARS).collect::<String>())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    AgyProcessOutput {
        timed_out,
        exit_desc: exit_description(status),
        stderr_tail,
    }
}

fn exit_description(status: ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!("exit {code}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("signal {signal}");
        }
    }
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::{emit_lines, flush_line, is_transient_failure};

    #[test]
    fn lines_are_emitted_as_they_complete_and_the_tail_on_exit() {
        let mut lines = Vec::new();
        let mut bytes = b"first\r\nsec".to_vec();
        emit_lines(&mut bytes, &mut |line| lines.push(line.to_string()));
        assert_eq!(lines, vec!["first"]);
        bytes.extend_from_slice("ond\n한글 꼬리".as_bytes());
        emit_lines(&mut bytes, &mut |line| lines.push(line.to_string()));
        flush_line(bytes, &mut |line| lines.push(line.to_string()));
        assert_eq!(lines, vec!["first", "second", "한글 꼬리"]);
    }

    #[test]
    fn transient_failure_matches_only_server_unavailable() {
        assert!(is_transient_failure(
            "error: Eligibility check failed: UNAVAILABLE (code 503): The service is currently unavailable."
        ));
        assert!(!is_transient_failure(
            "Error: Individual quota reached. Please upgrade your subscription to increase your limits."
        ));
        assert!(!is_transient_failure("Error: timeout waiting for response"));
        assert!(!is_transient_failure(""));
    }
}
