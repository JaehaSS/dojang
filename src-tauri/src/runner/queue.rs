use sqlx::SqlitePool;
use tokio::sync::OwnedSemaphorePermit;

use crate::db::{self, Task};
use crate::runner::capacity::RunnerCapacity;
use crate::runner::process;
use crate::runner::worktree_lock::WorktreeLocks;

/// DB lease와 in-process concurrency cap을 함께 적용하는 Runner queue worker.
#[derive(Clone)]
pub struct QueueWorker {
    pool: SqlitePool,
    slots: RunnerCapacity,
    terminal_active: process::ActiveTerminalTasks,
    conversation_active: process::ActiveConversationTasks,
    worktree_locks: WorktreeLocks,
}

pub struct LeasedTask {
    pub task: Task,
    _slot: OwnedSemaphorePermit,
    _task_fence: crate::side_question::RunnerTaskFence,
}

impl QueueWorker {
    pub fn new(pool: SqlitePool, max_concurrent_tasks: usize) -> Self {
        Self::with_worktree_locks(pool, max_concurrent_tasks, WorktreeLocks::default())
    }

    pub fn with_worktree_locks(
        pool: SqlitePool,
        max_concurrent_tasks: usize,
        worktree_locks: WorktreeLocks,
    ) -> Self {
        Self::with_capacity(
            pool,
            RunnerCapacity::new(max_concurrent_tasks),
            worktree_locks,
        )
    }

    pub fn with_capacity(
        pool: SqlitePool,
        slots: RunnerCapacity,
        worktree_locks: WorktreeLocks,
    ) -> Self {
        Self {
            pool,
            slots,
            terminal_active: process::active_terminal_tasks(),
            conversation_active: process::active_conversation_tasks(),
            worktree_locks,
        }
    }

    pub fn capacity(&self) -> RunnerCapacity {
        self.slots.clone()
    }

    pub fn worktree_locks(&self) -> WorktreeLocks {
        self.worktree_locks.clone()
    }

    /// 슬롯을 먼저 확보한 뒤 DB에서 queued 작업 하나를 lease한다.
    pub async fn lease_next(&self, now: i64) -> anyhow::Result<Option<LeasedTask>> {
        let Ok(slot) = self.slots.try_acquire() else {
            return Ok(None);
        };
        let Some(task) = db::claim_oldest_queued_task(&self.pool, now).await? else {
            return Ok(None);
        };
        // Main-task ownership stays exclusive until the provider returns.
        // Questions have separate ownership and consume their own semaphore permit.
        let Some(task_fence) = crate::side_question::try_runner_task_fence(task.id) else {
            sqlx::query("UPDATE tasks SET state = ? WHERE id = ? AND state = ?")
                .bind(db::state::QUEUED)
                .bind(task.id)
                .bind(db::state::STARTING)
                .execute(&self.pool)
                .await?;
            return Ok(None);
        };
        // 인증 preflight — 벤더가 로그아웃 상태면 시작하지 않고 큐로 되돌린다.
        // 실패시키지 않는 이유: 사용자가 로그인하면 컬럼만 비워도 그대로 재개된다.
        let agent = task.agent.clone();
        let blocked = tokio::task::spawn_blocking(move || {
            crate::agenthealth::blocking_vendor(agent.as_deref())
        })
        .await
        .unwrap_or(None);
        if let Some(vendor) = blocked {
            db::block_starting_task(&self.pool, task.id, &db::blocked::auth(&vendor), now).await?;
            return Ok(None);
        }
        Ok(Some(LeasedTask { task, _slot: slot, _task_fence: task_fence }))
    }

    /// queued 작업 하나를 lease하고, terminal agent가 종료될 때까지 해당 슬롯을 유지한다.
    pub async fn run_next(&self, now: i64) -> Result<Option<i32>, String> {
        let Some(leased) = self
            .lease_next(now)
            .await
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let exit_code = execute_task(
            self.pool.clone(),
            leased.task.clone(),
            now,
            self.terminal_active.clone(),
            self.conversation_active.clone(),
            self.worktree_locks.clone(),
        )
        .await?;
        drop(leased);
        Ok(Some(exit_code))
    }

    /// 가능한 슬롯마다 작업을 시작하고 즉시 반환한다. lease는 spawned task 종료까지 유지된다.
    pub async fn spawn_next(&self, now: i64) -> Result<bool, String> {
        let Some(leased) = self
            .lease_next(now)
            .await
            .map_err(|error| error.to_string())?
        else {
            return Ok(false);
        };
        let pool = self.pool.clone();
        let terminal_active = self.terminal_active.clone();
        let conversation_active = self.conversation_active.clone();
        let worktree_locks = self.worktree_locks.clone();
        tokio::spawn(async move {
            let task = leased.task.clone();
            let _ = execute_task(
                pool,
                task,
                now,
                terminal_active,
                conversation_active,
                worktree_locks,
            )
            .await;
            drop(leased);
        });
        Ok(true)
    }

    /// Run an already-durable isolated side question through the same global
    /// semaphore as normal Runner work, independently of the main task fence.
    pub fn spawn_side_question(&self, task_id: i64, turn_id: i64, now: i64) -> tokio::task::JoinHandle<()> {
        let pool = self.pool.clone();
        let slots = self.slots.clone();
        tokio::spawn(async move {
            loop {
                let queued: Option<String> = sqlx::query_scalar("SELECT state FROM side_question_turns WHERE id=? AND task_id=?")
                    .bind(turn_id).bind(task_id).fetch_optional(&pool).await.ok().flatten();
                if queued.as_deref() != Some("queued") { break; }
                let parent = db::get_task(&pool, task_id).await.ok().flatten();
                let Some(parent) = parent else {
                    let _ = crate::side_question::cancel(&pool, task_id, turn_id, now).await;
                    break;
                };
                if !crate::side_question::parent_allows_side_question(&parent) {
                    let _ = crate::side_question::cancel(&pool, task_id, turn_id, now).await;
                    break;
                }
                let Ok(permit) = slots.try_acquire() else {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    continue;
                };
                crate::side_question::run_turn(pool.clone(), task_id, turn_id, now).await;
                drop(permit);
                break;
            }
        })
    }

    /// 실행 중 terminal task의 활성 PTY stdin에 입력을 전달한다. `None` = 활성 세션 없음.
    pub fn write_terminal_input(&self, task_id: i64, data: &[u8]) -> Option<anyhow::Result<()>> {
        process::write_terminal_input(&self.terminal_active, task_id, data)
    }

    /// AwaitingReview 대화형 작업에 후속 메시지(B-1 주석 재전송 등)를 주입해 재개한다.
    /// 동기 가드(이미 진행 중/작업 없음/모드 불일치/상태 불일치)만 여기서 검사하고, 실제 턴은
    /// 백그라운드로 스폰해 즉시 반환한다 — `spawn_next`와 동일한 fire-and-forget 계약.
    pub async fn resume_conversation(
        &self,
        task_id: i64,
        message: String,
        now: i64,
    ) -> Result<(), String> {
        super::review_process::assert_task_unfenced(&self.pool, task_id).await?;
        if self
            .conversation_active
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(&task_id)
        {
            return Err("이미 진행 중인 턴이 있습니다 — 완료 후 다시 보내세요".into());
        }
        let task = db::get_task(&self.pool, task_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
        if task.mode != "conversation" {
            return Err("대화 모드 작업만 재전송을 지원합니다".into());
        }
        if task.state != db::state::AWAITING_REVIEW {
            return Err(format!(
                "검토 대기 중인 작업만 재전송할 수 있습니다 (현재 상태: {})",
                task.state
            ));
        }
        // Followups consume one shared permit and retain exclusive main-task ownership.
        let permit = self.slots.try_acquire()
            .map_err(|_| "동시 실행 한도에 도달해 슬롯이 없습니다 — 잠시 후 다시 보내세요".to_string())?;
        let task_fence = crate::side_question::try_runner_task_fence(task_id)
            .ok_or_else(|| "이미 진행 중인 턴이 있습니다 — 완료 후 다시 보내세요".to_string())?;
        if !db::mark_running_from_review(&self.pool, task_id, now)
            .await
            .map_err(|error| error.to_string())?
        {
            return Err("작업이 이미 완료 처리 중이거나 종료되었습니다".into());
        }
        let pool = self.pool.clone();
        let active = self.conversation_active.clone();
        tokio::spawn(async move {
            let _task_fence = task_fence;
            let _ = process::resume_conversation_task(pool, task, message, now, active).await;
            drop(permit);
        });
        Ok(())
    }

    /// 해당 task의 process group만 종료하고, 아직 Running일 때만 cancelled 상태를 기록한다.
    pub async fn cancel(&self, task_id: i64, now: i64) -> Result<bool, String> {
        super::review_process::assert_task_unfenced(&self.pool, task_id).await?;
        if !db::record_notification_cancel_intent(&self.pool, task_id)
            .await
            .map_err(|error| error.to_string())?
        {
            return Ok(false);
        }
        let terminal_cancelled = process::cancel_terminal_task(&self.terminal_active, task_id);
        let conversation_cancelled =
            process::cancel_conversation_task(&self.conversation_active, task_id);
        if !terminal_cancelled && !conversation_cancelled {
            db::clear_notification_cancel_if_signal_not_sent(&self.pool, task_id)
                .await
                .map_err(|error| error.to_string())?;
            return Ok(false);
        }
        db::finish_running_task(
            &self.pool,
            task_id,
            crate::db::state::FAILED,
            now,
            "cancelled",
            None,
        )
        .await
        .map_err(|error| error.to_string())
    }
}

async fn execute_task(
    pool: SqlitePool,
    task: Task,
    now: i64,
    terminal_active: process::ActiveTerminalTasks,
    conversation_active: process::ActiveConversationTasks,
    worktree_locks: WorktreeLocks,
) -> Result<i32, String> {
    let guard = worktree_locks
        .acquire(std::path::Path::new(&task.worktree_path))
        .await;
    verify_and_promote_start(&pool, &task, now).await?;
    drop(guard);
    match task.mode.as_str() {
        "terminal" => process::run_terminal_task(pool, task, now, terminal_active).await,
        "conversation" => {
            process::run_conversation_task(pool, task, now, conversation_active).await?;
            Ok(0)
        }
        mode => {
            let error = format!("Runner가 지원하지 않는 task mode입니다: {mode}");
            let _ = db::finish_running_task_with_notification(
                &pool,
                task.id,
                crate::db::state::FAILED,
                super::now_secs(),
                "failed",
                Some(&error),
                "failure",
            )
            .await;
            Err(error)
        }
    }
}

async fn verify_and_promote_start(pool: &SqlitePool, task: &Task, now: i64) -> Result<(), String> {
    // 메모리 정본은 창고 안 파일이다(설계 2026-09-13, P1) — 검증할 원장이 없으므로 시작
    // 영수증 없이 항상 승격한다. 옛 DB 투영(P2) 재검증은 제거됐다.
    let promoted = match db::promote_starting_task(pool, task.id, now).await {
        Ok(promoted) => promoted,
        Err(error) => return Err(cleanup_failed_start(pool, task.id, now, &error.to_string()).await),
    };
    if !promoted {
        return Err("Starting task changed state before verified spawn".into());
    }
    Ok(())
}

async fn cleanup_failed_start(pool: &SqlitePool, task_id: i64, now: i64, detail: &str) -> String {
    match db::fail_starting_task(pool, task_id, now, detail).await {
        Ok(true) => detail.to_string(),
        Ok(false) => format!("{detail}; Starting state changed before failure recording"),
        Err(error) => format!("{detail}; failure state recording failed: {error}"),
    }
}
