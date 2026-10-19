//! 마지막으로 성공한 Claude 잔량 관측을 디스크에 남긴다.
//!
//! 프로세스 메모리 캐시(`OAUTH_CACHE`)는 재시작에 사라져, 토큰이 만료된 상태로 앱을 켜면
//! 보여줄 값이 아무것도 없다. 마지막 성공값만 따로 파일에 두면 그 자리를 "낡은 값"으로
//! 채울 수 있다 — 자격증명은 담지 않으므로 이 파일은 시크릿이 아니다.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::{UsageWindow, VendorUsage};

/// 같은 관측을 폴링마다 다시 쓰지 않기 위한 표시(프로세스 한정).
static WRITTEN_AT: Mutex<Option<i64>> = Mutex::new(None);

/// 성공 스냅샷에서 다시 보여줄 값만 추린 것.
#[derive(Serialize, Deserialize)]
pub(super) struct LastSuccess {
    pub plan: Option<String>,
    pub five_hour: Option<UsageWindow>,
    pub weekly: Option<UsageWindow>,
    /// 값이 관측된 시각(epoch secs).
    pub observed_at: i64,
    /// 그때의 출처("statusline" | "oauth" | "manual-token").
    pub source: Option<String>,
}

/// 브리지 덤프와 같은 디렉터리 — `~/.claude/praxis-usage-last.json`.
pub(super) fn path(home: &Path) -> PathBuf {
    home.join(".claude").join("praxis-usage-last.json")
}

/// 성공 스냅샷 저장. 실패는 조용히 넘긴다 — 잔량 표시는 best-effort다.
pub(super) fn save(home: &Path, usage: &VendorUsage) {
    let Some(observed_at) = usage.updated_at else {
        return;
    };
    if let Ok(mut guard) = WRITTEN_AT.lock() {
        if *guard == Some(observed_at) {
            return;
        }
        *guard = Some(observed_at);
    }
    let record = LastSuccess {
        plan: usage.plan.clone(),
        five_hour: usage.five_hour.clone(),
        weekly: usage.weekly.clone(),
        observed_at,
        source: usage.source.clone(),
    };
    let Ok(body) = serde_json::to_string(&record) else {
        return;
    };
    write_atomic(&path(home), &body);
}

/// 마지막 성공값. 없거나 깨졌으면 None.
pub(super) fn load(home: &Path) -> Option<LastSuccess> {
    let raw = std::fs::read_to_string(path(home)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// 임시 파일에 쓰고 교체 — 중간 상태의 반쪽 JSON을 다음 실행이 읽지 않게 한다.
fn write_atomic(path: &Path, body: &str) {
    let tmp = path.with_extension("json.praxis-tmp");
    if std::fs::write(&tmp, body).is_err() {
        return;
    }
    if std::fs::rename(&tmp, path).is_err() {
        std::fs::remove_file(&tmp).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(tag: &str) -> PathBuf {
        let dir = crate::testtmp::dir().join(format!("praxis-last-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        dir
    }

    fn sample(observed_at: i64) -> VendorUsage {
        VendorUsage {
            vendor: "claude".into(),
            label: "Claude Code".into(),
            status: "ok".into(),
            detail: None,
            plan: Some("max".into()),
            five_hour: Some(UsageWindow {
                used_percent: 40.0,
                resets_at: Some(500),
                window_minutes: Some(300),
            }),
            weekly: None,
            quota_groups: Vec::new(),
            source: Some("statusline".into()),
            updated_at: Some(observed_at),
        }
    }

    #[test]
    fn saved_snapshot_survives_reload() {
        let home = temp_home("roundtrip");
        save(&home, &sample(1_234));
        let loaded = load(&home).expect("저장된 값이 다시 읽힌다");
        assert_eq!(loaded.observed_at, 1_234);
        assert_eq!(loaded.plan.as_deref(), Some("max"));
        assert_eq!(loaded.five_hour.unwrap().used_percent, 40.0);
        assert_eq!(loaded.source.as_deref(), Some("statusline"));
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn missing_file_is_none() {
        let home = temp_home("missing");
        std::fs::remove_file(path(&home)).ok();
        assert!(load(&home).is_none());
        std::fs::remove_dir_all(&home).ok();
    }
}
