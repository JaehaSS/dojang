//! Explicit composer attachments.  Unlike the legacy search preview, every
//! selected item is retained and delivered as one fail-closed snapshot.

use std::collections::HashSet;

use sha2::{Digest, Sha256};
use sqlx::{Row, SqliteConnection, SqlitePool};

use super::bindings::ProjectBinding;
use super::catalog::identifier;
use super::files::read_revision;
use super::provenance::{DraftPolicy, DraftPolicySource};
use super::retrieval::{ReferenceItem, ReferencePreview};
use super::{scope_allows_binding, scope_for_sources, Scope};

const PREVIEW_PREFIX: &str = "selected-preview-";
const POLICY_PREFIX: &str = "selected-policy-";
const MAX_DOCUMENTS: usize = 5;
const MAX_DOCUMENT_BYTES: usize = 2 * 1024;
const MAX_TOTAL_BYTES: usize = 8 * 1024;
const MAX_SOURCE_BYTES: i64 = 2 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Selection {
    pub revision_id: String,
    pub expected_hash: String,
}

#[derive(Clone, Debug)]
pub struct PreparedReference {
    pub document_id: String,
    pub title: String,
    pub scope: String,
    pub item: ReferenceItem,
    private: bool,
    grant_fingerprint: String,
}

#[derive(Clone, Debug)]
pub struct PreparedSnapshot {
    pub preview: ReferencePreview,
    pub references: Vec<PreparedReference>,
}

#[derive(Clone, Debug)]
pub struct PendingPolicy {
    pub preview_id: String,
    pub policy: DraftPolicy,
}

pub async fn prepare(
    pool: &SqlitePool,
    binding: &ProjectBinding,
    query: &str,
    client_ref: &str,
    selections: &[Selection],
    input_mode: &str,
    now: i64,
) -> anyhow::Result<PreparedSnapshot> {
    if !matches!(input_mode, "default" | "task_only") {
        anyhow::bail!("selected attachments require default or task_only input mode")
    }
    if selections.len() > MAX_DOCUMENTS {
        anyhow::bail!("selected attachments exceed 5 documents")
    }
    let unique = selections
        .iter()
        .map(|item| &item.revision_id)
        .collect::<HashSet<_>>();
    if unique.len() != selections.len() {
        anyhow::bail!("selected attachments must be unique")
    }

    let mut references = Vec::with_capacity(selections.len());
    let mut total_bytes = 0usize;
    for selection in selections {
        let reference = checked_reference(pool, binding, selection).await?;
        total_bytes += reference.item.snippet.len();
        if total_bytes > MAX_TOTAL_BYTES {
            anyhow::bail!("selected attachment excerpts exceed 8 KiB")
        }
        references.push(reference);
    }

    let query_hash = hash(query);
    let preview_id = identifier("selected-preview")?;
    let policy_id = identifier("selected-policy")?;
    let policy_mode = if references.iter().any(|item| item.private) {
        "private_attachment"
    } else {
        input_mode
    };
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE vault_reference_previews SET state = 'invalidated' WHERE binding_id = ? AND binding_epoch = ? AND client_ref = ? AND state = 'pending'")
        .bind(&binding.id).bind(&binding.epoch).bind(client_ref).execute(&mut *tx).await?;
    sqlx::query("UPDATE vault_draft_policies SET state = 'invalidated' WHERE binding_id = ? AND binding_epoch = ? AND client_ref = ? AND state = 'pending'")
        .bind(&binding.id).bind(&binding.epoch).bind(client_ref).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO vault_reference_previews (id, binding_id, binding_epoch, query_hash, client_ref, state, created_at) VALUES (?, ?, ?, ?, ?, 'pending', ?)")
        .bind(&preview_id).bind(&binding.id).bind(&binding.epoch).bind(&query_hash).bind(client_ref).bind(now).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO vault_draft_policies (id, binding_id, binding_epoch, client_ref, query_hash, input_mode, state, created_at) VALUES (?, ?, ?, ?, ?, ?, 'pending', ?)")
        .bind(&policy_id).bind(&binding.id).bind(&binding.epoch).bind(client_ref).bind(&query_hash).bind(policy_mode).bind(now).execute(&mut *tx).await?;
    for reference in &references {
        sqlx::query("INSERT INTO vault_reference_preview_items (preview_id, revision_id, revision_hash, snippet, reason) VALUES (?, ?, ?, ?, ?)")
            .bind(&preview_id).bind(&reference.item.revision_id).bind(&reference.item.revision_hash).bind(&reference.item.snippet).bind(&reference.item.reason).execute(&mut *tx).await?;
        if reference.private {
            sqlx::query("INSERT INTO vault_draft_policy_sources (policy_id, document_id, revision_id, revision_hash, grant_fingerprint, scope) VALUES (?, ?, ?, ?, ?, 'private-data')")
                .bind(&policy_id).bind(&reference.document_id).bind(&reference.item.revision_id).bind(&reference.item.revision_hash).bind(&reference.grant_fingerprint).execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    Ok(PreparedSnapshot {
        preview: ReferencePreview {
            id: preview_id,
            query_hash,
            created_at: now,
            references: references.iter().map(|item| item.item.clone()).collect(),
        },
        references,
    })
}

/// An empty selection cancels both halves together, allowing the ordinary
/// composer path to proceed again.
pub async fn cancel(
    pool: &SqlitePool,
    binding: &ProjectBinding,
    client_ref: &str,
    query: &str,
    now: i64,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE vault_reference_previews SET state = 'invalidated' WHERE binding_id = ? AND binding_epoch = ? AND client_ref = ? AND (state = 'pending' OR (id LIKE ? AND state = 'consumed'))")
        .bind(&binding.id).bind(&binding.epoch).bind(client_ref).bind(format!("{PREVIEW_PREFIX}%")).execute(&mut *tx).await?;
    sqlx::query("UPDATE vault_draft_policies SET state = 'invalidated' WHERE binding_id = ? AND binding_epoch = ? AND client_ref = ? AND (state = 'pending' OR (id LIKE ? AND state = 'consumed'))")
        .bind(&binding.id).bind(&binding.epoch).bind(client_ref).bind(format!("{POLICY_PREFIX}%")).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO vault_draft_policies (id, binding_id, binding_epoch, client_ref, query_hash, input_mode, state, created_at) VALUES (?, ?, ?, ?, ?, 'default', 'pending', ?)")
        .bind(identifier("draft-policy")?).bind(&binding.id).bind(&binding.epoch).bind(client_ref).bind(hash(query)).bind(now).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

/// A task-only message has no document excerpt, but it must still replace an
/// attachment snapshot atomically before the ordinary provenance path consumes
/// its restrictive policy.
pub async fn prepare_task_only(
    pool: &SqlitePool,
    binding: &ProjectBinding,
    query: &str,
    client_ref: &str,
    now: i64,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE vault_reference_previews SET state = 'invalidated' WHERE binding_id = ? AND binding_epoch = ? AND client_ref = ? AND (state = 'pending' OR (id LIKE ? AND state = 'consumed'))")
        .bind(&binding.id).bind(&binding.epoch).bind(client_ref).bind(format!("{PREVIEW_PREFIX}%")).execute(&mut *tx).await?;
    sqlx::query("UPDATE vault_draft_policies SET state = 'invalidated' WHERE binding_id = ? AND binding_epoch = ? AND client_ref = ? AND (state = 'pending' OR (id LIKE ? AND state = 'consumed'))")
        .bind(&binding.id).bind(&binding.epoch).bind(client_ref).bind(format!("{POLICY_PREFIX}%")).execute(&mut *tx).await?;
    let policy_id = identifier("draft-policy")?;
    sqlx::query("INSERT INTO vault_draft_policies (id, binding_id, binding_epoch, client_ref, query_hash, input_mode, state, created_at) VALUES (?, ?, ?, ?, ?, 'task_only', 'pending', ?)")
        .bind(policy_id).bind(&binding.id).bind(&binding.epoch).bind(client_ref).bind(hash(query)).bind(now).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn has_explicit_snapshot(pool: &SqlitePool, client_ref: &str) -> anyhow::Result<bool> {
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM vault_reference_previews WHERE client_ref = ? AND id LIKE ? AND state IN ('pending', 'consumed') LIMIT 1")
        .bind(client_ref).bind(format!("{PREVIEW_PREFIX}%")).fetch_optional(pool).await?;
    Ok(exists.is_some())
}

/// Validate the paired snapshot before provenance or delivery consumes either
/// record.  A consumed explicit snapshot deliberately requires re-prepare.
pub async fn pending_policy(
    pool: &SqlitePool,
    binding: &ProjectBinding,
    query: &str,
    client_ref: &str,
) -> anyhow::Result<Option<PendingPolicy>> {
    let Some((preview, policy)) = pending_pair(pool, binding, query, client_ref).await? else {
        return Ok(None);
    };
    validate_pair(pool, binding, &preview, &policy).await?;
    Ok(Some(PendingPolicy {
        preview_id: preview.id,
        policy: policy.policy,
    }))
}

/// Consume the exact snapshot that the delivery preflight accepted.  `BEGIN
/// IMMEDIATE` fences scope/grant/policy writers while the pair is loaded,
/// validated, and marked consumed; file drift is still checked by
/// `checked_reference` immediately before that mark.
pub async fn consume_expected(
    pool: &SqlitePool,
    binding: &ProjectBinding,
    query: &str,
    client_ref: &str,
    expected_preview_id: &str,
    task_id: i64,
) -> anyhow::Result<ReferencePreview> {
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let preview = load_preview_connection(
        &mut *transaction,
        binding,
        client_ref,
        "pending",
        Some(expected_preview_id),
    )
    .await?
    .ok_or_else(|| {
        anyhow::anyhow!("selected attachments changed before delivery; prepare them again")
    })?;
    let policy = load_policy_connection(&mut *transaction, binding, client_ref, "pending")
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!("selected attachment snapshot is incomplete; prepare it again")
        })?;
    if preview.query_hash != hash(query) || policy.query_hash != hash(query) {
        anyhow::bail!("selected attachments no longer match the message; prepare them again")
    }
    if preview.references.is_empty() {
        anyhow::bail!("selected attachment snapshot is empty")
    }
    validate_pair(pool, binding, &preview, &policy).await?;
    let preview_changed = sqlx::query("UPDATE vault_reference_previews SET state = 'consumed', consumed_task_id = ? WHERE id = ? AND state = 'pending'")
        .bind(task_id).bind(&preview.id).execute(&mut *transaction).await?.rows_affected();
    let policy_changed = sqlx::query("UPDATE vault_draft_policies SET state = 'consumed', consumed_task_id = ? WHERE id = ? AND state = 'pending'")
        .bind(task_id).bind(&policy.id).execute(&mut *transaction).await?.rows_affected();
    if preview_changed != 1 || policy_changed != 1 {
        anyhow::bail!("selected attachment snapshot changed before delivery")
    }
    transaction.commit().await?;
    Ok(preview)
}

async fn pending_pair(
    pool: &SqlitePool,
    binding: &ProjectBinding,
    query: &str,
    client_ref: &str,
) -> anyhow::Result<Option<(ReferencePreview, StoredPolicy)>> {
    let preview = load_preview(pool, binding, client_ref, "pending").await?;
    let policy = load_policy(pool, binding, client_ref, "pending").await?;
    match (preview, policy) {
        (None, None) => {
            let consumed: Option<i64> = sqlx::query_scalar("SELECT 1 FROM vault_reference_previews WHERE binding_id = ? AND binding_epoch = ? AND client_ref = ? AND id LIKE ? AND state = 'consumed' LIMIT 1")
                .bind(&binding.id).bind(&binding.epoch).bind(client_ref).bind(format!("{PREVIEW_PREFIX}%")).fetch_optional(pool).await?;
            if consumed.is_some() {
                anyhow::bail!("selected attachments were already consumed; prepare them again before retrying")
            }
            Ok(None)
        }
        (Some(preview), Some(policy)) => {
            if preview.query_hash != hash(query) || policy.query_hash != hash(query) {
                anyhow::bail!(
                    "selected attachments no longer match the message; prepare them again"
                )
            }
            if preview.references.is_empty() {
                anyhow::bail!("selected attachment snapshot is empty")
            }
            Ok(Some((preview, policy)))
        }
        _ => anyhow::bail!("selected attachment snapshot is incomplete; prepare it again"),
    }
}

struct StoredPolicy {
    id: String,
    query_hash: String,
    policy: DraftPolicy,
}

async fn load_preview(
    pool: &SqlitePool,
    binding: &ProjectBinding,
    client_ref: &str,
    state: &str,
) -> anyhow::Result<Option<ReferencePreview>> {
    let row = sqlx::query("SELECT id, query_hash, created_at FROM vault_reference_previews WHERE binding_id = ? AND binding_epoch = ? AND client_ref = ? AND id LIKE ? AND state = ?")
        .bind(&binding.id).bind(&binding.epoch).bind(client_ref).bind(format!("{PREVIEW_PREFIX}%")).bind(state).fetch_optional(pool).await?;
    let Some(row) = row else { return Ok(None) };
    let id: String = row.try_get("id")?;
    let references = strict_preview_items(sqlx::query("SELECT revision_id, revision_hash, snippet, reason, excluded, stale_reason FROM vault_reference_preview_items WHERE preview_id = ? ORDER BY rowid")
        .bind(&id).fetch_all(pool).await?)?;
    Ok(Some(ReferencePreview {
        id,
        query_hash: row.try_get("query_hash")?,
        created_at: row.try_get("created_at")?,
        references,
    }))
}

async fn load_preview_connection(
    connection: &mut SqliteConnection,
    binding: &ProjectBinding,
    client_ref: &str,
    state: &str,
    expected_id: Option<&str>,
) -> anyhow::Result<Option<ReferencePreview>> {
    let row = sqlx::query("SELECT id, query_hash, created_at FROM vault_reference_previews WHERE binding_id = ? AND binding_epoch = ? AND client_ref = ? AND id LIKE ? AND state = ? AND (? IS NULL OR id = ?)")
        .bind(&binding.id).bind(&binding.epoch).bind(client_ref).bind(format!("{PREVIEW_PREFIX}%")).bind(state).bind(expected_id).bind(expected_id).fetch_optional(&mut *connection).await?;
    let Some(row) = row else { return Ok(None) };
    let id: String = row.try_get("id")?;
    let references = strict_preview_items(sqlx::query("SELECT revision_id, revision_hash, snippet, reason, excluded, stale_reason FROM vault_reference_preview_items WHERE preview_id = ? ORDER BY rowid")
        .bind(&id).fetch_all(&mut *connection).await?)?;
    Ok(Some(ReferencePreview {
        id,
        query_hash: row.try_get("query_hash")?,
        created_at: row.try_get("created_at")?,
        references,
    }))
}

fn strict_preview_items(rows: Vec<sqlx::sqlite::SqliteRow>) -> anyhow::Result<Vec<ReferenceItem>> {
    rows.into_iter()
        .map(|row| {
            if row.try_get::<i64, _>("excluded")? != 0
                || row.try_get::<Option<String>, _>("stale_reason")?.is_some()
            {
                anyhow::bail!("selected attachment snapshot was modified; prepare it again")
            }
            Ok(ReferenceItem {
                revision_id: row.try_get("revision_id")?,
                revision_hash: row.try_get("revision_hash")?,
                snippet: row.try_get("snippet")?,
                reason: row.try_get("reason")?,
            })
        })
        .collect()
}

async fn load_policy(
    pool: &SqlitePool,
    binding: &ProjectBinding,
    client_ref: &str,
    state: &str,
) -> anyhow::Result<Option<StoredPolicy>> {
    let row = sqlx::query("SELECT id, query_hash, input_mode FROM vault_draft_policies WHERE binding_id = ? AND binding_epoch = ? AND client_ref = ? AND id LIKE ? AND state = ?")
        .bind(&binding.id).bind(&binding.epoch).bind(client_ref).bind(format!("{POLICY_PREFIX}%")).bind(state).fetch_optional(pool).await?;
    let Some(row) = row else { return Ok(None) };
    let id: String = row.try_get("id")?;
    let sources = sqlx::query("SELECT document_id, revision_id, revision_hash, grant_fingerprint, scope FROM vault_draft_policy_sources WHERE policy_id = ? ORDER BY revision_id")
        .bind(&id).fetch_all(pool).await?.into_iter().map(|row| Ok(DraftPolicySource { document_id: row.try_get("document_id")?, revision_id: row.try_get("revision_id")?, revision_hash: row.try_get("revision_hash")?, grant_fingerprint: row.try_get("grant_fingerprint")?, scope: row.try_get("scope")? })).collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Some(StoredPolicy {
        id,
        query_hash: row.try_get("query_hash")?,
        policy: DraftPolicy {
            input_mode: row.try_get("input_mode")?,
            sources,
        },
    }))
}

async fn load_policy_connection(
    connection: &mut SqliteConnection,
    binding: &ProjectBinding,
    client_ref: &str,
    state: &str,
) -> anyhow::Result<Option<StoredPolicy>> {
    let row = sqlx::query("SELECT id, query_hash, input_mode FROM vault_draft_policies WHERE binding_id = ? AND binding_epoch = ? AND client_ref = ? AND id LIKE ? AND state = ?")
        .bind(&binding.id).bind(&binding.epoch).bind(client_ref).bind(format!("{POLICY_PREFIX}%")).bind(state).fetch_optional(&mut *connection).await?;
    let Some(row) = row else { return Ok(None) };
    let id: String = row.try_get("id")?;
    let sources = sqlx::query("SELECT document_id, revision_id, revision_hash, grant_fingerprint, scope FROM vault_draft_policy_sources WHERE policy_id = ? ORDER BY revision_id")
        .bind(&id).fetch_all(&mut *connection).await?.into_iter().map(|row| Ok(DraftPolicySource { document_id: row.try_get("document_id")?, revision_id: row.try_get("revision_id")?, revision_hash: row.try_get("revision_hash")?, grant_fingerprint: row.try_get("grant_fingerprint")?, scope: row.try_get("scope")? })).collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Some(StoredPolicy {
        id,
        query_hash: row.try_get("query_hash")?,
        policy: DraftPolicy {
            input_mode: row.try_get("input_mode")?,
            sources,
        },
    }))
}

async fn validate_pair(
    pool: &SqlitePool,
    binding: &ProjectBinding,
    preview: &ReferencePreview,
    policy: &StoredPolicy,
) -> anyhow::Result<()> {
    if !binding_is_active(pool, binding).await? {
        anyhow::bail!("selected attachment repository binding changed")
    }
    let private = policy
        .policy
        .sources
        .iter()
        .map(|source| &source.revision_id)
        .collect::<HashSet<_>>();
    let selected = preview
        .references
        .iter()
        .map(|reference| &reference.revision_id)
        .collect::<HashSet<_>>();
    if private
        .iter()
        .any(|revision_id| !selected.contains(revision_id))
    {
        anyhow::bail!("selected private attachment snapshot is incomplete")
    }
    for reference in &preview.references {
        let scope = current_reference_scope(pool, binding, reference).await?;
        match scope {
            Scope::PrivateData => {
                if !private.contains(&reference.revision_id) {
                    anyhow::bail!("selected private attachment policy is missing")
                }
            }
            Scope::Common | Scope::Project { .. } => {
                if private.contains(&reference.revision_id) {
                    anyhow::bail!("selected attachment scope changed")
                }
            }
        }
    }
    if !super::provenance::draft_policy_current(pool, &policy.policy).await? {
        anyhow::bail!("selected private attachment grant or revision changed")
    }
    Ok(())
}

async fn checked_reference(
    pool: &SqlitePool,
    binding: &ProjectBinding,
    selection: &Selection,
) -> anyhow::Result<PreparedReference> {
    if !binding_is_active(pool, binding).await? {
        anyhow::bail!("selected attachment repository binding changed")
    }
    let row = sqlx::query("SELECT d.id AS document_id, d.title, d.state, d.current_revision, r.sha256, r.relative_path, r.size, v.enabled, v.writable FROM vault_revisions r JOIN vault_documents d ON d.id = r.document_id JOIN vaults v ON v.id = d.vault_id WHERE r.id = ?")
        .bind(&selection.revision_id).fetch_optional(pool).await?.ok_or_else(|| anyhow::anyhow!("selected attachment revision is missing"))?;
    if row.try_get::<String, _>("state")? != "active"
        || row
            .try_get::<Option<String>, _>("current_revision")?
            .as_deref()
            != Some(&selection.revision_id)
    {
        anyhow::bail!("selected attachment is no longer the active revision")
    }
    if row.try_get::<String, _>("sha256")? != selection.expected_hash {
        anyhow::bail!("selected attachment hash changed")
    }
    if row.try_get::<i64, _>("enabled")? == 0 || row.try_get::<i64, _>("writable")? == 0 {
        anyhow::bail!("selected attachment vault is not active")
    }
    let relative_path: String = row.try_get("relative_path")?;
    let supported = std::path::Path::new(&relative_path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "md" | "txt" | "csv"
            )
        })
        .unwrap_or(false);
    if !supported || !(0..=MAX_SOURCE_BYTES).contains(&row.try_get::<i64, _>("size")?) {
        anyhow::bail!("selected attachment cannot be previewed")
    }
    let scope = scope_for_sources(pool, std::slice::from_ref(&selection.revision_id))
        .await?
        .ok_or_else(|| anyhow::anyhow!("selected attachment has no active scope"))?;
    let private = matches!(scope, Scope::PrivateData);
    if !private
        && !scope_allows_binding(pool, std::slice::from_ref(&selection.revision_id), binding)
            .await?
    {
        anyhow::bail!("selected attachment is outside the current project scope")
    }
    let bytes = read_revision(pool, &selection.revision_id).await?;
    let snippet = utf8_prefix(&bytes, MAX_DOCUMENT_BYTES)
        .ok_or_else(|| anyhow::anyhow!("selected attachment is not UTF-8 text"))?;
    if snippet.is_empty() {
        anyhow::bail!("selected attachment has no deliverable text")
    }
    let grants: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM vault_grants WHERE revision_id = ? AND revoked_at IS NULL ORDER BY id",
    )
    .bind(&selection.revision_id)
    .fetch_all(pool)
    .await?;
    Ok(PreparedReference {
        document_id: row.try_get("document_id")?,
        title: row.try_get("title")?,
        scope: scope_name(&scope).into(),
        item: ReferenceItem {
            revision_id: selection.revision_id.clone(),
            revision_hash: selection.expected_hash.clone(),
            snippet,
            reason: row.try_get("title")?,
        },
        private,
        grant_fingerprint: grants.join(":"),
    })
}

async fn binding_is_active(pool: &SqlitePool, binding: &ProjectBinding) -> anyhow::Result<bool> {
    let active: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM vault_project_bindings WHERE id = ? AND epoch = ? AND active = 1",
    )
    .bind(&binding.id)
    .bind(&binding.epoch)
    .fetch_optional(pool)
    .await?;
    Ok(active.is_some())
}

async fn current_reference_scope(
    pool: &SqlitePool,
    binding: &ProjectBinding,
    reference: &ReferenceItem,
) -> anyhow::Result<Scope> {
    let checked = checked_reference(
        pool,
        binding,
        &Selection {
            revision_id: reference.revision_id.clone(),
            expected_hash: reference.revision_hash.clone(),
        },
    )
    .await?;
    if checked.private {
        return Ok(Scope::PrivateData);
    }
    scope_for_sources(pool, std::slice::from_ref(&reference.revision_id))
        .await?
        .ok_or_else(|| anyhow::anyhow!("selected attachment has no active scope"))
}

fn scope_name(scope: &Scope) -> &'static str {
    match scope {
        Scope::PrivateData => "private-data",
        Scope::Common => "common",
        Scope::Project { .. } => "project",
    }
}

fn utf8_prefix(bytes: &[u8], limit: usize) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut end = limit.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    Some(text[..end].to_owned())
}

fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
