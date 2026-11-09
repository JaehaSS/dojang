//! Explicit, task-local pull request delivery.  A preview is a short-lived immutable intent;
//! every mutating action rechecks the checkout before it starts a subprocess.

use super::pr_status::{compact_error, parse_public_github_origin, run_bounded};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const TIMEOUT: Duration = Duration::from_secs(20);
const OUTPUT_LIMIT: usize = 128 * 1024;
const PREVIEW_TTL_SECS: i64 = 15 * 60;
const MAX_COMMITS: usize = 80;
const MAX_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct PrDeliveryInput {
    pub title: Option<String>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrDeliveryPreview {
    pub token: String,
    pub repository: String,
    pub branch: String,
    pub base: String,
    pub head: String,
    pub title: String,
    pub body: String,
    pub commits: Vec<PrDeliveryCommit>,
    pub verification_summary: String,
    pub template_path: Option<String>,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrDeliveryCommit {
    pub sha: String,
    pub subject: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrDeliveryAction {
    pub action: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrDeliveryResult {
    pub action: String,
    pub repository: String,
    pub branch: String,
    pub head: String,
    pub pr_number: Option<u64>,
    pub url: Option<String>,
    pub message: String,
}

#[derive(Clone)]
pub struct DeliveryRequest {
    pub task_id: i64,
    pub worktree: PathBuf,
    pub branch: String,
    pub base: String,
    pub verification_summary: String,
}

#[derive(Clone)]
struct Prepared {
    request: DeliveryRequest,
    repository: String,
    origin_url: String,
    push_urls: Vec<String>,
    canonical_worktree: PathBuf,
    head: String,
    base_head: String,
    title: String,
    body: String,
    template: Option<Template>,
    expires_at: i64,
}

#[derive(Clone)]
struct Template {
    path: PathBuf,
    hash: String,
    content: String,
}

fn prepared() -> &'static Mutex<HashMap<String, Prepared>> {
    static TOKENS: OnceLock<Mutex<HashMap<String, Prepared>>> = OnceLock::new();
    TOKENS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn token_for(value: &Prepared) -> String {
    static COUNTER: OnceLock<Mutex<u64>> = OnceLock::new();
    let mut counter = COUNTER
        .get_or_init(|| Mutex::new(0))
        .lock()
        .expect("token mutex");
    *counter += 1;
    let mut hash = Sha256::new();
    hash.update(value.request.task_id.to_le_bytes());
    hash.update(value.head.as_bytes());
    hash.update(value.body.as_bytes());
    hash.update(now().to_le_bytes());
    hash.update(counter.to_le_bytes());
    format!("pr-{:x}", hash.finalize())
}

/// Prepares the exact title/body.  It never contacts GitHub and has no remote side effects.
pub fn preview(
    request: DeliveryRequest,
    input: PrDeliveryInput,
) -> Result<PrDeliveryPreview, String> {
    let canonical_worktree = std::fs::canonicalize(&request.worktree)
        .map_err(|_| "작업 디렉터리를 확인할 수 없습니다.".to_string())?;
    valid_ref(&request.branch, "작업 브랜치")?;
    valid_ref(&request.base, "기준 브랜치")?;
    if request.branch == request.base {
        return Err("작업 브랜치와 기준 브랜치가 같을 수 없습니다.".into());
    }
    let (origin_url, repository, push_urls) = remote_identity(&canonical_worktree)?;
    let head = identity(&canonical_worktree, &request.branch)?;
    let base_head = revision(&canonical_worktree, &request.base)?;
    let commits = commits(&canonical_worktree, &request.base)?;
    if commits.is_empty() {
        return Err("기준 브랜치 이후의 커밋이 없습니다.".into());
    }
    let template = template(&canonical_worktree)?;
    let title = input
        .title
        .unwrap_or_else(|| commits[0].subject.clone())
        .trim()
        .to_string();
    if title.is_empty() {
        return Err("PR 제목이 비어 있습니다.".into());
    }
    if title.len() > 256 {
        return Err("PR 제목이 너무 깁니다.".into());
    }
    let body = input.body.unwrap_or_else(|| {
        default_body(template.as_ref(), &commits, &request.verification_summary)
    });
    if body.len() > MAX_BODY_BYTES {
        return Err("PR 본문이 너무 깁니다.".into());
    }
    let expires_at = now() + PREVIEW_TTL_SECS;
    let entry = Prepared {
        request,
        repository: repository.clone(),
        origin_url,
        push_urls,
        canonical_worktree,
        head: head.clone(),
        base_head,
        title: title.clone(),
        body: body.clone(),
        template: template.clone(),
        expires_at,
    };
    let token = token_for(&entry);
    let mut store = prepared()
        .lock()
        .map_err(|_| "PR 준비 상태를 잠갔습니다.".to_string())?;
    store.retain(|_, value| value.expires_at >= now());
    store.insert(token.clone(), entry);
    Ok(PrDeliveryPreview {
        token: token.clone(),
        repository,
        branch: store.get(&token).unwrap().request.branch.clone(),
        base: store.get(&token).unwrap().request.base.clone(),
        head,
        title,
        body,
        commits,
        verification_summary: store
            .get(&token)
            .unwrap()
            .request
            .verification_summary
            .clone(),
        template_path: template.map(|value| value.path.to_string_lossy().into_owned()),
        expires_at,
    })
}

/// Consumes a prepared token before any remote mutation.  A failed action is never replayable.
pub fn execute(
    task_id: i64,
    token: &str,
    action: PrDeliveryAction,
) -> Result<PrDeliveryResult, String> {
    execute_inner(task_id, token, action, None)
}

/// Checks that the current task row still describes the worktree, branch, and base which the
/// token was prepared for.  It has no side effect; `execute` consumes the token.
pub fn matches_task(
    token: &str,
    task_id: i64,
    worktree: &Path,
    branch: &str,
    base: &str,
) -> Result<(), String> {
    let canonical = std::fs::canonicalize(worktree)
        .map_err(|_| "작업 디렉터리를 확인할 수 없습니다.".to_string())?;
    let store = prepared()
        .lock()
        .map_err(|_| "PR 준비 상태를 잠갔습니다.".to_string())?;
    let value = store.get(token).ok_or_else(|| {
        "PR 미리보기가 없거나 이미 사용되었습니다. 다시 미리보기하세요.".to_string()
    })?;
    if value.request.task_id != task_id
        || value.canonical_worktree != canonical
        || value.request.branch != branch
        || value.request.base != base
    {
        return Err(
            "PR 미리보기와 현재 작업 정보가 일치하지 않습니다. 다시 미리보기하세요.".into(),
        );
    }
    Ok(())
}

fn execute_inner(
    task_id: i64,
    token: &str,
    action: PrDeliveryAction,
    gh: Option<&Path>,
) -> Result<PrDeliveryResult, String> {
    let prepared = prepared()
        .lock()
        .map_err(|_| "PR 준비 상태를 잠갔습니다.".to_string())?
        .remove(token)
        .ok_or_else(|| {
            "PR 미리보기가 없거나 이미 사용되었습니다. 다시 미리보기하세요.".to_string()
        })?;
    if prepared.request.task_id != task_id {
        return Err("PR 미리보기의 작업이 일치하지 않습니다.".into());
    }
    if prepared.expires_at < now() {
        return Err("PR 미리보기의 유효 기간이 지났습니다. 다시 미리보기하세요.".into());
    }
    match action.action.as_str() {
        "push" | "create" | "update" => {}
        _ => return Err("알 수 없는 PR 전달 동작입니다.".into()),
    }
    revalidate(&prepared)?;
    match action.action.as_str() {
        "push" => push(&prepared),
        "create" => create_or_update(
            &prepared,
            false,
            match gh {
                Some(bin) => bin,
                None => gh_bin()?,
            },
        ),
        "update" => create_or_update(
            &prepared,
            true,
            match gh {
                Some(bin) => bin,
                None => gh_bin()?,
            },
        ),
        _ => unreachable!(),
    }
}

fn remote_identity(worktree: &Path) -> Result<(String, String, Vec<String>), String> {
    let origin_url = git(worktree, &["remote", "get-url", "origin"])?
        .trim()
        .to_string();
    let repository = parse_public_github_origin(&origin_url)
        .ok_or_else(|| "origin이 github.com 저장소를 가리키지 않습니다.".to_string())?;
    let push_urls: Vec<String> = git(
        worktree,
        &["remote", "get-url", "--push", "--all", "origin"],
    )?
    .lines()
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .map(str::to_string)
    .collect();
    if push_urls.len() != 1
        || parse_public_github_origin(&push_urls[0]).as_deref() != Some(repository.as_str())
    {
        return Err("origin push 대상이 하나의 같은 github.com 저장소가 아닙니다.".into());
    }
    Ok((origin_url, repository, push_urls))
}

fn revision(worktree: &Path, reference: &str) -> Result<String, String> {
    let value = git(worktree, &["rev-parse", reference])?.trim().to_string();
    if valid_sha(&value) {
        Ok(value)
    } else {
        Err("기준 브랜치의 commit 형식이 올바르지 않습니다.".into())
    }
}

fn identity(worktree: &Path, branch: &str) -> Result<String, String> {
    let top = git(worktree, &["rev-parse", "--show-toplevel"])?;
    if std::fs::canonicalize(top.trim())
        .map_err(|_| "Git checkout 경로를 확인할 수 없습니다.".to_string())?
        != worktree
    {
        return Err("요청한 작업 디렉터리가 실제 Git checkout과 일치하지 않습니다.".into());
    }
    if git(worktree, &["symbolic-ref", "--quiet", "--short", "HEAD"])?.trim() != branch {
        return Err("작업 디렉터리가 요청한 브랜치를 checkout하지 않았습니다.".into());
    }
    let head = git(worktree, &["rev-parse", "HEAD"])?;
    let head = head.trim().to_string();
    if !valid_sha(&head) {
        return Err("작업 디렉터리의 HEAD 형식이 올바르지 않습니다.".into());
    }
    Ok(head)
}

fn commits(worktree: &Path, base: &str) -> Result<Vec<PrDeliveryCommit>, String> {
    let range = format!("{base}..HEAD");
    let output = git(
        worktree,
        &["log", "--format=%H%x00%s%x1e", "--max-count=81", &range],
    )?;
    let values: Vec<_> = output
        .split('\x1e')
        .filter_map(|row| row.strip_suffix('\n').or(Some(row)))
        .filter(|row| !row.is_empty())
        .map(|row| {
            let (sha, subject) = row
                .split_once('\0')
                .ok_or_else(|| "커밋 목록 형식이 올바르지 않습니다.".to_string())?;
            if !valid_sha(sha) || subject.trim().is_empty() {
                return Err("커밋 목록 형식이 올바르지 않습니다.".into());
            }
            Ok(PrDeliveryCommit {
                sha: sha.into(),
                subject: subject.into(),
            })
        })
        .collect::<Result<_, String>>()?;
    if values.len() > MAX_COMMITS {
        return Err("PR에 포함할 커밋이 너무 많습니다.".into());
    }
    Ok(values)
}

fn template(worktree: &Path) -> Result<Option<Template>, String> {
    for relative in [
        ".github/pull_request_template.md",
        "PULL_REQUEST_TEMPLATE.md",
    ] {
        let path = worktree.join(relative);
        if path.is_file() {
            let canonical = std::fs::canonicalize(&path)
                .map_err(|_| "PR 템플릿을 확인하지 못했습니다.".to_string())?;
            if !canonical.starts_with(worktree) {
                return Err("PR 템플릿이 작업 디렉터리 밖을 가리킵니다.".into());
            }
            let metadata = std::fs::metadata(&canonical)
                .map_err(|_| "PR 템플릿을 확인하지 못했습니다.".to_string())?;
            if metadata.len() > MAX_BODY_BYTES as u64 {
                return Err("PR 템플릿이 너무 큽니다.".into());
            }
            let bytes = std::fs::read(&canonical)
                .map_err(|_| "PR 템플릿을 읽지 못했습니다.".to_string())?;
            if bytes.len() > MAX_BODY_BYTES {
                return Err("PR 템플릿이 너무 큽니다.".into());
            }
            let content = String::from_utf8(bytes.clone())
                .map_err(|_| "PR 템플릿의 UTF-8 형식이 올바르지 않습니다.".to_string())?;
            return Ok(Some(Template {
                path: canonical,
                hash: hash(&bytes),
                content,
            }));
        }
    }
    Ok(None)
}

fn default_body(
    template: Option<&Template>,
    commits: &[PrDeliveryCommit],
    verification: &str,
) -> String {
    let mut body = template
        .map(|template| template.content.clone())
        .unwrap_or_default();
    if !body.trim().is_empty() {
        body.push_str("\n\n");
    }
    body.push_str("## 변경 커밋\n");
    for commit in commits {
        body.push_str(&format!("- {} {}\n", &commit.sha[..12], commit.subject));
    }
    if !verification.trim().is_empty() {
        body.push_str("\n## 검증\n");
        body.push_str(verification.trim());
        body.push('\n');
    }
    body
}

fn revalidate(value: &Prepared) -> Result<(), String> {
    let canonical = std::fs::canonicalize(&value.request.worktree)
        .map_err(|_| "작업 디렉터리를 확인할 수 없습니다.".to_string())?;
    if canonical != value.canonical_worktree {
        return Err("작업 디렉터리 식별 정보가 변경되었습니다. 다시 미리보기하세요.".into());
    }
    let (origin_url, repository, push_urls) = remote_identity(&canonical)?;
    if repository != value.repository
        || origin_url != value.origin_url
        || push_urls != value.push_urls
    {
        return Err("origin 저장소가 변경되었습니다. 다시 미리보기하세요.".into());
    }
    if identity(&canonical, &value.request.branch)? != value.head {
        return Err("HEAD 또는 브랜치가 변경되었습니다. 다시 미리보기하세요.".into());
    }
    if revision(&canonical, &value.request.base)? != value.base_head {
        return Err("기준 브랜치가 변경되었습니다. 다시 미리보기하세요.".into());
    }
    let current_template = template(&canonical)?;
    if current_template.as_ref().map(|v| (&v.path, &v.hash))
        != value.template.as_ref().map(|v| (&v.path, &v.hash))
    {
        return Err("PR 템플릿이 변경되었습니다. 다시 미리보기하세요.".into());
    }
    Ok(())
}

fn push(value: &Prepared) -> Result<PrDeliveryResult, String> {
    let branch = &value.request.branch;
    let destination = format!("{}:refs/heads/{branch}", value.head);
    let output = run_bounded(
        Path::new("git"),
        &["push", "--", &value.push_urls[0], &destination],
        &value.canonical_worktree,
        TIMEOUT,
        OUTPUT_LIMIT,
    )
    .map_err(command_error)?;
    if !output.status_success {
        return Err(format!(
            "원격 브랜치 push에 실패했습니다: {}",
            compact_error(&output.stderr)
        ));
    }
    Ok(result(
        value,
        "push",
        None,
        None,
        "원격 브랜치에 커밋을 push했습니다.",
    ))
}

fn create_or_update(value: &Prepared, update: bool, gh: &Path) -> Result<PrDeliveryResult, String> {
    let remote = git(
        &value.canonical_worktree,
        &[
            "ls-remote",
            "--heads",
            &value.origin_url,
            &format!("refs/heads/{}", value.request.branch),
        ],
    )?;
    let remote_head = remote
        .split_whitespace()
        .next()
        .filter(|sha| valid_sha(sha));
    if remote_head != Some(value.head.as_str()) {
        return Err("원격 브랜치 HEAD가 미리보기의 HEAD와 다릅니다. 먼저 명시적으로 push한 뒤 다시 미리보기하세요.".into());
    }
    let existing = existing_open_pr(value, gh)?;
    match (update, existing.as_ref()) {
        (false, Some(_)) => {
            return Err("같은 브랜치의 열린 PR이 이미 있습니다. 업데이트를 선택하세요.".into())
        }
        (true, None) => return Err("업데이트할 열린 PR이 없습니다. 만들기를 선택하세요.".into()),
        _ => {}
    }
    let body_path = BodyFile {
        path: write_body_file(&value.body)?,
    };
    let result = if update {
        let pr = existing.unwrap();
        let number = pr.number.to_string();
        let args = [
            "pr",
            "edit",
            &number,
            "--repo",
            &value.repository,
            "--title",
            &value.title,
            "--body-file",
            body_path
                .path
                .to_str()
                .ok_or_else(|| "임시 PR 본문 경로가 올바르지 않습니다.".to_string())?,
        ];
        let output = run_bounded(gh, &args, &value.canonical_worktree, TIMEOUT, OUTPUT_LIMIT);
        body_path.cleanup()?;
        let output = output.map_err(command_error)?;
        if !output.status_success {
            return Err(format!(
                "PR 업데이트에 실패했습니다: {}",
                compact_error(&output.stderr)
            ));
        }
        result(
            value,
            "update",
            Some(pr.number),
            Some(pr.url),
            "열린 PR의 제목과 본문을 업데이트했습니다.",
        )
    } else {
        let args = [
            "pr",
            "create",
            "--repo",
            &value.repository,
            "--head",
            &value.request.branch,
            "--base",
            &value.request.base,
            "--title",
            &value.title,
            "--body-file",
            body_path
                .path
                .to_str()
                .ok_or_else(|| "임시 PR 본문 경로가 올바르지 않습니다.".to_string())?,
        ];
        let output = run_bounded(gh, &args, &value.canonical_worktree, TIMEOUT, OUTPUT_LIMIT);
        body_path.cleanup()?;
        let output = output.map_err(command_error)?;
        if !output.status_success {
            return Err(format!(
                "PR 만들기에 실패했습니다: {}",
                compact_error(&output.stderr)
            ));
        }
        let url = output
            .stdout
            .lines()
            .map(str::trim)
            .find(|line| line.starts_with("https://github.com/"))
            .map(str::to_string);
        result(value, "create", None, url, "새 PR을 만들었습니다.")
    };
    Ok(result)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Debug)]
struct RemotePr {
    number: u64,
    url: String,
    state: String,
    head_ref_oid: String,
    head_ref_name: String,
    base_ref_name: String,
    head_repository: Option<RemoteRepo>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Debug)]
struct RemoteRepo {
    name_with_owner: String,
}

fn existing_open_pr(value: &Prepared, gh: &Path) -> Result<Option<RemotePr>, String> {
    let json_fields = "number,url,state,headRefOid,headRefName,baseRefName,headRepository";
    let args = [
        "pr",
        "list",
        "--repo",
        &value.repository,
        "--head",
        &value.request.branch,
        "--state",
        "open",
        "--json",
        json_fields,
        "--limit",
        "2",
    ];
    let output = run_bounded(gh, &args, &value.canonical_worktree, TIMEOUT, OUTPUT_LIMIT)
        .map_err(command_error)?;
    if !output.status_success {
        return Err(format!(
            "열린 PR을 확인하지 못했습니다: {}",
            compact_error(&output.stderr)
        ));
    }
    let prs: Vec<RemotePr> = serde_json::from_str(&output.stdout)
        .map_err(|_| "GitHub PR 응답 형식이 올바르지 않습니다.".to_string())?;
    if prs.len() > 1 {
        return Err("같은 브랜치에 열린 PR이 여러 개입니다.".into());
    }
    let Some(pr) = prs.into_iter().next() else {
        return Ok(None);
    };
    if pr.state != "OPEN"
        || pr.head_ref_oid != value.head
        || pr.head_ref_name != value.request.branch
        || pr.base_ref_name != value.request.base
        || pr
            .head_repository
            .as_ref()
            .map(|repo| repo.name_with_owner.as_str())
            != Some(value.repository.as_str())
        || !pr.url.eq_ignore_ascii_case(&format!(
            "https://github.com/{}/pull/{}",
            value.repository, pr.number
        ))
    {
        return Err(
            "열린 PR이 미리보기의 저장소, 브랜치, HEAD 또는 기준 브랜치와 일치하지 않습니다."
                .into(),
        );
    }
    Ok(Some(pr))
}

fn result(
    value: &Prepared,
    action: &str,
    pr_number: Option<u64>,
    url: Option<String>,
    message: &str,
) -> PrDeliveryResult {
    PrDeliveryResult {
        action: action.into(),
        repository: value.repository.clone(),
        branch: value.request.branch.clone(),
        head: value.head.clone(),
        pr_number,
        url,
        message: message.into(),
    }
}
fn git(worktree: &Path, args: &[&str]) -> Result<String, String> {
    let out = run_bounded(Path::new("git"), args, worktree, TIMEOUT, OUTPUT_LIMIT)
        .map_err(command_error)?;
    if out.status_success {
        Ok(out.stdout)
    } else {
        Err(format!(
            "Git 명령에 실패했습니다: {}",
            compact_error(&out.stderr)
        ))
    }
}
fn command_error(error: super::pr_status::ProcessError) -> String {
    error.to_string()
}
fn gh_bin() -> Result<&'static Path, String> {
    static GH: OnceLock<Option<PathBuf>> = OnceLock::new();
    GH.get_or_init(|| crate::reviewer::which("gh").map(PathBuf::from))
        .as_deref()
        .ok_or_else(|| "GitHub CLI(gh)를 찾을 수 없습니다.".into())
}
fn valid_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn valid_ref(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.starts_with('-')
        || value.contains("..")
        || value.chars().any(|c| {
            c.is_control()
                || c.is_whitespace()
                || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        })
    {
        Err(format!("{label} 이름이 올바르지 않습니다."))
    } else {
        Ok(())
    }
}
fn hash(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}
struct BodyFile {
    path: PathBuf,
}
impl BodyFile {
    fn cleanup(&self) -> Result<(), String> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(
                "PR 동작의 결과를 다시 확인하세요. 임시 본문 파일을 정리하지 못했습니다.".into(),
            ),
        }
    }
}
impl Drop for BodyFile {
    fn drop(&mut self) {
        if self.cleanup().is_err() {
            eprintln!("PR temporary body cleanup failed");
        }
    }
}
fn write_body_file(body: &str) -> Result<PathBuf, String> {
    static COUNTER: OnceLock<Mutex<u64>> = OnceLock::new();
    let mut counter = COUNTER
        .get_or_init(|| Mutex::new(0))
        .lock()
        .map_err(|_| "임시 PR 본문을 만들지 못했습니다.".to_string())?;
    *counter += 1;
    let mut path = std::env::temp_dir();
    path.push(format!(
        "praxis-pr-body-{}-{}-{}",
        std::process::id(),
        now(),
        *counter
    ));
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|_| "임시 PR 본문을 만들지 못했습니다.".to_string())?;
    if file.write_all(body.as_bytes()).is_err() {
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err("임시 PR 본문을 쓰지 못했습니다.".into());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn fixture() -> (PathBuf, PathBuf) {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = crate::testtmp::dir().join(format!("pr-delivery-{id}"));
        let remote = root.join("remote.git");
        let worktree = root.join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        Command::new("git")
            .args(["init", "--bare", remote.to_str().unwrap()])
            .output()
            .unwrap();
        git(&worktree, &["init", "-q", "-b", "main"]);
        git(&worktree, &["config", "user.email", "test@example.com"]);
        git(&worktree, &["config", "user.name", "Test"]);
        std::fs::write(worktree.join("file"), "base").unwrap();
        git(&worktree, &["add", "file"]);
        git(&worktree, &["commit", "-qm", "base"]);
        git(
            &worktree,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/acme/widgets.git",
            ],
        );
        git(&worktree, &["checkout", "-qb", "feature/pr"]);
        std::fs::write(worktree.join("file"), "change").unwrap();
        git(&worktree, &["add", "file"]);
        git(&worktree, &["commit", "-qm", "delivery change"]);
        (root, worktree)
    }
    fn request(worktree: &Path) -> DeliveryRequest {
        DeliveryRequest {
            task_id: 7,
            worktree: worktree.into(),
            branch: "feature/pr".into(),
            base: "main".into(),
            verification_summary: "- test: passed (current)".into(),
        }
    }
    #[cfg(unix)]
    fn stub(root: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = root.join("gh.sh");
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn preview_uses_commits_template_and_editing_makes_a_new_token() {
        let (_root, worktree) = fixture();
        std::fs::create_dir_all(worktree.join(".github")).unwrap();
        std::fs::write(worktree.join(".github/pull_request_template.md"), "## 체크").unwrap();
        let first = preview(
            request(&worktree),
            PrDeliveryInput {
                title: None,
                body: None,
            },
        )
        .unwrap();
        assert_eq!(first.title, "delivery change");
        assert_eq!(first.commits.len(), 1);
        assert!(first.body.contains("## 체크"));
        assert!(first.body.contains("test: passed"));
        let second = preview(
            request(&worktree),
            PrDeliveryInput {
                title: Some("edited".into()),
                body: Some("frozen".into()),
            },
        )
        .unwrap();
        assert_ne!(first.token, second.token);
        assert_eq!(second.body, "frozen");
    }

    #[test]
    fn oversized_template_is_rejected_before_reading_it() {
        let (_root, worktree) = fixture();
        std::fs::create_dir_all(worktree.join(".github")).unwrap();
        std::fs::write(
            worktree.join(".github/pull_request_template.md"),
            vec![b'x'; MAX_BODY_BYTES + 1],
        )
        .unwrap();
        assert!(preview(
            request(&worktree),
            PrDeliveryInput {
                title: None,
                body: None
            }
        )
        .unwrap_err()
        .contains("너무 큽니다"));
    }

    #[cfg(unix)]
    #[test]
    fn outside_template_symlink_is_rejected() {
        use std::os::unix::fs::symlink;
        let (root, worktree) = fixture();
        std::fs::create_dir_all(worktree.join(".github")).unwrap();
        let outside = root.join("outside.md");
        std::fs::write(&outside, "outside").unwrap();
        symlink(&outside, worktree.join(".github/pull_request_template.md")).unwrap();
        assert!(preview(
            request(&worktree),
            PrDeliveryInput {
                title: None,
                body: None
            }
        )
        .unwrap_err()
        .contains("밖"));
    }

    #[test]
    fn stale_head_consumes_token_before_any_push() {
        let (_root, worktree) = fixture();
        let preview = preview(
            request(&worktree),
            PrDeliveryInput {
                title: None,
                body: None,
            },
        )
        .unwrap();
        std::fs::write(worktree.join("late"), "late").unwrap();
        git(&worktree, &["add", "late"]);
        git(&worktree, &["commit", "-qm", "late"]);
        let error = execute(
            7,
            &preview.token,
            PrDeliveryAction {
                action: "push".into(),
            },
        )
        .unwrap_err();
        assert!(error.contains("HEAD"));
        assert!(execute(
            7,
            &preview.token,
            PrDeliveryAction {
                action: "push".into()
            }
        )
        .unwrap_err()
        .contains("이미 사용"));
    }

    #[test]
    fn current_task_identity_and_origin_or_branch_drift_are_rejected() {
        let (_root, worktree) = fixture();
        let next_preview = preview(
            request(&worktree),
            PrDeliveryInput {
                title: None,
                body: None,
            },
        )
        .unwrap();
        assert!(matches_task(&next_preview.token, 8, &worktree, "feature/pr", "main").is_err());
        assert!(matches_task(&next_preview.token, 7, &worktree, "feature/pr", "other").is_err());
        git(
            &worktree,
            &[
                "remote",
                "set-url",
                "origin",
                "https://gitlab.com/acme/widgets.git",
            ],
        );
        assert!(execute(
            7,
            &next_preview.token,
            PrDeliveryAction {
                action: "push".into()
            }
        )
        .unwrap_err()
        .contains("origin"));

        let preview = preview(
            request(&worktree),
            PrDeliveryInput {
                title: None,
                body: None,
            },
        );
        assert!(
            preview.is_err(),
            "a non-GitHub origin must not prepare a token"
        );
    }

    #[cfg(unix)]
    #[test]
    fn fork_or_closed_pr_response_is_refused_before_edit() {
        let (root, worktree) = fixture();
        let canonical_worktree = std::fs::canonicalize(&worktree).unwrap();
        let head = identity(&canonical_worktree, "feature/pr").unwrap();
        let prepared = Prepared {
            request: request(&worktree),
            repository: "acme/widgets".into(),
            origin_url: "https://github.com/acme/widgets.git".into(),
            push_urls: vec!["https://github.com/acme/widgets.git".into()],
            canonical_worktree,
            head: head.clone(),
            base_head: revision(&worktree, "main").unwrap(),
            title: "title".into(),
            body: "body".into(),
            template: None,
            expires_at: now() + 60,
        };
        for response in [
            format!(
                r#"[{{"number":8,"url":"https://github.com/acme/widgets/pull/8","state":"OPEN","headRefOid":"{head}","headRefName":"feature/pr","baseRefName":"main","headRepository":{{"nameWithOwner":"fork/widgets"}}}}]"#
            ),
            format!(
                r#"[{{"number":8,"url":"https://github.com/acme/widgets/pull/8","state":"CLOSED","headRefOid":"{head}","headRefName":"feature/pr","baseRefName":"main","headRepository":{{"nameWithOwner":"acme/widgets"}}}}]"#
            ),
        ] {
            let script = format!(
                "#!/bin/sh\nprintf '%s\\n' '{}'\n",
                response.replace('\'', "'\\''")
            );
            let gh = stub(&root, &script);
            assert!(existing_open_pr(&prepared, &gh)
                .unwrap_err()
                .contains("열린 PR"));
        }
    }
}

#[cfg(test)]
#[path = "pr_delivery_flow_tests.rs"]
mod flow_tests;
