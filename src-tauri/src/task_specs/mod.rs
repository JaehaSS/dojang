//! Searchable project Markdown and immutable, task-scoped evidence snapshots.

pub mod links;

use std::fs::{File, OpenOptions};
use std::io::{Read, Take};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::db;
use crate::fsapi;

const MAX_CONTENT_BYTES: usize = 64 * 1024;
const MAX_SEARCH_HITS: usize = 40;
const MAX_SEARCH_ENTRIES: usize = 10_000;
const MAX_SEARCH_DOCUMENTS: usize = 1_000;
const MAX_SEARCH_TIME: std::time::Duration = std::time::Duration::from_millis(1_500);
const MAX_SNIPPET_CHARS: usize = 300;
const MAX_EXPANSION_REFERENCES: usize = 4;
const MAX_EXPANDED_BYTES: usize = 128 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecHit {
    pub relative_path: String,
    pub title: String,
    pub snippet: String,
    pub line: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecSearchPage {
    pub hits: Vec<SpecHit>,
    pub truncated: bool,
    pub scanned_files: u32,
    pub skipped_files: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecDocument {
    pub relative_path: String,
    pub title: String,
    pub content_hash: String,
    pub byte_length: u64,
    pub content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecSaveInput {
    pub relative_path: String,
    pub expected_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecVersion {
    pub id: i64,
    pub task_id: i64,
    pub relative_path: String,
    pub title: String,
    pub content_hash: String,
    pub byte_length: u64,
    pub created_at: i64,
    pub base_revision: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecSnapshot {
    pub id: i64,
    pub task_id: i64,
    pub relative_path: String,
    pub title: String,
    pub content_hash: String,
    pub byte_length: u64,
    pub created_at: i64,
    pub base_revision: Option<String>,
    pub content: String,
}

impl From<SpecSnapshot> for SpecVersion {
    fn from(value: SpecSnapshot) -> Self {
        Self {
            id: value.id,
            task_id: value.task_id,
            relative_path: value.relative_path,
            title: value.title,
            content_hash: value.content_hash,
            byte_length: value.byte_length,
            created_at: value.created_at,
            base_revision: value.base_revision,
        }
    }
}

#[derive(sqlx::FromRow)]
struct SnapshotRow {
    id: i64,
    task_id: i64,
    relative_path: String,
    title: String,
    content_hash: String,
    byte_length: i64,
    created_at: i64,
    base_revision: Option<String>,
    content: String,
}

#[derive(sqlx::FromRow)]
struct SnapshotSummaryRow {
    id: i64,
    task_id: i64,
    relative_path: String,
    title: String,
    content_hash: String,
    byte_length: i64,
    created_at: i64,
    base_revision: Option<String>,
}

impl SnapshotRow {
    fn snapshot(self) -> Result<SpecSnapshot, String> {
        let byte_length = u64::try_from(self.byte_length)
            .map_err(|_| "저장된 명세의 크기 정보가 손상되었습니다".to_string())?;
        Ok(SpecSnapshot {
            id: self.id,
            task_id: self.task_id,
            relative_path: self.relative_path,
            title: self.title,
            content_hash: self.content_hash,
            byte_length,
            created_at: self.created_at,
            base_revision: self.base_revision,
            content: self.content,
        })
    }
}

impl SnapshotSummaryRow {
    fn version(self) -> Result<SpecVersion, String> {
        Ok(SpecVersion {
            id: self.id,
            task_id: self.task_id,
            relative_path: self.relative_path,
            title: self.title,
            content_hash: self.content_hash,
            byte_length: u64::try_from(self.byte_length)
                .map_err(|_| "저장된 명세의 크기 정보가 손상되었습니다".to_string())?,
            created_at: self.created_at,
            base_revision: self.base_revision,
        })
    }
}

pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    for statement in [
        "CREATE TABLE IF NOT EXISTS task_spec_snapshots (id INTEGER PRIMARY KEY,task_id INTEGER NOT NULL,relative_path TEXT NOT NULL,title TEXT NOT NULL,content_hash TEXT NOT NULL,byte_length INTEGER NOT NULL,created_at INTEGER NOT NULL,base_revision TEXT,content TEXT NOT NULL)",
        "CREATE INDEX IF NOT EXISTS idx_task_spec_snapshots_list ON task_spec_snapshots(task_id,created_at DESC,id DESC)",
        "CREATE TRIGGER IF NOT EXISTS task_spec_snapshots_immutable_update BEFORE UPDATE ON task_spec_snapshots BEGIN SELECT RAISE(ABORT, 'task spec snapshot is immutable'); END",
        "CREATE TRIGGER IF NOT EXISTS task_spec_snapshots_cleanup BEFORE DELETE ON tasks BEGIN DELETE FROM task_spec_snapshots WHERE task_id=OLD.id; END",
    ] {
        sqlx::query(statement).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn search(
    pool: &SqlitePool,
    task_id: i64,
    query: &str,
) -> Result<SpecSearchPage, String> {
    let query = validate_query(query)?;
    if query.is_empty() {
        return Ok(SpecSearchPage {
            hits: Vec::new(),
            truncated: false,
            scanned_files: 0,
            skipped_files: 0,
        });
    }
    let task = task(pool, task_id).await?;
    tokio::task::spawn_blocking(move || search_blocking(PathBuf::from(task.worktree_path), &query))
        .await
        .map_err(|_| "명세 검색 작업이 중단되었습니다".to_string())?
}

pub async fn read(
    pool: &SqlitePool,
    task_id: i64,
    relative_path: &str,
) -> Result<SpecDocument, String> {
    let task = task(pool, task_id).await?;
    let relative_path = relative_path.to_string();
    tokio::task::spawn_blocking(move || {
        read_document(Path::new(&task.worktree_path), &relative_path)
    })
    .await
    .map_err(|_| "명세 파일 읽기 작업이 중단되었습니다".to_string())?
}

pub async fn save(
    pool: &SqlitePool,
    task_id: i64,
    input: SpecSaveInput,
) -> Result<SpecSnapshot, String> {
    validate_hash(&input.expected_hash)?;
    let task = task(pool, task_id).await?;
    let root = task.worktree_path.clone();
    let path = input.relative_path.clone();
    let document = tokio::task::spawn_blocking(move || read_document(Path::new(&root), &path))
        .await
        .map_err(|_| "명세 파일 읽기 작업이 중단되었습니다".to_string())??;
    if document.content_hash != input.expected_hash {
        return Err("원본 명세가 변경되어 보존할 수 없습니다. 새로고침하세요".into());
    }

    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|error| error.to_string())?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?)")
        .bind(task_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
    if !exists {
        return Err("작업을 찾을 수 없습니다".into());
    }
    let result = sqlx::query(
        "INSERT INTO task_spec_snapshots(task_id,relative_path,title,content_hash,byte_length,created_at,base_revision,content) VALUES(?,?,?,?,?,?,?,?)",
    )
    .bind(task_id)
    .bind(&document.relative_path)
    .bind(&document.title)
    .bind(&document.content_hash)
    .bind(i64::try_from(document.byte_length).map_err(|_| "명세 파일이 너무 큽니다")?)
    .bind(now())
    .bind(task.base_revision)
    .bind(&document.content)
    .execute(&mut *tx)
    .await
    .map_err(|error| error.to_string())?;
    let id = result.last_insert_rowid();
    let snapshot = snapshot_by_id(&mut tx, task_id, id).await?;
    tx.commit().await.map_err(|error| error.to_string())?;
    Ok(snapshot)
}

pub async fn list(pool: &SqlitePool, task_id: i64) -> Result<Vec<SpecVersion>, String> {
    task(pool, task_id).await?;
    let rows: Vec<SnapshotSummaryRow> = sqlx::query_as(
        "SELECT id,task_id,relative_path,title,content_hash,byte_length,created_at,base_revision FROM task_spec_snapshots WHERE task_id=? ORDER BY created_at DESC,id DESC",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;
    rows.into_iter().map(SnapshotSummaryRow::version).collect()
}

pub async fn get(
    pool: &SqlitePool,
    task_id: i64,
    snapshot_id: i64,
) -> Result<SpecSnapshot, String> {
    task(pool, task_id).await?;
    let row: Option<SnapshotRow> = sqlx::query_as(
        "SELECT id,task_id,relative_path,title,content_hash,byte_length,created_at,base_revision,content FROM task_spec_snapshots WHERE task_id=? AND id=?",
    )
    .bind(task_id)
    .bind(snapshot_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?;
    row.ok_or_else(|| "보존한 명세를 찾을 수 없습니다".to_string())?
        .snapshot()
}

/// Resolve evidence tokens from durable snapshots.  It deliberately performs
/// only one parse pass, so a token in a stored document remains data.
pub async fn expand_message(
    pool: &SqlitePool,
    task_id: i64,
    message: &str,
) -> Result<String, String> {
    let marker = "[[dojang-spec:";
    if message.contains(marker) && message.len() > MAX_EXPANDED_BYTES {
        return Err("명세 참조를 확장한 메시지가 128KiB를 초과합니다".into());
    }
    let mut cursor = 0;
    let mut output = String::new();
    let mut references = 0;
    while let Some(found) = message[cursor..].find(marker) {
        let start = cursor + found;
        output.push_str(&message[cursor..start]);
        let token_start = start + marker.len();
        let Some(end_relative) = message[token_start..].find("]]") else {
            return Err("명세 참조 형식이 올바르지 않습니다".into());
        };
        let end = token_start + end_relative;
        let token = &message[token_start..end];
        let (snapshot_id, hash) = parse_token(token)?;
        references += 1;
        if references > MAX_EXPANSION_REFERENCES {
            return Err("한 메시지에는 명세 참조를 최대 4개까지 넣을 수 있습니다".into());
        }
        if references == 1 {
            task(pool, task_id).await?;
        }
        let row: Option<SnapshotRow> = sqlx::query_as(
            "SELECT id,task_id,relative_path,title,content_hash,byte_length,created_at,base_revision,content FROM task_spec_snapshots WHERE task_id=? AND id=? AND content_hash=?",
        )
        .bind(task_id)
        .bind(snapshot_id)
        .bind(hash)
        .fetch_optional(pool)
        .await
        .map_err(|error| error.to_string())?;
        let snapshot = row
            .ok_or_else(|| "명세 참조가 이 작업의 보존 기록과 일치하지 않습니다".to_string())?
            .snapshot()?;
        if hash_text(&snapshot.content) != snapshot.content_hash {
            return Err("보존한 명세의 해시가 일치하지 않습니다".into());
        }
        output.push_str(&expanded_snapshot(&snapshot));
        if output.len() > MAX_EXPANDED_BYTES {
            return Err("명세 참조를 확장한 메시지가 128KiB를 초과합니다".into());
        }
        cursor = end + 2;
    }
    if references == 0 {
        return Ok(message.to_string());
    }
    output.push_str(&message[cursor..]);
    if output.len() > MAX_EXPANDED_BYTES {
        return Err("명세 참조를 확장한 메시지가 128KiB를 초과합니다".into());
    }
    Ok(output)
}

async fn task(pool: &SqlitePool, task_id: i64) -> Result<db::Task, String> {
    db::get_task(pool, task_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())
}

async fn snapshot_by_id(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    task_id: i64,
    snapshot_id: i64,
) -> Result<SpecSnapshot, String> {
    let row: SnapshotRow = sqlx::query_as(
        "SELECT id,task_id,relative_path,title,content_hash,byte_length,created_at,base_revision,content FROM task_spec_snapshots WHERE task_id=? AND id=?",
    )
    .bind(task_id)
    .bind(snapshot_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| error.to_string())?;
    row.snapshot()
}

fn validate_query(query: &str) -> Result<String, String> {
    if query.len() > 200 {
        return Err("명세 검색어는 UTF-8 200바이트 이하여야 합니다".into());
    }
    let query = query.trim().to_string();
    if query.is_empty() || query.chars().count() < 2 {
        Ok(String::new())
    } else {
        Ok(query)
    }
}

fn search_blocking(root: PathBuf, query: &str) -> Result<SpecSearchPage, String> {
    let canonical_root = root
        .canonicalize()
        .map_err(|_| "작업 워크트리를 해석할 수 없습니다".to_string())?;
    let folded = query.to_lowercase();
    let started = Instant::now();
    let mut hits = Vec::new();
    let mut scanned = 0u32;
    let mut skipped = 0u32;
    let mut truncated = false;
    let walker = ignore::WalkBuilder::new(&canonical_root)
        .hidden(true)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .require_git(false)
        .parents(false)
        .follow_links(false)
        .build();

    for (entries, result) in walker.enumerate() {
        if entries >= MAX_SEARCH_ENTRIES || scanned as usize >= MAX_SEARCH_DOCUMENTS {
            truncated = true;
            break;
        }
        if started.elapsed() > MAX_SEARCH_TIME {
            truncated = true;
            break;
        }
        let Ok(entry) = result else {
            skipped = skipped.saturating_add(1);
            continue;
        };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let relative = match normalized_markdown_path(&canonical_root, entry.path()) {
            Ok(path) => path,
            Err(_) => continue,
        };
        let document = match read_document(&canonical_root, &relative) {
            Ok(document) => document,
            Err(_) => {
                skipped = skipped.saturating_add(1);
                continue;
            }
        };
        scanned = scanned.saturating_add(1);
        let mut body_match = false;
        for (index, line) in document.content.lines().enumerate() {
            if !line.to_lowercase().contains(&folded) {
                continue;
            }
            body_match = true;
            if hits.len() == MAX_SEARCH_HITS {
                truncated = true;
                break;
            }
            hits.push(SpecHit {
                relative_path: document.relative_path.clone(),
                title: document.title.clone(),
                snippet: clip(line),
                line: (index + 1) as u32,
            });
        }
        if !body_match
            && (document.relative_path.to_lowercase().contains(&folded)
                || document.title.to_lowercase().contains(&folded))
        {
            if hits.len() == MAX_SEARCH_HITS {
                truncated = true;
            } else {
                let title_match = document.title.to_lowercase().contains(&folded);
                hits.push(SpecHit {
                    relative_path: document.relative_path.clone(),
                    title: document.title.clone(),
                    snippet: if title_match {
                        document.title.clone()
                    } else {
                        document.relative_path.clone()
                    },
                    line: title_line(&document.content).unwrap_or(1),
                });
            }
        }
        if truncated {
            break;
        }
    }
    Ok(SpecSearchPage {
        hits,
        truncated,
        scanned_files: scanned,
        skipped_files: skipped,
    })
}

fn read_document(root: &Path, relative_path: &str) -> Result<SpecDocument, String> {
    let relative_path = normalize_markdown_path(root, relative_path)?;
    let path = fsapi::safe_join(root, &relative_path).map_err(|error| error.to_string())?;
    let canonical_root = root
        .canonicalize()
        .map_err(|_| "작업 워크트리를 해석할 수 없습니다".to_string())?;
    reject_symlink_components(&canonical_root, &path)?;
    let metadata = path.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("명세 경로가 파일이 아닙니다".into());
    }
    let content = read_bounded_utf8(&path)?;
    Ok(SpecDocument {
        title: title(&content, &relative_path),
        content_hash: hash_text(&content),
        byte_length: content.len() as u64,
        relative_path,
        content,
    })
}

fn read_bounded_utf8(path: &Path) -> Result<String, String> {
    if path
        .symlink_metadata()
        .map_err(|error| error.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("심볼릭 링크 대상은 허용되지 않습니다".into());
    }
    let file = open_without_following_links(path)?;
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("명세 경로가 파일이 아닙니다".into());
    }
    let mut reader: Take<File> = file.take((MAX_CONTENT_BYTES + 1) as u64);
    let mut bytes = Vec::with_capacity(MAX_CONTENT_BYTES + 1);
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_CONTENT_BYTES {
        return Err("명세 파일이 64KiB를 초과합니다".into());
    }
    String::from_utf8(bytes).map_err(|_| "명세 파일은 UTF-8이어야 합니다".into())
}

fn normalize_markdown_path(root: &Path, relative: &str) -> Result<String, String> {
    if relative.is_empty()
        || relative.len() > 1024
        || relative.chars().any(char::is_control)
        || crate::project_paths::is_internal_path(relative)
    {
        return Err("명세 경로는 작업 파일이어야 합니다".into());
    }
    let path = fsapi::safe_join(root, relative).map_err(|error| error.to_string())?;
    let canonical_root = root
        .canonicalize()
        .map_err(|_| "작업 워크트리를 해석할 수 없습니다".to_string())?;
    reject_symlink_components(&canonical_root, &path)?;
    normalized_markdown_path(&canonical_root, &path)
}

/// `safe_join` proves a path stays below the worktree, but an in-tree alias
/// can still point at `.git` or app-managed data.  Specs never cross any link.
fn reject_symlink_components(root: &Path, path: &Path) -> Result<(), String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "명세 경로가 작업 폴더 밖을 가리킵니다".to_string())?;
    let mut cursor = root.to_path_buf();
    for component in relative.components() {
        cursor.push(component);
        let metadata = cursor
            .symlink_metadata()
            .map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err("심볼릭 링크 대상은 허용되지 않습니다".into());
        }
    }
    Ok(())
}

fn normalized_markdown_path(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "명세 경로가 작업 폴더 밖을 가리킵니다".to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    if relative.is_empty()
        || relative.len() > 1024
        || relative.chars().any(char::is_control)
        || crate::project_paths::is_internal_path(&relative)
        || relative.split('/').any(|part| part == ".git")
        || !is_markdown(&relative)
    {
        return Err("Markdown 작업 파일만 선택할 수 있습니다".into());
    }
    Ok(relative)
}

fn is_markdown(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .as_deref(),
        Some("md" | "markdown" | "mdown" | "mkdn")
    )
}

fn title(content: &str, relative_path: &str) -> String {
    for line in content.lines() {
        let candidate = line.trim_start();
        let heading = candidate.trim_start_matches('#');
        if heading.len() != candidate.len() && heading.starts_with(char::is_whitespace) {
            let heading = heading.trim();
            if !heading.is_empty() {
                return clip(heading);
            }
        }
    }
    Path::new(relative_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(relative_path)
        .to_string()
}

fn title_line(content: &str) -> Option<u32> {
    content.lines().enumerate().find_map(|(index, line)| {
        let candidate = line.trim_start();
        let heading = candidate.trim_start_matches('#');
        (heading.len() != candidate.len() && heading.starts_with(char::is_whitespace))
            .then_some((index + 1) as u32)
    })
}

fn clip(value: &str) -> String {
    if value.chars().count() <= MAX_SNIPPET_CHARS {
        return value.to_string();
    }
    let mut clipped: String = value.chars().take(MAX_SNIPPET_CHARS).collect();
    clipped.push('…');
    clipped
}

fn validate_hash(hash: &str) -> Result<(), String> {
    if hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("명세 해시는 64자리 SHA-256 16진수여야 합니다".into())
    }
}

fn hash_text(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

fn open_without_following_links(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW);
    }
    options.open(path).map_err(|error| error.to_string())
}

fn parse_token(token: &str) -> Result<(i64, &str), String> {
    let mut parts = token.split(':');
    let id = parts
        .next()
        .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or_else(|| "명세 참조 형식이 올바르지 않습니다".to_string())?
        .parse::<i64>()
        .map_err(|_| "명세 참조 형식이 올바르지 않습니다".to_string())?;
    let hash = parts
        .next()
        .ok_or_else(|| "명세 참조 형식이 올바르지 않습니다".to_string())?;
    if parts.next().is_some() {
        return Err("명세 참조 형식이 올바르지 않습니다".into());
    }
    if id <= 0 {
        return Err("명세 참조 형식이 올바르지 않습니다".into());
    }
    validate_hash(hash)?;
    Ok((id, hash))
}

fn expanded_snapshot(snapshot: &SpecSnapshot) -> String {
    format!(
        "\n\n--- dojang-spec snapshot data ---\nid: {}\npath: {}\nsha256: {}\ncreated_at: {}\nbody:\n{}\n--- end dojang-spec snapshot data ---\n",
        snapshot.id,
        snapshot.relative_path,
        snapshot.content_hash,
        snapshot.created_at,
        snapshot.content,
    )
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
