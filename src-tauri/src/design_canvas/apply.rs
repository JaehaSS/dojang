//! Design Canvas AST 제안의 검증과 같은 파일 핸들에 대한 직접 기록.
//!
//! Node는 문법을 이해하는 제안자일 뿐이다. 이 모듈이 작업 경로, 심볼릭 링크, 해시와 실제
//! 기록을 소유한다. 따라서 개발 서버와 Canvas 페이지에는 쓰기 경로가 없다.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_REQUEST_ID: usize = 128;
const MAX_SOURCE: usize = 1_024;
const MAX_VALUE: usize = 2_048;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
// apply 성공은 최대 4MiB의 source를 JSON string으로 돌려준다. 제어 문자의 JSON escape까지
// 고려한 상한을 둬서 유효한 최대 파일이 stdout cap 때문에 잘리는 경우를 막는다.
const MAX_NODE_OUTPUT: u64 = MAX_FILE_BYTES * 6 + 64 * 1024;
const NODE_TIMEOUT: Duration = Duration::from_secs(8);

#[cfg(test)]
thread_local! {
    static FAIL_TEMP_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EditOperation {
    Inspect,
    Apply,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EditScope {
    Instance,
    Component,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignCanvasEditRequest {
    pub request_id: String,
    pub operation: EditOperation,
    pub source: String,
    #[serde(default)]
    pub instance_source: Option<String>,
    pub scope: EditScope,
    #[serde(default)]
    pub property: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub expected_hash: Option<String>,
    #[serde(default)]
    pub component_confirmed: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignCanvasEditResult {
    pub request_id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

impl DesignCanvasEditResult {
    pub fn rejected(
        request_id: impl Into<String>,
        reason: &str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            ok: false,
            reason: Some(reason.into()),
            message: Some(message.into()),
            file: None,
            line: None,
            column: None,
            hash: None,
            editable: None,
            shared: None,
            source: None,
            selection_source: None,
            class_name: None,
            kind: None,
        }
    }

    fn from_node(request_id: String, node: NodeProposal) -> Self {
        let mut reason = node.reason;
        let mut message = node.message;
        if !node.ok {
            reason.get_or_insert_with(|| "write_failed".into());
            message.get_or_insert_with(|| "AST 편집기가 실패 이유를 반환하지 않았습니다".into());
        }
        Self {
            request_id,
            ok: node.ok,
            reason,
            message,
            file: node.file,
            line: node.line,
            column: node.column,
            hash: node.hash,
            editable: node.editable,
            shared: node.shared,
            source: node.source,
            selection_source: node.selection_source,
            class_name: node.class_name,
            kind: node.kind,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeProposal {
    ok: bool,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    column: Option<u32>,
    #[serde(default)]
    hash: Option<String>,
    #[serde(default)]
    editable: Option<bool>,
    #[serde(default)]
    shared: Option<bool>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    selection_source: Option<String>,
    #[serde(default)]
    class_name: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    previous_hash: Option<String>,
}

#[derive(Debug)]
pub(crate) struct EditError {
    code: &'static str,
    message: String,
}

impl EditError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// 작업별 mutex. AST 제안과 final hash/write를 같은 순서로 실행해 한 작업의 빠른 입력이 서로를
/// stale 처리하지 않게 한다.
fn task_lock(task_id: i64) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<i64, Arc<Mutex<()>>>>> = OnceLock::new();
    LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(task_id)
        .or_default()
        .clone()
}

pub fn execute(
    task_id: i64,
    root: &Path,
    request: DesignCanvasEditRequest,
) -> DesignCanvasEditResult {
    execute_guarded(task_id, root, request, || Ok(()))
}

/// `before_write`는 proposal 생성 뒤, 같은 task mutex와 file-handle 기록 직전에 실행된다.
/// 호출자는 현재 Canvas surface와 DB task 수명을 여기서 다시 묶는다.
pub(crate) fn execute_guarded<F>(
    task_id: i64,
    root: &Path,
    request: DesignCanvasEditRequest,
    before_write: F,
) -> DesignCanvasEditResult
where
    F: FnOnce() -> Result<(), EditError>,
{
    if let Err(error) = validate_request(&request) {
        return DesignCanvasEditResult::rejected(request.request_id, error.code, error.message);
    }
    let request_id = request.request_id.clone();
    let lock = task_lock(task_id);
    let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
    match execute_locked(root, &request, before_write) {
        Ok(result) => result,
        Err(error) => DesignCanvasEditResult::rejected(request_id, error.code, error.message),
    }
}

fn execute_locked<F>(
    root: &Path,
    request: &DesignCanvasEditRequest,
    before_write: F,
) -> Result<DesignCanvasEditResult, EditError>
where
    F: FnOnce() -> Result<(), EditError>,
{
    let root = checked_root(root)?;
    // Node가 읽기 전에 page-controlled input에 담긴 두 source 모두를 검사한다.
    checked_source_path(&root, &request.source)?;
    if let Some(source) = &request.instance_source {
        checked_source_path(&root, source)?;
    }

    let proposal = node_proposal(&root, request)?;
    if !proposal.ok {
        return Ok(DesignCanvasEditResult::from_node(
            request.request_id.clone(),
            proposal,
        ));
    }
    validate_node_success(&root, &proposal)?;
    if matches!(request.operation, EditOperation::Inspect) {
        return Ok(DesignCanvasEditResult::from_node(
            request.request_id.clone(),
            proposal,
        ));
    }
    if proposal.shared == Some(true)
        && request.scope == EditScope::Component
        && request.component_confirmed != Some(true)
    {
        return Err(EditError::new(
            "confirmation_required",
            "공용 컴포넌트 변경은 모든 사용처에 적용됩니다",
        ));
    }
    let content = proposal
        .content
        .as_deref()
        .ok_or_else(|| EditError::new("write_failed", "편집 제안에 파일 내용이 없습니다"))?;
    if content.len() as u64 > MAX_FILE_BYTES {
        return Err(EditError::new(
            "write_failed",
            "수정 결과가 파일 크기 제한을 넘었습니다",
        ));
    }
    let previous_hash = proposal
        .previous_hash
        .as_deref()
        .ok_or_else(|| EditError::new("write_failed", "편집 제안에 원본 해시가 없습니다"))?;
    let file = proposal
        .file
        .as_deref()
        .expect("validate_node_success requires file");
    before_write()?;
    write_checked(
        &root,
        Path::new(file),
        request.expected_hash.as_deref(),
        previous_hash,
        content,
    )?;
    Ok(DesignCanvasEditResult::from_node(
        request.request_id.clone(),
        proposal,
    ))
}

fn validate_node_success(root: &Path, proposal: &NodeProposal) -> Result<(), EditError> {
    let (
        Some(file),
        Some(line),
        Some(column),
        Some(hash),
        Some(editable),
        Some(shared),
        Some(source),
        Some(selection_source),
        Some(class_name),
        Some(kind),
    ) = (
        proposal.file.as_deref(),
        proposal.line,
        proposal.column,
        proposal.hash.as_deref(),
        proposal.editable,
        proposal.shared,
        proposal.source.as_deref(),
        proposal.selection_source.as_deref(),
        proposal.class_name.as_deref(),
        proposal.kind.as_deref(),
    )
    else {
        return Err(EditError::new(
            "write_failed",
            "AST 편집 결과에 필수 메타데이터가 없습니다",
        ));
    };
    if line == 0
        || !is_sha256(hash)
        || !matches!(kind, "static" | "dynamic" | "none")
        || class_name.len() > MAX_VALUE
    {
        return Err(EditError::new(
            "write_failed",
            "AST 편집 결과 메타데이터가 올바르지 않습니다",
        ));
    }
    let _ = (column, editable, shared, selection_source);
    checked_relative_path(root, file)?;
    let (source_path, _, _) = split_source(source)?;
    if source_path != Path::new(file) {
        return Err(EditError::new(
            "path_outside_worktree",
            "편집 결과의 source와 file이 일치하지 않습니다",
        ));
    }
    Ok(())
}

fn validate_request(request: &DesignCanvasEditRequest) -> Result<(), EditError> {
    if !safe_text(&request.request_id, MAX_REQUEST_ID) || request.request_id.is_empty() {
        return Err(EditError::new(
            "invalid_request",
            "requestId가 올바르지 않습니다",
        ));
    }
    if !safe_text(&request.source, MAX_SOURCE) {
        return Err(EditError::new(
            "invalid_request",
            "source가 올바르지 않습니다",
        ));
    }
    if let Some(source) = &request.instance_source {
        if !safe_text(source, MAX_SOURCE) {
            return Err(EditError::new(
                "invalid_request",
                "instanceSource가 올바르지 않습니다",
            ));
        }
    }
    if let Some(hash) = &request.expected_hash {
        if !is_sha256(hash) {
            return Err(EditError::new(
                "invalid_request",
                "expectedHash가 SHA-256 형식이 아닙니다",
            ));
        }
    }
    match request.operation {
        EditOperation::Inspect => Ok(()),
        EditOperation::Apply => {
            let property = request
                .property
                .as_deref()
                .filter(|value| crate::designmode::CSS_WHITELIST.contains(value))
                .ok_or_else(|| EditError::new("invalid_request", "허용되지 않은 속성입니다"))?;
            let value = request
                .value
                .as_deref()
                .filter(|value| safe_text(value, MAX_VALUE) && !value.is_empty())
                .ok_or_else(|| EditError::new("invalid_request", "값이 올바르지 않습니다"))?;
            let _ = (property, value);
            Ok(())
        }
    }
}

/// native scheme 처리에서는 파일을 열지 않고 크기와 값 형식만 먼저 제한한다.
pub fn request_is_bounded(request: &DesignCanvasEditRequest) -> bool {
    validate_request(request).is_ok()
}

fn safe_text(value: &str, max: usize) -> bool {
    value.len() <= max && !value.chars().any(char::is_control)
}
fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn checked_root(root: &Path) -> Result<PathBuf, EditError> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|e| EditError::new("path_outside_worktree", e.to_string()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(EditError::new(
            "symlink_path",
            "작업 폴더가 심볼릭 링크이거나 디렉터리가 아닙니다",
        ));
    }
    root.canonicalize()
        .map_err(|e| EditError::new("path_outside_worktree", e.to_string()))
}

fn checked_source_path(root: &Path, source: &str) -> Result<PathBuf, EditError> {
    let (path, _, _) = split_source(source)?;
    checked_relative_path(
        root,
        path.to_str()
            .ok_or_else(|| EditError::new("invalid_request", "source 경로가 UTF-8이 아닙니다"))?,
    )
}

fn split_source(source: &str) -> Result<(&Path, u32, u32), EditError> {
    let (head, column) = source
        .rsplit_once(':')
        .ok_or_else(|| EditError::new("invalid_request", "source 위치 형식이 올바르지 않습니다"))?;
    let (file, line) = head
        .rsplit_once(':')
        .ok_or_else(|| EditError::new("invalid_request", "source 위치 형식이 올바르지 않습니다"))?;
    let line = line
        .parse()
        .ok()
        .filter(|line: &u32| *line > 0)
        .ok_or_else(|| EditError::new("invalid_request", "source 줄 번호가 올바르지 않습니다"))?;
    let column = column
        .parse()
        .ok()
        .ok_or_else(|| EditError::new("invalid_request", "source 열 번호가 올바르지 않습니다"))?;
    Ok((Path::new(file), line, column))
}

fn checked_relative_path(root: &Path, value: &str) -> Result<PathBuf, EditError> {
    let relative = Path::new(value);
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(EditError::new(
            "path_outside_worktree",
            "작업 폴더 밖 경로는 편집할 수 없습니다",
        ));
    }
    let mut current = root.to_path_buf();
    for part in relative.components() {
        current.push(part.as_os_str());
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| EditError::new("path_outside_worktree", error.to_string()))?;
        if metadata.file_type().is_symlink() {
            return Err(EditError::new(
                "symlink_path",
                "심볼릭 링크 경로는 편집할 수 없습니다",
            ));
        }
    }
    let metadata = fs::symlink_metadata(&current)
        .map_err(|error| EditError::new("path_outside_worktree", error.to_string()))?;
    if !metadata.is_file() {
        return Err(EditError::new(
            "write_failed",
            "편집 대상이 일반 파일이 아닙니다",
        ));
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(EditError::new(
            "write_failed",
            "편집 대상이 파일 크기 제한을 넘었습니다",
        ));
    }
    Ok(current)
}

fn node_proposal(
    root: &Path,
    request: &DesignCanvasEditRequest,
) -> Result<NodeProposal, EditError> {
    let input = serde_json::json!({
        "root": root, "operation": request.operation, "source": request.source,
        "instanceSource": request.instance_source, "scope": request.scope,
        "property": request.property, "value": request.value, "expectedHash": request.expected_hash,
    });
    let bytes = serde_json::to_vec(&input)
        .map_err(|error| EditError::new("write_failed", error.to_string()))?;
    let output = run_node(root, &bytes)?;
    serde_json::from_slice::<NodeProposal>(&output).map_err(|error| {
        EditError::new(
            "write_failed",
            format!("AST 편집 결과 형식이 올바르지 않습니다: {error}"),
        )
    })
}

fn run_node(root: &Path, input: &[u8]) -> Result<Vec<u8>, EditError> {
    let child = crate::managed_process::spawn_registered(
        "node",
        &[
            "--input-type=module".into(),
            "-e".into(),
            include_str!("../../../scripts/design-apply.mjs").into(),
        ],
        None,
        |process| {
            process
                .current_dir(root)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
        },
    )
    .map_err(|error| {
        EditError::new(
            "write_failed",
            format!("Node 편집기를 시작할 수 없습니다: {error}"),
        )
    })?
    .child;
    let mut child = ReapChild(child);
    let stdout = child
        .0
        .stdout
        .take()
        .ok_or_else(|| EditError::new("write_failed", "Node 출력을 열 수 없습니다"))?;
    let stderr = child
        .0
        .stderr
        .take()
        .ok_or_else(|| EditError::new("write_failed", "Node 오류 출력을 열 수 없습니다"))?;
    // 파이프를 계속 비워야 잘못된 스크립트가 큰 로그를 내도 wait 전에 서로 막히지 않는다.
    let stdout_reader = std::thread::spawn(move || read_pipe_bounded(stdout, MAX_NODE_OUTPUT));
    let stderr_reader = std::thread::spawn(move || read_pipe_bounded(stderr, MAX_NODE_OUTPUT));
    let mut stdin = child
        .0
        .stdin
        .take()
        .ok_or_else(|| EditError::new("write_failed", "Node 입력을 열 수 없습니다"))?;
    stdin
        .write_all(input)
        .map_err(|error| EditError::new("write_failed", error.to_string()))?;
    drop(stdin);
    let deadline = Instant::now() + NODE_TIMEOUT;
    loop {
        if let Some(status) = child
            .0
            .try_wait()
            .map_err(|error| EditError::new("write_failed", error.to_string()))?
        {
            let stdout = stdout_reader
                .join()
                .map_err(|_| EditError::new("write_failed", "Node 출력 수집이 중단되었습니다"))?;
            let stderr = stderr_reader.join().map_err(|_| {
                EditError::new("write_failed", "Node 오류 출력 수집이 중단되었습니다")
            })?;
            if !status.success() {
                return Err(EditError::new(
                    "write_failed",
                    format!(
                        "AST 편집기가 실패했습니다: {}",
                        String::from_utf8_lossy(&stderr).trim()
                    ),
                ));
            }
            return Ok(stdout);
        }
        if Instant::now() >= deadline {
            crate::verify::kill_group(child.0.id());
            let _ = child.0.kill();
            let _ = child.0.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(EditError::new(
                "write_failed",
                "AST 편집기가 시간 안에 끝나지 않았습니다",
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn read_pipe_bounded(mut pipe: impl Read, limit: u64) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let Ok(read) = pipe.read(&mut buffer) else {
            break;
        };
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(kept.len() as u64) as usize;
        kept.extend_from_slice(&buffer[..read.min(remaining)]);
    }
    kept
}

struct ReapChild(Child);
impl Drop for ReapChild {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(Some(_))) {
            return;
        }
        crate::verify::kill_group(self.0.id());
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(unix)]
fn write_checked(
    root: &Path,
    relative: &Path,
    expected_hash: Option<&str>,
    previous_hash: &str,
    content: &str,
) -> Result<(), EditError> {
    let mut target = open_anchored(root, relative)?;
    let metadata = target
        .file
        .metadata()
        .map_err(|error| EditError::new("write_failed", error.to_string()))?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err(EditError::new(
            "write_failed",
            "편집 대상이 일반 파일이 아니거나 너무 큽니다",
        ));
    }
    let current = read_limited(&mut target.file)?;
    let actual_hash = sha256(&current);
    if expected_hash.is_some_and(|expected| !expected.eq_ignore_ascii_case(&actual_hash))
        || !previous_hash.eq_ignore_ascii_case(&actual_hash)
    {
        return Err(EditError::new(
            "stale_file",
            "파일이 편집 제안 이후 변경되었습니다",
        ));
    }
    let temporary = write_temporary(&target, content, &metadata)?;
    let parent_metadata = target
        .parent
        .metadata()
        .map_err(|error| EditError::new("write_failed", error.to_string()))?;
    let commit = (|| {
        // 임시 파일을 모두 sync한 뒤에만 현재 경로를 다시 연다. 이때 디렉터리 교체·symlink
        // 삽입·같은 inode 내용 변경을 모두 확인하고, 통과한 경우에만 renameat을 호출한다.
        let mut current = open_anchored(root, relative)?;
        let current_metadata = current
            .file
            .metadata()
            .map_err(|error| EditError::new("write_failed", error.to_string()))?;
        let current_parent_metadata = current
            .parent
            .metadata()
            .map_err(|error| EditError::new("write_failed", error.to_string()))?;
        if !same_file(&parent_metadata, &current_parent_metadata)
            || !same_file(&metadata, &current_metadata)
            || sha256(&read_limited(&mut current.file)?) != actual_hash
        {
            return Err(EditError::new(
                "stale_file",
                "편집 대상이 변경되었거나 다른 파일로 교체되었습니다",
            ));
        }
        rename_temporary(&target, &temporary)
    })();
    if commit.is_err() {
        let _ = remove_temporary(&target, &temporary);
    }
    commit
}

// 직접 기록은 openat·renameat로 경로 교체를 막는 데 기대므로 unix에서만 연다.
#[cfg(not(unix))]
fn write_checked(
    _root: &Path,
    _relative: &Path,
    _expected_hash: Option<&str>,
    _previous_hash: &str,
    _content: &str,
) -> Result<(), EditError> {
    Err(EditError::new(
        "write_failed",
        "이 플랫폼에서는 디자인 캔버스 직접 기록을 아직 지원하지 않습니다",
    ))
}

#[cfg(unix)]
struct AnchoredTarget {
    file: File,
    parent: File,
    name: std::ffi::OsString,
}

#[cfg(unix)]
fn open_anchored(root: &Path, relative: &Path) -> Result<AnchoredTarget, EditError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd, RawFd};
    use std::os::unix::ffi::OsStrExt;

    fn open_error(error: std::io::Error) -> EditError {
        let code = match error.raw_os_error() {
            Some(nix::libc::ELOOP | nix::libc::ENOTDIR) => "symlink_path",
            _ => "write_failed",
        };
        EditError::new(code, error.to_string())
    }
    fn c_path(path: &std::ffi::OsStr) -> Result<CString, EditError> {
        CString::new(path.as_bytes())
            .map_err(|_| EditError::new("path_outside_worktree", "경로에 NUL 문자가 있습니다"))
    }
    fn open_at(fd: RawFd, name: &std::ffi::OsStr, flags: i32) -> Result<RawFd, EditError> {
        let name = c_path(name)?;
        let opened = unsafe { nix::libc::openat(fd, name.as_ptr(), flags, 0) };
        if opened < 0 {
            Err(open_error(std::io::Error::last_os_error()))
        } else {
            Ok(opened)
        }
    }
    let root_name = c_path(root.as_os_str())?;
    let fd = unsafe {
        nix::libc::open(
            root_name.as_ptr(),
            nix::libc::O_RDONLY
                | nix::libc::O_DIRECTORY
                | nix::libc::O_NOFOLLOW
                | nix::libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(open_error(std::io::Error::last_os_error()));
    }
    let mut parent = unsafe { File::from_raw_fd(fd) };
    let parts: Vec<_> = relative.components().collect();
    let (name, parents) = parts
        .split_last()
        .ok_or_else(|| EditError::new("path_outside_worktree", "빈 대상 경로입니다"))?;
    for part in parents {
        let next = open_at(
            parent.as_raw_fd(),
            part.as_os_str(),
            nix::libc::O_RDONLY
                | nix::libc::O_DIRECTORY
                | nix::libc::O_NOFOLLOW
                | nix::libc::O_CLOEXEC,
        )?;
        parent = unsafe { File::from_raw_fd(next) };
    }
    let file = open_at(
        parent.as_raw_fd(),
        name.as_os_str(),
        nix::libc::O_RDWR | nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC,
    )?;
    Ok(AnchoredTarget {
        file: unsafe { File::from_raw_fd(file) },
        parent,
        name: name.as_os_str().to_os_string(),
    })
}

#[cfg(unix)]
fn write_temporary(
    target: &AnchoredTarget,
    content: &str,
    metadata: &fs::Metadata,
) -> Result<std::ffi::OsString, EditError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    static TEMP_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let name = std::ffi::OsString::from(format!(
        ".dojang-canvas-{}-{}.tmp",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let c_name = CString::new(name.as_bytes())
        .map_err(|_| EditError::new("write_failed", "임시 파일 이름이 올바르지 않습니다"))?;
    let fd = unsafe {
        nix::libc::openat(
            target.parent.as_raw_fd(),
            c_name.as_ptr(),
            nix::libc::O_WRONLY
                | nix::libc::O_CREAT
                | nix::libc::O_EXCL
                | nix::libc::O_NOFOLLOW
                | nix::libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(EditError::new(
            "write_failed",
            std::io::Error::last_os_error().to_string(),
        ));
    }
    let result = (|| {
        let mut file = unsafe { File::from_raw_fd(fd) };
        #[cfg(test)]
        if FAIL_TEMP_WRITE.with(|fail| fail.get()) {
            return Err(EditError::new(
                "write_failed",
                "injected temporary write failure",
            ));
        }
        file.write_all(content.as_bytes())
            .map_err(|error| EditError::new("write_failed", error.to_string()))?;
        file.sync_all()
            .map_err(|error| EditError::new("write_failed", error.to_string()))?;
        if unsafe { nix::libc::fchmod(file.as_raw_fd(), metadata.mode() as nix::libc::mode_t) } < 0
        {
            return Err(EditError::new(
                "write_failed",
                std::io::Error::last_os_error().to_string(),
            ));
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = remove_temporary(target, &name);
    }
    result.map(|()| name)
}

#[cfg(unix)]
fn rename_temporary(target: &AnchoredTarget, temporary: &std::ffi::OsStr) -> Result<(), EditError> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    let temporary = CString::new(temporary.as_bytes())
        .map_err(|_| EditError::new("write_failed", "임시 파일 이름이 올바르지 않습니다"))?;
    let target_name = CString::new(target.name.as_bytes())
        .map_err(|_| EditError::new("write_failed", "대상 파일 이름이 올바르지 않습니다"))?;
    let result = unsafe {
        nix::libc::renameat(
            target.parent.as_raw_fd(),
            temporary.as_ptr(),
            target.parent.as_raw_fd(),
            target_name.as_ptr(),
        )
    };
    if result < 0 {
        Err(EditError::new(
            "write_failed",
            std::io::Error::last_os_error().to_string(),
        ))
    } else {
        target
            .parent
            .sync_all()
            .map_err(|error| EditError::new("write_failed", error.to_string()))
    }
}

#[cfg(unix)]
fn remove_temporary(target: &AnchoredTarget, temporary: &std::ffi::OsStr) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    let name = CString::new(temporary.as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    if unsafe { nix::libc::unlinkat(target.parent.as_raw_fd(), name.as_ptr(), 0) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

fn read_limited(file: &mut File) -> Result<Vec<u8>, EditError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| EditError::new("write_failed", error.to_string()))?;
    let mut content = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut content)
        .map_err(|error| EditError::new("write_failed", error.to_string()))?;
    if content.len() as u64 > MAX_FILE_BYTES {
        return Err(EditError::new(
            "write_failed",
            "편집 대상이 파일 크기 제한을 넘었습니다",
        ));
    }
    Ok(content)
}
fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NONCE: AtomicU64 = AtomicU64::new(0);

    fn root(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "dojang-design-apply-{name}-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    struct RepoFixture {
        root: PathBuf,
        directory: PathBuf,
        relative: String,
        files: HashMap<String, String>,
    }

    impl Drop for RepoFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    fn repo_fixture(name: &str, files: &[(&str, &str)]) -> RepoFixture {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let relative = format!(
            ".dojang-design-write-test-{name}-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        );
        let directory = root.join(&relative);
        fs::create_dir_all(&directory).unwrap();
        let mut saved = HashMap::new();
        for (path, content) in files {
            fs::write(directory.join(path), content).unwrap();
            saved.insert((*path).to_string(), (*content).to_string());
        }
        RepoFixture {
            root,
            directory,
            relative,
            files: saved,
        }
    }

    impl RepoFixture {
        fn path(&self, file: &str) -> PathBuf {
            self.directory.join(file)
        }

        fn source_at(&self, file: &str, needle: &str) -> String {
            let content = self.files.get(file).unwrap();
            let index = content
                .find(needle)
                .unwrap_or_else(|| panic!("missing {needle}"));
            let line = content[..index]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                + 1;
            let column = index
                - content[..index]
                    .rfind('\n')
                    .map(|position| position + 1)
                    .unwrap_or(0);
            format!("{}/{file}:{line}:{column}", self.relative)
        }
    }

    fn request(
        request_id: &str,
        operation: EditOperation,
        source: String,
        instance_source: Option<String>,
        scope: EditScope,
    ) -> DesignCanvasEditRequest {
        DesignCanvasEditRequest {
            request_id: request_id.into(),
            operation,
            source,
            instance_source,
            scope,
            property: None,
            value: None,
            expected_hash: None,
            component_confirmed: None,
        }
    }

    fn apply_request(
        request_id: &str,
        source: String,
        instance_source: Option<String>,
        scope: EditScope,
        hash: String,
        confirmed: bool,
    ) -> DesignCanvasEditRequest {
        let mut request = request(
            request_id,
            EditOperation::Apply,
            source,
            instance_source,
            scope,
        );
        request.property = Some("padding".into());
        request.value = Some("8px".into());
        request.expected_hash = Some(hash);
        request.component_confirmed = Some(confirmed);
        request
    }

    #[test]
    fn embedded_node_inspect_apply_and_stale_guard_use_the_real_writer() {
        let fixture = repo_fixture("static", &[
            ("Card.tsx", "export function Card() {\n  return <div className=\"p-4 text-sm\">card</div>;\n}\n"),
        ]);
        let source = fixture.source_at("Card.tsx", "<div");
        let inspect = execute_guarded(
            900_001,
            &fixture.root,
            request(
                "inspect",
                EditOperation::Inspect,
                source.clone(),
                None,
                EditScope::Component,
            ),
            || Ok(()),
        );
        assert!(inspect.ok);
        assert_eq!(inspect.editable, Some(true));
        let apply = execute_guarded(
            900_001,
            &fixture.root,
            apply_request(
                "apply",
                source.clone(),
                None,
                EditScope::Component,
                inspect.hash.clone().unwrap(),
                true,
            ),
            || Ok(()),
        );
        assert!(apply.ok);
        assert!(fs::read_to_string(fixture.path("Card.tsx"))
            .unwrap()
            .contains("className=\"p-[8px] text-sm\""));

        let inspect = execute_guarded(
            900_002,
            &fixture.root,
            request(
                "inspect-stale",
                EditOperation::Inspect,
                source.clone(),
                None,
                EditScope::Component,
            ),
            || Ok(()),
        );
        let changed = fixture.path("Card.tsx");
        let stale = execute_guarded(
            900_002,
            &fixture.root,
            apply_request(
                "stale",
                source,
                None,
                EditScope::Component,
                inspect.hash.unwrap(),
                true,
            ),
            move || {
                fs::write(
                    &changed,
                    "export function Card() { return <div className=\"p-99\">external</div>; }\n",
                )
                .map_err(|error| EditError::new("write_failed", error.to_string()))
            },
        );
        assert!(!stale.ok);
        assert_eq!(stale.reason.as_deref(), Some("stale_file"));
        assert!(fs::read_to_string(fixture.path("Card.tsx"))
            .unwrap()
            .contains("p-99"));
    }

    #[test]
    fn embedded_node_instance_and_component_scope_preserve_the_other_file() {
        let fixture = repo_fixture("scopes", &[
            ("Card.tsx", "export interface CardProps { className?: string }\nexport function Card({ className }: CardProps) { return <div className={className}>card</div>; }\n"),
            ("Screen.tsx", "import { Card } from './Card';\nexport const Screen = () => <Card className=\"p-2\" />;\n"),
            ("Default.tsx", "export default function Default({ className }) { return <div className={className}>default</div>; }\n"),
            ("UsesDefault.tsx", "import Default from './Default';\nexport const UsesDefault = () => <Default className=\"p-3\" />;\n"),
            ("Shared.tsx", "export function Shared() { return <div className=\"p-1\">shared</div>; }\n"),
        ]);
        let definition = fixture.source_at("Card.tsx", "<div");
        let usage = fixture.source_at("Screen.tsx", "<Card");
        let inspect = execute_guarded(
            900_003,
            &fixture.root,
            request(
                "instance-inspect",
                EditOperation::Inspect,
                definition.clone(),
                Some(usage.clone()),
                EditScope::Instance,
            ),
            || Ok(()),
        );
        assert!(inspect.ok && inspect.shared == Some(true));
        let instance = execute_guarded(
            900_003,
            &fixture.root,
            apply_request(
                "instance-apply",
                definition,
                Some(usage),
                EditScope::Instance,
                inspect.hash.unwrap(),
                false,
            ),
            || Ok(()),
        );
        assert!(instance.ok);
        assert!(fs::read_to_string(fixture.path("Screen.tsx"))
            .unwrap()
            .contains("className=\"p-[8px]\""));
        assert!(fs::read_to_string(fixture.path("Card.tsx"))
            .unwrap()
            .contains("className={className}"));

        let default_definition = fixture.source_at("Default.tsx", "<div");
        let default_usage = fixture.source_at("UsesDefault.tsx", "<Default");
        let inspect = execute_guarded(
            900_006,
            &fixture.root,
            request(
                "default-inspect",
                EditOperation::Inspect,
                default_definition.clone(),
                Some(default_usage.clone()),
                EditScope::Instance,
            ),
            || Ok(()),
        );
        let default_apply = execute_guarded(
            900_006,
            &fixture.root,
            apply_request(
                "default-apply",
                default_definition,
                Some(default_usage),
                EditScope::Instance,
                inspect.hash.unwrap(),
                false,
            ),
            || Ok(()),
        );
        assert!(default_apply.ok);
        assert!(fs::read_to_string(fixture.path("UsesDefault.tsx"))
            .unwrap()
            .contains("className=\"p-[8px]\""));

        let shared = fixture.source_at("Shared.tsx", "<div");
        let inspect = execute_guarded(
            900_004,
            &fixture.root,
            request(
                "component-inspect",
                EditOperation::Inspect,
                shared.clone(),
                None,
                EditScope::Component,
            ),
            || Ok(()),
        );
        let refused = execute_guarded(
            900_004,
            &fixture.root,
            apply_request(
                "component-refused",
                shared.clone(),
                None,
                EditScope::Component,
                inspect.hash.clone().unwrap(),
                false,
            ),
            || Ok(()),
        );
        assert_eq!(refused.reason.as_deref(), Some("confirmation_required"));
        let accepted = execute_guarded(
            900_004,
            &fixture.root,
            apply_request(
                "component-accepted",
                shared,
                None,
                EditScope::Component,
                inspect.hash.unwrap(),
                true,
            ),
            || Ok(()),
        );
        assert!(accepted.ok);
        assert!(fs::read_to_string(fixture.path("Shared.tsx"))
            .unwrap()
            .contains("p-[8px]"));
    }

    #[test]
    fn embedded_node_reports_dynamic_class_without_writing() {
        let fixture = repo_fixture(
            "dynamic",
            &[(
                "Dynamic.tsx",
                "export const Dynamic = () => <div className={ready ? 'p-1' : 'p-2'} />;\n",
            )],
        );
        let source = fixture.source_at("Dynamic.tsx", "<div");
        let inspect = execute_guarded(
            900_005,
            &fixture.root,
            request(
                "dynamic-inspect",
                EditOperation::Inspect,
                source.clone(),
                None,
                EditScope::Component,
            ),
            || Ok(()),
        );
        assert!(inspect.ok);
        assert_eq!(inspect.editable, Some(false));
        assert_eq!(inspect.reason.as_deref(), Some("dynamic_classname"));
        assert_eq!(inspect.class_name.as_deref(), Some(""));
        let apply = execute_guarded(
            900_005,
            &fixture.root,
            apply_request(
                "dynamic-apply",
                source,
                None,
                EditScope::Component,
                inspect.hash.unwrap(),
                true,
            ),
            || Ok(()),
        );
        assert!(!apply.ok);
        assert_eq!(apply.reason.as_deref(), Some("dynamic_classname"));
    }

    #[test]
    fn rejects_path_outside_worktree() {
        let root = root("outside");
        let error = checked_relative_path(&root, "../outside.tsx").unwrap_err();
        assert_eq!(error.code, "path_outside_worktree");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn writes_same_regular_file_when_hashes_match() {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;
        let root = root("write");
        let target = root.join("screen.tsx");
        fs::write(&target, "const gap = 'p-4';\n").unwrap();
        #[cfg(unix)]
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        let previous = sha256(b"const gap = 'p-4';\n");
        write_checked(
            &root,
            Path::new("screen.tsx"),
            Some(&previous),
            &previous,
            "const gap = 'p-5';\n",
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "const gap = 'p-5';\n");
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn temporary_write_failure_leaves_source_unchanged() {
        let root = root("temporary-failure");
        let target = root.join("screen.tsx");
        fs::write(&target, "const gap = 'p-4';\n").unwrap();
        let previous = sha256(b"const gap = 'p-4';\n");
        FAIL_TEMP_WRITE.with(|fail| fail.set(true));
        let error = write_checked(
            &root,
            Path::new("screen.tsx"),
            Some(&previous),
            &previous,
            "const gap = 'p-5';\n",
        )
        .unwrap_err();
        FAIL_TEMP_WRITE.with(|fail| fail.set(false));
        assert_eq!(error.code, "write_failed");
        assert_eq!(fs::read_to_string(&target).unwrap(), "const gap = 'p-4';\n");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_file_between_proposal_and_write_is_refused() {
        let root = root("stale");
        let target = root.join("screen.tsx");
        fs::write(&target, "const gap = 'p-4';\n").unwrap();
        let proposal_hash = sha256(b"const gap = 'p-4';\n");
        // This models an agent write after the AST proposal but before the writer opens its handle.
        fs::write(&target, "const gap = 'p-6';\n").unwrap();
        let error = write_checked(
            &root,
            Path::new("screen.tsx"),
            Some(&proposal_hash),
            &proposal_hash,
            "const gap = 'p-5';\n",
        )
        .unwrap_err();
        assert_eq!(error.code, "stale_file");
        assert_eq!(fs::read_to_string(&target).unwrap(), "const gap = 'p-6';\n");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_intermediate_and_final_symlinks() {
        use std::os::unix::fs::symlink;
        let worktree = root("symlink");
        let outside = root("outside-target");
        fs::write(outside.join("target.tsx"), "outside").unwrap();
        symlink(&outside, worktree.join("linked-dir")).unwrap();
        let intermediate = checked_relative_path(&worktree, "linked-dir/target.tsx").unwrap_err();
        assert_eq!(intermediate.code, "symlink_path");
        symlink(outside.join("target.tsx"), worktree.join("linked-file.tsx")).unwrap();
        let final_link = checked_relative_path(&worktree, "linked-file.tsx").unwrap_err();
        assert_eq!(final_link.code, "symlink_path");
        fs::remove_dir_all(worktree).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn parent_symlink_substituted_after_proposal_cannot_escape_root() {
        use std::os::unix::fs::symlink;
        let worktree = root("parent-swap");
        let directory = worktree.join("src");
        fs::create_dir(&directory).unwrap();
        let target = directory.join("screen.tsx");
        fs::write(&target, "const gap = 'p-4';\n").unwrap();
        let proposal_hash = sha256(b"const gap = 'p-4';\n");
        checked_relative_path(&worktree, "src/screen.tsx").unwrap();
        let outside = root("parent-swap-outside");
        fs::write(outside.join("screen.tsx"), "outside\n").unwrap();
        fs::remove_file(&target).unwrap();
        fs::remove_dir(&directory).unwrap();
        symlink(&outside, &directory).unwrap();
        let error = write_checked(
            &worktree,
            Path::new("src/screen.tsx"),
            Some(&proposal_hash),
            &proposal_hash,
            "changed\n",
        )
        .unwrap_err();
        assert_eq!(error.code, "symlink_path");
        assert_eq!(
            fs::read_to_string(outside.join("screen.tsx")).unwrap(),
            "outside\n"
        );
        fs::remove_dir_all(worktree).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
