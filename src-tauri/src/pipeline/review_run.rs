//! 구조화 리뷰 fan-out — 벤더별 병렬 실행·파싱·종합. 실제 벤더 호출은 주입된 `ReviewerFn`이 맡는다(테스트는 가짜 클로저).
//! 드라이버는 `reviewer::run_reviewer_registered`를 감싼 클로저를 넘긴다. 모든 함수는 벤더 호출이 끝날 때까지 블로킹이다.

use crate::multireview::verdict::{
    build_verdict_review_prompt, build_verdict_synthesis_prompt, has_blocking, parse_findings,
    parse_synthesis, Finding, ReviewKind, Severity, SynthDecision, Synthesis,
};
use crate::multireview::ModelInfo;
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;

/// (vendor, prompt, timeout_secs) → stdout 텍스트 또는 에러 문자열.
pub type ReviewerFn = dyn Fn(&str, &str, u64) -> Result<String, String> + Send + Sync;

/// 종합 판정자(고정).
pub const JUDGE_VENDOR: &str = "claude";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VendorReview {
    pub vendor: String,
    pub ok: bool,
    pub findings: Vec<Finding>,
    pub raw: String,
    pub error: Option<String>,
}

/// nonce용 난수(OS 시드 RandomState).
pub fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0),
    );
    h.finish()
}

fn new_nonce(prefix: &str) -> String {
    format!("{prefix}-{}-{:016x}", std::process::id(), random_u64())
}

fn review_one(vendor: &str, prompt: &str, nonce: &str, timeout: u64, runner: &ReviewerFn) -> VendorReview {
    let fail = |raw: String, error: String| VendorReview {
        vendor: vendor.to_string(),
        ok: false,
        findings: Vec::new(),
        raw,
        error: Some(error),
    };
    match runner(vendor, prompt, timeout) {
        Err(e) => fail(String::new(), e),
        Ok(raw) => match parse_findings(&raw, nonce) {
            Ok(findings) => VendorReview {
                vendor: vendor.to_string(),
                ok: true,
                findings,
                raw,
                error: None,
            },
            Err(e) => fail(raw, e),
        },
    }
}

/// 벤더마다 스레드 하나로 같은 프롬프트(실행당 nonce 1개)를 병렬 실행한다. 결과 순서는 `vendors` 순서와 같다.
pub fn run_parallel_reviews(
    kind: ReviewKind,
    focus: &str,
    content: &str,
    vendors: &[String],
    timeout: u64,
    runner: Arc<ReviewerFn>,
) -> (String, String, Vec<VendorReview>) {
    let nonce = new_nonce("PRAXIS-REVIEW");
    let prompt = build_verdict_review_prompt(kind, focus, content, &nonce);
    let reviews = std::thread::scope(|s| {
        let handles: Vec<_> = vendors
            .iter()
            .map(|v| {
                let (prompt, nonce, runner) = (&prompt, &nonce, &runner);
                s.spawn(move || review_one(v, prompt, nonce, timeout, runner.as_ref()))
            })
            .collect();
        handles
            .into_iter()
            .zip(vendors)
            .map(|(h, v)| {
                h.join().unwrap_or_else(|_| VendorReview {
                    vendor: v.clone(),
                    ok: false,
                    findings: Vec::new(),
                    raw: String::new(),
                    error: Some("리뷰 스레드가 비정상 종료했습니다".into()),
                })
            })
            .collect()
    });
    (prompt, nonce, reviews)
}

/// 티켓 리뷰: 벤더 1명, 종합 없음. 반환은 (프롬프트, 결과).
pub fn single_review(
    kind: ReviewKind,
    focus: &str,
    content: &str,
    vendor: &str,
    timeout: u64,
    runner: Arc<ReviewerFn>,
) -> (String, VendorReview) {
    let nonce = new_nonce("PRAXIS-REVIEW");
    let prompt = build_verdict_review_prompt(kind, focus, content, &nonce);
    let r = review_one(vendor, &prompt, &nonce, timeout, runner.as_ref());
    (prompt, r)
}

/// Claude가 성공한 리뷰들을 종합한다. 성공 리뷰가 없으면 Err. 반환은 (프롬프트, 원문, 종합).
pub fn synthesize(
    kind: ReviewKind,
    focus: &str,
    reviews: &[VendorReview],
    runner: Arc<ReviewerFn>,
    timeout: u64,
) -> Result<(String, String, Synthesis), String> {
    let ok: Vec<(String, Vec<Finding>)> = reviews
        .iter()
        .filter(|r| r.ok)
        .map(|r| (r.vendor.clone(), r.findings.clone()))
        .collect();
    if ok.is_empty() {
        return Err("성공한 리뷰가 없어 종합할 수 없습니다".into());
    }
    let failed: Vec<String> = reviews.iter().filter(|r| !r.ok).map(|r| r.vendor.clone()).collect();
    let valid: Vec<String> = ok.iter().map(|(v, _)| v.clone()).collect();
    let nonce = new_nonce("PRAXIS-SYNTH");
    let prompt = build_verdict_synthesis_prompt(kind, focus, &ok, &failed, &nonce);
    let raw = runner(JUDGE_VENDOR, &prompt, timeout)?;
    let synth = parse_synthesis(&raw, &nonce, &valid)?;
    Ok((prompt, raw, synth))
}

/// 지적 목록의 사람이 읽는 텍스트. 비어 있으면 "지적 없음".
pub fn findings_markdown(findings: &[Finding]) -> String {
    if findings.is_empty() {
        return "지적 없음".into();
    }
    findings
        .iter()
        .map(|f| {
            let sev = if f.severity == Severity::Blocking { "blocking" } else { "advisory" };
            let loc = match (&f.file, f.line) {
                (Some(p), Some(l)) => format!(" ({p}:{l})"),
                (Some(p), None) => format!(" ({p})"),
                _ => String::new(),
            };
            let req = f.requirement.as_deref().map(|r| format!("[{r}] ")).unwrap_or_default();
            format!("- [{sev}] {req}{}{loc}", f.summary)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn synthesis_markdown(s: &Synthesis) -> String {
    let d = if s.decision == SynthDecision::Fix { "fix" } else { "pass" };
    let list: Vec<String> = s
        .findings
        .iter()
        .map(|sf| {
            let src = if sf.sources.is_empty() { String::new() } else { format!(" (출처: {})", sf.sources.join(", ")) };
            format!("{}{src}", findings_markdown(std::slice::from_ref(&sf.finding)))
        })
        .collect();
    let body = if list.is_empty() { "지적 없음".to_string() } else { list.join("\n") };
    format!("판정: {d}\n{}\n\n{body}", s.summary)
}

/// `reviews` 테이블에 넣을 직렬화 결과.
#[derive(Debug, Clone)]
pub struct ReviewRecord {
    pub result_json: String,
    pub model_info_json: String,
    pub ok_count: i64,
    pub total: i64,
}

/// 기존 `MultiReviewResult`({items:[{vendor,ok,text}], synthesis: string|null})와 model_info 래퍼
/// ({items:[ModelInfo], synthesis: ModelInfo|null}) 모양을 따르고, 구조화 데이터를 추가 필드로 싣는다:
/// `vendor_reviews`(findings·raw·error 포함), `synthesis_structured`(Synthesis), `blocking`(bool).
/// `model_of`는 벤더의 모델명을 돌려준다(모르면 빈 문자열).
pub fn build_review_record(
    reviews: &[VendorReview],
    synthesis: Option<&Synthesis>,
    model_of: &dyn Fn(&str) -> String,
) -> ReviewRecord {
    let items: Vec<_> = reviews
        .iter()
        .map(|r| {
            let text = if r.ok {
                findings_markdown(&r.findings)
            } else {
                r.error.clone().unwrap_or_else(|| "리뷰 실패".into())
            };
            json!({"vendor": r.vendor, "ok": r.ok, "text": text})
        })
        .collect();
    let blocking = match synthesis {
        Some(s) => s.decision == SynthDecision::Fix,
        None => reviews.iter().any(|r| has_blocking(&r.findings)),
    };
    let result = json!({
        "items": items,
        "synthesis": synthesis.map(synthesis_markdown),
        "vendor_reviews": reviews,
        "synthesis_structured": synthesis,
        "blocking": blocking,
    });
    let info = |v: &str| ModelInfo {
        vendor: v.to_string(),
        model: model_of(v),
        cmd: crate::reviewer::describe_invocation(v),
    };
    let model_info = json!({
        "items": reviews.iter().map(|r| info(&r.vendor)).collect::<Vec<_>>(),
        "synthesis": synthesis.map(|_| info(JUDGE_VENDOR)),
    });
    ReviewRecord {
        result_json: result.to_string(),
        model_info_json: model_info.to_string(),
        ok_count: reviews.iter().filter(|r| r.ok).count() as i64,
        total: reviews.len() as i64,
    }
}

#[cfg(test)]
#[path = "review_run_tests.rs"]
mod tests;
