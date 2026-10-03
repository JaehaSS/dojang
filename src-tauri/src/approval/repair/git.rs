use crate::worktree::Worktree;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub fn git(path: &Path, args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new("git")
        .current_dir(path)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .output()?;
    anyhow::ensure!(
        out.status.success(),
        "git {args:?}: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8(out.stdout)?)
}

pub fn revision(path: &Path, reference: &str) -> anyhow::Result<String> {
    Ok(git(path, &["rev-parse", "--verify", reference])?
        .trim()
        .into())
}

/// Includes staging boundaries, file bytes, untracked files and symbolic link targets.
pub fn fingerprint(path: &Path) -> anyhow::Result<String> {
    let mut digest = Sha256::new();
    for args in [
        vec!["rev-parse", "HEAD"],
        vec!["status", "--porcelain=v1", "-z", "-uall"],
        vec!["diff", "--no-ext-diff", "--binary", "HEAD"],
        vec!["diff", "--cached", "--no-ext-diff", "--binary"],
    ] {
        digest.update(git(path, &args)?.as_bytes());
        digest.update([0]);
    }
    let index = git(
        path,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )?;
    digest.update(std::fs::read(index.trim())?);
    let names = git(path, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    for name in names.split('\0').filter(|s| !s.is_empty()) {
        // App-owned candidate and journal directories aren't task input.
        if name.starts_with(".praxis/worktrees/") || name.starts_with(".praxis/approval/") {
            continue;
        }
        digest.update(name.as_bytes());
        digest.update([0]);
        let file = path.join(name);
        let meta = std::fs::symlink_metadata(&file)?;
        if meta.file_type().is_symlink() {
            digest.update(std::fs::read_link(file)?.as_os_str().as_encoded_bytes());
        } else if meta.is_file() {
            digest.update(std::fs::read(file)?);
        } else {
            anyhow::bail!("지원하지 않는 작업 파일: {name}");
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub fn files_fingerprint(root: &Path, names: &[String]) -> anyhow::Result<String> {
    let mut digest = Sha256::new();
    for name in names {
        anyhow::ensure!(
            Path::new(name)
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_))),
            "환경 파일 경로가 잘못됐습니다"
        );
        let file = root.join(name);
        anyhow::ensure!(
            file.canonicalize()?.starts_with(root.canonicalize()?),
            "환경 파일이 후보를 벗어났습니다"
        );
        digest.update(name.as_bytes());
        digest.update([0]);
        digest.update(std::fs::read(file)?);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub fn candidate_fingerprint(session: &super::Session) -> anyhow::Result<String> {
    let path = Path::new(&session.candidate_path);
    Ok(format!(
        "{}:{}",
        fingerprint(path)?,
        files_fingerprint(path, &session.environment_files)?
    ))
}

pub fn managed_dir(root: &Path, parts: &[&str]) -> anyhow::Result<PathBuf> {
    let mut path = root.canonicalize()?;
    for part in parts {
        path.push(part);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) => anyhow::ensure!(
                meta.is_dir() && !meta.file_type().is_symlink(),
                "관리 경로가 디렉터리가 아닙니다: {}",
                path.display()
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => std::fs::create_dir(&path)?,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(path)
}

pub fn snapshot(
    worktree: &Worktree,
    directory: &Path,
    exclude_mcp: bool,
) -> anyhow::Result<String> {
    anyhow::ensure!(
        git(&worktree.path, &["ls-files", "--unmerged", "-z"])?.is_empty(),
        "진행 중인 충돌을 먼저 마무리하세요"
    );
    let source = revision(&worktree.path, "HEAD")?;
    let index = directory.join("snapshot-index");
    anyhow::ensure!(!index.exists(), "snapshot index가 이미 존재합니다");
    let command = |args: &[&str]| -> anyhow::Result<String> {
        let out = Command::new("git")
            .current_dir(&worktree.path)
            .env("GIT_INDEX_FILE", &index)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .args(args)
            .output()?;
        anyhow::ensure!(
            out.status.success(),
            "snapshot 실패: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(String::from_utf8(out.stdout)?.trim().into())
    };
    let result = (|| {
        command(&["read-tree", &source])?;
        command(&["add", "-A"])?;
        if exclude_mcp {
            command(&["reset", "--quiet", &source, "--", ".mcp.json"])?;
        }
        command(&[
            "reset",
            "--quiet",
            &source,
            "--",
            ".praxis/worktrees",
            ".praxis/approval",
        ])?;
        let tree = command(&["write-tree"])?;
        command(&[
            "commit-tree",
            &tree,
            "-p",
            &source,
            "-m",
            "dojang: preserved input for automatic repair",
        ])
    })();
    let _ = std::fs::remove_file(&index);
    result
}

pub fn candidate(session: &super::Session) -> Worktree {
    Worktree {
        repo: session.repo.clone().into(),
        path: session.candidate_path.clone().into(),
        branch: session.candidate_branch.clone(),
        base: session.base.clone(),
        base_revision: Some(session.target_sha.clone()),
    }
}

pub fn assert_original(session: &super::Session) -> anyhow::Result<()> {
    anyhow::ensure!(
        files_fingerprint(Path::new(&session.source_path), &session.environment_files)?
            == session.source_environment_fingerprint,
        "원본 환경 파일이 변경됐습니다. 새 해결 세션을 준비하세요"
    );
    anyhow::ensure!(
        revision(
            Path::new(&session.repo),
            &format!("refs/heads/{}", session.base)
        )? == session.target_sha,
        "대상 브랜치가 변경됐습니다. 새 해결 세션을 준비하세요."
    );
    anyhow::ensure!(
        revision(Path::new(&session.source_path), "HEAD")? == session.source_sha
            && git(
                Path::new(&session.source_path),
                &["branch", "--show-current"]
            )?
            .trim()
                == session.source_branch
            && fingerprint(Path::new(&session.source_path))? == session.source_fingerprint,
        "원본 작업이 변경됐습니다. 원본을 보존하고 후보 채택을 중단했습니다."
    );
    Ok(())
}

pub fn assert_candidate(session: &super::Session) -> anyhow::Result<()> {
    let worktree = candidate(session);
    worktree.validate_isolated_approval()?;
    let root = managed_dir(&worktree.repo, &[".praxis", "worktrees"])?;
    anyhow::ensure!(
        worktree.path.canonicalize()?.parent() == Some(root.as_path()),
        "해결 후보가 관리 경로를 벗어났습니다"
    );
    let meta = std::fs::symlink_metadata(&worktree.path)?;
    anyhow::ensure!(
        !meta.file_type().is_symlink(),
        "해결 후보 symlink는 허용되지 않습니다"
    );
    Ok(())
}

pub fn verify_ancestry(session: &super::Session) -> anyhow::Result<()> {
    let path = Path::new(&session.candidate_path);
    for parent in [&session.snapshot_sha, &session.target_sha] {
        git(path, &["merge-base", "--is-ancestor", parent, "HEAD"])?;
    }
    Ok(())
}
