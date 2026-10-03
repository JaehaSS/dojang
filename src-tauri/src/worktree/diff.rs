use super::diff_limits::{marker, omission, LARGE, MAX_PATCH_BYTES};
use std::collections::HashSet;
use std::io::Read;
use std::process::{Command, Stdio};

use super::diff_stats::format_diff_stat;
use super::{run_git, BaselineStatus, DiffRange, FileDiff, Worktree};

struct ChangedPath {
    status: String,
    paths: Vec<String>,
}

impl Worktree {
    /// task 분기점부터 현재 파일까지의 변경 요약. staged·task commit·미추적 파일을 모두 포함한다.
    pub fn diff_stat(&self) -> anyhow::Result<String> {
        let files = self.diff_detailed()?;
        Ok(format_diff_stat(&files))
    }

    /// task 분기점부터 현재 파일까지의 단일 unified diff. 실제 Git index는 변경하지 않는다.
    pub fn diff_unified(&self, context: u32) -> anyhow::Result<String> {
        self.diff_unified_range(context, DiffRange::Session)
    }

    /// 범위를 골라 뜨는 unified diff.
    pub fn diff_unified_range(&self, context: u32, range: DiffRange) -> anyhow::Result<String> {
        let files = self.diff_detailed_with_context(context, range)?;
        Ok(files
            .into_iter()
            .map(|file| file.patch)
            .filter(|patch| !patch.is_empty())
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// task 분기점부터 현재 파일까지의 파일별 diff. staged·commit·미추적 파일을 포함한다.
    pub fn diff_detailed(&self) -> anyhow::Result<Vec<FileDiff>> {
        self.diff_detailed_with_context(3, DiffRange::Session)
    }

    /// 범위를 골라 뜨는 파일별 diff.
    pub fn diff_detailed_range(&self, range: DiffRange) -> anyhow::Result<Vec<FileDiff>> {
        self.diff_detailed_with_context(3, range)
    }

    /// 승인 정책용 변경 경로. rename/copy 양쪽과 staged·commit·미추적 파일을 모두 반환한다.
    pub fn changed_paths(&self) -> anyhow::Result<Vec<String>> {
        let base = self.diff_base()?;
        let mut paths = Vec::new();
        for path in tracked_changes(self, &base)?
            .into_iter()
            .flat_map(|change| change.paths)
            .chain(untracked_paths(self)?)
        {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        Ok(paths)
    }

    fn diff_detailed_with_context(
        &self,
        context: u32,
        range: DiffRange,
    ) -> anyhow::Result<Vec<FileDiff>> {
        let base = self.range_base(range)?;
        let changes = tracked_changes(self, &base)?;
        let mut files = Vec::with_capacity(changes.len());
        let mut known_paths = HashSet::new();
        for change in changes {
            known_paths.extend(change.paths.iter().cloned());
            files.push(tracked_diff(self, &base, context, change)?);
        }
        for path in untracked_paths(self)? {
            if !known_paths.contains(&path) {
                files.push(untracked_diff(self, context, path)?);
            }
        }
        Ok(files)
    }

    /// diff 기준점. 고정해 둔 SHA가 최우선이고, 그게 없거나 죽었을 때만 merge-base로 내려간다.
    ///
    /// merge-base를 기본으로 쓰면 base 브랜치가 이 작업의 커밋을 따라잡는 순간 기준점이
    /// HEAD 쪽으로 밀려, 이미 검토한 변경과 거기 붙은 주석이 함께 사라진다.
    /// 고른 범위의 기준점. 미커밋 범위는 HEAD가 곧 기준이라 기준점 판정이 필요 없다.
    ///
    /// `changed_paths()`는 이걸 쓰지 않는다 — 승인 정책은 범위를 고를 수 있는 대상이 아니다.
    fn range_base(&self, range: DiffRange) -> anyhow::Result<String> {
        match range {
            DiffRange::Session => self.diff_base(),
            DiffRange::Uncommitted => Ok("HEAD".to_string()),
        }
    }

    /// 화면이 지금 어떤 기준점을 보고 있는지. `diff_base()`와 같은 판정을 쓴다.
    pub fn baseline_status(&self) -> BaselineStatus {
        match &self.base_revision {
            None => BaselineStatus::Legacy,
            Some(revision) if super::is_ancestor(&self.path, revision, "HEAD") => {
                BaselineStatus::Pinned
            }
            Some(_) => BaselineStatus::Degraded,
        }
    }

    /// diff 기준점.
    ///
    /// 기본은 merge-base다. 고정해 둔 SHA는 **base가 이 작업을 삼켰을 때만** 꺼낸다.
    /// 두 실패 모드가 서로 반대여서 한쪽만으로는 안 된다:
    ///
    /// - base가 내 커밋을 흡수하면(PR 머지 후 pull) merge-base가 HEAD 쪽으로 밀려
    ///   이미 검토한 변경과 주석이 사라진다 → 고정 SHA가 필요하다.
    /// - 내가 base를 흡수하면(충돌 해결·`git merge dev`) 고정 SHA 기준에 남의 변경이
    ///   섞인다. `changed_paths()`는 Goal Contract 검사에 쓰이므로 표시 문제로 끝나지 않는다
    ///   → merge-base가 필요하다.
    ///
    /// 둘은 "base가 HEAD를 포함하는가"로 갈린다.
    fn diff_base(&self) -> anyhow::Result<String> {
        let Some(revision) = &self.base_revision else {
            // 구버전 작업 — 기준점이 없으니 종전대로 base 브랜치로 계산한다.
            return self.merge_base_with(&self.base);
        };
        if !super::is_ancestor(&self.path, revision, "HEAD") {
            // rebase/amend가 기준점을 버렸다. 근사치로 물러선다 — 호출자는
            // `baseline_status()`로 이 상태를 알 수 있다.
            return self.merge_base_with(revision);
        }
        if super::is_ancestor(&self.path, "HEAD", &self.base) {
            return Ok(revision.clone());
        }
        self.merge_base_with(&self.base)
    }

    fn merge_base_with(&self, other: &str) -> anyhow::Result<String> {
        let base = run_git(&self.path, &["merge-base", "HEAD", other])?
            .trim()
            .to_string();
        if base.is_empty() {
            anyhow::bail!("task diff 기준 commit을 찾을 수 없습니다");
        }
        Ok(base)
    }
}

fn tracked_changes(worktree: &Worktree, base: &str) -> anyhow::Result<Vec<ChangedPath>> {
    let raw = run_git(
        &worktree.path,
        &["diff", base, "--name-status", "-z", "-M", "-C"],
    )?;
    let mut fields = raw.split('\0').filter(|field| !field.is_empty());
    let mut changes = Vec::new();
    while let Some(raw_status) = fields.next() {
        let path_count = usize::from(raw_status.starts_with(['R', 'C'])) + 1;
        let paths = fields
            .by_ref()
            .take(path_count)
            .map(str::to_string)
            .collect();
        changes.push(ChangedPath {
            status: raw_status.chars().next().unwrap_or('?').to_string(),
            paths,
        });
    }
    Ok(changes)
}

fn tracked_diff(
    worktree: &Worktree,
    base: &str,
    context: u32,
    change: ChangedPath,
) -> anyhow::Result<FileDiff> {
    for path in &change.paths {
        if let Some(reason) = omission(&worktree.path.join(path)) {
            return Ok(FileDiff {
                path: change.paths.last().cloned().unwrap_or_default(),
                status: change.status,
                patch: marker(path, reason),
            });
        }
    }
    // 파일 목록은 UTF-8 경로를 쓰므로 patch 헤더도 같은 표기를 강제한다. Git 기본값의
    // 비ASCII C-style 이스케이프가 남으면 구조화 hunk가 파일에 연결되지 않는다.
    let mut args = vec![
        "-c".to_string(),
        "core.quotePath=false".to_string(),
        "diff".to_string(),
        base.to_string(),
        format!("--unified={context}"),
        "-M".to_string(),
        "-C".to_string(),
        "--".to_string(),
    ];
    args.extend(change.paths.iter().cloned());
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let path = change.paths.last().cloned().unwrap_or_default();
    let patch = run_diff_command(worktree, &refs, &path)?;
    Ok(FileDiff {
        path: change.paths.last().cloned().unwrap_or_default(),
        status: change.status,
        patch,
    })
}

fn untracked_paths(worktree: &Worktree) -> anyhow::Result<Vec<String>> {
    Ok(run_git(
        &worktree.path,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?
    .split('\0')
    .filter(|path| !path.is_empty())
    .map(str::to_string)
    .collect())
}

fn untracked_diff(worktree: &Worktree, context: u32, path: String) -> anyhow::Result<FileDiff> {
    if let Some(reason) = omission(&worktree.path.join(&path)) {
        return Ok(FileDiff {
            patch: marker(&path, reason),
            path,
            status: "A".into(),
        });
    }
    // tracked patch와 같은 경로 계약을 지켜 신규 파일도 기본 통합 보기에 연결한다.
    let patch = run_diff_command(
        worktree,
        &[
            "-c",
            "core.quotePath=false",
            "diff",
            "--no-index",
            &format!("--unified={context}"),
            "--",
            "/dev/null",
            &path,
        ],
        &path,
    )?;
    Ok(FileDiff {
        path,
        status: "A".to_string(),
        patch,
    })
}

fn run_diff_command(worktree: &Worktree, args: &[&str], path: &str) -> anyhow::Result<String> {
    let mut child = Command::new("git")
        .current_dir(&worktree.path)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut bytes = Vec::new();
    let read = child
        .stdout
        .take()
        .expect("piped stdout")
        .take(MAX_PATCH_BYTES + 1)
        .read_to_end(&mut bytes);
    if read.is_err() || bytes.len() as u64 > MAX_PATCH_BYTES {
        let _ = child.kill();
        let _ = child.wait();
        read?;
        return Ok(marker(path, LARGE));
    }
    let status = child.wait()?;
    if !status.success() && status.code() != Some(1) {
        anyhow::bail!("git diff failed for {path}: {status}");
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
