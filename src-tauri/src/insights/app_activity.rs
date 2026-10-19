//! 앱 사용 시간 — 앱이 켜져 있던 시간, 메인 창이 포커스를 가진 시간, 실제로 조작한 시간.
//!
//! 조작 시간(`input`)은 창 포커스 중 키보드·마우스 입력 사이의 간격이 `INPUT_GAP_SECS` 이하인
//! 구간이다. 입력 이벤트를 직접 받지 않고 하트비트마다 OS의 "마지막 입력 이후 경과 시간"을 읽어
//! 마지막 입력 시각을 역산한다 — 구간 끝이 하트비트가 아니라 마지막 입력에 맞춰지므로, 손을 뗀 뒤
//! 하트비트까지 흐른 시간은 세지 않는다.
//!
//! 두 종류의 구간(`open`·`focus`)을 행 하나씩으로 남기고, 하트비트가 열린 구간의 `last_seen`을
//! 밀어 올린다. 종료·크래시 때 따로 닫지 않아도 되는 이유다 — 마지막 하트비트가 곧 끝 시각이고,
//! 잃는 것은 하트비트 간격 이하다.
//!
//! 잠자기 동안에는 하트비트가 멈춘다. 깨어난 뒤 `last_seen`을 그대로 밀면 잠든 시간이 켜져 있던
//! 시간으로 잡히므로, 간격이 `GAP_SECS`를 넘으면 기존 구간을 닫고 새 구간을 연다.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use serde::Serialize;
use sqlx::SqlitePool;

/// 하트비트 주기(초).
pub const HEARTBEAT_SECS: u64 = 30;

/// 이보다 오래 하트비트가 없었으면 그 사이는 끊긴 것으로 본다(잠자기·멈춤).
const GAP_SECS: i64 = 90;

/// 입력 사이가 이보다 벌어지면 조작이 끊긴 것으로 본다. 읽기만 하는 시간은 이 이하일 때만 이어진다.
const INPUT_GAP_SECS: i64 = 60;

const KIND_OPEN: &str = "open";
const KIND_FOCUS: &str = "focus";
const KIND_INPUT: &str = "input";

const MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS app_activity (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  kind        TEXT NOT NULL,
  started_at  INTEGER NOT NULL,
  last_seen   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_app_activity_last_seen ON app_activity(last_seen);
"#;

pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(MIGRATION).execute(pool).await?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct Segment {
    id: i64,
    started_at: i64,
    last_seen: i64,
}

#[derive(Debug, Default)]
struct Current {
    open: Option<Segment>,
    focus: Option<Segment>,
    input: Option<Segment>,
}

/// 하트비트 시점의 관측값.
#[derive(Debug, Clone, Copy)]
pub struct Probe {
    /// 메인 창이 포커스를 가졌는가.
    pub focused: bool,
    /// 시스템 전체에서 마지막 입력 이후 흐른 초. 알 수 없으면 None — 조작 시간을 기록하지 않는다.
    pub idle_secs: Option<i64>,
}

/// macOS의 마지막 입력(키보드·마우스·스크롤) 이후 경과 초. 권한 없이 읽힌다.
#[cfg(target_os = "macos")]
pub fn system_idle_secs() -> Option<i64> {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
    }
    const COMBINED_SESSION_STATE: i32 = 0;
    const ANY_INPUT_EVENT: u32 = u32::MAX;
    // SAFETY: 인자 둘을 값으로 받고 전역 상태를 읽기만 하는 CoreGraphics 함수다.
    let secs = unsafe { CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, ANY_INPUT_EVENT) };
    secs.is_finite().then(|| secs.max(0.0) as i64)
}

#[cfg(not(target_os = "macos"))]
pub fn system_idle_secs() -> Option<i64> {
    None
}

/// 지금 열려 있는 구간. 프로세스당 하나(`TRACKER`)이고, 테스트는 따로 만든다.
///
/// 잠금을 DB 쓰기 내내 쥔다 — 하트비트와 포커스 이벤트가 겹쳐 둘 다 "열린 구간 없음"을 보면
/// 같은 구간이 두 행으로 열려 시간이 이중으로 잡힌다.
#[derive(Debug, Default)]
pub struct Tracker {
    current: tokio::sync::Mutex<Current>,
}

pub static TRACKER: LazyLock<Tracker> = LazyLock::new(Tracker::default);

async fn insert(pool: &SqlitePool, kind: &str, now: i64) -> anyhow::Result<Segment> {
    let id = sqlx::query("INSERT INTO app_activity (kind, started_at, last_seen) VALUES (?, ?, ?)")
        .bind(kind)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?
        .last_insert_rowid();
    Ok(Segment { id, started_at: now, last_seen: now })
}

async fn touch(pool: &SqlitePool, id: i64, now: i64) -> anyhow::Result<()> {
    sqlx::query("UPDATE app_activity SET last_seen = ? WHERE id = ? AND last_seen < ?")
        .bind(now)
        .bind(id)
        .bind(now)
        .execute(pool)
        .await?;
    Ok(())
}

/// 구간을 이어 가거나, 끊겼으면 새로 연다. 끊긴 구간은 마지막 하트비트에서 끝난 채로 둔다.
async fn extend(pool: &SqlitePool, kind: &str, seg: Option<Segment>, now: i64) -> anyhow::Result<Segment> {
    match seg {
        Some(seg) if now - seg.last_seen <= GAP_SECS => {
            touch(pool, seg.id, now).await?;
            Ok(Segment { last_seen: now.max(seg.last_seen), ..seg })
        }
        _ => insert(pool, kind, now).await,
    }
}

/// 마지막 입력 시각까지 조작 구간을 늘리거나, 간격이 벌어졌으면 그 시각에서 새로 연다.
async fn record_input(pool: &SqlitePool, seg: Option<Segment>, last_input: i64) -> anyhow::Result<Segment> {
    match seg {
        Some(seg) if last_input - seg.last_seen <= INPUT_GAP_SECS => {
            if last_input > seg.last_seen {
                touch(pool, seg.id, last_input).await?;
            }
            Ok(Segment { last_seen: last_input.max(seg.last_seen), ..seg })
        }
        _ => insert(pool, KIND_INPUT, last_input).await,
    }
}

impl Tracker {
    /// 열린 구간을 지금 상태에 맞춘다. 하트비트와 포커스 이벤트가 모두 이것을 부른다.
    ///
    /// `focused`는 잠금을 얻은 **뒤에** 묻는다 — 미리 읽은 값을 들고 기다리면, 그 사이 다른
    /// 호출이 닫은 포커스 구간을 낡은 값으로 다시 열게 된다. 이벤트가 실어 온 값 대신 창에
    /// 직접 묻는 것도 같은 이유다: 이벤트 태스크의 실행 순서가 뒤집혀도 마지막 호출이 현재를 본다.
    pub async fn sync(
        &self,
        pool: &SqlitePool,
        probe: impl FnOnce() -> Probe,
        now: i64,
    ) -> anyhow::Result<()> {
        let mut current = self.current.lock().await;
        let Probe { focused, idle_secs } = probe();
        let prev_focus = current.focus;
        current.open = Some(extend(pool, KIND_OPEN, current.open, now).await?);
        current.focus = if focused {
            Some(extend(pool, KIND_FOCUS, current.focus, now).await?)
        } else {
            if let Some(seg) = current.focus {
                if now - seg.last_seen <= GAP_SECS {
                    touch(pool, seg.id, now).await?;
                }
            }
            None
        };
        // 마지막 입력은 포커스 구간 시작보다 앞설 수 없다 — 그보다 이른 입력은 다른 앱에서 한 것이다.
        let last_input = |idle: i64, focus: Option<Segment>| {
            (now - idle).max(focus.map_or(now, |seg| seg.started_at))
        };
        current.input = match (focused, idle_secs) {
            (true, Some(idle)) if idle <= INPUT_GAP_SECS => {
                Some(record_input(pool, current.input, last_input(idle, current.focus)).await?)
            }
            // 포커스 중 손을 뗀 상태 — 구간은 마지막 입력에서 이미 끝나 있다. 다음 입력이 간격
            // 안에 오면 이어진다.
            (true, _) => current.input,
            // 포커스를 잃는 순간 — 직전 관측 이후의 입력까지 마감한 뒤 끊는다. 끊지 않으면 다른 앱에서
            // 보낸 시간이 돌아온 뒤 첫 입력으로 메워진다. 새 구간은 열지 않는다.
            (false, idle) => {
                if let (Some(seg), Some(idle)) = (current.input, idle) {
                    let at = last_input(idle, prev_focus);
                    if at > seg.last_seen && at - seg.last_seen <= INPUT_GAP_SECS {
                        touch(pool, seg.id, at).await?;
                    }
                }
                None
            }
        };
        Ok(())
    }
}

/// 직전 동일 길이 구간의 합계 — Δ 계산용. "all"에는 직전 구간이 없다.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct AppActivityPrev {
    pub open_secs: i64,
    pub focus_secs: i64,
    pub input_secs: i64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct AppActivity {
    /// 앱이 켜져 있던 시간(초).
    pub open_secs: i64,
    /// 메인 창이 포커스를 가진 시간(초).
    pub focus_secs: i64,
    /// 창 포커스 중 실제로 조작한(입력 간격 `INPUT_GAP_SECS` 이하) 시간(초).
    pub input_secs: i64,
    /// 앱이 한 번이라도 켜져 있던 로컬 날짜 수.
    pub active_days: i64,
    /// 기록이 시작된 시각(epoch초). 기록이 없으면 None — 이 기능 이전 기간은 비어 있다.
    pub since: Option<i64>,
    pub prev: Option<AppActivityPrev>,
}

fn window(range: &str) -> Option<i64> {
    match range {
        "7d" => Some(7 * 86_400),
        "30d" => Some(30 * 86_400),
        _ => None,
    }
}

async fn segments(pool: &SqlitePool, kind: &str, from: i64, to: i64) -> anyhow::Result<Vec<(i64, i64)>> {
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT MAX(started_at, ?), MIN(last_seen, ?) FROM app_activity \
         WHERE kind = ? AND last_seen > ? AND started_at < ?",
    )
    .bind(from)
    .bind(to)
    .bind(kind)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().filter(|(a, b)| b > a).collect())
}

fn total(segs: &[(i64, i64)]) -> i64 {
    segs.iter().map(|(a, b)| b - a).sum()
}

/// 구간들이 걸친 로컬 날짜 수. 자정을 넘는 구간은 양쪽 날짜에 모두 센다.
fn local_days(segs: &[(i64, i64)], tz_offset_secs: i64) -> i64 {
    let mut days = BTreeSet::new();
    for &(a, b) in segs {
        let first = (a + tz_offset_secs).div_euclid(86_400);
        let last = (b - 1 + tz_offset_secs).div_euclid(86_400);
        days.extend(first..=last);
    }
    days.len() as i64
}

pub async fn compute_app_activity(
    pool: &SqlitePool,
    range: &str,
    tz_offset_secs: i64,
    now: i64,
) -> anyhow::Result<AppActivity> {
    let span = window(range);
    let from = span.map_or(0, |s| now - s);
    let open = segments(pool, KIND_OPEN, from, now).await?;
    let focus = segments(pool, KIND_FOCUS, from, now).await?;
    let input = segments(pool, KIND_INPUT, from, now).await?;
    let since: Option<i64> = sqlx::query_scalar("SELECT MIN(started_at) FROM app_activity")
        .fetch_one(pool)
        .await?;
    // 직전 구간이 기록 시작보다 앞에서 시작하면 일부만 기록된 것이라, 비교하면 Δ가 부풀려진다.
    let prev = match (span, since) {
        (Some(s), Some(since)) if since <= from - s => Some(AppActivityPrev {
            open_secs: total(&segments(pool, KIND_OPEN, from - s, from).await?),
            focus_secs: total(&segments(pool, KIND_FOCUS, from - s, from).await?),
            input_secs: total(&segments(pool, KIND_INPUT, from - s, from).await?),
        }),
        _ => None,
    };
    Ok(AppActivity {
        open_secs: total(&open),
        focus_secs: total(&focus),
        input_secs: total(&input),
        active_days: local_days(&open, tz_offset_secs),
        since,
        prev,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static DATABASE_COUNTER: AtomicU32 = AtomicU32::new(0);

    async fn test_pool() -> SqlitePool {
        let sequence = DATABASE_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = crate::testtmp::dir().join(format!(
            "praxis-app-activity-{}-{sequence}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .unwrap();
        migrate(&pool).await.unwrap();
        pool
    }

    fn focused(idle_secs: Option<i64>) -> impl FnOnce() -> Probe {
        move || Probe { focused: true, idle_secs }
    }

    fn unfocused() -> impl FnOnce() -> Probe {
        || Probe { focused: false, idle_secs: Some(0) }
    }

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_787_500_000;

    #[tokio::test]
    async fn heartbeats_extend_open_and_focus_segments() {
        let pool = test_pool().await;
        let tracker = Tracker::default();
        let t0 = NOW - 600;
        tracker.sync(&pool, focused(None), t0).await.unwrap();
        tracker.sync(&pool, focused(None), t0 + 30).await.unwrap();
        tracker.sync(&pool, unfocused(), t0 + 40).await.unwrap();
        tracker.sync(&pool, unfocused(), t0 + 70).await.unwrap();

        let got = compute_app_activity(&pool, "all", 0, NOW).await.unwrap();
        assert_eq!(got.open_secs, 70);
        assert_eq!(got.focus_secs, 40);
        assert_eq!(got.since, Some(t0));
        assert_eq!(got.prev, None);
    }

    /// 잠자기처럼 하트비트가 끊기면 그 사이를 켜져 있던 시간으로 세지 않는다.
    #[tokio::test]
    async fn a_gap_longer_than_the_threshold_starts_a_new_segment() {
        let pool = test_pool().await;
        let tracker = Tracker::default();
        let t0 = NOW - 10_000;
        tracker.sync(&pool, focused(None), t0).await.unwrap();
        tracker.sync(&pool, focused(None), t0 + 30).await.unwrap();
        tracker.sync(&pool, focused(None), t0 + 3_600).await.unwrap();
        tracker.sync(&pool, focused(None), t0 + 3_630).await.unwrap();

        let got = compute_app_activity(&pool, "all", 0, NOW).await.unwrap();
        assert_eq!(got.open_secs, 60);
        assert_eq!(got.focus_secs, 60);
    }

    #[tokio::test]
    async fn segments_are_clipped_to_the_range_and_the_previous_window() {
        let pool = test_pool().await;
        // 7일 경계를 1시간 걸친 구간 하나: 경계 안쪽 1시간, 직전 구간 1시간.
        sqlx::query("INSERT INTO app_activity (kind, started_at, last_seen) VALUES ('open', ?, ?)")
            .bind(NOW - 7 * DAY - 3_600)
            .bind(NOW - 7 * DAY + 3_600)
            .execute(&pool)
            .await
            .unwrap();
        // 직전 구간 전체를 덮도록 기록 시작을 그 앞에 둔다.
        sqlx::query("INSERT INTO app_activity (kind, started_at, last_seen) VALUES ('open', ?, ?)")
            .bind(NOW - 15 * DAY)
            .bind(NOW - 15 * DAY + 60)
            .execute(&pool)
            .await
            .unwrap();
        let got = compute_app_activity(&pool, "7d", 0, NOW).await.unwrap();
        assert_eq!(got.open_secs, 3_600);
        assert_eq!(got.prev, Some(AppActivityPrev { open_secs: 3_600, focus_secs: 0, input_secs: 0 }));
    }

    /// 기록이 직전 구간 중간에 시작됐으면 직전 합계는 일부뿐이라 Δ를 내지 않는다.
    #[tokio::test]
    async fn no_previous_window_when_recording_started_inside_it() {
        let pool = test_pool().await;
        sqlx::query("INSERT INTO app_activity (kind, started_at, last_seen) VALUES ('open', ?, ?)")
            .bind(NOW - 10 * DAY)
            .bind(NOW - 10 * DAY + 3_600)
            .execute(&pool)
            .await
            .unwrap();
        let got = compute_app_activity(&pool, "7d", 0, NOW).await.unwrap();
        assert_eq!(got.prev, None);
        let wide = compute_app_activity(&pool, "30d", 0, NOW).await.unwrap();
        assert_eq!(wide.open_secs, 3_600);
        assert_eq!(wide.prev, None);
    }

    /// 조작 구간은 하트비트가 아니라 마지막 입력 시각에서 끝난다.
    #[tokio::test]
    async fn input_time_ends_at_the_last_input_not_the_heartbeat() {
        let pool = test_pool().await;
        let tracker = Tracker::default();
        let t0 = NOW - 1_000;
        tracker.sync(&pool, focused(Some(0)), t0).await.unwrap();
        // 30초 뒤 관측: 20초 전에 마지막 입력 → t0+10까지 조작.
        tracker.sync(&pool, focused(Some(20)), t0 + 30).await.unwrap();
        // 손을 뗀 채 흐른 시간은 더하지 않는다.
        tracker.sync(&pool, focused(Some(50)), t0 + 60).await.unwrap();
        tracker.sync(&pool, focused(Some(80)), t0 + 90).await.unwrap();

        let got = compute_app_activity(&pool, "all", 0, NOW).await.unwrap();
        assert_eq!(got.input_secs, 10);
        assert_eq!(got.focus_secs, 90);
    }

    #[tokio::test]
    async fn input_gap_longer_than_the_threshold_splits_and_drops_the_gap() {
        let pool = test_pool().await;
        let tracker = Tracker::default();
        let t0 = NOW - 1_000;
        tracker.sync(&pool, focused(Some(0)), t0).await.unwrap();
        tracker.sync(&pool, focused(Some(0)), t0 + 30).await.unwrap();
        // 마지막 입력 t0+30 → 다음 입력 t0+120: 90초 공백은 조작이 아니다.
        tracker.sync(&pool, focused(Some(0)), t0 + 120).await.unwrap();
        tracker.sync(&pool, focused(Some(0)), t0 + 150).await.unwrap();

        let got = compute_app_activity(&pool, "all", 0, NOW).await.unwrap();
        assert_eq!(got.input_secs, 60);
    }

    /// 다른 앱에 있다가 돌아온 경우, 떠나 있던 시간을 돌아온 뒤 첫 입력으로 메우지 않는다.
    #[tokio::test]
    async fn losing_focus_cuts_the_input_segment() {
        let pool = test_pool().await;
        let tracker = Tracker::default();
        let t0 = NOW - 1_000;
        tracker.sync(&pool, focused(Some(0)), t0).await.unwrap();
        tracker.sync(&pool, focused(Some(0)), t0 + 10).await.unwrap();
        // 포커스를 잃는 순간(전환 입력)까지 센다: t0..t0+20.
        tracker.sync(&pool, unfocused(), t0 + 20).await.unwrap();
        // 돌아온 뒤는 새 구간: t0+40..t0+50. 떠나 있던 20초는 빠진다.
        tracker.sync(&pool, focused(Some(0)), t0 + 40).await.unwrap();
        tracker.sync(&pool, focused(Some(0)), t0 + 50).await.unwrap();

        let got = compute_app_activity(&pool, "all", 0, NOW).await.unwrap();
        assert_eq!(got.input_secs, 30);
    }

    /// 하트비트 사이에 포커스를 얻고 잃어도, 그 사이 조작은 포커스를 잃는 시점까지 남는다.
    #[tokio::test]
    async fn input_between_heartbeats_is_kept_when_focus_is_lost() {
        let pool = test_pool().await;
        let tracker = Tracker::default();
        let t0 = NOW - 1_000;
        tracker.sync(&pool, focused(Some(0)), t0).await.unwrap();
        tracker.sync(&pool, || Probe { focused: false, idle_secs: Some(1) }, t0 + 20).await.unwrap();

        let got = compute_app_activity(&pool, "all", 0, NOW).await.unwrap();
        assert_eq!(got.input_secs, 19);
    }

    /// 사용자 입력 없이 포커스를 얻으면(프로그램이 창을 앞으로) 다른 앱에서 한 입력을 시작점으로
    /// 삼지 않는다.
    #[tokio::test]
    async fn input_before_focus_was_gained_is_not_counted() {
        let pool = test_pool().await;
        let tracker = Tracker::default();
        let t0 = NOW - 1_000;
        // 다른 앱에서 40초 전에 입력한 상태로 포커스를 얻는다.
        tracker.sync(&pool, focused(Some(40)), t0).await.unwrap();
        // 이 앱에서 t0+10에 입력.
        tracker.sync(&pool, focused(Some(20)), t0 + 30).await.unwrap();

        let got = compute_app_activity(&pool, "all", 0, NOW).await.unwrap();
        assert_eq!(got.input_secs, 10);
    }

    /// 입력 시각을 모르면(비 macOS) 조작 시간을 0으로 두고 다른 기록은 그대로 남긴다.
    #[tokio::test]
    async fn unknown_idle_records_no_input_time() {
        let pool = test_pool().await;
        let tracker = Tracker::default();
        tracker.sync(&pool, focused(None), NOW - 100).await.unwrap();
        tracker.sync(&pool, focused(None), NOW - 70).await.unwrap();

        let got = compute_app_activity(&pool, "all", 0, NOW).await.unwrap();
        assert_eq!(got.input_secs, 0);
        assert_eq!(got.focus_secs, 30);
    }

    /// 실제 CoreGraphics 호출이 링크되고 값을 돌려주는지 — 권한 없이 읽혀야 한다.
    #[cfg(target_os = "macos")]
    #[test]
    fn system_idle_is_readable_on_macos() {
        let idle = system_idle_secs().expect("idle seconds");
        assert!(idle >= 0);
    }

    #[test]
    fn local_days_counts_both_sides_of_midnight() {
        let midnight = 20_000 * DAY;
        assert_eq!(local_days(&[(midnight - 60, midnight + 60)], 0), 2);
        assert_eq!(local_days(&[(midnight - 60, midnight)], 0), 1);
        // UTC+9에서는 같은 순간이 로컬 오전 9시라 하루 안에 든다.
        assert_eq!(local_days(&[(midnight - 60, midnight + 60)], 9 * 3_600), 1);
        assert_eq!(local_days(&[], 0), 0);
    }
}
