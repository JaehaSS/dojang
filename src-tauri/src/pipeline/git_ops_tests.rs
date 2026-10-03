use super::*;
use std::fs;

fn sh(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git").current_dir(dir).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn repo(tag: &str) -> PathBuf {
    let dir = crate::testtmp::dir().join(format!("pipe-git-{tag}-{:x}", crate::pipeline::review_run::random_u64()));
    fs::create_dir_all(&dir).unwrap();
    sh(&dir, &["init", "-q", "-b", "main"]);
    sh(&dir, &["config", "user.name", "t"]);
    sh(&dir, &["config", "user.email", "t@t"]);
    fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    fs::write(dir.join(".gitignore"), "ignored.log\n").unwrap();
    sh(&dir, &["add", "-A"]);
    sh(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

#[test]
fn changed_paths_covers_committed_staged_unstaged_untracked() {
    let d = repo("changed");
    let fork = head(&d).unwrap();
    fs::write(d.join("c.txt"), "c").unwrap();
    sh(&d, &["add", "c.txt"]);
    sh(&d, &["commit", "-q", "-m", "c"]);
    fs::write(d.join("s.txt"), "s").unwrap();
    sh(&d, &["add", "s.txt"]);
    fs::write(d.join("a.txt"), "changed\n").unwrap();
    fs::create_dir_all(d.join("sub")).unwrap();
    fs::write(d.join("sub/u.txt"), "u").unwrap();
    fs::write(d.join("ignored.log"), "x").unwrap();
    let got = changed_paths(&d, &fork).unwrap();
    assert_eq!(got, vec!["a.txt", "c.txt", "s.txt", "sub/u.txt"]);
}

#[test]
fn diff_text_includes_untracked_and_keeps_index() {
    let d = repo("diff");
    let fork = head(&d).unwrap();
    fs::write(d.join("new.txt"), "hello-untracked\n").unwrap();
    fs::write(d.join("a.txt"), "one\nTWO\nthree\n").unwrap();
    let before = sh(&d, &["status", "--porcelain"]);
    let diff = diff_text(&d, &fork).unwrap();
    assert!(diff.contains("+hello-untracked"));
    assert!(diff.contains("+TWO"));
    assert_eq!(sh(&d, &["status", "--porcelain"]), before);
}

#[test]
fn commit_all_noop_returns_none_then_commits() {
    let d = repo("commit");
    assert_eq!(commit_all(&d, "x").unwrap(), None);
    fs::write(d.join("n.txt"), "n").unwrap();
    let rev = commit_all(&d, "add n").unwrap().unwrap();
    assert_eq!(rev, head(&d).unwrap());
    assert_eq!(sh(&d, &["status", "--porcelain"]).trim(), "");
}

fn branch_with(d: &Path, name: &str, file: &str, body: &str) {
    sh(d, &["checkout", "-q", "-b", name, "main"]);
    fs::write(d.join(file), body).unwrap();
    sh(d, &["add", "-A"]);
    sh(d, &["commit", "-q", "-m", name]);
    sh(d, &["checkout", "-q", "main"]);
}

#[test]
fn merge_success_and_nothing_to_merge() {
    let d = repo("merge-ok");
    branch_with(&d, "t1", "t1.txt", "t1\n");
    match merge_branch(&d, "t1", "merge t1").unwrap() {
        MergeOutcome::Merged { rev } => assert_eq!(rev, head(&d).unwrap()),
        o => panic!("{o:?}"),
    }
    assert_eq!(merge_branch(&d, "t1", "again").unwrap(), MergeOutcome::NothingToMerge);
}

#[test]
fn merge_conflict_then_finish_requires_resolution() {
    let d = repo("merge-conflict");
    branch_with(&d, "t1", "a.txt", "one\nT1\nthree\n");
    branch_with(&d, "t2", "a.txt", "one\nT2\nthree\n");
    assert!(matches!(merge_branch(&d, "t1", "m1").unwrap(), MergeOutcome::Merged { .. }));
    let pre = head(&d).unwrap();
    match merge_branch(&d, "t2", "m2").unwrap() {
        MergeOutcome::Conflict { paths } => assert_eq!(paths, vec!["a.txt"]),
        o => panic!("{o:?}"),
    }
    // 마커가 남은 채로는 마무리 불가
    let err = finish_merge_after_resolution(&d).unwrap_err().to_string();
    assert!(err.contains("충돌 마커"), "{err}");
    // add 후에도(마커가 커밋되는 것을 막는다)
    sh(&d, &["add", "a.txt"]);
    assert!(finish_merge_after_resolution(&d).is_err());
    fs::write(d.join("a.txt"), "one\nT1+T2\nthree\n").unwrap();
    let rev = finish_merge_after_resolution(&d).unwrap();
    assert_ne!(rev, pre);
    assert_eq!(sh(&d, &["rev-list", "--parents", "-n1", "HEAD"]).split_whitespace().count(), 3);
    assert_eq!(fs::read_to_string(d.join("a.txt")).unwrap(), "one\nT1+T2\nthree\n");
}

#[test]
fn reset_to_restores_after_conflict_and_cleans_untracked_keeps_ignored() {
    let d = repo("reset");
    branch_with(&d, "t1", "a.txt", "one\nT1\nthree\n");
    branch_with(&d, "t2", "a.txt", "one\nT2\nthree\n");
    merge_branch(&d, "t1", "m1").unwrap();
    let pre = head(&d).unwrap();
    assert!(matches!(merge_branch(&d, "t2", "m2").unwrap(), MergeOutcome::Conflict { .. }));
    fs::write(d.join("stray.txt"), "s").unwrap();
    fs::write(d.join("ignored.log"), "keep").unwrap();
    reset_to(&d, &pre).unwrap();
    assert_eq!(head(&d).unwrap(), pre);
    assert!(!d.join("stray.txt").exists());
    assert!(d.join("ignored.log").exists());
    assert_eq!(sh(&d, &["status", "--porcelain"]).trim(), "");
    assert!(finish_merge_after_resolution(&d).is_err());
}
