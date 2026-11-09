//! Read-only pull request status lookup for one checked-out worktree.

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const QUERY_TIMEOUT: Duration = Duration::from_secs(15);
const OUTPUT_LIMIT: usize = 512 * 1024;
const PR_LIST_LIMIT: usize = 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrStatusSnapshot {
    pub status: String,
    pub message: Option<String>,
    pub repository: Option<String>,
    pub branch: String,
    pub local_head: Option<String>,
    pub observed_at: i64,
    pub pr: Option<PrDetails>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrDetails {
    pub number: u64,
    pub url: String,
    pub title: String,
    pub state: String,
    pub draft: bool,
    pub head_sha: String,
    pub head_branch: String,
    pub base_branch: String,
    pub review_decision: String,
    pub review_requests: Vec<String>,
    pub checks: Vec<PrCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrCheck {
    pub name: String,
    pub status: String,
    pub description: String,
    pub details_url: Option<String>,
}

/// Looks up exactly `branch` in exactly the supplied worktree.  Failures are represented in the
/// snapshot so that a remote outage never prevents the local review screen from loading.
pub fn snapshot(worktree: &Path, branch: &str) -> PrStatusSnapshot {
    let Some(gh_bin) = crate::reviewer::which("gh") else {
        return unavailable(branch, None, None, "GitHub CLI(gh)를 찾을 수 없습니다.");
    };
    snapshot_with_config(
        worktree,
        branch,
        &LookupConfig {
            gh_bin: Path::new(&gh_bin),
            git_bin: Path::new("git"),
            timeout: QUERY_TIMEOUT,
            output_limit: OUTPUT_LIMIT,
        },
    )
}

#[derive(Clone, Copy)]
struct LookupConfig<'a> {
    gh_bin: &'a Path,
    git_bin: &'a Path,
    timeout: Duration,
    output_limit: usize,
}

#[cfg(test)]
pub(crate) fn snapshot_with_bins(
    worktree: &Path,
    branch: &str,
    gh_bin: &Path,
    timeout: Duration,
    output_limit: usize,
) -> PrStatusSnapshot {
    snapshot_with_config(
        worktree,
        branch,
        &LookupConfig {
            gh_bin,
            git_bin: Path::new("git"),
            timeout,
            output_limit,
        },
    )
}

fn snapshot_with_config(
    worktree: &Path,
    branch: &str,
    config: &LookupConfig<'_>,
) -> PrStatusSnapshot {
    let observed_at = observed_at();
    let worktree = match std::fs::canonicalize(worktree) {
        Ok(path) => path,
        Err(_) => {
            return unavailable_at(
                branch,
                None,
                None,
                observed_at,
                "작업 디렉터리를 확인할 수 없습니다.",
            )
        }
    };
    if branch.trim().is_empty() {
        return unavailable_at(
            branch,
            None,
            None,
            observed_at,
            "작업 브랜치가 비어 있습니다.",
        );
    }

    let repository = match git_output(config, &worktree, &["remote", "get-url", "origin"]) {
        Ok(url) => match parse_public_github_origin(&url) {
            Some(repository) => repository,
            None => {
                return unsupported_at(
                    branch,
                    observed_at,
                    "origin이 github.com 저장소를 가리키지 않습니다.",
                )
            }
        },
        Err(error) => {
            return unavailable_at(
                branch,
                None,
                None,
                observed_at,
                &git_message("origin을 읽지 못했습니다", error),
            )
        }
    };

    let before = match checkout_identity(config, &worktree, branch) {
        Ok(identity) => identity,
        Err(error) => return unavailable_at(branch, Some(repository), None, observed_at, &error),
    };

    let args = [
        "pr",
        "list",
        "--state",
        "all",
        "--head",
        branch,
        "--repo",
        &format!("github.com/{repository}"),
        "--json",
        "number,url,title,state,isDraft,headRefOid,headRefName,baseRefName,reviewDecision,reviewRequests,statusCheckRollup,updatedAt,headRepositoryOwner,headRepository",
        "--limit",
        &PR_LIST_LIMIT.to_string(),
    ];
    let listed = run_bounded(
        config.gh_bin,
        &args,
        &worktree,
        config.timeout,
        config.output_limit,
    );
    let after = checkout_identity(config, &worktree, branch);
    let after_head = after.as_ref().ok().map(|identity| identity.head.clone());
    if let Err(error) = after {
        return unavailable_at(branch, Some(repository), after_head, observed_at, &error);
    }
    if before.head != after_head.as_deref().unwrap_or_default() {
        return unavailable_at(
            branch,
            Some(repository),
            after_head,
            observed_at,
            "PR 조회 중 작업 디렉터리의 HEAD가 변경되었습니다.",
        );
    }
    let repository_after = git_output(config, &worktree, &["remote", "get-url", "origin"])
        .ok()
        .and_then(|url| parse_public_github_origin(&url));
    if repository_after.as_deref() != Some(repository.as_str()) {
        return unavailable_at(
            branch,
            Some(repository),
            after_head,
            observed_at,
            "PR 조회 중 origin 저장소가 변경되었습니다.",
        );
    }

    let output = match listed {
        Ok(output) if output.status_success => output.stdout,
        Ok(output) => {
            let detail = compact_error(&output.stderr);
            let message = if looks_unauthenticated(&detail) {
                "GitHub CLI 인증이 필요합니다.".to_string()
            } else if detail.is_empty() {
                "GitHub PR 조회 명령이 실패했습니다.".to_string()
            } else {
                format!("GitHub PR 조회 실패: {detail}")
            };
            return unavailable_at(
                branch,
                Some(repository),
                Some(before.head),
                observed_at,
                &message,
            );
        }
        Err(error) => {
            return unavailable_at(
                branch,
                Some(repository),
                Some(before.head),
                observed_at,
                &format!("GitHub PR 조회를 완료하지 못했습니다: {error}"),
            )
        }
    };

    let prs: Vec<GhPr> = match serde_json::from_str(&output) {
        Ok(prs) => prs,
        Err(error) => {
            return unavailable_at(
                branch,
                Some(repository),
                Some(before.head),
                observed_at,
                &format!("GitHub PR 응답을 해석하지 못했습니다: {error}"),
            )
        }
    };
    if prs.is_empty() {
        return PrStatusSnapshot {
            status: "no_pr".into(),
            message: None,
            repository: Some(repository),
            branch: branch.into(),
            local_head: Some(before.head),
            observed_at,
            pr: None,
        };
    }

    // A bounded list cannot prove which PR to select when additional history may be omitted.
    if prs.len() >= PR_LIST_LIMIT {
        return unavailable_at(
            branch,
            Some(repository),
            Some(before.head),
            observed_at,
            "PR 목록이 조회 상한에 도달했습니다. GitHub에서 브랜치의 PR을 확인하세요.",
        );
    }
    let selected = match select_pr(prs, &repository, branch) {
        Ok(pr) => pr,
        Err(error) => {
            return unavailable_at(
                branch,
                Some(repository),
                Some(before.head),
                observed_at,
                &error,
            )
        }
    };
    let details = match to_details(selected, &repository) {
        Ok(details) => details,
        Err(error) => {
            return unavailable_at(
                branch,
                Some(repository),
                Some(before.head),
                observed_at,
                &error,
            )
        }
    };
    PrStatusSnapshot {
        status: "ready".into(),
        message: None,
        repository: Some(repository),
        branch: branch.into(),
        local_head: Some(before.head),
        observed_at,
        pr: Some(details),
    }
}

#[derive(Debug)]
struct CheckoutIdentity {
    head: String,
}

fn checkout_identity(
    config: &LookupConfig<'_>,
    worktree: &Path,
    branch: &str,
) -> Result<CheckoutIdentity, String> {
    let top_level = git_output(config, worktree, &["rev-parse", "--show-toplevel"])
        .map_err(|error| git_message("작업 디렉터리의 checkout을 읽지 못했습니다", error))?;
    let actual = std::fs::canonicalize(top_level.trim())
        .map_err(|_| "작업 디렉터리의 checkout 경로를 확인할 수 없습니다.".to_string())?;
    if actual != worktree {
        return Err("요청한 작업 디렉터리가 실제 Git checkout과 일치하지 않습니다.".into());
    }
    let current_branch = git_output(
        config,
        worktree,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
    )
    .map_err(|_| "작업 디렉터리가 요청한 브랜치를 checkout하지 않았습니다.".to_string())?;
    if current_branch.trim() != branch {
        return Err("작업 디렉터리가 요청한 브랜치를 checkout하지 않았습니다.".into());
    }
    let head = git_output(config, worktree, &["rev-parse", "HEAD"])
        .map_err(|error| git_message("작업 디렉터리의 HEAD를 읽지 못했습니다", error))?;
    let head = head.trim();
    if !valid_sha(head) {
        return Err("작업 디렉터리의 HEAD 형식이 올바르지 않습니다.".into());
    }
    Ok(CheckoutIdentity { head: head.into() })
}

fn git_output(
    config: &LookupConfig<'_>,
    worktree: &Path,
    args: &[&str],
) -> Result<String, ProcessError> {
    let output = run_bounded(
        config.git_bin,
        args,
        worktree,
        config.timeout,
        config.output_limit,
    )?;
    if output.status_success {
        Ok(output.stdout)
    } else {
        Err(ProcessError::Failed(compact_error(&output.stderr)))
    }
}

fn git_message(action: &str, error: ProcessError) -> String {
    match error {
        ProcessError::Failed(detail) if !detail.is_empty() => format!("{action}: {detail}"),
        other => format!("{action}: {other}"),
    }
}

fn observed_at() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn unavailable(
    branch: &str,
    repository: Option<String>,
    local_head: Option<String>,
    message: &str,
) -> PrStatusSnapshot {
    unavailable_at(branch, repository, local_head, observed_at(), message)
}

fn unavailable_at(
    branch: &str,
    repository: Option<String>,
    local_head: Option<String>,
    observed_at: i64,
    message: &str,
) -> PrStatusSnapshot {
    PrStatusSnapshot {
        status: "unavailable".into(),
        message: Some(message.into()),
        repository,
        branch: branch.into(),
        local_head,
        observed_at,
        pr: None,
    }
}

fn unsupported_at(branch: &str, observed_at: i64, message: &str) -> PrStatusSnapshot {
    PrStatusSnapshot {
        status: "unsupported".into(),
        message: Some(message.into()),
        repository: None,
        branch: branch.into(),
        local_head: None,
        observed_at,
        pr: None,
    }
}

pub(super) fn parse_public_github_origin(url: &str) -> Option<String> {
    let url = url.trim();
    let path = if let Some(path) = strip_prefix_ignore_ascii_case(url, "git@github.com:") {
        path
    } else if let Some(path) = strip_prefix_ignore_ascii_case(url, "ssh://git@github.com/") {
        path
    } else if let Some(path) = strip_prefix_ignore_ascii_case(url, "https://github.com/") {
        path
    } else {
        return None;
    };
    let path = strip_suffix_ignore_ascii_case(path, ".git").unwrap_or(path);
    let mut pieces = path.split('/');
    let owner = pieces.next()?;
    let name = pieces.next()?;
    if pieces.next().is_some() || !valid_repo_segment(owner) || !valid_repo_segment(name) {
        return None;
    }
    Some(format!("{owner}/{name}"))
}

fn strip_prefix_ignore_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|start| start.eq_ignore_ascii_case(prefix))
        .map(|_| &value[prefix.len()..])
}

fn strip_suffix_ignore_ascii_case<'a>(value: &'a str, suffix: &str) -> Option<&'a str> {
    value
        .get(value.len().checked_sub(suffix.len())?..)
        .filter(|end| end.eq_ignore_ascii_case(suffix))
        .map(|_| &value[..value.len() - suffix.len()])
}

fn valid_repo_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPr {
    number: u64,
    url: String,
    title: String,
    state: String,
    is_draft: bool,
    head_ref_oid: String,
    head_ref_name: String,
    base_ref_name: String,
    review_decision: Option<String>,
    review_requests: Vec<GhReviewRequest>,
    status_check_rollup: Value,
    updated_at: String,
    head_repository_owner: Option<GhOwner>,
    head_repository: Option<GhRepository>,
}

#[derive(Debug, Deserialize)]
struct GhOwner {
    login: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhRepository {
    name_with_owner: String,
}

#[derive(Debug, Deserialize)]
struct GhReviewRequest {
    #[serde(default)]
    login: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

fn select_pr(prs: Vec<GhPr>, repository: &str, branch: &str) -> Result<GhPr, String> {
    if prs.iter().any(|pr| !is_exact_pr(pr, repository, branch)) {
        return Err("GitHub PR 응답의 저장소 또는 head 브랜치가 요청과 일치하지 않습니다.".into());
    }
    if prs
        .iter()
        .any(|pr| DateTime::parse_from_rfc3339(&pr.updated_at).is_err())
    {
        return Err("GitHub PR 응답의 갱신 시각 형식이 올바르지 않습니다.".into());
    }
    let (mut open, mut completed): (Vec<GhPr>, Vec<GhPr>) =
        prs.into_iter().partition(|pr| pr.state == "OPEN");
    if open.len() == 1 {
        return Ok(open.remove(0));
    }
    if open.len() > 1 {
        return Err("같은 브랜치에 열린 PR이 여러 개여서 선택할 수 없습니다.".into());
    }
    completed.retain(|pr| matches!(pr.state.as_str(), "CLOSED" | "MERGED"));
    if completed.is_empty() {
        return Err("GitHub PR 상태가 알려지지 않아 선택할 수 없습니다.".into());
    }
    completed.sort_by_key(|pr| std::cmp::Reverse(updated_at(pr)));
    if completed.len() > 1 && updated_at(&completed[0]) == updated_at(&completed[1]) {
        return Err("최근에 갱신된 종료 PR을 하나로 선택할 수 없습니다.".into());
    }
    Ok(completed.remove(0))
}

fn updated_at(pr: &GhPr) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(&pr.updated_at).expect("validated before ordering")
}

fn is_exact_pr(pr: &GhPr, repository: &str, branch: &str) -> bool {
    let owner_matches = pr.head_repository_owner.as_ref().is_some_and(|owner| {
        repository
            .split('/')
            .next()
            .is_some_and(|expected| owner.login.eq_ignore_ascii_case(expected))
    });
    let repository_matches = pr.head_repository.as_ref().is_some_and(|head_repository| {
        head_repository
            .name_with_owner
            .eq_ignore_ascii_case(repository)
    });
    owner_matches
        && repository_matches
        && pr.number > 0
        && valid_sha(&pr.head_ref_oid)
        && pr.head_ref_name == branch
        && exact_pr_url(&pr.url, repository, pr.number)
}

fn exact_pr_url(url: &str, repository: &str, number: u64) -> bool {
    url.eq_ignore_ascii_case(&format!("https://github.com/{repository}/pull/{number}"))
}

fn to_details(pr: GhPr, repository: &str) -> Result<PrDetails, String> {
    let state = match pr.state.as_str() {
        "OPEN" => "open",
        "CLOSED" => "closed",
        "MERGED" => "merged",
        _ => return Err("GitHub PR 상태가 알려지지 않았습니다.".into()),
    };
    let review_requests = pr
        .review_requests
        .into_iter()
        .map(|request| request.login.or(request.name))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| "GitHub 검토 요청 형식이 올바르지 않습니다.".to_string())?;
    let checks = normalize_checks(pr.status_check_rollup)?;
    if !exact_pr_url(&pr.url, repository, pr.number) {
        return Err("GitHub PR URL이 요청한 저장소와 일치하지 않습니다.".into());
    }
    Ok(PrDetails {
        number: pr.number,
        url: pr.url,
        title: pr.title,
        state: state.into(),
        draft: pr.is_draft,
        head_sha: pr.head_ref_oid,
        head_branch: pr.head_ref_name,
        base_branch: pr.base_ref_name,
        review_decision: pr.review_decision.unwrap_or_default(),
        review_requests,
        checks,
    })
}

fn normalize_checks(rollup: Value) -> Result<Vec<PrCheck>, String> {
    match rollup {
        Value::Null => Ok(Vec::new()),
        Value::Array(checks) => checks.into_iter().map(normalize_check).collect(),
        _ => Err("GitHub 검사 목록 형식이 올바르지 않습니다.".into()),
    }
}

fn normalize_check(value: Value) -> Result<PrCheck, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "GitHub 검사 형식이 올바르지 않습니다.".to_string())?;
    let kind = required_string(object, "__typename")?;
    match kind {
        "CheckRun" => {
            let name = required_string(object, "name")?.into();
            let status = optional_string(object, "status")?;
            let conclusion = optional_string(object, "conclusion")?;
            let normalized = normalize_check_run(
                status.as_deref(),
                conclusion.as_deref().filter(|value| !value.is_empty()),
            );
            Ok(PrCheck {
                name,
                status: normalized.into(),
                description: optional_string(object, "description")?.unwrap_or_default(),
                details_url: safe_url(optional_string(object, "detailsUrl")?),
            })
        }
        "StatusContext" => {
            let name = required_string(object, "context")?.into();
            let state = required_string(object, "state")?;
            Ok(PrCheck {
                name,
                status: normalize_status_context(state).into(),
                description: optional_string(object, "description")?.unwrap_or_default(),
                details_url: safe_url(optional_string(object, "targetUrl")?),
            })
        }
        _ => Err("GitHub 검사 종류가 알려지지 않았습니다.".into()),
    }
}

fn required_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a str, String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("GitHub 검사 {field} 필드가 올바르지 않습니다."))
}

fn optional_string(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<String>, String> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("GitHub 검사 {field} 필드가 올바르지 않습니다.")),
    }
}

fn normalize_check_run(status: Option<&str>, conclusion: Option<&str>) -> &'static str {
    match conclusion {
        Some("SUCCESS") => "pass",
        Some("FAILURE" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE" | "STALE") => "fail",
        Some("CANCELLED") => "cancelled",
        Some("SKIPPED" | "NEUTRAL") => "skipped",
        Some(_) => "unknown",
        None => match status {
            Some("QUEUED" | "IN_PROGRESS" | "PENDING" | "REQUESTED" | "WAITING") => "pending",
            _ => "unknown",
        },
    }
}

fn normalize_status_context(state: &str) -> &'static str {
    match state {
        "SUCCESS" => "pass",
        "FAILURE" | "ERROR" => "fail",
        "PENDING" | "EXPECTED" => "pending",
        _ => "unknown",
    }
}

fn safe_url(url: Option<String>) -> Option<String> {
    url.filter(|url| {
        let lower = url.to_ascii_lowercase();
        (lower.starts_with("https://") || lower.starts_with("http://"))
            && !url.chars().any(char::is_whitespace)
    })
}

fn valid_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn looks_unauthenticated(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    [
        "gh auth login",
        "not logged into",
        "authentication required",
        "no oauth token",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub(super) struct ProcessOutput {
    pub(super) status_success: bool,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

#[derive(Debug)]
pub(super) enum ProcessError {
    Spawn(String),
    Timeout,
    OutputLimit,
    Failed(String),
}

impl std::fmt::Display for ProcessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(error) => write!(formatter, "실행하지 못했습니다: {error}"),
            Self::Timeout => write!(formatter, "시간 제한을 초과했습니다"),
            Self::OutputLimit => write!(formatter, "출력 제한을 초과했습니다"),
            Self::Failed(error) => write!(formatter, "명령이 실패했습니다: {error}"),
        }
    }
}

pub(super) fn run_bounded(
    program: &Path,
    args: &[&str],
    current_dir: &Path,
    timeout: Duration,
    output_limit: usize,
) -> Result<ProcessOutput, ProcessError> {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_PAGER", "cat")
        .env("NO_COLOR", "1");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|error| ProcessError::Spawn(error.to_string()))?;
    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let capped = Arc::new(AtomicBool::new(false));
    let out_flag = Arc::clone(&capped);
    let err_flag = Arc::clone(&capped);
    let (sender, receiver) = mpsc::channel();
    let stdout_sender = sender.clone();
    let stdout_reader = thread::spawn(move || {
        let _ = stdout_sender.send((true, read_capped(stdout, output_limit, out_flag)));
    });
    let stderr_reader = thread::spawn(move || {
        let _ = sender.send((false, read_capped(stderr, output_limit, err_flag)));
    });
    let started = Instant::now();
    let mut failure = None;
    let mut status = None;
    let mut terminated = false;
    let mut stdout_output = None;
    let mut stderr_output = None;
    while status.is_none() || stdout_output.is_none() || stderr_output.is_none() {
        if !terminated {
            if capped.load(Ordering::Acquire) {
                failure = Some(ProcessError::OutputLimit);
                terminate_and_reap(&mut child);
                terminated = true;
            } else if started.elapsed() >= timeout {
                failure = Some(ProcessError::Timeout);
                terminate_and_reap(&mut child);
                terminated = true;
            } else if status.is_none() {
                match child.try_wait() {
                    Ok(Some(done)) => status = Some(done),
                    Ok(None) => {}
                    Err(error) => {
                        failure = Some(ProcessError::Spawn(error.to_string()));
                        terminate_and_reap(&mut child);
                        terminated = true;
                    }
                }
            }
        }
        match receiver.recv_timeout(Duration::from_millis(10)) {
            Ok((is_stdout, output)) if is_stdout => stdout_output = Some(output),
            Ok((_, output)) => stderr_output = Some(output),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) if terminated => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => thread::sleep(Duration::from_millis(10)),
        }
    }
    let _ = stdout_reader.join();
    let _ = stderr_reader.join();
    let stdout = stdout_output.unwrap_or_default();
    let stderr = stderr_output.unwrap_or_default();
    if let Some(error) = failure {
        return Err(error);
    }
    if capped.load(Ordering::Acquire) {
        return Err(ProcessError::OutputLimit);
    }
    let Some(status) = status else {
        return Err(ProcessError::Spawn(
            "프로세스 종료 상태를 읽지 못했습니다".into(),
        ));
    };
    Ok(ProcessOutput {
        status_success: status.success(),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

fn read_capped(mut reader: impl Read, limit: usize, capped: Arc<AtomicBool>) -> Vec<u8> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                let remaining = limit.saturating_sub(output.len());
                if count > remaining {
                    output.extend_from_slice(&buffer[..remaining]);
                    capped.store(true, Ordering::Release);
                    break;
                }
                output.extend_from_slice(&buffer[..count]);
            }
        }
    }
    output
}

fn terminate_and_reap(child: &mut std::process::Child) {
    crate::verify::kill_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

pub(super) fn compact_error(stderr: &str) -> String {
    stderr.trim().chars().take(500).collect()
}

#[cfg(test)]
pub(crate) fn normalize_check_for_test(value: Value) -> Result<PrCheck, String> {
    normalize_check(value)
}
