use std::collections::HashMap;

use serde::Serialize;
use sqlx::SqlitePool;

const HISTORY_LIMIT: i64 = 20;

mod storage;
use storage::{load_candidates, load_models, CandidateFeedbackRow, ModelSnapshot};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnsembleSelectionStatus {
    Pending,
    Selected,
    Ambiguous,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EnsembleFeedbackEntry {
    pub ensemble: String,
    pub updated_at: i64,
    pub candidate_count: i64,
    pub selection_status: EnsembleSelectionStatus,
    pub selected_task_id: Option<i64>,
    pub selected_agent: Option<String>,
    pub requested_model: Option<String>,
    pub resolved_model: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct EnsembleFeedbackHistory {
    pub entries: Vec<EnsembleFeedbackEntry>,
    pub selected_count: i64,
    pub pending_count: i64,
    pub ambiguous_count: i64,
}

pub async fn feedback_history(pool: &SqlitePool) -> anyhow::Result<EnsembleFeedbackHistory> {
    feedback_history_with_limit(pool, HISTORY_LIMIT).await
}

async fn feedback_history_with_limit(
    pool: &SqlitePool,
    limit: i64,
) -> anyhow::Result<EnsembleFeedbackHistory> {
    let limit = limit.clamp(1, 100);
    let candidates = load_candidates(pool, limit).await?;
    let models = load_models(pool, limit).await?;
    let mut groups: Vec<Vec<CandidateFeedbackRow>> = Vec::new();
    for candidate in candidates {
        match groups.last_mut() {
            Some(group) if group[0].ensemble == candidate.ensemble => group.push(candidate),
            _ => groups.push(vec![candidate]),
        }
    }
    let entries = groups
        .iter()
        .filter_map(|group| feedback_entry(group, &models))
        .collect();
    Ok(summarize_history(entries))
}

fn classify_selection<'a>(
    states: impl IntoIterator<Item = &'a str>,
) -> (EnsembleSelectionStatus, Option<usize>) {
    let mut selected = None;
    for (index, state) in states.into_iter().enumerate() {
        if state != crate::db::state::DONE {
            continue;
        }
        if selected.is_some() {
            return (EnsembleSelectionStatus::Ambiguous, None);
        }
        selected = Some(index);
    }
    match selected {
        Some(index) => (EnsembleSelectionStatus::Selected, Some(index)),
        None => (EnsembleSelectionStatus::Pending, None),
    }
}

fn feedback_entry(
    group: &[CandidateFeedbackRow],
    models: &HashMap<i64, ModelSnapshot>,
) -> Option<EnsembleFeedbackEntry> {
    let first = group.first()?;
    let (selection_status, selected_index) =
        classify_selection(group.iter().map(|candidate| candidate.state.as_str()));
    let selected = selected_index.and_then(|index| group.get(index));
    let model = selected.and_then(|candidate| models.get(&candidate.task_id));
    Some(EnsembleFeedbackEntry {
        ensemble: first.ensemble.clone(),
        updated_at: group.iter().map(|row| row.updated_at).max().unwrap_or(0),
        candidate_count: i64::try_from(group.len()).unwrap_or(i64::MAX),
        selection_status,
        selected_task_id: selected.map(|candidate| candidate.task_id),
        selected_agent: selected.map(|candidate| {
            candidate
                .agent
                .clone()
                .unwrap_or_else(|| candidate.branch.clone())
        }),
        requested_model: model
            .and_then(|snapshot| snapshot.requested.clone())
            .or_else(|| selected.and_then(|candidate| candidate.task_model.clone())),
        resolved_model: model.and_then(|snapshot| snapshot.resolved.clone()),
    })
}

fn summarize_history(entries: Vec<EnsembleFeedbackEntry>) -> EnsembleFeedbackHistory {
    let mut history = EnsembleFeedbackHistory {
        entries,
        ..EnsembleFeedbackHistory::default()
    };
    for entry in &history.entries {
        match entry.selection_status {
            EnsembleSelectionStatus::Pending => history.pending_count += 1,
            EnsembleSelectionStatus::Ambiguous => history.ambiguous_count += 1,
            EnsembleSelectionStatus::Selected => history.selected_count += 1,
        }
    }
    history
}

#[cfg(test)]
mod tests;
