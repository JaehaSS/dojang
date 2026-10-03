use std::path::Path;

use sqlx::SqlitePool;

use crate::db;
use crate::verify::VerifyReport;

const TERMINAL_STATES: [&str; 4] = [
    db::state::DONE,
    db::state::DISCARDED,
    db::state::FAILED,
    db::state::FINALIZING,
];

pub async fn persist_evidence(
    pool: &SqlitePool,
    task_id: i64,
    report: &VerifyReport,
    root: &Path,
    source_before: &crate::task_results::FingerprintObservation,
    source_after: &crate::task_results::FingerprintObservation,
    task_state: &str,
    started_at: i64,
    created_at: i64,
) -> Result<(), String> {
    let mut transaction = pool.begin().await.map_err(|error| error.to_string())?;
    // Receipt and compatibility evidence must commit together.  Otherwise a
    // legacy caller could see ready=true without the version fence that makes
    // that claim meaningful.
    let current_ready = crate::task_results::persist_verify_receipt_tx(
        &mut transaction,
        task_id,
        report,
        root,
        source_before,
        source_after,
        task_state,
        started_at,
        created_at,
    )
    .await?;
    let build = report.build.as_ref();
    let test = report.test.as_ref();
    let result = sqlx::query(
        "INSERT INTO evidence \
         (task_id, build_cmd, build_exit, test_cmd, test_exit, passed, failed, ready, created_at) \
         SELECT ?, ?, ?, ?, ?, ?, ?, ?, ? FROM tasks \
         WHERE id = ? AND state NOT IN (?, ?, ?, ?) \
         ON CONFLICT(task_id) DO UPDATE SET \
         build_cmd=excluded.build_cmd, build_exit=excluded.build_exit, \
         test_cmd=excluded.test_cmd, test_exit=excluded.test_exit, \
         passed=excluded.passed, failed=excluded.failed, \
         ready=excluded.ready, created_at=excluded.created_at",
    )
    .bind(task_id)
    .bind(build.map(|check| check.command.as_str()).unwrap_or(""))
    .bind(build.map(|check| i64::from(check.exit_code)).unwrap_or(-1))
    .bind(test.map(|check| check.command.as_str()).unwrap_or(""))
    .bind(test.map(|check| i64::from(check.exit_code)).unwrap_or(-1))
    .bind(
        report
            .summary
            .map(|summary| i64::from(summary.passed))
            .unwrap_or(0),
    )
    .bind(
        report
            .summary
            .map(|summary| i64::from(summary.failed))
            .unwrap_or(0),
    )
    .bind(current_ready)
    .bind(created_at)
    .bind(task_id)
    .bind(TERMINAL_STATES[0])
    .bind(TERMINAL_STATES[1])
    .bind(TERMINAL_STATES[2])
    .bind(TERMINAL_STATES[3])
    .execute(&mut *transaction)
    .await
    .map_err(|error| error.to_string())?;
    if result.rows_affected() != 1 {
        return Err("종료된 작업은 검증 결과를 저장할 수 없습니다".into());
    }
    append_event(
        &mut transaction,
        task_id,
        "verify",
        ready_detail(current_ready),
        created_at,
    )
    .await?;
    transaction
        .commit()
        .await
        .map_err(|error| error.to_string())
}

async fn append_event(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    task_id: i64,
    kind: &str,
    detail: &str,
    created_at: i64,
) -> Result<(), String> {
    sqlx::query("INSERT INTO task_events (task_id, ts, kind, detail) VALUES (?, ?, ?, ?)")
        .bind(task_id)
        .bind(created_at)
        .bind(kind)
        .bind(detail)
        .execute(&mut **transaction)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn ready_detail(ready: bool) -> &'static str {
    if ready {
        "ready"
    } else {
        "not-ready"
    }
}
