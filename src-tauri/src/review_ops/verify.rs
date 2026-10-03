use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::db::{self, state, Task};
use crate::managed_process::SharedProcessRegistrar;
use crate::review_ops::ReviewClaims;
use crate::verify::{self, ValidateSpec, VerifyReport};
use crate::worktree::Worktree;

mod execution;
use execution::VerifyRegistrars;

#[derive(Debug, Clone, Serialize)]
pub struct VerifyPreview {
    #[serde(flatten)]
    pub spec: ValidateSpec,
    pub preview_token: String,
}

pub async fn preview(
    pool: &SqlitePool,
    task_id: i64,
    root: &Path,
) -> Result<VerifyPreview, String> {
    let task = reviewable_task(pool, task_id).await?;
    verify_root(&task, root)?;
    let spec = verify::detect_spec(root);
    Ok(VerifyPreview {
        preview_token: preview_token(task_id, root, &spec),
        spec,
    })
}

pub async fn run(
    pool: SqlitePool,
    claims: ReviewClaims,
    task_id: i64,
    root: PathBuf,
    expected_token: String,
) -> Result<VerifyReport, String> {
    run_with_registrars(
        pool,
        claims,
        task_id,
        root,
        expected_token,
        VerifyRegistrars::default(),
    )
    .await
}

pub async fn run_managed(
    pool: SqlitePool,
    claims: ReviewClaims,
    task_id: i64,
    root: PathBuf,
    expected_token: String,
    build: SharedProcessRegistrar,
    test: SharedProcessRegistrar,
) -> Result<VerifyReport, String> {
    run_with_registrars(
        pool,
        claims,
        task_id,
        root,
        expected_token,
        VerifyRegistrars {
            build: Some(build),
            test: Some(test),
        },
    )
    .await
}

async fn run_with_registrars(
    pool: SqlitePool,
    claims: ReviewClaims,
    task_id: i64,
    root: PathBuf,
    expected_token: String,
    registrars: VerifyRegistrars,
) -> Result<VerifyReport, String> {
    tokio::spawn(run_owned(
        pool,
        claims,
        task_id,
        root,
        expected_token,
        registrars,
    ))
    .await
    .map_err(|error| error.to_string())?
}

async fn run_owned(
    pool: SqlitePool,
    claims: ReviewClaims,
    task_id: i64,
    root: PathBuf,
    expected_token: String,
    registrars: VerifyRegistrars,
) -> Result<VerifyReport, String> {
    let _claim = claims.claim_verify(task_id)?;
    let task = reviewable_task(&pool, task_id).await?;
    verify_root(&task, &root)?;
    let spec = verify::detect_spec(&root);
    if preview_token(task_id, &root, &spec) != expected_token {
        return Err("검증 명령이 미리보기 이후 변경되었습니다 — 다시 확인하세요".into());
    }
    let started_at = now();
    let started_epoch: i64 = sqlx::query_scalar(
        "SELECT completion_epoch FROM task_result_completion_epochs WHERE task_id=?",
    )
    .bind(task_id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?
    .unwrap_or(0);
    let source_before = crate::task_results::fingerprint_with_base(
        &root,
        &spec,
        task.base_revision.as_deref().or(Some(task.base.as_str())),
    );
    let root_for_worker = root.clone();
    let mut report =
        tokio::task::spawn_blocking(move || execution::execute(&root_for_worker, spec, registrars))
            .await
            .map_err(|error| error.to_string())?;
    let current = reviewable_task(&pool, task_id).await?;
    add_warnings(&current, &root, &mut report);
    let source_after = crate::task_results::fingerprint_with_base(
        &root,
        &report.spec,
        task.base_revision.as_deref().or(Some(task.base.as_str())),
    );
    let ended_epoch: i64 = sqlx::query_scalar(
        "SELECT completion_epoch FROM task_result_completion_epochs WHERE task_id=?",
    )
    .bind(task_id)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?
    .unwrap_or(0);
    crate::review_ops::store::persist_evidence(
        &pool,
        task_id,
        &report,
        &root,
        &source_before,
        &source_after,
        if task.state == state::RUNNING || started_epoch != ended_epoch {
            state::RUNNING
        } else {
            &current.state
        },
        started_at,
        now(),
    )
    .await?;
    let current_ready = crate::task_results::refresh_evidence(&pool, task_id).await?;
    if report.ready && !current_ready {
        report.warnings.push(
            "검사 후 결과 버전이 바뀌었거나 확인할 수 없어 현재 통과로 사용할 수 없습니다".into(),
        );
    }
    report.ready &= current_ready;
    Ok(report)
}

async fn reviewable_task(pool: &SqlitePool, task_id: i64) -> Result<Task, String> {
    let task = db::get_task(pool, task_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
    if matches!(
        task.state.as_str(),
        state::DONE | state::DISCARDED | state::FAILED | state::FINALIZING
    ) {
        return Err("종료된 작업은 검증할 수 없습니다".into());
    }
    Ok(task)
}

fn verify_root(task: &Task, root: &Path) -> Result<(), String> {
    // 두 canonicalize 실패는 원인이 다르다 — 앞은 작업의 워크트리가 사라진 것,
    // 뒤는 호출자가 넘긴 경로가 해석되지 않는 것. 같은 문구로 묶으면 진단이 어긋난다.
    let stored = Path::new(&task.worktree_path)
        .canonicalize()
        .map_err(|_| crate::worktree::missing_worktree_error(&task.worktree_path))?;
    let supplied = root
        .canonicalize()
        .map_err(|_| format!("검증 경로를 해석할 수 없습니다: {}", root.display()))?;
    if stored != supplied {
        return Err("작업의 워크트리와 검증 경로가 일치하지 않습니다".into());
    }
    Ok(())
}

fn add_warnings(task: &Task, root: &Path, report: &mut VerifyReport) {
    if task.state == state::RUNNING {
        report.warnings.push(
            "작업이 Running — 에이전트가 동시에 수정 중일 수 있어 결과가 불안정할 수 있습니다"
                .into(),
        );
    }
    if let Some(warning) =
        crate::goal_contract::manual_acceptance_warning(task.goal_contract.as_deref())
    {
        report.warnings.push(warning);
    }
    let worktree = Worktree {
        repo: task.repo.clone().into(),
        path: root.to_path_buf(),
        branch: task.branch.clone(),
        base: task.base.clone(),
        base_revision: task.base_revision.clone(),
    };
    if let Ok(files) = worktree.untracked() {
        if !files.is_empty() {
            report.warnings.push(format!(
                "검증이 미추적 파일 {}개 생성 — .gitignore 확인(Approve 시 함께 머지됨): {}",
                files.len(),
                files.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
            ));
        }
    }
}

fn preview_token(task_id: i64, root: &Path, spec: &ValidateSpec) -> String {
    let input = format!(
        "{task_id}\0{}\0{}\0{}\0{}",
        root.to_string_lossy(),
        spec.build.as_deref().unwrap_or(""),
        spec.test.as_deref().unwrap_or(""),
        spec.timeout_secs
    );
    format!("{:x}", Sha256::digest(input.as_bytes()))
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
