//! 파이프라인이 띄운 자식 프로세스(심판·리뷰어·수리 에이전트·검증 명령)의 메모리 내 등록부.
//! 실행(run)마다 하나를 두고, 취소·앱 종료 때 등록된 프로세스 그룹을 모두 종료한다.
//! 내구성 레지스트라(`runner::review_process`)는 task에 묶여 있어 쓰지 않는다 — 파이프라인 프로세스는
//! 재시작 후 이어받을 대상이 아니라(드라이버가 DB 상태에서 다시 시작한다) 앱과 함께 죽어야 한다.

use crate::managed_process::{ProcessLease, ProcessRegistrar, SharedProcessRegistrar};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Default)]
pub struct RunRegistrar {
    pids: Mutex<HashSet<u32>>,
    cancelled: AtomicBool,
}

struct Lease {
    owner: Arc<RunRegistrar>,
    pid: u32,
}

impl ProcessLease for Lease {
    fn complete(self: Box<Self>) -> Result<(), String> {
        self.owner.forget(self.pid);
        Ok(())
    }
    fn quarantine(self: Box<Self>, _detail: &str) -> Result<(), String> {
        self.owner.forget(self.pid);
        Ok(())
    }
}

struct Handle(Arc<RunRegistrar>);

impl ProcessRegistrar for Handle {
    fn register(&self, pid: u32) -> Result<Box<dyn ProcessLease>, String> {
        if pid == 0 {
            return Err("잘못된 pid".into());
        }
        // 취소 표시와 등록을 같은 락에서 판단해, 취소 직후 새로 뜨는 프로세스를 놓치지 않는다.
        let mut pids = self.0.pids.lock().unwrap();
        if self.0.cancelled.load(Ordering::SeqCst) {
            return Err("파이프라인 실행이 취소되었습니다".into());
        }
        pids.insert(pid);
        Ok(Box::new(Lease { owner: self.0.clone(), pid }))
    }
}

impl RunRegistrar {
    fn forget(&self, pid: u32) {
        self.pids.lock().unwrap().remove(&pid);
    }

    pub fn registrar(self: &Arc<Self>) -> SharedProcessRegistrar {
        Arc::new(Handle(self.clone()))
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn tracked(&self) -> usize {
        self.pids.lock().unwrap().len()
    }

    /// 취소를 표시하고 등록된 프로세스 그룹을 모두 종료한다. 이후 등록은 거절된다.
    pub fn kill_all(&self) -> usize {
        let pids: Vec<u32> = {
            let mut guard = self.pids.lock().unwrap();
            self.cancelled.store(true, Ordering::SeqCst);
            guard.drain().collect()
        };
        for pid in &pids {
            kill_group(*pid);
        }
        pids.len()
    }
}

#[cfg(unix)]
fn kill_group(pid: u32) {
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;
    // pid 0은 호출자 자신의 그룹을 죽이므로 register에서 이미 거른다.
    if pid == 0 {
        return;
    }
    let _ = killpg(Pid::from_raw(pid as i32), Signal::SIGTERM);
    let _ = killpg(Pid::from_raw(pid as i32), Signal::SIGKILL);
}

#[cfg(not(unix))]
fn kill_group(_pid: u32) {}

fn runs() -> &'static Mutex<HashMap<i64, Arc<RunRegistrar>>> {
    static RUNS: OnceLock<Mutex<HashMap<i64, Arc<RunRegistrar>>>> = OnceLock::new();
    RUNS.get_or_init(Default::default)
}

/// 실행의 등록부. 취소된 실행이 이후 살아나는 일은 없으므로(Cancelled는 종결) 취소 표시는 유지한다.
pub fn for_run(run_id: i64) -> Arc<RunRegistrar> {
    runs().lock().unwrap().entry(run_id).or_default().clone()
}

/// 실행의 모든 자식 프로세스를 종료하고 취소를 표시한다.
pub fn kill_run(run_id: i64) -> usize {
    let reg = runs().lock().unwrap().get(&run_id).cloned();
    reg.map(|r| r.kill_all()).unwrap_or_else(|| {
        // 아직 등록부가 없던 실행도 이후 등록이 거절되도록 만들어 둔다.
        for_run(run_id).kill_all()
    })
}

/// 앱 종료: 모든 실행의 자식 프로세스를 종료한다.
pub fn kill_everything() -> usize {
    let all: Vec<Arc<RunRegistrar>> = runs().lock().unwrap().values().cloned().collect();
    all.iter().map(|r| r.kill_all()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_complete_and_kill() {
        let reg = Arc::new(RunRegistrar::default());
        let shared = reg.registrar();
        let lease = shared.register(4_000_000).unwrap();
        assert_eq!(reg.tracked(), 1);
        lease.complete().unwrap();
        assert_eq!(reg.tracked(), 0);
    }

    #[test]
    fn kill_all_blocks_later_registration() {
        let reg = Arc::new(RunRegistrar::default());
        let shared = reg.registrar();
        // 존재하지 않을 pid라 killpg는 실패해도 무방하다.
        let _lease = shared.register(4_000_001).unwrap();
        assert_eq!(reg.kill_all(), 1);
        assert!(reg.is_cancelled());
        assert!(shared.register(4_000_002).is_err());
    }

    #[test]
    fn pid_zero_is_rejected() {
        let reg = Arc::new(RunRegistrar::default());
        assert!(reg.registrar().register(0).is_err());
    }

    #[test]
    fn kill_run_marks_unknown_run_cancelled() {
        assert_eq!(kill_run(987_654), 0);
        assert!(for_run(987_654).is_cancelled());
    }
}
