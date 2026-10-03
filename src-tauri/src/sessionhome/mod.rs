//! 세션홈(`~/.claude/projects`) 경로·해석·인덱싱의 단일 소유자.
//!
//! [설계](../../../docs/designs/2026-09-17-session-home-resume-design.md) 결정 1: 경로 산출·cwd
//! 인코딩·세션 id 해석이 한때 `transcript/mod.rs`와 `insights/mod.rs` 둘로 갈려 있어 심볼릭·정규화
//! 차이가 나면 "저장소 경로 접두 필터"와 워크트리 매칭이 서로 다른 답을 낼 수 있었다. 1.0에서
//! 트랜스크립트 파서를 통째로 없애면서 이 모듈이 경로 산출·cwd 인코딩·세션 id 해석의 유일한
//! 소유자가 됐다 — **단, 집계·파싱 로직(트랜스크립트 요약·사용량 통계)은 여기 두지 않는다.** 같은
//! 형식을 두 파서가 읽으면 반드시 어긋난다는 것은 원장(#236 로테이션 사고)에서 이미 치른 값이다.
//!
//! ## 정규화 계약
//! - [`resolve`]는 **라이브 스캔**이다 — 캐시를 쓰지 않는다. 실측상 벤더(claude)가 세션 id를
//!   세션홈 전역에서 해석하므로, 우리도 전역에서 찾아 **정확히 하나**일 때만 성립시킨다. mtime
//!   캐시를 두더라도 [`scan`]의 목록 표시 메타에만 쓰고 해석에는 쓰지 않는다 — 캐시로 유일성을
//!   판정하면 스캔 이후 심어진 동일 id 파일을 못 본다.
//! - [`scan`]은 **전량 파싱 금지**다. 파일당 선두 일부 줄 + 말미 일부 바이트 + `mtime`/`size`만
//!   읽는다(실측 세션홈: 1,568개 파일·1.2GB·최대 62MB). 손상된 줄은 조용히 건너뛴다.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;

/// 메타(`cwd`·`gitBranch`·`version`·`sessionId`)는 벤더 포맷상 선두 10줄 안에 있다(설계 실측).
/// 제목 후보(`custom-title`·`summary`)는 세션 중간에도 나올 수 있어 약간의 여유를
/// 둔다. 여전히 파일 전체를 읽지 않는 상수 크기 창이다.
const HEAD_SAMPLE_LINES: usize = 20;
/// 한 개의 비정상적으로 큰 JSONL 행이 목록 스캔 예산을 우회하지 못하게 한다.
const HEAD_SAMPLE_BYTES: u64 = 32 * 1024;
/// 말미에서 다시 읽는 바이트 수 — 마지막 `cwd`(워크트리 밖 이동 감지)·최근 제목 갱신용.
const TAIL_SAMPLE_BYTES: u64 = 32 * 1024;
/// 상세 대화는 선택할 때만 읽으며, 최근 256KiB 안에서만 추출한다.
const PREVIEW_TAIL_BYTES: u64 = 256 * 1024;
const PREVIEW_MESSAGES: usize = 6;
const PREVIEW_MESSAGE_CHARS: usize = 2_000;
const PREVIEW_TITLE_CHARS: usize = 512;
const MAX_SESSION_ENTRIES: usize = 20_000;
const SCAN_CACHE_CAPACITY: usize = 4_096;
/// `title`·`first_message`는 신뢰 경계 밖(쓰기 가능한 세션홈)의 데이터라 상한을 둔다.
const SNIPPET_MAX_CHARS: usize = 120;

/// 세션홈 루트를 `HOME` 산출 대신 명시 지정한다. **프로세스 수명 동안 한 번만** 정해진다 —
/// 중간에 바뀌면 이미 승계한 세션을 [`resolve`]가 더 이상 못 찾는 상태가 되고, 그 시점을
/// 추적할 방법이 없다. 이미 정해져 있으면 기존 값을 `Err`로 돌려준다.
///
/// 두 쓰임이 있다.
/// - Runner를 systemd 유닛 등으로 띄워 프로세스의 `HOME`이 세션을 만든 로그인 사용자와 다를
///   때(설계 "미결"의 Runner `HOME` 항목) 부팅 시 실제 세션홈을 가리킨다. **지금 설정 파일에
///   이 값을 싣는 경로는 없다** — 필요해지면 `RunnerConfig`에 키를 더해 여기로 넘긴다.
/// - 통합 테스트가 실제 `$HOME`을 건드리지 않고 픽스처 세션홈을 가리킨다. `HOME` 환경변수를
///   바꾸는 방법과 달리 git 등 다른 소비자에 번지지 않는다.
pub fn set_projects_root(root: PathBuf) -> Result<(), PathBuf> {
    ROOT_OVERRIDE.set(root).map_err(|_| {
        ROOT_OVERRIDE
            .get()
            .cloned()
            .unwrap_or_else(|| PathBuf::from(""))
    })
}

static ROOT_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();
static CODEX_ROOT_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();
static SCAN_CACHE: OnceLock<Mutex<HashMap<ScanCacheKey, SessionMeta>>> = OnceLock::new();

#[derive(Hash, PartialEq, Eq)]
struct ScanCacheKey {
    vendor: &'static str,
    path: PathBuf,
    size: u64,
    modified_ns: u128,
}

/// Codex session root을 명시 지정한다. 통합 테스트와 Runner가 로그인 사용자와 다른 환경에서
/// Codex 세션을 읽을 때 사용한다. 프로세스 수명 동안 한 번만 설정할 수 있다.
pub fn set_codex_sessions_root(root: PathBuf) -> Result<(), PathBuf> {
    CODEX_ROOT_OVERRIDE.set(root).map_err(|_| {
        CODEX_ROOT_OVERRIDE
            .get()
            .cloned()
            .unwrap_or_else(|| PathBuf::from(""))
    })
}

/// `~/.claude/projects`. [`set_projects_root`]로 지정된 값이 있으면 그것이 이긴다.
/// 지정이 없고 `HOME`/`USERPROFILE`도 못 찾으면 `None`.
pub fn projects_root() -> Option<PathBuf> {
    if let Some(root) = ROOT_OVERRIDE.get() {
        return Some(root.clone());
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    Some(PathBuf::from(home).join(".claude").join("projects"))
}

/// `~/.codex/sessions`. CODEX_HOME가 있으면 Codex CLI와 같은 규칙을 따른다.
pub fn codex_sessions_root() -> Option<PathBuf> {
    if let Some(root) = CODEX_ROOT_OVERRIDE.get() {
        return Some(root.clone());
    }
    // Claude fixture root만 바꾼 통합 테스트가 사용자의 실제 Codex 세션을 목록에 섞지 않게 한다.
    // Codex fixture가 필요하면 위 setter로 명시한다.
    if ROOT_OVERRIDE.get().is_some() {
        return None;
    }
    std::env::var_os("CODEX_HOME")
        .map(|root| PathBuf::from(root).join("sessions"))
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|home| PathBuf::from(home).join(".codex").join("sessions"))
        })
}

/// 세션홈 인덱스 항목. `cwd`는 세션 선두 값, `last_cwd`는 말미 값 — 세션 중간에 cwd가 바뀔 수
/// 있어(메시지마다 있는 필드) 인가 판정은 둘 다 봐야 한다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionMeta {
    /// 원본 세션을 여는 CLI. 구형 Runner 응답은 serde 기본값으로 Claude가 된다.
    #[serde(default = "default_vendor")]
    pub vendor: String,
    pub session_id: String,
    pub cwd: Option<String>,
    pub last_cwd: Option<String>,
    pub git_branch: Option<String>,
    /// `custom-title` > `summary` 우선순위. 120자 상한 + 제어문자 제거.
    pub title: Option<String>,
    /// 첫 user 메시지 텍스트. 120자 상한 + 제어문자 제거.
    pub first_message: Option<String>,
    /// 표본 안에서 확인한 가장 최근 실제 사용자 발언. 없으면 전체 대화를 읽지 않는다.
    #[serde(default)]
    pub recent_user_message: Option<String>,
    /// 파일 mtime(초 단위 epoch) 기준.
    pub last_active: i64,
    /// 선두/말미 표본에서 센 user·assistant 메시지 수 — 색인용 근사치이고 정확한 총계가 아니다.
    pub messages: usize,
    pub vendor_version: Option<String>,
}

fn default_vendor() -> String {
    "claude".to_string()
}

/// 선택 시에만 반환하는 제한된 대화 상세. 원문 파일 경로는 절대 노출하지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionPreview {
    pub meta: SessionMeta,
    pub messages: Vec<PreviewMessage>,
    /// `true`이면 정해진 tail 예산 때문에 더 이전 대화가 생략됐다.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreviewMessage {
    pub role: String,
    pub text: String,
}

/// [`scan`] 필터. 비어 있으면 필터링하지 않는다.
#[derive(Debug, Default, Clone)]
pub struct ScanFilter {
    /// 이 중 하나를 접두로 갖는 cwd(선두 또는 말미)만 포함.
    pub cwd_prefixes: Vec<String>,
    /// 제목·첫 메시지·cwd에 대한 대소문자 무시 부분 문자열 검색.
    pub query: Option<String>,
}

/// [`resolve`] 실패 사유. "세션 없음"과 "인가되지 않음"을 같은 404로 뭉치는 것은 이 모듈을
/// 부르는 커맨드/HTTP 계층의 책임이다 — 여기서는 사유를 구분해 돌려준다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// session_id가 UUID 문법이 아니거나 경로 요소를 담고 있다.
    InvalidId,
    /// 세션홈 전역에서 일치하는 파일이 없다(세션홈 자체가 없는 경우 포함).
    NotFound,
    /// 같은 id의 세션 파일이 둘 이상이다 — 벤더가 여는 파일과 우리가 인가한 파일이 갈릴 수 있어
    /// 거절한다. 진단용으로 발견된 경로를 담는다.
    Ambiguous(Vec<PathBuf>),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::InvalidId => write!(f, "세션 id 형식이 올바르지 않습니다"),
            ResolveError::NotFound => write!(f, "세션을 찾을 수 없습니다"),
            ResolveError::Ambiguous(paths) => {
                write!(f, "같은 id의 세션 파일이 {}개 있어 거절합니다", paths.len())
            }
        }
    }
}

impl std::error::Error for ResolveError {}

/// session_id → 세션홈 전역에서 유일하게 일치하는 `.jsonl` 경로. 캐시를 쓰지 않는 라이브 스캔.
pub fn resolve(session_id: &str) -> Result<PathBuf, ResolveError> {
    let root = projects_root().ok_or(ResolveError::NotFound)?;
    resolve_in(&root, session_id)
}

/// [`resolve`]의 본체 — 루트를 **명시 인자로** 받는다. `HOME` 산출에 기대지 않는 유일한
/// 진입점이라, 세션홈을 흉내 내는 테스트와 `HOME`이 세션 소유자와 다른 배포가 모두 이 문을
/// 쓴다([`set_projects_root`] 참조).
pub fn resolve_in(root: &Path, session_id: &str) -> Result<PathBuf, ResolveError> {
    if !is_valid_session_id(session_id) {
        return Err(ResolveError::InvalidId);
    }
    let root_canon = root.canonicalize().map_err(|_| ResolveError::NotFound)?;

    let mut matches = Vec::new();
    if let Ok(projects) = std::fs::read_dir(&root_canon) {
        for proj_entry in projects.flatten() {
            // 심볼릭 링크 프로젝트 디렉터리는 제외 — `fsapi::safe_join`과 같은 규약.
            if proj_entry
                .file_type()
                .map(|t| t.is_symlink())
                .unwrap_or(true)
            {
                continue;
            }
            let proj_path = proj_entry.path();
            if !proj_path.is_dir() {
                continue;
            }
            let candidate = proj_path.join(format!("{session_id}.jsonl"));
            let Ok(meta) = std::fs::symlink_metadata(&candidate) else {
                continue;
            };
            // 최종 엔트리가 심볼릭이면 거부 — target 존재 여부 무관.
            if meta.file_type().is_symlink() || !meta.is_file() {
                continue;
            }
            matches.push(candidate);
        }
    }

    match matches.len() {
        0 => Err(ResolveError::NotFound),
        1 => {
            let only = matches.into_iter().next().expect("길이 1 확인됨");
            // 방어: 실경로가 root 하위인지 재확인(레이스·이상 경로 대비).
            match only.canonicalize() {
                Ok(real)
                    if real.starts_with(&root_canon) && claude_file_has_id(&only, session_id) =>
                {
                    Ok(only)
                }
                _ => Err(ResolveError::NotFound),
            }
        }
        _ => Err(ResolveError::Ambiguous(matches)),
    }
}

/// UUID 문법만 허용(경로 요소·`..` 자동 거부 — 하이픈·16진수 외 문자가 없다).
fn is_valid_session_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    bytes.iter().enumerate().all(|(i, b)| match i {
        8 | 13 | 18 | 23 => *b == b'-',
        _ => b.is_ascii_hexdigit(),
    })
}

/// 세션홈 인덱스. 최근순 정렬 + `limit` 상한. 전량 파싱하지 않는다(모듈 문서 참조).
pub fn scan(filter: &ScanFilter, limit: usize) -> Vec<SessionMeta> {
    let mut out = Vec::new();
    if let Some(root) = projects_root() {
        out.extend(scan_in(&root, filter, limit));
    }
    if let Some(root) = codex_sessions_root() {
        out.extend(scan_codex_in(&root, filter, limit));
    }
    apply_filter(&mut out, filter);
    out.sort_by(|a, b| b.last_active.cmp(&a.last_active));
    out.truncate(limit);
    out
}

/// 공급자 하나만 스캔한다. 허용하지 않는 vendor는 빈 목록으로 처리한다.
pub fn scan_vendor(vendor: &str, filter: &ScanFilter, limit: usize) -> Vec<SessionMeta> {
    match vendor {
        "claude" => projects_root()
            .map(|root| scan_in(&root, filter, limit))
            .unwrap_or_default(),
        "codex" => codex_sessions_root()
            .map(|root| scan_codex_in(&root, filter, limit))
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// 테스트·Runner 어댑터가 명시 루트로 공급자 하나를 스캔할 때 쓴다.
pub fn scan_vendor_in(
    vendor: &str,
    root: &Path,
    filter: &ScanFilter,
    limit: usize,
) -> Vec<SessionMeta> {
    match vendor {
        "claude" => scan_in(root, filter, limit),
        "codex" => scan_codex_in(root, filter, limit),
        _ => Vec::new(),
    }
}

/// [`scan`]의 본체 — 루트를 **명시 인자로** 받는다. 근거는 [`resolve_in`]과 같다.
pub fn scan_in(root: &Path, filter: &ScanFilter, limit: usize) -> Vec<SessionMeta> {
    let mut out = Vec::new();
    let Ok(projects) = std::fs::read_dir(root) else {
        return out;
    };
    for proj_entry in projects.flatten() {
        if proj_entry
            .file_type()
            .map(|t| t.is_symlink())
            .unwrap_or(true)
        {
            continue;
        }
        let proj_path = proj_entry.path();
        if !proj_path.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&proj_path) else {
            continue;
        };
        for file_entry in files.flatten() {
            let Ok(file_type) = file_entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() || !file_type.is_file() {
                continue;
            }
            let file_path = file_entry.path();
            if file_path.extension().map(|e| e != "jsonl").unwrap_or(true) {
                continue;
            }
            if let Some(meta) = index_scan_cached("claude", &file_path) {
                out.push(meta);
            }
        }
    }
    apply_filter(&mut out, filter);
    out.sort_by(|a, b| b.last_active.cmp(&a.last_active));
    out.truncate(limit);
    out
}

fn apply_filter(sessions: &mut Vec<SessionMeta>, filter: &ScanFilter) {
    if !filter.cwd_prefixes.is_empty() {
        sessions.retain(|s| {
            filter.cwd_prefixes.iter().any(|p| {
                s.cwd.as_deref().is_some_and(|c| cwd_has_prefix(c, p))
                    || s.last_cwd.as_deref().is_some_and(|c| cwd_has_prefix(c, p))
            })
        });
    }
    if let Some(q) = filter
        .query
        .as_deref()
        .map(str::to_lowercase)
        .filter(|q| !q.is_empty())
    {
        sessions.retain(|s| {
            [
                s.title.as_deref(),
                s.first_message.as_deref(),
                s.recent_user_message.as_deref(),
                s.git_branch.as_deref(),
                s.cwd.as_deref(),
                s.last_cwd.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|field| field.to_lowercase().contains(&q))
        });
    }
}

/// 목록의 project filter는 경로 구성요소 경계를 지킨다(`/repo`가 `/repo-other`를 잡지 않는다).
/// macOS의 `/var` → `/private/var`처럼 같은 기존 경로의 별칭도 canonical 경로로 한 번 더
/// 비교한다. 둘 중 하나가 없어지면 원문 `Path::starts_with`만 사용한다.
fn cwd_has_prefix(cwd: &str, prefix: &str) -> bool {
    let cwd = Path::new(cwd);
    let prefix = Path::new(prefix);
    if cwd.starts_with(prefix) {
        return true;
    }
    match (cwd.canonicalize(), prefix.canonicalize()) {
        (Ok(cwd), Ok(prefix)) => cwd.starts_with(prefix),
        _ => false,
    }
}

/// session_id → 메타 하나. [`resolve`]로 유일한 파일을 확정한 뒤 **그 파일만** 인덱싱한다.
///
/// [`scan`]으로 같은 일을 하려면 세션홈 전체를 훑어야 한다(실측 1,568파일·1.2GB) — 목록이
/// 아니라 한 건이 필요한 호출부(승계)는 이쪽을 쓴다.
pub fn describe(session_id: &str) -> Result<SessionMeta, ResolveError> {
    let root = projects_root().ok_or(ResolveError::NotFound)?;
    describe_in(&root, session_id)
}

/// [`describe`]의 본체 — 루트를 명시 인자로 받는다. 근거는 [`resolve_in`]과 같다.
pub fn describe_in(root: &Path, session_id: &str) -> Result<SessionMeta, ResolveError> {
    let path = resolve_in(root, session_id)?;
    // 해석은 됐는데 인덱싱이 실패하는 경우(권한·경합으로 사라짐)는 "없음"과 같게 다룬다.
    index_claude_session_file(&path).ok_or(ResolveError::NotFound)
}

/// vendor와 id를 함께 써서 원본 세션을 확정한다. 기존 [`describe`]는 Claude 호환 경로다.
pub fn describe_vendor(vendor: &str, session_id: &str) -> Result<SessionMeta, ResolveError> {
    match vendor {
        "claude" => describe(session_id),
        "codex" => codex_sessions_root()
            .ok_or(ResolveError::NotFound)
            .and_then(|root| describe_codex_in(&root, session_id)),
        _ => Err(ResolveError::InvalidId),
    }
}

pub fn describe_vendor_in(
    vendor: &str,
    root: &Path,
    session_id: &str,
) -> Result<SessionMeta, ResolveError> {
    match vendor {
        "claude" => describe_in(root, session_id),
        "codex" => describe_codex_in(root, session_id),
        _ => Err(ResolveError::InvalidId),
    }
}

/// 선택된 세션의 최근 대화만 반환한다. 목록 스캔은 이 함수를 호출하지 않는다.
pub fn preview_vendor(vendor: &str, session_id: &str) -> Result<SessionPreview, ResolveError> {
    match vendor {
        "claude" => projects_root()
            .ok_or(ResolveError::NotFound)
            .and_then(|root| preview_vendor_in(vendor, &root, session_id)),
        "codex" => codex_sessions_root()
            .ok_or(ResolveError::NotFound)
            .and_then(|root| preview_vendor_in(vendor, &root, session_id)),
        _ => Err(ResolveError::InvalidId),
    }
}

pub fn preview_vendor_in(
    vendor: &str,
    root: &Path,
    session_id: &str,
) -> Result<SessionPreview, ResolveError> {
    let mut meta = describe_vendor_in(vendor, root, session_id)?;
    let path = match vendor {
        "claude" => resolve_in(root, session_id)?,
        "codex" => resolve_codex_in(root, session_id)?,
        _ => return Err(ResolveError::InvalidId),
    };
    let (lines, truncated) = read_tail_lines_with_truncation(&path, PREVIEW_TAIL_BYTES);
    let mut messages = Vec::new();
    let mut truncation = truncated;
    let mut prior: Option<(String, String, String)> = None;
    for line in lines {
        if let Some(message) = preview_message(vendor, &line) {
            let kind = serde_json::from_str::<serde_json::Value>(&line)
                .ok()
                .and_then(|value| {
                    value
                        .get("type")
                        .and_then(|kind| kind.as_str())
                        .map(str::to_owned)
                })
                .unwrap_or_default();
            let duplicate_pair = vendor == "codex"
                && prior.as_ref().is_some_and(|(role, text, prior_kind)| {
                    role == &message.role
                        && text == &message.text
                        && prior_kind != &kind
                        && matches!(kind.as_str(), "response_item" | "event_msg")
                        && matches!(prior_kind.as_str(), "response_item" | "event_msg")
                });
            if duplicate_pair {
                prior = None;
                continue;
            }
            prior = Some((message.role.clone(), message.text.clone(), kind));
            truncation |= message.text.chars().count() >= PREVIEW_MESSAGE_CHARS;
            messages.push(message);
        }
    }
    if messages.len() > PREVIEW_MESSAGES {
        messages.drain(..messages.len() - PREVIEW_MESSAGES);
        truncation = true;
    }
    truncation |= enrich_preview_meta(vendor, &path, &mut meta);
    Ok(SessionPreview {
        meta,
        messages,
        truncated: truncation,
    })
}

/// Codex는 날짜 하위 디렉터리에 rollout 파일을 둔다. 파일명만 믿지 않고 내부 session_meta의
/// id도 대조하며, 심볼릭 링크는 탐색·해석 모두에서 제외한다.
fn resolve_codex_in(root: &Path, session_id: &str) -> Result<PathBuf, ResolveError> {
    if !is_valid_session_id(session_id) {
        return Err(ResolveError::InvalidId);
    }
    let root_canon = root.canonicalize().map_err(|_| ResolveError::NotFound)?;
    let suffix = format!("{session_id}.jsonl");
    let mut pending = vec![root_canon.clone()];
    let mut matches = Vec::new();
    let mut visited = 0usize;
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            visited = visited.saturating_add(1);
            if visited > MAX_SESSION_ENTRIES {
                return Err(ResolveError::NotFound);
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file()
                && entry.file_name().to_string_lossy().ends_with(&suffix)
                && codex_file_has_id(&entry.path(), session_id)
            {
                matches.push(entry.path());
            }
        }
    }
    match matches.len() {
        0 => Err(ResolveError::NotFound),
        1 => {
            let only = matches.pop().expect("길이 1 확인됨");
            match only.canonicalize() {
                Ok(real) if real.starts_with(&root_canon) => Ok(only),
                _ => Err(ResolveError::NotFound),
            }
        }
        _ => Err(ResolveError::Ambiguous(matches)),
    }
}

fn codex_file_has_id(path: &Path, session_id: &str) -> bool {
    read_head_lines(path, HEAD_SAMPLE_LINES)
        .into_iter()
        .any(|line| {
            serde_json::from_str::<serde_json::Value>(&line)
                .ok()
                .is_some_and(|value| {
                    value.get("type").and_then(|v| v.as_str()) == Some("session_meta")
                        && value.pointer("/payload/id").and_then(|v| v.as_str()) == Some(session_id)
                })
        })
}

/// Claude 구형 행에는 sessionId가 없을 수 있어 그 경우는 파일명의 기존 계약을 유지한다.
/// 있지만 다르면 파일명 스푸핑이므로 목록·상세·재개 모두에서 제외한다.
fn claude_file_has_id(path: &Path, session_id: &str) -> bool {
    for line in read_head_lines(path, HEAD_SAMPLE_LINES) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let Some(id) = value.get("sessionId").and_then(|value| value.as_str()) else {
            continue;
        };
        if id != session_id {
            return false;
        }
    }
    true
}

fn describe_codex_in(root: &Path, session_id: &str) -> Result<SessionMeta, ResolveError> {
    let path = resolve_codex_in(root, session_id)?;
    index_codex_session_file(&path).ok_or(ResolveError::NotFound)
}

fn scan_codex_in(root: &Path, filter: &ScanFilter, limit: usize) -> Vec<SessionMeta> {
    let mut out = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    let mut visited = 0usize;
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            visited = visited.saturating_add(1);
            if visited > MAX_SESSION_ENTRIES {
                break;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                pending.push(entry.path());
                continue;
            }
            let path = entry.path();
            if kind.is_file() && path.extension().is_some_and(|ext| ext == "jsonl") {
                if let Some(meta) = index_scan_cached("codex", &path) {
                    out.push(meta)
                }
            }
        }
    }
    apply_filter(&mut out, filter);
    out.sort_by(|a, b| b.last_active.cmp(&a.last_active));
    out.truncate(limit);
    out
}

/// Claude 파일 하나를 선두/말미 표본만으로 인덱싱. 손상된 줄은 건너뛴다.
fn index_claude_session_file(path: &Path) -> Option<SessionMeta> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() {
        return None;
    }
    let session_id = path.file_stem()?.to_str()?.to_string();
    if !claude_file_has_id(path, &session_id) {
        return None;
    }

    let head_lines = read_head_lines(path, HEAD_SAMPLE_LINES);
    // 파일이 선두 표본 안에 다 들어오면 말미를 다시 읽지 않는다(중복 집계 방지 + 불필요한 IO 생략).
    let tail_lines = read_tail_lines(path, TAIL_SAMPLE_BYTES);

    let mut acc = MetaAcc::default();
    let mut seen = HashSet::new();
    for line in head_lines.iter().chain(tail_lines.iter()) {
        apply_claude_line(&mut acc, line, &mut seen);
    }

    // agent-name은 실행 워커 라벨일 뿐 사용자가 붙인 대화 제목이 아니다.
    let title = [acc.custom_title, acc.summary]
        .into_iter()
        .flatten()
        .find_map(|s| sanitize_snippet(&s));
    let first_message = acc.first_message.as_deref().and_then(sanitize_snippet);

    Some(SessionMeta {
        vendor: default_vendor(),
        session_id,
        cwd: acc.cwd_first,
        last_cwd: acc.cwd_last,
        git_branch: acc.git_branch,
        title,
        first_message,
        recent_user_message: recent_user_from_tail("claude", &tail_lines),
        last_active: mtime_secs(&metadata),
        messages: acc.message_count,
        vendor_version: acc.version,
    })
}

/// 목록 스캔 전용 캐시. 해석·상세는 항상 원본을 다시 읽어 승계 인가에 캐시가 끼지 않는다.
fn index_scan_cached(vendor: &'static str, path: &Path) -> Option<SessionMeta> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return None;
    }
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|time| time.as_nanos())
        .unwrap_or(0);
    let key = ScanCacheKey {
        vendor,
        path: path.to_path_buf(),
        size: metadata.len(),
        modified_ns,
    };
    let cache = SCAN_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(meta) = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&key)
        .cloned()
    {
        return Some(meta);
    }
    let meta = match vendor {
        "claude" => index_claude_session_file(path),
        "codex" => index_codex_session_file(path),
        _ => None,
    }?;
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.len() >= SCAN_CACHE_CAPACITY {
        cache.clear();
    }
    cache.insert(key, meta.clone());
    Some(meta)
}

fn index_codex_session_file(path: &Path) -> Option<SessionMeta> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() {
        return None;
    }
    let head_lines = read_head_lines(path, HEAD_SAMPLE_LINES);
    let tail_lines = read_tail_lines(path, TAIL_SAMPLE_BYTES);
    let mut acc = MetaAcc::default();
    let mut session_id = None;
    let mut seen = HashSet::new();
    for line in head_lines.iter().chain(tail_lines.iter()) {
        apply_codex_line(&mut acc, &mut session_id, line, &mut seen);
    }
    let session_id = session_id?;
    Some(SessionMeta {
        vendor: "codex".to_string(),
        session_id,
        cwd: acc.cwd_first,
        last_cwd: acc.cwd_last,
        git_branch: acc.git_branch,
        title: acc.custom_title.as_deref().and_then(sanitize_snippet),
        first_message: acc.first_message.as_deref().and_then(sanitize_snippet),
        recent_user_message: recent_user_from_tail("codex", &tail_lines),
        last_active: mtime_secs(&metadata),
        messages: acc.message_count,
        vendor_version: acc.version,
    })
}

fn mtime_secs(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn read_head_lines(path: &Path, max_lines: usize) -> Vec<String> {
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    BufReader::new(file.take(HEAD_SAMPLE_BYTES))
        .lines()
        .take(max_lines)
        .filter_map(Result::ok)
        .collect()
}

/// 파일 말미 `tail_bytes`를 읽어 완결된 줄만 반환한다(선두 절단분은 버린다).
fn read_tail_lines(path: &Path, tail_bytes: u64) -> Vec<String> {
    read_tail_lines_with_truncation(path, tail_bytes).0
}

fn read_tail_lines_with_truncation(path: &Path, tail_bytes: u64) -> (Vec<String>, bool) {
    let Ok(mut file) = std::fs::File::open(path) else {
        return (Vec::new(), false);
    };
    let Ok(len) = file.metadata().map(|m| m.len()) else {
        return (Vec::new(), false);
    };
    let start = len.saturating_sub(tail_bytes);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return (Vec::new(), false);
    }
    let mut bytes = Vec::new();
    if file.take(tail_bytes).read_to_end(&mut bytes).is_err() {
        return (Vec::new(), false);
    }
    // 잘린 지점이 UTF-8 문자 경계가 아닐 수 있으니 손실 허용 변환.
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    // start > 0이면 첫 줄은 파일 중간에서 잘렸을 수 있으니 버린다(깨진 JSON으로 읽혀도
    // 어차피 파서가 무시하지만, 잘못된 줄 절반을 온전한 것으로 오인하지 않기 위함).
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    (lines, start > 0)
}

/// 파싱 도중 누적하는 세션 메타. 필드별로 "첫 값"·"마지막 값" 의미가 다르다.
#[derive(Default)]
struct MetaAcc {
    cwd_first: Option<String>,
    cwd_last: Option<String>,
    git_branch: Option<String>,
    version: Option<String>,
    custom_title: Option<String>,
    summary: Option<String>,
    first_message: Option<String>,
    recent_user_message: Option<String>,
    message_count: usize,
}

/// JSONL 한 줄을 누적기에 반영. 손상된 줄(JSON 파싱 실패)은 조용히 무시.
/// `seen`은 선두/말미 표본이 겹칠 때(작은 파일) 같은 줄을 두 번 세지 않기 위한 방어.
fn apply_claude_line(acc: &mut MetaAcc, raw: &str, seen: &mut HashSet<String>) {
    let raw = raw.trim();
    if raw.is_empty() || !seen.insert(raw.to_string()) {
        return;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return;
    };

    if let Some(c) = v.get("cwd").and_then(|x| x.as_str()) {
        if acc.cwd_first.is_none() {
            acc.cwd_first = Some(c.to_string());
        }
        acc.cwd_last = Some(c.to_string());
    }
    if let Some(b) = v.get("gitBranch").and_then(|x| x.as_str()) {
        acc.git_branch = Some(b.to_string());
    }
    if acc.version.is_none() {
        if let Some(ver) = v.get("version").and_then(|x| x.as_str()) {
            acc.version = Some(ver.to_string());
        }
    }

    match v.get("type").and_then(|x| x.as_str()).unwrap_or("") {
        "custom-title" => {
            if let Some(t) = v.get("customTitle").and_then(|x| x.as_str()) {
                acc.custom_title = Some(t.to_string());
            }
        }
        "summary" => {
            if let Some(s) = v.get("summary").and_then(|x| x.as_str()) {
                acc.summary = Some(s.to_string());
            }
        }
        ty @ ("user" | "assistant") => {
            acc.message_count += 1;
            if ty == "user" && acc.first_message.is_none() {
                if let Some(text) = user_message_text(&v).and_then(|text| display_user_text(&text))
                {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        acc.first_message = Some(trimmed.to_string());
                    }
                }
            }
            if ty == "user" {
                if let Some(text) = user_message_text(&v).and_then(|text| display_user_text(&text))
                {
                    acc.recent_user_message = Some(text);
                }
            }
        }
        _ => {}
    }
}

fn apply_codex_line(
    acc: &mut MetaAcc,
    session_id: &mut Option<String>,
    raw: &str,
    seen: &mut HashSet<String>,
) {
    let raw = raw.trim();
    if raw.is_empty() || !seen.insert(raw.to_string()) {
        return;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return;
    };
    let kind = value.get("type").and_then(|v| v.as_str());
    let payload = value.get("payload").unwrap_or(&serde_json::Value::Null);
    if kind == Some("session_meta") {
        if let Some(id) = payload
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|id| is_valid_session_id(id))
        {
            *session_id = Some(id.to_string());
        }
        if acc.version.is_none() {
            acc.version = payload
                .get("cli_version")
                .and_then(|v| v.as_str())
                .map(str::to_string);
        }
    }
    if kind == Some("turn_context") || kind == Some("session_meta") {
        if let Some(cwd) = payload.get("cwd").and_then(|v| v.as_str()) {
            if acc.cwd_first.is_none() {
                acc.cwd_first = Some(cwd.to_string())
            }
            acc.cwd_last = Some(cwd.to_string());
        }
        if let Some(branch) = payload
            .get("git_branch")
            .or_else(|| payload.get("gitBranch"))
            .or_else(|| payload.pointer("/git/branch"))
            .and_then(|v| v.as_str())
        {
            acc.git_branch = Some(branch.to_string());
        }
    }
    if kind != Some("response_item")
        || payload.get("type").and_then(|v| v.as_str()) != Some("message")
    {
        return;
    }
    let role = payload.get("role").and_then(|v| v.as_str());
    if !matches!(role, Some("user") | Some("assistant")) {
        return;
    }
    acc.message_count += 1;
    let Some(text) = codex_message_text(payload).and_then(|text| {
        if role == Some("user") {
            display_user_text(&text)
        } else {
            (!text.trim().is_empty()).then_some(text.trim().to_string())
        }
    }) else {
        return;
    };
    let text = text.trim().to_string();
    if role == Some("user") {
        if acc.first_message.is_none() {
            acc.first_message = Some(text.clone())
        }
        acc.recent_user_message = Some(text);
    }
}

fn codex_message_text(payload: &serde_json::Value) -> Option<String> {
    let content = payload.get("content")?.as_array()?;
    let mut out = String::new();
    for block in content {
        if matches!(
            block.get("type").and_then(|v| v.as_str()),
            Some("input_text") | Some("output_text") | Some("text")
        ) {
            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                out.push_str(text);
                out.push(' ')
            }
        }
    }
    (!out.trim().is_empty()).then_some(out)
}

/// 끝 표본에 사용자가 없으면 선두의 오래된 발언을 "최근"이라고 추정하지 않는다.
fn recent_user_from_tail(vendor: &str, lines: &[String]) -> Option<String> {
    lines.iter().rev().find_map(|line| {
        preview_message(vendor, line)
            .filter(|message| message.role == "user")
            .and_then(|message| sanitize_snippet(&message.text))
    })
}

fn preview_message(vendor: &str, raw: &str) -> Option<PreviewMessage> {
    let value = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    let (role, text) = match vendor {
        "claude" => {
            let role = value.get("type")?.as_str()?;
            if !matches!(role, "user" | "assistant") {
                return None;
            }
            let text = user_message_text(&value)?;
            (
                role,
                if role == "user" {
                    display_user_text(&text)?
                } else {
                    text
                },
            )
        }
        "codex" => {
            let payload = value.get("payload")?;
            let event_type = value.get("type")?.as_str()?;
            let (role, text) = if event_type == "response_item"
                && payload.get("type")?.as_str()? == "message"
            {
                (payload.get("role")?.as_str()?, codex_message_text(payload)?)
            } else if event_type == "event_msg" {
                match payload.get("type")?.as_str()? {
                    "user_message" => ("user", payload.get("message")?.as_str()?.to_string()),
                    "agent_message" => ("assistant", payload.get("message")?.as_str()?.to_string()),
                    _ => return None,
                }
            } else {
                return None;
            };
            if !matches!(role, "user" | "assistant") {
                return None;
            }
            (
                role,
                if role == "user" {
                    display_user_text(&text)?
                } else {
                    text
                },
            )
        }
        _ => return None,
    };
    sanitize_preview_text(&text).map(|text| PreviewMessage {
        role: role.to_string(),
        text,
    })
}

/// Praxis가 사용자 메시지 앞에 붙이는 작업 지시를 대화 제목/목록의 "사용자 발언"으로
/// 오인하지 않는다. 원문을 요약·재작성하지 않고, 명시적인 경계 뒤 문자열만 사용한다.
fn display_user_text(raw: &str) -> Option<String> {
    let text = raw.trim();
    if text.starts_with("<system-reminder>") || text.starts_with("<local-command") {
        return None;
    }
    let text = text
        .split_once("# User request")
        .map(|(_, request)| request)
        .unwrap_or(text)
        .trim();
    if text.starts_with("# AGENTS.md instructions") || text.starts_with("<environment_context>") {
        return None;
    }
    // 알려진 호출 래퍼의 명시적인 끝 문장만 제거한다. 임의 본문의 마지막 문단을
    // 사용자 요청이라고 추측하지 않는다. 래퍼 형식이 다르면 원문 발췌를 유지한다.
    let text = if text.starts_with("# Workflow Harness") {
        text.split_once("This is conditional routing, not another mandatory profile transition or permission gate.")
            .map(|(_, request)| request.trim()).unwrap_or(text)
    } else {
        text
    };
    (!text.is_empty()).then_some(text.to_string())
}

fn sanitize_preview_text(raw: &str) -> Option<String> {
    let mut out = String::new();
    for c in raw.chars() {
        if out.chars().count() >= PREVIEW_MESSAGE_CHARS {
            break;
        }
        out.push(if c.is_control() && c != '\n' { ' ' } else { c });
    }
    (!out.trim().is_empty()).then_some(out.trim().to_string())
}

/// 상세는 목록보다 긴 제목을 보여도 되지만, 파일 전체를 읽어 제목을 "복원"하지는 않는다.
/// 선두·말미 표본에서 확인한 마지막 custom-title/summary만 512자로 확장한다.
fn enrich_preview_meta(vendor: &str, path: &Path, meta: &mut SessionMeta) -> bool {
    let mut custom_title = None;
    let mut summary = None;
    let mut first_request = None;
    let mut seen = HashSet::new();
    for line in read_head_lines(path, HEAD_SAMPLE_LINES)
        .into_iter()
        .chain(read_tail_lines(path, PREVIEW_TAIL_BYTES))
    {
        if !seen.insert(line.clone()) {
            continue;
        }
        if first_request.is_none() {
            first_request = preview_message(vendor, &line)
                .filter(|message| message.role == "user")
                .map(|message| message.text);
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if vendor == "claude" {
            match value.get("type").and_then(|v| v.as_str()) {
                Some("custom-title") => {
                    custom_title = value
                        .get("customTitle")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                }
                Some("summary") => {
                    summary = value
                        .get("summary")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                }
                _ => {}
            }
        }
    }
    let first_truncated = first_request
        .as_ref()
        .is_some_and(|text| text.chars().count() > PREVIEW_TITLE_CHARS);
    if let Some(text) = first_request {
        meta.first_message = sanitize_limited(&text, PREVIEW_TITLE_CHARS);
    }
    let Some(raw) = custom_title.or(summary) else {
        return first_truncated;
    };
    let was_truncated = first_truncated || raw.chars().count() > PREVIEW_TITLE_CHARS;
    meta.title = sanitize_limited(&raw, PREVIEW_TITLE_CHARS);
    was_truncated
}

/// `message.content` 추출 — 문자열 또는 `[{type:"text", text:"..."}]` 형태.
/// 제목 표시용으로 필요한 최소 추출이라 집계 로직과는 별도로 둔다.
fn user_message_text(v: &serde_json::Value) -> Option<String> {
    let content = v.get("message").and_then(|m| m.get("content"))?;
    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    let arr = content.as_array()?;
    let mut out = String::new();
    for block in arr {
        if block.get("type").and_then(|t| t.as_str()) == Some("text") {
            if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                out.push_str(t);
                out.push(' ');
            }
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// 제어문자를 공백으로 접고(연속 공백은 접힘) 120자로 자른다. 결과가 비면 `None`.
/// 세션홈은 신뢰 경계 밖의 쓰기 가능 디렉터리이므로 마크다운·제어문자를 그대로 돌려주지 않는다.
fn sanitize_snippet(raw: &str) -> Option<String> {
    sanitize_limited(raw, SNIPPET_MAX_CHARS)
}

fn sanitize_limited(raw: &str, max_chars: usize) -> Option<String> {
    let mut out = String::new();
    let mut count = 0usize;
    let mut last_was_space = true; // 선두 공백도 접히도록 초기값 true.
    for c in raw.chars() {
        if count >= max_chars {
            break;
        }
        let ch = if c.is_control() { ' ' } else { c };
        if ch == ' ' {
            if last_was_space {
                continue;
            }
            last_was_space = true;
        } else {
            last_was_space = false;
        }
        out.push(ch);
        count += 1;
    }
    let trimmed = out.trim_end();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 원격 승계 인가. `roots`는 이미 canonicalize된 실경로여야 한다(`runner/config.rs`의
/// `canonicalize_repository_roots`가 기동 시 보장).
///
/// `runner::auth::authorize_repository_path`를 재사용하지 **않는다** — 그 함수는
/// `canonicalize()` 실패를 거절로 다루는데, 끝난 Praxis 워크트리는 정리되어 cwd가 사라진다.
/// "끝난 세션을 다시 잇는다"가 이 기능의 주 사용이므로 그대로 쓰면 기능이 목적을 잃는다.
///
/// cwd가 실재하면 canonicalize 후 실경로 접두 비교. 없으면 존재하는 최상위 조상까지
/// canonicalize한 뒤 나머지를 어휘적으로(`.`·`..` 제거) 붙여 비교한다 — 존재하지 않는 경로는
/// 심볼릭 우회 대상이 아니므로 이 완화가 경계를 깎지 않는다. cwd가 `None`이면 거절.
pub fn authorize_cwd(roots: &[PathBuf], cwd: Option<&str>) -> bool {
    let Some(cwd) = cwd else {
        return false;
    };
    let path = Path::new(cwd);
    if !path.is_absolute() {
        return false;
    }
    let resolved = match path.canonicalize() {
        Ok(real) => real,
        Err(_) => match resolve_missing_path(path) {
            Some(p) => p,
            None => return false,
        },
    };
    roots.iter().any(|root| resolved.starts_with(root))
}

/// 존재하지 않는 절대 경로를 "존재하는 최상위 조상까지 canonicalize + 나머지 어휘적 정규화"로
/// 해석한다. 파일시스템에 없는 나머지 구간은 손대지 않으므로 심볼릭 우회 경계를 넓히지 않는다.
fn resolve_missing_path(path: &Path) -> Option<PathBuf> {
    let normalized = lexical_normalize(path);
    let ancestor = normalized.ancestors().find(|p| p.exists())?;
    let canonical_ancestor = ancestor.canonicalize().ok()?;
    let remaining = normalized.strip_prefix(ancestor).ok()?;
    Some(canonical_ancestor.join(remaining))
}

/// `.`은 지우고 `.. `은 직전 일반 요소를 지운다(루트 아래로는 내려가지 않는다). 파일시스템에
/// 접근하지 않는 순수 문자열 연산.
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut stack: Vec<Component> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(stack.last(), Some(Component::Normal(_))) {
                    stack.pop();
                }
            }
            other => stack.push(other),
        }
    }
    let mut out = PathBuf::new();
    for comp in stack {
        out.push(comp.as_os_str());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 각 테스트가 고유 임시 디렉터리를 갖도록 pid + 나노초로 유니크화한다(tempfile 미의존,
    /// `theme_store.rs` 테스트와 같은 관례).
    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before epoch")
            .as_nanos();
        let dir = crate::testtmp::dir().join(format!(
            "praxis-sessionhome-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("임시 루트 생성 실패");
        dir
    }

    fn write_jsonl(project_dir: &Path, session_id: &str, lines: &[String]) -> PathBuf {
        std::fs::create_dir_all(project_dir).unwrap();
        let path = project_dir.join(format!("{session_id}.jsonl"));
        std::fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    fn meta_line(session_id: &str, cwd: &str, git_branch: &str, version: &str) -> String {
        format!(
            r#"{{"type":"attachment","sessionId":"{session_id}","cwd":"{cwd}","gitBranch":"{git_branch}","version":"{version}"}}"#
        )
    }

    fn user_line(text: &str) -> String {
        format!(r#"{{"type":"user","message":{{"role":"user","content":"{text}"}}}}"#)
    }

    fn assistant_line() -> String {
        r#"{"type":"assistant","message":{"role":"assistant","content":"ok"}}"#.to_string()
    }

    fn write_codex_jsonl(root: &Path, session_id: &str, lines: &[String]) -> PathBuf {
        let dir = root.join("2026").join("09").join("23");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("rollout-2026-09-23T00-00-00-{session_id}.jsonl"));
        std::fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    fn codex_meta_line(session_id: &str, cwd: &str) -> String {
        serde_json::json!({"type":"session_meta","payload":{"id":session_id,"cwd":cwd,"cli_version":"0.154.0","git":{"branch":"codex-branch"}}}).to_string()
    }

    fn codex_message(role: &str, text: &str) -> String {
        serde_json::json!({"type":"response_item","payload":{"type":"message","role":role,"content":[{"type": if role == "user" { "input_text" } else { "output_text" },"text":text}]}}).to_string()
    }

    // ---- is_valid_session_id / resolve ----

    #[test]
    fn resolve_rejects_non_uuid_and_traversal_ids() {
        let root = temp_dir("resolve-invalid");
        assert_eq!(
            resolve_in(&root, "not-a-uuid"),
            Err(ResolveError::InvalidId)
        );
        assert_eq!(
            resolve_in(&root, "../../etc/passwd"),
            Err(ResolveError::InvalidId)
        );
        assert_eq!(
            resolve_in(&root, "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee/../x"),
            Err(ResolveError::InvalidId)
        );
    }

    #[test]
    fn resolve_finds_unique_session() {
        let root = temp_dir("resolve-unique");
        let id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let expected = write_jsonl(
            &root.join("-Users-x-repo"),
            id,
            &[meta_line(id, "/Users/x/repo", "main", "2.1.274")],
        );
        let got = resolve_in(&root, id).expect("유일 해석 성공해야 함");
        assert_eq!(
            got.canonicalize().unwrap(),
            expected.canonicalize().unwrap()
        );
    }

    #[test]
    fn resolve_reports_not_found_for_missing_session() {
        let root = temp_dir("resolve-missing");
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(
            resolve_in(&root, "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"),
            Err(ResolveError::NotFound)
        );
    }

    #[test]
    fn resolve_rejects_claude_file_with_mismatched_internal_id() {
        let root = temp_dir("resolve-id-mismatch");
        let id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        write_jsonl(
            &root.join("-Users-x-repo"),
            id,
            &[meta_line(
                "bbbbbbbb-cccc-dddd-eeee-ffffffffffff",
                "/Users/x/repo",
                "main",
                "2.1.274",
            )],
        );
        assert_eq!(resolve_in(&root, id), Err(ResolveError::NotFound));
    }

    #[test]
    fn resolve_rejects_duplicate_session_id_across_projects() {
        let root = temp_dir("resolve-ambiguous");
        let id = "bbbbbbbb-cccc-dddd-eeee-ffffffffffff";
        write_jsonl(
            &root.join("-Users-x-repo-a"),
            id,
            &[meta_line(id, "/Users/x/repo-a", "main", "2.1.274")],
        );
        write_jsonl(
            &root.join("-Users-x-repo-b"),
            id,
            &[meta_line(id, "/Users/x/repo-b", "main", "2.1.274")],
        );
        match resolve_in(&root, id) {
            Err(ResolveError::Ambiguous(paths)) => assert_eq!(paths.len(), 2),
            other => panic!("Ambiguous를 기대했지만 {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn resolve_excludes_symlinked_entry() {
        use std::os::unix::fs::symlink;

        let root = temp_dir("resolve-symlink");
        let id = "cccccccc-dddd-eeee-ffff-000000000000";
        let real_target = write_jsonl(
            &root.join("real-project"),
            id,
            &[meta_line(id, "/Users/x/repo", "main", "2.1.274")],
        );
        let link_dir = root.join("-Users-x-repo");
        symlink(real_target.parent().unwrap(), &link_dir).unwrap();

        // real-project 쪽만 유효한 매치이므로 유일 해석이 성립해야 한다(심볼릭 디렉터리는 스킵).
        let got = resolve_in(&root, id).expect("심볼릭이 아닌 실제 파일로 유일 해석되어야 함");
        assert_eq!(
            got.canonicalize().unwrap(),
            real_target.canonicalize().unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_excludes_symlinked_file_entry() {
        use std::os::unix::fs::symlink;

        let root = temp_dir("resolve-symlink-file");
        let id = "dddddddd-eeee-ffff-0000-111111111111";
        let real_target = write_jsonl(
            &root.join("real-project"),
            id,
            &[meta_line(id, "/Users/x/repo", "main", "2.1.274")],
        );
        let other_dir = root.join("other-project");
        std::fs::create_dir_all(&other_dir).unwrap();
        symlink(&real_target, other_dir.join(format!("{id}.jsonl"))).unwrap();

        // 심볼릭 파일 엔트리는 제외되므로 real-project 하나만 유일 매치.
        let got = resolve_in(&root, id).expect("유일 해석 성공해야 함");
        assert_eq!(
            got.canonicalize().unwrap(),
            real_target.canonicalize().unwrap()
        );
    }

    // ---- scan ----

    #[test]
    fn scan_extracts_meta_and_sorts_by_recency() {
        let root = temp_dir("scan-meta");
        let id_old = "11111111-1111-1111-1111-111111111111";
        let id_new = "22222222-2222-2222-2222-222222222222";
        let old_path = write_jsonl(
            &root.join("-Users-x-old"),
            id_old,
            &[
                meta_line(id_old, "/Users/x/old", "main", "2.1.200"),
                user_line("hello old session"),
                assistant_line(),
            ],
        );
        let new_path = write_jsonl(
            &root.join("-Users-x-new"),
            id_new,
            &[
                meta_line(id_new, "/Users/x/new", "feat/x", "2.1.274"),
                user_line("hello new session"),
                assistant_line(),
            ],
        );
        // mtime 순서를 명확히 하기 위해 old를 과거로 되돌린다.
        set_mtime_secs(&old_path, 1_000);
        set_mtime_secs(&new_path, 2_000);

        let out = scan_in(&root, &ScanFilter::default(), 10);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].session_id, id_new, "최근순 정렬이어야 함");
        assert_eq!(out[0].cwd.as_deref(), Some("/Users/x/new"));
        assert_eq!(out[0].git_branch.as_deref(), Some("feat/x"));
        assert_eq!(out[0].vendor_version.as_deref(), Some("2.1.274"));
        assert_eq!(out[0].first_message.as_deref(), Some("hello new session"));
        assert_eq!(out[0].messages, 2);
        assert_eq!(out[1].session_id, id_old);
    }

    #[test]
    fn scan_prefers_custom_title_then_summary() {
        let root = temp_dir("scan-title");
        let id = "33333333-3333-3333-3333-333333333333";
        write_jsonl(
            &root.join("-Users-x-repo"),
            id,
            &[
                meta_line(id, "/Users/x/repo", "main", "2.1.274"),
                r#"{"type":"agent-name","agentName":"task-1"}"#.to_string(),
                r#"{"type":"summary","summary":"세션 요약"}"#.to_string(),
                r#"{"type":"custom-title","customTitle":"내가 정한 제목"}"#.to_string(),
            ],
        );
        let out = scan_in(&root, &ScanFilter::default(), 10);
        assert_eq!(out[0].title.as_deref(), Some("내가 정한 제목"));
    }

    #[test]
    fn scan_never_uses_agent_name_as_a_title() {
        let root = temp_dir("scan-agent-name");
        let id = "33333333-3333-3333-3333-333333333334";
        write_jsonl(
            &root.join("-Users-x-repo"),
            id,
            &[
                meta_line(id, "/Users/x/repo", "main", "2.1.274"),
                r#"{"type":"agent-name","agentName":"task-1"}"#.to_string(),
            ],
        );
        assert!(scan_in(&root, &ScanFilter::default(), 10)[0]
            .title
            .is_none());
    }

    #[test]
    fn user_request_wrapper_keeps_only_the_actual_request() {
        let input = "# Workflow Harness\ninternal\n# User request\n세션 이어받기를 고친다";
        assert_eq!(
            display_user_text(input).as_deref(),
            Some("세션 이어받기를 고친다")
        );
        assert_eq!(display_user_text("<system-reminder>internal"), None);
        assert_eq!(
            display_user_text("# AGENTS.md instructions\ninternal only"),
            None
        );
    }

    #[test]
    fn scan_caps_title_and_strips_control_chars() {
        let root = temp_dir("scan-cap");
        let id = "44444444-4444-4444-4444-444444444444";
        let long_title = "가".repeat(200);
        let line = serde_json::json!({
            "type": "custom-title",
            "customTitle": format!("탭\t줄바꿈\n{long_title}")
        })
        .to_string();
        write_jsonl(
            &root.join("-Users-x-repo"),
            id,
            &[meta_line(id, "/Users/x/repo", "main", "2.1.274"), line],
        );
        let out = scan_in(&root, &ScanFilter::default(), 10);
        let title = out[0].title.as_ref().expect("제목이 있어야 함");
        assert!(title.chars().count() <= 120, "120자 상한을 지켜야 함");
        assert!(
            !title.contains('\t') && !title.contains('\n'),
            "제어문자가 남아있으면 안 됨: {title:?}"
        );
    }

    #[test]
    fn scan_ignores_corrupted_lines() {
        let root = temp_dir("scan-corrupt");
        let id = "55555555-5555-5555-5555-555555555555";
        write_jsonl(
            &root.join("-Users-x-repo"),
            id,
            &[
                "이건 JSON이 아니다".to_string(),
                meta_line(id, "/Users/x/repo", "main", "2.1.274"),
                "{ 잘린 json".to_string(),
                user_line("살아남은 메시지"),
            ],
        );
        let out = scan_in(&root, &ScanFilter::default(), 10);
        assert_eq!(
            out.len(),
            1,
            "손상된 줄이 있어도 세션 자체는 인덱싱되어야 함"
        );
        assert_eq!(out[0].cwd.as_deref(), Some("/Users/x/repo"));
        assert_eq!(out[0].first_message.as_deref(), Some("살아남은 메시지"));
    }

    #[test]
    fn scan_on_empty_home_returns_empty() {
        let root = temp_dir("scan-empty");
        std::fs::create_dir_all(&root).unwrap();
        let out = scan_in(&root, &ScanFilter::default(), 10);
        assert!(out.is_empty());
    }

    #[test]
    fn scan_on_missing_home_returns_empty() {
        let root = temp_dir("scan-missing").join("does-not-exist");
        let out = scan_in(&root, &ScanFilter::default(), 10);
        assert!(out.is_empty());
    }

    #[test]
    fn codex_scan_describe_and_preview_are_vendor_aware_and_bounded() {
        let root = temp_dir("codex-scan");
        let id = "019f205d-33b8-7111-93b8-63fe9eaea592";
        write_codex_jsonl(
            &root,
            id,
            &[
                codex_meta_line(id, "/Users/x/repo"),
                codex_message("user", "첫 요청"),
                codex_message("assistant", "첫 답변"),
                codex_message("user", "마지막 요청"),
                codex_message("assistant", "마지막 답변"),
            ],
        );
        let listed = scan_vendor_in("codex", &root, &ScanFilter::default(), 10);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].vendor, "codex");
        assert_eq!(listed[0].session_id, id);
        assert_eq!(listed[0].first_message.as_deref(), Some("첫 요청"));
        assert_eq!(
            listed[0].recent_user_message.as_deref(),
            Some("마지막 요청")
        );
        assert_eq!(listed[0].git_branch.as_deref(), Some("codex-branch"));
        assert_eq!(
            describe_vendor_in("codex", &root, id).unwrap().vendor,
            "codex"
        );

        let preview = preview_vendor_in("codex", &root, id).unwrap();
        assert!(!preview.truncated);
        assert_eq!(preview.messages.len(), 4);
        assert_eq!(preview.messages[2].text, "마지막 요청");
    }

    #[test]
    fn preview_returns_only_recent_messages_and_marks_tail_truncation() {
        let root = temp_dir("preview-tail");
        let id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let mut lines = vec![meta_line(id, "/Users/x/repo", "main", "2.1.274")];
        lines.extend((0..10).flat_map(|n| [user_line(&format!("u{n}")), assistant_line()]));
        lines.push("x".repeat(PREVIEW_TAIL_BYTES as usize + 1));
        write_jsonl(&root.join("-Users-x-repo"), id, &lines);
        let preview = preview_vendor_in("claude", &root, id).unwrap();
        assert!(preview.truncated);
        assert!(preview.messages.len() <= PREVIEW_MESSAGES);
    }

    #[test]
    fn preview_expands_title_to_its_detail_limit() {
        let root = temp_dir("preview-title");
        let id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let long_title = "가".repeat(600);
        write_jsonl(
            &root.join("-Users-x-repo"),
            id,
            &[
                meta_line(id, "/Users/x/repo", "main", "2.1.274"),
                serde_json::json!({"type":"custom-title","customTitle":long_title}).to_string(),
            ],
        );
        let preview = preview_vendor_in("claude", &root, id).unwrap();
        assert_eq!(
            preview.meta.title.unwrap().chars().count(),
            PREVIEW_TITLE_CHARS
        );
        assert!(preview.truncated);
    }

    #[cfg(unix)]
    #[test]
    fn scan_excludes_symlinked_project_dir() {
        use std::os::unix::fs::symlink;

        let root = temp_dir("scan-symlink");
        let id = "66666666-6666-6666-6666-666666666666";
        let real_dir = temp_dir("scan-symlink-target");
        write_jsonl(
            &real_dir,
            id,
            &[meta_line(id, "/Users/x/outside", "main", "2.1.274")],
        );
        symlink(&real_dir, root.join("-Users-x-outside")).unwrap();

        let out = scan_in(&root, &ScanFilter::default(), 10);
        assert!(
            out.is_empty(),
            "심볼릭 링크로 연결된 프로젝트 디렉터리는 인덱싱하지 않아야 함"
        );
    }

    #[test]
    fn scan_filters_by_cwd_prefix() {
        let root = temp_dir("scan-filter");
        let id_a = "77777777-7777-7777-7777-777777777777";
        let id_b = "88888888-8888-8888-8888-888888888888";
        write_jsonl(
            &root.join("-Users-x-repo"),
            id_a,
            &[meta_line(id_a, "/Users/x/repo", "main", "2.1.274")],
        );
        write_jsonl(
            &root.join("-Users-y-other"),
            id_b,
            &[meta_line(id_b, "/Users/y/other", "main", "2.1.274")],
        );

        let filter = ScanFilter {
            cwd_prefixes: vec!["/Users/x".to_string()],
            query: None,
        };
        let out = scan_in(&root, &filter, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].session_id, id_a);
    }

    #[test]
    fn codex_duplicate_events_do_not_erase_repeated_user_requests() {
        let root = temp_dir("codex-repeated-preview");
        let id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let path = root.join(format!("rollout-test-{id}.jsonl"));
        let response = codex_message("user", "repeat this request");
        let event = r#"{"type":"event_msg","payload":{"type":"user_message","message":"repeat this request"}}"#.to_string();
        std::fs::write(
            path,
            [
                codex_meta_line(id, "/repo"),
                response.clone(),
                event.clone(),
                response,
                event,
            ]
            .join("\n"),
        )
        .unwrap();
        let preview = preview_vendor_in("codex", &root, id).unwrap();
        assert_eq!(preview.messages.len(), 2);
    }

    #[test]
    fn branch_search_and_long_request_detail_are_available() {
        let root = temp_dir("branch-request-detail");
        let id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let request = "사용자 요청 ".repeat(100);
        write_jsonl(
            &root.join("project"),
            id,
            &[
                meta_line(id, "/Users/x/repo", "feature/unique-branch", "2.1.280"),
                user_line(&request),
            ],
        );
        let rows = scan_in(
            &root,
            &ScanFilter {
                cwd_prefixes: Vec::new(),
                query: Some("unique-branch".into()),
            },
            10,
        );
        assert_eq!(rows.len(), 1);
        assert!(rows[0].first_message.as_ref().unwrap().chars().count() <= 120);
        let preview = preview_vendor_in("claude", &root, id).unwrap();
        assert_eq!(preview.meta.first_message.unwrap().chars().count(), 512);
        assert!(preview.truncated);
    }

    #[cfg(unix)]
    #[test]
    fn cwd_filter_accepts_a_canonical_symlink_alias_without_prefix_leakage() {
        use std::os::unix::fs::symlink;

        let root = temp_dir("cwd-alias");
        let real = root.join("real-repository");
        std::fs::create_dir_all(&real).unwrap();
        let alias = root.join("alias-repository");
        symlink(&real, &alias).unwrap();
        assert!(cwd_has_prefix(
            real.to_str().unwrap(),
            alias.to_str().unwrap()
        ));
        assert!(!cwd_has_prefix(
            root.join("real-repository-other").to_str().unwrap(),
            real.to_str().unwrap()
        ));
    }

    #[test]
    fn scan_cache_invalidates_when_file_metadata_changes() {
        let root = temp_dir("cache-invalidate");
        let id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let path = write_jsonl(
            &root.join("-Users-x-repo"),
            id,
            &[
                meta_line(id, "/Users/x/repo", "main", "2.1.274"),
                user_line("one"),
            ],
        );
        assert_eq!(
            scan_in(&root, &ScanFilter::default(), 10)[0]
                .first_message
                .as_deref(),
            Some("one")
        );
        std::fs::write(
            &path,
            [
                meta_line(id, "/Users/x/repo", "main", "2.1.274"),
                user_line("two"),
            ]
            .join("\n"),
        )
        .unwrap();
        set_mtime_secs(&path, 3_000);
        assert_eq!(
            scan_in(&root, &ScanFilter::default(), 10)[0]
                .first_message
                .as_deref(),
            Some("two")
        );
    }

    /// 파일 mtime을 테스트가 원하는 값으로 되돌린다(정렬 검증용).
    fn set_mtime_secs(path: &Path, secs: u64) {
        let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
        let ft = filetime_set(path, time);
        assert!(ft, "mtime 설정 실패: {}", path.display());
    }

    fn filetime_set(path: &Path, time: std::time::SystemTime) -> bool {
        // `filetime` 크레이트 미의존 — libc utimes를 직접 호출하지 않고, std가 제공하는
        // File::set_modified로 충분하다(macOS·Linux·Windows 모두 안정화됨).
        std::fs::File::options()
            .write(true)
            .open(path)
            .and_then(|f| f.set_modified(time))
            .is_ok()
    }

    // ---- authorize_cwd ----

    #[test]
    fn authorize_cwd_none_is_rejected() {
        let root = temp_dir("authz-none");
        assert!(!authorize_cwd(&[root], None));
    }

    #[test]
    fn authorize_cwd_accepts_existing_path_under_root() {
        let root = temp_dir("authz-exists");
        let sub = root.join("repo").join("worktree");
        std::fs::create_dir_all(&sub).unwrap();
        let root_canon = root.canonicalize().unwrap();
        assert!(authorize_cwd(&[root_canon], Some(sub.to_str().unwrap())));
    }

    #[test]
    fn authorize_cwd_accepts_deleted_path_under_existing_root() {
        let root = temp_dir("authz-deleted");
        std::fs::create_dir_all(&root).unwrap();
        let root_canon = root.canonicalize().unwrap();
        let gone = root_canon.join("finished-worktree-that-was-removed");
        // gone은 만들지 않는다 — "끝난 워크트리가 정리됐다"를 재현.
        assert!(authorize_cwd(
            std::slice::from_ref(&root_canon),
            Some(gone.to_str().unwrap())
        ));
    }

    #[test]
    fn authorize_cwd_rejects_path_outside_roots() {
        let root = temp_dir("authz-outside-root");
        let outside = temp_dir("authz-outside-other");
        std::fs::create_dir_all(&outside).unwrap();
        let root_canon = root.canonicalize().unwrap_or(root);
        assert!(!authorize_cwd(
            &[root_canon],
            Some(outside.to_str().unwrap())
        ));
    }

    #[test]
    fn authorize_cwd_rejects_when_no_ancestor_exists_under_root() {
        let root = temp_dir("authz-no-ancestor");
        std::fs::create_dir_all(&root).unwrap();
        let root_canon = root.canonicalize().unwrap();
        let outside_missing = PathBuf::from("/definitely/not/under/root/at-all-xyz");
        assert!(!authorize_cwd(
            &[root_canon],
            Some(outside_missing.to_str().unwrap())
        ));
    }

    // ---- sanitize_snippet ----

    #[test]
    fn sanitize_snippet_collapses_control_chars_and_caps_length() {
        assert_eq!(
            sanitize_snippet("hello\tworld\n\nfoo"),
            Some("hello world foo".to_string())
        );
        assert_eq!(sanitize_snippet("   "), None);
        assert_eq!(sanitize_snippet(""), None);
        let long = "x".repeat(500);
        let capped = sanitize_snippet(&long).unwrap();
        assert_eq!(capped.chars().count(), 120);
    }
}

#[cfg(test)]
mod request_wrapper_tests {
    use super::display_user_text;
    #[test]
    fn known_harness_wrapper_preserves_the_actual_request() {
        assert_eq!(display_user_text("# Praxis synchronous turn contract\n...\n# User request\n# Workflow Harness\n...\nThis is conditional routing, not another mandatory profile transition or permission gate.\n\n세션을 개선하자"), Some("세션을 개선하자".into()));
        assert_eq!(
            display_user_text("<environment_context>cwd</environment_context>"),
            None
        );
        assert_eq!(
            display_user_text("# Workflow Harness\n새로운 형식을 설계해줘"),
            Some("# Workflow Harness\n새로운 형식을 설계해줘".into())
        );
    }
}
