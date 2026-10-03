//! Personal knowledge vault domain service.
//!
//! The module owns vault files and their durable catalog.  It deliberately
//! does not depend on Tauri so callers can enforce the desktop boundary.

mod bindings;
mod catalog;
mod files;
mod index;
mod notes;
mod operations;
pub mod ownership;
mod platform;
pub mod provenance;
mod recovery;
pub mod retrieval;
pub mod runtime_schema;
mod scan;
mod schema;
mod scope;
pub(crate) mod selected_attachments;
pub mod settings;
pub(crate) mod task_retention;
pub mod usage;
mod wiki;
pub mod workspace;

pub use admission::{exclusive as exclusive_admission, shared as shared_admission, AdmissionGuard};
pub use bindings::{
    add_worktree, rebind_project, register_project, resolve_project, ProjectBinding,
};
pub use catalog::{
    archive_document, change_scope, create_document, current_revision, document_detail,
    get_document, list_documents, rebind_vault, register_vault, DocumentDetail, DocumentDraft,
    Vault, VaultDocument,
};
pub use files::{
    create_text_source, create_url_source, import_batch, import_file, read_revision,
    verified_original_path, ImportRequest, ImportedFile, TextSourceDraft, UrlSourceDraft,
};
pub use index::{
    index_revision, rebuild_needed, search, search_browse, VaultSearchHit, VaultSearchPage,
};
pub use notes::{create_note, editable_note_body, NoteDraft, SavedNote};
pub use operations::{
    commit_operation, mark_files_ready, prepare_operation, write_operation_files, OperationPlan,
    PlannedRevision,
};
pub use platform::require_supported;
pub use recovery::{recover_operations, RecoveryReport};
pub use scan::{scan_vault, scan_vault_with_exclusions, VaultScanResult};
pub use scope::{scope_allows_binding, scope_for_sources, Scope, ScopeRequest, SourceRef};
pub use settings::VaultSettings;

use sqlx::SqlitePool;

/// 연결된(활성·쓰기 가능) 창고의 검증된 루트. 연결된 창고가 없으면 `None`.
///
/// 루트 정체(장치·inode)가 등록 때와 다르면 오류다 — 조용히 다른 폴더를 읽지 않는다.
/// 창고 밖에서 "지금 창고 파일을 읽어도 되는가"를 묻는 곳(대기 인사이트 카드)이 쓴다.
pub async fn active_vault_root(pool: &SqlitePool) -> anyhow::Result<Option<std::path::PathBuf>> {
    let id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM vaults WHERE enabled = 1 AND writable = 1 ORDER BY registered_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    match id {
        Some(id) => Ok(Some(files::vault_root(pool, &id).await?.path)),
        None => Ok(None),
    }
}

/// Create the vault schema.  The application calls this with other local
/// migrations; tests call it directly to keep the domain independently usable.
pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::raw_sql(schema::MIGRATION).execute(pool).await?;
    ensure_operation_columns(pool).await?;
    ensure_document_event_columns(pool).await?;
    runtime_schema::migrate(pool).await?;
    index::migrate_projection_layout(pool).await?;
    drop_proposal_tables_once(pool).await?;
    Ok(())
}

/// 옛 제안 파이프라인이 `vault_operations`에 남긴 두 컬럼. 표 자체는 남으므로
/// 구버전 DB에도 채워 둔다 — 제안은 사라졌어도 `accepts_drift`는 쓰인다.
async fn ensure_operation_columns(pool: &SqlitePool) -> anyhow::Result<()> {
    let operation_columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('vault_operations')")
            .fetch_all(pool)
            .await?;
    if !operation_columns
        .iter()
        .any(|column| column == "proposal_candidate_hash")
    {
        sqlx::query("ALTER TABLE vault_operations ADD COLUMN proposal_candidate_hash TEXT")
            .execute(pool)
            .await?;
    }
    if !operation_columns
        .iter()
        .any(|column| column == "accepts_drift")
    {
        sqlx::query(
            "ALTER TABLE vault_operations ADD COLUMN accepts_drift INTEGER NOT NULL DEFAULT 0",
        )
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// 수동 분석·제안 큐·잡 파이프라인과 WikiSkill이 쓰던 표를 **한 번** 떨군다
/// (계획 2026-09-13). `memory::file::purge_legacy_once`와 같은 방식 — 설정
/// 플래그 하나로 재실행을 막는다.
///
/// `vault_operations.proposal_id`가 `vault_proposals`를 FK로 참조하므로
/// 부모를 먼저 지우면 **그 뒤 모든 `vault_operations` INSERT가**
/// `no such table: main.vault_proposals`로 실패한다(NULL을 넣어도 그렇다).
/// 그래서 참조 절을 뗀 표로 먼저 갈아끼운다.
async fn drop_proposal_tables_once(pool: &SqlitePool) -> anyhow::Result<()> {
    const FLAG: &str = "vault_proposal_tables_dropped";
    // 한 번만 도는 표시는 `settings`에 산다. 앱 DB에는 항상 있지만 창고 스키마만
    // 올린 테스트 풀에는 없다 — 그런 DB에는 치울 옛 표도 없으므로 조용히 지나간다.
    let has_settings: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'settings')",
    )
    .fetch_one(pool)
    .await?;
    if !has_settings {
        return Ok(());
    }
    if crate::db::get_setting(pool, FLAG).await?.is_some() {
        return Ok(());
    }
    let mut conn = pool.acquire().await?;
    sqlx::raw_sql(schema::REBUILD_OPERATIONS_WITHOUT_PROPOSALS)
        .execute(&mut *conn)
        .await?;
    for table in [
        "vault_proposal_sources",
        "vault_proposal_snapshots",
        "vault_proposals",
        "vault_manual_review_sources",
        "vault_manual_jobs",
        "vault_manual_reviews",
        "vault_jobs",
        "vault_model_slots",
        "wikiskill_active",
        "wikiskill_versions",
        "wikiskill_evaluations",
        "wikiskill_trials",
        "wikiskill_proposals",
        "wikiskill_patterns",
        "wikiskill_pattern_revisions",
        "wikiskill_snapshots",
        "wikiskill_projects",
    ] {
        sqlx::query(&format!("DROP TABLE IF EXISTS {table}"))
            .execute(&mut *conn)
            .await?;
    }
    drop(conn);
    crate::db::set_setting(pool, FLAG, "true").await?;
    Ok(())
}

async fn ensure_document_event_columns(pool: &SqlitePool) -> anyhow::Result<()> {
    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('vault_document_events')")
            .fetch_all(pool)
            .await?;
    if !columns.iter().any(|column| column == "previous_hash") {
        sqlx::query("ALTER TABLE vault_document_events ADD COLUMN previous_hash TEXT")
            .execute(pool)
            .await?;
    }
    Ok(())
}

mod admission;
#[cfg(all(test, target_os = "macos"))]
mod tests;
