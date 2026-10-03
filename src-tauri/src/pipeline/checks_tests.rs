use super::*;

fn tmp(tag: &str) -> std::path::PathBuf {
    let d = crate::testtmp::dir().join(format!("pipe-chk-{tag}-{:x}", crate::pipeline::review_run::random_u64()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn run_shell_pass_fail_and_cwd() {
    let d = tmp("sh");
    std::fs::write(d.join("marker"), "x").unwrap();
    let ok = run_shell(&d, "test -f marker && echo hi", 10, None);
    assert!(ok.ok && ok.exit_code == 0 && !ok.timed_out);
    assert!(ok.tail.contains("hi"));
    let bad = run_shell(&d, "echo boom >&2; exit 3", 10, None);
    assert!(!bad.ok && bad.exit_code == 3 && !bad.timed_out);
    assert!(bad.tail.contains("boom"));
}

#[test]
fn run_shell_timeout() {
    let d = tmp("to");
    let t = std::time::Instant::now();
    let r = run_shell(&d, "sleep 30", 1, None);
    assert!(r.timed_out && !r.ok);
    assert!(t.elapsed().as_secs() < 15);
}

#[test]
fn tail_is_bounded() {
    let d = tmp("tail");
    let r = run_shell(&d, "yes abcdefghij | head -c 20000", 10, None);
    assert!(r.tail.chars().count() <= TAIL_MAX_CHARS);
}

fn validate(d: &std::path::Path, build: &str, test: &str) {
    std::fs::create_dir_all(d.join(".praxis")).unwrap();
    std::fs::write(
        d.join(".praxis/validate.toml"),
        format!("build = \"{build}\"\ntest = \"{test}\"\ntimeout_secs = 10\n"),
    )
    .unwrap();
}

#[test]
fn ticket_checks_run_spec_then_acceptance() {
    let d = tmp("all");
    validate(&d, "true", "true");
    let r = run_ticket_checks(&d, &["echo acc".into(), "  ".into()], None);
    assert_eq!(r.len(), 3);
    assert!(checks_passed(&r));
    assert_eq!(r[2].command, "echo acc");
}

#[test]
fn build_failure_skips_acceptance() {
    let d = tmp("bf");
    validate(&d, "exit 2", "true");
    let r = run_ticket_checks(&d, &["echo acc".into()], None);
    assert_eq!(r.len(), 2);
    assert!(!r[0].ok);
    assert!(!checks_passed(&r));
    assert!(r.iter().all(|o| o.command != "echo acc"));
    assert!(failure_summary(&r).contains("exit 2"));
}

#[test]
fn acceptance_failure_reported_and_empty_is_not_pass() {
    let d = tmp("af");
    validate(&d, "true", "true");
    let r = run_ticket_checks(&d, &["echo a".into(), "exit 1".into()], None);
    assert!(!checks_passed(&r));
    let s = failure_summary(&r);
    assert!(s.contains("exit 1") && !s.contains("echo a"));
    assert!(!checks_passed(&[]));
}
