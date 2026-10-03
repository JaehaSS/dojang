use std::sync::atomic::{AtomicU32, Ordering};

use praxis_lib::runner::config::{ExecutionPolicy, RunnerConfig};
use praxis_lib::runner::{self, QueuedTaskRequest};
use praxis_lib::{db, memory};

static COUNTER: AtomicU32 = AtomicU32::new(0);

pub struct Fixture {
    pub pool: sqlx::SqlitePool,
    pub db_path: std::path::PathBuf,
    pub repo: std::path::PathBuf,
    pub config: RunnerConfig,
    pub locks: runner::worktree_lock::WorktreeLocks,
    pub task: db::Task,
}

pub async fn fixture(label: &str) -> Fixture {
    let suffix = COUNTER.fetch_add(1, Ordering::SeqCst);
    let repo = super::temp_root::dir().join(format!(
        "praxis-runner-finalize-{label}-{}-{suffix}",
        std::process::id()
    ));
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "runner@example.test"]);
    git(&repo, &["config", "user.name", "Runner Test"]);
    std::fs::write(repo.join("AGENTS.md"), "# owner\n").unwrap();
    git(&repo, &["add", "AGENTS.md"]);
    git(&repo, &["commit", "-qm", "initial"]);
    let db_path = repo.with_extension("sqlite");
    let pool = db::init_pool(db_path.to_str().unwrap()).await.unwrap();
    memory::migrate(&pool).await.unwrap();
    runner::finalization::migrate(&pool).await.unwrap();
    seed_memory_file(&pool, &repo).await;
    let config = config(&repo);
    let locks = runner::worktree_lock::WorktreeLocks::default();
    let task = runner::create_queued_task(&config, &pool, &locks, request(&repo), 2_000_000_004)
        .await
        .unwrap();
    db::transition_state_with_runner_event(
        &pool,
        task.id,
        db::state::AWAITING_REVIEW,
        2_000_000_005,
        "review",
        None,
    )
    .await
    .unwrap();
    Fixture {
        pool,
        db_path,
        repo,
        config,
        locks,
        task,
    }
}

/// 파일형 메모리(설계 2026-09-13) — 창고의 `<repo-key>/MEMORY.md`가 정본이다.
/// 투영은 이 파일을 읽어 worktree 컨텍스트 파일의 managed block에 싣는다.
async fn seed_memory_file(pool: &sqlx::SqlitePool, repo: &std::path::Path) {
    let root = repo.with_extension("memory");
    let scope = repo.canonicalize().unwrap().to_string_lossy().into_owned();
    let key = memory::file::repo_key(&scope);
    std::fs::create_dir_all(root.join(&key)).unwrap();
    std::fs::write(
        root.join(&key).join("MEMORY.md"),
        "- runner finalize invariant\n",
    )
    .unwrap();
    db::set_setting(pool, "memory_root", root.to_string_lossy().as_ref())
        .await
        .unwrap();
}

fn config(repo: &std::path::Path) -> RunnerConfig {
    RunnerConfig {
        bind: "127.0.0.1:47831".parse().unwrap(),
        repository_roots: vec![repo.canonicalize().unwrap()],
        max_concurrent_tasks: 1,
        execution_policy: ExecutionPolicy::AlwaysApprove,
        pairing_token_file: super::temp_root::dir().join("unused-runner-token"),
    }
}

fn request(repo: &std::path::Path) -> QueuedTaskRequest {
    QueuedTaskRequest {
        repository: repo.to_string_lossy().into_owned(),
        instruction: "runner finalize invariant".to_string(),
        agent: "claude".to_string(),
        model: String::new(),
        mode: "terminal".to_string(),
        goal_contract: None,
        role: "implementer".to_string(),
        reasoning_effort: String::new(),
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

pub fn cleanup(fixture: Fixture) {
    let _ = std::fs::remove_dir_all(fixture.repo.with_extension("memory"));
    let _ = std::fs::remove_dir_all(fixture.repo);
    let _ = std::fs::remove_file(fixture.db_path);
}
