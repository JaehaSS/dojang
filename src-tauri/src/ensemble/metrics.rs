use serde::Serialize;
use sqlx::SqlitePool;
#[derive(Debug, Clone, Default, PartialEq)]
struct EventMetrics {
    requested_model: Option<String>,
    resolved_model: Option<String>,
    active_seconds: i64,
    user_turns: i64,
    completed_turns: i64,
    failed_turns: i64,
    tool_calls: i64,
    tool_errors: i64,
    tokens_in: i64,
    tokens_out: i64,
    cost_usd: f64,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CandidateBenchmarkMetrics {
    pub task_id: i64,
    pub agent: String,
    /// CLI에 전달한 task override 또는 실행 시점의 agent 기본 설정.
    pub model: Option<String>,
    /// 공급자 stream/session metadata에서 실제로 관측한 모델.
    pub resolved_model: Option<String>,
    pub state: String,
    pub active_seconds: i64,
    pub user_turns: i64,
    pub completed_turns: i64,
    pub failed_turns: i64,
    pub tool_calls: i64,
    pub tool_errors: i64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost_usd: f64,
}
#[derive(sqlx::FromRow)]
struct TimedEvent {
    timestamp: i64,
    event: String,
}
pub async fn candidate_metrics(
    pool: &SqlitePool,
    ensemble: &str,
) -> anyhow::Result<Vec<CandidateBenchmarkMetrics>> {
    let tasks = crate::db::tasks_by_ensemble(pool, ensemble).await?;
    let mut candidates = Vec::with_capacity(tasks.len());
    for task in tasks {
        let events = timed_events(pool, task.id).await?;
        let event_metrics =
            summarize_events(events.iter().map(|row| (row.timestamp, row.event.as_str())));
        candidates.push(candidate_from(task, event_metrics));
    }
    Ok(candidates)
}

async fn timed_events(pool: &SqlitePool, task_id: i64) -> anyhow::Result<Vec<TimedEvent>> {
    sqlx::query_as(
        "SELECT ts AS timestamp, event FROM convo_events WHERE task_id = ? ORDER BY id ASC",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

fn candidate_from(task: crate::db::Task, metrics: EventMetrics) -> CandidateBenchmarkMetrics {
    CandidateBenchmarkMetrics {
        task_id: task.id,
        agent: task.agent.unwrap_or(task.branch),
        model: metrics.requested_model.or(task.model),
        resolved_model: metrics.resolved_model,
        state: task.state,
        active_seconds: metrics.active_seconds,
        user_turns: metrics.user_turns,
        completed_turns: metrics.completed_turns,
        failed_turns: metrics.failed_turns,
        tool_calls: metrics.tool_calls,
        tool_errors: metrics.tool_errors,
        tokens_in: metrics.tokens_in,
        tokens_out: metrics.tokens_out,
        cost_usd: metrics.cost_usd,
    }
}

fn summarize_events<'a>(events: impl IntoIterator<Item = (i64, &'a str)>) -> EventMetrics {
    let mut metrics = EventMetrics::default();
    let mut active_turn_started = None;
    for (timestamp, raw) in events {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(raw) else {
            continue;
        };
        match event.get("kind").and_then(serde_json::Value::as_str) {
            Some("user") => {
                metrics.user_turns = metrics.user_turns.saturating_add(1);
                active_turn_started = Some(timestamp);
            }
            Some("result") => {
                finish_turn(&mut metrics, &event, active_turn_started, timestamp);
                active_turn_started = None;
            }
            Some("tool_use") => metrics.tool_calls = metrics.tool_calls.saturating_add(1),
            Some("tool_result") if event_bool(&event, "is_error") => {
                metrics.tool_errors = metrics.tool_errors.saturating_add(1);
            }
            Some("model_snapshot") => {
                update_model(&mut metrics.requested_model, &event, "requested");
                update_model(&mut metrics.resolved_model, &event, "resolved");
            }
            _ => {}
        }
    }
    metrics
}

fn finish_turn(
    metrics: &mut EventMetrics,
    event: &serde_json::Value,
    started_at: Option<i64>,
    finished_at: i64,
) {
    if event_bool(event, "is_error") {
        metrics.failed_turns = metrics.failed_turns.saturating_add(1);
    } else {
        metrics.completed_turns = metrics.completed_turns.saturating_add(1);
    }
    if let Some(started_at) = started_at.filter(|started| finished_at >= *started) {
        metrics.active_seconds = metrics
            .active_seconds
            .saturating_add(finished_at.saturating_sub(started_at));
    }
    metrics.tokens_in = metrics
        .tokens_in
        .saturating_add(event_i64(event, "tokens_in"));
    metrics.tokens_out = metrics
        .tokens_out
        .saturating_add(event_i64(event, "tokens_out"));
    metrics.cost_usd = add_cost(metrics.cost_usd, event_f64(event, "cost_usd"));
}

fn update_model(current: &mut Option<String>, event: &serde_json::Value, key: &str) {
    let Some(model) = event
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty() && model.chars().count() <= 160)
    else {
        return;
    };
    *current = Some(model.to_string());
}

fn event_bool(event: &serde_json::Value, key: &str) -> bool {
    event
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn event_i64(event: &serde_json::Value, key: &str) -> i64 {
    event
        .get(key)
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0)
        .max(0)
}

fn event_f64(event: &serde_json::Value, key: &str) -> f64 {
    event
        .get(key)
        .and_then(serde_json::Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(0.0)
}

fn add_cost(current: f64, addition: f64) -> f64 {
    let total = current + addition;
    if total.is_finite() {
        return total;
    }
    f64::MAX
}

#[cfg(test)]
mod tests;
