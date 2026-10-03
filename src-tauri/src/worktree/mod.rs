//! Git worktree 관리 — Tauri 비의존, `cargo test`로 검증 가능.
//!
//! Phase 1: 레포에 격리 worktree(+브랜치)를 만들고, 에이전트가 거기서 작업하게 한 뒤
//! `git diff --stat`로 변경 요약을 보여주고, 승인(머지+정리) 또는 폐기(제거)한다.
//! worktree는 `<repo>/.praxis/worktrees/<slug>` 에 생성한다(.praxis는 gitignore).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod approval_guard;
mod approval_merge;
pub mod readiness;
pub mod bootstrap;
pub mod refresh;
mod conflict;
mod direct_checkout_lock;
mod diff;
mod diff_stats;
mod diff_limits;
mod review;
mod recovery_identity;

pub use conflict::{ConflictFile, Resolution};
pub use direct_checkout_lock::{DirectCheckoutGuard, DirectCheckoutLocks};

/// 워크트리가 사라진 작업에 무언가를 실행하려 할 때의 안내.
///
/// "없습니다"만으로는 사용자가 무엇을 할 수 있는지 알 수 없다. 워크트리는 앱 밖에서도 사라지므로
/// (수동 `rm`, 다른 도구의 `git worktree prune`) 경로와 복구 수단을 함께 적는다. 브랜치는 대개
/// 남아 있어 — 앱의 정리 경로는 브랜치까지 지운다 — 커밋된 작업물은 거기서 되찾을 수 있다.
pub fn missing_worktree_error(path: &str) -> String {
    format!(
        "워크트리 디렉터리가 없습니다: {path}\n\
         앱 밖에서 삭제된 것으로 보입니다. 사이드바에서 이 작업을 버리거나, \
         `git worktree add <경로> <브랜치>`로 되살린 뒤 다시 시도하세요."
    )
}

/// 파일 단위 변경 (DiffViewer S-04용).
#[derive(Debug, Clone, Serialize)]
pub struct FileDiff {
    pub path: String,
    pub status: String, // M/A/D/R/...
    pub patch: String,  // unified diff
}

/// diff 응답 — 파일 목록과 그것을 만든 기준점의 상태를 함께 싣는다.
///
/// 상태를 별도 커맨드로 빼지 않는 이유는 Diff 화면이 5초마다 폴링하기 때문이다. 왕복이
/// 두 배가 되고, 두 응답이 서로 다른 시점을 가리킬 수도 있다.
#[derive(Debug, Clone, Serialize)]
pub struct TaskDiffResult {
    pub files: Vec<FileDiff>,
    pub baseline: BaselineStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<ReviewSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    pub hunks: Vec<crate::diffmodel::DiffHunk>,
    pub annotations: Vec<crate::annotations::RematchedAnnotation>,
}

/// diff를 어디부터 뜰 것인가.
///
/// 화면이 고르는 값이라 기본값이 곧 첫 인상이다. 세션 전체를 기본으로 두는 이유는
/// "이 작업이 무엇을 했나"가 검토자가 가장 먼저 묻는 것이기 때문이다 — 커밋 여부는
/// 그다음 문제다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffRange {
    /// 세션 시작부터 지금까지. 커밋된 변경을 포함한다.
    #[default]
    Session,
    /// 아직 커밋하지 않은 것만.
    Uncommitted,
}

/// diff 기준점이 어떤 상태인지 — 화면이 근사치를 보고 있는지 사용자에게 알리기 위한 것.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BaselineStatus {
    /// 고정 기준점이 살아 있다 — 정상.
    Pinned,
    /// rebase/amend가 기준점을 버려 근사치를 보고 있다.
    Degraded,
    /// 기준점을 기록하기 전에 만들어진 작업 — base 브랜치로 계산한다.
    Legacy,
}

/// 폐기 시점 상태를 어디에 남겼는가 — 커밋과 **그것을 붙드는 브랜치**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreservedEvidence {
    pub commit: String,
    pub branch: String,
}

/// 격리 worktree 핸들.
#[derive(Debug, Clone)]
pub struct Worktree {
    pub repo: PathBuf,
    pub path: PathBuf,
    pub branch: String,
    /// 승인 머지가 들어갈 목적지. 브랜치 이름이다(직접 모드만 SHA).
    pub base: String,
    /// diff 기준점의 불변 SHA. `base`와 갈라 둔 이유는 하나가 둘을 겸할 수 없어서다 —
    /// 머지 목적지는 움직이는 브랜치여야 하고, diff 기준점은 움직이면 안 된다.
    /// None이면 레거시 경로(`base`로 merge-base)로 돈다.
    pub base_revision: Option<String>,
}

/// Git 메타데이터 없이 직접 실행한 작업의 브랜치·base 대체값.
pub const DIRECT_BRANCH: &str = "direct";

fn run_git(cwd: &Path, args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new("git").current_dir(cwd).args(args).output()?;
    if !out.status.success() {
        anyhow::bail!(
            "git {:?} 실패: {}",
            args,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn run_git_in_validation_index(
    cwd: &Path,
    git_dir: &Path,
    index: &Path,
    args: &[&str],
) -> anyhow::Result<String> {
    let out = Command::new("git")
        .current_dir(cwd)
        .env("GIT_DIR", git_dir)
        .env("GIT_WORK_TREE", cwd)
        .env("GIT_INDEX_FILE", index)
        .args(args)
        .output()?;
    if !out.status.success() {
        anyhow::bail!(
            "git {:?} 실패: {}",
            args,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn hook_validation_dir(git_dir: &Path) -> anyhow::Result<PathBuf> {
    let root = git_dir.join("praxis-hook-validation");
    std::fs::create_dir_all(&root)?;
    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?;
    let path = root.join(format!("{}-{}", std::process::id(), timestamp.as_nanos()));
    std::fs::create_dir(&path)?;
    Ok(path)
}

fn working_tree_fingerprint(path: &Path) -> anyhow::Result<String> {
    let diff = run_git(path, &["diff", "--no-ext-diff", "--binary", "HEAD"])?;
    let untracked = run_git(path, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    let mut hasher = Sha256::new();
    hasher.update(diff.as_bytes());
    for relative in untracked.split('\0').filter(|path| !path.is_empty()) {
        let file = path.join(relative);
        let metadata = std::fs::symlink_metadata(&file)?;
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        if metadata.file_type().is_symlink() {
            hasher.update(std::fs::read_link(file)?.to_string_lossy().as_bytes());
        } else if metadata.is_file() {
            hasher.update(std::fs::read(file)?);
        } else {
            anyhow::bail!("approval hook worktree has an unsupported untracked path: {relative}");
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// 네트워크를 타는 git 명령의 상한. 넘으면 죽이고 실패로 본다.
const NETWORK_GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// 원격에 닿는 git 명령 전용 실행기.
///
/// `run_git`과 둘로 나눈 이유는 **자격증명 프롬프트** 하나다. HTTPS 원격에서 git은 기본적으로
/// 터미널이나 헬퍼에게 아이디/비밀번호를 묻고, 응답이 없으면 무한히 기다린다. 앱에는 그 프롬프트를
/// 보여줄 터미널이 없으므로 서브프로세스가 그대로 멈추고 **작업 생성 전체가 그 뒤에 매달린다.**
/// `GIT_TERMINAL_PROMPT=0`으로 묻지 말고 실패하게 하고, 그래도 안 죽는 경우를 위해 타임아웃을 건다.
///
/// **로컬 전용 명령에 쓰지 말 것** — 큰 레포의 정상 `worktree add`·`merge`가 30초를 넘길 수 있고,
/// 그때 죽이면 인덱스가 반쯤 쓰인 상태로 남는다.
fn run_git_network(cwd: &Path, args: &[&str]) -> anyhow::Result<String> {
    use std::process::Stdio;

    let mut child = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        // askpass 헬퍼가 GUI 창을 띄우는 경로도 막는다 — 창이 뜨면 사용자는 작업을 만들려다
        // 갑자기 인증 대화상자를 보게 된다.
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        // ssh 원격도 같은 이유로 물어보지 않게 한다.
        .env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes -oStrictHostKeyChecking=accept-new")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let deadline = std::time::Instant::now() + NETWORK_GIT_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait()? {
            let out = child.wait_with_output()?;
            if !status.success() {
                anyhow::bail!(
                    "git {:?} 실패: {}",
                    args,
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
            return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!(
                "git {:?}가 {}초 안에 끝나지 않았습니다",
                args,
                NETWORK_GIT_TIMEOUT.as_secs()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// 지시문/브랜치명을 git 브랜치 슬러그로 정규화 (영숫자/'-'/'/'만, 소문자).
pub fn slugify(input: &str) -> String {
    let first = input.lines().next().unwrap_or("").trim();
    let mut s = String::new();
    let mut prev_dash = false;
    for c in first.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_ascii_alphanumeric() {
            s.push(c);
            prev_dash = false;
        } else if !prev_dash && !s.is_empty() {
            s.push('-');
            prev_dash = true;
        }
    }
    let s = s.trim_matches('-');
    let s: String = s.chars().take(40).collect();
    if s.is_empty() {
        "task".into()
    } else {
        s.trim_matches('-').to_string()
    }
}

/// 현재 체크아웃된 브랜치명.
pub fn current_branch(repo: &Path) -> anyhow::Result<String> {
    Ok(run_git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string())
}

/// 두 ref의 공통 조상. 기준점 backfill이 "지금 기준으로 어디서 갈라졌나"를 물을 때 쓴다.
pub fn merge_base(repo: &Path, left: &str, right: &str) -> anyhow::Result<String> {
    Ok(run_git(repo, &["merge-base", left, right])?.trim().to_string())
}

/// 현재 HEAD의 불변 commit SHA.
pub fn current_revision(repo: &Path) -> anyhow::Result<String> {
    Ok(run_git(repo, &["rev-parse", "HEAD"])?.trim().to_string())
}

/// `ancestor`가 `descendant`에 포함되는가 — 두 갈래가 서로를 삼켰는지 묻는 데 쓴다.
///
/// diff 기준점 판정이 두 방향을 모두 물어야 해서 일반형이다. 하나는 "고정해 둔 SHA가 아직
/// 살아 있나"(rebase/amend가 버리지 않았나), 다른 하나는 "base가 이 작업을 이미 삼켰나"다.
/// 실패(존재하지 않는 ref 등)도 거짓으로 접는다 — 판정 불가와 조상 아님은 후속 처리가 같다.
pub fn is_ancestor(repo: &Path, ancestor: &str, descendant: &str) -> bool {
    Command::new("git")
        .current_dir(repo)
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// 로컬 브랜치 목록(최근 커밋순) — 새 작업의 base 후보로 UI가 고른다.
/// 원격 추적 브랜치는 담지 않는다: 승인 머지는 로컬 브랜치로 들어가므로 고를 수 있는 것도
/// 로컬이어야 한다(`merge_for_approval`).
pub fn list_local_branches(repo: &Path) -> anyhow::Result<Vec<String>> {
    Ok(run_git(
        repo,
        &[
            "for-each-ref",
            "--format=%(refname:short)",
            "--sort=-committerdate",
            "refs/heads/",
        ],
    )?
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty())
    .map(str::to_string)
    .collect())
}

/// 로컬 브랜치 실재 여부. 리모트·태그·SHA는 여기서 false다.
pub fn local_branch_exists(repo: &Path, branch: &str) -> bool {
    Command::new("git")
        .current_dir(repo)
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .status()
        .is_ok_and(|status| status.success())
}

/// linked worktree 중 선택 브랜치를 이미 checkout한 경로.
fn branch_worktree_path(repo: &Path, branch: &str) -> anyhow::Result<Option<PathBuf>> {
    let output = run_git(repo, &["worktree", "list", "--porcelain"])?;
    let target = format!("refs/heads/{branch}");
    let mut path = None;
    for line in output.lines() {
        if let Some(value) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(value));
            continue;
        }
        if line.strip_prefix("branch ") == Some(target.as_str()) {
            return Ok(path);
        }
        if line.is_empty() {
            path = None;
        }
    }
    Ok(None)
}

/// 직접 실행이 시작할 기존 로컬 브랜치로 메인 체크아웃을 전환한다.
/// 다른 브랜치로 옮길 때의 변경은 untracked 파일까지 stash에 보존하고, 대상 브랜치에는 적용하지 않는다.
pub fn checkout_local_branch(repo: &Path, branch: &str) -> anyhow::Result<()> {
    let branch = branch.trim();
    if branch.is_empty() || current_branch(repo)? == branch {
        return Ok(());
    }
    if !local_branch_exists(repo, branch) {
        anyhow::bail!("'{branch}' 로컬 브랜치를 찾을 수 없습니다");
    }
    if let Some(path) = branch_worktree_path(repo, branch)? {
        anyhow::bail!(
            "'{branch}' 브랜치는 다른 워크트리에서 사용 중입니다: {}",
            path.display()
        );
    }
    let status = run_git(
        repo,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    if !status.is_empty() {
        let message = format!("Dojang: '{branch}' 전환 전 자동 보관");
        run_git(
            repo,
            &["stash", "push", "--include-untracked", "-m", &message],
        )?;
        let status = run_git(
            repo,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )?;
        if !status.is_empty() {
            anyhow::bail!(
                "메인 체크아웃의 변경 사항을 stash에 보관했지만 작업 디렉터리가 깨끗해지지 않아 '{branch}' 브랜치로 전환할 수 없습니다"
            );
        }
    }
    run_git(repo, &["checkout", branch])?;
    let current = current_branch(repo)?;
    if current != branch {
        anyhow::bail!("브랜치 전환 후 체크아웃이 '{current}'입니다 — 요청한 '{branch}'와 다릅니다");
    }
    Ok(())
}

/// 직접 실행은 Git 저장소가 아닌 디렉터리도 허용한다. Git 브랜치를 읽을 수 있으면 보존하고,
/// 읽을 수 없으면 Git 연산을 요구하지 않는 작업 메타데이터로 대체한다.
pub fn current_branch_or_direct(repo: &Path) -> String {
    current_branch(repo).unwrap_or_else(|_| DIRECT_BRANCH.to_string())
}

/// Git 의존 기능(diff·merge)을 실행해도 되는 저장소인지 확인한다.
pub fn is_git_repository(repo: &Path) -> bool {
    Command::new("git")
        .current_dir(repo)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .is_ok_and(|out| {
            out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true"
        })
}

/// 레포 로컬 `.git/info/<file>`에 주어진 라인들을 멱등하게 추가한다. 커밋 대상 파일
/// (`.gitattributes`/`.gitignore`)은 건드리지 않는다 (여러 worktree가 공유하는 레포 로컬
/// 설정이므로 `--git-common-dir` 기준).
fn ensure_git_info_lines(repo: &Path, file: &str, lines: &[&str]) -> anyhow::Result<()> {
    let common_dir = run_git(repo, &["rev-parse", "--git-common-dir"])?;
    let common_dir = common_dir.trim();
    let common_dir = if Path::new(common_dir).is_absolute() {
        PathBuf::from(common_dir)
    } else {
        repo.join(common_dir)
    };
    let info_dir = common_dir.join("info");
    std::fs::create_dir_all(&info_dir)?;
    let path = info_dir.join(file);
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut content = existing.clone();
    for line in lines {
        if content.lines().any(|l| l.trim() == *line) {
            continue;
        }
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(line);
        content.push('\n');
    }
    if content != existing {
        std::fs::write(&path, content)?;
    }
    Ok(())
}

/// projector가 worktree 루트에 다루는 하네스 컨텍스트 파일(CLAUDE.md/AGENTS.md, 옛 GEMINI.md 포함)을
/// 레포 로컬 `info/exclude`로 무시한다. 이 파일들이 `approve()`의 `add -A`에 딸려 브랜치에
/// 커밋되면 base 쪽에 같은 파일이 미추적으로 존재할 때 머지가 "untracked working tree files
/// would be overwritten"으로 실패한다 — 커밋 자체를 막아 원천 차단.
/// 선행 `/`로 루트 한정(하위 디렉터리의 동명 파일은 무관). ignore 규칙은 이미 tracked인
/// 파일에는 효력이 없으므로, 컨텍스트 파일을 커밋해서 쓰는 레포의 동작은 바뀌지 않는다.
fn ensure_context_files_excluded(repo: &Path) -> anyhow::Result<()> {
    let lines: Vec<String> = crate::projector::all_targets()
        .iter()
        .map(|t| format!("/{t}"))
        .collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    ensure_git_info_lines(repo, "exclude", &refs)
}

/// `init_repository`가 초기 커밋에 담을 파일 수 상한.
///
/// git 저장소가 아닌 폴더는 `node_modules`처럼 커밋할 이유가 없는 거대 트리를 품고 있을 수
/// 있다. 사용자 폴더에 되돌리기 번거로운 거대 커밋을 만드는 대신, 상한을 넘으면 초기화를
/// 거부하고 사용자가 `.gitignore`를 두고 직접 초기화하도록 안내한다.
pub const INIT_FILE_LIMIT: usize = 5000;

/// 초기화 대상 폴더의 파일 수를 상한까지만 센다(상한을 넘는 순간 멈춘다).
/// `.git`은 이미 저장소인 경우에만 있으므로 세지 않는다.
fn count_files_up_to(dir: &Path, limit: usize) -> usize {
    let mut count = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            // 심볼릭은 따라가지 않는다 — 링크 루프로 카운트가 폭주하지 않게.
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if entry.file_name() == ".git" {
                    continue;
                }
                stack.push(entry.path());
            } else {
                count += 1;
                if count > limit {
                    return count;
                }
            }
        }
    }
    count
}

/// `HEAD`가 실제 커밋을 가리키는가 — `git init` 직후의 unborn HEAD를 걸러낸다.
///
/// `is_git_repository`로는 이걸 알 수 없다. 저장소이면서 커밋이 0개인 상태가 존재하고,
/// 그때 `rev-parse HEAD`는 "ambiguous argument 'HEAD'"로 실패한다.
fn has_commit(dir: &Path) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])
        .output()
        .is_ok_and(|out| out.status.success())
}

/// 격리·diff가 성립하는 저장소인가 — 저장소이면서 `HEAD`가 커밋을 가리킬 때만 참.
///
/// `is_git_repository`는 "git 명령을 쓸 수 있나"를 묻는다. 이쪽은 "이 저장소로 작업을 시작할 수
/// 있나"를 묻는다. 둘은 `git init` 직후에 갈린다 — 저장소는 맞지만 `create`도 `diff_base`도
/// 설 자리가 없다. 사용자에게 상태를 보여주는 자리에서는 이쪽을 써야 한다.
pub fn is_ready_repository(dir: &Path) -> bool {
    is_git_repository(dir) && has_commit(dir)
}

/// 커밋 신원(`user.email`)이 설정돼 있는가 — 전역/로컬 어느 쪽이든.
fn has_commit_identity(dir: &Path) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(["config", "user.email"])
        .output()
        .is_ok_and(|out| {
            out.status.success() && !String::from_utf8_lossy(&out.stdout).trim().is_empty()
        })
}

/// 폴더를 git 저장소로 초기화하고 현재 내용을 초기 커밋으로 남긴다.
///
/// 워크트리 격리(`create`)는 `HEAD`가 가리키는 커밋이 있어야 성립한다 — 빈 커밋만 두면
/// 새 워크트리에 파일이 하나도 없어 작업이 무의미해지므로, 현재 내용을 그대로 커밋한다.
/// 커밋이 이미 있으면 아무것도 하지 않는다(멱등).
///
/// **저장소인 것과 커밋이 있는 것은 다르다.** 사용자가 `git init`만 해 둔 폴더는 저장소지만
/// unborn HEAD라, 여기서 그냥 돌려보내면 diff 기준점(`diff_base`)·체크포인트·승인 가드가
/// 모두 `rev-parse HEAD`에서 깨진다. 그래서 저장소여도 커밋이 없으면 초기 커밋까지 만든다.
/// 이때 `git init`과 파일 수 상한은 건너뛴다 — 사용자가 직접 만든 저장소는 이미 `.gitignore`를
/// 갖췄을 가능성이 높고, 상한으로 막으면 되레 작업을 시작할 수 없다.
pub fn init_repository(dir: &Path) -> anyhow::Result<()> {
    if !dir.is_dir() {
        anyhow::bail!("디렉터리가 아닙니다");
    }
    if is_git_repository(dir) {
        if has_commit(dir) {
            return Ok(());
        }
    } else {
        let count = count_files_up_to(dir, INIT_FILE_LIMIT);
        if count > INIT_FILE_LIMIT {
            anyhow::bail!(
                "파일이 {}개를 넘어 자동 초기화하지 않았습니다 — .gitignore를 먼저 두고 직접 `git init`하세요",
                INIT_FILE_LIMIT
            );
        }
        run_git(dir, &["init"])?;
    }
    run_git(dir, &["add", "-A"])?;
    // 사용자 전역 user.name/email이 없을 수 있으므로 그때만 이 커밋에 신원을 지정한다.
    // 이미 설정돼 있으면 건드리지 않는다 — 사용자 저장소의 첫 커밋 author를 Dojang으로 덮어쓰지 않기 위해서다.
    let mut args: Vec<&str> = Vec::new();
    if !has_commit_identity(dir) {
        args.extend(["-c", "user.name=Dojang", "-c", "user.email=dojang@local"]);
    }
    args.extend(["commit", "-m", "Initial commit", "--allow-empty"]);
    run_git(dir, &args)?;
    Ok(())
}

/// 생성 파이프라인에서 사용자에게 보이는 단계. 진행 이벤트와 계측이 같은 이름을 쓴다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateStage {
    Refresh,
    Worktree,
    Bootstrap,
}

impl CreateStage {
    pub fn as_str(self) -> &'static str {
        match self {
            CreateStage::Refresh => "refresh",
            CreateStage::Worktree => "worktree",
            CreateStage::Bootstrap => "bootstrap",
        }
    }
}

/// 격리 worktree + 새 브랜치 생성.
///
/// `on_stage`는 **실제로 거치는 단계에서만** 불린다 — 건너뛴 단계를 알리면 UI가 일어나지
/// 않은 일을 표시하고 계측에는 0ms 구간이 생긴다.
///
/// `refresh`가 참이면 분기 **전에** base를 원격 최신으로 맞춘다(fast-forward만). 그 결과를
/// 함께 돌려주는 이유는 호출부가 UI로 흘려보내야 하기 때문이다 — 삼키면 "왜 옛날 코드에서
/// 갈라졌지?"에 답할 방법이 없어진다. 최신화 실패는 **작업 생성을 막지 않는다**(계약 4).
///
/// `base`는 분기 기준 로컬 브랜치다. `None`(또는 빈 문자열)이면 종전대로 레포가 지금 체크아웃한
/// 브랜치에서 분기한다. 실재하지 않는 브랜치는 여기서 거부한다 — `git worktree add`에 그대로
/// 넘기면 "invalid reference" 수준의 메시지만 남아 무엇이 잘못됐는지 사용자에게 닿지 않는다.
pub fn create(
    repo: &Path,
    branch: &str,
    base: Option<&str>,
    refresh: bool,
    on_stage: &(dyn Fn(CreateStage) + Send + Sync),
) -> anyhow::Result<(Worktree, refresh::RefreshOutcome)> {
    let base = match base.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => {
            if !local_branch_exists(repo, name) {
                anyhow::bail!("'{name}' 브랜치를 찾을 수 없습니다");
            }
            name.to_string()
        }
        None => current_branch(repo)?,
    };
    ensure_context_files_excluded(repo)?;

    let outcome = if refresh {
        on_stage(CreateStage::Refresh);
        refresh::refresh_base(repo, &base)
    } else {
        refresh::RefreshOutcome::Skipped
    };
    on_stage(CreateStage::Worktree);
    // 다른 worktree가 base를 붙들고 있으면 로컬 ref를 못 옮긴다 — 원격 ref에서 직접 갈라진다.
    let start_point = if outcome.prefers_remote_ref() {
        format!("origin/{base}")
    } else {
        base.clone()
    };

    let dir_component = branch.replace('/', "-");
    let path = repo.join(".praxis").join("worktrees").join(dir_component);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("worktree 경로가 UTF-8이 아님"))?;
    // 기준점은 worktree를 만들기 **직전**에 잡는다. 만든 뒤에 잡으면 그 사이 base 브랜치가
    // 움직였을 때 실제 분기점과 어긋난다 — 그러면 남의 커밋이 이 작업의 diff로 새어 들어온다.
    let base_revision = run_git(repo, &["rev-parse", &base])
        .ok()
        .map(|out| out.trim().to_string())
        .filter(|revision| !revision.is_empty());
    run_git(
        repo,
        &["worktree", "add", "-b", branch, path_str, start_point.as_str()],
    )?;
    let worktree = Worktree {
        repo: repo.to_path_buf(),
        path,
        branch: branch.to_string(),
        base,
        base_revision,
    };
    // 새 worktree는 tracked 파일만 갖는다 — `.worktreeinclude`가 나열한 untracked
    // 파일을 채우고 셋업 훅을 돌린다. 실패해도 작업 생성을 막지 않는다(계약 4).
    //
    // is_direct()는 여기서 참이 될 수 없지만(create는 항상 새 경로를 만든다) 방어한다 —
    // 참인 채로 부르면 사용자의 메인 체크아웃에 복사하고 스크립트를 실행하게 된다.
    if !worktree.is_direct() {
        on_stage(CreateStage::Bootstrap);
        let transcript = bootstrap::run(repo, &worktree.path);
        if !transcript.is_empty() {
            eprintln!("[worktree] 환경 부트스트랩: {transcript:?}");
        }
    }
    Ok((worktree, outcome))
}

/// 최신화 없이 워크트리만 만든다.
///
/// 최신화가 **무의미하거나 해로운** 경로를 위한 것이다 — 원격이 없는 테스트 픽스처, 그리고
/// 앙상블 후보처럼 이미 정해진 base에서 갈라져야 하는 자리. 최신화 결과를 버리는 것이
/// 명시적이므로, 호출부가 결과를 삼키고 있는 것인지 애초에 안 하는 것인지 읽힌다.
pub fn create_plain(repo: &Path, branch: &str, base: Option<&str>) -> anyhow::Result<Worktree> {
    create(repo, branch, base, false, &|_| {}).map(|(wt, _)| wt)
}

impl Worktree {
    /// 직접 모드(워크트리 미격리) 판별 — 스키마 변경 없이 "경로 == repo 경로" 동일성으로
    /// 판별한다. true면 merge/cleanup(worktree add·remove, 브랜치 삭제)을 절대 호출하면
    /// 안 된다 — 메인 체크아웃 자체를 건드리게 된다.
    pub fn is_direct(&self) -> bool {
        self.path == self.repo
    }

    /// 변경 여부.
    pub fn has_changes(&self) -> anyhow::Result<bool> {
        Ok(!run_git(&self.path, &["status", "--porcelain"])?
            .trim()
            .is_empty())
    }

    fn has_staged_changes(&self) -> anyhow::Result<bool> {
        Ok(!run_git(&self.path, &["diff", "--cached", "--name-only"])?
            .trim()
            .is_empty())
    }

    /// 미추적(`??`) 파일 목록 — .gitignore는 git이 자동 제외.
    /// 검증(빌드/테스트) 후 산출물 오염 경고에 사용.
    pub fn untracked(&self) -> anyhow::Result<Vec<String>> {
        let out = run_git(
            &self.path,
            &["status", "--porcelain", "--untracked-files=all"],
        )?;
        Ok(out
            .lines()
            .filter_map(|l| l.strip_prefix("?? "))
            .map(|s| s.trim().to_string())
            .collect())
    }

    /// 승인: worktree 변경 커밋 → base에 머지 → worktree/브랜치 정리.
    pub fn approve(&self) -> anyhow::Result<()> {
        self.approve_observed(false, |_, _| {})
    }

    pub fn approve_with_generated_mcp_excluded(&self) -> anyhow::Result<()> {
        self.approve_observed(true, |_, _| {})
    }

    /// The observer records the actual failing stage without guessing from error text.
    pub fn approve_observed(
        &self,
        exclude_generated_mcp: bool,
        mut observe: impl FnMut(&str, Option<&str>),
    ) -> anyhow::Result<()> {
        observe("commit", None);
        let commit = self.commit_with_generated_mcp(exclude_generated_mcp)?;
        observe("merge", Some(&commit));
        self.merge_for_approval(&commit)?;
        observe("cleanup", Some(&commit));
        self.cleanup_after_finalization()
    }

    /// Commit only. The returned immutable SHA is persisted before merge for crash recovery.
    pub fn commit_for_approval(&self) -> anyhow::Result<String> {
        self.commit_with_generated_mcp(false)
    }

    /// Commit only while excluding Praxis-generated `.mcp.json`.
    pub fn commit_for_approval_with_generated_mcp_excluded(&self) -> anyhow::Result<String> {
        self.unstage_generated_mcp()?;
        let commit = self.commit_with_generated_mcp(true)?;
        self.validate_decision_commit(&commit, true)?;
        Ok(commit)
    }

    fn commit_with_generated_mcp(&self, exclude_generated_mcp: bool) -> anyhow::Result<String> {
        // 이 exclude가 도입되기 전에 만들어진 worktree도 커밋 직전에 소급 적용(멱등).
        ensure_context_files_excluded(&self.repo)?;
        let mut committed = false;
        if self.has_changes()? {
            if exclude_generated_mcp {
                run_git(&self.path, &["add", "-A", "--", ".", ":(exclude).mcp.json"])?;
            } else {
                run_git(&self.path, &["add", "-A"])?;
            }
            if self.has_staged_changes()? {
                run_git(
                    &self.path,
                    &["commit", "-m", &format!("dojang: {}", self.branch)],
                )?;
                committed = true;
            }
        }
        let commit = run_git(&self.path, &["rev-parse", "HEAD"])?
            .trim()
            .to_string();
        self.validate_approval_hooks(&commit, !committed)?;
        Ok(commit)
    }

    /// Merge only. Repeating after a successful merge is a no-op.
    pub fn merge_for_approval(&self, commit: &str) -> anyhow::Result<()> {
        approval_merge::merge(self, commit)
    }

    pub fn commit_is_merged(&self, commit: &str) -> bool {
        approval_merge::commit_is_merged(self, commit)
    }

    /// 폐기: worktree + 브랜치 삭제 (변경 버림).
    pub fn discard(&self) -> anyhow::Result<()> {
        self.cleanup_after_finalization()
    }

    /// 고아 종결: 남은 등록 메타데이터만 걷어내고 **브랜치는 남긴다**.
    ///
    /// [`discard`](Self::discard)와의 차이는 `branch -D` 하나다. 워크트리가 앱 밖에서 사라진
    /// 작업에서 브랜치는 커밋된 작업물의 **유일한** 사본이다 — 앱의 정상 정리 경로가 둘을 함께
    /// 지우기 때문에, 브랜치만 남은 조합은 애초에 앱이 만든 흔적이 아니다. 그 상태를 종결하면서
    /// 브랜치까지 지우면 사용자는 되찾을 수단을 잃는다.
    ///
    /// 등록과 자체 Git 메타데이터가 모두 남은 디렉터리만 `worktree remove`로 정상 제거한다.
    /// 등록 없이 `.git`도 없는 잔여 디렉터리는 근거로 남기고 종결한다.
    ///
    /// 워크트리에 담긴 미커밋 변경까지 남겨야 하는 사용자 폐기는 [`preserve_and_retire`]를 쓴다.
    ///
    /// [`preserve_and_retire`]: Self::preserve_and_retire
    pub fn retire_preserving_branch(&self) -> anyhow::Result<()> {
        approval_merge::cleanup(self)?;
        match recovery_identity::classify(self)? {
            recovery_identity::RetirementTarget::Registered(_) => {
                let path_str = self
                    .path
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("worktree 경로가 UTF-8이 아님"))?;
                run_git(&self.repo, &["worktree", "remove", "--force", path_str])?;
            }
            recovery_identity::RetirementTarget::Missing => {
                let _ = run_git(&self.repo, &["worktree", "prune"]);
            }
            recovery_identity::RetirementTarget::DetachedResidue => {}
        }
        Ok(())
    }

    /// 사용자 폐기: **버리기 직전 상태를 브랜치에 커밋한 뒤** 워크트리만 걷어낸다.
    ///
    /// [`discard`](Self::discard)와 다른 점이 둘이다 — 커밋을 남기고, `branch -D`를 하지 않는다.
    /// 폐기는 "이 변경을 머지하지 않겠다"는 결정이지 "근거를 지우겠다"는 결정이 아니다.
    /// `worktree remove --force`는 디렉터리를 통째로 지우므로 커밋되지 않은 파일은 reflog에도
    /// dangling 객체에도 남지 않는다 — 커밋 없이 지우면 되찾을 방법이 **없다**(설계 0056).
    ///
    /// 커밋이 실패하면 아무것도 지우지 않고 에러를 올린다. 정리하려다 근거를 태우는 것보다
    /// 작업이 검토 대기로 되돌아오는 편이 낫다 — 호출자 양쪽이 복원 계약을 이미 갖고 있다.
    ///
    /// 워크트리가 이미 없으면(고아) 커밋할 대상이 없으므로 보존 종결만 한다.
    /// 반환값은 근거가 남은 커밋과 그것을 붙드는 브랜치 — 저널에 기록해 나중에 어디를
    /// 봐야 하는지 알려준다.
    pub fn preserve_and_retire(&self) -> anyhow::Result<Option<PreservedEvidence>> {
        let preserved = match recovery_identity::classify(self)? {
            recovery_identity::RetirementTarget::Registered(head) => {
                let commit = self.checkpoint_commit("discard: 폐기 시점 상태 보존")?;
                let branch = self.anchor_preserved(&head, &commit)?;
                Some(PreservedEvidence { commit, branch })
            }
            recovery_identity::RetirementTarget::Missing
            | recovery_identity::RetirementTarget::DetachedResidue => None,
        };
        self.retire_preserving_branch()?;
        Ok(preserved)
    }

    /// 보존 커밋을 무엇이 붙드는가 — 그 브랜치 이름을 돌려준다.
    fn anchor_preserved(
        &self,
        head: &recovery_identity::Head,
        commit: &str,
    ) -> anyhow::Result<String> {
        match head {
            recovery_identity::Head::Task => Ok(self.branch.clone()),
            // 워크트리 안에서 갈아탄 브랜치 — 커밋이 이미 그 브랜치를 옮겼으므로 할 일이 없다.
            recovery_identity::Head::Branch(branch) => Ok(branch.clone()),
            recovery_identity::Head::Detached => {
                // `worktree remove`는 그 워크트리의 HEAD·reflog를 함께 지운다 — 분리 HEAD의
                // 커밋은 어디서도 닿을 수 없는 dangling 객체가 된다. 커밋해 놓고 잃는 것은
                // 커밋하지 않은 것과 같다(설계 0056). 구조 브랜치로 붙들어 둔다.
                let rescue = format!("{}-detached-{}", self.branch, &commit[..commit.len().min(8)]);
                run_git(&self.repo, &["branch", "--force", &rescue, commit])?;
                Ok(rescue)
            }
        }
    }

    /// B-2 부분 승인 체크포인트 — 현재 전체 worktree 상태를 로컬 브랜치에만 커밋한다(머지 대상
    /// 아님, `approve()`의 최종 커밋과 무관). `partial::apply`가 역패치 적용 전에 만들고,
    /// 실패/롤백 시 [`restore_to_checkpoint`]로 되돌린다. 변경이 없으면 현재 HEAD를 그대로 반환.
    pub fn checkpoint_commit(&self, message: &str) -> anyhow::Result<String> {
        ensure_context_files_excluded(&self.repo)?;
        if self.has_changes()? {
            run_git(&self.path, &["add", "-A"])?;
            if self.has_staged_changes()? {
                if self.is_direct() {
                    run_git(&self.path, &["commit", "-m", message])?;
                } else {
                    run_git(&self.path, &["commit", "--no-verify", "-m", message])?;
                }
            }
        }
        Ok(run_git(&self.path, &["rev-parse", "HEAD"])?
            .trim()
            .to_string())
    }

    fn validate_approval_hooks(&self, commit: &str, validate_message: bool) -> anyhow::Result<()> {
        if self.is_direct() {
            return Ok(());
        }
        let base = match run_git(&self.path, &["merge-base", commit, &self.base]) {
            Ok(base) => base.trim().to_string(),
            Err(_) => self
                .base_revision
                .clone()
                .ok_or_else(|| anyhow::anyhow!("approval hook base commit을 찾을 수 없습니다"))?,
        };
        if base == commit {
            return Ok(());
        }
        let git_dir = PathBuf::from(
            run_git(&self.path, &["rev-parse", "--absolute-git-dir"])?
                .trim()
                .to_string(),
        );
        let before = working_tree_fingerprint(&self.path)?;
        let validation = hook_validation_dir(&git_dir)?;
        let result =
            self.run_approval_hooks(&validation, &git_dir, &base, commit, validate_message);
        let after = working_tree_fingerprint(&self.path);
        let _ = std::fs::remove_dir_all(&validation);
        let _ = std::fs::remove_dir(validation.parent().unwrap_or(&git_dir));
        let after = after?;
        if before != after {
            anyhow::bail!(
                "승인 훅이 작업 파일을 변경했습니다. 변경 내용을 검토한 뒤 다시 승인하세요."
            );
        }
        result
    }

    fn run_approval_hooks(
        &self,
        validation: &Path,
        git_dir: &Path,
        base: &str,
        commit: &str,
        validate_message: bool,
    ) -> anyhow::Result<()> {
        std::fs::write(validation.join("HEAD"), format!("{base}\n"))?;
        let config = git_dir.join("config.worktree");
        if config.is_file() {
            std::fs::copy(config, validation.join("config.worktree"))?;
        }
        let common_dir = run_git(
            &self.path,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        std::fs::write(validation.join("commondir"), common_dir)?;
        let index = validation.join("index");
        run_git_in_validation_index(&self.path, validation, &index, &["read-tree", commit])?;
        let expected_tree =
            run_git_in_validation_index(&self.path, validation, &index, &["write-tree"])?;
        run_git_in_validation_index(
            &self.path,
            validation,
            &index,
            &["hook", "run", "--ignore-missing", "pre-commit"],
        )?;
        if run_git_in_validation_index(&self.path, validation, &index, &["write-tree"])?
            != expected_tree
        {
            anyhow::bail!("pre-commit hook changed the approval index");
        }
        if !validate_message {
            return Ok(());
        }
        let message = run_git(&self.path, &["show", "-s", "--format=format:%B", commit])?;
        let path = validation.join("COMMIT_EDITMSG");
        std::fs::write(&path, &message)?;
        let message_path = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("approval message path is not UTF-8"))?;
        run_git_in_validation_index(
            &self.path,
            validation,
            &index,
            &[
                "hook",
                "run",
                "--ignore-missing",
                "commit-msg",
                "--",
                message_path,
            ],
        )?;
        if run_git_in_validation_index(&self.path, validation, &index, &["write-tree"])?
            != expected_tree
        {
            anyhow::bail!("commit-msg hook changed the approval index");
        }
        if std::fs::read_to_string(path)? != message {
            anyhow::bail!("commit-msg hook changed the approval message");
        }
        Ok(())
    }

    /// 체크포인트 커밋으로 working tree+index를 완전히 원복한다(그 이후의 모든 변경 버림).
    /// `clean -fd`로 체크포인트 이후 생성된 미추적 파일도 제거한다 — 체크포인트가 `add -A`로
    /// 당시의 모든 미추적 파일까지 커밋에 포함시켰으므로, 이후 남는 미추적 파일은 실패한
    /// 부분 적용 시도의 잔재뿐이다.
    pub fn restore_to_checkpoint(&self, checkpoint: &str) -> anyhow::Result<()> {
        run_git(&self.path, &["reset", "--hard", checkpoint])?;
        run_git(&self.path, &["clean", "-fd"])?;
        Ok(())
    }

    /// Idempotent worktree/branch cleanup used by durable finalization recovery.
    pub fn cleanup_after_finalization(&self) -> anyhow::Result<()> {
        approval_merge::cleanup(self)?;
        let path_str = self
            .path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("worktree 경로가 UTF-8이 아님"))?;
        if self.path.exists() {
            run_git(&self.repo, &["worktree", "remove", "--force", path_str])?;
        } else {
            let _ = run_git(&self.repo, &["worktree", "prune"]);
        }
        // 브랜치 삭제는 best-effort (머지 후엔 -d, 폐기 시엔 -D 필요)
        let _ = run_git(&self.repo, &["branch", "-D", &self.branch]);
        Ok(())
    }
}

#[cfg(test)]
mod init_tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir =
            crate::testtmp::dir().join(format!("praxis-init-{}-{}-{}", tag, std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn init_makes_a_repository_whose_content_survives_into_a_worktree() {
        let dir = tmp_dir("basic");
        std::fs::write(dir.join("note.txt"), "hello").unwrap();
        assert!(!is_git_repository(&dir));

        init_repository(&dir).unwrap();
        assert!(is_git_repository(&dir));
        // 초기 커밋이 있어야 격리 워크트리를 만들 수 있고, 기존 파일이 거기 존재해야 한다.
        // 최신화를 켜도 원격이 없으면 조용히 넘어가고 **작업 생성은 성공한다**(계약 4).
        // 이 불변식이 깨지면 비행기 안이나 VPN 밖에서 앱이 작업을 못 만든다.
        let (wt, outcome) = create(&dir, "praxis/refresh-noremote", None, true, &|_| {}).unwrap();
        assert_eq!(outcome, refresh::RefreshOutcome::Skipped);
        assert!(wt.path.exists(), "최신화가 작업 생성을 막았다");
        wt.discard().unwrap();

        let wt = create(&dir, "praxis/after-init", None, false, &|_| {}).unwrap().0;
        assert_eq!(
            std::fs::read_to_string(wt.path.join("note.txt")).unwrap(),
            "hello"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    fn branch_exists(repo: &Path, branch: &str) -> bool {
        run_git(repo, &["rev-parse", "--verify", branch]).is_ok()
    }

    #[cfg(unix)]
    fn install_cached_docs_rejecting_hook(repo: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let hooks = repo.join("test-hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("pre-commit");
        std::fs::write(
            &hook,
            "#!/bin/sh\n\
             git config test.pre-commit-ran yes\n\
             git diff --cached --name-only | grep -q '^docs/' && exit 1\n\
             exit 0\n",
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        run_git(repo, &["config", "core.hooksPath", hooks.to_str().unwrap()]).unwrap();
        hooks
    }

    #[cfg(unix)]
    fn install_worktree_mutating_hook(repo: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let hooks = repo.join("test-hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("pre-commit");
        std::fs::write(
            &hook,
            "#!/bin/sh\n\
             case \"$GIT_DIR\" in\n\
             *praxis-hook-validation*) printf 'mutated\\n' > hook-mutated.txt;;\n\
             esac\n\
             exit 0\n",
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        run_git(repo, &["config", "core.hooksPath", hooks.to_str().unwrap()]).unwrap();
    }

    #[cfg(unix)]
    fn install_worktree_cached_docs_rejecting_hook(worktree: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let git_dir = PathBuf::from(
            run_git(worktree, &["rev-parse", "--absolute-git-dir"])
                .unwrap()
                .trim(),
        );
        let hooks = git_dir.join("test-hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("pre-commit");
        std::fs::write(
            &hook,
            "#!/bin/sh\n\
             git diff --cached --name-only | grep -q '^docs/' && exit 1\n\
             exit 0\n",
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        run_git(worktree, &["config", "extensions.worktreeConfig", "true"]).unwrap();
        run_git(
            worktree,
            &[
                "config",
                "--worktree",
                "core.hooksPath",
                hooks.to_str().unwrap(),
            ],
        )
        .unwrap();
    }

    #[test]
    fn retiring_an_orphan_keeps_the_branch_that_discard_would_delete() {
        let dir = tmp_dir("orphan");
        std::fs::write(dir.join("note.txt"), "hello").unwrap();
        init_repository(&dir).unwrap();
        let wt = create(&dir, "praxis/orphan", None, false, &|_| {}).unwrap().0;
        // 워크트리가 앱 밖에서 사라진 상황 — 디렉터리만 없애고 git 등록은 남긴다.
        std::fs::remove_dir_all(&wt.path).unwrap();

        wt.retire_preserving_branch().unwrap();
        assert!(
            branch_exists(&dir, "praxis/orphan"),
            "워크트리가 없는 지금 브랜치는 커밋된 작업물의 유일한 사본이라 남아야 한다"
        );

        // 대비군 — 같은 자리에서 통상 폐기는 브랜치까지 지운다. 이 대비가 성립해야
        // 고아 종결을 따로 둔 이유가 있다.
        wt.discard().unwrap();
        assert!(!branch_exists(&dir, "praxis/orphan"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discarding_a_worktree_commits_its_work_before_removing_it() {
        let dir = tmp_dir("preserve");
        std::fs::write(dir.join("note.txt"), "hello").unwrap();
        init_repository(&dir).unwrap();
        let wt = create(&dir, "praxis/preserve", None, false, &|_| {}).unwrap().0;
        // 폐기되는 작업의 산출물이 문서뿐인 경우 — 커밋된 적 없는 이 파일이 근거의 유일한 사본이다.
        // `worktree remove --force`는 이것을 reflog에도 남기지 않으므로 먼저 커밋해야 한다.
        std::fs::create_dir_all(wt.path.join("docs")).unwrap();
        std::fs::write(wt.path.join("docs/brief.md"), "근거").unwrap();

        let preserved = wt
            .preserve_and_retire()
            .unwrap()
            .expect("워크트리가 있으면 보존 커밋이 있어야 한다")
            .commit;

        assert!(!wt.path.exists(), "워크트리는 걷어내야 한다");
        assert!(
            branch_exists(&dir, "praxis/preserve"),
            "폐기는 머지하지 않겠다는 결정이지 근거를 지우겠다는 결정이 아니다"
        );
        assert_eq!(
            run_git(&dir, &["show", &format!("{preserved}:docs/brief.md")])
                .unwrap()
                .trim(),
            "근거",
            "커밋된 적 없던 파일이 보존 커밋에 담겨야 한다"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn preservation_bypasses_the_hook_and_keeps_committed_and_untracked_evidence() {
        let dir = tmp_dir("preserve-hook");
        init_repository(&dir).unwrap();
        let wt = create(&dir, "praxis/preserve-hook", None, false, &|_| {})
            .unwrap()
            .0;
        std::fs::write(wt.path.join("committed.md"), "committed evidence").unwrap();
        run_git(&wt.path, &["add", "-A"]).unwrap();
        run_git(&wt.path, &["commit", "-m", "existing evidence"]).unwrap();
        let hooks = install_cached_docs_rejecting_hook(&dir);
        std::fs::create_dir_all(wt.path.join("docs")).unwrap();
        std::fs::write(wt.path.join("docs/untracked.md"), "untracked evidence").unwrap();

        let preserved = wt.preserve_and_retire().unwrap().unwrap().commit;

        assert!(!wt.path.exists());
        assert!(branch_exists(&dir, "praxis/preserve-hook"));
        assert_eq!(
            run_git(&dir, &["show", &format!("{preserved}:committed.md")])
                .unwrap()
                .trim(),
            "committed evidence"
        );
        assert_eq!(
            run_git(&dir, &["show", &format!("{preserved}:docs/untracked.md")])
                .unwrap()
                .trim(),
            "untracked evidence"
        );
        assert_eq!(
            run_git(&dir, &["config", "--get", "core.hooksPath"])
                .unwrap()
                .trim(),
            hooks.to_str().unwrap()
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn approval_revalidates_a_clean_checkpoint_against_its_base() {
        let dir = tmp_dir("approval-clean-checkpoint");
        init_repository(&dir).unwrap();
        let wt = create(
            &dir,
            "praxis/approval-clean-checkpoint",
            None,
            false,
            &|_| {},
        )
        .unwrap()
        .0;
        install_cached_docs_rejecting_hook(&dir);
        std::fs::create_dir_all(wt.path.join("docs")).unwrap();
        std::fs::write(wt.path.join("docs/invalid.md"), "invalid snapshot").unwrap();

        wt.checkpoint_commit("praxis: snapshot").unwrap();

        assert!(!wt.has_changes().unwrap());
        assert!(
            wt.commit_for_approval().is_err(),
            "clean snapshots must still pass the approval hook"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn worktree_hook_rejects_ordinary_and_clean_snapshot_approval() {
        let dir = tmp_dir("approval-worktree-hook");
        init_repository(&dir).unwrap();
        let wt = create(&dir, "praxis/approval-worktree-hook", None, false, &|_| {})
            .unwrap()
            .0;
        install_worktree_cached_docs_rejecting_hook(&wt.path);
        std::fs::create_dir_all(wt.path.join("docs")).unwrap();
        std::fs::write(wt.path.join("docs/invalid.md"), "invalid ordinary change").unwrap();

        assert!(wt.commit_for_approval().is_err());
        run_git(&wt.path, &["reset", "--hard", "HEAD"]).unwrap();
        std::fs::create_dir_all(wt.path.join("docs")).unwrap();
        std::fs::write(wt.path.join("docs/invalid.md"), "invalid snapshot").unwrap();
        wt.checkpoint_commit("praxis: snapshot").unwrap();

        assert!(!wt.has_changes().unwrap());
        assert!(wt.commit_for_approval().is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn approval_revalidates_snapshot_changes_after_a_later_commit() {
        let dir = tmp_dir("approval-dirty-checkpoint");
        init_repository(&dir).unwrap();
        let wt = create(
            &dir,
            "praxis/approval-dirty-checkpoint",
            None,
            false,
            &|_| {},
        )
        .unwrap()
        .0;
        install_cached_docs_rejecting_hook(&dir);
        std::fs::create_dir_all(wt.path.join("docs")).unwrap();
        std::fs::write(wt.path.join("docs/invalid.md"), "invalid snapshot").unwrap();
        wt.checkpoint_commit("praxis: snapshot").unwrap();
        std::fs::write(wt.path.join("later.txt"), "ordinary approval change").unwrap();

        assert!(
            wt.commit_for_approval().is_err(),
            "a later commit cannot hide invalid snapshot changes from the hook"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn an_ordinary_approval_commit_still_runs_the_pre_commit_hook() {
        let dir = tmp_dir("approval-ordinary-hook");
        init_repository(&dir).unwrap();
        let wt = create(&dir, "praxis/approval-ordinary-hook", None, false, &|_| {})
            .unwrap()
            .0;
        install_cached_docs_rejecting_hook(&dir);
        std::fs::write(wt.path.join("ordinary.txt"), "valid change").unwrap();

        wt.commit_for_approval().unwrap();

        assert_eq!(
            run_git(&dir, &["config", "--get", "test.pre-commit-ran"])
                .unwrap()
                .trim(),
            "yes"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn approval_rejects_a_hook_that_mutates_the_worktree() {
        let dir = tmp_dir("approval-hook-mutation");
        init_repository(&dir).unwrap();
        let wt = create(&dir, "praxis/approval-hook-mutation", None, false, &|_| {})
            .unwrap()
            .0;
        install_worktree_mutating_hook(&dir);
        std::fs::write(wt.path.join("ordinary.txt"), "valid change").unwrap();
        let base_before = run_git(&dir, &["rev-parse", "HEAD"]).unwrap();

        assert!(wt.commit_for_approval().is_err());
        assert_eq!(run_git(&dir, &["rev-parse", "HEAD"]).unwrap(), base_before);
        assert_eq!(
            std::fs::read_to_string(wt.path.join("hook-mutated.txt")).unwrap(),
            "mutated\n"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn preserving_a_vanished_worktree_has_nothing_to_commit() {
        let dir = tmp_dir("preserve-orphan");
        init_repository(&dir).unwrap();
        let wt = create(&dir, "praxis/preserve-orphan", None, false, &|_| {}).unwrap().0;
        std::fs::remove_dir_all(&wt.path).unwrap();

        // 고아는 커밋할 워킹 트리가 없다 — 커밋을 시도하면 실패하고 종결이 막힌다.
        assert_eq!(wt.preserve_and_retire().unwrap(), None);
        assert!(branch_exists(&dir, "praxis/preserve-orphan"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn preserving_unregistered_residue_does_not_mutate_the_parent_repository() {
        let dir = tmp_dir("preserve-residue");
        init_repository(&dir).unwrap();
        let wt = create(&dir, "praxis/preserve-residue", None, false, &|_| {})
            .unwrap()
            .0;
        run_git(
            &dir,
            &[
                "worktree",
                "remove",
                "--force",
                wt.path.to_str().unwrap(),
            ],
        )
        .unwrap();
        std::fs::create_dir_all(&wt.path).unwrap();
        std::fs::write(wt.path.join("evidence.md"), "residual evidence").unwrap();
        std::fs::write(dir.join("staged.md"), "staged bytes").unwrap();
        run_git(&dir, &["add", "staged.md"]).unwrap();
        std::fs::write(dir.join("unstaged.md"), "unstaged bytes").unwrap();

        let head = current_revision(&dir).unwrap();
        let index = std::fs::read(dir.join(".git/index")).unwrap();
        let staged = std::fs::read(dir.join("staged.md")).unwrap();
        let unstaged = std::fs::read(dir.join("unstaged.md")).unwrap();

        assert_eq!(wt.preserve_and_retire().unwrap(), None);
        assert_eq!(current_revision(&dir).unwrap(), head);
        assert_eq!(std::fs::read(dir.join(".git/index")).unwrap(), index);
        assert_eq!(std::fs::read(dir.join("staged.md")).unwrap(), staged);
        assert_eq!(std::fs::read(dir.join("unstaged.md")).unwrap(), unstaged);
        assert_eq!(
            std::fs::read_to_string(wt.path.join("evidence.md")).unwrap(),
            "residual evidence"
        );
        assert!(branch_exists(&dir, "praxis/preserve-residue"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn preserving_residue_without_a_parent_repository_keeps_its_evidence() {
        let dir = tmp_dir("preserve-no-parent-residue-repo");
        let residue = tmp_dir("preserve-no-parent-residue");
        init_repository(&dir).unwrap();
        run_git(&dir, &["branch", "praxis/no-parent-residue"]).unwrap();
        std::fs::write(residue.join("evidence.md"), "residual evidence").unwrap();
        let wt = Worktree {
            repo: dir.clone(),
            path: residue.clone(),
            branch: "praxis/no-parent-residue".into(),
            base: current_branch(&dir).unwrap(),
            base_revision: None,
        };
        let head = current_revision(&dir).unwrap();
        let index = std::fs::read(dir.join(".git/index")).unwrap();

        assert_eq!(wt.preserve_and_retire().unwrap(), None);
        assert_eq!(current_revision(&dir).unwrap(), head);
        assert_eq!(std::fs::read(dir.join(".git/index")).unwrap(), index);
        assert_eq!(
            std::fs::read_to_string(residue.join("evidence.md")).unwrap(),
            "residual evidence"
        );
        assert!(branch_exists(&dir, "praxis/no-parent-residue"));

        std::fs::remove_dir_all(&residue).ok();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn preserving_from_a_linked_repository_uses_its_common_git_directory() {
        let dir = tmp_dir("preserve-linked-root");
        let linked = tmp_dir("preserve-linked-repository");
        std::fs::remove_dir_all(&linked).unwrap();
        init_repository(&dir).unwrap();
        run_git(
            &dir,
            &[
                "worktree",
                "add",
                "-b",
                "linked-repository",
                linked.to_str().unwrap(),
            ],
        )
        .unwrap();
        let path = linked.join(".praxis/worktrees/praxis-linked-preserve");
        run_git(
            &linked,
            &[
                "worktree",
                "add",
                "-b",
                "praxis/linked-preserve",
                path.to_str().unwrap(),
            ],
        )
        .unwrap();
        let wt = Worktree {
            repo: linked.clone(),
            path,
            branch: "praxis/linked-preserve".into(),
            base: "linked-repository".into(),
            base_revision: None,
        };
        std::fs::write(wt.path.join("evidence.md"), "evidence").unwrap();
        assert!(wt.preserve_and_retire().unwrap().is_some());
        assert!(!wt.path.exists());
        run_git(
            &dir,
            &[
                "worktree",
                "remove",
                "--force",
                linked.to_str().unwrap(),
            ],
        )
        .unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unrelated_vanished_registration_does_not_block_preservation() {
        let dir = tmp_dir("preserve-unrelated-registration");
        init_repository(&dir).unwrap();
        let stale = create(&dir, "praxis/stale-registration", None, false, &|_| {})
            .unwrap()
            .0;
        let wt = create(&dir, "praxis/valid-registration", None, false, &|_| {})
            .unwrap()
            .0;
        std::fs::remove_dir_all(&stale.path).unwrap();
        std::fs::write(wt.path.join("evidence.md"), "evidence").unwrap();

        assert!(wt.preserve_and_retire().unwrap().is_some());
        assert!(!wt.path.exists());

        let _ = run_git(&dir, &["worktree", "prune"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn preservation_rejects_malformed_worktree_metadata() {
        let dir = tmp_dir("preserve-malformed-metadata");
        init_repository(&dir).unwrap();
        let wt = create(&dir, "praxis/malformed-metadata", None, false, &|_| {})
            .unwrap()
            .0;
        std::fs::remove_file(wt.path.join(".git")).unwrap();
        assert!(wt.preserve_and_retire().is_err());
        std::fs::create_dir(wt.path.join(".git")).unwrap();

        assert!(wt.preserve_and_retire().is_err());
        assert!(wt.path.exists());
        assert!(branch_exists(&dir, "praxis/malformed-metadata"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn preservation_rejects_foreign_repository() {
        let dir = tmp_dir("preserve-identity");
        let foreign = tmp_dir("preserve-foreign");
        init_repository(&dir).unwrap();
        init_repository(&foreign).unwrap();
        let wt = create(&dir, "praxis/identity", None, false, &|_| {}).unwrap().0;
        let foreign_identity = Worktree {
            repo: foreign.clone(),
            ..wt.clone()
        };

        assert!(foreign_identity.preserve_and_retire().is_err());
        assert!(wt.path.exists());
        assert!(branch_exists(&dir, "praxis/identity"));

        std::fs::remove_dir_all(&foreign).ok();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn preservation_follows_a_branch_switched_inside_the_worktree() {
        let dir = tmp_dir("preserve-switched-branch");
        init_repository(&dir).unwrap();
        let wt = create(&dir, "praxis/switched", None, false, &|_| {}).unwrap().0;
        // 에이전트가 격리 워크트리 안에서 PR용 브랜치를 만들어 갈아타는 것은 정상 작업이다.
        // 소유권은 등록·루트·공통 git 디렉터리가 정하므로 이것이 폐기를 막으면 안 된다.
        run_git(&wt.path, &["checkout", "-b", "feature/pr"]).unwrap();
        std::fs::write(wt.path.join("evidence.md"), "근거").unwrap();

        let preserved = wt
            .preserve_and_retire()
            .unwrap()
            .expect("브랜치를 갈아탔어도 보존 커밋이 있어야 한다");

        assert_eq!(
            preserved.branch, "feature/pr",
            "커밋을 붙드는 것은 갈아탄 브랜치다 — 되찾을 좌표는 그쪽이다"
        );
        assert_eq!(
            run_git(&dir, &["show", &format!("{}:evidence.md", preserved.commit)])
                .unwrap()
                .trim(),
            "근거"
        );
        assert!(
            branch_exists(&dir, "praxis/switched"),
            "작업 브랜치는 폐기가 지우지 않는다"
        );
        assert!(branch_exists(&dir, "feature/pr"));
        assert!(!wt.path.exists(), "워크트리는 걷어내야 한다");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn preservation_anchors_a_detached_head_so_the_commit_survives() {
        let dir = tmp_dir("preserve-detached-head");
        init_repository(&dir).unwrap();
        let wt = create(&dir, "praxis/detached", None, false, &|_| {}).unwrap().0;
        std::fs::write(wt.path.join("first.md"), "first").unwrap();
        run_git(&wt.path, &["add", "-A"]).unwrap();
        run_git(&wt.path, &["commit", "-m", "first"]).unwrap();
        run_git(&wt.path, &["checkout", "--detach"]).unwrap();
        std::fs::write(wt.path.join("evidence.md"), "근거").unwrap();

        let preserved = wt
            .preserve_and_retire()
            .unwrap()
            .expect("분리 HEAD여도 보존 커밋이 있어야 한다");

        // `worktree remove`가 그 워크트리의 HEAD·reflog를 함께 지우므로, 브랜치가 붙들지
        // 않으면 이 커밋은 닿을 수 없는 dangling 객체가 된다 — 커밋해 놓고 잃는 셈이다.
        assert!(!wt.path.exists());
        assert!(
            branch_exists(&dir, &preserved.branch),
            "구조 브랜치가 실재해야 커밋에 닿을 수 있다"
        );
        assert_eq!(
            run_git(&dir, &["show", &format!("{}:evidence.md", preserved.branch)])
                .unwrap()
                .trim(),
            "근거"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn preservation_rejects_a_symlinked_worktree_path() {
        use std::os::unix::fs::symlink;

        let dir = tmp_dir("preserve-symlink");
        let alias = tmp_dir("preserve-symlink-alias");
        std::fs::remove_dir_all(&alias).unwrap();
        init_repository(&dir).unwrap();
        let wt = create(&dir, "praxis/symlink", None, false, &|_| {}).unwrap().0;
        symlink(&wt.path, &alias).unwrap();
        let symlinked = Worktree {
            path: alias.clone(),
            ..wt.clone()
        };

        assert!(symlinked.preserve_and_retire().is_err());
        assert!(wt.path.exists());
        std::fs::remove_file(&alias).unwrap();
        wt.discard().unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_failed_preservation_commit_leaves_the_worktree_alone() {
        let dir = tmp_dir("preserve-fail");
        init_repository(&dir).unwrap();
        let wt = create(&dir, "praxis/preserve-fail", None, false, &|_| {}).unwrap().0;
        std::fs::write(wt.path.join("evidence.md"), "근거").unwrap();
        // 다른 git 프로세스가 인덱스를 붙들고 있는 상황 — `add -A`가 반드시 실패한다.
        let git_dir = PathBuf::from(
            run_git(&wt.path, &["rev-parse", "--absolute-git-dir"])
                .unwrap()
                .trim(),
        );
        std::fs::write(git_dir.join("index.lock"), "").unwrap();

        assert!(
            wt.preserve_and_retire().is_err(),
            "커밋이 실패하면 에러를 올려야 한다"
        );
        assert!(
            wt.path.exists(),
            "정리하려다 근거를 태우는 것보다 작업이 검토 대기로 돌아가는 편이 낫다"
        );
        assert!(branch_exists(&dir, "praxis/preserve-fail"));

        std::fs::remove_file(git_dir.join("index.lock")).ok();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn create_fills_the_worktree_from_worktreeinclude() {
        let dir = tmp_dir("bootstrap");
        // .env 는 gitignore 대상이라 새 worktree 에 체크아웃되지 않는다 — 부트스트랩이
        // 채워야 하는 바로 그 파일이다.
        std::fs::write(dir.join(".gitignore"), ".env\n").unwrap();
        std::fs::write(dir.join(".worktreeinclude"), ".env\n").unwrap();
        std::fs::write(
            dir.join(bootstrap::SETUP_SCRIPT),
            "#!/bin/sh\ncat .env > setup-ran.txt\n",
        )
        .unwrap();
        init_repository(&dir).unwrap();
        std::fs::write(dir.join(".env"), "TOKEN=abc").unwrap();

        let wt = create(&dir, "praxis/bootstrap", None, false, &|_| {}).unwrap().0;

        assert_eq!(
            std::fs::read_to_string(wt.path.join(".env")).unwrap(),
            "TOKEN=abc",
            "worktree 가 .env 를 받아야 한다"
        );
        assert_eq!(
            std::fs::read_to_string(wt.path.join("setup-ran.txt")).unwrap(),
            "TOKEN=abc",
            "셋업 훅은 복사 이후에 돌아야 한다"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `dir`에 `name` 브랜치를 만들고 커밋 하나를 얹는다 — base 후보를 현재 체크아웃과
    /// 구분되게 하려면 각자 다른 커밋을 가리켜야 한다.
    fn branch_with_commit(dir: &Path, name: &str, file: &str) {
        let head = current_branch(dir).unwrap();
        run_git(dir, &["checkout", "-b", name]).unwrap();
        std::fs::write(dir.join(file), "x").unwrap();
        run_git(dir, &["add", "-A"]).unwrap();
        run_git(
            dir,
            &[
                "-c",
                "user.name=Praxis",
                "-c",
                "user.email=praxis@local",
                "commit",
                "-m",
                file,
            ],
        )
        .unwrap();
        run_git(dir, &["checkout", &head]).unwrap();
    }

    #[test]
    fn create_branches_from_the_requested_base() {
        let dir = tmp_dir("base");
        std::fs::write(dir.join("seed.txt"), "seed").unwrap();
        init_repository(&dir).unwrap();
        branch_with_commit(&dir, "dev", "only-on-dev.txt");

        let wt = create(&dir, "praxis/from-dev", Some("dev"), false, &|_| {}).unwrap().0;

        assert_eq!(wt.base, "dev", "base는 고른 브랜치를 그대로 기록해야 한다");
        assert!(
            wt.path.join("only-on-dev.txt").exists(),
            "worktree가 dev의 커밋에서 분기해야 한다"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn direct_checkout_switches_to_the_requested_local_branch() {
        let dir = tmp_dir("direct-checkout");
        std::fs::write(dir.join("seed.txt"), "seed").unwrap();
        init_repository(&dir).unwrap();
        branch_with_commit(&dir, "dev", "only-on-dev.txt");

        checkout_local_branch(&dir, "dev").unwrap();

        assert_eq!(current_branch(&dir).unwrap(), "dev");
        assert!(dir.join("only-on-dev.txt").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn direct_checkout_stashes_a_dirty_checkout_before_switching() {
        let dir = tmp_dir("direct-dirty");
        std::fs::write(dir.join("seed.txt"), "seed").unwrap();
        init_repository(&dir).unwrap();
        branch_with_commit(&dir, "dev", "only-on-dev.txt");
        std::fs::write(dir.join("seed.txt"), "modified").unwrap();
        std::fs::write(dir.join("uncommitted.txt"), "keep me").unwrap();

        checkout_local_branch(&dir, "dev").unwrap();

        assert_eq!(current_branch(&dir).unwrap(), "dev");
        assert_eq!(
            std::fs::read_to_string(dir.join("seed.txt")).unwrap(),
            "seed"
        );
        assert!(!dir.join("uncommitted.txt").exists());
        assert!(
            !run_git(&dir, &["stash", "list", "--format=%gs"])
                .unwrap()
                .is_empty(),
            "변경 사항은 대상 브랜치가 아니라 stash에 남아야 한다"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn direct_checkout_allows_a_dirty_checkout_when_the_branch_is_unchanged() {
        let dir = tmp_dir("direct-dirty-current");
        std::fs::write(dir.join("seed.txt"), "seed").unwrap();
        init_repository(&dir).unwrap();
        let current = current_branch(&dir).unwrap();
        std::fs::write(dir.join("uncommitted.txt"), "keep me").unwrap();

        checkout_local_branch(&dir, &current).unwrap();

        assert_eq!(current_branch(&dir).unwrap(), current);
        assert_eq!(std::fs::read_to_string(dir.join("uncommitted.txt")).unwrap(), "keep me");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn direct_checkout_reports_the_worktree_occupying_the_selected_branch() {
        let dir = tmp_dir("direct-occupied");
        std::fs::write(dir.join("seed.txt"), "seed").unwrap();
        init_repository(&dir).unwrap();
        run_git(&dir, &["branch", "dev"]).unwrap();
        let occupied = dir.join("occupied-dev");
        let occupied_str = occupied.to_string_lossy().into_owned();
        run_git(&dir, &["worktree", "add", &occupied_str, "dev"]).unwrap();

        let error = checkout_local_branch(&dir, "dev").unwrap_err().to_string();

        assert!(error.contains("다른 워크트리에서 사용 중"));
        assert!(error.contains(&occupied_str));
        run_git(&dir, &["worktree", "remove", "--force", &occupied_str]).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn create_falls_back_to_the_checked_out_branch() {
        let dir = tmp_dir("base-default");
        std::fs::write(dir.join("seed.txt"), "seed").unwrap();
        init_repository(&dir).unwrap();
        branch_with_commit(&dir, "dev", "only-on-dev.txt");
        let head = current_branch(&dir).unwrap();

        // None과 빈 문자열은 같게 다룬다 — UI가 "고르지 않음"을 빈 값으로 보낼 수 있다.
        for (i, base) in [None, Some(""), Some("  ")].into_iter().enumerate() {
            let wt = create(&dir, &format!("praxis/default-{i}"), base, false, &|_| {}).unwrap().0;
            assert_eq!(wt.base, head);
            assert!(!wt.path.join("only-on-dev.txt").exists());
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 콜백은 **거친 단계만** 순서대로 알린다. 건너뛴 단계를 보내면 UI가 일어나지 않은 일을
    /// 표시하고, 계측에는 0ms 구간이 생겨 표본이 흐려진다.
    #[test]
    fn create_reports_only_the_stages_it_runs() {
        let dir = tmp_dir("stage-callback");
        std::fs::write(dir.join("seed.txt"), "seed").unwrap();
        init_repository(&dir).unwrap();
        let stages = |branch: &str, refresh: bool| {
            let seen = std::sync::Mutex::new(Vec::new());
            create(&dir, branch, None, refresh, &|stage| {
                seen.lock().unwrap().push(stage)
            })
            .unwrap();
            seen.into_inner().unwrap()
        };

        assert_eq!(
            stages("praxis/stage-no-refresh", false),
            vec![CreateStage::Worktree, CreateStage::Bootstrap],
            "최신화가 꺼진 생성이 refresh를 알렸다"
        );
        assert_eq!(
            stages("praxis/stage-refresh", true),
            vec![
                CreateStage::Refresh,
                CreateStage::Worktree,
                CreateStage::Bootstrap
            ]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 두 범위가 실제로 다른 것을 보여줘야 한다. 같은 결과를 내면 선택기가 장식이 된다.
    #[test]
    fn the_uncommitted_range_drops_what_the_session_range_keeps() {
        let dir = tmp_dir("range");
        init_repository(&dir).unwrap();
        let wt = create_plain(&dir, "praxis/range", None).unwrap();

        std::fs::write(wt.path.join("committed.txt"), "done\n").unwrap();
        run_git(&wt.path, &["add", "-A"]).unwrap();
        run_git(
            &wt.path,
            &[
                "-c",
                "user.name=Praxis",
                "-c",
                "user.email=praxis@local",
                "commit",
                "-m",
                "committed",
            ],
        )
        .unwrap();
        std::fs::write(wt.path.join("pending.txt"), "wip\n").unwrap();

        let session = wt.diff_detailed_range(DiffRange::Session).unwrap();
        let paths = |files: &[FileDiff]| {
            files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
        };
        let session_paths = paths(&session);
        assert!(
            session_paths.contains(&"committed.txt".to_string())
                && session_paths.contains(&"pending.txt".to_string()),
            "세션 범위는 커밋 여부를 가리지 않는다: {session_paths:?}"
        );

        let uncommitted_paths = paths(&wt.diff_detailed_range(DiffRange::Uncommitted).unwrap());
        assert_eq!(
            uncommitted_paths,
            vec!["pending.txt".to_string()],
            "미커밋 범위는 커밋된 것을 빼야 한다"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 이력이 재작성돼 기준점이 조상 관계를 잃으면 근사치로 물러서되, 그 사실을 알려야 한다.
    /// 조용히 부풀린 diff를 보여주면 사용자는 왜 남의 커밋이 섞였는지 알 수 없다.
    #[test]
    fn a_rewritten_baseline_reports_degraded() {
        let dir = tmp_dir("baseline-rewritten");
        init_repository(&dir).unwrap();
        branch_with_commit(&dir, "dev", "dev.txt");
        branch_with_commit(&dir, "other", "other.txt");
        let wt = create_plain(&dir, "praxis/rewritten", Some("dev")).unwrap();
        assert_eq!(
            wt.baseline_status(),
            BaselineStatus::Pinned,
            "갓 만든 작업의 기준점은 살아 있어야 한다"
        );

        // 기준점을 포함하지 않는 이력으로 갈아탄다 — rebase가 하는 일과 같다.
        run_git(&wt.path, &["reset", "--hard", "other"]).unwrap();

        assert_eq!(wt.baseline_status(), BaselineStatus::Degraded);
        assert!(
            wt.diff_detailed().is_ok(),
            "근사치라도 diff는 나와야 한다 — 빈 화면보다 낫다"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// base 브랜치가 이 작업의 커밋을 따라잡아도 diff는 남아야 한다.
    ///
    /// PR을 머지하고 로컬 base를 당겨오면 벌어지는 일이다. 기준점이 브랜치 이름이면
    /// `merge-base`가 HEAD 쪽으로 밀려 이미 검토한 변경이 통째로 사라진다.
    #[test]
    fn diff_survives_the_base_branch_catching_up() {
        let dir = tmp_dir("base-catchup");
        init_repository(&dir).unwrap();
        branch_with_commit(&dir, "dev", "dev.txt");
        let wt = create_plain(&dir, "praxis/catchup", Some("dev")).unwrap();

        std::fs::write(wt.path.join("new.txt"), "work\n").unwrap();
        run_git(&wt.path, &["add", "-A"]).unwrap();
        run_git(
            &wt.path,
            &[
                "-c",
                "user.name=Praxis",
                "-c",
                "user.email=praxis@local",
                "commit",
                "-m",
                "work",
            ],
        )
        .unwrap();

        // base 브랜치를 이 작업의 커밋까지 전진시킨다 — 머지 후 pull과 같은 상태다.
        let head = run_git(&wt.path, &["rev-parse", "HEAD"]).unwrap();
        run_git(&dir, &["branch", "-f", "dev", head.trim()]).unwrap();

        let files = wt.diff_detailed().unwrap();
        assert!(
            files.iter().any(|file| file.path == "new.txt"),
            "base가 따라잡아도 이 작업의 변경은 diff에 남아야 한다 (남은 파일: {:?})",
            files.iter().map(|f| &f.path).collect::<Vec<_>>()
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn create_rejects_a_base_that_is_not_a_local_branch() {
        let dir = tmp_dir("base-missing");
        init_repository(&dir).unwrap();

        let err = create(&dir, "praxis/nope", Some("origin/dev"), false, &|_| {})
            .unwrap_err()
            .to_string();

        assert!(err.contains("찾을 수 없습니다"), "{err}");
        // 거부했으면 브랜치도 worktree도 만들지 않은 상태여야 한다.
        assert!(!local_branch_exists(&dir, "praxis/nope"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn local_branches_are_listed_without_remotes() {
        let dir = tmp_dir("branches");
        init_repository(&dir).unwrap();
        branch_with_commit(&dir, "dev", "d.txt");
        let head = current_branch(&dir).unwrap();

        let branches = list_local_branches(&dir).unwrap();

        assert!(branches.contains(&"dev".to_string()));
        assert!(branches.contains(&head));
        assert!(
            branches.iter().all(|b| !b.starts_with("origin/")),
            "{branches:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn approval_merge_uses_the_saved_base_when_the_repository_is_switched_and_dirty() {
        let dir = tmp_dir("base-guard");
        std::fs::write(dir.join("seed.txt"), "seed").unwrap();
        init_repository(&dir).unwrap();
        branch_with_commit(&dir, "dev", "only-on-dev.txt");
        let wt = create(&dir, "praxis/guarded", Some("dev"), false, &|_| {})
            .unwrap()
            .0;
        std::fs::write(wt.path.join("work.txt"), "done").unwrap();
        let commit = wt.commit_for_approval().unwrap();
        run_git(&dir, &["checkout", "dev"]).unwrap();
        std::fs::write(dir.join("base-later.txt"), "base\n").unwrap();
        run_git(&dir, &["add", "base-later.txt"]).unwrap();
        run_git(&dir, &["commit", "-m", "base later"]).unwrap();
        branch_with_commit(&dir, "feature", "feature-only.txt");
        run_git(&dir, &["checkout", "feature"]).unwrap();
        std::fs::write(dir.join("staged.txt"), "keep\n").unwrap();
        run_git(&dir, &["add", "staged.txt"]).unwrap();
        std::fs::write(dir.join("untracked.txt"), "keep\n").unwrap();
        let head = current_revision(&dir).unwrap();
        let status = run_git(&dir, &["status", "--porcelain=v1", "-z"]).unwrap();

        wt.merge_for_approval(&commit).unwrap();
        assert_eq!(run_git(&dir, &["show", "dev:work.txt"]).unwrap(), "done");
        assert!(!run_git(&dir, &["rev-list", "--merges", "dev"]).unwrap().is_empty());
        assert_eq!(current_revision(&dir).unwrap(), head);
        assert_eq!(run_git(&dir, &["status", "--porcelain=v1", "-z"]).unwrap(), status);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn approval_merge_rejects_a_missing_saved_base_without_using_head() {
        let dir = tmp_dir("base-missing");
        init_repository(&dir).unwrap();
        branch_with_commit(&dir, "dev", "only-on-dev.txt");
        let wt = create(&dir, "praxis/missing", Some("dev"), false, &|_| {})
            .unwrap()
            .0;
        std::fs::write(wt.path.join("work.txt"), "done").unwrap();
        let commit = wt.commit_for_approval().unwrap();
        run_git(&dir, &["branch", "-D", "dev"]).unwrap();

        let error = wt.merge_for_approval(&commit).unwrap_err().to_string();
        assert!(error.contains("현재 HEAD로 대체하지 않았습니다"), "{error}");
        assert!(!dir.join("work.txt").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn approval_merge_reports_the_worktree_holding_the_saved_base() {
        let dir = tmp_dir("base-holder");
        init_repository(&dir).unwrap();
        branch_with_commit(&dir, "dev", "only-on-dev.txt");
        let holder = dir.join("holder");
        run_git(&dir, &["worktree", "add", holder.to_str().unwrap(), "dev"]).unwrap();
        let wt = create(&dir, "praxis/held", Some("dev"), false, &|_| {})
            .unwrap()
            .0;
        std::fs::write(wt.path.join("work.txt"), "done").unwrap();
        let commit = wt.commit_for_approval().unwrap();

        let error = wt.merge_for_approval(&commit).unwrap_err().to_string();
        assert!(error.contains(holder.to_string_lossy().as_ref()), "{error}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn approval_merge_allows_the_matching_base() {
        let dir = tmp_dir("base-guard-ok");
        std::fs::write(dir.join("seed.txt"), "seed").unwrap();
        init_repository(&dir).unwrap();
        let wt = create(&dir, "praxis/plain", None, false, &|_| {}).unwrap().0;
        std::fs::write(wt.path.join("work.txt"), "done").unwrap();
        let commit = wt.commit_for_approval().unwrap();

        wt.merge_for_approval(&commit).unwrap();

        assert!(dir.join("work.txt").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn init_is_idempotent_on_an_existing_repository() {
        let dir = tmp_dir("idem");
        init_repository(&dir).unwrap();
        let head = current_branch(&dir).unwrap();
        // 두 번째 호출은 커밋을 더 만들지 않는다.
        init_repository(&dir).unwrap();
        assert_eq!(current_branch(&dir).unwrap(), head);
        let log = run_git(&dir, &["rev-list", "--count", "HEAD"]).unwrap();
        assert_eq!(log.trim(), "1");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn init_adopts_a_repository_that_has_no_commit_yet() {
        let dir = tmp_dir("unborn");
        std::fs::write(dir.join("note.txt"), "hello").unwrap();
        // 사용자가 `git init`만 해 둔 상태 — 저장소이지만 HEAD가 가리킬 커밋이 없다.
        run_git(&dir, &["init"]).unwrap();
        assert!(is_git_repository(&dir));
        assert!(
            current_revision(&dir).is_err(),
            "이 상태에서 rev-parse HEAD가 실패하는 것이 이 테스트의 전제다"
        );

        init_repository(&dir).unwrap();

        // 전제가 채워져야 diff 기준점·체크포인트·격리가 모두 성립한다.
        assert!(current_revision(&dir).is_ok());
        let wt = create(&dir, "praxis/after-adopt", None, false, &|_| {}).unwrap().0;
        assert_eq!(
            std::fs::read_to_string(wt.path.join("note.txt")).unwrap(),
            "hello"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_repository_without_a_commit_is_not_ready() {
        let dir = tmp_dir("ready");
        assert!(!is_ready_repository(&dir), "저장소가 아니면 준비 전이다");

        run_git(&dir, &["init"]).unwrap();
        assert!(is_git_repository(&dir), "git 명령은 쓸 수 있는 상태다");
        assert!(
            !is_ready_repository(&dir),
            "그래도 커밋이 없으면 격리도 diff도 설 자리가 없다 — 이 구분이 UI 안내를 가른다"
        );

        init_repository(&dir).unwrap();
        assert!(is_ready_repository(&dir));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn init_keeps_the_configured_identity_on_an_adopted_repository() {
        let dir = tmp_dir("identity");
        run_git(&dir, &["init"]).unwrap();
        run_git(&dir, &["config", "user.name", "Someone"]).unwrap();
        run_git(&dir, &["config", "user.email", "someone@example.com"]).unwrap();
        std::fs::write(dir.join("note.txt"), "hello").unwrap();

        init_repository(&dir).unwrap();

        // 사용자 저장소의 첫 커밋 author를 Dojang으로 덮어쓰지 않는다.
        let author = run_git(&dir, &["log", "-1", "--format=%ae"]).unwrap();
        assert_eq!(author.trim(), "someone@example.com");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn init_does_not_apply_the_file_limit_to_an_existing_repository() {
        let dir = tmp_dir("limit-existing");
        run_git(&dir, &["init"]).unwrap();
        let heavy = dir.join("heavy");
        std::fs::create_dir_all(&heavy).unwrap();
        for i in 0..=INIT_FILE_LIMIT {
            std::fs::write(heavy.join(format!("f{i}")), "x").unwrap();
        }
        // 상한은 "남의 폴더에 거대 커밋을 만들지 않는다"는 규칙이다. 이미 저장소라면 커밋 여부는
        // 사용자가 이미 택한 것이므로, 여기서 막으면 작업 자체를 시작할 수 없다.
        init_repository(&dir).unwrap();
        assert!(has_commit(&dir));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn init_refuses_a_directory_with_too_many_files() {
        let dir = tmp_dir("limit");
        let heavy = dir.join("heavy");
        std::fs::create_dir_all(&heavy).unwrap();
        for i in 0..=INIT_FILE_LIMIT {
            std::fs::write(heavy.join(format!("f{i}")), "x").unwrap();
        }
        let err = init_repository(&dir).unwrap_err().to_string();
        assert!(err.contains("자동 초기화하지 않았습니다"), "{err}");
        // 거부했으면 저장소를 만들지 않은 상태 그대로여야 한다.
        assert!(!is_git_repository(&dir));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn init_rejects_a_non_directory() {
        let dir = tmp_dir("file");
        let file = dir.join("f.txt");
        std::fs::write(&file, "x").unwrap();
        assert!(init_repository(&file).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn count_stops_at_the_limit() {
        let dir = tmp_dir("count");
        for i in 0..5 {
            std::fs::write(dir.join(format!("f{i}")), "x").unwrap();
        }
        assert_eq!(count_files_up_to(&dir, 10), 5);
        // 상한을 넘으면 정확한 총계가 아니라 "넘었다"만 알 수 있으면 된다.
        assert!(count_files_up_to(&dir, 2) > 2);
        std::fs::remove_dir_all(&dir).ok();
    }
}
