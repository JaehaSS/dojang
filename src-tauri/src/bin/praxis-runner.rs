use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use praxis_lib::db;
use praxis_lib::runner::actions::{RunnerTaskActions, SharedTaskActions};
use praxis_lib::runner::auth::RunnerAuth;
use praxis_lib::runner::config::RunnerConfig;
use praxis_lib::runner::events::EventHub;
use praxis_lib::runner::http::{self, RunnerHttpState};
use praxis_lib::runner::queue::QueueWorker;
use praxis_lib::runner::{assistant, orchestrator_mcp, push, session, RETENTION_DAYS};

#[tokio::main]
async fn main() {
    praxis_lib::envpath::augment_path();
    if let Err(error) = run().await {
        eprintln!("praxis-runner 시작 실패: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let config_path = std::env::var("PRAXIS_RUNNER_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_home().join(".config/praxis/runner.toml"));
    let config_text = std::fs::read_to_string(&config_path)
        .map_err(|error| format!("Runner 설정을 읽을 수 없습니다 ({}): {error}", config_path.display()))?;
    let config = RunnerConfig::from_toml(&config_text)?;
    let auth = RunnerAuth::from_file(&config.pairing_token_file)?;

    let db_path = std::env::var("PRAXIS_RUNNER_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_home().join(".local/share/praxis/runner.sqlite"));
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    if let Some(parent) = std::path::absolute(&db_path)
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
    {
        let cache = parent.join("fastembed");
        if std::fs::create_dir_all(&cache).is_ok() {
            praxis_lib::embed::set_cache_dir(cache);
        }
    }

    let pool = db::init_pool(db_path.to_str().ok_or("유효하지 않은 DB 경로입니다")?)
        .await
        .map_err(|error| error.to_string())?;
    praxis_lib::memory::migrate(&pool)
        .await
        .map_err(|error| error.to_string())?;
    session::migrate(&pool).await.map_err(|error| error.to_string())?;
    push::migrate(&pool).await.map_err(|error| error.to_string())?;
    assistant::migrate(&pool).await.map_err(|error| error.to_string())?;
    praxis_lib::schedule::migrate(&pool)
        .await
        .map_err(|error| error.to_string())?;

    let now = praxis_lib::runner::now_secs();
    let recovered_tasks = recover_interrupted_tasks(&pool, now).await?;
    let _ = db::prune_runner_history(&pool, now - RETENTION_DAYS * 86_400).await;
    let _ = assistant::ensure_assistant_task(&pool, &config, now).await?;

    let events = EventHub::start(pool.clone());
    let queue = QueueWorker::new(pool.clone(), config.max_concurrent_tasks);

    if let Ok(vapid) = push::VapidKeys::load_or_create(&push::default_key_path()) {
        tokio::spawn(push::watch(pool.clone(), events.clone(), vapid));
    }

    let state = RunnerHttpState {
        auth,
        pool: pool.clone(),
        config: config.clone(),
        recovered_tasks,
        events,
        queue: queue.clone(),
        started_at: now,
        review_claims: Default::default(),
    };

    let worker_state = state.clone();
    let actions: SharedTaskActions = Arc::new(RunnerTaskActions);
    tokio::spawn(async move {
        let mut last_minute_tick = 0_i64;
        loop {
            let tick_now = praxis_lib::runner::now_secs();
            let _ = worker_state
                .queue
                .spawn_assistant_if_queued(&worker_state.config, tick_now)
                .await;
            while matches!(worker_state.queue.spawn_next(tick_now).await, Ok(true)) {}

            if tick_now - last_minute_tick >= 60 {
                last_minute_tick = tick_now;
                orchestrator_mcp::run_due_schedules(&worker_state, &actions, tick_now).await;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    });

    eprintln!(
        "Praxis Runner initialized on {} (max {} tasks, recovered {})",
        state.config.bind,
        state.config.max_concurrent_tasks,
        recovered_tasks
    );

    let listener = tokio::net::TcpListener::bind(state.config.bind)
        .await
        .map_err(|error| format!("Runner bind 실패 ({}): {error}", state.config.bind))?;
    let (draining, drained) = tokio::sync::oneshot::channel();
    let server = axum::serve(
        listener,
        http::router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        let _ = draining.send(());
    });
    let served = tokio::select! {
        result = std::future::IntoFuture::into_future(server) =>
            result.map_err(|error| format!("Runner HTTP server가 종료되었습니다: {error}")),
        _ = async {
            let _ = drained.await;
            tokio::time::sleep(Duration::from_secs(3)).await;
        } => Ok(()),
    };
    praxis_lib::side_question::shutdown_all();
    served
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {},
                    _ = terminate.recv() => {},
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
    praxis_lib::side_question::shutdown_all();
}

async fn recover_interrupted_tasks(
    pool: &sqlx::SqlitePool,
    now: i64,
) -> Result<u64, String> {
    let _ = sqlx::query(
        "UPDATE tasks
         SET state = ?, updated_at = ?
         WHERE kind = 'assistant' AND state IN (?, ?, ?)",
    )
    .bind(db::state::AWAITING_REVIEW)
    .bind(now)
    .bind(db::state::RUNNING)
    .bind(db::state::STARTING)
    .bind(db::state::QUEUED)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;

    let updated = sqlx::query(
        "UPDATE tasks
         SET state = ?, updated_at = ?
         WHERE kind = 'coding' AND state IN (?, ?)",
    )
    .bind(db::state::FAILED)
    .bind(now)
    .bind(db::state::RUNNING)
    .bind(db::state::STARTING)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;

    Ok(updated.rows_affected())
}

fn default_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
