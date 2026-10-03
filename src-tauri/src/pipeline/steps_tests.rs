use super::*;
use std::path::PathBuf;
use std::process::Command;

fn sh(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git").current_dir(dir).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn repo(tag: &str) -> PathBuf {
    let dir = crate::testtmp::dir().join(format!("pipe-steps-{tag}-{:x}", crate::pipeline::review_run::random_u64()));
    std::fs::create_dir_all(&dir).unwrap();
    sh(&dir, &["init", "-q", "-b", "main"]);
    sh(&dir, &["config", "user.name", "t"]);
    sh(&dir, &["config", "user.email", "t@t"]);
    std::fs::write(dir.join("base.txt"), "base\n").unwrap();
    sh(&dir, &["add", "-A"]);
    sh(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

fn write(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

fn v(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn status(dir: &Path) -> String {
    sh(dir, &["status", "--porcelain"])
}

#[test]
fn verify_commits_work_and_drops_check_artifacts() {
    let d = repo("verify-artifacts");
    let fork = git_ops::head(&d).unwrap();
    write(&d, "src/a.rs", "fn a() {}\n");
    let out = verify_ticket(&d, &fork, &v(&["src/"]), &v(&["echo built > artifact.out"]), None).unwrap();
    assert!(out.passed, "{out:?}");
    // 산출물은 허용 경로 밖이지만 변경으로 집계되지 않고, 커밋에도 들어가지 않는다.
    assert_eq!(git_ops::committed_paths(&d, &fork).unwrap(), vec!["src/a.rs"]);
    assert!(!d.join("artifact.out").exists());
    assert_eq!(status(&d), "");
}

#[test]
fn verify_rejects_paths_outside_allowed_without_running_checks() {
    let d = repo("verify-outside");
    let fork = git_ops::head(&d).unwrap();
    write(&d, "src/a.rs", "x");
    write(&d, "other/b.rs", "y");
    let out = verify_ticket(&d, &fork, &v(&["src/"]), &v(&["echo ran > ran.out"]), None).unwrap();
    assert!(!out.passed);
    assert!(out.summary.contains("other/b.rs"), "{}", out.summary);
    assert!(!d.join("ran.out").exists());
}

#[test]
fn verify_reports_no_change_and_failing_checks() {
    let d = repo("verify-fail");
    let fork = git_ops::head(&d).unwrap();
    let none = verify_ticket(&d, &fork, &v(&["src/"]), &v(&["true"]), None).unwrap();
    assert!(!none.passed && none.summary.contains("변경이 하나도 없습니다"));
    write(&d, "src/a.rs", "x");
    let bad = verify_ticket(&d, &fork, &v(&["src/"]), &v(&["echo junk > j.out; exit 3"]), None).unwrap();
    assert!(!bad.passed && bad.summary.contains("exit 3"), "{}", bad.summary);
    assert!(!d.join("j.out").exists());
    assert_eq!(status(&d), "");
}

#[test]
fn verify_after_a_fix_round_counts_the_fix_commit_too() {
    let d = repo("verify-rounds");
    let fork = git_ops::head(&d).unwrap();
    write(&d, "src/a.rs", "v1");
    assert!(verify_ticket(&d, &fork, &v(&["src/"]), &v(&["true"]), None).unwrap().passed);
    // 수정 라운드: 수리 에이전트가 파일을 더 고친다.
    write(&d, "src/a.rs", "v2");
    write(&d, "src/c.rs", "c");
    assert!(verify_ticket(&d, &fork, &v(&["src/"]), &v(&["true"]), None).unwrap().passed);
    assert_eq!(git_ops::committed_paths(&d, &fork).unwrap(), vec!["src/a.rs", "src/c.rs"]);
    assert_eq!(std::fs::read_to_string(d.join("src/a.rs")).unwrap(), "v2");
}

#[test]
fn final_fix_commits_in_scope_change_and_cleans_artifacts() {
    let d = repo("fix-ok");
    let pre = git_ops::head(&d).unwrap();
    let r = apply_final_fix(&d, &pre, &v(&["src/"]), &v(&["echo b > build.out"]), None, || {
        write(&d, "src/a.rs", "fixed");
        Ok("ok".into())
    });
    assert_eq!(r, Ok(()));
    assert_ne!(git_ops::head(&d).unwrap(), pre);
    assert!(!d.join("build.out").exists());
    assert_eq!(status(&d), "");
}

#[test]
fn final_fix_outside_allowed_paths_is_reverted() {
    let d = repo("fix-outside");
    let pre = git_ops::head(&d).unwrap();
    let r = apply_final_fix(&d, &pre, &v(&["src/"]), &v(&["true"]), None, || {
        write(&d, "src/a.rs", "ok");
        write(&d, "secrets/k.txt", "no");
        Ok(String::new())
    });
    let e = r.unwrap_err();
    assert!(e.contains("secrets/k.txt"), "{e}");
    assert_eq!(git_ops::head(&d).unwrap(), pre);
    assert!(!d.join("secrets").exists() && !d.join("src/a.rs").exists());
}

#[test]
fn final_fix_failures_revert_to_pre() {
    let d = repo("fix-fail");
    let pre = git_ops::head(&d).unwrap();
    let verify_fail = apply_final_fix(&d, &pre, &v(&["src/"]), &v(&["exit 1"]), None, || {
        write(&d, "src/a.rs", "x");
        Ok(String::new())
    });
    assert!(verify_fail.unwrap_err().contains("검증 실패"));
    assert_eq!(git_ops::head(&d).unwrap(), pre);
    assert_eq!(status(&d), "");
    let agent_fail = apply_final_fix(&d, &pre, &v(&["src/"]), &v(&["true"]), None, || {
        write(&d, "src/half.rs", "half");
        Err("에이전트 종료".into())
    });
    assert_eq!(agent_fail, Err("에이전트 종료".to_string()));
    assert!(!d.join("src/half.rs").exists());
    let noop = apply_final_fix(&d, &pre, &v(&["src/"]), &v(&["true"]), None, || Ok(String::new()));
    assert!(noop.unwrap_err().contains("변경도 만들지"));
}

#[test]
fn succeeded_integration_is_never_reset_on_resume() {
    use IntegrationRecovery::*;
    let pre = || Some("abc".to_string());
    // 통합 성공 뒤 상태 전이만 실패한 경우: 합쳐진 결과를 지우면 안 된다.
    assert_eq!(integration_recovery(true, Some("succeeded"), pre()), FinishTransition);
    assert_eq!(integration_recovery(true, Some("running"), pre()), ResetTo("abc".into()));
    assert_eq!(integration_recovery(true, Some("failed"), pre()), ResetTo("abc".into()));
    assert_eq!(integration_recovery(true, None, None), FromHead);
    assert_eq!(integration_recovery(false, Some("running"), pre()), FromHead);
}

#[test]
fn pre_marker_roundtrips_with_and_without_prompt() {
    assert_eq!(recorded_pre(&with_pre("abc123", "")), Some("abc123".into()));
    let p = with_pre("abc123", "수정하라\n둘째 줄");
    assert_eq!(recorded_pre(&p), Some("abc123".into()));
    assert!(p.ends_with("둘째 줄"));
    assert_eq!(recorded_pre("티켓 수정"), None);
}

#[test]
fn bootstrap_leftovers_are_not_a_split_violation() {
    let d = repo("split-status");
    // 부트스트랩이 복사한(ignore되지 않은) 파일이 split 전부터 있다.
    write(&d, ".env.local", "A=1\n");
    let before = git_ops::status_porcelain(&d).unwrap();
    assert!(!before.trim().is_empty());
    let after_untouched = git_ops::status_porcelain(&d).unwrap();
    assert!(!tree_changed_during(&before, &after_untouched));
    write(&d, "judge-wrote.txt", "x");
    assert!(tree_changed_during(&before, &git_ops::status_porcelain(&d).unwrap()));
}
