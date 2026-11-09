//! Immutable, task-scoped artifact snapshots for human result review.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::db;
use crate::fsapi::{self, FileKind};

const MAX_RESOURCES: usize = 64;
const MAX_PREVIEW_BYTES: usize = 8 * 1024 * 1024;
const MAX_PAYLOAD_BYTES: usize = 12 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactResourceInput {
    pub relative_path: String,
    pub expected_content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactSaveInput {
    pub source_id: String,
    pub expected_revision: i64,
    pub relative_path: String,
    pub expected_content: String,
    pub preview_html: Option<String>,
    pub unavailable: Vec<String>,
    pub resources: Vec<ArtifactResourceInput>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactVersion {
    pub id: i64,
    pub task_id: i64,
    pub source_id: String,
    pub result_revision: i64,
    pub relative_path: String,
    pub version: i64,
    pub kind: String,
    pub content_hash: String,
    pub created_at: i64,
    pub base_revision: Option<String>,
    pub unavailable: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactSnapshot {
    pub id: i64,
    pub task_id: i64,
    pub source_id: String,
    pub result_revision: i64,
    pub relative_path: String,
    pub version: i64,
    pub kind: String,
    pub content_hash: String,
    pub created_at: i64,
    pub base_revision: Option<String>,
    pub unavailable: Vec<String>,
    pub content: String,
    pub preview_html: Option<String>,
}

impl From<ArtifactSnapshot> for ArtifactVersion {
    fn from(value: ArtifactSnapshot) -> Self {
        Self {
            id: value.id,
            task_id: value.task_id,
            source_id: value.source_id,
            result_revision: value.result_revision,
            relative_path: value.relative_path,
            version: value.version,
            kind: value.kind,
            content_hash: value.content_hash,
            created_at: value.created_at,
            base_revision: value.base_revision,
            unavailable: value.unavailable,
        }
    }
}

#[derive(sqlx::FromRow)]
struct ArtifactRow {
    id: i64,
    task_id: i64,
    source_id: String,
    result_revision: i64,
    relative_path: String,
    version: i64,
    kind: String,
    content_hash: String,
    created_at: i64,
    base_revision: Option<String>,
    unavailable_json: String,
    content: String,
    preview_html: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ArtifactSummaryRow {
    id: i64,
    task_id: i64,
    source_id: String,
    result_revision: i64,
    relative_path: String,
    version: i64,
    kind: String,
    content_hash: String,
    created_at: i64,
    base_revision: Option<String>,
    unavailable_json: String,
}

impl ArtifactRow {
    fn snapshot(self) -> Result<ArtifactSnapshot, String> {
        Ok(ArtifactSnapshot {
            id: self.id,
            task_id: self.task_id,
            source_id: self.source_id,
            result_revision: self.result_revision,
            relative_path: self.relative_path,
            version: self.version,
            kind: self.kind,
            content_hash: self.content_hash,
            created_at: self.created_at,
            base_revision: self.base_revision,
            unavailable: serde_json::from_str(&self.unavailable_json)
                .map_err(|_| "저장된 산출물의 누락 리소스 정보가 손상되었습니다".to_string())?,
            content: self.content,
            preview_html: self.preview_html,
        })
    }
}

impl ArtifactSummaryRow {
    fn version(self) -> Result<ArtifactVersion, String> {
        Ok(ArtifactVersion {
            id: self.id,
            task_id: self.task_id,
            source_id: self.source_id,
            result_revision: self.result_revision,
            relative_path: self.relative_path,
            version: self.version,
            kind: self.kind,
            content_hash: self.content_hash,
            created_at: self.created_at,
            base_revision: self.base_revision,
            unavailable: serde_json::from_str(&self.unavailable_json)
                .map_err(|_| "저장된 산출물의 누락 리소스 정보가 손상되었습니다".to_string())?,
        })
    }
}

pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    for statement in [
        "CREATE TABLE IF NOT EXISTS task_review_artifacts (id INTEGER PRIMARY KEY,task_id INTEGER NOT NULL,source_id TEXT NOT NULL,result_revision INTEGER NOT NULL,relative_path TEXT NOT NULL,version INTEGER NOT NULL,kind TEXT NOT NULL CHECK(kind IN ('html','markdown','image')),content_hash TEXT NOT NULL,created_at INTEGER NOT NULL,base_revision TEXT,unavailable_json TEXT NOT NULL,resources_json TEXT NOT NULL,content TEXT NOT NULL,preview_html TEXT,UNIQUE(task_id,source_id,relative_path,version))",
        "CREATE INDEX IF NOT EXISTS idx_task_review_artifacts_list ON task_review_artifacts(task_id,source_id,relative_path,version DESC)",
        "CREATE TRIGGER IF NOT EXISTS task_review_artifacts_immutable_update BEFORE UPDATE ON task_review_artifacts BEGIN SELECT RAISE(ABORT, 'review artifact is immutable'); END",
        "CREATE TRIGGER IF NOT EXISTS task_review_artifacts_cleanup BEFORE DELETE ON tasks BEGIN DELETE FROM task_review_artifacts WHERE task_id=OLD.id; END",
    ] {
        sqlx::query(statement).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn list(pool: &SqlitePool, task_id: i64) -> Result<Vec<ArtifactVersion>, String> {
    task_exists(pool, task_id).await?;
    let rows: Vec<ArtifactSummaryRow> = sqlx::query_as(
        "SELECT id,task_id,source_id,result_revision,relative_path,version,kind,content_hash,created_at,base_revision,unavailable_json FROM task_review_artifacts WHERE task_id=? ORDER BY created_at DESC,id DESC",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;
    rows.into_iter().map(ArtifactSummaryRow::version).collect()
}

pub async fn get(
    pool: &SqlitePool,
    task_id: i64,
    artifact_id: i64,
) -> Result<ArtifactSnapshot, String> {
    task_exists(pool, task_id).await?;
    let row: Option<ArtifactRow> = sqlx::query_as(
        "SELECT id,task_id,source_id,result_revision,relative_path,version,kind,content_hash,created_at,base_revision,unavailable_json,content,preview_html FROM task_review_artifacts WHERE task_id=? AND id=?",
    )
    .bind(task_id)
    .bind(artifact_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?;
    row.ok_or_else(|| "산출물 버전을 찾을 수 없습니다".to_string())?
        .snapshot()
}

pub async fn save(
    pool: &SqlitePool,
    task_id: i64,
    input: ArtifactSaveInput,
) -> Result<ArtifactSnapshot, String> {
    let task = db::get_task(pool, task_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
    let root = Path::new(&task.worktree_path);
    let relative_path = normalize_relative(root, &input.relative_path)?;
    validate_input(&input, root)?;

    let initial = stable_result(pool, task_id, &input).await?;
    let kind = verify_files(root, &relative_path, &input)?;
    // A fingerprint can advance while resources are being read. Re-observe it,
    // then read once more so neither half of the submitted representation is
    // accepted after a local change.
    let current = stable_result(pool, task_id, &input).await?;
    verify_files(root, &relative_path, &input)?;

    let unavailable_json =
        serde_json::to_string(&input.unavailable).map_err(|error| error.to_string())?;
    let resources_json =
        serde_json::to_string(&input.resources).map_err(|error| error.to_string())?;
    let content_hash = representation_hash(&kind, &input)?;
    let created_at = now();
    // Acquire the writer before reading the next version so concurrent saves cannot share it.
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|error| error.to_string())?;
    crate::task_results::validate_snapshot_tx(&mut tx, task_id, &current).await?;
    // Keep this explicit check close to the write. It documents that the
    // initial read is only useful when both observations name the same result.
    if initial.result_revision != current.result_revision
        || initial.source_id != current.source_id
        || initial.fingerprint != current.fingerprint
    {
        return Err("결과가 변경되어 산출물을 저장할 수 없습니다 — 새로고침하세요".into());
    }
    let next: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version),0)+1 FROM task_review_artifacts WHERE task_id=? AND source_id=? AND relative_path=?",
    )
    .bind(task_id)
    .bind(&current.source_id)
    .bind(&relative_path)
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| error.to_string())?;
    let result = sqlx::query(
        "INSERT INTO task_review_artifacts(task_id,source_id,result_revision,relative_path,version,kind,content_hash,created_at,base_revision,unavailable_json,resources_json,content,preview_html) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(task_id)
    .bind(&current.source_id)
    .bind(current.result_revision)
    .bind(&relative_path)
    .bind(next)
    .bind(&kind)
    .bind(&content_hash)
    .bind(created_at)
    .bind(task.base_revision)
    .bind(&unavailable_json)
    .bind(resources_json)
    .bind(&input.expected_content)
    .bind(&input.preview_html)
    .execute(&mut *tx)
    .await
    .map_err(|error| error.to_string())?;
    let id = result.last_insert_rowid();
    tx.commit().await.map_err(|error| error.to_string())?;
    get(pool, task_id, id).await
}

async fn stable_result(
    pool: &SqlitePool,
    task_id: i64,
    input: &ArtifactSaveInput,
) -> Result<crate::task_results::TaskResultSnapshot, String> {
    let result = crate::task_results::snapshot(pool, task_id).await?;
    if result.freshness != "current" {
        return Err("실행 중이거나 관측할 수 없는 결과에는 산출물을 저장할 수 없습니다".into());
    }
    if result.source_id != input.source_id || result.result_revision != input.expected_revision {
        return Err("결과가 변경되어 산출물을 저장할 수 없습니다 — 새로고침하세요".into());
    }
    Ok(result)
}

fn validate_input(input: &ArtifactSaveInput, root: &Path) -> Result<(), String> {
    if input.source_id.is_empty() || input.expected_revision < 1 {
        return Err("산출물의 결과 식별자가 유효하지 않습니다".into());
    }
    if input.resources.len() > MAX_RESOURCES || input.unavailable.len() > MAX_RESOURCES {
        return Err("산출물 리소스는 최대 64개까지 저장할 수 있습니다".into());
    }
    if input
        .preview_html
        .as_ref()
        .is_some_and(|html| html.len() > MAX_PREVIEW_BYTES)
    {
        return Err("산출물 미리보기가 8MB를 초과합니다".into());
    }
    let mut payload = input.expected_content.len()
        + input.preview_html.as_ref().map_or(0, String::len)
        + input.unavailable.iter().map(String::len).sum::<usize>();
    let mut paths = std::collections::HashSet::new();
    for resource in &input.resources {
        let path = normalize_relative(root, &resource.relative_path)?;
        if !paths.insert(path) {
            return Err("같은 리소스를 두 번 저장할 수 없습니다".into());
        }
        payload = payload.saturating_add(resource.expected_content.len());
    }
    for unavailable in &input.unavailable {
        if unavailable.len() > 16 * 1024 || unavailable.chars().any(char::is_control) {
            return Err("미포함 리소스 정보가 유효하지 않습니다".into());
        }
    }
    if payload > MAX_PAYLOAD_BYTES {
        return Err("산출물 저장 내용이 12MB를 초과합니다".into());
    }
    Ok(())
}

fn verify_files(
    root: &Path,
    relative_path: &str,
    input: &ArtifactSaveInput,
) -> Result<String, String> {
    let main = fsapi::read_file(root, relative_path).map_err(|error| error.to_string())?;
    let kind = artifact_kind(relative_path, &main.kind)?;
    if main.content != input.expected_content {
        return Err("원본 파일이 변경되어 산출물을 저장할 수 없습니다 — 새로고침하세요".into());
    }
    match kind.as_str() {
        "image" => {
            if input.preview_html.is_some() {
                return Err("이미지 산출물에는 HTML 미리보기를 저장할 수 없습니다".into());
            }
        }
        _ => {
            let preview = input
                .preview_html
                .as_deref()
                .ok_or_else(|| "HTML과 Markdown 산출물에는 미리보기가 필요합니다".to_string())?;
            if !preview.trim_start().starts_with('<') {
                return Err("산출물 미리보기는 HTML이어야 합니다".into());
            }
        }
    }
    for resource in &input.resources {
        let path = normalize_relative(root, &resource.relative_path)?;
        let current = fsapi::read_file(root, &path).map_err(|error| error.to_string())?;
        if !matches!(current.kind, FileKind::Text | FileKind::Image) {
            return Err("산출물 리소스는 텍스트 또는 이미지 파일이어야 합니다".into());
        }
        if current.content != resource.expected_content {
            return Err("산출물 리소스가 변경되어 저장할 수 없습니다 — 새로고침하세요".into());
        }
    }
    Ok(kind)
}

fn normalize_relative(root: &Path, relative: &str) -> Result<String, String> {
    if relative.is_empty() || crate::project_paths::is_internal_path(relative) {
        return Err("산출물 경로는 작업 파일이어야 합니다".into());
    }
    let path = fsapi::safe_join(root, relative).map_err(|error| error.to_string())?;
    let canonical_root = root
        .canonicalize()
        .map_err(|_| "작업 워크트리를 해석할 수 없습니다".to_string())?;
    let relative = path
        .strip_prefix(&canonical_root)
        .map_err(|_| "산출물 경로가 작업 폴더 밖을 가리킵니다".to_string())?;
    let normalized = relative.to_string_lossy().replace('\\', "/");
    if crate::project_paths::is_internal_path(&normalized)
        || normalized.split('/').any(|part| part == ".git")
    {
        return Err("산출물 경로는 작업 파일이어야 합니다".into());
    }
    Ok(normalized)
}

fn artifact_kind(path: &str, file_kind: &FileKind) -> Result<String, String> {
    let extension = Path::new(path)
        .extension()
        .and_then(|part| part.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let kind = match extension.as_str() {
        "html" | "htm" | "xhtml" if *file_kind == FileKind::Text => "html",
        "md" | "mdown" | "mkdn" | "markdown" if *file_kind == FileKind::Text => "markdown",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg"
            if *file_kind == FileKind::Image =>
        {
            "image"
        }
        _ => return Err("지원하지 않는 산출물 파일입니다".into()),
    };
    Ok(kind.into())
}

fn representation_hash(kind: &str, input: &ArtifactSaveInput) -> Result<String, String> {
    #[derive(Serialize)]
    struct Representation<'a> {
        kind: &'a str,
        content: &'a str,
        preview_html: &'a Option<String>,
        unavailable: &'a [String],
        resources: &'a [ArtifactResourceInput],
    }
    let bytes = serde_json::to_vec(&Representation {
        kind,
        content: &input.expected_content,
        preview_html: &input.preview_html,
        unavailable: &input.unavailable,
        resources: &input.resources,
    })
    .map_err(|error| error.to_string())?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

async fn task_exists(pool: &SqlitePool, task_id: i64) -> Result<(), String> {
    let exists = db::get_task(pool, task_id)
        .await
        .map_err(|error| error.to_string())?
        .is_some();
    if exists {
        Ok(())
    } else {
        Err("작업을 찾을 수 없습니다".into())
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
