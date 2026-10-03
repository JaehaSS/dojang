//! 앱 시작 시 벤더 CLI 자동 업데이트.
//!
//! ## 왜 하필 시작 시점인가
//!
//! `agent_action_open`은 활성 작업이 하나라도 있으면 업데이트를 거부한다 — 바이너리를
//! 갈아치우면 돌던 작업이 중간에 깨지기 때문이다. 그런데 앱을 켜고 곧바로 작업을 시작하면
//! 그 조건은 그날 내내 맞지 않는다. **시작 직후가 구조적으로 유일하게 열려 있는 창이다.**
//!
//! ## 그 창은 저절로 닫히지 않는다
//!
//! 업데이트가 도는 수십 초 사이에 사용자가 작업을 시작하면 발밑에서 바이너리가 바뀐다.
//! 손으로 누르던 시절에는 사용자가 터미널을 보고 있어서 덜 위험했지만, 자동이면 이 창이
//! 매번 조용히 열린다. 그래서 도는 동안 `AppState::updating`으로 작업 생성을 막는다.
//!
//! 그 플래그를 되돌리는 일은 [`UpdateGuard`]에게 맡긴다. 타임아웃·조기 반환·패닉·태스크
//! 취소 어느 경로로 빠져나가도 풀려야 하는데, 한 군데라도 빠뜨리면 **앱이 재시작할 때까지
//! 작업을 하나도 만들 수 없게 된다.** 실패의 대가가 기능 자체보다 크므로 사람 손에 두지 않는다.
//!
//! **갱신할 것이 없으면 플래그를 아예 잡지 않는다.** 대부분의 실행은 업데이트가 없는데,
//! 스냅샷 조회(CLI 프로브 + 레지스트리 왕복, 수 초)까지 플래그를 든 채 하면 평상시 매 실행마다
//! 그만큼 작업 생성이 막힌다. 조회는 읽기라 바이너리를 건드리지 않으므로 밖에서 해도 안전하다.
//!
//! ## 프레임워크를 모른다
//!
//! 이 모듈은 `tauri`에 의존하지 않는다. 결과를 어디에 저장하고 무엇으로 알릴지는 조립 지점
//! (`lib.rs`)이 정한다 — 그래야 [`execute`]가 단위 테스트 안에서 돌 수 있다.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

use super::action::{command_for, ActionKind};
use super::detect;

/// 설정 키. 미설정이면 꺼진 것으로 본다 — 사용자의 전역 CLI를 바꾸는 일이라 명시적 동의가 먼저다.
pub const SETTING_KEY: &str = "auto_update_on_launch";

/// 결과가 도착했음을 알리는 이벤트 이름. 이미 설정 패널이 열려 있을 때를 위한 것이고,
/// **유일한 전달 수단은 아니다** — 놓쳐도 `auto_update_last`로 읽을 수 있다.
pub const EVENT: &str = "autoupdate://done";

/// 벤더 하나에 허용하는 시간. npm 전역 설치가 레지스트리를 타는 시간을 감안한 값이다.
/// 넘으면 죽이고 다음 벤더로 간다 — 하나가 매달려 작업 생성을 영영 막게 두지 않는다.
pub const UPDATE_TIMEOUT: Duration = Duration::from_secs(300);

/// 실패 출력에서 사람에게 보여줄 최대 길이. 설치 실패 로그는 수십 KB가 예사다.
const DETAIL_LIMIT: usize = 400;

/// 업데이트할 대상 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub vendor: String,
    pub label: String,
    /// 업데이트 전 버전.
    pub installed: Option<String>,
}

/// 업데이트 한 건의 결과.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UpdateOutcome {
    pub vendor: String,
    pub label: String,
    pub from: Option<String>,
    /// 업데이트 후 실제로 읽어낸 버전. 명령이 성공해도 버전이 그대로일 수 있다.
    pub to: Option<String>,
    pub ok: bool,
    /// 실패 사유. 성공이면 None.
    pub error: Option<String>,
}

/// 한 번의 시작 시 자동 업데이트 전체 결과.
#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct AutoUpdateReport {
    pub outcomes: Vec<UpdateOutcome>,
    /// 업데이트를 하지 않았다면 그 이유. **성공적으로 아무것도 안 한 경우도 채운다** —
    /// "돌아봤지만 최신이었다"와 "아예 돌지 않았다"가 UI에서 같아 보이면 안 된다.
    pub skipped: Option<String>,
    pub finished_at: i64,
}

/// 설정값 해석. **미설정은 꺼짐이다** — 사용자의 전역 CLI(`npm -g`·`brew`)를 교체하는 일이라
/// 설치 직후 동의 없이 돌면 안 된다(1.0 공개 배포 결정).
///
/// 명시적으로 켠 흔한 참 표기만 켜짐으로 본다 — 알 수 없는 값으로 바이너리가 교체되면 안 된다.
pub fn enabled_from(raw: Option<&str>) -> bool {
    let Some(raw) = raw else { return false };
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "on" | "yes"
    )
}

/// 설정에 저장할 문자열. `enabled_from`과 짝이며, 둘이 어긋나면 사용자가 끈 설정이 무시된다.
pub fn setting_value(enabled: bool) -> &'static str {
    if enabled {
        "true"
    } else {
        "false"
    }
}

/// 업데이트를 걸 수 있는 벤더인가.
///
/// 판정을 여기서 다시 세우지 않고 `command_for`에게 묻는다 — 명령을 만들 수 있는지가 곧
/// 업데이트할 수 있는지이고, 규칙이 두 곳에 있으면 언젠가 갈린다.
pub fn is_target(health: &super::VendorHealth) -> bool {
    health.update_available
        && command_for(
            ActionKind::Update,
            &health.vendor,
            health.install_method,
            super::package_of(&health.vendor),
        )
        .is_ok()
}

/// `updating` 플래그의 수명을 쥔다. Drop에서 반드시 내려간다.
///
/// 이 타입이 하는 일은 한 줄이지만, 그 한 줄을 사람이 모든 반환 경로에 적는 것보다
/// 컴파일러가 챙기게 하는 편이 안전하다. 못 내리면 앱 재시작 전까지 작업 생성이 막힌다.
struct UpdateGuard(Arc<AtomicBool>);

impl UpdateGuard {
    /// 이미 누가 돌고 있으면 None — 두 번 겹쳐 돌리지 않는다.
    fn acquire(flag: &Arc<AtomicBool>) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Self(flag.clone()))
    }
}

impl Drop for UpdateGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// 실패 출력을 사람이 읽을 한 줄로. stderr가 비면 stdout으로 물러선다 —
/// npm은 실패를 stdout에 적는 경우가 있어 stderr만 보면 빈 메시지가 나간다.
fn failure_detail(stdout: &str, stderr: &str, code: Option<i32>) -> String {
    let picked = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    if picked.is_empty() {
        return format!("종료 코드 {code:?}");
    }
    // `chars()`로 잘라야 한국어·이모지가 UTF-8 경계에서 깨지지 않는다.
    picked.chars().take(DETAIL_LIMIT).collect()
}

/// 명령 하나를 돌린다. 타임아웃이면 자식을 거두고 실패로 돌려준다.
///
/// `detect::probe`와 형태가 닮았지만 합치지 않는다 — 그쪽은 5초 안에 끝나야 하는 상태
/// 조회이고 이쪽은 몇 분이 정상인 설치다. 출력 처리도 다르다(그쪽은 파싱, 이쪽은 사람용 메시지).
///
/// `timeout`을 인자로 받는 이유는 테스트다. 상수에 묶어두면 이 분기를 검증할 방법이 없다.
fn run(bin: &str, args: &[String], timeout: Duration) -> Result<(), String> {
    let mut command = Command::new(bin);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command
        .spawn()
        .map_err(|error| format!("실행할 수 없습니다: {error}"))?;
    let id = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok(output)) if output.status.success() => Ok(()),
        Ok(Ok(output)) => Err(failure_detail(
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
            output.status.code(),
        )),
        Ok(Err(error)) => Err(format!("실행 실패: {error}")),
        Err(_) => {
            // 자식만 죽인다. 손자(npm → node)가 파이프를 물고 있으면 리더 스레드는 EOF를
            // 받지 못한 채 남는데, 그 스레드는 채널만 붙들 뿐 플래그와 무관하므로 작업
            // 생성을 막지는 않는다 — 프로세스 그룹 종료는 이 기능의 범위를 넘는다.
            detect::kill_pid(id);
            Err(format!("{}초 안에 끝나지 않아 중단했습니다", timeout.as_secs()))
        }
    }
}

/// 벤더 하나를 업데이트한다. 실제 프로세스를 띄우므로 blocking 컨텍스트에서 부른다.
pub fn update_one(target: Target) -> UpdateOutcome {
    let Target {
        vendor,
        label,
        installed,
    } = target;
    let outcome = |ok: bool, to: Option<String>, error: Option<String>| UpdateOutcome {
        vendor: vendor.clone(),
        label: label.clone(),
        from: installed.clone(),
        to,
        ok,
        error,
    };

    let Some(bin) = super::bin_of(&vendor) else {
        return outcome(false, None, Some(format!("알 수 없는 벤더입니다: {vendor}")));
    };
    let method = detect::install_method_of_bin(bin);
    let command = match command_for(ActionKind::Update, &vendor, method, super::package_of(&vendor))
    {
        Ok(command) => command,
        Err(error) => return outcome(false, None, Some(error)),
    };
    let Some(resolved) = detect::resolve_bin(&command.bin) else {
        return outcome(
            false,
            None,
            Some(format!("PATH에서 찾을 수 없습니다: {}", command.bin)),
        );
    };

    match run(&resolved, &command.args, UPDATE_TIMEOUT) {
        // 명령이 성공해도 버전을 다시 읽는다 — "성공했다"와 "새 버전이 깔렸다"는 다른 말이고,
        // 사용자가 알고 싶은 것은 뒤쪽이다.
        Ok(()) => outcome(true, detect::installed_version(bin), None),
        Err(error) => outcome(false, detect::installed_version(bin), Some(error)),
    }
}

/// 실제 스냅샷에서 업데이트 대상을 뽑는다.
async fn real_targets() -> Vec<Target> {
    super::snapshot(true)
        .await
        .vendors
        .iter()
        .filter(|health| is_target(health))
        .map(|health| Target {
            vendor: health.vendor.clone(),
            label: health.label.clone(),
            installed: health.installed.clone(),
        })
        .collect()
}

/// 업데이터 함수 — 테스트에서 갈아끼우기 위해 값으로 받는다.
type Updater = Arc<dyn Fn(Target) -> UpdateOutcome + Send + Sync>;

/// 시작 시 자동 업데이트 본체.
///
/// `active_work`는 지금 돌고 있는 작업 수를 세는 클로저다. 부팅 직후라고 0이라 단정하지
/// 않는 이유: `reconcile_stale_running`이 이전 세션의 대화 턴을 되살려 감시를 이어간다.
/// 되살아난 턴 위로 바이너리를 갈아치우면 정확히 우리가 막으려던 일이 일어난다.
pub async fn execute<A>(enabled: bool, updating: &Arc<AtomicBool>, active_work: A) -> AutoUpdateReport
where
    A: Fn() -> usize,
{
    execute_with(
        enabled,
        updating,
        active_work,
        real_targets,
        Arc::new(update_one),
    )
    .await
}

async fn execute_with<A, F, Fut>(
    enabled: bool,
    updating: &Arc<AtomicBool>,
    active_work: A,
    fetch_targets: F,
    updater: Updater,
) -> AutoUpdateReport
where
    A: Fn() -> usize,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Vec<Target>>,
{
    let finished = |outcomes: Vec<UpdateOutcome>, skipped: Option<String>| AutoUpdateReport {
        outcomes,
        skipped,
        finished_at: super::now_secs(),
    };

    if !enabled {
        return finished(Vec::new(), Some("자동 업데이트가 꺼져 있습니다".into()));
    }

    // 조회는 플래그 밖에서 한다. 읽기만 하므로 바이너리를 건드리지 않고, 대부분의 실행은
    // 여기서 끝난다 — 그동안 작업 생성을 막을 이유가 없다.
    let targets = fetch_targets().await;
    if targets.is_empty() {
        return finished(Vec::new(), Some("모든 CLI가 최신입니다".into()));
    }

    // 플래그를 먼저 세우고 작업 수를 센다. 순서를 뒤집으면 "0을 확인한 뒤 플래그를 세우기
    // 전"에 끼어든 작업을 놓친다.
    let Some(_guard) = UpdateGuard::acquire(updating) else {
        return finished(Vec::new(), Some("이미 업데이트가 진행 중입니다".into()));
    };
    let active = active_work();
    if active > 0 {
        return finished(
            Vec::new(),
            Some(format!(
                "작업 {active}개가 돌고 있어 건너뜁니다 — 업데이트는 바이너리를 교체합니다"
            )),
        );
    }

    // 순차로 돈다. 병렬로 돌리면 npm 전역 설치가 서로의 트리를 건드릴 수 있고, 어차피
    // 시작 시점이라 서두를 이유도 없다.
    let mut outcomes = Vec::with_capacity(targets.len());
    for target in targets {
        let updater = updater.clone();
        let described = target.clone();
        let outcome = tokio::task::spawn_blocking(move || updater(target)).await;
        outcomes.push(outcome.unwrap_or_else(|error| UpdateOutcome {
            vendor: described.vendor,
            label: described.label,
            from: described.installed,
            to: None,
            ok: false,
            // 태스크가 죽었는데 아무 줄도 남기지 않으면 UI에서 "아무 일 없었음"과 같아진다.
            error: Some(format!("업데이트 태스크가 죽었습니다: {error}")),
        }));
    }
    finished(outcomes, None)
}

#[cfg(test)]
mod tests {
    use super::super::detect::InstallMethod;
    use super::*;

    fn flag() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    fn target(vendor: &str) -> Target {
        Target {
            vendor: vendor.into(),
            label: vendor.into(),
            installed: Some("1.0.0".into()),
        }
    }

    fn ok_outcome(target: Target) -> UpdateOutcome {
        UpdateOutcome {
            vendor: target.vendor,
            label: target.label,
            from: target.installed,
            to: Some("2.0.0".into()),
            ok: true,
            error: None,
        }
    }

    // ── 설정 해석 ────────────────────────────────────────────────

    #[test]
    fn unset_means_off_because_updating_global_clis_needs_consent() {
        assert!(!enabled_from(None));
        assert!(enabled_from(Some("true")));
    }

    #[test]
    fn an_explicit_off_is_honoured_in_the_spellings_people_actually_write() {
        // 설정을 손으로 고친 사람이 "0"이라 적었다고 바이너리가 교체되면 안 된다.
        for raw in ["false", "FALSE", " false ", "0", "off", "no"] {
            assert!(!enabled_from(Some(raw)), "{raw}는 꺼짐이어야 한다");
        }
    }

    #[test]
    fn what_we_write_is_what_we_read_back() {
        // 저장 문자열과 해석이 갈리면 사용자가 끈 설정이 다음 시작에 되살아난다.
        assert!(!enabled_from(Some(setting_value(false))));
        assert!(enabled_from(Some(setting_value(true))));
    }

    // ── 플래그 수명 ──────────────────────────────────────────────

    #[test]
    fn the_guard_releases_the_flag_when_dropped() {
        let flag = flag();
        {
            let _guard = UpdateGuard::acquire(&flag).expect("첫 획득은 성공해야 한다");
            assert!(flag.load(Ordering::SeqCst));
        }
        assert!(!flag.load(Ordering::SeqCst), "Drop 이 플래그를 내려야 한다");
    }

    #[test]
    fn the_guard_releases_the_flag_even_on_panic() {
        // 패닉 경로에서 플래그가 남으면 앱을 재시작할 때까지 작업을 만들 수 없다.
        let flag = flag();
        let taken = flag.clone();
        let result = std::panic::catch_unwind(move || {
            let _guard = UpdateGuard::acquire(&taken).expect("획득");
            panic!("업데이트 도중 터진다");
        });
        assert!(result.is_err());
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn dropping_the_future_mid_await_releases_the_flag() {
        // 앱 종료로 태스크가 취소되면 future 가 await 지점에서 통째로 drop 된다.
        // 그때 플래그가 남으면 다음 실행이 아니라 **이번 실행 내내** 작업이 막힌다.
        let flag = flag();
        let held = flag.clone();
        let mut future = Box::pin(async move {
            let _guard = UpdateGuard::acquire(&held).expect("획득");
            tokio::time::sleep(Duration::from_secs(3600)).await;
        });
        let poll = futures_lite_poll(&mut future);
        assert!(poll.is_pending(), "sleep 에서 멈춰 있어야 한다");
        assert!(flag.load(Ordering::SeqCst), "가드를 든 상태여야 한다");
        drop(future);
        assert!(!flag.load(Ordering::SeqCst), "취소돼도 플래그는 풀려야 한다");
    }

    /// 테스트용 1회 poll — 런타임 없이 future 를 한 번만 진행시킨다.
    fn futures_lite_poll<F: std::future::Future>(
        future: &mut std::pin::Pin<Box<F>>,
    ) -> std::task::Poll<F::Output> {
        use std::task::{Context, RawWaker, RawWakerVTable, Waker};
        fn noop(_: *const ()) {}
        fn clone(_: *const ()) -> RawWaker {
            RawWaker::new(std::ptr::null(), &VTABLE)
        }
        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        future.as_mut().poll(&mut Context::from_waker(&waker))
    }

    #[test]
    fn a_second_acquire_is_refused_while_the_first_is_alive() {
        let flag = flag();
        let first = UpdateGuard::acquire(&flag).expect("첫 획득");
        assert!(UpdateGuard::acquire(&flag).is_none());
        drop(first);
        assert!(UpdateGuard::acquire(&flag).is_some());
    }

    // ── execute 분기 ─────────────────────────────────────────────

    #[tokio::test]
    async fn disabled_never_touches_the_flag() {
        let flag = flag();
        let report = execute_with(
            false,
            &flag,
            || 0,
            || async { vec![target("claude")] },
            Arc::new(ok_outcome),
        )
        .await;
        assert_eq!(report.skipped.as_deref(), Some("자동 업데이트가 꺼져 있습니다"));
        assert!(report.outcomes.is_empty());
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn nothing_to_update_never_blocks_task_creation() {
        // 평상시 경로다. 여기서 플래그를 잡으면 매 실행마다 스냅샷 시간만큼 작업이 막힌다.
        let flag = flag();
        let seen = Arc::new(AtomicBool::new(false));
        let watch = seen.clone();
        let report = execute_with(
            true,
            &flag,
            move || {
                watch.store(true, Ordering::SeqCst);
                0
            },
            || async { Vec::new() },
            Arc::new(ok_outcome),
        )
        .await;
        assert_eq!(report.skipped.as_deref(), Some("모든 CLI가 최신입니다"));
        assert!(!flag.load(Ordering::SeqCst));
        assert!(
            !seen.load(Ordering::SeqCst),
            "대상이 없으면 작업 수를 셀 필요조차 없다"
        );
    }

    #[tokio::test]
    async fn running_work_skips_the_update_and_releases_the_flag() {
        // 부팅 직후에도 되살아난 대화 턴이 있을 수 있다 — 그 위로 바이너리를 바꾸지 않는다.
        let flag = flag();
        let report = execute_with(
            true,
            &flag,
            || 2,
            || async { vec![target("claude")] },
            Arc::new(ok_outcome),
        )
        .await;
        assert!(report.skipped.unwrap().contains("2개"));
        assert!(report.outcomes.is_empty());
        assert!(
            !flag.load(Ordering::SeqCst),
            "건너뛴 뒤에도 플래그는 풀려 있어야 한다"
        );
    }

    #[tokio::test]
    async fn an_update_already_in_flight_is_not_started_twice() {
        // 플래그가 이미 서 있으면 가드를 얻지 못한다. 이때 **플래그를 건드리면 안 된다** —
        // 남의 가드를 내려버리면 첫 업데이트가 도는 중에 작업 생성이 열린다.
        let flag = Arc::new(AtomicBool::new(true));
        let report = execute_with(
            true,
            &flag,
            || 0,
            || async { vec![target("claude")] },
            Arc::new(ok_outcome),
        )
        .await;
        assert_eq!(report.skipped.as_deref(), Some("이미 업데이트가 진행 중입니다"));
        assert!(flag.load(Ordering::SeqCst), "남의 플래그를 내리면 안 된다");
    }

    #[tokio::test]
    async fn the_flag_is_already_up_when_work_is_counted() {
        // 순서가 뒤집히면 "0을 확인한 뒤 플래그를 세우기 전"에 끼어든 작업을 놓친다.
        let flag = flag();
        let observed = Arc::new(AtomicBool::new(false));
        let watch = observed.clone();
        let seen = flag.clone();
        execute_with(
            true,
            &flag,
            move || {
                watch.store(seen.load(Ordering::SeqCst), Ordering::SeqCst);
                0
            },
            || async { vec![target("claude")] },
            Arc::new(ok_outcome),
        )
        .await;
        assert!(
            observed.load(Ordering::SeqCst),
            "작업 수를 셀 때 플래그가 이미 서 있어야 한다"
        );
    }

    #[tokio::test]
    async fn every_target_gets_an_outcome_in_order() {
        let flag = flag();
        let report = execute_with(
            true,
            &flag,
            || 0,
            || async { vec![target("claude"), target("codex")] },
            Arc::new(ok_outcome),
        )
        .await;
        let vendors: Vec<_> = report.outcomes.iter().map(|o| o.vendor.as_str()).collect();
        assert_eq!(vendors, ["claude", "codex"]);
        assert!(report.skipped.is_none());
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn one_failure_does_not_stop_the_rest() {
        let flag = flag();
        let report = execute_with(
            true,
            &flag,
            || 0,
            || async { vec![target("claude"), target("codex")] },
            Arc::new(|t: Target| {
                if t.vendor == "claude" {
                    UpdateOutcome {
                        vendor: t.vendor,
                        label: t.label,
                        from: t.installed,
                        to: None,
                        ok: false,
                        error: Some("터졌다".into()),
                    }
                } else {
                    ok_outcome(t)
                }
            }),
        )
        .await;
        assert_eq!(report.outcomes.len(), 2);
        assert!(!report.outcomes[0].ok);
        assert!(report.outcomes[1].ok);
    }

    #[tokio::test]
    async fn a_dead_update_task_still_leaves_a_line() {
        // 패닉한 벤더가 리포트에서 통째로 사라지면 "아무 일 없었음"과 구분되지 않는다.
        let flag = flag();
        let report = execute_with(
            true,
            &flag,
            || 0,
            || async { vec![target("claude")] },
            Arc::new(|_| panic!("업데이트 중 터진다")),
        )
        .await;
        assert_eq!(report.outcomes.len(), 1);
        assert!(!report.outcomes[0].ok);
        assert!(report.outcomes[0]
            .error
            .as_deref()
            .unwrap()
            .contains("죽었습니다"));
        assert!(
            !flag.load(Ordering::SeqCst),
            "태스크가 죽어도 플래그는 풀려야 한다"
        );
    }

    // ── 명령 실행 ────────────────────────────────────────────────

    #[test]
    fn failure_detail_prefers_stderr_but_falls_back_to_stdout() {
        // npm 은 실패를 stdout 에 적는 경우가 있다 — stderr 만 보면 빈 메시지가 나간다.
        assert_eq!(failure_detail("out", "err", Some(1)), "err");
        assert_eq!(failure_detail("out", "   ", Some(1)), "out");
    }

    #[test]
    fn failure_detail_falls_back_to_the_exit_code_when_there_is_nothing_to_say() {
        assert_eq!(failure_detail("", "", Some(7)), "종료 코드 Some(7)");
    }

    #[test]
    fn failure_detail_truncates_on_character_boundaries() {
        // 바이트로 자르면 한국어가 깨져 사용자에게 깨진 글자가 보인다.
        let long = "한".repeat(1000);
        let detail = failure_detail("", &long, Some(1));
        assert_eq!(detail.chars().count(), DETAIL_LIMIT);
        assert!(detail.starts_with('한'));
    }

    #[test]
    fn a_command_that_hangs_is_cut_off_rather_than_blocking_forever() {
        // 하나가 매달리면 작업 생성이 그만큼 막힌다 — 반드시 끊겨야 한다.
        let error = run(
            "/bin/sh",
            &["-c".into(), "sleep 30".into()],
            Duration::from_millis(200),
        )
        .expect_err("타임아웃이어야 한다");
        assert!(error.contains("중단했습니다"), "{error}");
    }

    #[test]
    fn a_failing_command_reports_its_own_words() {
        let error = run(
            "/bin/sh",
            &["-c".into(), "echo 망했다 >&2; exit 3".into()],
            Duration::from_secs(10),
        )
        .expect_err("실패여야 한다");
        assert!(error.contains("망했다"), "{error}");
    }

    #[test]
    fn a_missing_binary_is_a_failure_not_a_panic() {
        let error = run("/nonexistent/binary", &[], Duration::from_secs(10))
            .expect_err("실패여야 한다");
        assert!(error.contains("실행할 수 없습니다"), "{error}");
    }

    #[test]
    fn a_successful_command_is_ok() {
        assert!(run("/bin/sh", &["-c".into(), "exit 0".into()], Duration::from_secs(10)).is_ok());
    }

    // ── 대상 선별 ────────────────────────────────────────────────

    #[test]
    fn an_unknown_install_method_is_not_a_target() {
        // 명령을 만들 수 없는 것을 대상에 넣으면 매번 실패 한 줄이 남는다.
        assert!(!is_target(&health(InstallMethod::Unknown, true)));
    }

    #[test]
    fn an_updatable_vendor_is_a_target() {
        assert!(is_target(&health(InstallMethod::Npm, true)));
    }

    #[test]
    fn a_current_vendor_is_not_a_target() {
        assert!(!is_target(&health(InstallMethod::Npm, false)));
    }

    fn health(install_method: InstallMethod, update_available: bool) -> super::super::VendorHealth {
        super::super::VendorHealth {
            vendor: "codex".into(),
            label: "Codex".into(),
            auth: super::super::detect::AuthState::Ok,
            auth_detail: None,
            account: None,
            plan: None,
            installed: Some("0.1.0".into()),
            latest: Some("0.2.0".into()),
            update_available,
            install_method,
            bin_path: None,
            checked_at: 0,
        }
    }
}
