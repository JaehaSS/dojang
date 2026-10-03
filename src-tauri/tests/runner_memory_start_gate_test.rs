//! 파일형 메모리(설계 2026-09-13) 이후의 승인 게이트 규약.
//!
//! 옛 규약은 worktree의 블록 바이트를 원장 해시와 대조해 변조를 막는 것이었다. 파일형
//! 투영에서는 블록이 창고 파일의 사본이고 소유권 해시가 없다(설계 §3) — 승인은 블록의
//! 상태와 무관하게 진행되고, 다음 작업이 파일에서 블록을 다시 만든다.

#[path = "support/temp_root.rs"]
mod temp_root;

use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::runner::config::{ExecutionPolicy, RunnerConfig};
use praxis_lib::runner::{self, QueuedTaskRequest};
use praxis_lib::{db, memory};

static COUNTER: AtomicU32 = AtomicU32::new(0);

#[tokio::test]
async fn pending_approval_proceeds_even_when_the_block_was_edited() {
    let suffix = COUNTER.fetch_add(1, Ordering::SeqCst);
    let repo = temp_root::dir().join(format!(
        "praxis-runner-start-gate-{}-{suffix}",
        std::process::id()
    ));
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "runner@example.test"]);
    git(&repo, &["config", "user.name", "Runner Test"]);
    std::fs::write(repo.join("CLAUDE.md"), "# owner\n").unwrap();
    git(&repo, &["add", "CLAUDE.md"]);
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
    let worktree_locks = praxis_lib::runner::worktree_lock::WorktreeLocks::default();
    let task = runner::create_queued_task(
        &config,
        &pool,
        &worktree_locks,
        QueuedTaskRequest {
            repository: repo.to_string_lossy().into_owned(),
            instruction: "empty projection".to_string(),
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
    assert_eq!(task.state, db::state::PENDING_APPROVAL);
    std::fs::write(
        std::path::Path::new(&task.worktree_path).join("CLAUDE.md"),
        "<!-- PRAXIS MEMORY START -->\ntampered\n<!-- PRAXIS MEMORY END -->\n# owner\n",
    )
    .unwrap();

    runner::approve_pending_task(&pool, &worktree_locks, task.id, 2_000_000_001)
        .await
        .unwrap();
    assert_eq!(
        db::get_task(&pool, task.id).await.unwrap().unwrap().state,
        db::state::QUEUED
    );
    assert!(!db::list_runner_events_after(&pool, 0, 10)
        .await
        .unwrap()
        .iter()
        .any(|event| event.kind == "memory_projection_start_blocked"));
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
