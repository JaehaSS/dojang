//! Read models only: provider contracts and registered execution ownership.
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone, Serialize)]
pub struct Feature {
    pub key: &'static str,
    pub state: &'static str,
    pub enabled: Option<bool>,
    pub reason: &'static str,
}
#[derive(Debug, Clone, Serialize)]
pub struct Capabilities {
    pub agent: String,
    pub route: String,
    pub features: Vec<Feature>,
}

/// The adapter contract used by question-session admission, not a CLI version guess.
pub fn question_runtime(agent: &str) -> Option<&'static str> {
    match agent {
        "codex" => Some(crate::convo::interaction::RUNTIME),
        "claude" => Some(crate::convo::interaction::RUNTIME_LOCAL),
        _ => None,
    }
}
pub fn supports_agent_switch(agent: &str) -> bool {
    matches!(agent.trim(), "claude" | "codex" | "agy")
}
pub fn supports_isolated_provider(agent: &str) -> bool {
    agent == "claude"
}

pub fn capabilities(
    agent: &str,
    mode: &str,
    runtime: Option<&str>,
    approvals: bool,
    resumable: bool,
) -> Capabilities {
    let known = matches!(agent, "claude" | "codex" | "agy" | "gemini" | "antigravity");
    let conversation = mode == "conversation";
    let structured = question_runtime(agent).is_some_and(|r| Some(r) == runtime);
    let route = if !conversation {
        "terminal"
    } else if structured {
        if agent == "codex" {
            "codex-app-server"
        } else {
            "claude-mcp"
        }
    } else if known {
        "stream"
    } else {
        "unknown"
    };
    let mut features = Vec::new();
    let mut add = |key, state, enabled, reason| {
        features.push(Feature {
            key,
            state,
            enabled,
            reason,
        })
    };
    let usable = known && conversation;
    add(
        "resume",
        if usable { "supported" } else { "unsupported" },
        Some(usable && resumable),
        "저장된 벤더 세션 ID가 있어야 같은 대화를 재개할 수 있습니다.",
    );
    add(
        "interrupt",
        if usable || mode == "terminal" { "supported" } else { "unknown" },
        Some(usable || mode == "terminal"),
        "기존 실행 컨트롤과 프로세스 정리 계약을 사용합니다.",
    );
    add(
        "questions",
        if structured {
            "supported"
        } else {
            "unsupported"
        },
        Some(structured),
        "질문 실행 계약에 바인딩된 Claude·Codex 세션에서만 구조화 질문을 제공합니다.",
    );
    add(
        "approvals",
        if structured {
            "supported"
        } else {
            "unsupported"
        },
        Some(structured && approvals),
        "질문 실행 계약과 해당 세션의 승인 정책이 켜져 있어야 합니다.",
    );
    add("subagents", if usable && matches!(agent,"claude"|"codex") {"supported"} else {"unknown"}, None, "Claude 스트림·Codex의 제한된 자식 기록을 관측합니다. 전체 자식 프로세스 목록을 보장하지 않습니다.");
    add(
        "tokens",
        if usable { "supported" } else { "unknown" },
        None,
        "벤더가 보고한 토큰만 사용합니다. 보고가 없는 값은 미확인입니다.",
    );
    add("cost", if usable && agent == "claude" {"supported"} else {"unsupported"}, None, "Claude가 보고한 달러 비용만 계측합니다. Codex·Antigravity의 달러 비용을 0으로 추정하지 않습니다.");
    add(
        "isolated_questions",
        if conversation && supports_isolated_provider(agent) {
            "supported"
        } else {
            "unsupported"
        },
        None,
        "별도 질의의 실제 사용 가능 여부는 기존 정책·CLI 검사와 작업 상태가 최종 판단합니다.",
    );
    add(
        "agent_switch",
        if conversation && supports_agent_switch(agent) {
            "supported"
        } else {
            "unsupported"
        },
        Some(conversation && supports_agent_switch(agent)),
        "작업의 상태와 기존 세션 전환 가드를 유지합니다.",
    );
    Capabilities {
        agent: agent.into(),
        route: route.into(),
        features,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Observation {
    pub task_id: i64,
    pub revision: String,
    pub checked_at: i64,
    pub task_state: String,
    pub status: String,
    pub registered_execution: bool,
    pub cleanup_required: bool,
    pub pending_questions: i64,
    pub review_processes: i64,
    pub side_questions: i64,
    pub deliveries: i64,
    pub approvals: i64,
    pub child_processes: Option<i64>,
    pub capabilities: Capabilities,
}

/// Counts are one SQLite read transaction. Runtime ownership is a separate live fact.
pub async fn snapshot(
    pool: &SqlitePool,
    id: i64,
    live_turn: bool,
    now: i64,
) -> Result<Observation, String> {
    let runtime = crate::convo::interaction::runtime_of(pool, id).await?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let task = sqlx::query_as::<_, crate::db::Task>("SELECT * FROM tasks WHERE id=?")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("작업을 찾을 수 없습니다")?;
    let approval_on: Option<bool> =
        sqlx::query_scalar("SELECT approvals FROM convo_runtime_bindings WHERE task_id=?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    let counts=sqlx::query("SELECT \
      (SELECT COUNT(*) FROM convo_executions WHERE task_id=?1 AND state IN ('starting','running','cancelling','finalizing','cleanup_failed')) executions, \
      (SELECT COUNT(*) FROM convo_interactions i JOIN convo_executions e ON e.id=i.execution_id WHERE e.task_id=?1 AND e.state IN ('starting','running','cancelling','finalizing','cleanup_failed') AND i.state='pending' AND i.expires_at>?2) questions, \
      (SELECT COUNT(*) FROM convo_executions WHERE task_id=?1 AND state='cleanup_failed') cleanup, \
      (SELECT COUNT(*) FROM review_process_leases WHERE task_id=?1) reviews, \
      (SELECT COUNT(*) FROM side_question_turns WHERE task_id=?1 AND state IN ('queued','running','stopping')) side_questions, \
      (SELECT COUNT(*) FROM result_review_outbox WHERE task_id=?1 AND state IN ('queued','dispatching','delivery_unknown')) deliveries, \
      (SELECT COUNT(*) FROM local_approval_finalizations WHERE task_id=?1 AND state<>'completed') approvals")
      .bind(id).bind(now).fetch_one(&mut *tx).await.map_err(|e|e.to_string())?;
    let read = |key: &str| counts.get::<i64, _>(key);
    let pending_questions = read("questions");
    let review_processes = read("reviews");
    let side_questions = read("side_questions");
    let deliveries = read("deliveries");
    let approvals = read("approvals");
    let registered_execution = live_turn || read("executions") > 0 || task.convo_pgid.is_some();
    let residual = review_processes + side_questions + deliveries + approvals;
    let cleanup_required = read("cleanup") > 0;
    let status = if cleanup_required {
        "remaining_work"
    } else {
        classify(
            &task.state,
            task.awaiting_kind.as_deref(),
            task.blocked_reason.as_deref(),
            registered_execution,
            pending_questions,
            residual,
        )
    };
    tx.commit().await.map_err(|e| e.to_string())?;
    let capabilities = capabilities(
        task.agent.as_deref().unwrap_or("claude"),
        &task.mode,
        runtime.as_deref(),
        approval_on.unwrap_or(false),
        task.convo_session_id.is_some(),
    );
    let contract = serde_json::to_string(&capabilities).map_err(|e| e.to_string())?;
    let revision=format!("{:x}",Sha256::digest(format!("{}:{}:{:?}:{:?}:{registered_execution}:{pending_questions}:{review_processes}:{side_questions}:{deliveries}:{approvals}:{runtime:?}:{approval_on:?}:{contract}:{cleanup_required}",task.state,task.updated_at,task.awaiting_kind,task.blocked_reason).as_bytes()));
    Ok(Observation {
        task_id: id,
        revision,
        checked_at: now,
        task_state: task.state,
        status: status.into(),
        registered_execution,
        cleanup_required,
        pending_questions,
        review_processes,
        side_questions,
        deliveries,
        approvals,
        child_processes: None,
        capabilities,
    })
}
fn classify(
    state: &str,
    awaiting: Option<&str>,
    blocked: Option<&str>,
    execution: bool,
    questions: i64,
    residual: i64,
) -> &'static str {
    if questions > 0
        || blocked.is_some()
        || state == "PendingApproval"
        || (state == "AwaitingReview" && awaiting == Some("question"))
    {
        return "input_needed";
    }
    if execution {
        return "running";
    }
    if residual > 0 {
        return "remaining_work";
    }
    match state {
        "Created" | "Queued" => "queued",
        "Starting" | "Running" | "Finalizing" => "unknown",
        "AwaitingReview" => "review_needed",
        "Done" | "Failed" | "Discarded" => "ended",
        _ => "unknown",
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn does_not_infer_end_from_missing_process_or_result() {
        assert_eq!(classify("Running", None, None, false, 0, 0), "unknown");
        assert_eq!(
            classify("AwaitingReview", None, None, false, 0, 1),
            "remaining_work"
        );
        assert_eq!(classify("Done", None, None, false, 0, 1), "remaining_work");
        assert_eq!(classify("Running", None, None, true, 1, 0), "input_needed");
        assert_eq!(
            classify("AwaitingReview", None, None, false, 0, 0),
            "review_needed"
        );
    }
    #[test]
    fn contract_matches_question_admission_and_cost_support() {
        for agent in ["claude", "codex", "agy", "custom"] {
            assert_eq!(
                question_runtime(agent),
                crate::convo::interaction::runtime_for_agent(agent)
            );
            let c = capabilities(agent, "conversation", question_runtime(agent), false, false);
            assert_eq!(
                c.features
                    .iter()
                    .find(|f| f.key == "questions")
                    .unwrap()
                    .state,
                if question_runtime(agent).is_some() {
                    "supported"
                } else {
                    "unsupported"
                }
            );
            assert_eq!(
                c.features.iter().find(|f| f.key == "cost").unwrap().state,
                if agent == "claude" {
                    "supported"
                } else {
                    "unsupported"
                }
            );
        }
    }
}
