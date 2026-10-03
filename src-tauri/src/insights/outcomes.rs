use serde::Serialize;
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct OutcomeInsights {
    pub task_count: i64,
    pub accepted_task_count: i64,
    pub goal_contract_task_count: i64,
    pub ready_accepted_task_count: i64,
    pub ensemble_count: i64,
    pub selected_ensemble_count: i64,
    pub ambiguous_ensemble_count: i64,
    pub no_selection_ensemble_count: i64,
    pub average_accept_seconds: Option<f64>,
    pub no_reexplanation_target_task_count: i64,
    pub no_reexplanation_observed_task_count: i64,
    pub no_reexplanation_success_task_count: i64,
    pub no_reexplanation_unmeasured_task_count: i64,
    pub no_reexplanation_completion_rate: Option<f64>,
}

#[derive(FromRow)]
struct TaskOutcomeStats {
    task_count: i64,
    accepted_task_count: i64,
    goal_contract_task_count: i64,
    ready_accepted_task_count: i64,
    average_accept_seconds: Option<f64>,
}

#[derive(FromRow)]
struct EnsembleOutcomeStats {
    ensemble_count: i64,
    selected_ensemble_count: i64,
    ambiguous_ensemble_count: i64,
    no_selection_ensemble_count: i64,
}

#[derive(FromRow)]
struct NoReexplanationStats {
    target_task_count: i64,
    observed_task_count: i64,
    success_task_count: i64,
    unmeasured_task_count: i64,
}

pub async fn compute_outcomes(
    pool: &SqlitePool,
    range: &str,
    now: i64,
) -> anyhow::Result<OutcomeInsights> {
    let cutoff = cutoff(range, now);
    let tasks = load_task_stats(pool, cutoff).await?;
    let ensembles = load_ensemble_stats(pool, cutoff).await?;
    let no_reexplanation = load_no_reexplanation_stats(pool, cutoff).await?;
    let no_reexplanation_completion_rate = complete_observation_rate(&no_reexplanation);
    Ok(OutcomeInsights {
        task_count: tasks.task_count,
        accepted_task_count: tasks.accepted_task_count,
        goal_contract_task_count: tasks.goal_contract_task_count,
        ready_accepted_task_count: tasks.ready_accepted_task_count,
        ensemble_count: ensembles.ensemble_count,
        selected_ensemble_count: ensembles.selected_ensemble_count,
        ambiguous_ensemble_count: ensembles.ambiguous_ensemble_count,
        no_selection_ensemble_count: ensembles.no_selection_ensemble_count,
        average_accept_seconds: tasks.average_accept_seconds,
        no_reexplanation_target_task_count: no_reexplanation.target_task_count,
        no_reexplanation_observed_task_count: no_reexplanation.observed_task_count,
        no_reexplanation_success_task_count: no_reexplanation.success_task_count,
        no_reexplanation_unmeasured_task_count: no_reexplanation.unmeasured_task_count,
        no_reexplanation_completion_rate,
    })
}

fn cutoff(range: &str, now: i64) -> i64 {
    match range {
        "7d" => now.saturating_sub(7 * 86_400),
        "30d" => now.saturating_sub(30 * 86_400),
        _ => 0,
    }
}

async fn load_task_stats(pool: &SqlitePool, cutoff: i64) -> anyhow::Result<TaskOutcomeStats> {
    sqlx::query_as(
        "WITH scoped AS (SELECT * FROM tasks WHERE updated_at >= ?) \
         SELECT COUNT(*) AS task_count, \
                COALESCE(SUM(state = 'Done'), 0) AS accepted_task_count, \
                COALESCE(SUM(goal_contract IS NOT NULL), 0) AS goal_contract_task_count, \
                (SELECT COUNT(DISTINCT s.id) FROM scoped s \
                   JOIN evidence e ON e.task_id = s.id \
                  WHERE s.state = 'Done' AND e.ready = 1) AS ready_accepted_task_count, \
                AVG(CASE WHEN state = 'Done' AND updated_at >= created_at \
                         THEN updated_at - created_at END) AS average_accept_seconds \
           FROM scoped",
    )
    .bind(cutoff)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

async fn load_ensemble_stats(
    pool: &SqlitePool,
    cutoff: i64,
) -> anyhow::Result<EnsembleOutcomeStats> {
    sqlx::query_as(
        "WITH eligible AS ( \
           SELECT ensemble FROM tasks \
            WHERE ensemble IS NOT NULL AND TRIM(ensemble) <> '' \
            GROUP BY ensemble HAVING MAX(updated_at) >= ? \
         ), decisions AS ( \
           SELECT t.ensemble, SUM(t.state = 'Done') AS done_count \
             FROM tasks t JOIN eligible e ON e.ensemble = t.ensemble \
            GROUP BY t.ensemble \
         ) \
         SELECT COUNT(*) AS ensemble_count, \
                COALESCE(SUM(done_count = 1), 0) AS selected_ensemble_count, \
                COALESCE(SUM(done_count > 1), 0) AS ambiguous_ensemble_count, \
                COALESCE(SUM(done_count = 0), 0) AS no_selection_ensemble_count \
           FROM decisions",
    )
    .bind(cutoff)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

async fn load_no_reexplanation_stats(
    pool: &SqlitePool,
    cutoff: i64,
) -> anyhow::Result<NoReexplanationStats> {
    sqlx::query_as(
        "WITH eligible AS ( \
           SELECT t.id, \
                  EXISTS(SELECT 1 FROM task_events e WHERE e.task_id = t.id \
                    AND e.kind = 'followup_observation_started') AS observed, \
                  EXISTS(SELECT 1 FROM task_events e WHERE e.task_id = t.id \
                    AND e.kind = 'user_followup_input_observed') AS followed_up \
             FROM tasks t \
            WHERE t.updated_at >= ? \
              AND t.state IN ('AwaitingReview', 'Done') \
              AND EXISTS(SELECT 1 FROM task_events e WHERE e.task_id = t.id \
                    AND e.kind = 'followup_observation_started') \
         ) \
         SELECT COUNT(*) AS target_task_count, \
                COALESCE(SUM(observed), 0) AS observed_task_count, \
                COALESCE(SUM(observed = 1 AND followed_up = 0), 0) AS success_task_count, \
                COALESCE(SUM(observed = 0), 0) AS unmeasured_task_count \
           FROM eligible",
    )
    .bind(cutoff)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

fn complete_observation_rate(stats: &NoReexplanationStats) -> Option<f64> {
    if stats.target_task_count == 0 || stats.observed_task_count != stats.target_task_count {
        return None;
    }
    Some(stats.success_task_count as f64 / stats.observed_task_count as f64)
}

#[cfg(test)]
mod tests;
