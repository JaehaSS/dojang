//! Opt-in, human-owned result review. Decisions never merge or execute a task.
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Criterion {
    pub text: String,
    pub checked: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Round {
    pub id: String,
    pub task_id: i64,
    pub source_id: String,
    pub result_revision: i64,
    pub completion_epoch: i64,
    pub revision: i64,
    pub state: String,
    pub criteria: Vec<Criterion>,
    pub active: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Decision {
    pub action: String,
    pub reason: String,
    pub round_id: String,
    pub created_at: i64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Delivery {
    pub mutation_id: String,
    pub state: String,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub source_id: String,
    pub result_revision: i64,
    pub completion_epoch: i64,
    pub active: Option<Round>,
    pub rounds: Vec<Round>,
    pub decisions: Vec<Decision>,
    pub deliveries: Vec<Delivery>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Input {
    pub source_id: String,
    pub result_revision: i64,
    pub round_id: Option<String>,
    pub expected_revision: i64,
    pub mutation_id: String,
    pub action: String,
    pub reason: String,
    pub criteria: Vec<String>,
    pub checked: Vec<bool>,
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn delivery_owner() -> &'static str {
    static OWNER: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    OWNER.get_or_init(|| {
        format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        )
    })
}

pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    for sql in [
        "CREATE TABLE IF NOT EXISTS result_review_rounds(id TEXT PRIMARY KEY,task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,source_id TEXT NOT NULL,result_revision INTEGER NOT NULL,completion_epoch INTEGER NOT NULL,revision INTEGER NOT NULL DEFAULT 1,state TEXT NOT NULL,criteria TEXT NOT NULL,active INTEGER NOT NULL,created_at INTEGER NOT NULL)",
        "CREATE UNIQUE INDEX IF NOT EXISTS result_review_one_active ON result_review_rounds(source_id,task_id) WHERE active=1",
        "CREATE TABLE IF NOT EXISTS result_review_decisions(mutation_id TEXT PRIMARY KEY,payload TEXT NOT NULL,round_id TEXT NOT NULL REFERENCES result_review_rounds(id) ON DELETE CASCADE,action TEXT NOT NULL,reason TEXT NOT NULL,created_at INTEGER NOT NULL)",
        "CREATE TABLE IF NOT EXISTS result_review_outbox(mutation_id TEXT PRIMARY KEY REFERENCES result_review_decisions(mutation_id) ON DELETE CASCADE,task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,message TEXT NOT NULL,state TEXT NOT NULL,error TEXT,dispatch_owner TEXT)",
    ] { sqlx::query(sql).execute(&mut *tx).await?; }
    crate::db::add_column_if_missing(&mut *tx, "result_review_outbox", "dispatch_owner TEXT")
        .await?;
    tx.commit().await?;
    Ok(())
}
fn round(row: &sqlx::sqlite::SqliteRow) -> Result<Round, String> {
    Ok(Round {
        id: row.get("id"),
        task_id: row.get("task_id"),
        source_id: row.get("source_id"),
        result_revision: row.get("result_revision"),
        completion_epoch: row.get("completion_epoch"),
        revision: row.get("revision"),
        state: row.get("state"),
        criteria: serde_json::from_str(row.get("criteria")).map_err(err)?,
        active: row.get("active"),
    })
}

pub async fn snapshot(pool: &SqlitePool, task_id: i64) -> Result<Snapshot, String> {
    let result = crate::task_results::snapshot(pool, task_id).await?;
    let mut tx = pool.begin().await.map_err(err)?;
    crate::task_results::validate_snapshot_tx(&mut tx, task_id, &result).await?;
    let state: String = sqlx::query_scalar("SELECT state FROM tasks WHERE id=?")
        .bind(task_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(err)?;
    let active = sqlx::query(
        "SELECT * FROM result_review_rounds WHERE source_id=? AND task_id=? AND active=1",
    )
    .bind(&result.source_id)
    .bind(task_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(err)?;
    if let Some(row) = active {
        let current = round(&row)?;
        if matches!(state.as_str(), "Done" | "Discarded") {
            sqlx::query("UPDATE result_review_rounds SET state='closed',active=0,revision=revision+1 WHERE id=?").bind(&current.id).execute(&mut *tx).await.map_err(err)?;
        } else if current.result_revision != result.result_revision
            && current.state != "changes_requested"
        {
            sqlx::query("UPDATE result_review_rounds SET active=0,revision=revision+1 WHERE id=?")
                .bind(&current.id)
                .execute(&mut *tx)
                .await
                .map_err(err)?;
            let criteria: Vec<_> = current
                .criteria
                .into_iter()
                .map(|c| Criterion {
                    text: c.text,
                    checked: false,
                })
                .collect();
            sqlx::query("INSERT INTO result_review_rounds(id,task_id,source_id,result_revision,completion_epoch,state,criteria,active,created_at) VALUES(?,?,?,?,?,'pending',?,1,?)")
                .bind(crate::preview_bridge::random_hex_id()?).bind(task_id).bind(&result.source_id).bind(result.result_revision).bind(result.completion_epoch)
                .bind(serde_json::to_string(&criteria).map_err(err)?).bind(now()).execute(&mut *tx).await.map_err(err)?;
        }
    }
    tx.commit().await.map_err(err)?;
    // A dispatch whose caller disappeared is reconciled from durable admission evidence.
    sqlx::query("UPDATE result_review_outbox SET state='sent',error=NULL WHERE task_id=? AND state IN ('dispatching','delivery_unknown') AND EXISTS(SELECT 1 FROM convo_events e WHERE e.task_id=result_review_outbox.task_id AND json_valid(e.event) AND json_extract(e.event,'$.receipt_request_id')='review:'||result_review_outbox.mutation_id)")
        .bind(task_id).execute(pool).await.map_err(err)?;
    sqlx::query("UPDATE result_review_outbox SET state='delivery_unknown',error='이전 프로세스의 대화 접수 여부를 확인하지 못했습니다' WHERE task_id=? AND state='dispatching' AND (dispatch_owner IS NULL OR dispatch_owner<>?)")
        .bind(task_id).bind(delivery_owner()).execute(pool).await.map_err(err)?;
    let rows = sqlx::query(
        "SELECT * FROM result_review_rounds WHERE task_id=? AND source_id=? ORDER BY created_at,id",
    )
    .bind(task_id)
    .bind(&result.source_id)
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let rounds = rows.iter().map(round).collect::<Result<Vec<_>, _>>()?;
    let decisions=sqlx::query("SELECT d.* FROM result_review_decisions d JOIN result_review_rounds r ON r.id=d.round_id WHERE r.task_id=? AND r.source_id=? ORDER BY d.created_at,d.rowid")
        .bind(task_id).bind(&result.source_id).fetch_all(pool).await.map_err(err)?.iter().map(|r| Decision{ action:r.get("action"),reason:r.get("reason"),round_id:r.get("round_id"),created_at:r.get("created_at") }).collect();
    let deliveries=sqlx::query("SELECT o.* FROM result_review_outbox o JOIN result_review_decisions d ON d.mutation_id=o.mutation_id JOIN result_review_rounds r ON r.id=d.round_id WHERE o.task_id=? AND r.source_id=? ORDER BY d.created_at")
        .bind(task_id).bind(&result.source_id).fetch_all(pool).await.map_err(err)?.iter().map(|r| Delivery{mutation_id:r.get("mutation_id"),state:r.get("state"),error:r.get("error")}).collect();
    Ok(Snapshot {
        source_id: result.source_id,
        result_revision: result.result_revision,
        completion_epoch: result.completion_epoch,
        active: rounds.iter().find(|r| r.active).cloned(),
        rounds,
        decisions,
        deliveries,
    })
}

pub async fn decide(pool: &SqlitePool, task_id: i64, input: &Input) -> Result<Snapshot, String> {
    if input.mutation_id.is_empty()
        || input.mutation_id.len() > 128
        || input.reason.trim().is_empty()
        || input.reason.len() > 8000
        || input.criteria.len() > 32
        || input
            .criteria
            .iter()
            .any(|c| c.trim().is_empty() || c.len() > 2000)
    {
        return Err("검토 사유와 기준을 확인하세요".into());
    }
    let payload = serde_json::to_string(&(task_id, input)).map_err(err)?;
    let existing: Option<String> =
        sqlx::query_scalar("SELECT payload FROM result_review_decisions WHERE mutation_id=?")
            .bind(&input.mutation_id)
            .fetch_optional(pool)
            .await
            .map_err(err)?;
    if let Some(existing) = existing {
        if existing != payload {
            return Err("같은 요청 번호에 다른 검토 내용을 보낼 수 없습니다".into());
        }
        return snapshot(pool, task_id).await;
    }
    let view = snapshot(pool, task_id).await?;
    let result = crate::task_results::snapshot(pool, task_id).await?;
    if input.source_id != view.source_id
        || input.source_id != result.source_id
        || input.result_revision != result.result_revision
    {
        return Err("결과 버전이 바뀌었습니다. 새로고침하세요".into());
    }
    let task = crate::db::get_task(pool, task_id)
        .await
        .map_err(err)?
        .ok_or("작업을 찾을 수 없습니다")?;
    let mut tx = pool.begin().await.map_err(err)?;
    crate::task_results::validate_snapshot_tx(&mut tx, task_id, &result).await?;
    let state: String = sqlx::query_scalar("SELECT state FROM tasks WHERE id=?")
        .bind(task_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(err)?;
    if input.action != "cancel" && state != "AwaitingReview" {
        return Err("완료된 검토 대기 결과에서만 결정할 수 있습니다".into());
    }
    let stored = sqlx::query(
        "SELECT * FROM result_review_rounds WHERE source_id=? AND task_id=? AND active=1",
    )
    .bind(&input.source_id)
    .bind(task_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(err)?;
    let id;
    if input.action == "start" {
        if stored.is_some() || input.round_id.is_some() || input.expected_revision != 0 {
            return Err("이미 검토가 있거나 오래된 요청입니다".into());
        }
        id = crate::preview_bridge::random_hex_id()?;
        let mut texts = input.criteria.clone();
        if let Some(contract) = task.goal_contract.as_deref() {
            for criterion in &contract.acceptance {
                if !texts.contains(criterion) {
                    texts.push(criterion.clone());
                }
            }
        }
        let criteria: Vec<_> = texts
            .into_iter()
            .map(|text| Criterion {
                text,
                checked: false,
            })
            .collect();
        sqlx::query("INSERT INTO result_review_rounds(id,task_id,source_id,result_revision,completion_epoch,state,criteria,active,created_at) VALUES(?,?,?,?,?,'pending',?,1,?)")
            .bind(&id).bind(task_id).bind(&input.source_id).bind(result.result_revision).bind(result.completion_epoch).bind(serde_json::to_string(&criteria).map_err(err)?).bind(now()).execute(&mut *tx).await.map_err(err)?;
    } else {
        let current = round(&stored.ok_or("활성 검토가 없습니다")?)?;
        if input.round_id.as_deref() != Some(&current.id)
            || input.expected_revision != current.revision
        {
            return Err("검토가 다른 창에서 변경되었습니다".into());
        }
        id = current.id.clone();
        let mut criteria = current.criteria;
        let next = match input.action.as_str() {
            "cancel" => "cancelled",
            "accept" if current.state == "pending" => {
                if result.freshness != "current"
                    || result.fingerprint.is_none()
                    || current.result_revision != result.result_revision
                    || input.checked.len() != criteria.len()
                    || input.checked.iter().any(|v| !v)
                {
                    return Err("현재 결과와 모든 완료 기준을 직접 확인해야 합니다".into());
                }
                for c in &mut criteria {
                    c.checked = true;
                }
                "accepted"
            }
            "request_changes"
                if current.state == "pending" || current.state == "changes_requested" =>
            {
                "changes_requested"
            }
            "resubmit"
                if current.state == "changes_requested"
                    && result.completion_epoch > current.completion_epoch =>
            {
                "pending"
            }
            _ => return Err("이 상태에서는 요청한 검토 전환을 할 수 없습니다".into()),
        };
        if input.action == "resubmit" {
            for c in &mut criteria {
                c.checked = false;
            }
        }
        let changed=sqlx::query("UPDATE result_review_rounds SET state=?,active=?,revision=revision+1,result_revision=?,completion_epoch=?,criteria=? WHERE id=? AND revision=? AND active=1")
            .bind(next).bind(next!="cancelled").bind(result.result_revision).bind(result.completion_epoch).bind(serde_json::to_string(&criteria).map_err(err)?).bind(&id).bind(input.expected_revision).execute(&mut *tx).await.map_err(err)?;
        if changed.rows_affected() != 1 {
            return Err("검토 revision 충돌".into());
        }
    }
    sqlx::query("INSERT INTO result_review_decisions(mutation_id,payload,round_id,action,reason,created_at) VALUES(?,?,?,?,?,?)")
        .bind(&input.mutation_id).bind(payload).bind(&id).bind(&input.action).bind(&input.reason).bind(now()).execute(&mut *tx).await.map_err(err)?;
    if input.action == "request_changes" {
        let message=format!("다음 사람 검토 의견에 따라 수정해 주세요. 기존 작업 권한과 중지 조건은 그대로입니다.\n\n{}",input.reason);
        sqlx::query("INSERT INTO result_review_outbox(mutation_id,task_id,message,state) VALUES(?,?,?,'queued')")
            .bind(&input.mutation_id).bind(task_id).bind(message).execute(&mut *tx).await.map_err(err)?;
    }
    tx.commit().await.map_err(err)?;
    snapshot(pool, task_id).await
}

/// Claim once before calling conversation admission. Unknown deliveries are never replayed.
pub async fn claim_delivery(
    pool: &SqlitePool,
    task_id: i64,
    mutation_id: &str,
) -> Result<Option<String>, String> {
    let mut tx = pool.begin().await.map_err(err)?;
    let changed=sqlx::query("UPDATE result_review_outbox SET state='dispatching',dispatch_owner=? WHERE task_id=? AND mutation_id=? AND state='queued' AND EXISTS(SELECT 1 FROM result_review_decisions d JOIN result_review_rounds r ON r.id=d.round_id WHERE d.mutation_id=result_review_outbox.mutation_id AND r.active=1 AND r.state='changes_requested' AND r.source_id=(SELECT value FROM settings WHERE key='notification_source_id'))")
        .bind(delivery_owner()).bind(task_id).bind(mutation_id).execute(&mut *tx).await.map_err(err)?;
    let message = if changed.rows_affected() == 1 {
        Some(
            sqlx::query_scalar("SELECT message FROM result_review_outbox WHERE mutation_id=?")
                .bind(mutation_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(err)?,
        )
    } else {
        None
    };
    tx.commit().await.map_err(err)?;
    Ok(message)
}
pub async fn finish_delivery(
    pool: &SqlitePool,
    mutation_id: &str,
    result: &Result<(), String>,
) -> Result<(), String> {
    sqlx::query("UPDATE result_review_outbox SET state=?,error=? WHERE mutation_id=? AND state='dispatching'")
        .bind(if result.is_ok(){"sent"}else{"delivery_unknown"}).bind(result.as_ref().err()).bind(mutation_id).execute(pool).await.map_err(err)?;
    Ok(())
}
pub async fn guard_apply(pool: &SqlitePool, task_id: i64) -> Result<(), String> {
    let view = snapshot(pool, task_id).await?;
    if let Some(active) = view.active {
        let result = crate::task_results::snapshot(pool, task_id).await?;
        if active.state != "accepted"
            || active.result_revision != result.result_revision
            || result.freshness != "current"
            || result.fingerprint.is_none()
        {
            return Err("현재 결과의 사람 검토를 완료하거나 검토를 취소하세요".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn fixture(name: &str) -> (SqlitePool, i64, std::path::PathBuf) {
        let root = crate::testtmp::dir().join(format!("human-review-{name}"));
        std::fs::create_dir_all(&root).unwrap();
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        for args in [
            vec!["init", "-b", "main"],
            vec!["config", "user.email", "test@example.invalid"],
            vec!["config", "user.name", "Test"],
        ] {
            assert!(std::process::Command::new("git")
                .current_dir(&repo)
                .args(args)
                .output()
                .unwrap()
                .status
                .success());
        }
        std::fs::write(repo.join("file.txt"), "first").unwrap();
        assert!(std::process::Command::new("git")
            .current_dir(&repo)
            .args(["add", "."])
            .output()
            .unwrap()
            .status
            .success());
        assert!(std::process::Command::new("git")
            .current_dir(&repo)
            .args(["commit", "-m", "initial"])
            .output()
            .unwrap()
            .status
            .success());
        let pool = crate::db::init_pool(root.join("test.sqlite").to_str().unwrap())
            .await
            .unwrap();
        let id = crate::db::insert_task(
            &pool,
            repo.to_str().unwrap(),
            "main",
            "main",
            repo.to_str().unwrap(),
            "review",
            Some("codex"),
            None,
            "conversation",
            1,
        )
        .await
        .unwrap();
        crate::db::update_state(&pool, id, "AwaitingReview", 2)
            .await
            .unwrap();
        (pool, id, repo)
    }
    fn input(view: &Snapshot, action: &str, key: &str) -> Input {
        Input {
            source_id: view.source_id.clone(),
            result_revision: view.result_revision,
            round_id: view.active.as_ref().map(|r| r.id.clone()),
            expected_revision: view.active.as_ref().map_or(0, |r| r.revision),
            mutation_id: key.into(),
            action: action.into(),
            reason: "직접 확인했습니다".into(),
            criteria: vec!["동작 확인".into()],
            checked: vec![true],
        }
    }
    #[tokio::test]
    async fn acceptance_is_versioned_and_does_not_merge() {
        let (pool, id, repo) = fixture("accept").await;
        let v = snapshot(&pool, id).await.unwrap();
        let start = input(&v, "start", "start");
        let v = decide(&pool, id, &start).await.unwrap();
        assert!(guard_apply(&pool, id).await.is_err());
        assert_eq!(decide(&pool, id, &start).await.unwrap().rounds.len(), 1);
        let accepted = decide(&pool, id, &input(&v, "accept", "accept"))
            .await
            .unwrap();
        assert_eq!(accepted.active.unwrap().state, "accepted");
        guard_apply(&pool, id).await.unwrap();
        assert_eq!(
            crate::db::get_task(&pool, id).await.unwrap().unwrap().state,
            "AwaitingReview"
        );
        std::fs::write(repo.join("file.txt"), "changed").unwrap();
        assert!(guard_apply(&pool, id).await.is_err());
        let next = snapshot(&pool, id).await.unwrap();
        assert_eq!(next.active.unwrap().state, "pending");
        assert_eq!(next.rounds.len(), 2);
        assert!(decide(&pool, id, &input(&v, "accept", "stale"))
            .await
            .is_err());
    }
    #[tokio::test]
    async fn requests_have_one_dispatch_and_need_a_later_completed_round() {
        let (pool, id, _) = fixture("delivery").await;
        let v = snapshot(&pool, id).await.unwrap();
        let v = decide(&pool, id, &input(&v, "start", "start"))
            .await
            .unwrap();
        let request = input(&v, "request_changes", "request");
        let requested = decide(&pool, id, &request).await.unwrap();
        assert!(claim_delivery(&pool, id, "request")
            .await
            .unwrap()
            .is_some());
        finish_delivery(&pool, "request", &Err("timeout".into()))
            .await
            .unwrap();
        assert!(claim_delivery(&pool, id, "request")
            .await
            .unwrap()
            .is_none());
        assert!(
            decide(&pool, id, &input(&requested, "resubmit", "too-soon"))
                .await
                .is_err()
        );
        sqlx::query("INSERT INTO convo_events(task_id,ts,event) VALUES(?,3,?)")
            .bind(id)
            .bind(r#"{"kind":"user","receipt_request_id":"review:request"}"#)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            snapshot(&pool, id).await.unwrap().deliveries[0].state,
            "sent"
        );
        crate::db::update_state(&pool, id, "Running", 4)
            .await
            .unwrap();
        crate::db::update_state(&pool, id, "AwaitingReview", 4)
            .await
            .unwrap();
        let later = snapshot(&pool, id).await.unwrap();
        let resubmitted = decide(&pool, id, &input(&later, "resubmit", "again"))
            .await
            .unwrap();
        assert_eq!(resubmitted.active.unwrap().state, "pending");
    }
    #[tokio::test]
    async fn outbox_and_decision_commit_together_and_checks_are_manual() {
        let (pool, id, _) = fixture("atomic").await;
        let v = snapshot(&pool, id).await.unwrap();
        let v = decide(&pool, id, &input(&v, "start", "start"))
            .await
            .unwrap();
        let mut unchecked = input(&v, "accept", "unchecked");
        unchecked.checked = vec![false];
        assert!(decide(&pool, id, &unchecked).await.is_err());
        sqlx::query("CREATE TRIGGER fail_outbox BEFORE INSERT ON result_review_outbox BEGIN SELECT RAISE(ABORT,'outbox unavailable'); END").execute(&pool).await.unwrap();
        assert!(decide(&pool, id, &input(&v, "request_changes", "failed"))
            .await
            .is_err());
        let after = snapshot(&pool, id).await.unwrap();
        assert_eq!(after.active.as_ref().unwrap().state, "pending");
        assert_eq!(after.decisions.len(), 1);
        let mut wrong = input(&after, "accept", "source");
        wrong.source_id = "old".into();
        assert!(decide(&pool, id, &wrong).await.is_err());
        let cancelled = decide(&pool, id, &input(&after, "cancel", "cancel"))
            .await
            .unwrap();
        assert!(cancelled.active.is_none());
        guard_apply(&pool, id).await.unwrap();
    }
    #[tokio::test]
    async fn abandoned_dispatch_is_unknown_but_cancelled_queued_work_cannot_send() {
        let (pool, id, _) = fixture("restart").await;
        let v = snapshot(&pool, id).await.unwrap();
        let v = decide(&pool, id, &input(&v, "start", "start"))
            .await
            .unwrap();
        let v = decide(&pool, id, &input(&v, "request_changes", "request"))
            .await
            .unwrap();
        assert!(claim_delivery(&pool, id, "request")
            .await
            .unwrap()
            .is_some());
        sqlx::query("UPDATE result_review_outbox SET dispatch_owner='previous-process' WHERE mutation_id='request'").execute(&pool).await.unwrap();
        let restored = snapshot(&pool, id).await.unwrap();
        assert_eq!(restored.deliveries[0].state, "delivery_unknown");
        assert!(claim_delivery(&pool, id, "request")
            .await
            .unwrap()
            .is_none());
        let requested = decide(&pool, id, &input(&v, "request_changes", "retry"))
            .await
            .unwrap();
        decide(&pool, id, &input(&requested, "cancel", "cancel"))
            .await
            .unwrap();
        assert!(claim_delivery(&pool, id, "retry").await.unwrap().is_none());
        guard_apply(&pool, id).await.unwrap();
    }

    #[tokio::test]
    async fn latest_failed_receipt_overrides_pass_and_history_becomes_stale() {
        let (pool, id, repo) = fixture("receipts").await;
        let spec = crate::verify::detect_spec(&repo);
        let observation = crate::task_results::fingerprint_with_base(&repo, &spec, Some("main"));
        let mut report = crate::verify::VerifyReport {
            spec,
            build: Some(crate::verify::CheckResult {
                command: "true".into(),
                exit_code: 0,
                tail: "verified".into(),
            }),
            test: None,
            summary: None,
            ready: true,
            checks: vec![],
            warnings: vec![],
        };
        let mut tx = pool.begin().await.unwrap();
        crate::task_results::persist_verify_receipt_tx(
            &mut tx,
            id,
            &report,
            &repo,
            &observation,
            &observation,
            "AwaitingReview",
            10,
            10,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        assert!(crate::task_results::refresh_evidence(&pool, id)
            .await
            .unwrap());
        report.ready = false;
        report.build.as_mut().unwrap().exit_code = 1;
        let mut tx = pool.begin().await.unwrap();
        crate::task_results::persist_verify_receipt_tx(
            &mut tx,
            id,
            &report,
            &repo,
            &observation,
            &observation,
            "AwaitingReview",
            10,
            10,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        assert!(!crate::task_results::refresh_evidence(&pool, id)
            .await
            .unwrap());
        let current = crate::task_results::snapshot(&pool, id).await.unwrap();
        assert_eq!(current.receipts.len(), 2);
        assert_ne!(
            current.receipts[0].verify_run_id,
            current.receipts[1].verify_run_id
        );
        assert_eq!(current.receipts[0].outcome, "failed");
        assert_eq!(
            current.receipts[0].report.as_ref().unwrap()["build"]["tail"],
            "verified"
        );
        std::fs::write(repo.join("file.txt"), "new version").unwrap();
        let later = crate::task_results::snapshot(&pool, id).await.unwrap();
        assert!(later
            .receipts
            .iter()
            .all(|receipt| receipt.freshness == "stale"));
    }

    #[tokio::test]
    async fn runner_finalization_cannot_bypass_pending_human_review() {
        let (pool, id, repo) = fixture("runner-gate").await;
        let view = snapshot(&pool, id).await.unwrap();
        decide(&pool, id, &input(&view, "start", "start"))
            .await
            .unwrap();
        let config = crate::runner::config::RunnerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            repository_roots: vec![repo.canonicalize().unwrap()],
            max_concurrent_tasks: 1,
            execution_policy: crate::runner::config::ExecutionPolicy::RequireApproval,
            pairing_token_file: repo.join("unused"),
        };
        let error = crate::runner::finalize_task(
            &config,
            &pool,
            &crate::runner::worktree_lock::WorktreeLocks::default(),
            id,
            true,
            10,
        )
        .await
        .unwrap_err();
        assert!(error.contains("사람 검토"), "{error}");
        assert_eq!(
            crate::db::get_task(&pool, id).await.unwrap().unwrap().state,
            "AwaitingReview"
        );
        assert!(repo.join("file.txt").exists());
    }

    #[tokio::test]
    async fn deleted_reference_remains_visible_with_its_original_version() {
        let (pool, id, repo) = fixture("reference-history").await;
        let result = crate::task_results::snapshot(&pool, id).await.unwrap();
        let result = crate::task_results::add_reference(
            &pool,
            id,
            crate::task_results::ReferenceInput {
                expected_source_id: result.source_id,
                expected_revision: result.result_revision,
                kind: "report".into(),
                relative_path: "file.txt".into(),
            },
        )
        .await
        .unwrap();
        let version = result.result_revision;
        assert_eq!(result.references[0].availability, "available");
        std::fs::remove_file(repo.join("file.txt")).unwrap();
        let next = crate::task_results::snapshot(&pool, id).await.unwrap();
        assert!(next.result_revision > version);
        assert_eq!(next.references[0].availability, "missing");
        assert_eq!(next.references[0].result_revision, version);
    }
}
