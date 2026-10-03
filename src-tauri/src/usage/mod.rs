//! 벤더별 사용 한도 잔량 — 에디터 하단 상태바용 스냅샷.
//!
//! 소스(벤더마다 다름, 모두 best-effort — 없으면 상태만 바꾸고 UI는 계속 뜬다):
//! - Codex: `~/.codex/sessions/**/rollout-*.jsonl`의 마지막 `token_count.rate_limits`.
//!   5시간(primary)/주간(secondary) 소진율이 로컬 파일에 그대로 남아 정확하다.
//! - Claude Code: (1) statusline 브리지가 남긴 `~/.claude/praxis-usage.json`(공식
//!   statusline JSON의 `rate_limits`를 그대로 덤프), (2) macOS Keychain 또는 레거시
//!   `~/.claude/.credentials.json`의 OAuth 토큰으로 `/api/oauth/usage` 조회. (1)이 신선하면
//!   네트워크를 타지 않는다.
//! - Antigravity: 잔량을 노출하는 로컬 소스가 없어 `unsupported` 고정.
//!
//! 네트워크 조회는 429가 잦아(claude-code#31637) `MIN_OAUTH_INTERVAL` 간격으로만 재시도하고
//! 그 사이에는 프로세스 메모리 캐시를 돌려준다.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub mod bridge;
mod claude_credentials;
mod last_success;

use claude_credentials::{load_credentials, CredentialError};

/// OAuth 사용량 재조회 최소 간격(초). 엔드포인트가 공격적으로 429를 내므로 넉넉히 잡는다.
const MIN_OAUTH_INTERVAL: i64 = 300;
/// statusline 브리지 캐시를 신선하다고 볼 최대 나이(초). 이보다 낡으면 OAuth를 시도한다.
const BRIDGE_FRESH_SECS: i64 = 900;
/// 세션 로그 tail로 읽을 최대 바이트 — rate_limits는 파일 끝 근처에 반복 기록된다.
const TAIL_BYTES: u64 = 256 * 1024;

/// 롤링 윈도 한 개의 소진 상태.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UsageWindow {
    /// 소진율 0~100.
    pub used_percent: f64,
    /// 윈도가 리셋되는 시각(epoch secs). 소스가 안 주면 None.
    pub resets_at: Option<i64>,
    /// 윈도 길이(분). 300=5시간, 10080=주간.
    pub window_minutes: Option<u64>,
}

/// 벤더 한 곳의 잔량 스냅샷.
#[derive(Debug, Clone, Serialize)]
pub struct VendorUsage {
    /// agent::PRESETS의 key — "claude" | "codex" | "agy".
    pub vendor: String,
    pub label: String,
    /// "ok" | "stale" | "no_data" | "unauthenticated" | "unsupported" | "error".
    /// `stale`는 마지막 성공 관측을 그대로 싣되 새 값을 받지 못했다는 뜻이다.
    pub status: String,
    /// 상태 부연(툴팁). ok일 때는 None.
    pub detail: Option<String>,
    /// 구독 플랜 문자열(소스가 줄 때만).
    pub plan: Option<String>,
    pub five_hour: Option<UsageWindow>,
    pub weekly: Option<UsageWindow>,
    /// "session-log" | "statusline" | "oauth" | "manual-token" — 값의 출처.
    pub source: Option<String>,
    /// 데이터가 관측된 시각(epoch secs).
    pub updated_at: Option<i64>,
}

impl VendorUsage {
    /// 값 없는 상태 카드.
    fn blank(vendor: &str, label: &str, status: &str, detail: Option<String>) -> Self {
        Self {
            vendor: vendor.to_string(),
            label: label.to_string(),
            status: status.to_string(),
            detail,
            plan: None,
            five_hour: None,
            weekly: None,
            source: None,
            updated_at: None,
        }
    }
}

/// 하단 상태바가 한 번에 받는 전체 스냅샷.
#[derive(Debug, Clone, Serialize)]
pub struct UsageSnapshot {
    pub vendors: Vec<VendorUsage>,
    pub fetched_at: i64,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

/// 파일 끝에서 최대 `max` 바이트를 읽어 문자열로. 잘린 앞부분은 첫 개행까지 버려 라인 경계를 맞춘다.
pub(crate) fn tail_string(path: &Path, max: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(max);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    f.read_to_end(&mut buf).ok()?;
    let s = String::from_utf8_lossy(&buf).into_owned();
    if start == 0 {
        return Some(s);
    }
    // 앞이 잘렸으면 깨진 첫 줄을 버린다.
    s.find('\n').map(|i| s[i + 1..].to_string())
}

/// JSON 트리에서 `key`를 가진 첫 객체 값을 깊이 우선으로 찾는다.
/// 소스마다 중첩 위치가 달라(payload.rate_limits 등) 경로를 고정하지 않는다.
fn find_key<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    match v {
        serde_json::Value::Object(map) => {
            if let Some(found) = map.get(key) {
                return Some(found);
            }
            map.values().find_map(|c| find_key(c, key))
        }
        serde_json::Value::Array(items) => items.iter().find_map(|c| find_key(c, key)),
        _ => None,
    }
}

/// 여러 후보 키 중 먼저 잡히는 수치. 소스별 필드명 차이(utilization/used_percent…)를 흡수한다.
fn num_of(v: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| v.get(*k).and_then(|x| x.as_f64()))
}

/// 숫자 epoch 또는 Anthropic OAuth의 RFC3339 리셋 시각을 epoch 초로 읽는다.
fn timestamp_of(v: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| match v.get(*key) {
        Some(serde_json::Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value as i64)),
        Some(serde_json::Value::String(value)) => chrono::DateTime::parse_from_rfc3339(value)
            .ok()
            .map(|value| value.timestamp()),
        _ => None,
    })
}

/// 윈도 객체 하나를 파싱. 소진율이 없으면 윈도 자체를 버린다.
/// `resets_in_seconds`(상대)만 주는 소스는 `now`를 더해 절대 시각으로 바꾼다.
fn parse_window(v: &serde_json::Value, now: i64) -> Option<UsageWindow> {
    let used = num_of(v, &["used_percent", "used_percentage", "utilization"])?;
    let resets_at = timestamp_of(v, &["resets_at", "reset_at", "resets_at_epoch_seconds"])
        .or_else(|| num_of(v, &["resets_in_seconds"]).map(|n| now + n as i64));
    Some(UsageWindow {
        used_percent: used.clamp(0.0, 100.0),
        resets_at,
        window_minutes: num_of(v, &["window_minutes"]).map(|n| n as u64),
    })
}

/// `rate_limits` 객체(codex/statusline/oauth 공통 형태)를 5시간/주간 쌍으로.
/// codex는 primary/secondary, Claude는 five_hour/seven_day로 부른다.
fn parse_rate_limits(
    rl: &serde_json::Value,
    now: i64,
) -> (Option<UsageWindow>, Option<UsageWindow>) {
    let pick = |keys: &[&str]| -> Option<UsageWindow> {
        keys.iter()
            .find_map(|k| rl.get(*k))
            .and_then(|v| parse_window(v, now))
    };
    let short = pick(&["five_hour", "primary"]);
    let long = pick(&["seven_day", "weekly", "secondary"]);
    // 윈도 길이가 있으면 그것을 신뢰해 뒤바뀐 매핑을 바로잡는다(주간이 primary로 오는 소스 대비).
    match (&short, &long) {
        (Some(s), Some(l)) if s.window_minutes > l.window_minutes && l.window_minutes.is_some() => {
            (long, short)
        }
        (Some(window), None) if window.window_minutes == Some(10_080) => (None, short),
        (None, Some(window)) if window.window_minutes == Some(300) => (long, None),
        _ => (short, long),
    }
}

// ---------------------------------------------------------------- Codex

/// `~/.codex/sessions` 하위에서 최근 세션 로그 경로들 — 연/월/일 디렉터리를 역순으로 내려가
/// 최대 `limit`개만 고른다. 전체 walk는 세션이 쌓이면 비싸다.
fn codex_recent_sessions(root: &Path, limit: usize) -> Vec<PathBuf> {
    fn sorted_children(dir: &Path, want_dir: bool) -> Vec<PathBuf> {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut v: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() == want_dir)
            .collect();
        v.sort();
        v.reverse();
        v
    }

    let mut out = Vec::new();
    for year in sorted_children(root, true) {
        for month in sorted_children(&year, true) {
            for day in sorted_children(&month, true) {
                let mut files: Vec<PathBuf> = sorted_children(&day, false)
                    .into_iter()
                    .filter(|p| p.extension().map(|e| e == "jsonl").unwrap_or(false))
                    .collect();
                out.append(&mut files);
                if out.len() >= limit {
                    out.truncate(limit);
                    return out;
                }
            }
        }
    }
    out
}

/// 세션 로그 tail에서 가장 마지막 `rate_limits`와 그 줄의 타임스탬프.
pub fn codex_parse_tail(tail: &str, now: i64) -> Option<(serde_json::Value, Option<i64>)> {
    for line in tail.lines().rev() {
        if !line.contains("\"rate_limits\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(rl) = find_key(&v, "rate_limits") else {
            continue;
        };
        if rl.is_null() {
            continue;
        }
        let ts = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(parse_iso8601)
            .or(Some(now));
        return Some((rl.clone(), ts));
    }
    None
}

/// "2026-03-09T15:47:14.954Z" → epoch secs. 형식이 어긋나면 None.
fn parse_iso8601(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<i64> { s.get(a..z)?.parse().ok() };
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, se) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se)
}

/// 1970-01-01부터의 일수 (Howard Hinnant days_from_civil).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn codex_usage(now: i64) -> VendorUsage {
    let Some(home) = home_dir() else {
        return VendorUsage::blank(
            "codex",
            "Codex",
            "no_data",
            Some("홈 디렉터리를 찾지 못했습니다".into()),
        );
    };
    let root = home.join(".codex").join("sessions");
    if !root.is_dir() {
        return VendorUsage::blank(
            "codex",
            "Codex",
            "no_data",
            Some("~/.codex/sessions 없음 — Codex를 한 번 실행하세요".into()),
        );
    }
    // 최근 파일 몇 개를 훑어 가장 새 관측치를 고른다(마지막 세션이 rate_limits 없이 끝날 수 있다).
    let mut best: Option<(i64, serde_json::Value)> = None;
    for path in codex_recent_sessions(&root, 5) {
        let Some(tail) = tail_string(&path, TAIL_BYTES) else {
            continue;
        };
        let Some((rl, ts)) = codex_parse_tail(&tail, now) else {
            continue;
        };
        let ts = ts.unwrap_or(0);
        if best.as_ref().map(|(b, _)| ts > *b).unwrap_or(true) {
            best = Some((ts, rl));
        }
    }
    let Some((observed, rl)) = best else {
        return VendorUsage::blank(
            "codex",
            "Codex",
            "no_data",
            Some("최근 세션에 한도 정보가 없습니다".into()),
        );
    };
    let (five_hour, weekly) = parse_rate_limits(&rl, now);
    VendorUsage {
        vendor: "codex".into(),
        label: "Codex".into(),
        status: if five_hour.is_some() || weekly.is_some() {
            "ok".into()
        } else {
            "no_data".into()
        },
        detail: None,
        plan: rl
            .get("plan_type")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        five_hour,
        weekly,
        source: Some("session-log".into()),
        updated_at: Some(observed),
    }
}

// ---------------------------------------------------------------- Claude

/// statusline 브리지가 덤프하는 파일 경로 — `~/.claude/praxis-usage.json`.
pub fn bridge_path(home: &Path) -> PathBuf {
    home.join(".claude").join("praxis-usage.json")
}

/// 브리지 캐시에서 읽기. `{ "rate_limits": {...}, "updated_at": 172… }` 형태.
fn claude_from_bridge(home: &Path, now: i64) -> Option<VendorUsage> {
    let raw = std::fs::read_to_string(bridge_path(home)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let observed = v.get("updated_at").and_then(|t| t.as_i64()).unwrap_or(0);
    if now - observed > BRIDGE_FRESH_SECS {
        return None; // 낡음 — OAuth로 넘어간다.
    }
    let rl = find_key(&v, "rate_limits")?;
    let (five_hour, weekly) = parse_rate_limits(rl, now);
    if five_hour.is_none() && weekly.is_none() {
        return None;
    }
    Some(VendorUsage {
        vendor: "claude".into(),
        label: "Claude Code".into(),
        status: "ok".into(),
        detail: None,
        plan: v.get("plan").and_then(|p| p.as_str()).map(str::to_string),
        five_hour,
        weekly,
        source: Some("statusline".into()),
        updated_at: Some(observed),
    })
}

/// User-Agent에 넣을 Claude Code 버전. 없으면 소스 미상 폴백.
/// 엔드포인트가 `claude-code/<version>` UA가 아니면 더 좁은 레이트 버킷을 쓴다.
fn claude_cli_version() -> String {
    static CACHE: Mutex<Option<String>> = Mutex::new(None);
    if let Ok(guard) = CACHE.lock() {
        if let Some(v) = guard.as_ref() {
            return v.clone();
        }
    }
    let claude = crate::reviewer::which("claude").unwrap_or_else(|| "claude".to_string());
    let detected = std::process::Command::new(claude)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .next()
                .map(str::to_string)
        })
        .unwrap_or_else(|| "0.0.0".to_string());
    if let Ok(mut guard) = CACHE.lock() {
        *guard = Some(detected.clone());
    }
    detected
}

/// 사용량 엔드포인트. 토큰 종류(CLI 로그인 / 수동 등록)와 무관하게 같은 곳을 친다.
const USAGE_ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";
/// 만료·미조회 상태에서 마지막 관측을 보여줄 때의 안내.
const STALE_DETAIL: &str =
    "토큰이 만료돼 새 값을 받지 못했습니다 — claude CLI를 한 번 실행하면 갱신됩니다";

/// 조회 실패 사유. 호출부마다 문구가 달라야 해서(상태 카드 vs 토큰 검증) 코드만 올린다.
enum FetchError {
    /// 성공이 아닌 HTTP 상태 코드.
    Status(u16),
    /// 요청·응답 처리 실패 — 사용자에게 보여줄 한 줄.
    Failed(String),
    /// 2xx인데 한도 정보가 없다.
    NoData,
}

/// 주어진 토큰으로 사용량을 조회한다. 토큰은 이 함수 밖으로 나가지 않는다(로그·에러 문구 포함).
async fn fetch_usage(
    token: &str,
    plan: Option<String>,
    now: i64,
    source: &str,
) -> Result<VendorUsage, FetchError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|_| FetchError::Failed("HTTP 클라이언트 생성 실패".into()))?;
    let resp = client
        .get(USAGE_ENDPOINT)
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .header(
            "User-Agent",
            format!("claude-code/{}", claude_cli_version()),
        )
        .send()
        .await
        .map_err(|_| FetchError::Failed("사용량 조회 요청 실패".into()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(FetchError::Status(status.as_u16()));
    }
    let value = resp
        .json::<serde_json::Value>()
        .await
        .map_err(|_| FetchError::Failed("응답을 해석하지 못했습니다".into()))?;
    // 응답이 rate_limits로 감싸져 오기도, 최상위에 five_hour/seven_day가 오기도 한다.
    let rl = find_key(&value, "rate_limits").unwrap_or(&value);
    let (five_hour, weekly) = parse_rate_limits(rl, now);
    if five_hour.is_none() && weekly.is_none() {
        return Err(FetchError::NoData);
    }
    Ok(VendorUsage {
        vendor: "claude".into(),
        label: "Claude Code".into(),
        status: "ok".into(),
        detail: None,
        plan,
        five_hour,
        weekly,
        source: Some(source.into()),
        updated_at: Some(now),
    })
}

/// 조회 실패를 상태바 카드로. 401만 로그아웃으로 본다 — 나머지는 조회 실패다.
fn fetch_error_card(error: FetchError) -> VendorUsage {
    let (kind, detail) = match error {
        FetchError::Status(401) => (
            "unauthenticated",
            "인증 실패 — claude CLI로 다시 로그인하세요".to_string(),
        ),
        FetchError::Status(429) => ("error", "사용량 조회가 일시적으로 제한됐습니다".to_string()),
        FetchError::Status(code) => ("error", format!("사용량 조회 실패 ({code})")),
        FetchError::Failed(message) => ("error", message),
        FetchError::NoData => ("no_data", "응답에 한도 정보가 없습니다".to_string()),
    };
    VendorUsage::blank("claude", "Claude Code", kind, Some(detail))
}

/// 새 값을 못 받았을 때의 카드 — 마지막 성공 관측을 그대로 싣는다. 없으면 값 없는 안내.
fn claude_stale(home: &Path) -> VendorUsage {
    let Some(last) = last_success::load(home) else {
        return VendorUsage::blank(
            "claude",
            "Claude Code",
            "no_data",
            Some(STALE_DETAIL.into()),
        );
    };
    VendorUsage {
        vendor: "claude".into(),
        label: "Claude Code".into(),
        status: "stale".into(),
        detail: Some(STALE_DETAIL.into()),
        plan: last.plan,
        five_hour: last.five_hour,
        weekly: last.weekly,
        source: last.source,
        updated_at: Some(last.observed_at),
    }
}

/// CLI 자격증명으로 조회. 만료는 로그아웃이 아니므로 마지막 관측을 낡은 값으로 돌려준다.
async fn claude_from_oauth(home: &Path, now: i64) -> VendorUsage {
    let credentials = match load_credentials(home, now).await {
        Ok(credentials) => credentials,
        Err(CredentialError::Expired) => return claude_stale(home),
        Err(CredentialError::Unauthenticated) => {
            return VendorUsage::blank(
                "claude",
                "Claude Code",
                "unauthenticated",
                Some("자격증명을 찾지 못했습니다 — claude CLI로 로그인하세요".into()),
            )
        }
        Err(CredentialError::Error) => {
            return VendorUsage::blank(
                "claude",
                "Claude Code",
                "error",
                Some("Claude 자격증명을 읽지 못했습니다".into()),
            )
        }
    };
    let (token, plan) = credentials.into_parts();
    fetch_usage(&token, plan, now, "oauth")
        .await
        .unwrap_or_else(fetch_error_card)
}

/// OAuth 결과 캐시 — (조회 시각, 결과). 429를 피하려 `MIN_OAUTH_INTERVAL` 안에서는 재사용한다.
static OAUTH_CACHE: Mutex<Option<(i64, VendorUsage)>> = Mutex::new(None);

async fn claude_usage(now: i64, force: bool) -> VendorUsage {
    let Some(home) = home_dir() else {
        return VendorUsage::blank(
            "claude",
            "Claude Code",
            "no_data",
            Some("홈 디렉터리를 찾지 못했습니다".into()),
        );
    };
    // 브리지 캐시가 신선하면 네트워크를 타지 않는다.
    if let Some(v) = claude_from_bridge(&home, now) {
        last_success::save(&home, &v);
        return v;
    }
    if !force {
        if let Ok(guard) = OAUTH_CACHE.lock() {
            if let Some((at, cached)) = guard.as_ref() {
                if now - at < MIN_OAUTH_INTERVAL {
                    return cached.clone();
                }
            }
        }
    }
    // 수동 등록 토큰이 우선한다 — 만료 검사 없이 그대로 쓴다(장기 토큰이라 갱신이 없다).
    let fresh = match manual_token().await {
        Some(token) => fetch_usage(&token, None, now, MANUAL_SOURCE)
            .await
            .unwrap_or_else(fetch_error_card),
        None => claude_from_oauth(&home, now).await,
    };
    if fresh.status == "ok" {
        last_success::save(&home, &fresh);
    }
    if let Ok(mut guard) = OAUTH_CACHE.lock() {
        *guard = Some((now, fresh.clone()));
    }
    fresh
}

// -------------------------------------------------- 사용량 조회용 수동 토큰

/// 수동 등록 토큰의 키체인 kind. 값은 이 모듈 안에서만 다룬다.
const USAGE_TOKEN_KIND: &str = "claude_usage_token";
/// 수동 토큰으로 얻은 값의 출처 표시.
const MANUAL_SOURCE: &str = "manual-token";

async fn manual_token() -> Option<String> {
    crate::secret::get_secret(USAGE_TOKEN_KIND)
        .await
        .ok()
        .flatten()
        .filter(|token| !token.is_empty())
}

/// 검증 실패 문구 — 저장하지 않았다는 사실을 함께 알린다. 토큰 원문은 절대 싣지 않는다.
fn describe_token_error(error: FetchError) -> String {
    match error {
        FetchError::Status(code @ (401 | 403)) => format!(
            "이 토큰으로는 사용량을 조회할 수 없습니다(HTTP {code}) — 이 엔드포인트는 CLI 로그인 토큰만 받는 것일 수 있습니다"
        ),
        FetchError::Status(code) => {
            format!("사용량 조회에 실패해 저장하지 않았습니다 ({code})")
        }
        FetchError::Failed(message) => format!("{message} — 저장하지 않았습니다"),
        FetchError::NoData => "응답에 한도 정보가 없어 저장하지 않았습니다".to_string(),
    }
}

/// 수동 토큰 저장. **저장 전에 실호출로 검증한다** — 이 엔드포인트가 장기 토큰을 받는지는
/// 확인되지 않았으므로, 되지 않는 토큰을 키체인에 남기면 조용히 조회가 죽는다.
pub async fn set_manual_token(token: String) -> Result<VendorUsage, String> {
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("토큰이 비어 있습니다".into());
    }
    let now = now_secs();
    let usage = fetch_usage(&token, None, now, MANUAL_SOURCE)
        .await
        .map_err(describe_token_error)?;
    crate::secret::set_secret(USAGE_TOKEN_KIND, &token).await?;
    if let Some(home) = home_dir() {
        last_success::save(&home, &usage);
    }
    if let Ok(mut guard) = OAUTH_CACHE.lock() {
        *guard = Some((now, usage.clone()));
    }
    Ok(usage)
}

/// 수동 토큰 삭제. 다음 조회가 CLI 자격증명으로 돌아가도록 캐시도 비운다.
pub async fn clear_manual_token() -> Result<(), String> {
    crate::secret::clear_secret(USAGE_TOKEN_KIND).await?;
    if let Ok(mut guard) = OAUTH_CACHE.lock() {
        *guard = None;
    }
    Ok(())
}

/// 저장 여부만. 토큰 값은 어떤 경로로도 프론트에 돌려주지 않는다.
pub async fn manual_token_stored() -> Result<bool, String> {
    Ok(crate::secret::get_secret(USAGE_TOKEN_KIND).await?.is_some())
}

// ---------------------------------------------------------------- 스냅샷

/// 세 벤더의 현재 잔량. 실패한 벤더도 상태 카드로 남겨 UI 자리가 흔들리지 않게 한다.
pub async fn snapshot(force: bool) -> UsageSnapshot {
    let now = now_secs();
    let claude = claude_usage(now, force).await;
    let codex = tokio::task::spawn_blocking(move || codex_usage(now))
        .await
        .unwrap_or_else(|_| {
            VendorUsage::blank("codex", "Codex", "error", Some("집계 실패".into()))
        });
    let agy = VendorUsage::blank(
        "agy",
        "Antigravity",
        "unsupported",
        Some("잔량을 노출하는 로컬 소스가 없습니다".into()),
    );
    UsageSnapshot {
        vendors: vec![claude, codex, agy],
        fetched_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_tail_picks_last_rate_limits() {
        let tail = concat!(
            r#"{"timestamp":"2026-03-09T10:00:00.000Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":5.0,"window_minutes":300,"resets_at":100}}}}"#,
            "\n",
            r#"{"timestamp":"2026-03-09T15:47:14.954Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":14.0,"window_minutes":300,"resets_at":1773070872},"secondary":{"used_percent":25.0,"window_minutes":10080,"resets_at":1773577825},"plan_type":"plus"}}}"#,
            "\n",
        );
        let (rl, ts) = codex_parse_tail(tail, 0).expect("rate_limits 발견");
        assert_eq!(rl.get("plan_type").unwrap(), "plus");
        assert_eq!(ts, Some(parse_iso8601("2026-03-09T15:47:14.954Z").unwrap()));

        let (five, week) = parse_rate_limits(&rl, 0);
        assert_eq!(
            five,
            Some(UsageWindow {
                used_percent: 14.0,
                resets_at: Some(1773070872),
                window_minutes: Some(300),
            })
        );
        assert_eq!(week.unwrap().used_percent, 25.0);
    }

    #[test]
    fn codex_tail_skips_lines_without_limits() {
        let tail = concat!(
            r#"{"timestamp":"2026-03-09T10:00:00.000Z","payload":{"rate_limits":{"primary":{"used_percent":7.0}}}}"#,
            "\n",
            r#"{"timestamp":"2026-03-09T11:00:00.000Z","payload":{"type":"agent_message"}}"#,
            "\n",
            "not json at all\n",
        );
        let (rl, _) = codex_parse_tail(tail, 0).expect("앞선 줄에서 발견");
        assert_eq!(parse_rate_limits(&rl, 0).0.unwrap().used_percent, 7.0);
    }

    #[test]
    fn codex_tail_ignores_null_rate_limits() {
        // codex는 한도 정보가 아직 없을 때 rate_limits: null을 쓴다.
        let tail = concat!(
            r#"{"timestamp":"2026-03-09T10:00:00.000Z","payload":{"rate_limits":{"primary":{"used_percent":3.0}}}}"#,
            "\n",
            r#"{"timestamp":"2026-03-09T12:00:00.000Z","payload":{"rate_limits":null}}"#,
            "\n",
        );
        let (rl, _) = codex_parse_tail(tail, 0).expect("null은 건너뛰고 그 앞을 쓴다");
        assert_eq!(parse_rate_limits(&rl, 0).0.unwrap().used_percent, 3.0);
    }

    #[test]
    fn claude_statusline_field_names_parse() {
        // 공식 statusline JSON의 필드명(used_percentage/resets_at).
        let rl: serde_json::Value = serde_json::from_str(
            r#"{"five_hour":{"used_percentage":45,"resets_at":1738425600},
                "seven_day":{"used_percentage":80.5,"resets_at":1738857600}}"#,
        )
        .unwrap();
        let (five, week) = parse_rate_limits(&rl, 0);
        assert_eq!(five.unwrap().used_percent, 45.0);
        let week = week.unwrap();
        assert_eq!(week.used_percent, 80.5);
        assert_eq!(week.resets_at, Some(1738857600));
    }

    #[test]
    fn relative_reset_becomes_absolute() {
        let rl: serde_json::Value =
            serde_json::from_str(r#"{"primary":{"used_percent":10,"resets_in_seconds":600}}"#)
                .unwrap();
        let (five, _) = parse_rate_limits(&rl, 1_000_000);
        assert_eq!(five.unwrap().resets_at, Some(1_000_600));
    }

    #[test]
    fn windows_swap_when_lengths_are_reversed() {
        // 짧은 윈도가 secondary로 오는 소스도 길이를 보고 바로잡는다.
        let rl: serde_json::Value = serde_json::from_str(
            r#"{"primary":{"used_percent":9,"window_minutes":10080},
                "secondary":{"used_percent":3,"window_minutes":300}}"#,
        )
        .unwrap();
        let (five, week) = parse_rate_limits(&rl, 0);
        assert_eq!(five.unwrap().window_minutes, Some(300));
        assert_eq!(week.unwrap().window_minutes, Some(10080));
    }

    #[test]
    fn lone_weekly_primary_maps_to_weekly() {
        let rl: serde_json::Value =
            serde_json::from_str(r#"{"primary":{"used_percent":2,"window_minutes":10080}}"#)
                .unwrap();
        let (five, week) = parse_rate_limits(&rl, 0);
        assert!(five.is_none());
        assert_eq!(week.unwrap().window_minutes, Some(10080));
    }

    #[test]
    fn rfc3339_reset_string_is_parsed() {
        let window: serde_json::Value = serde_json::from_str(
            r#"{"used_percent":10,"resets_at":"2026-07-27T12:34:56.789+00:00"}"#,
        )
        .unwrap();
        assert_eq!(
            parse_window(&window, 0).unwrap().resets_at,
            Some(parse_iso8601("2026-07-27T12:34:56Z").unwrap())
        );
    }

    #[test]
    fn window_without_percent_is_dropped() {
        let rl: serde_json::Value =
            serde_json::from_str(r#"{"five_hour":{"resets_at":123}}"#).unwrap();
        assert_eq!(parse_rate_limits(&rl, 0).0, None);
    }

    #[test]
    fn percent_is_clamped() {
        let rl: serde_json::Value =
            serde_json::from_str(r#"{"five_hour":{"used_percentage":140}}"#).unwrap();
        assert_eq!(parse_rate_limits(&rl, 0).0.unwrap().used_percent, 100.0);
    }

    #[test]
    fn expired_token_is_not_a_logout() {
        // CLI가 refresh token으로 스스로 갱신하므로 만료는 "로그인 필요"가 아니다.
        let result = claude_credentials::parse_credentials_json(
            r#"{"claudeAiOauth":{"accessToken":"test-token","expiresAt":1000000}}"#,
            2_000,
        );
        assert!(matches!(result, Err(CredentialError::Expired)));
    }

    #[test]
    fn stale_card_carries_last_success() {
        let home = crate::testtmp::dir().join(format!("praxis-stale-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(
            last_success::path(&home),
            r#"{"plan":"max","five_hour":{"used_percent":30.0,"resets_at":null,"window_minutes":300},
                "weekly":null,"observed_at":1700,"source":"statusline"}"#,
        )
        .unwrap();

        let card = claude_stale(&home);
        assert_eq!(card.status, "stale");
        assert_eq!(card.five_hour.unwrap().used_percent, 30.0);
        assert_eq!(card.plan.as_deref(), Some("max"));
        assert_eq!(card.updated_at, Some(1700));
        assert_eq!(card.source.as_deref(), Some("statusline"));
        assert!(card.detail.unwrap().contains("claude CLI"));
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn stale_without_last_success_is_no_data() {
        let home = crate::testtmp::dir().join(format!("praxis-stale-empty-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let card = claude_stale(&home);
        assert_eq!(card.status, "no_data");
        assert!(card.five_hour.is_none());
        assert!(card.detail.unwrap().contains("claude CLI"));
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn only_401_is_reported_as_logged_out() {
        assert_eq!(
            fetch_error_card(FetchError::Status(401)).status,
            "unauthenticated"
        );
        assert_eq!(fetch_error_card(FetchError::Status(403)).status, "error");
        assert_eq!(fetch_error_card(FetchError::Status(429)).status, "error");
        assert_eq!(fetch_error_card(FetchError::NoData).status, "no_data");
    }

    #[test]
    fn rejected_token_message_names_the_code_without_the_token() {
        let message = describe_token_error(FetchError::Status(403));
        assert!(message.contains("HTTP 403"));
        assert!(message.contains("CLI 로그인 토큰"));
    }

    #[test]
    fn credential_json_parses_token_and_plan() {
        let result = claude_credentials::parse_credentials_json(
            r#"{"claudeAiOauth":{"accessToken":"test-token","expiresAt":2000000,"subscriptionType":"max"}}"#,
            1_000,
        );
        let Ok(credentials) = result else {
            panic!("valid credentials must parse");
        };
        let (token, plan) = credentials.into_parts();
        assert_eq!(token, "test-token");
        assert_eq!(plan.as_deref(), Some("max"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn security_child_timeout_is_killed_and_reaped() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant};

        let dir = crate::testtmp::dir().join(format!("praxis-security-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let stub = dir.join("security-stub");
        std::fs::write(&stub, "#!/bin/sh\nwhile :; do :; done\n").unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o700)).unwrap();

        let started = Instant::now();
        let result = claude_credentials::read_security_password(
            &stub,
            "test-account",
            Duration::from_millis(50),
        );
        assert!(matches!(result, Err(CredentialError::Error)));
        assert!(started.elapsed() < Duration::from_secs(1));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stale_bridge_cache_is_ignored() {
        let dir = crate::testtmp::dir().join(format!("praxis-bridge-test-{}", std::process::id()));
        let claude = dir.join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        let body = r#"{"updated_at":1000,"rate_limits":{"five_hour":{"used_percentage":30}}}"#;
        std::fs::write(bridge_path(&dir), body).unwrap();

        assert!(
            claude_from_bridge(&dir, 1000 + BRIDGE_FRESH_SECS + 1).is_none(),
            "낡으면 무시"
        );
        let fresh = claude_from_bridge(&dir, 1000 + 10).expect("신선하면 사용");
        assert_eq!(fresh.source.as_deref(), Some("statusline"));
        assert_eq!(fresh.five_hour.unwrap().used_percent, 30.0);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 실제 홈 디렉터리를 읽는 스모크 — 환경에 의존하므로 기본 실행에서는 제외한다.
    /// `cargo test --lib usage -- --ignored --nocapture`로 눈으로 확인한다.
    #[test]
    #[ignore]
    fn local_snapshot_smoke() {
        let snap = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(snapshot(true));
        for v in &snap.vendors {
            println!(
                "{:12} {:16} 5h={:?} week={:?} src={:?} detail={:?}",
                v.vendor,
                v.status,
                v.five_hour.as_ref().map(|w| w.used_percent),
                v.weekly.as_ref().map(|w| w.used_percent),
                v.source,
                v.detail
            );
        }
        assert_eq!(snap.vendors.len(), 3);
    }

    #[test]
    fn recent_sessions_are_newest_first() {
        let root = crate::testtmp::dir().join(format!("praxis-sess-test-{}", std::process::id()));
        for (y, m, d) in [("2026", "03", "07"), ("2026", "03", "09")] {
            let day = root.join(y).join(m).join(d);
            std::fs::create_dir_all(&day).unwrap();
            std::fs::write(day.join(format!("rollout-{y}{m}{d}.jsonl")), "{}").unwrap();
        }
        let found = codex_recent_sessions(&root, 5);
        assert_eq!(found.len(), 2);
        assert!(
            found[0].to_string_lossy().contains("20260309"),
            "최신 날짜 먼저: {found:?}"
        );
        std::fs::remove_dir_all(&root).ok();
    }
}
