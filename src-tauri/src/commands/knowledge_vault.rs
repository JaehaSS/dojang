use std::path::Path;

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::{AppHandle, State};

use crate::knowledge::vault::{self, Scope, ScopeRequest};

use super::{now, pool_of as app_pool, AppState};

pub(super) fn pool_of(state: &AppState) -> Result<SqlitePool, String> {
    vault::require_supported().map_err(text)?;
    app_pool(state)
}

#[derive(Serialize)]
pub struct VaultDto {
    pub id: String,
    pub vault_root: String,
    pub enabled: bool,
}
#[derive(Serialize)]
pub struct BindingDto {
    pub id: String,
    pub epoch: String,
    pub canonical_repo_root: String,
}
#[derive(Serialize)]
pub struct DocumentDto {
    pub id: String,
    pub vault_id: String,
    pub kind: String,
    pub title: String,
    pub state: String,
    pub current_revision_id: Option<String>,
    pub current_revision_hash: Option<String>,
    pub current_scope: String,
}
#[derive(Serialize)]
pub struct RevisionDto {
    pub id: String,
    pub document_id: String,
    pub relative_path: String,
    pub sha256: String,
    pub size: i64,
    pub predecessor: Option<String>,
}
#[derive(Serialize)]
pub struct DocumentDetailDto {
    pub document: DocumentDto,
    pub current_revision: Option<RevisionDto>,
    pub revision_history: Vec<RevisionDto>,
    pub bounded_text: Option<String>,
    pub read_error: Option<String>,
    pub unsupported_reason: Option<String>,
    pub source_revisions: Vec<RelatedDocumentDto>,
    pub backlinks: Vec<RelatedDocumentDto>,
    pub index_status: String,
    pub index_reason: Option<String>,
}
#[derive(Serialize)]
pub struct RelatedDocumentDto {
    pub document_id: String,
    pub title: String,
    pub revision_id: String,
}
#[derive(Serialize)]
pub struct ReferenceDto {
    pub document_id: String,
    pub title: String,
    pub scope: String,
    pub revision_id: String,
    pub revision_hash: String,
    pub snippet: String,
    pub reason: String,
    pub excluded: bool,
    pub stale_reason: Option<String>,
}
#[derive(Serialize)]
pub struct SearchPageDto {
    pub hits: Vec<ReferenceDto>,
    pub has_more: bool,
}
#[derive(Serialize)]
pub struct PreviewDto {
    pub id: String,
    pub query_hash: String,
    pub created_at: i64,
    pub references: Vec<ReferenceDto>,
}
#[derive(Deserialize)]
pub struct AttachmentSelectionDto {
    pub revision_id: String,
    pub expected_hash: String,
}
#[derive(Serialize)]
pub struct ImportedSourceDto {
    pub document_id: String,
    pub revision_id: String,
    pub sha256: String,
}
#[derive(Serialize)]
pub struct ImportPathResultDto {
    pub path: String,
    pub source: Option<ImportedSourceDto>,
    pub error: Option<String>,
}
#[derive(Serialize)]
pub struct VaultScanDto {
    pub indexed: usize,
    pub skipped: usize,
    pub partial: bool,
    pub warnings: Vec<String>,
}
#[tauri::command]
pub async fn knowledge_vault_connect(
    state: State<'_, AppState>,
    vault_root: String,
) -> Result<VaultDto, String> {
    let vault = vault::register_vault(&pool_of(&state)?, Path::new(&vault_root), now())
        .await
        .map_err(text)?;
    Ok(VaultDto {
        id: vault.id,
        vault_root: vault.canonical_root,
        enabled: vault.enabled,
    })
}

#[tauri::command]
pub async fn knowledge_vault_disconnect(
    state: State<'_, AppState>,
    vault_id: String,
) -> Result<(), String> {
    sqlx::query("UPDATE vaults SET enabled = 0 WHERE id = ?")
        .bind(vault_id)
        .execute(&pool_of(&state)?)
        .await
        .map_err(text)?;
    Ok(())
}

#[tauri::command]
pub async fn knowledge_vault_list_documents(
    state: State<'_, AppState>,
    vault_id: String,
) -> Result<Vec<DocumentDto>, String> {
    documents(&pool_of(&state)?, &vault_id).await.map_err(text)
}

#[tauri::command]
pub async fn knowledge_vault_document(
    state: State<'_, AppState>,
    document_id: String,
) -> Result<DocumentDetailDto, String> {
    let pool = pool_of(&state)?;
    let document = document(&pool, &document_id).await.map_err(text)?;
    let detail = vault::document_detail(&pool, &document_id)
        .await
        .map_err(text)?
        .ok_or_else(|| "vault document is missing".to_string())?;
    let current_revision = detail.current_revision.map(|item| RevisionDto {
        id: item.id,
        document_id: item.document_id,
        relative_path: item.relative_path,
        sha256: item.sha256,
        size: item.size,
        predecessor: item.predecessor,
    });
    let revision_history = detail
        .history
        .into_iter()
        .map(|item| RevisionDto {
            id: item.id,
            document_id: item.document_id,
            relative_path: item.relative_path,
            sha256: item.sha256,
            size: item.size,
            predecessor: item.predecessor,
        })
        .collect();
    let (bounded_text, read_error, unsupported_reason) = if document.kind == "note" {
        editable_note_text(&pool, current_revision.as_ref()).await
    } else {
        preview_text(&pool, current_revision.as_ref()).await
    };
    let source_revisions = related_sources(&pool, current_revision.as_ref())
        .await
        .map_err(text)?;
    let backlinks = related_backlinks(&pool, &document.id).await.map_err(text)?;
    let (index_status, index_reason) = if detail.indexed {
        ("indexed".into(), None)
    } else {
        (
            "not-indexed".into(),
            Some(if document.state == "active" {
                "no index entry".into()
            } else {
                document.state.clone()
            }),
        )
    };
    Ok(DocumentDetailDto {
        document,
        current_revision,
        revision_history,
        bounded_text,
        read_error,
        unsupported_reason,
        source_revisions,
        backlinks,
        index_status,
        index_reason,
    })
}

#[tauri::command]
pub async fn knowledge_vault_archive(
    state: State<'_, AppState>,
    document_id: String,
    archived: bool,
) -> Result<(), String> {
    vault::archive_document(&pool_of(&state)?, &document_id, archived)
        .await
        .map_err(text)
}

#[tauri::command]
pub async fn knowledge_vault_scope(
    state: State<'_, AppState>,
    revision_id: String,
    scope: String,
    repo_root: Option<String>,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    let request = scope_request(&pool, &scope, repo_root.as_deref())
        .await
        .map_err(text)?;
    vault::change_scope(&pool, &revision_id, &request, now())
        .await
        .map_err(text)
}

#[tauri::command]
pub async fn knowledge_vault_create_note(
    state: State<'_, AppState>,
    vault_id: String,
    title: String,
    body: String,
    source_revisions: Vec<String>,
    scope: String,
    repo_root: Option<String>,
) -> Result<RevisionDto, String> {
    let pool = pool_of(&state)?;
    let request = scope_request(&pool, &scope, repo_root.as_deref())
        .await
        .map_err(text)?;
    let note = vault::create_note(
        &pool,
        &vault::NoteDraft {
            vault_id,
            title,
            body,
            target_document_id: None,
            expected_base: None,
            source_revisions,
            scope: request,
        },
        now(),
    )
    .await
    .map_err(text)?;
    revision_for_document(&pool, &note.document_id)
        .await
        .map_err(text)?
        .ok_or_else(|| "saved note has no revision".into())
}

#[tauri::command]
pub async fn knowledge_vault_update_note(
    state: State<'_, AppState>,
    vault_id: String,
    document_id: String,
    expected_base: String,
    body: String,
    source_revisions: Vec<String>,
    scope: String,
    repo_root: Option<String>,
) -> Result<RevisionDto, String> {
    let pool = pool_of(&state)?;
    let request = scope_request(&pool, &scope, repo_root.as_deref())
        .await
        .map_err(text)?;
    let document = vault::get_document(&pool, &document_id)
        .await
        .map_err(text)?
        .ok_or_else(|| "note target is missing".to_string())?;
    let note = vault::create_note(
        &pool,
        &vault::NoteDraft {
            vault_id,
            title: document.title,
            body,
            target_document_id: Some(document_id),
            expected_base: Some(expected_base),
            source_revisions,
            scope: request,
        },
        now(),
    )
    .await
    .map_err(text)?;
    revision_for_document(&pool, &note.document_id)
        .await
        .map_err(text)?
        .ok_or_else(|| "saved note has no revision".into())
}

#[tauri::command]
pub async fn knowledge_vault_import_files(
    state: State<'_, AppState>,
    vault_id: String,
    paths: Vec<String>,
    scope: Option<String>,
    repo_root: Option<String>,
) -> Result<Vec<ImportPathResultDto>, String> {
    let pool = pool_of(&state)?;
    let scope = scope_request(
        &pool,
        scope.as_deref().unwrap_or("private-data"),
        repo_root.as_deref(),
    )
    .await
    .map_err(text)?;
    let requests = paths
        .iter()
        .map(|path| vault::ImportRequest {
            vault_id: vault_id.clone(),
            source: path.into(),
            title: Path::new(path)
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("Source")
                .into(),
            scope: scope.clone(),
        })
        .collect::<Vec<_>>();
    let results = vault::import_batch(&pool, &requests, now())
        .await
        .map_err(text)?;
    Ok(paths
        .into_iter()
        .zip(results)
        .map(|(path, result)| match result {
            Ok(source) => ImportPathResultDto {
                path,
                source: Some(imported(source)),
                error: None,
            },
            Err(error) => ImportPathResultDto {
                path,
                source: None,
                error: Some(error),
            },
        })
        .collect())
}

#[tauri::command]
pub async fn knowledge_vault_text_source(
    state: State<'_, AppState>,
    vault_id: String,
    title: String,
    body: String,
    scope: Option<String>,
    repo_root: Option<String>,
) -> Result<ImportedSourceDto, String> {
    let pool = pool_of(&state)?;
    let scope = scope_request(
        &pool,
        scope.as_deref().unwrap_or("private-data"),
        repo_root.as_deref(),
    )
    .await
    .map_err(text)?;
    vault::create_text_source(
        &pool,
        &vault::TextSourceDraft {
            vault_id,
            title,
            body,
            scope,
        },
        now(),
    )
    .await
    .map(imported)
    .map_err(text)
}

#[tauri::command]
pub async fn knowledge_vault_url_source(
    state: State<'_, AppState>,
    vault_id: String,
    title: String,
    url: String,
    memo: String,
    scope: Option<String>,
    repo_root: Option<String>,
) -> Result<ImportedSourceDto, String> {
    let pool = pool_of(&state)?;
    let scope = scope_request(
        &pool,
        scope.as_deref().unwrap_or("private-data"),
        repo_root.as_deref(),
    )
    .await
    .map_err(text)?;
    vault::create_url_source(
        &pool,
        &vault::UrlSourceDraft {
            vault_id,
            title,
            url,
            memo,
            scope,
        },
        now(),
    )
    .await
    .map(imported)
    .map_err(text)
}

#[tauri::command]
pub async fn knowledge_vault_scan(
    state: State<'_, AppState>,
    vault_id: String,
    exclusions: Vec<String>,
    scope: Option<String>,
    repo_root: Option<String>,
) -> Result<VaultScanDto, String> {
    let pool = pool_of(&state)?;
    let scope = scope_request(
        &pool,
        scope.as_deref().unwrap_or("private-data"),
        repo_root.as_deref(),
    )
    .await
    .map_err(text)?;
    let exclusions = exclusions.into_iter().map(Into::into).collect::<Vec<_>>();
    let result = vault::scan_vault_with_exclusions(&pool, &vault_id, scope, &exclusions, now())
        .await
        .map_err(text)?;
    Ok(VaultScanDto {
        indexed: result.indexed,
        skipped: result.skipped,
        partial: result.partial,
        warnings: result.warnings,
    })
}

#[tauri::command]
pub async fn knowledge_vault_open_original(
    app: AppHandle,
    state: State<'_, AppState>,
    revision_id: String,
) -> Result<String, String> {
    use tauri_plugin_opener::OpenerExt;
    let path = vault::verified_original_path(&pool_of(&state)?, &revision_id)
        .await
        .map_err(text)?;
    let shown = path.to_string_lossy().into_owned();
    app.opener().open_path(&shown, None::<&str>).map_err(text)?;
    Ok(shown)
}

#[tauri::command]
pub async fn knowledge_vault_search(
    state: State<'_, AppState>,
    query: String,
    offset: i64,
) -> Result<SearchPageDto, String> {
    let pool = pool_of(&state)?;
    let page = vault::search_browse(&pool, &query, offset)
        .await
        .map_err(text)?;
    let mut references = Vec::new();
    for hit in page.hits {
        references.push(
            reference(
                &pool,
                &hit.document_id,
                &hit.revision_id,
                hit.snippet,
                hit.title,
            )
            .await
            .map_err(text)?,
        );
    }
    Ok(SearchPageDto {
        hits: references,
        has_more: page.has_more,
    })
}

#[tauri::command]
pub async fn knowledge_vault_prepare_attachments(
    state: State<'_, AppState>,
    repo_root: String,
    query: String,
    client_ref: String,
    selections: Vec<AttachmentSelectionDto>,
    input_mode: Option<String>,
) -> Result<PreviewDto, String> {
    let pool = pool_of(&state)?;
    let binding = binding(&pool, &repo_root).await.map_err(text)?;
    let input_mode = input_mode.unwrap_or_else(|| "default".into());
    if !matches!(input_mode.as_str(), "default" | "task_only") {
        return Err("selected attachments require default or task_only input mode".into());
    }
    if selections.is_empty() {
        if input_mode == "task_only" {
            vault::selected_attachments::prepare_task_only(
                &pool, &binding, &query, &client_ref, now(),
            )
            .await
            .map_err(text)?;
        } else {
            vault::selected_attachments::cancel(&pool, &binding, &client_ref, &query, now())
                .await
                .map_err(text)?;
        }
        return Ok(PreviewDto {
            id: String::new(),
            query_hash: String::new(),
            created_at: now(),
            references: Vec::new(),
        });
    }
    let selections = selections
        .into_iter()
        .map(|item| vault::selected_attachments::Selection {
            revision_id: item.revision_id,
            expected_hash: item.expected_hash,
        })
        .collect::<Vec<_>>();
    let snapshot = vault::selected_attachments::prepare(
        &pool,
        &binding,
        &query,
        &client_ref,
        &selections,
        &input_mode,
        now(),
    )
    .await
    .map_err(text)?;
    Ok(PreviewDto {
        id: snapshot.preview.id,
        query_hash: snapshot.preview.query_hash,
        created_at: snapshot.preview.created_at,
        references: snapshot.references.into_iter().map(|item| ReferenceDto {
            document_id: item.document_id,
            title: item.title,
            scope: item.scope,
            revision_id: item.item.revision_id,
            revision_hash: item.item.revision_hash,
            snippet: item.item.snippet,
            reason: item.item.reason,
            excluded: false,
            stale_reason: None,
        }).collect(),
    })
}

#[tauri::command]
pub async fn knowledge_vault_register_project(
    state: State<'_, AppState>,
    repo_root: String,
) -> Result<BindingDto, String> {
    Ok(binding_dto(
        vault::register_project(&pool_of(&state)?, Path::new(&repo_root), now())
            .await
            .map_err(text)?,
    ))
}

#[tauri::command]
pub async fn knowledge_vault_rebind_project(
    state: State<'_, AppState>,
    binding_id: String,
    repo_root: String,
) -> Result<BindingDto, String> {
    Ok(binding_dto(
        vault::rebind_project(&pool_of(&state)?, &binding_id, Path::new(&repo_root), now())
            .await
            .map_err(text)?,
    ))
}

pub(super) fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}
pub(super) async fn vaults(pool: &SqlitePool) -> anyhow::Result<Vec<VaultDto>> {
    let rows = sqlx::query("SELECT id, canonical_root, enabled FROM vaults ORDER BY registered_at")
        .fetch_all(pool)
        .await?;
    rows.iter()
        .map(|r| {
            Ok(VaultDto {
                id: r.try_get("id")?,
                vault_root: r.try_get("canonical_root")?,
                enabled: r.try_get::<i64, _>("enabled")? != 0,
            })
        })
        .collect()
}
async fn documents(pool: &SqlitePool, vault_id: &str) -> anyhow::Result<Vec<DocumentDto>> {
    let rows =
        sqlx::query("SELECT id FROM vault_documents WHERE vault_id = ? ORDER BY created_at DESC")
            .bind(vault_id)
            .fetch_all(pool)
            .await?;
    let mut items = Vec::new();
    for row in rows {
        items.push(document(pool, &row.try_get::<String, _>("id")?).await?);
    }
    Ok(items)
}
async fn document(pool: &SqlitePool, id: &str) -> anyhow::Result<DocumentDto> {
    let row = sqlx::query("SELECT id, vault_id, kind, title, state, current_revision FROM vault_documents WHERE id = ?").bind(id).fetch_one(pool).await?;
    let revision: Option<String> = row.try_get("current_revision")?;
    let current_revision_hash = match revision.as_deref() {
        Some(revision_id) => {
            sqlx::query_scalar("SELECT sha256 FROM vault_revisions WHERE id = ?")
                .bind(revision_id)
                .fetch_optional(pool)
                .await?
        }
        None => None,
    };
    let current_scope = match revision {
        Some(id) => scope_name(vault::scope_for_sources(pool, &[id]).await?),
        None => "private-data".into(),
    };
    Ok(DocumentDto {
        id: row.try_get("id")?,
        vault_id: row.try_get("vault_id")?,
        kind: row.try_get("kind")?,
        title: row.try_get("title")?,
        state: row.try_get("state")?,
        current_revision_id: row.try_get("current_revision")?,
        current_revision_hash,
        current_scope,
    })
}
async fn revision_for_document(
    pool: &SqlitePool,
    document_id: &str,
) -> anyhow::Result<Option<RevisionDto>> {
    let row = sqlx::query("SELECT r.id, r.document_id, r.relative_path, r.sha256, r.size, r.predecessor FROM vault_revisions r JOIN vault_documents d ON d.current_revision = r.id WHERE d.id = ?").bind(document_id).fetch_optional(pool).await?;
    row.map(|r| {
        Ok(RevisionDto {
            id: r.try_get("id")?,
            document_id: r.try_get("document_id")?,
            relative_path: r.try_get("relative_path")?,
            sha256: r.try_get("sha256")?,
            size: r.try_get("size")?,
            predecessor: r.try_get("predecessor")?,
        })
    })
    .transpose()
}

async fn related_sources(
    pool: &SqlitePool,
    revision: Option<&RevisionDto>,
) -> anyhow::Result<Vec<RelatedDocumentDto>> {
    let Some(revision) = revision else {
        return Ok(Vec::new());
    };
    let rows = sqlx::query("SELECT d.id AS document_id, d.title, r.id AS revision_id FROM vault_revision_sources s JOIN vault_revisions r ON r.id = s.source_revision_id JOIN vault_documents d ON d.id = r.document_id WHERE s.revision_id = ? ORDER BY d.title, r.id")
        .bind(&revision.id)
        .fetch_all(pool)
        .await?;
    rows.iter().map(related_document).collect()
}

async fn related_backlinks(
    pool: &SqlitePool,
    document_id: &str,
) -> anyhow::Result<Vec<RelatedDocumentDto>> {
    let rows = sqlx::query("SELECT DISTINCT d.id AS document_id, d.title, r.id AS revision_id FROM vault_revision_sources s JOIN vault_revisions source ON source.id = s.source_revision_id JOIN vault_revisions r ON r.id = s.revision_id JOIN vault_documents d ON d.id = r.document_id WHERE source.document_id = ? AND d.current_revision = r.id ORDER BY d.title, r.id")
        .bind(document_id)
        .fetch_all(pool)
        .await?;
    rows.iter().map(related_document).collect()
}

fn related_document(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<RelatedDocumentDto> {
    Ok(RelatedDocumentDto {
        document_id: row.try_get("document_id")?,
        title: row.try_get("title")?,
        revision_id: row.try_get("revision_id")?,
    })
}
pub(super) async fn binding(
    pool: &SqlitePool,
    root: &str,
) -> anyhow::Result<vault::ProjectBinding> {
    vault::resolve_project(pool, Path::new(root))
        .await?
        .ok_or_else(|| anyhow::anyhow!("repository is not bound to the vault"))
}
pub(super) fn binding_dto(item: vault::ProjectBinding) -> BindingDto {
    BindingDto {
        id: item.id,
        epoch: item.epoch,
        canonical_repo_root: item.canonical_root,
    }
}
pub(super) async fn scope_request(
    pool: &SqlitePool,
    scope: &str,
    repo: Option<&str>,
) -> anyhow::Result<ScopeRequest> {
    Ok(ScopeRequest {
        scope: match scope {
            "private-data" => Scope::PrivateData,
            "common" => Scope::Common,
            "project" => {
                let item = binding(
                    pool,
                    repo.ok_or_else(|| anyhow::anyhow!("project scope requires repo_root"))?,
                )
                .await?;
                Scope::Project {
                    key: item.id,
                    binding_epoch: item.epoch,
                }
            }
            _ => anyhow::bail!("unknown vault scope"),
        },
    })
}
fn scope_name(scope: Option<Scope>) -> String {
    match scope {
        Some(Scope::PrivateData) | None => "private-data".into(),
        Some(Scope::Common) => "common".into(),
        Some(Scope::Project { .. }) => "project".into(),
    }
}
async fn reference(
    pool: &SqlitePool,
    document_id: &str,
    revision_id: &str,
    snippet: String,
    title: String,
) -> anyhow::Result<ReferenceDto> {
    let revision_hash = sqlx::query_scalar("SELECT sha256 FROM vault_revisions WHERE id = ?")
        .bind(revision_id)
        .fetch_one(pool)
        .await?;
    Ok(ReferenceDto {
        document_id: document_id.into(),
        title,
        scope: scope_name(vault::scope_for_sources(pool, &[revision_id.into()]).await?),
        revision_id: revision_id.into(),
        revision_hash,
        snippet,
        reason: "search".into(),
        excluded: false,
        stale_reason: None,
    })
}
fn imported(item: vault::ImportedFile) -> ImportedSourceDto {
    ImportedSourceDto {
        document_id: item.document.id,
        revision_id: item.revision_id,
        sha256: item.sha256,
    }
}
async fn preview_text(
    pool: &SqlitePool,
    revision: Option<&RevisionDto>,
) -> (Option<String>, Option<String>, Option<String>) {
    const MAX_BYTES: i64 = 2 * 1024 * 1024;
    let Some(revision) = revision else {
        return (None, None, None);
    };
    if revision.size > MAX_BYTES {
        return (None, None, Some("text preview exceeds 2 MiB".into()));
    }
    let extension = Path::new(&revision.relative_path)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    if !matches!(extension.as_deref(), Some("md" | "txt" | "csv")) {
        return (
            None,
            None,
            Some("preview requires a .md, .txt, or .csv revision".into()),
        );
    }
    match vault::read_revision(pool, &revision.id).await {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => (Some(text), None, None),
            Err(_) => (None, None, Some("revision is binary".into())),
        },
        Err(error) => (None, Some(error.to_string()), None),
    }
}

async fn editable_note_text(
    pool: &SqlitePool,
    revision: Option<&RevisionDto>,
) -> (Option<String>, Option<String>, Option<String>) {
    let Some(revision) = revision else {
        return (None, None, None);
    };
    match vault::editable_note_body(pool, &revision.id).await {
        Ok(body) => (Some(body), None, None),
        Err(error) => (None, Some(error.to_string()), None),
    }
}
