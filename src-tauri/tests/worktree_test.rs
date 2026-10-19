//! worktree 모듈 통합 테스트 — 임시 git 저장소로 검증. `cargo test`

#[path = "support/temp_root.rs"]
mod temp_root;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::worktree::{self, Worktree};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn git(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn run_git_output(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git");
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// 커밋 1개 있는 임시 레포 생성.
fn temp_repo() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = temp_root::dir().join(format!("praxis-wt-test-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.email", "t@t.t"]);
    git(&dir, &["config", "user.name", "t"]);
    git(&dir, &["config", "commit.gpgsign", "false"]);
    std::fs::write(dir.join("main.rs"), "fn main() {}\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "base"]);
    dir
}

#[test]
fn create_makes_isolated_worktree_and_branch() {
    let repo = temp_repo();
    let wt = worktree::create_plain(&repo, "dojang/feat-x", None).expect("create");
    assert!(wt.path.exists(), "worktree dir should exist");
    assert_eq!(wt.path, repo.join(".dojang/worktrees/dojang-feat-x"));
    assert!(!repo.join(".praxis").exists());
    assert_eq!(worktree::current_branch(&wt.path).unwrap(), "dojang/feat-x");
    assert!(run_git_output(&repo, &["status", "--porcelain"]).is_empty());
    assert_eq!(wt.base, "main");
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn an_existing_legacy_worktree_can_still_be_approved_and_removed() {
    let repo = temp_repo();
    let mut wt = worktree::create_plain(&repo, "praxis/legacy", None).unwrap();
    let legacy = repo.join(".praxis/worktrees/praxis-legacy");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    git(&repo, &["worktree", "move", wt.path.to_str().unwrap(), legacy.to_str().unwrap()]);
    wt.path = legacy;
    std::fs::write(wt.path.join("legacy-result.txt"), "preserved\n").unwrap();
    wt.approve().unwrap();
    assert_eq!(std::fs::read_to_string(repo.join("legacy-result.txt")).unwrap(), "preserved\n");
    assert!(!wt.path.exists());
    std::fs::remove_dir_all(repo).ok();
}

#[cfg(unix)]
#[test]
fn create_refuses_a_symlinked_project_storage_root() {
    use std::os::unix::fs::symlink;
    let repo = temp_repo();
    let outside = temp_root::dir().join("worktree-storage-outside");
    std::fs::create_dir_all(&outside).unwrap();
    symlink(&outside, repo.join(".dojang")).unwrap();
    assert!(worktree::create_plain(&repo, "dojang/unsafe-storage", None).is_err());
    assert!(!outside.join("worktrees").exists());
    std::fs::remove_dir_all(repo).unwrap();
    std::fs::remove_dir_all(outside).unwrap();
}

#[test]
fn direct_branch_falls_back_when_directory_is_not_a_git_repository() {
    let dir = temp_root::dir().join(format!("praxis-no-git-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    assert!(!worktree::is_git_repository(&dir));
    assert_eq!(
        worktree::current_branch_or_direct(&dir),
        worktree::DIRECT_BRANCH
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn direct_branch_preserves_git_branch_when_available() {
    let repo = temp_repo();

    assert!(worktree::is_git_repository(&repo));
    assert_eq!(worktree::current_branch_or_direct(&repo), "main");
    assert_eq!(
        worktree::current_revision(&repo).unwrap(),
        run_git_output(&repo, &["rev-parse", "HEAD"]).trim()
    );

    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn diff_stat_shows_changes_including_untracked() {
    let repo = temp_repo();
    let wt = worktree::create_plain(&repo, "praxis/diff", None).expect("create");
    std::fs::write(wt.path.join("main.rs"), "fn main() { let x = 1; }\n").unwrap();
    std::fs::write(wt.path.join("new.txt"), "hello\n").unwrap();
    let stat = wt.diff_stat().expect("diff_stat");
    assert!(stat.contains("main.rs"), "stat: {stat}");
    assert!(stat.contains("new.txt"), "untracked should show: {stat}");
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn approve_merges_changes_into_base_and_cleans_up() {
    let repo = temp_repo();
    let wt = worktree::create_plain(&repo, "praxis/approve", None).expect("create");
    std::fs::write(wt.path.join("added.txt"), "merged content\n").unwrap();
    wt.approve().expect("approve");
    // base에 변경이 머지됨
    assert!(
        repo.join("added.txt").exists(),
        "merged file should be in base"
    );
    // worktree/브랜치 정리됨
    assert!(!wt.path.exists(), "worktree dir should be removed");
    let branches = Command::new("git")
        .current_dir(&repo)
        .args(["branch", "--list", "praxis/approve"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&branches.stdout).trim().is_empty(),
        "branch should be deleted"
    );
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn discard_removes_worktree_without_touching_base() {
    let repo = temp_repo();
    let wt = worktree::create_plain(&repo, "praxis/discard", None).expect("create");
    std::fs::write(wt.path.join("trash.txt"), "discard me\n").unwrap();
    wt.discard().expect("discard");
    assert!(!wt.path.exists(), "worktree removed");
    assert!(!repo.join("trash.txt").exists(), "base untouched");
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn diff_detailed_lists_files_with_status_and_patch() {
    let repo = temp_repo();
    let wt = worktree::create_plain(&repo, "praxis/dd", None).expect("create");
    std::fs::write(wt.path.join("main.rs"), "fn main() { let x = 1; }\n").unwrap(); // 수정
    std::fs::write(wt.path.join("added.txt"), "brand new\n").unwrap(); // 추가
    let diffs = wt.diff_detailed().expect("diff_detailed");

    let main = diffs
        .iter()
        .find(|d| d.path == "main.rs")
        .expect("main.rs present");
    assert_eq!(main.status, "M");
    assert!(main.patch.contains("let x = 1"), "patch: {}", main.patch);

    let added = diffs
        .iter()
        .find(|d| d.path == "added.txt")
        .expect("added.txt present");
    assert_eq!(added.status, "A");
    assert!(added.patch.contains("brand new"));
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn diff_methods_include_staged_changes() {
    let repo = temp_repo();
    let wt = worktree::create_plain(&repo, "praxis/staged-diff", None).expect("create");
    std::fs::write(wt.path.join("main.rs"), "fn staged() {}\n").unwrap();
    git(&wt.path, &["add", "main.rs"]);

    assert!(wt.diff_stat().unwrap().contains("main.rs"));
    assert!(wt.diff_unified(3).unwrap().contains("+fn staged() {}"));
    assert!(wt
        .diff_detailed()
        .unwrap()
        .iter()
        .any(|diff| diff.path == "main.rs"));
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn diff_methods_include_changes_committed_on_the_task_branch() {
    let repo = temp_repo();
    let wt = worktree::create_plain(&repo, "praxis/committed-diff", None).expect("create");
    std::fs::write(wt.path.join("main.rs"), "fn committed() {}\n").unwrap();
    git(&wt.path, &["add", "main.rs"]);
    git(&wt.path, &["commit", "-qm", "agent commit"]);

    assert!(wt.diff_stat().unwrap().contains("main.rs"));
    assert!(wt.diff_unified(3).unwrap().contains("+fn committed() {}"));
    assert!(wt
        .diff_detailed()
        .unwrap()
        .iter()
        .any(|diff| diff.path == "main.rs"));
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn direct_diff_includes_commits_since_the_task_baseline() {
    let repo = temp_repo();
    let baseline = run_git_output(&repo, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let wt = Worktree {
        repo: repo.clone(),
        path: repo.clone(),
        branch: "main".to_string(),
        // 직접 모드가 실제로 만드는 모양 그대로 — base와 기준점이 같은 SHA다.
        base_revision: Some(baseline.clone()),
        base: baseline,
    };
    std::fs::write(repo.join("main.rs"), "fn committed_directly() {}\n").unwrap();
    git(&repo, &["add", "main.rs"]);
    git(&repo, &["commit", "-qm", "direct agent commit"]);

    assert!(wt.diff_stat().unwrap().contains("main.rs"));
    assert!(wt
        .diff_unified(3)
        .unwrap()
        .contains("+fn committed_directly() {}"));
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn reading_diff_does_not_mutate_the_worktree_index() {
    let repo = temp_repo();
    let wt = worktree::create_plain(&repo, "praxis/read-only-diff", None).expect("create");
    std::fs::write(wt.path.join("untracked.txt"), "new\n").unwrap();
    let before = run_git_output(&wt.path, &["status", "--porcelain"]);

    assert!(wt
        .diff_detailed()
        .unwrap()
        .iter()
        .any(|diff| diff.path == "untracked.txt"));
    assert_eq!(run_git_output(&wt.path, &["status", "--porcelain"]), before);
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn diff_unified_returns_single_multi_file_diff_text() {
    let repo = temp_repo();
    let wt = worktree::create_plain(&repo, "praxis/du", None).expect("create");
    std::fs::write(wt.path.join("main.rs"), "fn main() { let x = 1; }\n").unwrap(); // 수정
    std::fs::write(wt.path.join("added.txt"), "brand new\n").unwrap(); // 추가

    let diff = wt.diff_unified(3).expect("diff_unified");

    assert!(
        diff.contains("diff --git a/main.rs b/main.rs"),
        "diff: {diff}"
    );
    assert!(
        diff.contains("diff --git a/added.txt b/added.txt"),
        "diff: {diff}"
    );
    assert!(diff.contains("+brand new"));
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn changed_paths_include_both_sides_of_a_rename() {
    let repo = temp_repo();
    let wt = worktree::create_plain(&repo, "praxis/rename-paths", None).expect("create");
    git(&wt.path, &["mv", "main.rs", "renamed.rs"]);

    let paths = wt.changed_paths().expect("changed paths");

    assert!(paths.contains(&"main.rs".to_string()), "paths: {paths:?}");
    assert!(
        paths.contains(&"renamed.rs".to_string()),
        "paths: {paths:?}"
    );
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn changed_paths_include_changes_committed_on_the_task_branch() {
    let repo = temp_repo();
    let wt = worktree::create_plain(&repo, "praxis/committed-paths", None).expect("create");
    std::fs::write(wt.path.join("main.rs"), "fn committed() {}\n").unwrap();
    git(&wt.path, &["add", "main.rs"]);
    git(&wt.path, &["commit", "-qm", "agent commit"]);

    assert_eq!(wt.changed_paths().unwrap(), ["main.rs"]);
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn slugify_normalizes() {
    assert_eq!(
        worktree::slugify("Add User Auth Flow!"),
        "add-user-auth-flow"
    );
    assert_eq!(worktree::slugify("  trailing  "), "trailing");
    assert_eq!(worktree::slugify(""), "task");
}

#[allow(dead_code)]
fn _assert_send(_: &Worktree) {}

/// 문서 충돌은 이제 **파일 구조**가 막는다 (ADR 0148).
///
/// 전에는 `docs/INDEX.md` 한 표에 두 워크트리가 각자 행을 더했고, union 속성과 3-way union
/// fallback이 그것을 자동 해소했다. 그 두 장치는 사라졌다 — 원천이 `docs/index/`의 기능당
/// 파일이 되어 서로 다른 브랜치가 서로 다른 파일을 만들기 때문이다.
///
/// 이 테스트가 지키는 것은 그 계약이다: **같은 base에서 갈라진 두 워크트리가 각자 다른 색인
/// 파일을 더하고 순차 approve해도 충돌 없이 둘 다 남는다.**
#[test]
fn approve_merges_separate_index_files_without_conflict() {
    let repo = temp_repo();
    std::fs::create_dir_all(repo.join("docs/index")).unwrap();
    std::fs::write(repo.join("docs/index/0001-base.md"), "# base\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "docs base"]);

    let wt1 = worktree::create_plain(&repo, "praxis/idx-1", None).expect("create wt1");
    let wt2 = worktree::create_plain(&repo, "praxis/idx-2", None).expect("create wt2");

    std::fs::write(wt1.path.join("docs/index/0002-one.md"), "# one\n").unwrap();
    std::fs::write(wt2.path.join("docs/index/0003-two.md"), "# two\n").unwrap();

    wt1.approve().expect("first approve should succeed");
    // 머지 드라이버도, fallback도 없다. 그런데도 성공해야 한다 — 서로 다른 파일이기 때문이다.
    wt2.approve().expect("second approve should not conflict");

    for f in ["0001-base.md", "0002-one.md", "0003-two.md"] {
        assert!(
            repo.join("docs/index").join(f).exists(),
            "{f}이 머지 후 사라졌다"
        );
    }
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn approve_still_fails_on_non_docs_index_conflict() {
    let repo = temp_repo();
    let wt1 = worktree::create_plain(&repo, "praxis/code-1", None).expect("create wt1");
    let wt2 = worktree::create_plain(&repo, "praxis/code-2", None).expect("create wt2");

    std::fs::write(wt1.path.join("main.rs"), "fn main() { let x = 1; }\n").unwrap();
    std::fs::write(wt2.path.join("main.rs"), "fn main() { let x = 2; }\n").unwrap();

    wt1.approve().expect("first approve should succeed");
    let result = wt2.approve();
    assert!(result.is_err(), "conflicting code file merge should fail");

    // 메인 체크아웃이 머지 중간 상태로 남지 않아야 함(충돌/미해결 파일 없음).
    // 테스트 픽스처는 .gitignore가 없어 worktree용 .praxis/ 디렉터리가 "??"로
    // 잡히므로, 완전한 clean 대신 미해결(U) 상태 부재로 검증한다.
    let status = Command::new("git")
        .current_dir(&repo)
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    let status = String::from_utf8_lossy(&status.stdout);
    assert!(
        !status
            .lines()
            .any(|l| l.starts_with('U') || l.contains("UU")),
        "no unresolved conflict files should remain: {status}"
    );
    assert!(
        !repo.join(".git/MERGE_HEAD").exists(),
        "merge should be aborted"
    );

    // worktree는 보존됨(재시도/폐기 가능하도록).
    assert!(
        wt2.path.exists(),
        "worktree should be preserved after failed approve"
    );
    let _ = wt2.discard();
    std::fs::remove_dir_all(&repo).ok();
}

/// 재발 방지: projector가 worktree 루트에 만드는 하네스 컨텍스트 파일(CLAUDE.md 등)은
/// 커밋에서 제외되어야 하고, base에 같은 파일이 미추적으로 있어도 머지가
/// "untracked working tree files would be overwritten"으로 실패하지 않아야 한다.
#[test]
fn approve_excludes_harness_context_files_from_commit_and_merge() {
    let repo = temp_repo();
    // base(메인 체크아웃)에 미추적 컨텍스트 파일 — 과거 머지 실패를 유발한 상황 재현.
    std::fs::write(repo.join("CLAUDE.md"), "base untracked\n").unwrap();

    let wt = worktree::create_plain(&repo, "praxis/ctx", None).expect("create");
    // projector 주입을 흉내: worktree 루트에 컨텍스트 파일 3종 + 실제 산출물 1개.
    std::fs::write(wt.path.join("CLAUDE.md"), "injected\n").unwrap();
    std::fs::write(wt.path.join("AGENTS.md"), "injected\n").unwrap();
    std::fs::write(wt.path.join("GEMINI.md"), "injected\n").unwrap();
    std::fs::write(wt.path.join("real.txt"), "real output\n").unwrap();

    // 변경 요약(diff)에도 컨텍스트 파일은 노이즈로 잡히지 않아야 한다.
    let stat = wt.diff_stat().expect("diff_stat");
    assert!(stat.contains("real.txt"), "stat: {stat}");
    assert!(!stat.contains("CLAUDE.md"), "stat: {stat}");

    wt.approve()
        .expect("approve should merge without untracked-overwrite error");

    // 산출물은 머지되고, 컨텍스트 파일은 커밋/머지되지 않는다.
    assert!(
        repo.join("real.txt").exists(),
        "real output should be merged"
    );
    let tracked = Command::new("git")
        .current_dir(&repo)
        .args(["ls-files"])
        .output()
        .unwrap();
    let tracked = String::from_utf8_lossy(&tracked.stdout);
    for ctx in ["CLAUDE.md", "AGENTS.md", "GEMINI.md"] {
        assert!(
            !tracked.contains(ctx),
            "{ctx} must not be tracked: {tracked}"
        );
    }
    // base의 미추적 파일은 그대로 보존.
    assert_eq!(
        std::fs::read_to_string(repo.join("CLAUDE.md")).unwrap(),
        "base untracked\n"
    );
    std::fs::remove_dir_all(&repo).ok();
}

/// 이미 tracked인 컨텍스트 파일(사용자가 커밋해 쓰는 레포)은 exclude의 영향을 받지 않고
/// worktree에서의 수정이 정상 머지되어야 한다.
#[test]
fn approve_still_merges_tracked_context_file_changes() {
    let repo = temp_repo();
    std::fs::write(repo.join("CLAUDE.md"), "user rules v1\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "user CLAUDE.md"]);

    let wt = worktree::create_plain(&repo, "praxis/ctx-tracked", None).expect("create");
    std::fs::write(wt.path.join("CLAUDE.md"), "user rules v2\n").unwrap();
    wt.approve().expect("approve");
    assert_eq!(
        std::fs::read_to_string(repo.join("CLAUDE.md")).unwrap(),
        "user rules v2\n"
    );
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn approve_excludes_generated_mcp_config_only() {
    let repo = temp_repo();
    let wt = worktree::create_plain(&repo, "praxis/generated-mcp", None).expect("create");
    std::fs::write(wt.path.join(".mcp.json"), "{\"mcpServers\":{}}\n").unwrap();
    std::fs::write(wt.path.join("real.txt"), "real output\n").unwrap();

    wt.approve_with_generated_mcp_excluded()
        .expect("approve generated MCP exclusion");

    assert!(repo.join("real.txt").exists(), "real output must merge");
    assert!(
        !repo.join(".mcp.json").exists(),
        "generated MCP config must not merge"
    );
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn approve_succeeds_when_generated_mcp_config_is_the_only_change() {
    let repo = temp_repo();
    let wt = worktree::create_plain(&repo, "praxis/generated-mcp-only", None).expect("create");
    std::fs::write(wt.path.join(".mcp.json"), "{\"mcpServers\":{}}\n").unwrap();

    wt.approve_with_generated_mcp_excluded()
        .expect("generated MCP config alone must not make approve fail");

    assert!(
        !repo.join(".mcp.json").exists(),
        "generated MCP config must not merge"
    );
    assert!(!wt.path.exists(), "worktree should still be cleaned up");
    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn untracked_lists_new_excludes_ignored() {
    let repo = temp_repo();
    let wt = worktree::create_plain(&repo, "praxis/untracked-x", None).unwrap();
    std::fs::write(wt.path.join("new.txt"), "x").unwrap();
    std::fs::write(wt.path.join(".gitignore"), "ignored.txt\n").unwrap();
    std::fs::write(wt.path.join("ignored.txt"), "y").unwrap();
    let u = wt.untracked().unwrap();
    assert!(
        u.iter().any(|p| p.contains("new.txt")),
        "new.txt should be listed"
    );
    assert!(
        !u.iter().any(|p| p.contains("ignored.txt")),
        ".gitignore excludes ignored.txt"
    );
    let _ = wt.discard();
}
