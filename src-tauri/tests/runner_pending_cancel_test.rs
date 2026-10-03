#[path = "support/temp_root.rs"]
mod temp_root;

use praxis_lib::runner::config::{ExecutionPolicy, RunnerConfig};
use praxis_lib::runner::{self, QueuedTaskRequest};
use praxis_lib::{db, memory};

#[tokio::test]
async fn pending_cancel_uses_the_shared_worktree_lock_and_discards_once() {
    let repo = temp_root::dir().join(format!("praxis-pending-cancel-{}", std::process::id()));
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "runner@example.test"]);
    git(&repo, &["config", "user.name", "Runner Test"]);
    std::fs::write(repo.join("README.md"), "base\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-qm", "initial"]);
    let db_path = repo.with_extension("sqlite");
    let pool = db::init_pool(db_path.to_str().unwrap()).await.unwrap();
    memory::migrate(&pool).await.unwrap();
    let config = RunnerConfig {
        bind: "127.0.0.1:47831".parse().unwrap(),
        repository_roots: vec![repo.canonicalize().unwrap()],
        max_concurrent_tasks: 1,
        execution_policy: ExecutionPolicy::RequireApproval,
        pairing_token_file: temp_root::dir().join("unused-runner-token"),
    };
    let locks = runner::worktree_lock::WorktreeLocks::default();
    let task = runner::create_queued_task(
        &config,
        &pool,
        &locks,
        QueuedTaskRequest {
            repository: repo.to_string_lossy().into_owned(),
            instruction: "cancel safely".to_string(),
            agent: "claude".to_string(),
            role: "implementer".to_string(),
            model: String::new(),
            reasoning_effort: String::new(),
            mode: "terminal".to_string(),
            goal_contract: None,
            resume_session: None,
            resume_vendor: None,
        },
        2_000_000_000,
    )
    .await
    .unwrap();

    runner::cancel_pending_task(&config, &pool, &locks, task.id, 2_000_000_001)
        .await
        .unwrap();
    assert_eq!(
        db::get_task(&pool, task.id).await.unwrap().unwrap().state,
        db::state::DISCARDED
    );
    // 파일형 투영(설계 2026-09-13)은 원장을 남기지 않는다 — 폐기가 워크트리를 지우면 블록도 함께 사라진다.
    assert!(!std::path::Path::new(&task.worktree_path).exists());
    let _ = std::fs::remove_dir_all(repo);
    let _ = std::fs::remove_file(db_path);
}

fn git(root: &std::path::Path, args: &[&str]) {
    assert!(std::process::Command::new("git")
        .current_dir(root)
        .args(args)
        .status()
        .unwrap()
        .success());
}
