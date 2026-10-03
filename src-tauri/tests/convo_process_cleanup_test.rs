#![cfg(unix)]

#[path = "support/temp_root.rs"]
mod temp_root;

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use praxis_lib::convo::{run_turn, ConvoEvent, Vendor};

static NEXT_STUB: AtomicU32 = AtomicU32::new(0);

fn stub_script(body: &str) -> (std::path::PathBuf, String) {
    let sequence = NEXT_STUB.fetch_add(1, Ordering::SeqCst);
    let directory = temp_root::dir().join(format!(
        "praxis-cleanup-stub-{}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let binary = directory.join("vendor-stub");
    let mut file = std::fs::File::create(&binary).unwrap();
    write!(file, "#!/bin/sh\n{body}\n").unwrap();
    let mut permissions = file.metadata().unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&binary, permissions).unwrap();
    (directory, binary.to_string_lossy().into_owned())
}

fn run(vendor: Vendor, binary: &str) -> (Result<String, String>, Vec<ConvoEvent>) {
    let mut events = Vec::new();
    let result = run_turn(
        &temp_root::dir().to_string_lossy(),
        "cleanup test",
        None,
        5,
        vendor,
        binary,
        None,
        |_| {},
        |event| events.push(event),
    )
    .map(|outcome| outcome.session_id);
    (result, events)
}

fn wait_until_absent(pid: i32) -> bool {
    for _ in 0..20 {
        if unsafe { nix::libc::kill(pid, 0) } != 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn turn_terminates_a_child_that_rehomes_its_group_on_term() {
    let script = [
        "pid_file=\"$(dirname \"$0\")/detached.pid\"",
        "ready_file=\"$(dirname \"$0\")/detached.ready\"",
        "READY_FILE=\"$ready_file\" /usr/bin/python3 -c \
         'import os,signal,time; signal.signal(signal.SIGTERM, lambda *_: os.setsid()); \
         open(os.environ[\"READY_FILE\"], \"w\").close(); time.sleep(5)' &",
        "printf '%s' \"$!\" > \"$pid_file\"",
        "while [ ! -f \"$ready_file\" ]; do /bin/sleep 0.01; done",
        "printf '%s\\n' 'completed before detached child'",
    ]
    .join("\n");
    let (directory, binary) = stub_script(&script);
    let started = Instant::now();

    let (result, events) = run(Vendor::Agy, &binary);

    assert!(result.is_ok());
    // 경계가 유휴 상한(5초)보다 작기만 하면 "분리 자식을 기다렸다"는 여전히 잡힌다 — 매달렸다면
    // 상한에 걸려 5초를 넘긴다. 2초로 조이면 전체 스위트와 함께 도는 부하에서 인터프리터
    // 시작만으로 예산을 다 써, 코드가 아니라 부하가 테스트를 떨어뜨린다.
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(4),
        "분리 자식의 stdout을 기다린 것으로 보인다: {elapsed:?}"
    );
    assert!(events.iter().any(
        |event| matches!(event, ConvoEvent::Result { session_id, .. } if session_id == "continue")
    ));
    let detached_pid = std::fs::read_to_string(directory.join("detached.pid"))
        .unwrap()
        .parse::<i32>()
        .unwrap();
    assert!(
        wait_until_absent(detached_pid),
        "턴 종료 후 분리 자식이 남음: pid={detached_pid}"
    );
    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn turn_does_not_terminate_an_unmarked_process_group() {
    let mut unrelated = std::process::Command::new("/bin/sleep")
        .arg("5")
        .process_group(0)
        .spawn()
        .unwrap();
    let (directory, binary) = stub_script(
        "printf '%s\\n' \
         '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"safe-scope\"}' \
         '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"done\",\"session_id\":\"safe-scope\"}'",
    );

    let (result, _) = run(Vendor::Claude, &binary);

    assert_eq!(result.as_deref(), Ok("safe-scope"));
    assert!(unrelated.try_wait().unwrap().is_none());
    unrelated.kill().ok();
    unrelated.wait().ok();
    std::fs::remove_dir_all(&directory).ok();
}

fn result_flag(events: &[ConvoEvent]) -> (bool, String) {
    events
        .iter()
        .find_map(|event| match event {
            ConvoEvent::Result { is_error, text, .. } => Some((*is_error, text.clone())),
            _ => None,
        })
        .expect("Result 이벤트가 없음")
}

/// 백그라운드를 남기지 않은 평범한 턴이 프로세스 관측 때문에 실패로 찍히면 안 된다.
///
/// vendor는 자기 프로세스 그룹의 리더이고 마커도 상속하므로, 생존자 집계에서 vendor의
/// pgid를 빼지 않으면 **모든 턴이** 여기서 실패한다. 이 테스트가 그 오탐의 가드다.
#[test]
fn a_clean_turn_is_not_flagged_by_the_process_observation() {
    let (directory, binary) = stub_script(
        "printf '%s\\n' \
         '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"clean\"}' \
         '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"done\",\"session_id\":\"clean\"}'",
    );

    let (_, events) = run(Vendor::Claude, &binary);

    let (is_error, text) = result_flag(&events);
    assert!(!is_error, "백그라운드를 안 남긴 턴이 실패로 찍혔다: {text}");
    std::fs::remove_dir_all(&directory).ok();
}

/// 그룹을 탈출한 자식이 살아 있으면, 하니스가 접수 문구를 남기지 않았어도 실패로 표시한다.
///
/// `setsid`로 세션을 갈아탄 자식은 vendor 그룹과 함께 정리되지 않는다 — 실제로 Vite
/// preview가 이렇게 빠져나가 PPID 1 상태로 며칠간 포트를 물고 있었다(ADR 0067). 같은
/// 그룹에 남는 자식은 vendor와 함께 죽으므로 경고 대상이 아니다.
#[test]
fn an_escaped_background_child_flags_the_result() {
    let script = [
        "ready_file=\"$(dirname \"$0\")/escaped.ready\"",
        "READY_FILE=\"$ready_file\" /usr/bin/python3 -c \
         'import os,time; os.setsid(); open(os.environ[\"READY_FILE\"], \"w\").close(); \
         time.sleep(5)' &",
        "while [ ! -f \"$ready_file\" ]; do /bin/sleep 0.01; done",
        "printf '%s\\n' \
         '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"done\",\"session_id\":\"escaped\"}'",
    ]
    .join("\n");
    let (directory, binary) = stub_script(&script);

    let (_, events) = run(Vendor::Claude, &binary);

    let (is_error, text) = result_flag(&events);
    assert!(is_error, "탈출한 자식이 살아 있는데 성공으로 통과했다: {text}");
    assert!(
        text.contains("남은 프로세스 그룹"),
        "무엇이 남았는지 밝혀야 한다: {text}"
    );
    // 번호만으로는 사후 판별이 불가능하다 — 경고를 낸 직후 그룹이 죽기 때문이다.
    // macOS의 `/usr/bin/python3`는 Xcode 안 `…/MacOS/Python`으로 exec되므로 이름만 본다.
    assert!(
        text.to_lowercase().contains("python"),
        "남은 프로세스의 명령줄이 실려야 한다: {text}"
    );
    assert!(
        text.contains("os.setsid()"),
        "긴 인터프리터 경로에 밀려 인자가 잘리면 안 된다: {text}"
    );
    assert!(
        !text.contains("_TURN_TOKEN="),
        "마커 환경변수가 명령줄에 섞여 나왔다: {text}"
    );
    std::fs::remove_dir_all(&directory).ok();
}
