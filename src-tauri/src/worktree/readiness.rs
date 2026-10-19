//! Advisory inspection. Never commits, fetches, runs hooks, or changes the index/ref.
use std::path::Path;
use std::process::Command;

use serde::Serialize;

use super::{approval_merge, Worktree};

#[derive(Clone, Debug, Serialize)]
pub struct ChangedPath {
    pub status: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct Issue {
    pub code: &'static str,
    pub message: String,
    pub location: Option<String>,
    pub paths: Vec<ChangedPath>,
    pub total_paths: usize,
}

#[derive(Debug, Serialize)]
pub struct Readiness {
    pub base: String,
    pub source_sha: String,
    pub target_sha: String,
    pub observed_at: i64,
    pub source_changes: usize,
    pub already_integrated: bool,
    pub direct: bool,
    pub remote_ahead: Option<u64>,
    pub remote_behind: Option<u64>,
    pub issues: Vec<Issue>,
}

pub(super) fn git(path: &Path, args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new("git")
        .current_dir(path)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .output()?;
    if !out.status.success() {
        anyhow::bail!(
            "Git 상태 확인 실패: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8(out.stdout)?)
}

pub(super) fn changes(path: &Path) -> anyhow::Result<Vec<ChangedPath>> {
    let raw = git(
        path,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let mut entries = raw.split('\0').filter(|entry| !entry.is_empty());
    let mut result = Vec::new();
    while let Some(entry) = entries.next() {
        let status = entry
            .get(..2)
            .ok_or_else(|| anyhow::anyhow!("invalid Git status"))?;
        let name = entry
            .get(3..)
            .ok_or_else(|| anyhow::anyhow!("invalid Git path"))?;
        // -z rename/copy records contain the destination followed by the old name.
        if status.contains('R') || status.contains('C') {
            entries
                .next()
                .ok_or_else(|| anyhow::anyhow!("incomplete Git rename"))?;
        }
        if status == "??"
            && crate::project_paths::is_auxiliary_path(name)
        {
            continue;
        }
        result.push(ChangedPath {
            status: status.into(),
            path: name.into(),
        });
    }
    Ok(result)
}

pub(super) fn operation_open(path: &Path) -> anyhow::Result<bool> {
    let dir = git(path, &["rev-parse", "--absolute-git-dir"])?;
    Ok([
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-apply",
        "rebase-merge",
        "sequencer",
    ]
    .iter()
    .any(|name| Path::new(dir.trim()).join(name).exists()))
}

fn issue(
    code: &'static str,
    message: String,
    location: Option<&Path>,
    mut paths: Vec<ChangedPath>,
) -> Issue {
    let total_paths = paths.len();
    paths.truncate(20);
    Issue {
        code,
        message,
        location: location.map(|p| p.to_string_lossy().into_owned()),
        paths,
        total_paths,
    }
}

impl Worktree {
    pub fn approval_readiness(&self) -> anyhow::Result<Readiness> {
        let source_sha = git(&self.path, &["rev-parse", "HEAD"])?.trim().to_owned();
        let source_changes = changes(&self.path)?.len();
        let mut result = Readiness {
            base: self.base.clone(),
            source_sha,
            target_sha: String::new(),
            observed_at: chrono::Utc::now().timestamp(),
            source_changes,
            already_integrated: false,
            direct: self.is_direct(),
            remote_ahead: None,
            remote_behind: None,
            issues: vec![],
        };
        if self.is_direct() {
            return Ok(result);
        }
        self.validate_isolated_approval()?;
        let base = approval_merge::base_ref(self)?;
        result.target_sha = git(&self.repo, &["rev-parse", &base])?.trim().to_owned();
        result.already_integrated =
            source_changes == 0 && self.commit_is_merged(&result.source_sha);
        if operation_open(&self.path)? {
            result.issues.push(issue(
                if self.conflict_session_open() {
                    "source_merge"
                } else {
                    "source_operation"
                },
                "작업 폴더에서 Git 작업이 진행 중입니다".into(),
                Some(&self.path),
                vec![],
            ));
        }
        // An integrated retry goes straight to cleanup; a dirty base is irrelevant there.
        if !result.already_integrated {
            let holders = approval_merge::holders(self, &base)?;
            if let Some(holder) = holders.first() {
                let temporary = approval_merge::temporary_path(self, false)?;
                if approval_merge::same_path(holder, &self.repo)
                    || approval_merge::same_path(holder, &temporary)
                {
                    if approval_merge::same_path(holder, &temporary) {
                        approval_merge::validate_temporary(self, holder)?;
                    }
                    let changed = changes(holder)?;
                    if !changed.is_empty() {
                        result.issues.push(issue(
                            "target_dirty",
                            format!("대상 폴더에 변경 {}건이 있습니다", changed.len()),
                            Some(holder),
                            changed,
                        ));
                    }
                    if operation_open(holder)? {
                        result.issues.push(issue(
                            "target_operation",
                            "대상 폴더에서 Git 작업이 진행 중입니다".into(),
                            Some(holder),
                            vec![],
                        ));
                    }
                } else {
                    // Never inspect a foreign holder: it may be outside Runner's allowed roots.
                    result.issues.push(issue(
                        "target_busy",
                        "대상 브랜치를 다른 워크트리가 사용 중입니다".into(),
                        Some(holder),
                        vec![],
                    ));
                }
            } else {
                let temporary = approval_merge::temporary_path(self, false)?;
                if std::fs::symlink_metadata(&temporary).is_ok() {
                    result.issues.push(issue(
                        "temporary_residue",
                        "승인용 임시 폴더의 잔여물을 확인해야 합니다".into(),
                        Some(&temporary),
                        vec![],
                    ));
                }
            }
            // Only inspect committed input. Pending edits must not be advertised as checked.
            if source_changes == 0 && !operation_open(&self.path)? {
                let conflicts = self.conflicting_paths_against_base()?;
                if !conflicts.is_empty() {
                    let paths = conflicts
                        .into_iter()
                        .map(|path| ChangedPath {
                            status: "UU".into(),
                            path,
                        })
                        .collect();
                    result.issues.push(issue(
                        "merge_conflict",
                        "커밋된 작업과 대상 브랜치가 충돌합니다".into(),
                        Some(&self.path),
                        paths,
                    ));
                }
            }
        }
        let remote = format!("refs/remotes/origin/{}", self.base);
        let remote_exists = Command::new("git")
            .current_dir(&self.repo)
            .args(["show-ref", "--verify", "--quiet", &remote])
            .status()?;
        match remote_exists.code() {
            Some(0) => {
                let counts = git(
                    &self.repo,
                    &[
                        "rev-list",
                        "--left-right",
                        "--count",
                        &format!("{base}...{remote}"),
                    ],
                )?;
                let values: Vec<u64> = counts
                    .split_whitespace()
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
                anyhow::ensure!(values.len() == 2, "invalid ahead/behind response");
                result.remote_ahead = Some(values[0]);
                result.remote_behind = Some(values[1]);
            }
            Some(1) => {}
            _ => anyhow::bail!("원격 추적 브랜치를 확인할 수 없습니다"),
        }
        if git(&self.path, &["rev-parse", "HEAD"])?.trim() != result.source_sha
            || git(&self.repo, &["rev-parse", &base])?.trim() != result.target_sha
        {
            anyhow::bail!("확인 중 브랜치가 변경됐습니다. 다시 확인하세요.");
        }
        Ok(result)
    }
}
