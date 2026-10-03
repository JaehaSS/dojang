use sqlx::SqlitePool;

pub(crate) const FOLLOWUP_OBSERVATION_STARTED: &str = "followup_observation_started";
pub(crate) const USER_FOLLOWUP_INPUT_OBSERVED: &str = "user_followup_input_observed";

const INSERT_OBSERVATION_EVENT: &str = "\
INSERT INTO task_events (task_id, ts, kind, detail) VALUES (?, ?, ?, NULL) \
ON CONFLICT(task_id, kind) \
WHERE kind IN ('followup_observation_started', 'user_followup_input_observed') \
DO NOTHING";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConversationInputOrigin {
    InitialTask,
    UserMessage,
    AnnotationResend,
    RemoteReviewRetry,
}

impl ConversationInputOrigin {
    pub(crate) fn is_initial(self) -> bool {
        self == Self::InitialTask
    }
}

/// 파일 메모리가 실제 내용(사용자·레포 `Capped` 중 본문이 있는 쪽)과 함께 주입됐다는 마커.
/// "메모리 안내 후 재설명 없음" 인사이트(`insights::outcomes`)가 이 이벤트의 존재로 대상
/// 작업의 자격을 가른다 — 없으면 애초에 메모리를 준 적이 없으니 재설명 여부를 셀 수 없다.
pub(crate) async fn insert_observation_start(
    pool: &SqlitePool,
    task_id: i64,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query(INSERT_OBSERVATION_EVENT)
        .bind(task_id)
        .bind(now)
        .bind(FOLLOWUP_OBSERVATION_STARTED)
        .execute(pool)
        .await?;
    Ok(())
}

pub(crate) async fn record_user_followup(
    pool: &SqlitePool,
    task_id: i64,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query(INSERT_OBSERVATION_EVENT)
        .bind(task_id)
        .bind(now)
        .bind(USER_FOLLOWUP_INPUT_OBSERVED)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
pub(crate) async fn record_conversation_input(
    pool: &SqlitePool,
    task_id: i64,
    origin: ConversationInputOrigin,
    now: i64,
) -> anyhow::Result<()> {
    if origin.is_initial() {
        return Ok(());
    }
    record_user_followup(pool, task_id, now).await
}

pub(crate) async fn record_followup_before_forward<F>(
    pool: &SqlitePool,
    task_id: i64,
    now: i64,
    forward: F,
) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    record_user_followup(pool, task_id, now).await?;
    forward()
}

#[cfg(test)]
mod tests;
