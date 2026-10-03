use super::Vendor;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

const MARKER_NAME: &str = "DOJANG_TURN_TOKEN";
/// 이름을 바꾸기 전의 마커. 이전 버전이 `convo_executions.process_marker`에 저장한 값을
/// 복구하고, 그 버전이 띄운 프로세스의 argv에서 환경변수가 새지 않게 하려고 남겨 둔다.
const LEGACY_MARKER_NAME: &str = "PRAXIS_TURN_TOKEN";
/// 여러 빌드가 재사용하도록 턴보다 오래 사는 JVM 빌드 데몬의 메인 클래스.
///
/// `./gradlew`는 포그라운드로 끝나도 분리된 데몬을 남기고, 그 데몬은 마커를 상속한 채 자기
/// 그룹의 리더가 된다. 빌드는 이미 끝났으므로 유실된 작업이 아니고, 죽이면 다음 빌드만
/// 콜드 스타트가 된다.
const SHARED_BUILD_DAEMONS: &[&str] = &[
    "org.gradle.launcher.daemon.bootstrap.GradleDaemon",
    "org.jetbrains.kotlin.daemon.KotlinCompileDaemon",
];
/// 손상된 버퍼에서 argc를 그대로 믿지 않기 위한 상한.
const ARGUMENT_LIMIT: usize = 64;
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(0);

/// 턴이 끝났는데도 살아 있던 프로세스 그룹 하나.
///
/// 번호만으로는 아무것도 판별할 수 없다 — 경고를 낸 직후 `Drop`이 그룹을 죽이므로, 사용자가
/// 번호를 읽는 시점에 그 프로세스는 이미 없다. 유실된 작업(빌드·테스트)인지 무해한
/// 잔여물(MCP 서버 등)인지 가르는 것은 `arguments`뿐이다.
///
/// 인자는 **다듬지 않은 argv 그대로** 싣는다. 얼마나 줄여 보여줄지는 그것을 화면에 쓰는
/// 쪽(`turn_guard`)의 규칙이지 관측의 규칙이 아니다 — 여기서 잘라 버리면 다른 소비자가
/// 생겼을 때 원본이 어디에도 남지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Survivor {
    pub(crate) group: u32,
    /// 그룹 대표 프로세스의 argv. 조회 전에 죽었거나 읽을 수 없으면 `None`.
    pub(crate) arguments: Option<Vec<String>>,
}

pub(crate) struct TurnProcessScope {
    #[cfg(unix)]
    marker: String,
}

impl TurnProcessScope {
    pub(crate) fn attach(command: &mut Command) -> Self {
        let token = unique_token();
        command.env(MARKER_NAME, &token);
        Self {
            #[cfg(unix)]
            marker: format!("{MARKER_NAME}={token}"),
        }
    }

    #[cfg(target_os="macos")]
    pub(crate) fn executable(pid:u32)->Option<std::path::PathBuf> {
        use std::os::unix::ffi::OsStringExt;
        let mut path=vec![0u8;4096];
        let n=unsafe{nix::libc::proc_pidpath(pid as i32,path.as_mut_ptr().cast(),path.len() as u32)};
        if n<=0{return None}
        path.truncate(path.iter().position(|b|*b==0).unwrap_or(n as usize));
        Some(std::ffi::OsString::from_vec(path).into())
    }
    #[cfg(target_os="linux")]
    pub(crate) fn executable(pid:u32)->Option<std::path::PathBuf> {std::fs::read_link(format!("/proc/{pid}/exe")).ok()}
    #[cfg(not(any(target_os="macos",target_os="linux")))]
    pub(crate) fn executable(_:u32)->Option<std::path::PathBuf> {None}

    #[cfg(unix)]
    pub(crate) fn member_program(pid:u32)->String {
        process_arguments(pid).and_then(|args|args.into_iter().next())
            .and_then(|program|std::path::Path::new(&program).file_name().map(|p|p.to_string_lossy().into_owned()))
            .unwrap_or_else(||"unknown".into())
    }
    #[cfg(not(unix))]
    pub(crate) fn member_program(_:u32)->String {"unknown".into()}

    #[cfg(any(target_os="macos",target_os="linux"))]
    pub(crate) fn members(&self) -> Vec<(u32,String)> {
        let mut reader=MarkerReader::new();
        process_ids().into_iter().filter(|pid|*pid>1&&reader.has_marker(*pid,self.marker.as_bytes()))
            .filter_map(|pid|crate::runner::process_identity::observe(pid).ok().flatten().map(|birth|(pid,birth))).collect()
    }
    #[cfg(not(any(target_os="macos",target_os="linux")))]
    pub(crate) fn members(&self) -> Vec<(u32,String)> {Vec::new()}

    #[cfg(unix)]
    pub(crate) fn marker(&self) -> &str { &self.marker }

    #[cfg(unix)]
    pub(crate) fn recover(marker: &str) -> Result<Self, String> {
        let suffix = [MARKER_NAME, LEGACY_MARKER_NAME]
            .iter()
            .find_map(|name| marker.strip_prefix(name)?.strip_prefix('='))
            .ok_or("Invalid process marker")?;
        if suffix.is_empty() || suffix.len() > 120 || !suffix.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
            return Err("Invalid process marker".into());
        }
        Ok(Self { marker: marker.into() })
    }

    #[cfg(unix)]
    pub(crate) fn cleanup(&self) -> bool {
        terminate_marked_groups(&self.marker);
        for _ in 0..50 {
            if marked_process_groups(&self.marker).is_empty() { return true; }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[cfg(not(unix))]
    pub(crate) fn marker(&self)->&str {"unsupported"}
    #[cfg(not(unix))]
    pub(crate) fn recover(_: &str)->Result<Self,String> {Err("Process identity unsupported".into())}
    #[cfg(not(unix))]
    pub(crate) fn cleanup(&self)->bool {false}

    /// 아직 살아 있는, 이 턴이 띄운 프로세스 그룹. `exclude`는 집계에서 뺀다.
    ///
    /// `Drop`은 이것들을 죽이지만, 죽이기 전에 **무엇이 남았는지 알리는** 것이 이 메서드의
    /// 몫이다. `turn_guard`의 판정은 하니스 문구에 기대는데(`is_launch_receipt`), 문구가
    /// 바뀌거나 새 하니스가 다른 표현을 쓰면 조용히 뚫린다. 실제 프로세스로 교차 확인한다.
    ///
    /// `exclude`에는 **vendor 자신의 pgid**를 넘겨야 한다. vendor는 자기 프로세스 그룹을
    /// 따로 만들고(`process_group(0)`) 마커도 상속하므로, 빼지 않으면 모든 턴이 걸린다.
    ///
    /// argv는 남을 그룹에 대해서만 읽는다 — 전체 프로세스에 대고 하면 스캔 비용이 배가
    /// 되는데, 매치는 드물고 대부분의 턴에서 결과는 빈 목록이다.
    #[cfg(unix)]
    pub(crate) fn survivors(&self, exclude: u32, vendor: Vendor) -> Vec<Survivor> {
        // A runtime helper may own a separate group while Codex is still emitting
        // its Result. Exclude only that PID, before electing group representatives:
        // a job sharing the helper's group must still be reported.
        let codex_group = (vendor == Vendor::Codex).then_some(exclude);
        marked_group_leaders(&self.marker, codex_group)
            .into_iter()
            .filter(|(group, _)| *group != exclude)
            .map(|(group, pid)| Survivor {
                group,
                arguments: process_arguments(pid),
            })
            .collect()
    }

    #[cfg(not(unix))]
    pub(crate) fn survivors(&self, _exclude: u32, _vendor: Vendor) -> Vec<Survivor> {
        Vec::new()
    }
}

impl Drop for TurnProcessScope {
    fn drop(&mut self) {
        #[cfg(unix)]
        terminate_marked_groups(&self.marker);
    }
}

fn unique_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
    format!("{}-{nanos}-{sequence}", std::process::id())
}

#[cfg(unix)]
fn terminate_marked_groups(marker: &str) {
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;

    let groups = marked_process_groups(marker);
    for group in &groups {
        let _ = killpg(Pid::from_raw(*group as i32), Signal::SIGTERM);
    }
    if groups.is_empty() {
        return;
    }
    std::thread::sleep(Duration::from_millis(200));
    let remaining = marked_process_groups(marker);
    for group in remaining {
        crate::verify::kill_group(group);
    }
}

#[cfg(unix)]
fn marked_process_groups(marker: &str) -> BTreeSet<u32> {
    // Runtime helpers remain owned by the turn and must also be terminated.
    marked_group_leaders(marker, None).into_keys().collect()
}

/// 마커를 가진 프로세스 그룹 → 그 그룹을 대표할 pid.
///
/// 대표는 **그룹 리더**(`pid == pgid`)다 — 그룹을 만든 명령이 무엇인지 가장 잘 말해준다.
/// 리더가 이미 죽고 자식만 남은 그룹도 있으므로, 그때는 가장 작은 pid로 정한다.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn marked_group_leaders(marker: &str, codex_group: Option<u32>) -> BTreeMap<u32, u32> {
    let own_group = unsafe { nix::libc::getpgrp() };
    let mut reader = MarkerReader::new();
    let mut leaders: BTreeMap<u32, u32> = BTreeMap::new();
    for pid in process_ids() {
        if pid <= 1 || !reader.has_marker(pid, marker.as_bytes()) {
            continue;
        }
        let group = unsafe { nix::libc::getpgid(pid as i32) };
        if group <= 1 || group == own_group {
            continue;
        }
        if codex_group.is_some_and(|vendor_group| {
            is_codex_runtime_helper(pid, vendor_group, marker.as_bytes(), &mut reader)
        }) {
            continue;
        }
        let group = group as u32;
        let current = leaders.entry(group).or_insert(pid);
        *current = elect(*current, pid, group);
    }
    // 데몬이 띄운 워커는 데몬의 그룹에 속하므로 그룹째 뺀다. 리더가 아닌 대표는 그룹을
    // 만든 명령이 아니므로 판정 근거가 되지 못한다.
    leaders.retain(|group, pid| {
        *pid != *group || !process_arguments(*pid).is_some_and(|argv| is_shared_build_daemon(&argv))
    });
    leaders
}

/// 턴이 아니라 사용자 환경이 소유하는 JVM 빌드 데몬인가.
///
/// 관측과 정리 양쪽에서 뺀다. 인자 하나만 보면 아무 프로그램이나 클래스 이름을 인자로
/// 넘겨 정리를 피할 수 있으므로, 실행 파일이 `java`인 경우로 좁힌다.
fn is_shared_build_daemon(argv: &[String]) -> bool {
    let Some(program) = argv.first() else {
        return false;
    };
    std::path::Path::new(program).file_name() == Some(std::ffi::OsStr::new("java"))
        && argv[1..]
            .iter()
            .any(|entry| SHARED_BUILD_DAEMONS.contains(&entry.as_str()))
}

/// 두 후보 중 그룹을 대표할 pid. 리더(`pid == group`)가 이기고, 없으면 작은 쪽이 이긴다.
///
/// 순수 함수로 떼어 둔 이유는 **순회 순서에 무관해야** 하기 때문이다. `process_ids()`가
/// 돌려주는 순서는 macOS `proc_listallpids`도 Linux `read_dir`도 보장하지 않는데, 그 위에서
/// 접기(fold)를 하면서 순서 의존이 숨어들면 실행마다 다른 줄이 찍힌다.
fn elect(current: u32, candidate: u32, group: u32) -> u32 {
    if current == group {
        return current;
    }
    if candidate == group || candidate < current {
        return candidate;
    }
    current
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn marked_group_leaders(_marker: &str, _codex_group: Option<u32>) -> BTreeMap<u32, u32> {
    BTreeMap::new()
}

/// Trust the OS executable paths and the direct parent, never argv[0]. Missing
/// observations leave the process in the warning. This is an observation-only
/// exception; neither the helper's descendants nor cleanup inherit it.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn is_codex_runtime_helper(
    pid: u32,
    vendor_group: u32,
    marker: &[u8],
    reader: &mut MarkerReader,
) -> bool {
    let Some(helper) = TurnProcessScope::executable(pid) else {
        return false;
    };
    if helper.file_name().and_then(|name| name.to_str()) != Some("codex-code-mode-host") {
        return false;
    }
    let Some(parent) = process_parent(pid).filter(|parent| *parent > 1) else {
        return false;
    };
    if unsafe { nix::libc::getpgid(parent as i32) } != vendor_group as i32
        || !reader.has_marker(parent, marker)
    {
        return false;
    }
    let Some(codex) = TurnProcessScope::executable(parent) else {
        return false;
    };
    if codex.file_name().and_then(|name| name.to_str()) != Some("codex")
        || helper != codex.with_file_name("codex-code-mode-host")
    {
        return false;
    }
    if parent == vendor_group {
        return true;
    }
    // The npm launcher is node -> native codex, both in the vendor's group.
    // A nested codex launched as user work is not the top-level runtime.
    process_parent(parent) == Some(vendor_group)
        && reader.has_marker(vendor_group, marker)
        && TurnProcessScope::executable(vendor_group)
            .and_then(|path| path.file_name().map(|name| name == "node"))
            == Some(true)
}

#[cfg(target_os = "macos")]
fn process_parent(pid: u32) -> Option<u32> {
    let mut info = std::mem::MaybeUninit::<nix::libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<nix::libc::proc_bsdinfo>() as i32;
    let read = unsafe {
        nix::libc::proc_pidinfo(
            i32::try_from(pid).ok()?,
            nix::libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    (read == size).then(|| unsafe { info.assume_init() }.pbi_ppid)
}

#[cfg(target_os = "linux")]
fn process_parent(pid: u32) -> Option<u32> {
    parse_proc_parent(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

#[cfg(any(target_os = "linux", all(test, unix)))]
fn parse_proc_parent(stat: &str) -> Option<u32> {
    // comm can contain spaces and parentheses; state and ppid follow its last ')'.
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
#[path = "process_cleanup_runtime_tests.rs"]
mod runtime_tests;

/// 시스템의 모든 pid.
///
/// `proc_listallpids`의 반환값은 PID **개수**, buffersize 인자만 **바이트 수**다.
/// 반환값까지 sizeof(pid)로 나누면 할당과 truncate 양쪽에서 목록을 줄여,
/// 오래된 자식 프로세스를 관측·정리하지 못한다.
#[cfg(target_os = "macos")]
fn process_ids() -> Vec<u32> {
    const PID_SIZE: usize = std::mem::size_of::<i32>();
    /// 조회와 실제 읽기 사이에 새로 뜬 프로세스를 담을 여유분.
    const HEADROOM: usize = 64;

    let probed_count = unsafe { nix::libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if probed_count <= 0 {
        return Vec::new();
    }
    let mut pids = vec![0i32; probed_count as usize + HEADROOM];
    let capacity_bytes = pids
        .len()
        .checked_mul(PID_SIZE)
        .and_then(|size| i32::try_from(size).ok())
        .unwrap_or(i32::MAX);
    let written_count = unsafe {
        nix::libc::proc_listallpids(pids.as_mut_ptr().cast(), capacity_bytes)
    }
    .max(0) as usize;
    pids.truncate(written_count.min(pids.len()));
    pids.into_iter()
        .filter_map(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid > 0)
        .collect()
}

#[cfg(target_os = "macos")]
struct MarkerReader {
    buffer: Vec<u8>,
}

#[cfg(target_os = "macos")]
impl MarkerReader {
    fn new() -> Self {
        Self {
            buffer: vec![0u8; argument_capacity()],
        }
    }

    /// **SIP 보호 바이너리는 여기서 잡히지 않는다.** macOS는 restricted 실행 파일
    /// (`/bin/sh`, `/bin/sleep`, `/usr/bin/env` 등)의 환경변수를 `KERN_PROCARGS2`에서
    /// 가린다 — argv는 주지만 env 영역이 비어 마커가 보이지 않는다. 그래서 `sh -c`로 띄운 뒤
    /// `setsid`로 그룹을 탈출한 자식은 관측도 정리도 되지 않는다. 실측으로 확인했다:
    /// `/bin/sleep`·`/bin/sh`는 false, Xcode의 python3는 true. 관측되는 것은 사용자 설치
    /// 바이너리(node·python3·직접 빌드한 실행 파일)이며, 실제로 문제를 일으키는 장기 실행
    /// 프로세스는 대개 그쪽이다.
    fn has_marker(&mut self, pid: u32, marker: &[u8]) -> bool {
        let Ok(pid) = i32::try_from(pid) else {
            return false;
        };
        let mut size = self.buffer.len();
        let mut mib = [nix::libc::CTL_KERN, nix::libc::KERN_PROCARGS2, pid];
        let result = unsafe {
            nix::libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as u32,
                self.buffer.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        result == 0 && contains_entry(&self.buffer[..size.min(self.buffer.len())], marker)
    }
}

/// `kern.argmax`를 못 읽었을 때 쓸 버퍼 크기.
///
/// 0을 돌려주면 **기능 전체가 조용히 죽는다** — `MarkerReader`의 버퍼가 비어 `has_marker`가
/// 항상 false가 되고, 관측도 정리도 하지 않은 채 로그 한 줄 없이 턴이 깨끗해 보인다.
/// sysctl 실패는 기능을 끄는 사유가 아니라 보수적인 크기로 계속 도는 사유다.
#[cfg(target_os = "macos")]
const FALLBACK_ARGUMENT_CAPACITY: usize = 256 * 1024;

/// 할당 방어 상한. 손상된 `kern.argmax`가 터무니없는 크기를 요구하는 것을 막는다.
///
/// **초과를 거부하지 않고 잘라 쓴다.** 오늘 macOS의 `kern.argmax`가 정확히 1MiB인데, 거부
/// 방식이면 Apple이 값을 올리는 순간 관측이 통째로 멈춘다 — 상한은 방어이지 계약이 아니다.
#[cfg(target_os = "macos")]
const MAX_ARGUMENT_CAPACITY: usize = 4 * 1024 * 1024;

/// `KERN_PROCARGS2`를 읽을 버퍼 크기. **항상 0보다 크다** — 호출자는 0을 처리하지 않아도 된다.
#[cfg(target_os = "macos")]
fn argument_capacity() -> usize {
    let mut argmax = 0i32;
    let mut argmax_size = std::mem::size_of::<i32>();
    let mut mib = [nix::libc::CTL_KERN, nix::libc::KERN_ARGMAX];
    let result = unsafe {
        nix::libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            (&mut argmax as *mut i32).cast(),
            &mut argmax_size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result == 0 && argmax > 0 {
        return (argmax as usize).min(MAX_ARGUMENT_CAPACITY);
    }
    static WARNED: std::sync::Once = std::sync::Once::new();
    WARNED.call_once(|| {
        eprintln!(
            "[praxis] kern.argmax 조회 실패 (sysctl={result}, argmax={argmax}) — \
             {FALLBACK_ARGUMENT_CAPACITY}바이트로 폴백합니다. 턴 가드의 프로세스 관측은 계속 돕니다."
        );
    });
    FALLBACK_ARGUMENT_CAPACITY
}

/// 프로세스의 argv. 마커가 매치된 **대표 pid에 대해서만** 호출한다.
///
/// 마커를 읽을 때와 같은 `KERN_PROCARGS2`를 다시 읽는다. 그사이 프로세스가 죽으면 sysctl이
/// 실패해 `None`이 된다 — 경고는 그대로 나가고 그 줄만 번호로 남는다. argv를 못 얻는 것이
/// 경고를 삼킬 이유는 되지 못한다.
#[cfg(target_os = "macos")]
fn process_arguments(pid: u32) -> Option<Vec<String>> {
    let pid = i32::try_from(pid).ok()?;
    let capacity = argument_capacity();
    let mut buffer = vec![0u8; capacity];
    let mut size = buffer.len();
    let mut mib = [nix::libc::CTL_KERN, nix::libc::KERN_PROCARGS2, pid];
    let result = unsafe {
        nix::libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            buffer.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return None;
    }
    buffer.truncate(size.min(capacity));
    non_empty(procargs2_arguments(&buffer))
}

/// `KERN_PROCARGS2` 버퍼에서 argv를 꺼낸다.
///
/// 레이아웃은 `[argc:i32][exec_path\0][정렬용 널…][argv[0]\0]…[argv[argc-1]\0][환경변수…]`.
/// exec_path 뒤 패딩 길이가 고정이 아니라, 널을 건너뛰며 첫 인자를 찾아야 한다. argc를 세지
/// 않고 널로만 끊으면 뒤따르는 환경변수까지 딸려와 마커 토큰이 화면에 실린다.
///
/// argc를 세는 것만으로는 부족하다. `argv[0]`이 빈 문자열이면(execve가 허용한다) 패딩
/// 건너뛰기가 그것까지 삼켜 창이 한 칸 밀리고, `take(argc)`가 환경변수 영역을 물게 된다.
/// 그래서 마커로 시작하는 엔트리를 만나면 거기서 멈춘다 — 유출을 막고 싶은 대상이 정확히
/// 그것이고, 환경변수 영역의 시작을 다른 방법으로는 알 수 없다.
///
/// `#[cfg]`를 걸지 않는다. 순수 바이트 파서라 플랫폼 의존이 없고, macOS에서만 컴파일하면
/// Linux 러너뿐인 CI에서 이 함수가 **한 번도 실행되지 않는다**.
fn procargs2_arguments(buffer: &[u8]) -> Vec<String> {
    let Some(head) = buffer.get(..4) else {
        return Vec::new();
    };
    let count = i32::from_ne_bytes([head[0], head[1], head[2], head[3]]);
    if count <= 0 {
        return Vec::new();
    }
    let count = (count as usize).min(ARGUMENT_LIMIT);
    let rest = &buffer[4..];
    let Some(path_end) = rest.iter().position(|byte| *byte == 0) else {
        return Vec::new();
    };
    let rest = &rest[path_end..];
    let Some(first) = rest.iter().position(|byte| *byte != 0) else {
        return Vec::new();
    };
    rest[first..]
        .split(|byte| *byte == 0)
        .take(count)
        .take_while(|entry| !starts_with_marker(entry))
        .map(|entry| String::from_utf8_lossy(entry).into_owned())
        .collect()
}

/// 이 엔트리가 우리 마커 환경변수인가 — argv 영역이 끝났다는 유일한 신호.
fn starts_with_marker(entry: &[u8]) -> bool {
    [MARKER_NAME, LEGACY_MARKER_NAME]
        .iter()
        .any(|name| entry.starts_with(name.as_bytes()) && entry.get(name.len()) == Some(&b'='))
}

/// `/proc/<pid>/cmdline` 바이트에서 argv를 꺼낸다. macOS 파서와 같은 규칙을 따른다.
///
/// 빈 엔트리를 버리지 않는다 — `sh -c '' foo`의 빈 인자를 지우면 위치가 밀려
/// `command_line`이 실행 파일로 착각하는 것이 argv[1]이 된다. 마지막 널 뒤 빈 꼬리만 뗀다.
///
/// Linux 밖에서는 호출자가 없지만 `#[cfg]`로 잘라내지 않는다. 순수 바이트 파서라 어디서든
/// 돌고, 잘라내면 macOS 개발기에서 이 파서의 테스트가 통째로 사라진다.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn cmdline_arguments(raw: &[u8]) -> Vec<String> {
    let trimmed = raw.strip_suffix(&[0]).unwrap_or(raw);
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed
        .split(|byte| *byte == 0)
        .take(ARGUMENT_LIMIT)
        .take_while(|entry| !starts_with_marker(entry))
        .map(|entry| String::from_utf8_lossy(entry).into_owned())
        .collect()
}

/// 남길 것이 하나도 없는 argv는 "읽었다"고 하지 않는다 — 화면에는 어차피 같은 말이 나가지만,
/// `Some(vec![""])`은 읽기에 성공한 척하면서 아무것도 알려주지 않는다.
fn non_empty(parsed: Vec<String>) -> Option<Vec<String>> {
    parsed
        .iter()
        .any(|entry| !entry.trim().is_empty())
        .then_some(parsed)
}

#[cfg(target_os = "linux")]
fn process_arguments(pid: u32) -> Option<Vec<String>> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    non_empty(cmdline_arguments(&raw))
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn process_arguments(_pid: u32) -> Option<Vec<String>> {
    None
}

#[cfg(target_os = "linux")]
fn process_ids() -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_string_lossy().parse::<u32>().ok())
        .collect()
}

#[cfg(target_os = "linux")]
struct MarkerReader;

#[cfg(target_os = "linux")]
impl MarkerReader {
    fn new() -> Self {
        Self
    }

    fn has_marker(&mut self, pid: u32, marker: &[u8]) -> bool {
        std::fs::read(format!("/proc/{pid}/environ"))
            .map(|environment| contains_entry(&environment, marker))
            .unwrap_or(false)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn contains_entry(buffer: &[u8], marker: &[u8]) -> bool {
    buffer.split(|byte| *byte == 0).any(|entry| entry == marker)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// 아무도 달지 않은 마커로는 아무것도 잡히지 않는다 — 스캔이 무관한 프로세스를
    /// 긁어오지 않는다는 최소 보장. 실제 탐지·오탐 시나리오는 통합 테스트가 본다
    /// (`tests/convo_process_cleanup_test.rs`).
    #[test]
    fn an_unused_marker_matches_nothing() {
        let groups = marked_process_groups("DOJANG_TURN_TOKEN=marker-that-nobody-carries");
        assert!(groups.is_empty(), "무관한 그룹이 잡혔다: {groups:?}");
    }

    #[test]
    fn survivors_drops_the_excluded_group() {
        let mut command = Command::new("/bin/sh");
        let scope = TurnProcessScope::attach(&mut command);
        // 이 스코프로는 아무 프로세스도 띄우지 않았으므로 어느 쪽이든 비어야 한다.
        // 실제로 잡히는 경우의 제외 동작은 survivors_finds_a_marked_child_… 가 본다.
        assert!(scope.survivors(0, Vendor::Claude).is_empty());
    }

    /// 마커를 단 자식의 argv는 읽히되 그 환경변수는 섞여 들어오면 안 된다.
    ///
    /// 자기 자신을 읽으면 오염 검사가 무의미하다 — 테스트 바이너리에는 마커가 없어서 파서가
    /// 환경변수 영역까지 넘어 읽어도 assert가 통과한다. 마커를 심은 자식을 봐야 한다.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn process_arguments_reads_a_childs_argv_without_its_environment() {
        let mut command = sleeper();
        let scope = TurnProcessScope::attach(&mut command);
        let mut child = command.spawn().expect("자식을 못 띄웠다");

        let parsed = wait_for(|| process_arguments(child.id())).expect("자식 argv를 못 읽었다");
        assert!(
            parsed.iter().any(|entry| entry.contains(SLEEPER_SCRIPT)),
            "자식의 argv가 아니다: {parsed:?}"
        );
        assert!(
            !parsed.iter().any(|entry| entry.contains(MARKER_NAME)),
            "환경변수가 argv에 딸려 왔다: {parsed:?}"
        );

        drop(scope); // 마커 그룹을 정리한다.
        child.kill().ok();
        child.wait().ok();
    }

    /// 잠깐 살아 있을 자식. **SIP 보호 바이너리를 쓰면 안 된다.**
    ///
    /// macOS는 SIP로 보호된 실행 파일(`/bin/sh`, `/bin/sleep`, `/usr/bin/env` …)의 환경변수를
    /// `KERN_PROCARGS2`에서 가린다 — argv는 주지만 env는 빈다. 그런 자식은 마커가 안 보여
    /// 스캔에 잡히지 않으므로 이 모듈을 검증할 수 없다. Xcode/Command Line Tools의 python3는
    /// SIP 밖이라 환경변수가 보인다(통합 테스트도 같은 이유로 python3를 쓴다).
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    const SLEEPER_SCRIPT: &str = "import time; time.sleep(5)";

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn sleeper() -> Command {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new("/usr/bin/python3");
        command.args(["-c", SLEEPER_SCRIPT]).process_group(0); // 자기 그룹의 리더가 된다 ⇒ pgid == pid
        command
    }

    /// `spawn`이 반환해도 exec은 아직 끝나지 않았을 수 있다 — 그때까지 커널은 옛 argv를
    /// 보여준다. 조건이 설 때까지 짧게 폴링한다.
    ///
    /// 예산이 3초인 이유는 이 테스트가 **전체 스위트와 함께** 돌기 때문이다. 코어가 포화된
    /// 상태에서는 인터프리터 하나 뜨는 데도 1초를 넘길 수 있고, 그때 상한이 짧으면 코드가
    /// 아니라 부하가 테스트를 떨어뜨린다.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn wait_for<T>(mut probe: impl FnMut() -> Option<T>) -> Option<T> {
        for _ in 0..300 {
            if let Some(value) = probe() {
                return Some(value);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    /// 죽은 pid는 조회에 실패할 뿐, 스캔 전체를 무너뜨리지 않는다.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn a_dead_pid_yields_no_arguments() {
        // pid 0은 커널 소유라 KERN_PROCARGS2 / /proc 어느 쪽으로도 argv를 주지 않는다.
        assert_eq!(process_arguments(0), None);
    }

    /// 마커를 단 자식은 생존자로 잡히고, 그 그룹을 `exclude`로 주면 빠진다.
    ///
    /// 프로세스를 하나도 안 띄우고 빈 목록만 확인하면 `survivors`의 필터를 통째로 지워도
    /// 통과한다 — 실제로 잡히는 것이 있어야 제외가 검증된다.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn survivors_finds_a_marked_child_and_drops_the_excluded_group() {
        let mut command = sleeper();
        let scope = TurnProcessScope::attach(&mut command);
        let mut child = command.spawn().expect("자식을 못 띄웠다");
        let group = child.id();

        // 폴링 조건이 argv까지 보는 이유: 마커가 보이는 시점과 argv가 제 것이 되는 시점이
        // 다르다. exec 전에는 커널이 부모의 옛 argv를 주거나 조회가 실패해 `arguments`가
        // `None`이다 — 둘 다 `Survivor`의 정상 상태다. 그룹만 보고 멈추면 그 창에서 깨진다.
        let mine = wait_for(|| {
            scope.survivors(0, Vendor::Claude).into_iter().find(|survivor| {
                survivor.group == group
                    && survivor
                        .arguments
                        .as_ref()
                        .is_some_and(|argv| argv.iter().any(|e| e.contains(SLEEPER_SCRIPT)))
            })
        })
        .expect("마커 단 자식의 argv가 안 잡혔다");
        let argv = mine.arguments.expect("폴링을 통과했으면 argv가 실려 있다");
        assert!(
            argv.iter().any(|entry| entry.contains(SLEEPER_SCRIPT)),
            "엉뚱한 프로세스의 argv다: {argv:?}"
        );
        assert!(
            scope.survivors(group, Vendor::Claude).iter().all(|s| s.group != group),
            "exclude로 준 그룹은 빠져야 한다"
        );

        drop(scope);
        child.kill().ok();
        child.wait().ok();
    }

    /// 대표 선정은 순회 순서에 무관해야 한다 — `process_ids()`는 순서를 보장하지 않는다.
    #[test]
    fn the_group_leader_wins_regardless_of_arrival_order() {
        // 그룹 500, 구성원 {500(리더), 700, 900}. 어느 순서로 접어도 리더가 남아야 한다.
        for order in [[500, 700, 900], [900, 700, 500], [700, 900, 500]] {
            let mut current = order[0];
            for candidate in &order[1..] {
                current = elect(current, *candidate, 500);
            }
            assert_eq!(current, 500, "순서 {order:?}에서 리더를 놓쳤다");
        }
    }

    /// 리더가 이미 죽은 그룹은 가장 작은 pid가 대표가 된다 — 이것도 순서에 무관해야 한다.
    #[test]
    fn without_a_leader_the_smallest_pid_wins() {
        for order in [[700, 900, 1100], [1100, 900, 700], [900, 1100, 700]] {
            let mut current = order[0];
            for candidate in &order[1..] {
                current = elect(current, *candidate, 500);
            }
            assert_eq!(current, 700, "순서 {order:?}에서 최소 pid를 놓쳤다");
        }
    }

    fn procargs2(argc: i32, exec_path: &str, entries: &[&str]) -> Vec<u8> {
        let mut buffer = argc.to_ne_bytes().to_vec();
        buffer.extend_from_slice(exec_path.as_bytes());
        buffer.extend_from_slice(&[0, 0, 0]); // 종료 널 + 정렬 패딩
        for entry in entries {
            buffer.extend_from_slice(entry.as_bytes());
            buffer.push(0);
        }
        buffer
    }

    /// argc를 세지 않고 널로만 끊으면 argv 뒤 환경변수까지 딸려온다 — 마커 토큰이 화면에 실린다.
    #[test]
    fn arguments_stop_at_argc_and_skip_the_exec_path() {
        let buffer = procargs2(
            2,
            "/bin/echo",
            &["echo", "hello", "DOJANG_TURN_TOKEN=1-2-3"],
        );
        assert_eq!(procargs2_arguments(&buffer), vec!["echo", "hello"]);
    }

    /// argc가 실제 argv보다 크면 argc만으로는 환경변수를 막지 못한다 — 마커에서 멈춰야 한다.
    #[test]
    fn a_lying_argc_still_cannot_leak_the_marker() {
        let buffer = procargs2(
            4,
            "/bin/echo",
            &["echo", "hello", "DOJANG_TURN_TOKEN=1-2-3"],
        );
        assert_eq!(procargs2_arguments(&buffer), vec!["echo", "hello"]);
    }

    /// `argv[0]`이 빈 문자열이면 패딩 건너뛰기가 그것을 삼켜 창이 한 칸 밀린다.
    /// 그 상태에서도 마커는 새어 나가면 안 된다.
    #[test]
    fn an_empty_argv0_does_not_leak_the_marker() {
        let buffer = procargs2(3, "/x", &["", "a", "b", "DOJANG_TURN_TOKEN=1-2-3"]);
        let parsed = procargs2_arguments(&buffer);
        assert!(
            !parsed.iter().any(|entry| entry.contains(MARKER_NAME)),
            "마커가 새어 나왔다: {parsed:?}"
        );
    }

    /// 이름이 같아도 `=`가 붙지 않으면 우리 마커가 아니다 — 멀쩡한 인자를 잘라선 안 된다.
    #[test]
    fn a_lookalike_argument_is_not_mistaken_for_the_marker() {
        let buffer = procargs2(2, "/bin/x", &["x", "DOJANG_TURN_TOKENISH"]);
        assert_eq!(
            procargs2_arguments(&buffer),
            vec!["x", "DOJANG_TURN_TOKENISH"]
        );
    }

    #[test]
    fn a_truncated_buffer_yields_nothing() {
        assert!(procargs2_arguments(&[]).is_empty());
        assert!(procargs2_arguments(&[1, 0, 0]).is_empty());
        assert!(procargs2_arguments(&procargs2(0, "/bin/echo", &["echo"])).is_empty());
        assert!(procargs2_arguments(&i32::MIN.to_ne_bytes()).is_empty());
        // exec_path 뒤에 널이 하나도 없는 버퍼.
        assert!(procargs2_arguments(&[2, 0, 0, 0, b'/', b'b']).is_empty());
        // 패딩만 있고 argv가 없는 버퍼.
        assert!(procargs2_arguments(&[2, 0, 0, 0, b'/', 0, 0, 0]).is_empty());
    }

    /// argv는 프로세스가 정하는 값이라 유효한 UTF-8이라는 보장이 없다.
    #[test]
    fn invalid_utf8_does_not_panic() {
        let mut buffer = 1i32.to_ne_bytes().to_vec();
        buffer.extend_from_slice(b"/x\0\0");
        buffer.extend_from_slice(&[0xff, 0xfe, 0]);
        assert_eq!(procargs2_arguments(&buffer).len(), 1);
    }

    /// Linux 쪽 파서도 같은 규칙을 지켜야 한다 — 빈 인자를 지우면 위치가 밀린다.
    #[test]
    fn cmdline_keeps_empty_arguments_but_stops_at_the_marker() {
        assert_eq!(
            cmdline_arguments(b"sh\0-c\0\0foo\0"),
            vec!["sh", "-c", "", "foo"]
        );
        assert_eq!(
            cmdline_arguments(b"node\0server.js\0DOJANG_TURN_TOKEN=1-2-3\0"),
            vec!["node", "server.js"]
        );
        // 이름을 바꾸기 전 버전이 띄운 프로세스의 마커도 argv 끝으로 본다.
        assert_eq!(
            cmdline_arguments(b"node\0server.js\0PRAXIS_TURN_TOKEN=1-2-3\0"),
            vec!["node", "server.js"]
        );
        assert!(cmdline_arguments(b"").is_empty());
        assert!(cmdline_arguments(b"\0").is_empty());
    }

    /// 새로 생긴 자기 자신만 확인하면 목록 앞부분만 읽는 버그를 놓친다.
    /// 독립적인 ps 관측 전후에 계속 살아 있는 모든 PID가 조회되어야 한다.
    #[test]
    #[cfg(target_os = "macos")]
    fn process_ids_includes_all_stable_processes() {
        fn observed_pids() -> std::collections::BTreeSet<u32> {
            let output = Command::new("/bin/ps")
                .args(["-axo", "pid="])
                .output()
                .expect("ps must run");
            assert!(output.status.success(), "ps failed");
            String::from_utf8(output.stdout)
                .unwrap()
                .split_whitespace()
                .map(|pid| pid.parse::<u32>().unwrap())
                .filter(|pid| *pid > 0)
                .collect()
        }

        let before = observed_pids();
        let pids = process_ids();
        let after = observed_pids();
        let missing: Vec<_> = before
            .intersection(&after)
            .filter(|pid| !pids.contains(pid))
            .copied()
            .collect();
        assert!(missing.is_empty(), "stable processes omitted: {missing:?}");
        assert!(!pids.is_empty(), "프로세스가 하나도 안 잡혔다");
        assert!(
            pids.iter().all(|pid| *pid > 0),
            "0 패딩이 섞였다 ({}건 중 {}건이 0)",
            pids.len(),
            pids.iter().filter(|pid| **pid == 0).count()
        );
        assert!(
            pids.contains(&std::process::id()),
            "자기 자신이 목록에 없다 — 과소 할당이거나 잘못 잘렸다"
        );
    }

    /// 버퍼 크기는 **절대 0이 되지 않는다**. 0이면 `has_marker`가 항상 false가 되어
    /// 관측도 정리도 조용히 멈춘다 — 실패해도 폴백으로 계속 도는 것이 계약이다.
    #[test]
    #[cfg(target_os = "macos")]
    fn argument_capacity_never_disables_observation() {
        let capacity = argument_capacity();
        assert!(capacity > 0, "버퍼 크기가 0이면 기능이 조용히 죽는다");
        assert!(
            capacity <= MAX_ARGUMENT_CAPACITY,
            "방어 상한을 넘겼다: {capacity}"
        );
    }

    /// 남길 것이 없는 argv는 "읽었다"고 하지 않는다.
    #[test]
    fn a_blank_argv_is_not_a_successful_read() {
        assert_eq!(non_empty(vec![]), None);
        assert_eq!(non_empty(vec![String::new()]), None);
        assert_eq!(non_empty(vec!["  ".to_string()]), None);
        assert_eq!(
            non_empty(vec!["sh".to_string()]),
            Some(vec!["sh".to_string()])
        );
    }

    fn argv(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|entry| entry.to_string()).collect()
    }

    #[test]
    fn a_java_build_daemon_is_shared() {
        assert!(is_shared_build_daemon(&argv(&[
            "/opt/homebrew/Cellar/openjdk@17/17.0.17/libexec/openjdk.jdk/Contents/Home/bin/java",
            "--add-opens=java.base/java.util=ALL-UNNAMED",
            "-cp",
            "/Users/x/.gradle/wrapper/dists/gradle-8.14.3/lib/gradle-daemon-main-8.14.3.jar",
            "org.gradle.launcher.daemon.bootstrap.GradleDaemon",
            "8.14.3",
        ])));
        assert!(is_shared_build_daemon(&argv(&[
            "java",
            "-cp",
            "kotlin-daemon.jar",
            "org.jetbrains.kotlin.daemon.KotlinCompileDaemon",
        ])));
    }

    /// 클래스 이름을 인자로 넘기는 것만으로 정리를 피할 수 없어야 한다.
    #[test]
    fn a_class_name_alone_does_not_make_a_shared_daemon() {
        assert!(!is_shared_build_daemon(&argv(&[
            "/usr/bin/python3",
            "server.py",
            "org.gradle.launcher.daemon.bootstrap.GradleDaemon",
        ])));
        assert!(!is_shared_build_daemon(&argv(&["/usr/bin/java", "-jar", "app.jar"])));
        assert!(!is_shared_build_daemon(&argv(&[
            "/opt/bin/javaw",
            "org.gradle.launcher.daemon.bootstrap.GradleDaemon",
        ])));
        assert!(!is_shared_build_daemon(&[]));
    }

    /// 이전 버전이 DB에 남긴 마커로도 크래시 복구 정리가 돌아야 한다.
    #[test]
    #[cfg(unix)]
    fn recover_accepts_the_current_and_the_legacy_marker() {
        assert!(TurnProcessScope::recover("DOJANG_TURN_TOKEN=1-2-3").is_ok());
        assert!(TurnProcessScope::recover("PRAXIS_TURN_TOKEN=1-2-3").is_ok());
        assert!(TurnProcessScope::recover("DOJANG_TURN_TOKENX=1-2-3").is_err());
        assert!(TurnProcessScope::recover("OTHER=1-2-3").is_err());
        assert!(TurnProcessScope::recover("DOJANG_TURN_TOKEN=").is_err());
    }

    /// 마커를 단 빌드 데몬 그룹은 생존자로 잡히지 않고, 스코프가 끝나도 죽지 않는다.
    ///
    /// SIP 밖 python3의 실제 바이너리를 `java`라는 심볼릭 링크로 띄워 데몬 모양의 argv를
    /// 만든다. `/usr/bin/python3`는 macOS에서 실제 인터프리터를 다시 exec하는 셸이라
    /// argv[0]를 바꿔도 남지 않으므로, 한 번 띄워 본 뒤 커널이 보고한 실행 파일을 쓴다.
    /// 데몬이 아닌 자식을 함께 띄워, 관측 자체가 동작하는데도 데몬만 빠진다는 것을 확인한다.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn a_marked_build_daemon_is_neither_reported_nor_killed() {
        use std::os::unix::process::CommandExt;
        let mut probe = sleeper().spawn().expect("python3를 못 띄웠다");
        let interpreter = wait_for(|| {
            let path = TurnProcessScope::executable(probe.id())?;
            // macOS에서는 셸 체인(python3 → xcrun → …)이 끝난 뒤의 프레임워크 바이너리가
            // 필요하다. 중간 단계를 잡으면 링크가 다시 exec되어 argv[0]가 사라진다.
            (!cfg!(target_os = "macos") || path.file_name()? == "Python").then_some(path)
        })
        .expect("python3의 실제 실행 파일을 못 찾았다");
        probe.kill().ok();
        probe.wait().ok();
        let directory = std::env::temp_dir().join(format!("dojang-daemon-{}", unique_token()));
        std::fs::create_dir_all(&directory).unwrap();
        let java = directory.join("java");
        std::os::unix::fs::symlink(&interpreter, &java).unwrap();

        let mut daemon = Command::new(&java);
        daemon
            .args([
                "-c",
                SLEEPER_SCRIPT,
                "org.gradle.launcher.daemon.bootstrap.GradleDaemon",
            ])
            .process_group(0);
        let scope = TurnProcessScope::attach(&mut daemon);
        let mut daemon = daemon.spawn().expect("데몬 모양 자식을 못 띄웠다");
        let mut control = sleeper();
        control.env(MARKER_NAME, scope.marker().split_once('=').unwrap().1);
        let mut control = control.spawn().expect("대조군 자식을 못 띄웠다");

        wait_for(|| {
            let observed = process_arguments(daemon.id())?;
            observed
                .iter()
                .any(|entry| entry.ends_with("GradleDaemon"))
                .then_some(())
        })
        .expect("데몬 모양 자식의 argv가 안 잡혔다");
        wait_for(|| {
            scope
                .survivors(0, Vendor::Claude)
                .iter()
                .any(|survivor| survivor.group == control.id())
                .then_some(())
        })
        .expect("대조군이 안 잡히면 관측 자체가 동작하지 않는 것이다");
        let survivors = scope.survivors(0, Vendor::Claude);
        assert!(
            survivors.iter().all(|survivor| survivor.group != daemon.id()),
            "빌드 데몬이 생존자로 보고됐다: {survivors:?}"
        );

        drop(scope);
        assert!(
            daemon.try_wait().expect("상태 조회 실패").is_none(),
            "빌드 데몬이 턴 정리에서 죽었다"
        );
        assert!(
            wait_for(|| control.try_wait().ok().flatten()).is_some(),
            "대조군은 턴 정리에서 죽어야 한다"
        );
        daemon.kill().ok();
        daemon.wait().ok();
        std::fs::remove_dir_all(&directory).ok();
    }
}
