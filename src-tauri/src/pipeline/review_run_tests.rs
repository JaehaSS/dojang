use super::*;
use std::sync::Mutex;

fn vs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// 프롬프트에서 nonce 줄(PRAXIS-…)을 뽑아 `body`를 뒤에 붙여 응답을 만든다.
fn reply(prompt: &str, body: &str) -> String {
    let nonce = prompt
        .lines()
        .find(|l| l.starts_with("PRAXIS-REVIEW-") || l.starts_with("PRAXIS-SYNTH-"))
        .unwrap();
    format!("{nonce}\n{body}")
}

const BLOCKING: &str = r#"{"findings":[{"severity":"blocking","summary":"bad","evidence":"e","file":"a.rs","line":3}]}"#;
const CLEAN: &str = r#"{"findings":[]}"#;

#[test]
fn parallel_all_ok_in_order_with_shared_nonce() {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let s2 = seen.clone();
    let runner: Arc<ReviewerFn> = Arc::new(move |v, p, _| {
        s2.lock().unwrap().push(p.to_string());
        Ok(reply(p, if v == "codex" { BLOCKING } else { CLEAN }))
    });
    let (prompt, nonce, rs) =
        run_parallel_reviews(ReviewKind::Final, "", "diff", &vs(&["claude", "codex", "agy"]), 5, runner);
    assert!(prompt.contains(&nonce));
    assert_eq!(rs.iter().map(|r| r.vendor.as_str()).collect::<Vec<_>>(), ["claude", "codex", "agy"]);
    assert!(rs.iter().all(|r| r.ok));
    assert_eq!(rs[1].findings.len(), 1);
    assert!(seen.lock().unwrap().iter().all(|p| *p == prompt));
}

#[test]
fn vendor_error_and_parse_failure_are_not_ok() {
    let runner: Arc<ReviewerFn> = Arc::new(|v, p, _| match v {
        "codex" => Err("boom".into()),
        "agy" => Ok("no nonce here {\"findings\":[]}".into()),
        _ => Ok(reply(p, CLEAN)),
    });
    let (_, _, rs) = run_parallel_reviews(ReviewKind::Plan, "", "x", &vs(&["claude", "codex", "agy"]), 5, runner);
    assert!(rs[0].ok);
    assert!(!rs[1].ok && rs[1].error.as_deref() == Some("boom"));
    assert!(!rs[2].ok && rs[2].error.is_some() && !rs[2].raw.is_empty());
}

#[test]
fn synthesis_forces_fix_when_blocking_and_filters_sources() {
    let reviews = vec![
        VendorReview { vendor: "claude".into(), ok: true, findings: vec![], raw: String::new(), error: None },
        VendorReview { vendor: "codex".into(), ok: false, findings: vec![], raw: String::new(), error: Some("x".into()) },
    ];
    let runner: Arc<ReviewerFn> = Arc::new(|v, p, _| {
        assert_eq!(v, "claude");
        assert!(p.contains("codex"), "실패 벤더가 프롬프트에 표시된다");
        Ok(reply(
            p,
            r#"{"decision":"pass","summary":"s","findings":[{"severity":"blocking","summary":"b","evidence":"e","sources":["claude","ghost"]}]}"#,
        ))
    });
    let (prompt, raw, s) = synthesize(ReviewKind::Final, "", &reviews, runner, 5).unwrap();
    assert!(!prompt.is_empty() && !raw.is_empty());
    assert_eq!(s.decision, SynthDecision::Fix);
    assert_eq!(s.findings[0].sources, vec!["claude"]);
}

#[test]
fn synthesis_errors_without_ok_reviews_or_bad_output() {
    let runner: Arc<ReviewerFn> = Arc::new(|_, _, _| Ok("garbage".into()));
    let none = vec![VendorReview { vendor: "codex".into(), ok: false, findings: vec![], raw: String::new(), error: None }];
    assert!(synthesize(ReviewKind::Plan, "", &none, runner.clone(), 5).is_err());
    let ok = vec![VendorReview { vendor: "claude".into(), ok: true, findings: vec![], raw: String::new(), error: None }];
    assert!(synthesize(ReviewKind::Plan, "", &ok, runner, 5).is_err());
}

#[test]
fn single_review_ok_and_error() {
    let ok: Arc<ReviewerFn> = Arc::new(|_, p, _| Ok(reply(p, BLOCKING)));
    let (prompt, r) = single_review(ReviewKind::Ticket, "f", "diff", "codex", 5, ok);
    assert!(r.ok && r.vendor == "codex" && has_blocking(&r.findings));
    assert!(prompt.contains("diff"));
    let bad: Arc<ReviewerFn> = Arc::new(|_, _, _| Err("timeout".into()));
    let (_, r) = single_review(ReviewKind::Ticket, "f", "diff", "agy", 5, bad);
    assert!(!r.ok && r.error.as_deref() == Some("timeout"));
}

#[test]
fn record_matches_existing_shapes() {
    let reviews = vec![
        VendorReview {
            vendor: "claude".into(),
            ok: true,
            findings: vec![],
            raw: String::new(),
            error: None,
        },
        VendorReview { vendor: "codex".into(), ok: false, findings: vec![], raw: String::new(), error: Some("e".into()) },
    ];
    let rec = build_review_record(&reviews, None, &|_| "m".into());
    let v: serde_json::Value = serde_json::from_str(&rec.result_json).unwrap();
    assert_eq!(v["items"][0]["vendor"], "claude");
    assert_eq!(v["items"][1]["text"], "e");
    assert!(v["synthesis"].is_null());
    assert_eq!(v["vendor_reviews"].as_array().unwrap().len(), 2);
    assert_eq!((rec.ok_count, rec.total), (1, 2));
    let m: serde_json::Value = serde_json::from_str(&rec.model_info_json).unwrap();
    assert_eq!(m["items"][0]["model"], "m");
    assert!(m["synthesis"].is_null());
}
