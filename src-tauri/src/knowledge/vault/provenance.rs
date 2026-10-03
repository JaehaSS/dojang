use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

use super::bindings::ProjectBinding;
use super::catalog::identifier;

const UNKNOWN: &str = "unknown";
const PRIVATE: &str = "private-data";
const ALLOWED: &str = "capture_allowed";
const MAX_TERMINAL_BYTES: usize = 40 * 1024;

#[derive(Clone, Debug)]
pub struct DraftPolicy {
    pub input_mode: String,
    pub sources: Vec<DraftPolicySource>,
}

#[derive(Clone, Debug)]
pub struct DraftPolicySource {
    pub document_id: String,
    pub revision_id: String,
    pub revision_hash: String,
    pub(crate) grant_fingerprint: String,
    pub scope: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputOrigin {
    UserMessage,
    DocumentRevision,
    ToolResult,
    PriorConversation,
    Capsule,
    SkillExpanded,
}

impl InputOrigin {
    fn as_str(self) -> &'static str {
        match self {
            Self::UserMessage => "user_message",
            Self::DocumentRevision => "document_revision",
            Self::ToolResult => "tool_result",
            Self::PriorConversation => "prior_conversation",
            Self::Capsule => "capsule",
            Self::SkillExpanded => "skill_expanded",
        }
    }
}

pub async fn start_attempt(
    pool: &SqlitePool,
    task_id: i64,
    vault_id: &str,
    binding: &ProjectBinding,
    provider: &str,
    client_ref: Option<&str>,
    now: i64,
) -> anyhow::Result<String> {
    let id = identifier("attempt")?;
    let vault: (String, i64, i64) = sqlx::query_as("SELECT canonical_root, root_device, root_inode FROM vaults WHERE id = ? AND enabled = 1 AND writable = 1").bind(vault_id).fetch_one(pool).await?;
    sqlx::query("INSERT INTO vault_task_attempts (id, task_id, vault_id, vault_root, vault_device, vault_inode, binding_id, binding_epoch, provider, client_ref, provenance_complete, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?)")
        .bind(&id).bind(task_id).bind(vault_id).bind(vault.0).bind(vault.1).bind(vault.2).bind(&binding.id).bind(&binding.epoch).bind(provider).bind(client_ref).bind(now).execute(pool).await?;
    Ok(id)
}

pub async fn active_writable_vault(pool: &SqlitePool) -> anyhow::Result<Option<String>> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM vaults WHERE enabled = 1 AND writable = 1 ORDER BY registered_at",
    )
    .fetch_all(pool)
    .await?;
    if ids.len() > 1 {
        anyhow::bail!("more than one writable vault is active")
    }
    Ok(ids.into_iter().next())
}

pub async fn save_draft_policy(
    pool: &SqlitePool,
    binding: &ProjectBinding,
    client_ref: &str,
    query: &str,
    input_mode: &str,
    revision_ids: &[String],
    now: i64,
) -> anyhow::Result<()> {
    if !matches!(input_mode, "default" | "task_only" | "private_attachment") {
        anyhow::bail!("invalid vault draft policy")
    }
    if revision_ids.len() > 5 || (input_mode == "private_attachment") == revision_ids.is_empty() {
        anyhow::bail!("private attachments require 1 to 5 selected revisions")
    }
    let unique = revision_ids
        .iter()
        .collect::<std::collections::HashSet<_>>();
    if unique.len() != revision_ids.len() {
        anyhow::bail!("private attachments must be unique")
    }
    let mut sources = Vec::with_capacity(revision_ids.len());
    for revision_id in revision_ids {
        let row = sqlx::query("SELECT d.id AS document_id, r.sha256 FROM vault_revisions r JOIN vault_documents d ON d.id = r.document_id WHERE r.id = ? AND d.current_revision = r.id AND d.state = 'active'")
            .bind(revision_id).fetch_one(pool).await?;
        let scope = super::scope_for_sources(pool, std::slice::from_ref(revision_id))
            .await?
            .ok_or_else(|| anyhow::anyhow!("selected attachment has no current scope"))?;
        if !matches!(scope, super::Scope::PrivateData) {
            anyhow::bail!("private attachments must remain private")
        }
        let grants: Vec<String> = sqlx::query_scalar("SELECT id FROM vault_grants WHERE revision_id = ? AND revoked_at IS NULL ORDER BY id")
            .bind(revision_id).fetch_all(pool).await?;
        sources.push(DraftPolicySource {
            document_id: row.try_get("document_id")?,
            revision_id: revision_id.clone(),
            revision_hash: row.try_get("sha256")?,
            grant_fingerprint: grants.join(":"),
            scope: PRIVATE.into(),
        });
    }
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE vault_draft_policies SET state = 'invalidated' WHERE binding_id = ? AND binding_epoch = ? AND client_ref = ? AND state = 'pending'")
        .bind(&binding.id).bind(&binding.epoch).bind(client_ref).execute(&mut *tx).await?;
    let id = identifier("draft-policy")?;
    sqlx::query("INSERT INTO vault_draft_policies (id, binding_id, binding_epoch, client_ref, query_hash, input_mode, state, created_at) VALUES (?, ?, ?, ?, ?, ?, 'pending', ?)")
        .bind(&id).bind(&binding.id).bind(&binding.epoch).bind(client_ref).bind(hash(query)).bind(input_mode).bind(now).execute(&mut *tx).await?;
    for source in sources {
        sqlx::query("INSERT INTO vault_draft_policy_sources (policy_id, document_id, revision_id, revision_hash, grant_fingerprint, scope) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(&id).bind(source.document_id).bind(source.revision_id).bind(source.revision_hash).bind(source.grant_fingerprint).bind(source.scope).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn consume_draft_policy(
    pool: &SqlitePool,
    binding: &ProjectBinding,
    client_ref: &str,
    query: &str,
    task_id: i64,
) -> anyhow::Result<Option<DraftPolicy>> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query("SELECT id, query_hash, input_mode FROM vault_draft_policies WHERE binding_id = ? AND binding_epoch = ? AND client_ref = ? AND state = 'pending'")
        .bind(&binding.id).bind(&binding.epoch).bind(client_ref).fetch_optional(&mut *tx).await?;
    let Some(row) = row else { return Ok(None) };
    let id: String = row.try_get("id")?;
    if row.try_get::<String, _>("query_hash")? != hash(query) {
        sqlx::query("UPDATE vault_draft_policies SET state = 'invalidated' WHERE id = ?")
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(None);
    }
    let sources = sqlx::query("SELECT document_id, revision_id, revision_hash, grant_fingerprint, scope FROM vault_draft_policy_sources WHERE policy_id = ? ORDER BY revision_id")
        .bind(&id).fetch_all(&mut *tx).await?.into_iter().map(|row| Ok(DraftPolicySource {
            document_id: row.try_get("document_id")?, revision_id: row.try_get("revision_id")?, revision_hash: row.try_get("revision_hash")?, grant_fingerprint: row.try_get("grant_fingerprint")?, scope: row.try_get("scope")?,
        })).collect::<anyhow::Result<Vec<_>>>()?;
    let changed = sqlx::query("UPDATE vault_draft_policies SET state = 'consumed', consumed_task_id = ? WHERE id = ? AND state = 'pending'")
        .bind(task_id).bind(&id).execute(&mut *tx).await?.rows_affected();
    if changed != 1 {
        return Ok(None);
    }
    tx.commit().await?;
    Ok(Some(DraftPolicy {
        input_mode: row.try_get("input_mode")?,
        sources,
    }))
}

pub async fn restrictive_draft_policy_seen(
    pool: &SqlitePool,
    binding: &ProjectBinding,
    client_ref: &str,
) -> anyhow::Result<bool> {
    let input_mode: Option<String> = sqlx::query_scalar(
        "SELECT input_mode FROM vault_draft_policies \
         WHERE binding_id = ? AND binding_epoch = ? AND client_ref = ? \
         ORDER BY rowid DESC LIMIT 1",
    )
    .bind(&binding.id)
    .bind(&binding.epoch)
    .bind(client_ref)
    .fetch_optional(pool)
    .await?;
    Ok(matches!(input_mode.as_deref(), Some("task_only" | "private_attachment")))
}

pub async fn pending_restrictive_draft_policy(
    pool: &SqlitePool,
    client_ref: &str,
) -> anyhow::Result<bool> {
    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM vault_draft_policies WHERE client_ref = ? AND state = 'pending' \
         AND input_mode IN ('task_only', 'private_attachment') LIMIT 1",
    )
    .bind(client_ref)
    .fetch_optional(pool)
    .await?;
    Ok(exists.is_some())
}

pub async fn draft_policy_current(pool: &SqlitePool, policy: &DraftPolicy) -> anyhow::Result<bool> {
    for source in &policy.sources {
        let current: Option<String> = sqlx::query_scalar("SELECT r.sha256 FROM vault_revisions r JOIN vault_documents d ON d.id = r.document_id WHERE d.id = ? AND d.current_revision = ? AND r.id = ? AND d.state = 'active'")
            .bind(&source.document_id).bind(&source.revision_id).bind(&source.revision_id).fetch_optional(pool).await?;
        if current.as_deref() != Some(source.revision_hash.as_str())
            || !matches!(
                super::scope_for_sources(pool, std::slice::from_ref(&source.revision_id)).await?,
                Some(super::Scope::PrivateData)
            )
        {
            return Ok(false);
        }
        let grants: Vec<String> = sqlx::query_scalar("SELECT id FROM vault_grants WHERE revision_id = ? AND revoked_at IS NULL ORDER BY id")
            .bind(&source.revision_id).fetch_all(pool).await?;
        if grants.join(":") != source.grant_fingerprint { return Ok(false) }
    }
    Ok(true)
}

pub async fn record_task_only_input(
    pool: &SqlitePool,
    attempt: &str,
    payload: &str,
    now: i64,
) -> anyhow::Result<()> {
    record_input(
        pool,
        attempt,
        InputOrigin::UserMessage,
        payload,
        PRIVATE,
        "task_only",
        None,
        None,
        None,
        None,
        now,
    )
    .await
}

pub async fn record_private_revision_input(
    pool: &SqlitePool,
    attempt: &str,
    source: &DraftPolicySource,
    now: i64,
) -> anyhow::Result<()> {
    record_input(
        pool,
        attempt,
        InputOrigin::DocumentRevision,
        &source.revision_hash,
        &source.scope,
        "task_only",
        None,
        Some(&source.document_id),
        Some(&source.revision_id),
        Some(&source.revision_hash),
        now,
    )
    .await
}

pub async fn record_unknown_input(
    pool: &SqlitePool,
    task_id: i64,
    origin: InputOrigin,
    payload: &str,
    now: i64,
) -> anyhow::Result<()> {
    let Some(attempt) = latest_attempt(pool, task_id).await? else {
        return Ok(());
    };
    record_unknown_input_for_attempt(pool, &attempt, origin, payload, now).await
}

pub async fn record_unknown_input_for_attempt(
    pool: &SqlitePool,
    attempt: &str,
    origin: InputOrigin,
    payload: &str,
    now: i64,
) -> anyhow::Result<()> {
    record_input(
        pool, attempt, origin, payload, UNKNOWN, UNKNOWN, None, None, None, None, now,
    )
    .await
}

pub async fn record_unknown_or_fail_closed(
    pool: &SqlitePool,
    attempt: &str,
    origin: InputOrigin,
    payload: &str,
    now: i64,
) -> anyhow::Result<()> {
    if let Err(error) = record_unknown_input_for_attempt(pool, attempt, origin, payload, now).await {
        sqlx::query("UPDATE vault_task_attempts SET provenance_failed = 1 WHERE id = ?")
            .bind(attempt)
            .execute(pool)
            .await?;
        return Err(error);
    }
    Ok(())
}

pub async fn begin_conversation_provenance(
    pool: &SqlitePool,
    attempt: &str,
) -> anyhow::Result<()> {
    let changed = sqlx::query(
        "UPDATE vault_task_attempts SET provenance_complete = 0 WHERE id = ? AND provenance_failed = 0",
    )
    .bind(attempt)
    .execute(pool)
    .await?
    .rows_affected();
    if changed != 1 {
        anyhow::bail!("conversation attempt is no longer eligible")
    }
    Ok(())
}

pub async fn complete_conversation_provenance(
    pool: &SqlitePool,
    attempt: &str,
) -> anyhow::Result<()> {
    let changed = sqlx::query(
        "UPDATE vault_task_attempts SET provenance_complete = 1 WHERE id = ? AND provenance_failed = 0",
    )
    .bind(attempt)
    .execute(pool)
    .await?
    .rows_affected();
    if changed != 1 {
        anyhow::bail!("conversation provenance cannot be completed")
    }
    Ok(())
}

pub async fn record_user_input(
    pool: &SqlitePool,
    task_id: i64,
    payload: &str,
    now: i64,
) -> anyhow::Result<()> {
    let Some(attempt) = latest_attempt(pool, task_id).await? else {
        return Ok(());
    };
    let consent: Option<String> = sqlx::query_scalar("SELECT c.id FROM vault_task_attempts a JOIN vault_capture_consents c ON c.binding_id = a.binding_id AND c.binding_epoch = a.binding_epoch AND c.provider = a.provider AND c.revoked_at IS NULL WHERE a.id = ?")
        .bind(&attempt).fetch_optional(pool).await?;
    let Some(consent) = consent else {
        return record_input(
            pool,
            &attempt,
            InputOrigin::UserMessage,
            payload,
            UNKNOWN,
            UNKNOWN,
            None,
            None,
            None,
            None,
            now,
        )
        .await;
    };
    record_input(
        pool,
        &attempt,
        InputOrigin::UserMessage,
        payload,
        "project",
        ALLOWED,
        Some(&consent),
        None,
        None,
        None,
        now,
    )
    .await
}

pub async fn record_revision_input(
    pool: &SqlitePool,
    attempt_id: &str,
    document_id: &str,
    revision_id: &str,
    revision_hash: &str,
    now: i64,
) -> anyhow::Result<()> {
    let scope = match super::scope_for_sources(pool, &[revision_id.to_string()]).await? {
        Some(super::Scope::Project { .. }) => "project",
        Some(super::Scope::Common) => "common",
        _ => UNKNOWN,
    };
    record_input(
        pool,
        attempt_id,
        InputOrigin::DocumentRevision,
        revision_hash,
        scope,
        UNKNOWN,
        None,
        Some(document_id),
        Some(revision_id),
        Some(revision_hash),
        now,
    )
    .await
}

pub async fn grant_consent(
    pool: &SqlitePool,
    binding: &ProjectBinding,
    provider: &str,
    now: i64,
) -> anyhow::Result<String> {
    sqlx::query("UPDATE vault_capture_consents SET revoked_at = ? WHERE binding_id = ? AND binding_epoch = ? AND provider = ? AND revoked_at IS NULL")
        .bind(now).bind(&binding.id).bind(&binding.epoch).bind(provider).execute(pool).await?;
    let id = identifier("consent")?;
    sqlx::query("INSERT INTO vault_capture_consents (id, binding_id, binding_epoch, provider, input_kinds_json, created_at) VALUES (?, ?, ?, ?, '[\"user_message\"]', ?)")
        .bind(&id).bind(&binding.id).bind(&binding.epoch).bind(provider).bind(now).execute(pool).await?;
    Ok(id)
}

pub async fn revoke_consent(
    pool: &SqlitePool,
    binding: &ProjectBinding,
    provider: &str,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE vault_capture_consents SET revoked_at = ? WHERE binding_id = ? AND binding_epoch = ? AND provider = ? AND revoked_at IS NULL")
        .bind(now).bind(&binding.id).bind(&binding.epoch).bind(provider).execute(pool).await?;
    Ok(())
}

pub async fn active_consent(
    pool: &SqlitePool,
    binding: &ProjectBinding,
    provider: &str,
) -> anyhow::Result<Option<String>> {
    sqlx::query_scalar("SELECT id FROM vault_capture_consents WHERE binding_id = ? AND binding_epoch = ? AND provider = ? AND revoked_at IS NULL")
        .bind(&binding.id).bind(&binding.epoch).bind(provider).fetch_optional(pool).await.map_err(Into::into)
}

pub async fn auto_capture_allowed(
    pool: &SqlitePool,
    task_id: i64,
    provider: &str,
) -> anyhow::Result<bool> {
    let schema_present: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'vaults'")
            .fetch_optional(pool)
            .await?;
    if schema_present.is_none() {
        return Ok(true);
    }
    let history: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM vault_task_attempts WHERE task_id = ?")
            .bind(task_id)
            .fetch_one(pool)
            .await?;
    let enabled: Option<i64> = sqlx::query_scalar("SELECT 1 FROM vaults WHERE enabled = 1 LIMIT 1")
        .fetch_optional(pool)
        .await?;
    if enabled.is_none() {
        return Ok(history == 0);
    }
    let Some(attempt) = latest_attempt(pool, task_id).await? else {
        return Ok(false);
    };
    let current: Option<String> = sqlx::query_scalar("SELECT c.id FROM vault_task_attempts a JOIN vaults v ON v.id = a.vault_id AND v.enabled = 1 AND v.writable = 1 AND v.canonical_root = a.vault_root AND v.root_device = a.vault_device AND v.root_inode = a.vault_inode JOIN vault_project_bindings b ON b.id = a.binding_id AND b.epoch = a.binding_epoch AND b.active = 1 JOIN vault_capture_consents c ON c.binding_id = a.binding_id AND c.binding_epoch = a.binding_epoch AND c.provider = a.provider AND c.revoked_at IS NULL WHERE a.id = ? AND a.provider = ? AND a.provenance_failed = 0 AND a.provenance_complete = 1")
        .bind(&attempt).bind(provider).fetch_optional(pool).await?;
    if current.is_none() {
        return Ok(false);
    }
    let row = sqlx::query("SELECT vault_root, vault_device, vault_inode, binding_id, binding_epoch FROM vault_task_attempts WHERE id = ?").bind(&attempt).fetch_one(pool).await?;
    let root: String = row.try_get("vault_root")?;
    let identity = super::platform::verified_root(std::path::Path::new(&root))?;
    if identity.canonical_root != root
        || identity.device != row.try_get::<i64, _>("vault_device")?
        || identity.inode != row.try_get::<i64, _>("vault_inode")?
    {
        return Ok(false);
    }
    let binding_root: String = sqlx::query_scalar("SELECT canonical_root FROM vault_project_bindings WHERE id = ? AND epoch = ? AND active = 1").bind(row.try_get::<String, _>("binding_id")?).bind(row.try_get::<String, _>("binding_epoch")?).fetch_one(pool).await?;
    let Some(binding) = super::resolve_project(pool, std::path::Path::new(&binding_root)).await?
    else {
        return Ok(false);
    };
    if binding.id != row.try_get::<String, _>("binding_id")?
        || binding.epoch != row.try_get::<String, _>("binding_epoch")?
    {
        return Ok(false);
    }
    let total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM vault_attempt_inputs WHERE attempt_id = ?")
            .bind(&attempt)
            .fetch_one(pool)
            .await?;
    let blocked: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM vault_task_attempts a WHERE a.task_id = (SELECT task_id FROM vault_task_attempts WHERE id = ?) AND (a.provenance_failed = 1 OR a.provenance_complete = 0 OR EXISTS (SELECT 1 FROM vault_attempt_inputs i WHERE i.attempt_id = a.id AND (i.declared_scope IN (?, ?) OR i.capture_purpose <> ? OR i.consent_id IS NULL OR NOT EXISTS (SELECT 1 FROM vault_capture_consents c WHERE c.id = i.consent_id AND c.binding_id = a.binding_id AND c.binding_epoch = a.binding_epoch AND c.provider = a.provider AND c.revoked_at IS NULL))))")
        .bind(&attempt).bind(UNKNOWN).bind(PRIVATE).bind(ALLOWED).fetch_one(pool).await?;
    Ok(total > 0 && blocked == 0)
}

pub async fn auto_capture_skip_reason(
    pool: &SqlitePool,
    task_id: i64,
    provider: &str,
) -> anyhow::Result<Option<&'static str>> {
    if auto_capture_allowed(pool, task_id, provider).await? {
        return Ok(None);
    }
    let private_or_task_only: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM vault_attempt_inputs i JOIN vault_task_attempts a ON a.id = i.attempt_id \
         WHERE a.task_id = ? AND (i.declared_scope = ? OR i.capture_purpose = 'task_only') LIMIT 1",
    )
    .bind(task_id)
    .bind(PRIVATE)
    .fetch_optional(pool)
    .await?;
    Ok(Some(if private_or_task_only.is_some() {
        "skipped_private_input"
    } else {
        "skipped_unknown_provenance"
    }))
}

pub async fn completion_snapshot_current(
    pool: &SqlitePool,
    snapshot_id: &str,
) -> anyhow::Result<bool> {
    let row = sqlx::query("SELECT s.attempt_id, s.bounded_body, s.body_hash, s.input_snapshot_id, s.terminal_hash, a.vault_id, a.vault_root, a.vault_device, a.vault_inode, a.binding_id, a.binding_epoch, a.provider, a.provenance_failed, a.provenance_complete FROM vault_terminal_snapshots s JOIN vault_task_attempts a ON a.id = s.attempt_id WHERE s.id = ?")
        .bind(snapshot_id).fetch_optional(pool).await?;
    let Some(row) = row else { return Ok(false) };
    if row.try_get::<i64, _>("provenance_failed")? != 0
        || row.try_get::<i64, _>("provenance_complete")? == 0
    {
        return Ok(false);
    }
    let profile = crate::capture::invoke::profile(pool).await;
    if row.try_get::<String, _>("provider")? != crate::capture::invoke::provider_identity(&profile)
    {
        return Ok(false);
    }
    let body: String = row.try_get("bounded_body")?;
    let body_hash: String = row.try_get("body_hash")?;
    let input_id: String = row.try_get("input_snapshot_id")?;
    let hashes: Vec<String> = sqlx::query_scalar("SELECT payload_hash FROM vault_attempt_inputs WHERE attempt_id = ? ORDER BY created_at, id").bind(row.try_get::<String, _>("attempt_id")?).fetch_all(pool).await?;
    let input_hash = hash(&hashes.join(":"));
    if hashes.is_empty() {
        return Ok(false);
    }
    let stored_input: String = sqlx::query_scalar("SELECT body_hash FROM vault_input_snapshots WHERE id = ? AND attempt_id = ? AND vault_id = ? AND vault_root = ? AND vault_device = ? AND vault_inode = ?").bind(&input_id).bind(row.try_get::<String, _>("attempt_id")?).bind(row.try_get::<String, _>("vault_id")?).bind(row.try_get::<String, _>("vault_root")?).bind(row.try_get::<i64, _>("vault_device")?).bind(row.try_get::<i64, _>("vault_inode")?).fetch_one(pool).await?;
    if hash(&body) != body_hash
        || input_hash != stored_input
        || hash(&(body_hash.clone() + &input_hash)) != row.try_get::<String, _>("terminal_hash")?
    {
        return Ok(false);
    }
    let current: Option<i64> = sqlx::query_scalar("SELECT 1 FROM vaults v JOIN vault_project_bindings b ON b.id = ? AND b.epoch = ? AND b.active = 1 WHERE v.id = ? AND v.enabled = 1 AND v.writable = 1 AND v.canonical_root = ? AND v.root_device = ? AND v.root_inode = ?")
        .bind(row.try_get::<String, _>("binding_id")?).bind(row.try_get::<String, _>("binding_epoch")?).bind(row.try_get::<String, _>("vault_id")?).bind(row.try_get::<String, _>("vault_root")?).bind(row.try_get::<i64, _>("vault_device")?).bind(row.try_get::<i64, _>("vault_inode")?).fetch_optional(pool).await?;
    if current.is_none() {
        return Ok(false);
    }
    let root: String = row.try_get("vault_root")?;
    let identity = super::platform::verified_root(std::path::Path::new(&root))?;
    if identity.canonical_root != root
        || identity.device != row.try_get::<i64, _>("vault_device")?
        || identity.inode != row.try_get::<i64, _>("vault_inode")?
    {
        return Ok(false);
    }
    let binding_root: String = sqlx::query_scalar("SELECT canonical_root FROM vault_project_bindings WHERE id = ? AND epoch = ? AND active = 1").bind(row.try_get::<String, _>("binding_id")?).bind(row.try_get::<String, _>("binding_epoch")?).fetch_one(pool).await?;
    let Some(binding) = super::resolve_project(pool, std::path::Path::new(&binding_root)).await?
    else {
        return Ok(false);
    };
    if binding.id != row.try_get::<String, _>("binding_id")?
        || binding.epoch != row.try_get::<String, _>("binding_epoch")?
    {
        return Ok(false);
    }
    let invalid: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM vault_task_attempts a WHERE a.task_id = (SELECT task_id FROM vault_task_attempts WHERE id = ?) AND (a.provenance_failed = 1 OR a.provenance_complete = 0 OR EXISTS (SELECT 1 FROM vault_attempt_inputs i WHERE i.attempt_id = a.id AND (i.declared_scope <> 'project' OR i.capture_purpose <> 'capture_allowed' OR i.consent_id IS NULL OR NOT EXISTS (SELECT 1 FROM vault_capture_consents c WHERE c.id = i.consent_id AND c.binding_id = a.binding_id AND c.binding_epoch = a.binding_epoch AND c.provider = a.provider AND c.revoked_at IS NULL))))")
        .bind(row.try_get::<String, _>("attempt_id")?).fetch_one(pool).await?;
    Ok(invalid == 0)
}

pub async fn record_terminal_snapshot(
    pool: &SqlitePool,
    task_id: i64,
    state: &str,
    body: &str,
    now: i64,
) -> anyhow::Result<Option<String>> {
    let Some(attempt) = latest_attempt(pool, task_id).await? else {
        return Ok(None);
    };
    let input_hash = input_snapshot(pool, &attempt, now).await?;
    let bounded_body = utf8_prefix(body, MAX_TERMINAL_BYTES);
    let body_hash = hash(&bounded_body);
    let terminal_hash = hash(&(body_hash.clone() + &input_hash.1));
    let existing = sqlx::query("SELECT id, body_hash, input_snapshot_id, terminal_hash, terminal_state FROM vault_terminal_snapshots WHERE attempt_id = ?")
        .bind(&attempt).fetch_optional(pool).await?;
    if let Some(existing) = existing {
        if existing.try_get::<String, _>("body_hash")? == body_hash
            && existing.try_get::<String, _>("input_snapshot_id")? == input_hash.0
            && existing.try_get::<String, _>("terminal_hash")? == terminal_hash
            && existing.try_get::<String, _>("terminal_state")? == state
        {
            return Ok(Some(existing.try_get("id")?));
        }
        anyhow::bail!("terminal snapshot input changed")
    }
    let row = sqlx::query("SELECT vault_id, vault_root, vault_device, vault_inode, binding_id, binding_epoch FROM vault_task_attempts WHERE id = ?").bind(&attempt).fetch_one(pool).await?;
    let id = identifier("terminal")?;
    sqlx::query("INSERT INTO vault_terminal_snapshots (id, attempt_id, task_id, vault_id, vault_root, vault_device, vault_inode, binding_id, binding_epoch, scope, bounded_body, body_hash, input_snapshot_id, terminal_hash, terminal_state, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'project', ?, ?, ?, ?, ?, ?)")
        .bind(&id).bind(&attempt).bind(task_id).bind(row.try_get::<String, _>("vault_id")?).bind(row.try_get::<String, _>("vault_root")?).bind(row.try_get::<i64, _>("vault_device")?).bind(row.try_get::<i64, _>("vault_inode")?).bind(row.try_get::<String, _>("binding_id")?).bind(row.try_get::<String, _>("binding_epoch")?).bind(&bounded_body).bind(&body_hash).bind(&input_hash.0).bind(terminal_hash).bind(state).bind(now).execute(pool).await?;
    Ok(Some(id))
}

pub async fn latest_attempt(pool: &SqlitePool, task_id: i64) -> anyhow::Result<Option<String>> {
    sqlx::query_scalar(
        "SELECT id FROM vault_task_attempts WHERE task_id = ? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn input_snapshot(
    pool: &SqlitePool,
    attempt: &str,
    now: i64,
) -> anyhow::Result<(String, String)> {
    let hashes: Vec<String> = sqlx::query_scalar("SELECT payload_hash FROM vault_attempt_inputs WHERE attempt_id = ? ORDER BY created_at, id").bind(attempt).fetch_all(pool).await?;
    let digest = hash(&hashes.join(":"));
    let existing =
        sqlx::query("SELECT id, body_hash FROM vault_input_snapshots WHERE attempt_id = ?")
            .bind(attempt)
            .fetch_optional(pool)
            .await?;
    if let Some(existing) = existing {
        if existing.try_get::<String, _>("body_hash")? == digest {
            return Ok((existing.try_get("id")?, digest));
        }
        anyhow::bail!("input snapshot changed")
    }
    let id = identifier("input-snapshot")?;
    let vault: (String, String, i64, i64) = sqlx::query_as("SELECT vault_id, vault_root, vault_device, vault_inode FROM vault_task_attempts WHERE id = ?").bind(attempt).fetch_one(pool).await?;
    sqlx::query("INSERT INTO vault_input_snapshots (id, attempt_id, vault_id, vault_root, vault_device, vault_inode, body_hash, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)").bind(&id).bind(attempt).bind(vault.0).bind(vault.1).bind(vault.2).bind(vault.3).bind(&digest).bind(now).execute(pool).await?;
    Ok((id, digest))
}

#[allow(clippy::too_many_arguments)]
async fn record_input(
    pool: &SqlitePool,
    attempt: &str,
    origin: InputOrigin,
    payload: &str,
    scope: &str,
    purpose: &str,
    consent: Option<&str>,
    document: Option<&str>,
    revision: Option<&str>,
    revision_hash: Option<&str>,
    now: i64,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO vault_attempt_inputs (id, attempt_id, origin_kind, payload_hash, declared_scope, capture_purpose, consent_id, document_id, revision_id, revision_hash, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(identifier("input")?).bind(attempt).bind(origin.as_str()).bind(hash(payload)).bind(scope).bind(purpose).bind(consent).bind(document).bind(revision).bind(revision_hash).bind(now).execute(pool).await?;
    Ok(())
}

fn utf8_prefix(value: &str, limit: usize) -> String {
    let mut end = value.len().min(limit);
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1
    }
    value[..end].to_string()
}
fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::knowledge::vault::register_project;

    #[tokio::test]
    async fn inherited_unknown_input_blocks_capture() {
        let root = crate::testtmp::dir().join(format!("vault-provenance-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let pool = crate::knowledge::tests::raw_pool().await;
        crate::knowledge::migrate(&pool).await.unwrap();
        let root_text = root.to_string_lossy().into_owned();
        sqlx::query("INSERT INTO tasks (id, repo, branch, base, worktree_path, instruction, state, created_at, updated_at) VALUES (1, ?, 'main', 'base', ?, 'test', 'running', 1, 1)").bind(&root_text).bind(&root_text).execute(&pool).await.unwrap();
        let binding = register_project(&pool, &root, 1).await.unwrap();
        let vault = crate::knowledge::vault::register_vault(&pool, &root, 1)
            .await
            .unwrap();
        start_attempt(&pool, 1, &vault.id, &binding, "claude", None, 1)
            .await
            .unwrap();
        record_unknown_input(&pool, 1, InputOrigin::PriorConversation, "secret", 2)
            .await
            .unwrap();
        grant_consent(&pool, &binding, "claude", 3).await.unwrap();
        assert!(!auto_capture_allowed(&pool, 1, "claude").await.unwrap());
    }

    #[tokio::test]
    async fn failed_tool_provenance_blocks_later_completion_and_reopen() {
        let root = crate::testtmp::dir().join(format!("vault-tool-failure-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let pool = crate::knowledge::tests::raw_pool().await;
        crate::knowledge::migrate(&pool).await.unwrap();
        let root_text = root.to_string_lossy().into_owned();
        sqlx::query("INSERT INTO tasks (id, repo, branch, base, worktree_path, instruction, state, created_at, updated_at) VALUES (6, ?, 'main', 'base', ?, 'test', 'done', 1, 1)")
            .bind(&root_text).bind(&root_text).execute(&pool).await.unwrap();
        let vault = crate::knowledge::vault::register_vault(&pool, &root, 1).await.unwrap();
        let binding = register_project(&pool, &root, 1).await.unwrap();
        let provider = crate::capture::invoke::provider_identity(&crate::capture::invoke::profile(&pool).await);
        grant_consent(&pool, &binding, &provider, 2).await.unwrap();
        let failed = start_attempt(&pool, 6, &vault.id, &binding, &provider, None, 3).await.unwrap();
        record_user_input(&pool, 6, "consented", 4).await.unwrap();
        sqlx::query("CREATE TRIGGER fail_tool_input BEFORE INSERT ON vault_attempt_inputs WHEN NEW.origin_kind = 'tool_result' BEGIN SELECT RAISE(ABORT, 'injected tool persistence failure'); END")
            .execute(&pool).await.unwrap();
        assert!(record_unknown_or_fail_closed(&pool, &failed, InputOrigin::ToolResult, "tool", 5).await.is_err());
        sqlx::query("DROP TRIGGER fail_tool_input").execute(&pool).await.unwrap();
        assert!(!auto_capture_allowed(&pool, 6, &provider).await.unwrap());
        start_attempt(&pool, 6, &vault.id, &binding, &provider, None, 6).await.unwrap();
        record_user_input(&pool, 6, "reopened", 7).await.unwrap();
        assert!(!auto_capture_allowed(&pool, 6, &provider).await.unwrap());
        let snapshot = record_terminal_snapshot(&pool, 6, "done", "completion", 8).await.unwrap().unwrap();
        assert!(!completion_snapshot_current(&pool, &snapshot).await.unwrap());
    }

    #[tokio::test]
    async fn tool_free_conversation_completes_before_capture() {
        let root = crate::testtmp::dir().join(format!("vault-convo-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let pool = crate::knowledge::tests::raw_pool().await;
        crate::knowledge::migrate(&pool).await.unwrap();
        let root_text = root.to_string_lossy().into_owned();
        sqlx::query("INSERT INTO tasks (id, repo, branch, base, worktree_path, instruction, state, created_at, updated_at) VALUES (7, ?, 'main', 'base', ?, 'test', 'done', 1, 1)")
            .bind(&root_text)
            .bind(&root_text)
            .execute(&pool)
            .await
            .unwrap();
        let vault = crate::knowledge::vault::register_vault(&pool, &root, 1).await.unwrap();
        let binding = register_project(&pool, &root, 1).await.unwrap();
        let provider = crate::capture::invoke::provider_identity(&crate::capture::invoke::profile(&pool).await);
        grant_consent(&pool, &binding, &provider, 2).await.unwrap();
        let attempt = start_attempt(&pool, 7, &vault.id, &binding, &provider, None, 3).await.unwrap();
        record_user_input(&pool, 7, "consented", 4).await.unwrap();
        begin_conversation_provenance(&pool, &attempt).await.unwrap();
        assert!(!auto_capture_allowed(&pool, 7, &provider).await.unwrap());
        complete_conversation_provenance(&pool, &attempt).await.unwrap();
        assert!(auto_capture_allowed(&pool, 7, &provider).await.unwrap());
        let snapshot = record_terminal_snapshot(&pool, 7, "done", "completion", 5)
            .await
            .unwrap()
            .unwrap();
        assert!(completion_snapshot_current(&pool, &snapshot).await.unwrap());
    }

    #[tokio::test]
    async fn migration_denies_unproven_historical_attempts() {
        let root = crate::testtmp::dir().join(format!("vault-upgrade-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let pool = crate::knowledge::tests::raw_pool().await;
        sqlx::query("CREATE TABLE vault_task_attempts (id TEXT PRIMARY KEY, task_id INTEGER NOT NULL, vault_id TEXT NOT NULL, vault_root TEXT NOT NULL, vault_device INTEGER NOT NULL, vault_inode INTEGER NOT NULL, binding_id TEXT NOT NULL, binding_epoch TEXT NOT NULL, provider TEXT NOT NULL, client_ref TEXT, provenance_failed INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        crate::knowledge::migrate(&pool).await.unwrap();
        let root_text = root.to_string_lossy().into_owned();
        for id in [8, 9] {
            sqlx::query("INSERT INTO tasks (id, repo, branch, base, worktree_path, instruction, state, created_at, updated_at) VALUES (?, ?, 'main', 'base', ?, 'test', 'done', 1, 1)")
                .bind(id)
                .bind(&root_text)
                .bind(&root_text)
                .execute(&pool)
                .await
                .unwrap();
        }
        let vault = crate::knowledge::vault::register_vault(&pool, &root, 1).await.unwrap();
        let binding = register_project(&pool, &root, 1).await.unwrap();
        let provider = crate::capture::invoke::provider_identity(&crate::capture::invoke::profile(&pool).await);
        grant_consent(&pool, &binding, &provider, 2).await.unwrap();
        let identity = super::super::platform::verified_root(&root).unwrap();
        sqlx::query("INSERT INTO vault_task_attempts (id, task_id, vault_id, vault_root, vault_device, vault_inode, binding_id, binding_epoch, provider, created_at) VALUES ('old', 8, ?, ?, ?, ?, ?, ?, ?, 3)")
            .bind(&vault.id)
            .bind(&identity.canonical_root)
            .bind(identity.device)
            .bind(identity.inode)
            .bind(&binding.id)
            .bind(&binding.epoch)
            .bind(&provider)
            .execute(&pool)
            .await
            .unwrap();
        record_user_input(&pool, 8, "historical", 4).await.unwrap();
        assert!(!auto_capture_allowed(&pool, 8, &provider).await.unwrap());
        start_attempt(&pool, 9, &vault.id, &binding, &provider, None, 5)
            .await
            .unwrap();
        record_user_input(&pool, 9, "current", 6).await.unwrap();
        assert!(auto_capture_allowed(&pool, 9, &provider).await.unwrap());
    }

    #[tokio::test]
    async fn changed_vault_identity_blocks_an_existing_attempt() {
        let root = crate::testtmp::dir().join(format!("vault-identity-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let pool = crate::knowledge::tests::raw_pool().await;
        crate::knowledge::migrate(&pool).await.unwrap();
        let root_text = root.to_string_lossy().into_owned();
        sqlx::query("INSERT INTO tasks (id, repo, branch, base, worktree_path, instruction, state, created_at, updated_at) VALUES (2, ?, 'main', 'base', ?, 'test', 'running', 1, 1)").bind(&root_text).bind(&root_text).execute(&pool).await.unwrap();
        let vault = crate::knowledge::vault::register_vault(&pool, &root, 1)
            .await
            .unwrap();
        let binding = register_project(&pool, &root, 1).await.unwrap();
        grant_consent(&pool, &binding, "claude", 2).await.unwrap();
        start_attempt(&pool, 2, &vault.id, &binding, "claude", None, 3)
            .await
            .unwrap();
        record_user_input(&pool, 2, "consented", 4).await.unwrap();
        assert!(auto_capture_allowed(&pool, 2, "claude").await.unwrap());
        let moved = root.with_extension("replaced");
        std::fs::rename(&root, &moved).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        assert!(!auto_capture_allowed(&pool, 2, "claude").await.unwrap());
    }

    #[tokio::test]
    async fn profile_change_or_regrant_does_not_restore_old_input() {
        let root = crate::testtmp::dir().join(format!("vault-provider-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let pool = crate::knowledge::tests::raw_pool().await;
        crate::knowledge::migrate(&pool).await.unwrap();
        let root_text = root.to_string_lossy().into_owned();
        for id in [3, 4] {
            sqlx::query("INSERT INTO tasks (id, repo, branch, base, worktree_path, instruction, state, created_at, updated_at) VALUES (?, ?, 'main', 'base', ?, 'test', 'running', 1, 1)").bind(id).bind(&root_text).bind(&root_text).execute(&pool).await.unwrap();
        }
        let vault = crate::knowledge::vault::register_vault(&pool, &root, 1)
            .await
            .unwrap();
        let binding = register_project(&pool, &root, 1).await.unwrap();
        let old = crate::capture::invoke::provider_identity(
            &crate::capture::invoke::profile(&pool).await,
        );
        grant_consent(&pool, &binding, &old, 2).await.unwrap();
        start_attempt(&pool, 3, &vault.id, &binding, &old, None, 3)
            .await
            .unwrap();
        record_user_input(&pool, 3, "consented", 4).await.unwrap();
        assert!(auto_capture_allowed(&pool, 3, &old).await.unwrap());
        revoke_consent(&pool, &binding, &old, 5).await.unwrap();
        grant_consent(&pool, &binding, &old, 6).await.unwrap();
        assert!(!auto_capture_allowed(&pool, 3, &old).await.unwrap());
        crate::db::set_setting(&pool, crate::capture::invoke::KEY_MODEL, "haiku")
            .await
            .unwrap();
        let current = crate::capture::invoke::provider_identity(
            &crate::capture::invoke::profile(&pool).await,
        );
        grant_consent(&pool, &binding, &current, 7).await.unwrap();
        assert!(!auto_capture_allowed(&pool, 3, &current).await.unwrap());
        start_attempt(&pool, 4, &vault.id, &binding, &current, None, 8)
            .await
            .unwrap();
        record_user_input(&pool, 4, "new consent", 9).await.unwrap();
        assert!(auto_capture_allowed(&pool, 4, &current).await.unwrap());
    }

    #[tokio::test]
    async fn repeated_done_returns_the_existing_terminal_snapshot() {
        let root = crate::testtmp::dir().join(format!("vault-snapshot-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let pool = crate::knowledge::tests::raw_pool().await;
        crate::knowledge::migrate(&pool).await.unwrap();
        let root_text = root.to_string_lossy().into_owned();
        sqlx::query("INSERT INTO tasks (id, repo, branch, base, worktree_path, instruction, state, created_at, updated_at) VALUES (5, ?, 'main', 'base', ?, 'test', 'done', 1, 1)")
            .bind(&root_text)
            .bind(&root_text)
            .execute(&pool)
            .await
            .unwrap();
        let vault = crate::knowledge::vault::register_vault(&pool, &root, 1)
            .await
            .unwrap();
        let binding = register_project(&pool, &root, 1).await.unwrap();
        grant_consent(&pool, &binding, "claude", 2).await.unwrap();
        start_attempt(&pool, 5, &vault.id, &binding, "claude", None, 3)
            .await
            .unwrap();
        record_user_input(&pool, 5, "consented", 4).await.unwrap();
        let first = record_terminal_snapshot(&pool, 5, "done", "completion", 5)
            .await
            .unwrap();
        let second = record_terminal_snapshot(&pool, 5, "done", "completion", 6)
            .await
            .unwrap();
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn private_draft_policy_binds_the_exact_revision_and_query() {
        let root = crate::testtmp::dir().join(format!("vault-policy-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let pool = crate::knowledge::tests::raw_pool().await;
        crate::knowledge::migrate(&pool).await.unwrap();
        let vault = crate::knowledge::vault::register_vault(&pool, &root, 1)
            .await
            .unwrap();
        let binding = register_project(&pool, &root, 1).await.unwrap();
        let root_text = root.to_string_lossy().into_owned();
        sqlx::query("INSERT INTO tasks (id, repo, branch, base, worktree_path, instruction, state, created_at, updated_at) VALUES (1, ?, 'main', 'base', ?, 'test', 'running', 1, 1)")
            .bind(&root_text)
            .bind(&root_text)
            .execute(&pool)
            .await
            .unwrap();
        let source = crate::knowledge::vault::create_text_source(
            &pool,
            &crate::knowledge::vault::TextSourceDraft {
                vault_id: vault.id,
                title: "private".into(),
                body: "evidence".into(),
                scope: crate::knowledge::vault::ScopeRequest {
                    scope: crate::knowledge::vault::Scope::PrivateData,
                },
            },
            2,
        )
        .await
        .unwrap();
        save_draft_policy(
            &pool,
            &binding,
            "draft-1",
            "question",
            "private_attachment",
            std::slice::from_ref(&source.revision_id),
            3,
        )
        .await
        .unwrap();
        assert!(pending_restrictive_draft_policy(&pool, "draft-1")
            .await
            .unwrap());
        assert!(
            consume_draft_policy(&pool, &binding, "draft-1", "changed", 1)
                .await
                .unwrap()
                .is_none()
        );
        save_draft_policy(
            &pool,
            &binding,
            "draft-2",
            "question",
            "private_attachment",
            std::slice::from_ref(&source.revision_id),
            4,
        )
        .await
        .unwrap();
        let policy = consume_draft_policy(&pool, &binding, "draft-2", "question", 1)
            .await
            .unwrap()
            .unwrap();
        assert!(draft_policy_current(&pool, &policy).await.unwrap());
        assert_eq!(policy.sources.len(), 1);

        save_draft_policy(
            &pool,
            &binding,
            "draft-3",
            "question",
            "private_attachment",
            std::slice::from_ref(&source.revision_id),
            5,
        )
        .await
        .unwrap();
        let policy = consume_draft_policy(&pool, &binding, "draft-3", "question", 1)
            .await
            .unwrap()
            .unwrap();
        crate::knowledge::vault::change_scope(
            &pool,
            &source.revision_id,
            &crate::knowledge::vault::ScopeRequest {
                scope: crate::knowledge::vault::Scope::PrivateData,
            },
            6,
        )
        .await
        .unwrap();
        assert!(!draft_policy_current(&pool, &policy).await.unwrap());
        assert!(restrictive_draft_policy_seen(&pool, &binding, "draft-1")
            .await
            .unwrap());
        grant_consent(&pool, &binding, "claude", 7).await.unwrap();
        for now in [8, 9] {
            let attempt = start_attempt(
                &pool,
                1,
                &source.document.vault_id,
                &binding,
                "claude",
                Some("draft-1"),
                now,
            )
            .await
            .unwrap();
            record_task_only_input(&pool, &attempt, "changed question", now)
                .await
                .unwrap();
            assert!(!auto_capture_allowed(&pool, 1, "claude").await.unwrap());
        }
    }
}
