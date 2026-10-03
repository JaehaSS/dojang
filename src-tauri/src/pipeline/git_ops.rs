//! 파이프라인용 git 조작 — 모두 `git -C <dir>` 의미로 `current_dir`에서 실행하는 동기 함수다.
//! 티켓 worktree의 변경 수집, 통합 worktree의 merge·충돌 마무리·되돌림을 맡는다.
//! 훅(.githooks 등)이 파이프라인 커밋을 막지 않도록 커밋에는 `--no-verify`를 쓴다.

use anyhow::{anyhow, bail, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// diff 본문 상한. 넘으면 끝에 잘림 안내를 붙인다.
pub const DIFF_MAX_BYTES: usize = 400 * 1024;
/// 충돌 마커 검사 시 읽을 파일 크기 상한.
const MARKER_SCAN_MAX_BYTES: u64 = 4 * 1024 * 1024;

struct GitOut {
    ok: bool,
    stdout: Vec<u8>,
    stderr: String,
}

fn git_raw(dir: &Path, args: &[&str], envs: &[(&str, &Path)]) -> Result<GitOut> {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir).args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().map_err(|e| anyhow!("git 실행 실패: {e}"))?;
    Ok(GitOut {
        ok: out.status.success(),
        stdout: out.stdout,
        stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
    })
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = git_raw(dir, args, &[])?;
    if !out.ok {
        bail!("git {:?} 실패: {}", args, out.stderr);
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn nul_list(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// 저장소에 커밋 identity가 없으면 임시 identity를 `-c`로 넘긴다.
fn identity_args(dir: &Path) -> Vec<&'static str> {
    let has = |key: &str| {
        git_raw(dir, &["config", "--get", key], &[])
            .map(|o| o.ok && !o.stdout.iter().all(|b| b.is_ascii_whitespace()))
            .unwrap_or(false)
    };
    if has("user.name") && has("user.email") {
        Vec::new()
    } else {
        vec!["-c", "user.name=Dojang", "-c", "user.email=dojang@local"]
    }
}

fn git_with_identity(dir: &Path, args: &[&str]) -> Result<GitOut> {
    let ident = identity_args(dir);
    let mut full: Vec<&str> = ident;
    full.extend_from_slice(args);
    git_raw(dir, &full, &[])
}

pub fn head(dir: &Path) -> Result<String> {
    Ok(git(dir, &["rev-parse", "HEAD"])?.trim().to_string())
}

fn untracked(dir: &Path) -> Result<Vec<String>> {
    let out = git_raw(dir, &["ls-files", "-z", "--others", "--exclude-standard"], &[])?;
    if !out.ok {
        bail!("git ls-files 실패: {}", out.stderr);
    }
    Ok(nul_list(&out.stdout))
}

/// `git status --porcelain` 원문(추적·미추적 변경). 읽기 전용 단계 전후를 비교하는 데 쓴다.
pub fn status_porcelain(dir: &Path) -> Result<String> {
    git(dir, &["status", "--porcelain"])
}

/// fork point 이후 커밋된 변경 + staged + unstaged + untracked(ignored 제외). 중복 제거·정렬된 저장소 상대 경로.
/// rename은 이전·새 경로를 모두 포함한다.
pub fn changed_paths(worktree: &Path, fork_point: &str) -> Result<Vec<String>> {
    let out = git_raw(
        worktree,
        &["diff", "-z", "--name-only", "--no-renames", fork_point],
        &[],
    )?;
    if !out.ok {
        bail!("git diff 실패: {}", out.stderr);
    }
    let mut set: BTreeSet<String> = nul_list(&out.stdout).into_iter().collect();
    set.extend(untracked(worktree)?);
    Ok(set.into_iter().collect())
}

/// fork point..HEAD 사이에 커밋된 변경 경로만(작업 트리·index·untracked 제외). 정렬된 저장소 상대 경로.
pub fn committed_paths(worktree: &Path, fork_point: &str) -> Result<Vec<String>> {
    let out = git_raw(worktree, &["diff", "-z", "--name-only", "--no-renames", fork_point, "HEAD"], &[])?;
    if !out.ok {
        bail!("git diff 실패: {}", out.stderr);
    }
    let set: BTreeSet<String> = nul_list(&out.stdout).into_iter().collect();
    Ok(set.into_iter().collect())
}

/// 실제 index를 건드리지 않고 임시 index 복사본에서 `add -A` 한 뒤 fork point와 비교한 unified diff.
/// untracked 파일은 추가로 나타난다. 상한을 넘으면 잘라 안내를 붙인다.
pub fn diff_text(worktree: &Path, fork_point: &str) -> Result<String> {
    let real_index = {
        let p = git(worktree, &["rev-parse", "--git-path", "index"])?;
        let p = PathBuf::from(p.trim());
        if p.is_absolute() { p } else { worktree.join(p) }
    };
    let tmp = std::env::temp_dir().join(format!(
        "dojang-pipe-index-{}-{:x}",
        std::process::id(),
        crate::pipeline::review_run::random_u64()
    ));
    if real_index.exists() {
        std::fs::copy(&real_index, &tmp).map_err(|e| anyhow!("index 복사 실패: {e}"))?;
    }
    let result = (|| -> Result<String> {
        let env = [("GIT_INDEX_FILE", tmp.as_path())];
        let add = git_raw(worktree, &["add", "-A"], &env)?;
        if !add.ok {
            bail!("git add -A 실패: {}", add.stderr);
        }
        let d = git_raw(
            worktree,
            &["diff", "--cached", "--no-color", "--no-renames", fork_point],
            &env,
        )?;
        if !d.ok {
            bail!("git diff --cached 실패: {}", d.stderr);
        }
        Ok(String::from_utf8_lossy(&d.stdout).into_owned())
    })();
    let _ = std::fs::remove_file(&tmp);
    let text = result?;
    if text.len() <= DIFF_MAX_BYTES {
        return Ok(text);
    }
    let mut end = DIFF_MAX_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Ok(format!(
        "{}\n\n[... diff가 {} KB 상한으로 잘렸습니다 (전체 {} KB) ...]\n",
        &text[..end],
        DIFF_MAX_BYTES / 1024,
        text.len() / 1024
    ))
}

/// `git add -A` 후 변경이 있으면 커밋하고 새 rev를 돌려준다. 변경이 없으면 `None`.
pub fn commit_all(worktree: &Path, message: &str) -> Result<Option<String>> {
    let add = git_raw(worktree, &["add", "-A"], &[])?;
    if !add.ok {
        bail!("git add -A 실패: {}", add.stderr);
    }
    // exit 0 = 변경 없음, 1 = 변경 있음
    let quiet = git_raw(worktree, &["diff", "--cached", "--quiet"], &[])?;
    if quiet.ok {
        return Ok(None);
    }
    let c = git_with_identity(worktree, &["commit", "--no-verify", "-m", message])?;
    if !c.ok {
        bail!("git commit 실패: {}", c.stderr);
    }
    Ok(Some(head(worktree)?))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    Merged { rev: String },
    Conflict { paths: Vec<String> },
    NothingToMerge,
}

fn unmerged_paths(dir: &Path) -> Result<Vec<String>> {
    let out = git_raw(dir, &["diff", "-z", "--name-only", "--diff-filter=U"], &[])?;
    if !out.ok {
        bail!("git diff --diff-filter=U 실패: {}", out.stderr);
    }
    Ok(nul_list(&out.stdout))
}

fn merge_in_progress(dir: &Path) -> bool {
    git_raw(dir, &["rev-parse", "-q", "--verify", "MERGE_HEAD"], &[])
        .map(|o| o.ok)
        .unwrap_or(false)
}

/// `git merge --no-ff` 로 `branch`를 통합한다. 충돌이면 merge를 진행 중 상태로 두고 충돌 경로를 돌려준다.
pub fn merge_branch(integration: &Path, branch: &str, message: &str) -> Result<MergeOutcome> {
    let anc = git_raw(integration, &["merge-base", "--is-ancestor", branch, "HEAD"], &[])?;
    if anc.ok {
        return Ok(MergeOutcome::NothingToMerge);
    }
    let m = git_with_identity(
        integration,
        &["merge", "--no-ff", "--no-edit", "--no-verify", "-m", message, branch],
    )?;
    if m.ok {
        return Ok(MergeOutcome::Merged { rev: head(integration)? });
    }
    let paths = unmerged_paths(integration)?;
    if paths.is_empty() {
        let _ = git_raw(integration, &["merge", "--abort"], &[]);
        bail!("git merge 실패: {}", m.stderr);
    }
    Ok(MergeOutcome::Conflict { paths })
}

fn has_conflict_markers(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() && m.len() <= MARKER_SCAN_MAX_BYTES => {}
        _ => return false,
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    text.lines().any(|l| {
        l.starts_with("<<<<<<< ")
            || l == "<<<<<<<"
            || l.starts_with(">>>>>>> ")
            || l == ">>>>>>>"
    })
}

/// 충돌 해소 뒤 마무리: 충돌 마커가 남은 파일이 없는지 확인하고 `add -A` + `commit --no-edit`.
/// 마커 검사는 HEAD 대비 변경 파일 전체(충돌 경로 포함)와 untracked 파일에 한다.
pub fn finish_merge_after_resolution(integration: &Path) -> Result<String> {
    if !merge_in_progress(integration) {
        bail!("진행 중인 merge가 없습니다");
    }
    let mut files: BTreeSet<String> = unmerged_paths(integration)?.into_iter().collect();
    let d = git_raw(integration, &["diff", "-z", "--name-only", "--no-renames", "HEAD"], &[])?;
    if d.ok {
        files.extend(nul_list(&d.stdout));
    }
    files.extend(untracked(integration)?);
    let bad: Vec<&String> = files
        .iter()
        .filter(|f| has_conflict_markers(&integration.join(f)))
        .collect();
    if !bad.is_empty() {
        bail!(
            "충돌 마커가 남아 있습니다: {}",
            bad.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        );
    }
    let add = git_raw(integration, &["add", "-A"], &[])?;
    if !add.ok {
        bail!("git add -A 실패: {}", add.stderr);
    }
    let left = unmerged_paths(integration)?;
    if !left.is_empty() {
        bail!("해소되지 않은 경로가 있습니다: {}", left.join(", "));
    }
    let c = git_with_identity(integration, &["commit", "--no-edit", "--no-verify"])?;
    if !c.ok {
        bail!("merge 커밋 실패: {}", c.stderr);
    }
    head(integration)
}

/// 진행 중인 merge를 중단하고 `rev`로 hard reset, 추적되지 않는 파일을 정리한다(ignored는 보존).
pub fn reset_to(integration: &Path, rev: &str) -> Result<()> {
    if merge_in_progress(integration) {
        let _ = git_raw(integration, &["merge", "--abort"], &[]);
    }
    git(integration, &["reset", "--hard", rev])?;
    git(integration, &["clean", "-fd"])?;
    Ok(())
}

#[cfg(test)]
#[path = "git_ops_tests.rs"]
mod tests;
