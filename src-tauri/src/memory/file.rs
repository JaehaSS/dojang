//! 파일형 메모리 — 저장소마다 상한 있는 `MEMORY.md` 하나가 정본이다.
//!
//! 설계: `docs/designs/2026-09-13-memory-is-a-file-in-the-vault.md`.
//! DB는 **색인·상태**만 가진다(`memory_files`). 본문은 절대 DB에 넣지 않는다(R8).
//! 옛 추출 파이프라인(`memory::projection`·`capture`·`selfimprove`)은 P1에서 호출이 끊기고
//! P2에서 지워진다 — 이 모듈이 그 자리를 전부 대신한다.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

use super::{MARK_END, MARK_START};

/// 설정 키 — 전부 범용 `settings` 테이블(`db::get_setting`).
pub const SETTING_ROOT: &str = "memory_root";
pub const SETTING_CAP_LINES: &str = "memory_cap_lines";
pub const SETTING_CAP_BYTES: &str = "memory_cap_bytes";
pub const SETTING_USER_CAP_LINES: &str = "memory_user_cap_lines";
pub const SETTING_USER_CAP_BYTES: &str = "memory_user_cap_bytes";

pub const DEFAULT_CAP_LINES: u32 = 100;
pub const DEFAULT_CAP_BYTES: u64 = 8192;
pub const DEFAULT_USER_CAP_LINES: u32 = 40;
pub const DEFAULT_USER_CAP_BYTES: u64 = 3072;

/// 사람이 손으로 넣을 수 있는 상한의 상한. 블록은 AGENTS.md에 *추가로* 실리므로
/// 여기서 막지 않으면 컨텍스트 전체가 메모리로 채워진다.
pub const MAX_CAP_LINES: u32 = 2000;
pub const MAX_CAP_BYTES: u64 = 262_144;

pub const USER_FILE: &str = "USER.md";
pub const REPO_FILE: &str = "MEMORY.md";

pub const KIND_USER: &str = "user";
pub const KIND_REPO: &str = "repo";

/// 파일 하나의 상한. 줄 수와 바이트를 **함께** 본다 — 한 줄이 몇 KB인 덤프가 실제로 있었다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub lines: u32,
    pub bytes: u64,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            lines: DEFAULT_CAP_LINES,
            bytes: DEFAULT_CAP_BYTES,
        }
    }
}

impl Caps {
    pub fn user_default() -> Self {
        Self {
            lines: DEFAULT_USER_CAP_LINES,
            bytes: DEFAULT_USER_CAP_BYTES,
        }
    }
}

/// 상한까지만 읽어 온 본문 + 원본의 크기. `total_*`는 파일 전체이고 `body`는 잘린 뒤다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capped {
    pub body: String,
    pub total_lines: u32,
    pub total_bytes: u64,
    pub truncated_lines: u32,
}

/// Wiki 화면이 읽는 한 행. 본문은 없다 — 정본은 파일이다.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryFileInfo {
    pub path: String,
    /// `"user"` | `"repo"`.
    pub kind: String,
    pub repo: Option<String>,
    pub repo_key: Option<String>,
    pub exists: bool,
    pub lines: u32,
    pub bytes: u64,
    pub cap_lines: u32,
    pub cap_bytes: u64,
    pub modified_at: Option<i64>,
    pub last_projected_at: Option<i64>,
    pub last_task_id: Option<i64>,
}

/// `<basename>-<sha256(절대경로)[..8]>`. 같은 이름의 저장소가 여러 개여도 갈라지고,
/// 사람이 폴더 이름만 보고 어느 저장소인지 알 수 있다.
///
/// worktree는 자동으로 같은 키를 쓴다 — `task.repo`가 언제나 메인 저장소 루트이고
/// `.praxis/worktrees/` 아래 경로는 `worktree_path`로 따로 들고 다니기 때문이다.
pub fn repo_key(repo: &str) -> String {
    let trimmed = repo.trim();
    let trimmed = trimmed.trim_end_matches('/');
    if trimmed.is_empty() {
        return "unknown-00000000".to_string();
    }
    let digest = format!("{:x}", Sha256::digest(trimmed.as_bytes()));
    let base = Path::new(trimmed)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "repo".to_string());
    let base = sanitize_component(&base);
    format!("{base}-{}", &digest[..8])
}

/// 폴더 이름에 쓸 수 없는 문자를 접는다 — 키는 디렉터리 이름이 된다.
fn sanitize_component(value: &str) -> String {
    let folded: String = value
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let folded = folded.trim_matches('-').to_string();
    if folded.is_empty() {
        "repo".to_string()
    } else {
        folded
    }
}

/// 풀이 열고 있는 DB 파일의 디렉터리 = 앱 데이터 디렉터리.
///
/// Runner는 Tauri가 없어 `app_data_dir()`을 부를 수 없고, 로컬 앱은 DB를 정확히 그 디렉터리에
/// 만든다(`lib.rs`). 경로 하나를 두 경로로 구하지 않기 위해 양쪽 다 이 함수를 쓴다.
pub fn data_dir(pool: &SqlitePool) -> PathBuf {
    let options = pool.connect_options();
    let filename = options.get_filename().to_path_buf();
    match filename.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

async fn setting(pool: &SqlitePool, key: &str) -> Option<String> {
    crate::db::get_setting(pool, key).await.ok().flatten()
}

fn parse_cap_lines(raw: Option<String>, fallback: u32) -> u32 {
    raw.and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|value| *value > 0 && *value <= MAX_CAP_LINES)
        .unwrap_or(fallback)
}

fn parse_cap_bytes(raw: Option<String>, fallback: u64) -> u64 {
    raw.and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0 && *value <= MAX_CAP_BYTES)
        .unwrap_or(fallback)
}

/// 저장소 파일 상한(설정 → 기본값).
pub async fn caps(pool: &SqlitePool) -> Caps {
    Caps {
        lines: parse_cap_lines(setting(pool, SETTING_CAP_LINES).await, DEFAULT_CAP_LINES),
        bytes: parse_cap_bytes(setting(pool, SETTING_CAP_BYTES).await, DEFAULT_CAP_BYTES),
    }
}

/// 전역(USER.md) 상한 — 저장소 파일보다 좁다. 모든 저장소에 실리기 때문이다.
pub async fn user_caps(pool: &SqlitePool) -> Caps {
    Caps {
        lines: parse_cap_lines(
            setting(pool, SETTING_USER_CAP_LINES).await,
            DEFAULT_USER_CAP_LINES,
        ),
        bytes: parse_cap_bytes(
            setting(pool, SETTING_USER_CAP_BYTES).await,
            DEFAULT_USER_CAP_BYTES,
        ),
    }
}

/// 메모리 루트. 설정 > 창고(`<canonical_root>/memory`) > 앱 데이터 디렉터리 순.
///
/// **디렉터리를 만들지 않는다** — 만드는 것은 사람이 여는 순간(커맨드)이나 에이전트다.
/// 조회만으로 폴더가 생기면 창고를 옮긴 뒤 빈 껍데기가 남는다.
pub async fn memory_root(pool: &SqlitePool, data_dir: &Path) -> PathBuf {
    if let Some(root) = setting(pool, SETTING_ROOT).await {
        let root = root.trim();
        if !root.is_empty() {
            return PathBuf::from(root);
        }
    }
    if let Some(vault) = active_vault_root(pool).await {
        return PathBuf::from(vault).join("memory");
    }
    data_dir.join("memory")
}

async fn active_vault_root(pool: &SqlitePool) -> Option<String> {
    // 창고 스키마가 아직 없는 DB(테스트·Runner 초기 부팅)에서도 조용히 넘어간다.
    sqlx::query_scalar::<_, String>(
        "SELECT canonical_root FROM vaults WHERE enabled = 1 \
         ORDER BY writable DESC, registered_at ASC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

pub fn user_path(root: &Path) -> PathBuf {
    root.join(USER_FILE)
}

pub fn repo_path(root: &Path, repo_key: &str) -> PathBuf {
    root.join(repo_key).join(REPO_FILE)
}

/// 상한 안쪽까지만 읽는다. 파일이 없으면 `Ok(None)`.
///
/// 두 상한 중 **먼저 걸리는 쪽**에서 자르고, 자르는 자리는 언제나 줄 경계다 —
/// 바이트 중간에서 자르면 마지막 항목이 반만 실려 뜻이 뒤집힌다.
pub fn read_capped(path: &Path, caps: &Caps) -> std::io::Result<Option<Capped>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let total_bytes = raw.len() as u64;
    let lines: Vec<&str> = raw.lines().collect();
    let total_lines = lines.len() as u32;
    let mut kept: Vec<&str> = Vec::new();
    let mut bytes: u64 = 0;
    for line in &lines {
        if kept.len() as u32 >= caps.lines {
            break;
        }
        let next = bytes + line.len() as u64 + 1;
        if next > caps.bytes && !kept.is_empty() {
            break;
        }
        kept.push(line);
        bytes = next;
    }
    let truncated_lines = total_lines - kept.len() as u32;
    let mut body = kept.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    Ok(Some(Capped {
        body,
        total_lines,
        total_bytes,
        truncated_lines,
    }))
}

/// 마커 문자열이 본문에 그대로 있으면 블록 경계가 그 자리에서 끊긴다 — capsule과 같은 방식으로 접는다.
fn escape_managed_markers(value: &str) -> String {
    value
        .replace(MARK_START, "&lt;!-- PRAXIS MEMORY START -->")
        .replace(MARK_END, "&lt;!-- PRAXIS MEMORY END -->")
}

fn human_bytes(bytes: u64) -> String {
    if bytes % 1024 == 0 {
        format!("{}KB", bytes / 1024)
    } else {
        format!("{bytes}B")
    }
}

// ── 쓰기 API (발견 공간 §8 "메모리에 남기기", T4) ────────────────────
//
// 지금까지 이 모듈에는 쓰기 API가 없었다(투영·종료 기록·목록만). 카드 행동 "메모리에
// 남기기"가 처음 붙이는 쓰기다 — 경로는 `memory_root` + `user_path`/`repo_path`로만
// 만들고, 커맨드가 경로 인자를 직접 받지 않는다(설계 §8).

/// 메모리에 남길 대상 — 저장소에 묶이지 않는 신호는 `User`, 저장소에 묶인 신호는 그
/// 저장소의 `MEMORY.md`(`Repo`)다(설계 §8 O2).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum MemoryTarget {
    User,
    Repo { repo: String },
}

/// 추가 결과. 상한을 넘기면 쓰지 않고 `Full`을 돌려준다 — 호출자는 그 경로를 에디터로
/// 열어 사람이 합치게 한다(설계 §8 "가득 찼다 — 합친 뒤 다시 시도").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum AppendOutcome {
    Appended,
    Full { path: String },
}

/// 줄바꿈을 공백으로 접고 앞뒤 공백을 정리한다. 결과가 비면 `None` — 빈 문장은 남기지 않는다.
fn sanitize_line(text: &str) -> Option<String> {
    let collapsed = text
        .replace(['\n', '\r'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed)
    }
}

/// 대상의 파일 경로 + 상한. `Repo`는 **알려진 작업 저장소만** 받는다 — 임의 경로를 써 주는
/// 통로가 되지 않게 `db::known_repos`로 검증한다(설계 §8, 커맨드가 경로 인자를 받지 않는다).
async fn resolve_target(
    pool: &SqlitePool,
    root: &Path,
    target: &MemoryTarget,
) -> anyhow::Result<(PathBuf, Caps)> {
    match target {
        MemoryTarget::User => Ok((user_path(root), user_caps(pool).await)),
        MemoryTarget::Repo { repo } => {
            let repo = repo.trim();
            if repo.is_empty() {
                anyhow::bail!("저장소 경로가 비어 있습니다");
            }
            let known = crate::db::known_repos(pool).await?;
            if !known.iter().any(|r| r == repo) {
                anyhow::bail!("알 수 없는 저장소입니다: {repo}");
            }
            Ok((repo_path(root, &repo_key(repo)), caps(pool).await))
        }
    }
}

/// `MEMORY.md`/`USER.md`에 한 줄(`- {text}`) 추가. 상한(줄 수·바이트)을 넘기면 **쓰지 않고**
/// `Full`을 돌려준다 — 상한을 넘겨 쓰면 `read_capped`가 잘라내 조용히 사라지기 때문이다.
///
/// 에이전트가 같은 세션에서 이 파일을 고치는 중일 수 있으므로 읽고-고쳐-쓰기 대신 append
/// 모드로만 연다. 파일이 없거나 부모 폴더가 없으면 만든다(`project`가 창고를 새로 여는 것과
/// 같은 취급).
pub async fn append(
    pool: &SqlitePool,
    data_dir: &Path,
    target: &MemoryTarget,
    text: &str,
) -> anyhow::Result<AppendOutcome> {
    let line = sanitize_line(text).ok_or_else(|| anyhow::anyhow!("빈 문장은 남길 수 없습니다"))?;
    let root = memory_root(pool, data_dir).await;
    let (path, caps) = resolve_target(pool, &root, target).await?;

    let existing = std::fs::read(&path).unwrap_or_default();
    let existing_lines = if existing.is_empty() {
        0
    } else {
        existing.iter().filter(|&&b| b == b'\n').count() as u32
            + u32::from(!existing.ends_with(b"\n"))
    };
    let needs_leading_newline = !existing.is_empty() && !existing.ends_with(b"\n");
    let addition = format!(
        "{}- {line}\n",
        if needs_leading_newline { "\n" } else { "" }
    );
    let new_lines = existing_lines + 1;
    let new_bytes = existing.len() as u64 + addition.len() as u64;
    if new_lines > caps.lines || new_bytes > caps.bytes {
        return Ok(AppendOutcome::Full {
            path: path.to_string_lossy().into_owned(),
        });
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(addition.as_bytes())?;
    Ok(AppendOutcome::Appended)
}

/// 컨텍스트 파일에 들어갈 managed block. 설계 §3의 모양 그대로다.
///
/// 파일이 하나도 없어도 블록을 싣는다 — **규칙이 곧 "여기에 써라"**이기 때문이다(R2).
pub fn render_block(
    root: &Path,
    repo_key: &str,
    user: Option<&Capped>,
    repo: Option<&Capped>,
    caps: &Caps,
    user_caps: &Caps,
) -> String {
    let root_display = root.to_string_lossy();
    let repo_file = format!("{root_display}/{repo_key}/{REPO_FILE}");
    let user_file = format!("{root_display}/{USER_FILE}");
    let mut body = String::new();
    body.push_str(&format!("# Dojang Memory (파일 정본: {repo_file})\n\n"));
    body.push_str("## 규칙\n");
    body.push_str(
        "- 이 블록은 위 파일의 사본이다. 배운 것을 남기려면 **그 파일을 직접 고쳐라** (add / replace / remove).\n",
    );
    body.push_str("- 남길 것: 사용자 선호·정정, 코드에서 유도할 수 없는 결정·관례, 외부 위치.\n");
    body.push_str(
        "- 넣지 말 것: 코드에서 다시 얻을 수 있는 것(구조·경로·해법), 로그·표·코드 덤프, 임시 경로, 이미 AGENTS.md·문서에 있는 것.\n",
    );
    body.push_str(&format!(
        "- 상한 {}줄 / {} (USER.md는 {}줄 / {}). 80%를 넘으면 합치고 지워라. 넘긴 부분은 다음 세션에 실리지 않는다.\n",
        caps.lines,
        human_bytes(caps.bytes),
        user_caps.lines,
        human_bytes(user_caps.bytes),
    ));
    body.push_str(
        "- 작업을 마치기 전에 한 번 \"다음 세션이 알아야 할 것이 생겼나\"를 묻고 갱신하라.\n",
    );
    body.push_str("\n## USER.md\n");
    body.push_str(&section(user, &format!("(없음 — {user_file} 를 만들면 실린다)")));
    body.push_str("\n## MEMORY.md\n");
    body.push_str(&section(
        repo,
        "(없음 — 위 경로에 만들면 다음 작업부터 실린다)",
    ));
    format!("{MARK_START}\n{}{MARK_END}\n", escape_managed_markers(&body))
}

/// 본문 섹션 하나 — 없으면 안내 한 줄, 잘렸으면 경고가 **첫 줄**이다.
fn section(capped: Option<&Capped>, missing: &str) -> String {
    let Some(capped) = capped.filter(|c| !c.body.trim().is_empty()) else {
        return format!("{missing}\n");
    };
    if capped.truncated_lines > 0 {
        format!(
            "⚠ {}줄 잘림 — 합쳐서 상한 안으로 줄여라\n{}",
            capped.truncated_lines, capped.body
        )
    } else {
        capped.body.clone()
    }
}

/// `Capped`가 실제로 보여줄 본문을 가졌는지 — 공백뿐이면 "없음"과 같다(`section`과 같은 기준).
fn has_content(capped: Option<&Capped>) -> bool {
    capped.is_some_and(|c| !c.body.trim().is_empty())
}

/// 작업 시작 투영(R1–R3). 파일을 읽어 블록을 쓰고 상태 행을 갱신한다.
///
/// 실패는 파일 쓰기 실패뿐이다 — 읽기 실패(깨진 인코딩 등)는 "없음"으로 접는다.
/// 메모리 파일 하나 때문에 작업이 시작되지 못하는 쪽이 더 나쁘다.
///
/// 반환값은 **규칙만 있는 블록과 실제 메모리가 실린 블록을 가른다** — `true`는 사용자·레포
/// `Capped` 중 하나라도 공백이 아닌 본문을 가졌다는 뜻이다. 블록 자체는 이 값과 무관하게
/// 항상 쓴다(규칙 섹션은 메모리가 비어도 유용하다). 호출자는 이 값으로 "메모리 안내 후
/// 재설명 없음" 인사이트의 마커(`followup_observation::insert_observation_start`)를 남긴다.
#[allow(clippy::too_many_arguments)]
pub async fn project(
    pool: &SqlitePool,
    data_dir: &Path,
    repo: &str,
    worktree: &Path,
    targets: &[&str],
    task_id: i64,
    now: i64,
) -> anyhow::Result<bool> {
    let root = memory_root(pool, data_dir).await;
    let caps = caps(pool).await;
    let user_caps = user_caps(pool).await;
    let key = repo_key(repo);
    let user_file = user_path(&root);
    let repo_file = repo_path(&root, &key);
    let user = read_tolerant(&user_file, &user_caps);
    let repo_capped = read_tolerant(&repo_file, &caps);
    let had_content = has_content(user.as_ref()) || has_content(repo_capped.as_ref());
    let block = render_block(
        &root,
        &key,
        user.as_ref(),
        repo_capped.as_ref(),
        &caps,
        &user_caps,
    );
    crate::projector::write_block(worktree, targets, MARK_START, MARK_END, &block)?;
    record_file(
        pool,
        &user_file,
        KIND_USER,
        None,
        None,
        Some(now),
        Some(task_id),
    )
    .await?;
    record_file(
        pool,
        &repo_file,
        KIND_REPO,
        Some(repo),
        Some(&key),
        Some(now),
        Some(task_id),
    )
    .await?;
    Ok(had_content)
}

/// 작업이 끝나면 worktree 컨텍스트 파일에서 메모리 블록만 걷어낸다.
///
/// 정본은 창고의 파일이고 블록은 세션용 사본이다 — 남겨 두면 승인 커밋에 섞여
/// 저장소의 `AGENTS.md`에 영구히 들어간다(옛 파이프라인이 회수로 막던 것과 같은 위험).
/// 마커 밖은 건드리지 않고, 블록이 내용의 전부였던 파일(=투영이 만든 파일)은 지운다.
pub fn retire(worktree: &Path) -> std::io::Result<()> {
    let targets = crate::projector::all_targets();
    let preimages = crate::projector::capture_targets(worktree, &targets)?;
    let updates = preimages
        .iter()
        .map(|preimage| {
            let content = preimage.content.as_deref()?;
            if !content.contains(MARK_START) {
                return Some(content.to_string());
            }
            let stripped = crate::projector::remove_managed_block(content, MARK_START, MARK_END);
            // 블록이 맨 앞에 얹혔으면 투영이 넣은 빈 줄까지 걷는다 — 승인 diff가
            // 내용 없는 줄바꿈 변경으로 더러워지지 않게.
            let stripped = if content.starts_with(MARK_START) {
                stripped.trim_start_matches('\n').to_string()
            } else {
                stripped
            };
            (!stripped.trim().is_empty()).then_some(stripped)
        })
        .collect::<Vec<_>>();
    crate::projector::apply_updates(worktree, &preimages, &updates)
}

/// 작업 ID로 worktree를 찾아 블록을 걷어낸다. 작업·경로가 이미 없으면 할 일이 없다.
pub async fn retire_task(pool: &SqlitePool, task_id: i64) -> anyhow::Result<()> {
    let Some(task) = crate::db::get_task(pool, task_id).await? else {
        return Ok(());
    };
    let worktree = Path::new(&task.worktree_path);
    if !worktree.is_dir() {
        return Ok(());
    }
    retire(worktree)?;
    Ok(())
}

fn read_tolerant(path: &Path, caps: &Caps) -> Option<Capped> {
    match read_capped(path, caps) {
        Ok(capped) => capped,
        Err(error) => {
            eprintln!("메모리 파일을 읽지 못했습니다({}): {error}", path.display());
            None
        }
    }
}

/// 세션 종료(R8) — 파일의 크기·수정 시각만 기록한다. 본문은 DB에 넣지 않는다.
pub async fn record_exit(
    pool: &SqlitePool,
    data_dir: &Path,
    repo: &str,
    now: i64,
) -> anyhow::Result<()> {
    let _ = now;
    let root = memory_root(pool, data_dir).await;
    let key = repo_key(repo);
    record_file(pool, &user_path(&root), KIND_USER, None, None, None, None).await?;
    record_file(
        pool,
        &repo_path(&root, &key),
        KIND_REPO,
        Some(repo),
        Some(&key),
        None,
        None,
    )
    .await?;
    Ok(())
}

/// 파일 실측 크기로 상태 행을 upsert. 투영 시각·작업 id는 **준 값이 있을 때만** 덮는다 —
/// 종료 훅이 `None`으로 지나가면서 마지막 투영 기록을 지우면 화면에서 근거가 사라진다.
async fn record_file(
    pool: &SqlitePool,
    path: &Path,
    kind: &str,
    repo: Option<&str>,
    repo_key: Option<&str>,
    projected_at: Option<i64>,
    task_id: Option<i64>,
) -> anyhow::Result<()> {
    let measured = measure(path);
    sqlx::query(
        "INSERT INTO memory_files \
           (path, kind, repo, repo_key, lines, bytes, modified_at, last_projected_at, last_task_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(path) DO UPDATE SET \
           kind = excluded.kind, \
           repo = COALESCE(excluded.repo, memory_files.repo), \
           repo_key = COALESCE(excluded.repo_key, memory_files.repo_key), \
           lines = excluded.lines, \
           bytes = excluded.bytes, \
           modified_at = excluded.modified_at, \
           last_projected_at = COALESCE(excluded.last_projected_at, memory_files.last_projected_at), \
           last_task_id = COALESCE(excluded.last_task_id, memory_files.last_task_id)",
    )
    .bind(path.to_string_lossy().into_owned())
    .bind(kind)
    .bind(repo)
    .bind(repo_key)
    .bind(measured.0 as i64)
    .bind(measured.1 as i64)
    .bind(measured.2)
    .bind(projected_at)
    .bind(task_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// (lines, bytes, modified_at) — 파일이 없으면 (0, 0, None).
fn measure(path: &Path) -> (u32, u64, Option<i64>) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return (0, 0, None);
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64);
    let lines = std::fs::read_to_string(path)
        .map(|raw| raw.lines().count() as u32)
        .unwrap_or(0);
    (lines, metadata.len(), modified)
}

/// Wiki 화면용 목록 — USER.md가 언제나 첫 행이고(없어도 자리를 지킨다), 저장소 파일은
/// 디스크 스캔과 상태 테이블의 **합집합**이다. 손으로 만든 파일도, 창고를 옮겨 사라진
/// 파일도 둘 다 보여야 사람이 무슨 일이 있었는지 안다.
pub async fn list(pool: &SqlitePool, data_dir: &Path) -> anyhow::Result<Vec<MemoryFileInfo>> {
    let root = memory_root(pool, data_dir).await;
    let caps = caps(pool).await;
    let user_caps = user_caps(pool).await;
    let rows = status_rows(pool).await?;

    let user_file = user_path(&root);
    let mut out = vec![info_for(
        &user_file,
        KIND_USER,
        None,
        None,
        &user_caps,
        rows.iter().find(|row| row.path == user_file.to_string_lossy()),
    )];

    let mut repo_paths: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let candidate = entry.path().join(REPO_FILE);
            if candidate.is_file() {
                repo_paths.push(candidate);
            }
        }
    }
    for row in rows.iter().filter(|row| row.kind == KIND_REPO) {
        let path = PathBuf::from(&row.path);
        if !repo_paths.contains(&path) {
            repo_paths.push(path);
        }
    }

    let mut repos: Vec<MemoryFileInfo> = repo_paths
        .into_iter()
        .map(|path| {
            let row = rows.iter().find(|row| row.path == path.to_string_lossy());
            let key = path
                .parent()
                .and_then(|parent| parent.file_name())
                .map(|name| name.to_string_lossy().into_owned());
            let repo = row
                .and_then(|row| row.repo.clone())
                .or_else(|| frontmatter_repo(&path));
            info_for(&path, KIND_REPO, repo, key, &caps, row)
        })
        .collect();
    repos.sort_by(|a, b| {
        b.modified_at
            .cmp(&a.modified_at)
            .then_with(|| a.path.cmp(&b.path))
    });
    out.extend(repos);
    Ok(out)
}

fn info_for(
    path: &Path,
    kind: &str,
    repo: Option<String>,
    repo_key: Option<String>,
    caps: &Caps,
    row: Option<&StatusRow>,
) -> MemoryFileInfo {
    let (lines, bytes, modified_at) = measure(path);
    let exists = modified_at.is_some();
    MemoryFileInfo {
        path: path.to_string_lossy().into_owned(),
        kind: kind.to_string(),
        repo: repo.or_else(|| row.and_then(|row| row.repo.clone())),
        repo_key: repo_key.or_else(|| row.and_then(|row| row.repo_key.clone())),
        exists,
        lines: if exists {
            lines
        } else {
            row.map(|row| row.lines).unwrap_or(0)
        },
        bytes: if exists {
            bytes
        } else {
            row.map(|row| row.bytes).unwrap_or(0)
        },
        cap_lines: caps.lines,
        cap_bytes: caps.bytes,
        modified_at: modified_at.or_else(|| row.and_then(|row| row.modified_at)),
        last_projected_at: row.and_then(|row| row.last_projected_at),
        last_task_id: row.and_then(|row| row.last_task_id),
    }
}

/// frontmatter `repo:` — 사람이 폴더 키만 보고 어느 저장소인지 되찾는 길.
fn frontmatter_repo(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let mut lines = raw.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let line = line.trim();
        if line == "---" {
            break;
        }
        if let Some(value) = line.strip_prefix("repo:") {
            let value = value.trim().trim_matches('"').trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

struct StatusRow {
    path: String,
    kind: String,
    repo: Option<String>,
    repo_key: Option<String>,
    lines: u32,
    bytes: u64,
    modified_at: Option<i64>,
    last_projected_at: Option<i64>,
    last_task_id: Option<i64>,
}

async fn status_rows(pool: &SqlitePool) -> anyhow::Result<Vec<StatusRow>> {
    let rows = sqlx::query(
        "SELECT path, kind, repo, repo_key, lines, bytes, modified_at, last_projected_at, \
                last_task_id FROM memory_files",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| StatusRow {
            path: row.get::<String, _>("path"),
            kind: row.get::<String, _>("kind"),
            repo: row.get::<Option<String>, _>("repo"),
            repo_key: row.get::<Option<String>, _>("repo_key"),
            lines: row.get::<i64, _>("lines").max(0) as u32,
            bytes: row.get::<i64, _>("bytes").max(0) as u64,
            modified_at: row.get::<Option<i64>, _>("modified_at"),
            last_projected_at: row.get::<Option<i64>, _>("last_projected_at"),
            last_task_id: row.get::<Option<i64>, _>("last_task_id"),
        })
        .collect())
}

/// 옛 파이프라인이 쌓아 둔 행을 **한 번만** 지운다(설계 §7 O2, 사용자 결정).
///
/// 플래그가 이미 있으면 아무것도 하지 않는다 — 사용자가 다시 만든 데이터를 다음 부팅이
/// 지우면 삭제가 아니라 파괴다(첫 부팅 1회성 시드에 플래그를 남기는 판단과 같다).
pub async fn purge_legacy_once(pool: &SqlitePool) -> anyhow::Result<bool> {
    const FLAG: &str = "legacy_memory_purged";
    if crate::db::get_setting(pool, FLAG).await?.is_some() {
        return Ok(false);
    }
    // 지우기 전에 불변·삭제금지 트리거를 뗀다. 감사 원장으로서의 보호였고, 원장 자체를
    // 폐기하는 지금은 그 보호가 폐기를 막는 유일한 장애물이다. 아래에서 다시 세운다.
    for trigger in [
        "memory_citations_no_delete",
        "memory_projection_no_delete",
        "memory_injections_no_delete",
        "memory_approval_receipts_no_delete",
        "memory_evidence_checks_no_delete",
        "memory_evidence_no_delete",
        "memory_events_no_delete",
        "memory_versions_no_delete",
        // 외부콘텐츠 FTS 동기화 트리거 — 대량 삭제는 아래 rebuild가 더 싸고 안전하다.
        "memories_ad",
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger}"))
            .execute(pool)
            .await?;
    }
    // 자식 → 부모 순. FK 선언은 없지만 참조 방향대로 지워야 중간에 실패해도 고아가 남지 않는다.
    for table in [
        "memory_citations",
        "memory_usages",
        "memory_injections",
        "memory_projection_journal",
        "memory_approval_receipts",
        "memory_evidence_checks",
        "memory_evidence",
        "memory_events",
        "memory_versions",
        "si_proposals",
        "memories",
    ] {
        if !table_exists(pool, table).await? {
            continue;
        }
        sqlx::query(&format!("DELETE FROM {table}"))
            .execute(pool)
            .await?;
    }
    if table_exists(pool, "memories_fts").await? {
        sqlx::query("INSERT INTO memories_fts(memories_fts) VALUES('rebuild')")
            .execute(pool)
            .await?;
    }
    // 지금의 `migrate`는 위에서 뗀 옛 트리거를 다시 만들지 않는다(P2 제거, 설계 2026-09-13) —
    // 표는 빈 채로 남고, 그 뒤로는 아무도 쓰지 않으니 보호가 없어도 안전하다. 여기 남은
    // 호출은 파일형 메모리의 상태 색인(`memory_files`)만 만들고, 그 DDL도 `IF NOT EXISTS`라
    // 이 시점에도 안전히 no-op으로 실행되기 때문이다.
    super::migrate(pool).await?;
    crate::db::set_setting(pool, FLAG, "true").await?;
    Ok(true)
}

async fn table_exists(pool: &SqlitePool, name: &str) -> anyhow::Result<bool> {
    let found: Option<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE name = ? AND type IN ('table')")
            .bind(name)
            .fetch_optional(pool)
            .await?;
    Ok(found.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(label: &str) -> PathBuf {
        let dir = crate::testtmp::dir().join(format!(
            "praxis-memory-file-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn repo_key_is_stable_and_shared_by_worktrees() {
        let repo = "/Users/me/work/praxis";
        let key = repo_key(repo);
        assert!(key.starts_with("praxis-"), "{key}");
        assert_eq!(key.len(), "praxis-".len() + 8);
        // 끝 슬래시는 같은 저장소다 — 다른 키가 나오면 폴더가 둘로 갈라진다.
        assert_eq!(key, repo_key("/Users/me/work/praxis/"));
        assert_eq!(key, repo_key("  /Users/me/work/praxis  "));
        // 작업은 worktree가 아니라 메인 루트를 repo로 들고 다니므로 같은 키가 나온다.
        assert_eq!(key, repo_key("/Users/me/work/praxis"));
        assert_ne!(key, repo_key("/Users/me/other/praxis"));
    }

    #[test]
    fn read_capped_cuts_on_the_first_cap_that_bites() {
        let dir = tmp("read-capped");
        let path = dir.join("MEMORY.md");
        let body: String = (1..=10).map(|n| format!("line {n}\n")).collect();
        std::fs::write(&path, &body).unwrap();

        let by_lines = read_capped(
            &path,
            &Caps {
                lines: 4,
                bytes: 8192,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(by_lines.body.lines().count(), 4);
        assert_eq!(by_lines.total_lines, 10);
        assert_eq!(by_lines.truncated_lines, 6);
        assert_eq!(by_lines.total_bytes, body.len() as u64);

        let by_bytes = read_capped(&path, &Caps { lines: 100, bytes: 21 })
            .unwrap()
            .unwrap();
        assert_eq!(by_bytes.body.lines().count(), 3, "{:?}", by_bytes.body);
        assert_eq!(by_bytes.truncated_lines, 7);

        assert!(read_capped(&dir.join("absent.md"), &Caps::default())
            .unwrap()
            .is_none());
    }

    #[test]
    fn missing_files_still_render_the_rules() {
        let root = PathBuf::from("/vault/memory");
        let block = render_block(
            &root,
            "praxis-3f9a1c2e",
            None,
            None,
            &Caps::default(),
            &Caps::user_default(),
        );
        assert!(block.starts_with(MARK_START));
        assert!(block.trim_end().ends_with(MARK_END));
        assert!(
            block.contains("# Dojang Memory (파일 정본: /vault/memory/praxis-3f9a1c2e/MEMORY.md)")
        );
        assert!(block.contains("## 규칙"));
        assert!(block.contains("상한 100줄 / 8KB (USER.md는 40줄 / 3KB)"));
        assert!(block.contains("(없음 — /vault/memory/USER.md 를 만들면 실린다)"));
        assert!(block.contains("(없음 — 위 경로에 만들면 다음 작업부터 실린다)"));
        // 인용 관측은 폐기됐다 — M-id 지시문이 남아 있으면 에이전트가 없는 규약을 따른다.
        assert!(!block.contains("ID를 표기"));
        assert!(!block.contains("M-"));
    }

    #[test]
    fn over_cap_body_carries_exactly_the_cap_and_a_warning() {
        let dir = tmp("over-cap");
        let path = dir.join("MEMORY.md");
        let body: String = (1..=120).map(|n| format!("- item {n}\n")).collect();
        std::fs::write(&path, body).unwrap();
        let caps = Caps {
            lines: 100,
            bytes: 8192,
        };
        let capped = read_capped(&path, &caps).unwrap().unwrap();
        let block = render_block(
            Path::new("/vault/memory"),
            "praxis-3f9a1c2e",
            None,
            Some(&capped),
            &caps,
            &Caps::user_default(),
        );
        let section = block
            .split("## MEMORY.md\n")
            .nth(1)
            .unwrap()
            .replace(MARK_END, "");
        let lines: Vec<&str> = section.trim_end().lines().collect();
        assert_eq!(lines[0], "⚠ 20줄 잘림 — 합쳐서 상한 안으로 줄여라");
        assert_eq!(lines.len(), 1 + caps.lines as usize);
        assert_eq!(lines[1], "- item 1");
        assert_eq!(lines[100], "- item 100");
    }

    #[test]
    fn markers_inside_the_body_are_escaped() {
        let capped = Capped {
            body: format!("- {MARK_START} 와 {MARK_END} 를 본문에 적었다\n"),
            total_lines: 1,
            total_bytes: 0,
            truncated_lines: 0,
        };
        let block = render_block(
            Path::new("/vault/memory"),
            "praxis-3f9a1c2e",
            None,
            Some(&capped),
            &Caps::default(),
            &Caps::user_default(),
        );
        assert_eq!(block.matches(MARK_START).count(), 1, "{block}");
        assert_eq!(block.matches(MARK_END).count(), 1, "{block}");
        assert!(block.contains("&lt;!-- PRAXIS MEMORY START -->"));
        assert!(block.contains("&lt;!-- PRAXIS MEMORY END -->"));
    }

    #[test]
    fn block_round_trips_through_the_projector() {
        let block = render_block(
            Path::new("/vault/memory"),
            "praxis-3f9a1c2e",
            None,
            None,
            &Caps::default(),
            &Caps::user_default(),
        );
        let existing = "# 사용자 문서\n\n사용자 소유 문단.\n";
        let first = crate::projector::upsert_managed_block(existing, MARK_START, MARK_END, &block);
        assert!(first.contains("사용자 소유 문단."));
        let second = crate::projector::upsert_managed_block(&first, MARK_START, MARK_END, &block);
        assert_eq!(first, second, "같은 블록을 두 번 써도 같은 파일이어야 한다");
        assert_eq!(second.matches(MARK_START).count(), 1);
        let removed = crate::projector::remove_managed_block(&second, MARK_START, MARK_END);
        assert!(!removed.contains("Dojang Memory"));
        assert!(removed.contains("사용자 소유 문단."));
    }

    #[test]
    fn caps_fall_back_when_the_setting_is_garbage() {
        assert_eq!(parse_cap_lines(None, 100), 100);
        assert_eq!(parse_cap_lines(Some("abc".into()), 100), 100);
        assert_eq!(parse_cap_lines(Some("0".into()), 100), 100);
        assert_eq!(parse_cap_lines(Some("999999".into()), 100), 100);
        assert_eq!(parse_cap_lines(Some(" 250 ".into()), 100), 250);
        assert_eq!(parse_cap_bytes(Some("4096".into()), 8192), 4096);
        assert_eq!(parse_cap_bytes(Some("-1".into()), 8192), 8192);
    }

    #[test]
    fn frontmatter_repo_is_read_only_from_the_leading_block() {
        let dir = tmp("frontmatter");
        let with = dir.join("with.md");
        std::fs::write(&with, "---\nrepo: /Users/me/work/praxis\n---\n- 항목\n").unwrap();
        assert_eq!(
            frontmatter_repo(&with).as_deref(),
            Some("/Users/me/work/praxis")
        );
        let without = dir.join("without.md");
        std::fs::write(&without, "- repo: /not/frontmatter\n").unwrap();
        assert_eq!(frontmatter_repo(&without), None);
    }

    static DB_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    async fn pool() -> SqlitePool {
        // 초 단위 시각으로 이름을 지으면 같은 초에 뜬 두 테스트가 같은 파일을 물어 잠긴다.
        let n = DB_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = crate::testtmp::dir().join(format!(
            "praxis-memory-file-{}-{n}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let pool = crate::db::init_pool(path.to_string_lossy().as_ref())
            .await
            .unwrap();
        super::super::migrate(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn project_writes_the_block_and_records_status() {
        let dir = tmp("project");
        let root = dir.join("memory");
        let worktree = dir.join("wt");
        std::fs::create_dir_all(&worktree).unwrap();
        let pool = pool().await;
        crate::db::set_setting(&pool, SETTING_ROOT, root.to_string_lossy().as_ref())
            .await
            .unwrap();
        let repo = "/Users/me/work/praxis";
        let key = repo_key(repo);
        std::fs::create_dir_all(root.join(&key)).unwrap();
        std::fs::write(root.join(&key).join(REPO_FILE), "- 관례 한 줄\n").unwrap();
        std::fs::write(root.join(USER_FILE), "- 한국어로 답한다\n").unwrap();

        let had_content = project(&pool, &dir, repo, &worktree, &["CLAUDE.md"], 7, 1_700_000_000)
            .await
            .unwrap();
        assert!(had_content, "USER.md·MEMORY.md 둘 다 본문이 있으면 true");

        let written = std::fs::read_to_string(worktree.join("CLAUDE.md")).unwrap();
        assert!(written.contains("- 관례 한 줄"));
        assert!(written.contains("- 한국어로 답한다"));

        let rows = list(&pool, &dir).await.unwrap();
        assert_eq!(rows[0].kind, KIND_USER);
        assert!(rows[0].exists);
        let repo_row = rows.iter().find(|row| row.kind == KIND_REPO).unwrap();
        assert_eq!(repo_row.repo.as_deref(), Some(repo));
        assert_eq!(repo_row.repo_key.as_deref(), Some(key.as_str()));
        assert_eq!(repo_row.lines, 1);
        assert_eq!(repo_row.last_task_id, Some(7));
        assert_eq!(repo_row.last_projected_at, Some(1_700_000_000));

        // 종료 훅은 크기만 갱신하고 투영 기록을 지우지 않는다.
        std::fs::write(
            root.join(&key).join(REPO_FILE),
            "- 관례 한 줄\n- 두 번째 줄\n",
        )
        .unwrap();
        record_exit(&pool, &dir, repo, 1_700_000_100).await.unwrap();
        let rows = list(&pool, &dir).await.unwrap();
        let repo_row = rows.iter().find(|row| row.kind == KIND_REPO).unwrap();
        assert_eq!(repo_row.lines, 2);
        assert_eq!(repo_row.last_projected_at, Some(1_700_000_000));
        pool.close().await;
    }

    /// "메모리 안내 후 재설명 없음" 인사이트의 마커(`followup_observation::insert_observation_start`)는
    /// 이 반환값을 보고서만 남긴다 — 파일이 없거나 공백뿐이면 안내한 것이 없으니 false여야 한다.
    #[tokio::test]
    async fn project_reports_whether_any_file_had_content() {
        let dir = tmp("project-empty");
        let root = dir.join("memory");
        let worktree = dir.join("wt");
        std::fs::create_dir_all(&worktree).unwrap();
        let pool = pool().await;
        crate::db::set_setting(&pool, SETTING_ROOT, root.to_string_lossy().as_ref())
            .await
            .unwrap();
        let repo = "/Users/me/work/praxis";

        // 창고 자체가 없다 — USER.md·MEMORY.md 둘 다 없음.
        let had_content = project(&pool, &dir, repo, &worktree, &["CLAUDE.md"], 1, 1_700_000_000)
            .await
            .unwrap();
        assert!(!had_content, "파일이 없으면 false");

        // 공백뿐인 파일도 "없음"과 같다(`section`의 기준과 동일).
        let key = repo_key(repo);
        std::fs::create_dir_all(root.join(&key)).unwrap();
        std::fs::write(root.join(&key).join(REPO_FILE), "   \n").unwrap();
        std::fs::write(root.join(USER_FILE), "\n").unwrap();
        let had_content = project(&pool, &dir, repo, &worktree, &["CLAUDE.md"], 2, 1_700_000_001)
            .await
            .unwrap();
        assert!(!had_content, "공백뿐이면 false");

        // 둘 중 하나(MEMORY.md)에만 본문이 생기면 true.
        std::fs::write(root.join(&key).join(REPO_FILE), "- 관례 한 줄\n").unwrap();
        let had_content = project(&pool, &dir, repo, &worktree, &["CLAUDE.md"], 3, 1_700_000_002)
            .await
            .unwrap();
        assert!(had_content, "MEMORY.md에 본문이 있으면 true");
        pool.close().await;
    }

    #[test]
    fn retirement_takes_the_block_back_out_of_the_context_files() {
        let dir = tmp("retire");
        let worktree = dir.join("wt");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(worktree.join("CLAUDE.md"), "# 사용자 소유\n").unwrap();
        let block = render_block(
            Path::new("/vault/memory"),
            "praxis-3f9a1c2e",
            None,
            None,
            &Caps::default(),
            &Caps::user_default(),
        );
        crate::projector::write_block(
            &worktree,
            &["CLAUDE.md", "AGENTS.md"],
            MARK_START,
            MARK_END,
            &block,
        )
        .unwrap();
        assert!(worktree.join("AGENTS.md").exists());

        retire(&worktree).unwrap();

        // 사용자 내용은 그대로 남고 블록만 사라진다.
        let claude = std::fs::read_to_string(worktree.join("CLAUDE.md")).unwrap();
        assert!(claude.contains("# 사용자 소유"), "{claude}");
        assert!(!claude.contains(MARK_START), "{claude}");
        // 투영이 만든 파일은 빈 껍데기로 남기지 않는다 — 승인 커밋에 섞인다.
        assert!(!worktree.join("AGENTS.md").exists());
    }

    /// 지금의 `memory::migrate`는 이 표들을 더는 만들지 않는다(P2 제거, 설계 2026-09-13) —
    /// 여기서 만드는 DDL은 옛 앱 버전이 실제로 남겼을 법한 사용자 DB를 흉내낸 것으로,
    /// `purge_legacy_once`가 지금도 그런 잔존 DB를 다뤄야 하기 때문에 원시 SQL로 재현한다.
    async fn seed_legacy_schema(pool: &SqlitePool) {
        sqlx::raw_sql(
            "CREATE TABLE memories ( \
               id INTEGER PRIMARY KEY AUTOINCREMENT, tier TEXT NOT NULL, scope_key TEXT, \
               kind TEXT NOT NULL, content TEXT NOT NULL, confidence REAL NOT NULL DEFAULT 0.5, \
               created_at INTEGER NOT NULL); \
             CREATE VIRTUAL TABLE memories_fts USING fts5(content, content='memories', content_rowid='id'); \
             CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN \
               INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content); END; \
             CREATE TABLE memory_events ( \
               id INTEGER PRIMARY KEY AUTOINCREMENT, memory_id INTEGER NOT NULL, action TEXT NOT NULL, \
               actor_kind TEXT NOT NULL, created_at INTEGER NOT NULL); \
             CREATE TRIGGER memory_events_no_delete BEFORE DELETE ON memory_events BEGIN \
               SELECT RAISE(ABORT, 'memory event cannot be deleted'); END;",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn purge_runs_once_and_empties_the_legacy_tables() {
        let pool = pool().await;
        // si_proposals는 옛 자기개선 제안 표다(selfimprove 모듈은 1.0에서 제거됐다) — 이
        // 테스트는 그 표가 남은 옛 사용자 DB를 흉내내야 하므로, 지운 모듈 대신 같은 모양의
        // DDL을 직접 심는다.
        sqlx::query(
            "CREATE TABLE si_proposals ( \
               id INTEGER PRIMARY KEY AUTOINCREMENT, repo TEXT NOT NULL, kind TEXT NOT NULL, \
               content TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'proposed', \
               created_at INTEGER NOT NULL);",
        )
        .execute(&pool)
        .await
        .unwrap();
        seed_legacy_schema(&pool).await;
        sqlx::query(
            "INSERT INTO memories (tier, scope_key, kind, content, confidence, created_at) \
             VALUES ('project', '/repo', 'fact', '옛 메모리', 0.5, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO memory_events (memory_id, action, actor_kind, created_at) \
             VALUES (1, 'created', 'agent', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO si_proposals (repo, kind, content, created_at) \
             VALUES ('/repo', 'skill', '제안', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();

        assert!(purge_legacy_once(&pool).await.unwrap());
        for table in ["memories", "memory_events", "si_proposals"] {
            let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(count, 0, "{table}");
        }
        // FTS 외부콘텐츠 인덱스도 비었다 — 남으면 검색이 유령 행을 돌려준다.
        let ghosts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memories_fts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(ghosts, 0);
        // 삭제금지 트리거는 되살아나지 않는다 — 지금의 `migrate`는 이 트리거를 더는 만들지
        // 않는다(P2 제거). 아무도 이 표에 다시 쓰지 않으므로 보호가 없어도 안전하다.
        let restored: Option<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type='trigger' AND name='memory_events_no_delete'",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(restored.is_none());
        assert!(!purge_legacy_once(&pool).await.unwrap(), "두 번은 돌지 않는다");
        pool.close().await;
    }

    // ── append (T4 "메모리에 남기기") ────────────────────────────────

    #[tokio::test]
    async fn append_writes_one_line_to_user_md() {
        let dir = tmp("append-user");
        let root = dir.join("memory");
        let pool = pool().await;
        crate::db::set_setting(&pool, SETTING_ROOT, root.to_string_lossy().as_ref())
            .await
            .unwrap();

        let outcome = append(&pool, &dir, &MemoryTarget::User, "회고에서 배운 것")
            .await
            .unwrap();
        assert_eq!(outcome, AppendOutcome::Appended);
        let body = std::fs::read_to_string(root.join(USER_FILE)).unwrap();
        assert_eq!(body, "- 회고에서 배운 것\n");

        // 두 번째 줄은 append 모드로만 붙는다 — 첫 줄을 다시 읽어 고치지 않는다.
        let outcome = append(&pool, &dir, &MemoryTarget::User, "두 번째 문장")
            .await
            .unwrap();
        assert_eq!(outcome, AppendOutcome::Appended);
        let body = std::fs::read_to_string(root.join(USER_FILE)).unwrap();
        assert_eq!(body, "- 회고에서 배운 것\n- 두 번째 문장\n");
        pool.close().await;
    }

    #[tokio::test]
    async fn append_writes_one_line_to_repo_memory_md() {
        let dir = tmp("append-repo");
        let root = dir.join("memory");
        let pool = pool().await;
        crate::db::set_setting(&pool, SETTING_ROOT, root.to_string_lossy().as_ref())
            .await
            .unwrap();
        let repo = "/Users/me/work/praxis";
        sqlx::query(
            "INSERT INTO tasks (repo, branch, base, worktree_path, instruction, state, \
               created_at, updated_at) VALUES (?, 'main', 'base', ?, 'instruction', 'done', 1, 1)",
        )
        .bind(repo)
        .bind(repo)
        .execute(&pool)
        .await
        .unwrap();

        let target = MemoryTarget::Repo {
            repo: repo.to_string(),
        };
        let outcome = append(&pool, &dir, &target, "저장소 전용 교훈")
            .await
            .unwrap();
        assert_eq!(outcome, AppendOutcome::Appended);
        let body = std::fs::read_to_string(root.join(repo_key(repo)).join(REPO_FILE)).unwrap();
        assert_eq!(body, "- 저장소 전용 교훈\n");
        pool.close().await;
    }

    #[tokio::test]
    async fn append_rejects_unknown_repo() {
        let dir = tmp("append-unknown-repo");
        let root = dir.join("memory");
        let pool = pool().await;
        crate::db::set_setting(&pool, SETTING_ROOT, root.to_string_lossy().as_ref())
            .await
            .unwrap();

        let target = MemoryTarget::Repo {
            repo: "/no/such/repo".to_string(),
        };
        let error = append(&pool, &dir, &target, "문장").await.unwrap_err();
        assert!(error.to_string().contains("알 수 없는"), "{error}");
        assert!(
            !root.join(repo_key("/no/such/repo")).exists(),
            "검증에 실패하면 폴더조차 만들지 않는다"
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn append_strips_newlines_and_collapses_whitespace() {
        let dir = tmp("append-newlines");
        let root = dir.join("memory");
        let pool = pool().await;
        crate::db::set_setting(&pool, SETTING_ROOT, root.to_string_lossy().as_ref())
            .await
            .unwrap();

        append(
            &pool,
            &dir,
            &MemoryTarget::User,
            "첫 줄\n두 번째 줄\r\n  세 번째  ",
        )
        .await
        .unwrap();
        let body = std::fs::read_to_string(root.join(USER_FILE)).unwrap();
        assert_eq!(body, "- 첫 줄 두 번째 줄 세 번째\n");
        pool.close().await;
    }

    #[tokio::test]
    async fn append_rejects_empty_text() {
        let dir = tmp("append-empty");
        let root = dir.join("memory");
        let pool = pool().await;
        crate::db::set_setting(&pool, SETTING_ROOT, root.to_string_lossy().as_ref())
            .await
            .unwrap();

        let error = append(&pool, &dir, &MemoryTarget::User, "   \n \r\n ")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("빈 문장"), "{error}");
        assert!(!root.join(USER_FILE).exists(), "쓰지 않는다");
        pool.close().await;
    }

    #[tokio::test]
    async fn append_returns_full_and_does_not_write_when_cap_is_exceeded() {
        let dir = tmp("append-full");
        let root = dir.join("memory");
        std::fs::create_dir_all(&root).unwrap();
        let pool = pool().await;
        crate::db::set_setting(&pool, SETTING_ROOT, root.to_string_lossy().as_ref())
            .await
            .unwrap();
        // 사용자 상한은 줄 40 · 바이트 3072(DEFAULT_USER_CAP_*). 줄 상한을 1로 낮춰
        // 한 줄만 있어도 가득 차게 만든다.
        crate::db::set_setting(&pool, SETTING_USER_CAP_LINES, "1")
            .await
            .unwrap();
        let existing = "- 이미 가득 찬 파일\n";
        std::fs::write(root.join(USER_FILE), existing).unwrap();

        let outcome = append(&pool, &dir, &MemoryTarget::User, "새 문장")
            .await
            .unwrap();
        match outcome {
            AppendOutcome::Full { path } => {
                assert_eq!(path, root.join(USER_FILE).to_string_lossy());
            }
            other => panic!("Full을 기대했다: {other:?}"),
        }
        // 파일은 바이트 하나도 바뀌지 않는다 — 쓰기 전에 상한을 먼저 본다.
        let after = std::fs::read(root.join(USER_FILE)).unwrap();
        assert_eq!(after, existing.as_bytes());
        pool.close().await;
    }
}
