use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use super::*;
use crate::worktree::{DirectCheckoutLocks, Resolution};

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Fixture {
    repo: PathBuf,
    worktree: Worktree,
    commit: String,
}

fn fixture(label: &str) -> Fixture {
    let serial = COUNTER.fetch_add(1, Ordering::SeqCst);
    let repo = crate::testtmp::dir().join(format!(
        "praxis-approval-merge-{label}-{}-{serial}",
        std::process::id()
    ));
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(
        &repo,
        &["config", "user.email", "approval-merge@example.test"],
    );
    git(&repo, &["config", "user.name", "Approval Merge Test"]);
    std::fs::write(repo.join("README.md"), "before\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-qm", "initial"]);
    git(&repo, &["checkout", "-qb", "task.base"]);
    let worktree = super::super::create_plain(
        &repo,
        &format!("praxis/{label}-{serial}"),
        Some("task.base"),
    )
    .unwrap();
    std::fs::write(worktree.path.join("result.txt"), "approved\n").unwrap();
    let commit = worktree.commit_for_approval().unwrap();
    git(&repo, &["checkout", "-qb", "feature"]);
    Fixture {
        repo,
        worktree,
        commit,
    }
}

fn git(root: &Path, args: &[&str]) -> String {
    super::run_git(root, args).unwrap_or_else(|error| panic!("git {args:?} failed: {error}"))
}

fn temp_path(fixture: &Fixture) -> PathBuf {
    temporary_path(&fixture.worktree, false).unwrap()
}

fn temp_checkout(fixture: &Fixture) -> PathBuf {
    let base = base_ref(&fixture.worktree).unwrap();
    match checkout(&fixture.worktree, &base).unwrap() {
        Checkout::Temporary(path) => path,
        Checkout::Repository => panic!("fixture must force an approval temporary worktree"),
    }
}

fn clean(fixture: Fixture) {
    let _ = std::fs::remove_dir_all(fixture.repo);
}

#[test]
fn merge_resumes_a_clean_registered_temp_residual_and_removes_it() {
    let fixture = fixture("resume-clean");
    let temp = temp_checkout(&fixture);

    fixture
        .worktree
        .merge_for_approval(&fixture.commit)
        .unwrap();

    assert!(!temp.exists());
    assert_eq!(
        git(&fixture.repo, &["show", "task.base:result.txt"]),
        "approved\n"
    );
    clean(fixture);
}

#[test]
fn merge_resumes_a_legacy_temporary_worktree_without_creating_another() {
    let fixture = fixture("resume-legacy");
    let current = temp_checkout(&fixture);
    assert!(current.starts_with(fixture.repo.canonicalize().unwrap().join(".dojang/approval")));
    let legacy = fixture.repo.join(".praxis/approval").join(current.file_name().unwrap());
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    git(&fixture.repo, &["worktree", "move", current.to_str().unwrap(), legacy.to_str().unwrap()]);
    fixture.worktree.merge_for_approval(&fixture.commit).unwrap();
    assert!(!legacy.exists());
    assert!(!current.exists());
    assert_eq!(git(&fixture.repo, &["show", "task.base:result.txt"]), "approved\n");
    clean(fixture);
}

#[test]
fn merge_preserves_a_detached_original_head_and_updates_saved_base() {
    let fixture = fixture("detached-original");
    git(&fixture.repo, &["checkout", "--detach"]);
    let original_head = git(&fixture.repo, &["rev-parse", "HEAD"]);

    fixture
        .worktree
        .merge_for_approval(&fixture.commit)
        .unwrap();

    assert_eq!(git(&fixture.repo, &["rev-parse", "HEAD"]), original_head);
    assert!(super::run_git(&fixture.repo, &["symbolic-ref", "--quiet", "HEAD"]).is_err());
    assert_eq!(
        git(&fixture.repo, &["show", "task.base:result.txt"]),
        "approved\n"
    );
    clean(fixture);
}

#[test]
fn merged_retry_and_finalization_cleanup_remove_temp_residuals() {
    let fixture = fixture("merged-cleanup");
    let first = temp_checkout(&fixture);
    merge_into(&fixture.worktree, &first, &fixture.commit).unwrap();

    fixture
        .worktree
        .merge_for_approval(&fixture.commit)
        .unwrap();
    assert!(!first.exists());

    let second = temp_checkout(&fixture);
    fixture.worktree.cleanup_after_finalization().unwrap();
    assert!(!second.exists());
    assert!(!fixture.worktree.path.exists());
    clean(fixture);
}

#[test]
fn cleanup_preserves_dirty_and_switched_temp_worktrees() {
    let dirty = fixture("dirty-temp");
    let dirty_temp = temp_checkout(&dirty);
    let dirty_file = dirty_temp.join("later.txt");
    std::fs::write(&dirty_file, "must survive\n").unwrap();

    assert!(dirty.worktree.cleanup_after_finalization().is_err());
    assert!(dirty.worktree.path.exists());
    assert_eq!(
        std::fs::read_to_string(dirty_file).unwrap(),
        "must survive\n"
    );
    clean(dirty);

    let switched = fixture("switched-temp");
    let switched_temp = temp_checkout(&switched);
    git(&switched_temp, &["checkout", "-qb", "unrelated"]);
    let marker = switched_temp.join("identity.txt");
    std::fs::write(&marker, "must survive\n").unwrap();

    assert!(switched.worktree.cleanup_after_finalization().is_err());
    assert!(switched.worktree.path.exists());
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "must survive\n");
    clean(switched);
}

#[test]
fn cleanup_rejects_missing_registered_and_unregistered_temp_residues() {
    let missing = fixture("missing-temp");
    let missing_temp = temp_checkout(&missing);
    std::fs::remove_dir_all(&missing_temp).unwrap();

    assert!(missing.worktree.cleanup_after_finalization().is_err());
    assert!(missing.worktree.path.exists());
    clean(missing);

    let residue = fixture("unregistered-temp");
    let residue_temp = temp_path(&residue);
    std::fs::create_dir_all(&residue_temp).unwrap();
    let sentinel = residue_temp.join("sentinel.txt");
    std::fs::write(&sentinel, "preserve\n").unwrap();

    assert!(residue.worktree.cleanup_after_finalization().is_err());
    assert!(residue.worktree.path.exists());
    assert_eq!(std::fs::read_to_string(sentinel).unwrap(), "preserve\n");
    clean(residue);
}

#[test]
fn failed_temp_conflict_is_removed_and_resolved_reverse_merge_can_finish() {
    let fixture = fixture("conflict");
    std::fs::write(fixture.worktree.path.join("README.md"), "task change\n").unwrap();
    let task_commit = fixture.worktree.commit_for_approval().unwrap();
    git(&fixture.repo, &["checkout", "task.base"]);
    std::fs::write(fixture.repo.join("README.md"), "base change\n").unwrap();
    git(&fixture.repo, &["add", "README.md"]);
    git(&fixture.repo, &["commit", "-qm", "base change"]);
    git(&fixture.repo, &["checkout", "feature"]);
    let temp = temp_path(&fixture);

    assert!(fixture.worktree.merge_for_approval(&task_commit).is_err());
    assert!(!temp.exists());

    fixture.worktree.begin_conflict_resolution().unwrap();
    fixture
        .worktree
        .resolve_conflict("README.md", &Resolution::Ours)
        .unwrap();
    let resolved = fixture.worktree.finish_conflict_resolution().unwrap();
    fixture.worktree.merge_for_approval(&resolved).unwrap();

    assert_eq!(
        git(&fixture.repo, &["show", "task.base:README.md"]),
        "task change\n"
    );
    clean(fixture);
}

#[test]
fn merge_preserves_a_clean_existing_sequencer_in_the_base_checkout() {
    let fixture = fixture("sequencer");
    git(&fixture.repo, &["checkout", "task.base"]);
    let sequencer = fixture.repo.join(".git").join("sequencer");
    std::fs::create_dir_all(&sequencer).unwrap();
    let marker = sequencer.join("todo");
    std::fs::write(&marker, "pick deadbeef\n").unwrap();

    assert!(fixture
        .worktree
        .merge_for_approval(&fixture.commit)
        .is_err());
    assert!(fixture.worktree.path.exists());
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "pick deadbeef\n");
    clean(fixture);
}

#[cfg(unix)]
#[test]
fn post_checkout_hook_dirty_temp_and_merge_state_are_preserved_on_rejection() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = fixture("hook-state");
    let hooks = fixture.repo.join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let hook = hooks.join("post-checkout");
    std::fs::write(
        &hook,
        "#!/bin/sh\nprintf 'dirty\\n' > dirty-from-hook.txt\ngit_dir=$(git rev-parse --absolute-git-dir)\nprintf 'open\\n' > \"$git_dir/MERGE_HEAD\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    git(
        &fixture.repo,
        &["config", "core.hooksPath", hooks.to_str().unwrap()],
    );
    let temp = temp_path(&fixture);

    assert!(fixture
        .worktree
        .merge_for_approval(&fixture.commit)
        .is_err());
    assert!(fixture.worktree.path.exists());
    assert_eq!(
        std::fs::read_to_string(temp.join("dirty-from-hook.txt")).unwrap(),
        "dirty\n"
    );
    assert!(git_dir(&temp).unwrap().join("MERGE_HEAD").exists());
    clean(fixture);
}

#[tokio::test]
async fn approval_and_direct_checkout_locks_contend_across_task_identities() {
    let fixture = fixture("locks");
    let alternate = Worktree {
        branch: "praxis/other-task".to_string(),
        ..fixture.worktree.clone()
    };
    let locks = DirectCheckoutLocks::default();
    let direct = locks.acquire(&fixture.repo).await.unwrap();

    assert!(approval_lock(&alternate).is_err());
    drop(direct);

    let approval = approval_lock(&fixture.worktree).unwrap();
    assert!(approval_lock(&alternate).is_err());
    drop(approval);
    assert!(approval_lock(&alternate).is_ok());
    clean(fixture);
}

#[tokio::test]
async fn existing_legacy_branch_lock_blocks_new_approval_and_checkout() {
    let fixture = fixture("legacy-lock");
    let common = canonical_git(&fixture.repo, git(&fixture.repo, &["rev-parse", "--git-common-dir"]).trim()).unwrap();
    let legacy = OpenOptions::new().create(true).truncate(false).read(true).write(true)
        .open(common.join("praxis-direct-branch.lock")).unwrap();
    legacy.lock_exclusive().unwrap();
    assert!(approval_lock(&fixture.worktree).is_err());
    let locks = DirectCheckoutLocks::default();
    let repo = fixture.repo.clone();
    let checkout = tokio::spawn(async move { locks.acquire(&repo).await });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(!checkout.is_finished());
    drop(legacy);
    drop(checkout.await.unwrap().unwrap());
    assert!(approval_lock(&fixture.worktree).is_ok());
    clean(fixture);
}

#[test]
fn new_approval_also_locks_the_legacy_git_metadata_name() {
    let fixture = fixture("new-lock-first");
    let common = canonical_git(&fixture.repo, git(&fixture.repo, &["rev-parse", "--git-common-dir"]).trim()).unwrap();
    assert!(!common.join("praxis-direct-branch.lock").exists());
    let guard = approval_lock(&fixture.worktree).unwrap();
    let legacy = OpenOptions::new().read(true).write(true)
        .open(common.join("praxis-direct-branch.lock")).unwrap();
    assert!(legacy.try_lock_exclusive().is_err());
    drop(guard);
    assert!(legacy.try_lock_exclusive().is_ok());
    drop(legacy);
    assert!(!fixture.repo.join(".praxis").exists());
    clean(fixture);
}

#[test]
fn retiring_clean_temp_releases_it_but_dirty_temp_preserves_the_task() {
    let clean_residue = fixture("retire-clean");
    let clean_temp = temp_checkout(&clean_residue);

    clean_residue.worktree.retire_preserving_branch().unwrap();
    assert!(!clean_temp.exists());
    assert!(!clean_residue.worktree.path.exists());
    clean(clean_residue);

    let dirty_residue = fixture("retire-dirty");
    let dirty_temp = temp_checkout(&dirty_residue);
    let evidence = dirty_temp.join("evidence.txt");
    std::fs::write(&evidence, "preserve\n").unwrap();

    assert!(dirty_residue.worktree.retire_preserving_branch().is_err());
    assert!(dirty_residue.worktree.path.exists());
    assert_eq!(std::fs::read_to_string(evidence).unwrap(), "preserve\n");
    clean(dirty_residue);
}

#[cfg(unix)]
#[test]
fn managed_parent_and_temp_git_symlinks_cannot_escape_the_repository() {
    use std::os::unix::fs::symlink;

    let parent = fixture("parent-symlink");
    let outside = crate::testtmp::dir().join(format!(
        "praxis-approval-outside-{}",
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&outside).unwrap();
    let sentinel = outside.join("sentinel.txt");
    std::fs::write(&sentinel, "outside\n").unwrap();
    let approval = parent.repo.join(".dojang").join("approval");
    symlink(&outside, &approval).unwrap();

    assert!(parent.worktree.merge_for_approval(&parent.commit).is_err());
    assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "outside\n");
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
    clean(parent);
    let _ = std::fs::remove_dir_all(&outside);

    let git_link = fixture("git-symlink");
    let temp = temp_checkout(&git_link);
    let outside_git = crate::testtmp::dir().join(format!(
        "praxis-approval-git-outside-{}",
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&outside_git).unwrap();
    let git_sentinel = outside_git.join("sentinel.txt");
    std::fs::write(&git_sentinel, "outside\n").unwrap();
    let pointer = temp.join(".git");
    std::fs::remove_file(&pointer).unwrap();
    symlink(&git_sentinel, &pointer).unwrap();

    assert!(git_link.worktree.cleanup_after_finalization().is_err());
    assert!(git_link.worktree.path.exists());
    assert_eq!(std::fs::read_to_string(&git_sentinel).unwrap(), "outside\n");
    clean(git_link);
    let _ = std::fs::remove_dir_all(&outside_git);
}
