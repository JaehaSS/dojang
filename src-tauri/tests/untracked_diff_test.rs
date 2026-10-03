//! 세션 diff의 신규 파일 회귀 테스트.

#[path = "support/temp_root.rs"]
mod temp_root;

use std::path::Path;
use std::process::Command;

use praxis_lib::diffmodel;
use praxis_lib::worktree;

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn session_diff_connects_an_untracked_unicode_file_to_its_hunk() {
    let repo = temp_root::dir().join(format!("praxis-untracked-diff-{}", std::process::id()));
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "test@example.test"]);
    git(&repo, &["config", "user.name", "Praxis Test"]);
    std::fs::write(repo.join("seed.txt"), "seed\n").unwrap();
    git(&repo, &["add", "seed.txt"]);
    git(&repo, &["commit", "-qm", "seed"]);

    let worktree = worktree::create_plain(&repo, "praxis/untracked-unicode", None).unwrap();
    let path = "문서/신규 파일.md";
    std::fs::create_dir_all(worktree.path.join("문서")).unwrap();
    std::fs::write(worktree.path.join(path), "새 내용\n").unwrap();

    let files = worktree.diff_detailed().unwrap();
    assert!(files.iter().any(|file| file.path == path));

    let unified = worktree.diff_unified(3).unwrap();
    let hunks = diffmodel::build_hunks(&unified, &[]);
    assert!(
        hunks.iter().any(|hunk| hunk.path == path),
        "신규 파일의 실제 경로로 hunk가 연결되어야 한다: {hunks:?}"
    );

    git(&worktree.path, &["add", path]);
    let staged_hunks = diffmodel::build_hunks(&worktree.diff_unified(3).unwrap(), &[]);
    assert!(
        staged_hunks.iter().any(|hunk| hunk.path == path),
        "stage 후에도 신규 파일의 실제 경로가 유지되어야 한다: {staged_hunks:?}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

fn fixture(label: &str) -> (std::path::PathBuf, worktree::Worktree) {
    let repo = temp_root::dir().join(format!("praxis-diff-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "test@example.test"]);
    git(&repo, &["config", "user.name", "Praxis Test"]);
    std::fs::write(repo.join("seed.txt"), "seed\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "seed"]);
    let wt = worktree::create_plain(&repo, "praxis/bounded-diff", None).unwrap();
    (repo, wt)
}

#[test]
fn large_and_binary_files_stay_listed_without_text_hunks() {
    let (repo, wt) = fixture("limits");
    // A real failure case: PDF has no NUL in Git's initial binary probe.
    std::fs::write(
        wt.path.join("document.pdf"),
        b"%PDF-1.7\nplain ASCII PDF body\n",
    )
    .unwrap();
    std::fs::write(wt.path.join("large.pdf"), vec![b'x'; 36_330_000]).unwrap();
    std::fs::write(wt.path.join("binary.dat"), b"a\0b").unwrap();
    std::fs::write(wt.path.join("normal.txt"), "normal text\n").unwrap();
    for staged in [false, true] {
        if staged {
            git(&wt.path, &["add", "."]);
        }
        let files = wt.diff_detailed().unwrap();
        assert_eq!(files.len(), 4);
        assert!(files.iter().all(|file| file.patch.len() < 1024));
        assert!(files
            .iter()
            .find(|f| f.path == "large.pdf")
            .unwrap()
            .patch
            .contains("Praxis diff omitted"));
        assert!(files
            .iter()
            .find(|f| f.path == "document.pdf")
            .unwrap()
            .patch
            .contains("Binary files"));
        let review = wt
            .review_snapshot(&files, worktree::DiffRange::Session, &[], &[])
            .unwrap();
        assert_eq!(review.hunks.len(), 1);
        assert_eq!(review.hunks[0].path, "normal.txt");
        assert!(!review.hunks[0].committed);
    }
    std::fs::remove_dir_all(repo).ok();
}

#[test]
fn deleting_large_tracked_text_caps_git_output() {
    let (repo, wt) = fixture("deleted");
    std::fs::write(wt.path.join("large.txt"), "line\n".repeat(300_000)).unwrap();
    git(&wt.path, &["add", "."]);
    git(&wt.path, &["commit", "-qm", "large"]);
    std::fs::remove_file(wt.path.join("large.txt")).unwrap();
    let files = wt
        .diff_detailed_range(worktree::DiffRange::Uncommitted)
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].status, "D");
    assert!(files[0].patch.contains("Praxis diff omitted"));
    assert!(files[0].patch.len() < 1024);
    std::fs::remove_dir_all(repo).ok();
}

#[test]
fn shared_review_keeps_committed_and_pending_hunk_identity() {
    let (repo, wt) = fixture("snapshot");
    std::fs::write(wt.path.join("committed.txt"), "committed\n").unwrap();
    git(&wt.path, &["add", "."]);
    git(&wt.path, &["commit", "-qm", "change"]);
    std::fs::write(wt.path.join("pending.txt"), "pending\n").unwrap();
    let files = wt.diff_detailed().unwrap();
    let review = wt
        .review_snapshot(&files, worktree::DiffRange::Session, &[], &[])
        .unwrap();
    assert_eq!(review.hunks.len(), 2);
    assert!(
        review
            .hunks
            .iter()
            .find(|h| h.path == "committed.txt")
            .unwrap()
            .committed
    );
    assert!(
        !review
            .hunks
            .iter()
            .find(|h| h.path == "pending.txt")
            .unwrap()
            .committed
    );
    let expected = diffmodel::build_hunks(&wt.diff_unified(3).unwrap(), &[]);
    assert_eq!(
        review.hunks.iter().map(|h| &h.id).collect::<Vec<_>>(),
        expected.iter().map(|h| &h.id).collect::<Vec<_>>()
    );
    std::fs::remove_dir_all(repo).ok();
}
