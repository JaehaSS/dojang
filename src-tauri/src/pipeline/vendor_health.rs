//! 벤더 할당량 소진 상태 — 프로세스 전역, TTL 1시간.
//! 소진된 벤더를 재배정·리뷰어 선정에서 빼서, 같은 429를 반복해서 맞고 승인 요청이 쌓이는 일을 막는다.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 소진 표시를 유지하는 시간. 지나면 다시 시도해 본다.
pub const EXHAUSTED_TTL: Duration = Duration::from_secs(3600);

const QUOTA_MARKERS: [&str; 8] = [
    "resource_exhausted",
    "quota",
    "rate limit",
    "rate_limit",
    "usage limit",
    "code 429",
    "status 429",
    "too many requests",
];

/// 에이전트 오류 문구가 할당량·속도 제한 소진을 가리키는가(대소문자 무시).
pub fn is_quota_error(text: &str) -> bool {
    let lower = text.to_lowercase();
    QUOTA_MARKERS.iter().any(|m| lower.contains(m))
}

/// 사유 문구를 한 줄 80자로 줄인다(UI 표시용).
fn short_reason(reason: &str) -> String {
    let line = reason.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    line.chars().take(80).collect()
}

#[derive(Default)]
pub struct Health {
    exhausted: HashMap<String, (Instant, String)>,
}

impl Health {
    pub fn mark(&mut self, vendor: &str, reason: &str, now: Instant) {
        self.exhausted.insert(vendor.to_string(), (now, short_reason(reason)));
    }

    pub fn reason(&self, vendor: &str, now: Instant) -> Option<String> {
        let (at, reason) = self.exhausted.get(vendor)?;
        (now.saturating_duration_since(*at) < EXHAUSTED_TTL).then(|| reason.clone())
    }
}

fn global() -> &'static Mutex<Health> {
    static G: OnceLock<Mutex<Health>> = OnceLock::new();
    G.get_or_init(Default::default)
}

pub fn mark_exhausted(vendor: &str, reason: &str) {
    global().lock().unwrap().mark(vendor, reason, Instant::now());
}

pub fn exhausted_reason(vendor: &str) -> Option<String> {
    global().lock().unwrap().reason(vendor, Instant::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_quota_errors() {
        assert!(is_quota_error(r#"AGY_ERROR: {"short_error":"RESOURCE_EXHAUSTED (code 429)"}"#));
        assert!(is_quota_error("error: Individual quota reached"));
        assert!(is_quota_error("Rate Limit exceeded"));
        assert!(is_quota_error("rate_limit_error"));
        assert!(is_quota_error("You hit your usage limit"));
        assert!(is_quota_error("HTTP status 429"));
        assert!(is_quota_error("Too Many Requests"));
        assert!(!is_quota_error("SyntaxError: invalid syntax"));
        assert!(!is_quota_error(""));
    }

    #[test]
    fn exhaustion_expires_after_ttl() {
        let mut h = Health::default();
        let t0 = Instant::now();
        assert_eq!(h.reason("agy", t0), None);
        h.mark("agy", "RESOURCE_EXHAUSTED (code 429)\nmore", t0);
        assert_eq!(h.reason("agy", t0 + Duration::from_secs(10)).as_deref(), Some("RESOURCE_EXHAUSTED (code 429)"));
        assert_eq!(h.reason("codex", t0), None);
        assert_eq!(h.reason("agy", t0 + EXHAUSTED_TTL), None);
    }
}
