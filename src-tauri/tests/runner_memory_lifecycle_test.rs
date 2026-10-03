//! Runner task creation must use the same fail-closed projection gate as Desktop.

#[path = "support/temp_root.rs"]
mod temp_root;

use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::runner::config::{ExecutionPolicy, RunnerConfig};
use praxis_lib::runner::{create_queued_task, QueuedTaskRequest};
use praxis_lib::{db, memory};

static COUNTER: AtomicU32 = AtomicU32::new(0);

#[tokio::test]
async fn queued_task_carries_the_memory_file_into_its_worktree() {
    let (pool, db_path, repo) = setup("applied", "# owner\n").await;
    let now = 2_000_000_000;
    let scope = repo.canonicalize().unwrap().to_string_lossy().into_owned();
    let memory_root = repo.with_extension("memory");
    let key = memory::file::repo_key(&scope);
    std::fs::create_dir_all(memory_root.join(&key)).unwrap();
    std::fs::write(
        memory_root.join(&key).join("MEMORY.md"),
        "- ship runner parity\n",
    )
    .unwrap();
    db::set_setting(&pool, "memory_root", memory_root.to_string_lossy().as_ref())
        .await
        .unwrap();

    let task = create_queued_task(
        &config(&repo, ExecutionPolicy::AlwaysApprove),
        &pool,
        &praxis_lib::runner::worktree_lock::WorktreeLocks::default(),
        request(&repo, "ship runner parity"),
        now + 4,
    )
    .await
    .unwrap();

    assert_eq!(task.state, db::state::QUEUED);
    // 파일 본문이 그대로 컨텍스트 파일 블록에 실린다(설계 2026-09-13 R1).
    let projected =
        std::fs::read_to_string(std::path::Path::new(&task.worktree_path).join("AGENTS.md"))
            .unwrap();
    assert!(projected.contains("- ship runner parity"), "{projected}");
    assert!(
        projected.contains("# Dojang Memory (파일 정본:"),
        "{projected}"
    );
    assert!(projected.contains("# owner"), "{projected}");
    // 원장 대신 파일 상태만 남는다(R8).
    let rows = memory::file::list(&pool, &repo).await.unwrap();
    let repo_row = rows
        .iter()
        .find(|row| row.repo_key.as_deref() == Some(key.as_str()))
        .unwrap();
    assert_eq!(repo_row.last_task_id, Some(task.id));
    let _ = std::fs::remove_dir_all(memory_root);
    cleanup(db_path, repo);
}

#[tokio::test]
async fn projection_failure_fails_task_and_discards_isolated_worktree() {
    // 컨텍스트 파일 자리에 디렉터리가 있으면 투영은 쓸 곳이 없다 — 게이트는 닫혀야 한다.
    let (pool, db_path, repo) = setup_with_context_directory("failure").await;

    assert!(create_queued_task(
        &config(&repo, ExecutionPolicy::AlwaysApprove),
        &pool,
        &praxis_lib::runner::worktree_lock::WorktreeLocks::default(),
        request(&repo, "must not queue"),
        2_000_000_000,
    )
    .await
    .is_err());

    let tasks = db::list_tasks(&pool).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].state, db::state::FAILED);
    assert!(!std::path::Path::new(&tasks[0].worktree_path).exists());
    cleanup(db_path, repo);
}

async fn setup_with_context_directory(
    label: &str,
) -> (sqlx::SqlitePool, std::path::PathBuf, std::path::PathBuf) {
    let suffix = COUNTER.fetch_add(1, Ordering::SeqCst);
    let repo = temp_root::dir().join(format!(
        "praxis-runner-memory-{label}-{}-{suffix}",
        std::process::id()
    ));
    std::fs::create_dir_all(repo.join("AGENTS.md")).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "runner@example.test"]);
    git(&repo, &["config", "user.name", "Runner Test"]);
    std::fs::write(repo.join("AGENTS.md").join("keep.txt"), "not a file\n").unwrap();
    git(&repo, &["add", "AGENTS.md"]);
    git(&repo, &["commit", "-qm", "initial"]);
    let db_path = repo.with_extension("sqlite");
    let pool = db::init_pool(db_path.to_str().unwrap()).await.unwrap();
    memory::migrate(&pool).await.unwrap();
    (pool, db_path, repo)
}

async fn setup(
    label: &str,
    context: &str,
) -> (sqlx::SqlitePool, std::path::PathBuf, std::path::PathBuf) {
    let suffix = COUNTER.fetch_add(1, Ordering::SeqCst);
    let repo = temp_root::dir().join(format!(
        "praxis-runner-memory-{label}-{}-{suffix}",
        std::process::id()
    ));
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "runner@example.test"]);
    git(&repo, &["config", "user.name", "Runner Test"]);
    std::fs::write(repo.join("AGENTS.md"), context).unwrap();
    git(&repo, &["add", "AGENTS.md"]);
    git(&repo, &["commit", "-qm", "initial"]);
    let db_path = repo.with_extension("sqlite");
    let pool = db::init_pool(db_path.to_str().unwrap()).await.unwrap();
    memory::migrate(&pool).await.unwrap();
    (pool, db_path, repo)
}

fn config(repo: &std::path::Path, execution_policy: ExecutionPolicy) -> RunnerConfig {
    RunnerConfig {
        bind: "127.0.0.1:47831".parse().unwrap(),
        repository_roots: vec![repo.canonicalize().unwrap()],
        max_concurrent_tasks: 1,
        execution_policy,
        pairing_token_file: temp_root::dir().join("unused-runner-token"),
    }
}

fn request(repo: &std::path::Path, instruction: &str) -> QueuedTaskRequest {
    QueuedTaskRequest {
        repository: repo.to_string_lossy().into_owned(),
        instruction: instruction.to_string(),
        agent: "claude".to_string(),
        role: "implementer".to_string(),
        model: String::new(),
        reasoning_effort: String::new(),
        mode: "terminal".to_string(),
        goal_contract: None,
        resume_session: None,
        resume_vendor: None,
    }
}

fn git(root: &std::path::Path, args: &[&str]) {
    assert!(std::process::Command::new("git")
        .current_dir(root)
        .args(args)
        .status()
        .unwrap()
        .success());
}

fn cleanup(db_path: std::path::PathBuf, repo: std::path::PathBuf) {
    let _ = std::fs::remove_dir_all(repo);
    let _ = std::fs::remove_file(db_path);
}
