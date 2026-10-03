use sqlx::SqlitePool;

pub async fn assert_task_unfenced(pool: &SqlitePool, task_id: i64) -> Result<(), String> {
    if task_is_fenced(pool, task_id)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("durable review process lease must be resolved first".into());
    }
    Ok(())
}

pub async fn task_is_fenced(pool: &SqlitePool, task_id: i64) -> anyhow::Result<bool> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM review_process_leases WHERE task_id = ?")
            .bind(task_id)
            .fetch_one(pool)
            .await?;
    Ok(count > 0)
}

pub async fn assert_task_not_quarantined(pool: &SqlitePool, task_id: i64) -> Result<(), String> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM review_process_leases \
         WHERE task_id = ? AND state = 'quarantined'",
    )
    .bind(task_id)
    .fetch_one(pool)
    .await
    .map_err(|error| error.to_string())?;
    if count > 0 {
        return Err("durable review process lease is quarantined".into());
    }
    Ok(())
}
