use std::collections::HashMap;

use sqlx::{FromRow, Row, SqlitePool};

#[derive(Debug, FromRow)]
pub(super) struct CandidateFeedbackRow {
    pub(super) ensemble: String,
    pub(super) task_id: i64,
    pub(super) agent: Option<String>,
    pub(super) branch: String,
    pub(super) task_model: Option<String>,
    pub(super) state: String,
    pub(super) updated_at: i64,
}

#[derive(Default)]
pub(super) struct ModelSnapshot {
    pub(super) requested: Option<String>,
    pub(super) resolved: Option<String>,
}

pub(super) async fn load_candidates(
    pool: &SqlitePool,
    limit: i64,
) -> anyhow::Result<Vec<CandidateFeedbackRow>> {
    sqlx::query_as(
        "WITH recent AS ( \
           SELECT ensemble, MAX(updated_at) AS latest_at FROM tasks \
           WHERE ensemble IS NOT NULL AND TRIM(ensemble) <> '' \
           GROUP BY ensemble ORDER BY latest_at DESC, ensemble DESC LIMIT ? \
         ) \
         SELECT t.ensemble, t.id AS task_id, t.agent, t.branch, t.model AS task_model, \
                t.state, t.updated_at \
         FROM recent r JOIN tasks t ON t.ensemble = r.ensemble \
         ORDER BY r.latest_at DESC, r.ensemble DESC, t.id ASC",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub(super) async fn load_models(
    pool: &SqlitePool,
    limit: i64,
) -> anyhow::Result<HashMap<i64, ModelSnapshot>> {
    let rows = sqlx::query(
        "WITH recent AS ( \
           SELECT ensemble, MAX(updated_at) AS latest_at FROM tasks \
           WHERE ensemble IS NOT NULL AND TRIM(ensemble) <> '' \
           GROUP BY ensemble ORDER BY latest_at DESC, ensemble DESC LIMIT ? \
         ) \
         SELECT t.id AS task_id, e.event FROM recent r \
         JOIN tasks t ON t.ensemble = r.ensemble \
         JOIN convo_events e ON e.task_id = t.id \
         WHERE e.event LIKE '%\"model_snapshot\"%' ORDER BY e.id ASC",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    let mut models = HashMap::new();
    for row in rows {
        let task_id: i64 = row.try_get("task_id")?;
        let event: String = row.try_get("event")?;
        update_model_snapshot(models.entry(task_id).or_default(), &event);
    }
    Ok(models)
}

fn update_model_snapshot(snapshot: &mut ModelSnapshot, raw: &str) {
    let Ok(event) = serde_json::from_str::<serde_json::Value>(raw) else {
        return;
    };
    if event.get("kind").and_then(|value| value.as_str()) != Some("model_snapshot") {
        return;
    }
    update_model(&mut snapshot.requested, &event, "requested");
    update_model(&mut snapshot.resolved, &event, "resolved");
}

fn update_model(current: &mut Option<String>, event: &serde_json::Value, key: &str) {
    let Some(model) = event
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|model| !model.is_empty() && model.chars().count() <= 160)
    else {
        return;
    };
    *current = Some(model.to_string());
}
