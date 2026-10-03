//! 머지 충돌 해소 — 충돌을 **worktree 안에 가둔다**.
//!
//! `merge_for_approval`은 저장된 base를 체크아웃한 원본 또는 승인 전용 worktree에서 머지한다.
//! 충돌 후 이번 merge를 abort해도 충돌 원인이 그대로라 **재시도는 같은 실패를 반복한다**.
//!
//! 여기서는 방향을 뒤집는다 — worktree에서 base를 머지해(`git merge <base>`) 충돌을 worktree에
//! 가두고, 해소한 뒤 승인 대상에서는 fast-forward할 수 있게 만든다. 해소 세션의 `MERGE_HEAD`는
//! 작업 worktree에만 남긴다. 원본은 사용자의 작업 공간이므로 해소 중에도 다른 작업을 계속할 수 있다.
//!
//! 방향을 뒤집은 부수 효과로 라벨이 직관과 맞는다 — `ours`가 **작업 쪽**, `theirs`가 **base 쪽**이다
//! (repo에서 머지하면 정반대가 된다).
//!
//! 세션 상태는 DB에 두지 않는다. worktree의 `MERGE_HEAD`가 곧 "세션이 열려 있다"이므로,
//! DB와 git이 어긋날 여지 자체가 없다. 앱이 죽어도 재시작 후 그대로 이어진다.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{run_git, Worktree};

/// 충돌 파일 하나.
///
/// 세 스테이지 모두 없을 수 있다 — both-added면 base가 없고, 삭제/수정 충돌이면 한쪽이 없다.
/// `None`과 빈 문자열은 다르다: 전자는 "그 쪽에 파일이 없음", 후자는 "빈 파일".
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConflictFile {
    pub path: String,
    /// 스테이지 2 — **작업(worktree) 쪽**. 역방향 머지라 ours가 작업이다.
    pub ours: Option<String>,
    /// 스테이지 3 — **base 쪽**.
    pub theirs: Option<String>,
    /// 스테이지 1 — 공통 조상.
    pub base: Option<String>,
    /// ours→theirs unified diff. 프론트가 **다른 줄만** 짚어 보여주는 데 쓴다.
    ///
    /// 전문 두 벌을 나란히 놓으면 어디가 다른지는 사람 눈이 찾아야 한다. 실제 차이가 한 줄인데
    /// 파일이 수백 줄이면 그 비교는 사실상 불가능하고, 그 상태로 채택 버튼을 누르는 것은
    /// 확인이 아니라 도박이다.
    ///
    /// 한쪽이 없으면(삭제/수정 충돌) `None`이다 — 비교 상대가 없으니 전문을 그대로 보여주는 것이 맞다.
    pub patch: Option<String>,
}

/// 파일 하나를 어떻게 해소할지.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind", content = "body")]
pub enum Resolution {
    /// 작업 쪽을 채택.
    Ours,
    /// base 쪽을 채택.
    Theirs,
    /// 양쪽을 모두 남긴다(`merge-file --union`). 원장처럼 덧붙이는 파일용.
    Union,
    /// 사용자가 직접 쓴 본문.
    Manual(String),
}

/// 해소 본문 상한. 프론트에서 오는 `Manual` 본문이 무한정 커지는 것을 막는다.
/// `fsapi`의 파일 쓰기 캡(2MB)과 같은 값이다.
const MAX_MANUAL_BYTES: usize = 2 * 1024 * 1024;

impl Worktree {
    /// 이 worktree에 머지 세션이 열려 있는가 — `MERGE_HEAD`의 존재가 곧 진실이다.
    pub fn conflict_session_open(&self) -> bool {
        self.worktree_git_dir()
            .map(|dir| dir.join("MERGE_HEAD").exists())
            .unwrap_or(false)
    }

    /// worktree의 실제 git 디렉터리. worktree에서 `.git`은 디렉터리가 아니라 파일이므로
    /// 경로를 조립하지 않고 git에게 묻는다.
    fn worktree_git_dir(&self) -> Option<PathBuf> {
        run_git(&self.path, &["rev-parse", "--absolute-git-dir"])
            .ok()
            .map(|out| PathBuf::from(out.trim()))
    }

    /// 중단 시 되돌아갈 체크포인트를 적어 두는 곳. repo 쪽 `.git` 아래에 둔다 —
    /// worktree의 `.git`은 파일이라 하위 경로를 만들 수 없다.
    fn conflict_checkpoint_path(&self) -> PathBuf {
        // 브랜치명에 '/'가 있으면 경로가 갈라진다(feature/JH2-58-…). 파일 하나로 눕힌다.
        let flat = self.branch.replace(['/', '\\'], "_");
        self.repo
            .join(".git")
            .join("praxis-conflict")
            .join(flat)
    }

    fn write_conflict_checkpoint(&self, sha: &str) -> anyhow::Result<()> {
        let path = self.conflict_checkpoint_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, sha)?;
        Ok(())
    }

    fn read_conflict_checkpoint(&self) -> anyhow::Result<String> {
        let path = self.conflict_checkpoint_path();
        let sha = std::fs::read_to_string(&path).map_err(|e| {
            anyhow::anyhow!(
                "충돌 해소 체크포인트를 찾을 수 없습니다({}): {e}",
                path.display()
            )
        })?;
        Ok(sha.trim().to_string())
    }

    fn clear_conflict_checkpoint(&self) {
        let _ = std::fs::remove_file(self.conflict_checkpoint_path());
    }

    /// 승인 머지가 충돌할지 **미리** 본다. 실제로 머지하지 않으므로 어느 쪽도 건드리지 않는다.
    ///
    /// `task_approve`가 실패했을 때 "충돌이라 해소 가능"인지 "그 외 실패"인지 가르는 데 쓴다.
    /// `git merge-tree --write-tree`는 충돌 시 exit 1과 함께 파일 목록을 stdout에 준다 —
    /// 실패를 에러로 바꾸는 `run_git`을 쓰지 않고 직접 실행하는 이유다.
    pub fn conflicting_paths_against_base(&self) -> anyhow::Result<Vec<String>> {
        if self.is_direct() {
            return Ok(Vec::new());
        }
        let head = run_git(&self.path, &["rev-parse", "HEAD"])?.trim().to_string();
        let out = std::process::Command::new("git")
            .current_dir(&self.repo)
            .args([
                "merge-tree",
                "--write-tree",
                "--name-only",
                &super::approval_merge::base_ref(self)?,
                &head,
            ])
            .output()?;
        if out.status.success() {
            return Ok(Vec::new());
        }
        if out.status.code() != Some(1) {
            anyhow::bail!("병합 충돌을 확인할 수 없습니다: {}", String::from_utf8_lossy(&out.stderr));
        }
        // 출력 형식: <tree oid>\n<충돌 파일들…>\n\n<메시지>
        // 첫 줄(oid)을 버리고 빈 줄 전까지가 파일 목록이다.
        let stdout = String::from_utf8_lossy(&out.stdout);
        Ok(stdout
            .lines()
            .skip(1)
            .take_while(|line| !line.trim().is_empty())
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect())
    }

    /// 충돌 해소 세션을 연다.
    ///
    /// ① 체크포인트 ② worktree에서 base를 역방향 머지 ③ 충돌 파일 수집.
    /// 이미 세션이 열려 있으면(앱 재시작 등) 머지를 다시 걸지 않고 현재 상태를 그대로 읽는다.
    ///
    /// 머지가 충돌 없이 끝나도 `--no-commit`이라 세션은 열린 채 남는다 —
    /// [`finish_conflict_resolution`](Self::finish_conflict_resolution)이 일관되게 커밋한다.
    pub fn begin_conflict_resolution(&self) -> anyhow::Result<Vec<ConflictFile>> {
        if self.conflict_session_open() {
            return self.collect_conflicts();
        }
        if self.is_direct() {
            anyhow::bail!("직접 실행 작업은 머지 대상이 아니라 충돌 해소가 필요 없습니다");
        }
        let base = super::approval_merge::base_ref(self)?;
        if !super::local_branch_exists(&self.repo, &self.base) {
            anyhow::bail!(
                "base 브랜치 '{}'가 없어 충돌을 해소할 수 없습니다",
                self.base
            );
        }
        let checkpoint = self.checkpoint_commit("dojang: pre-conflict")?;
        self.write_conflict_checkpoint(&checkpoint)?;
        // 충돌은 정상 경로다 — exit code로 실패를 가리지 않는다.
        let _ = run_git(&self.path, &["merge", "--no-commit", "--no-ff", &base]);
        if !self.conflict_session_open() {
            // 머지가 아예 시작되지 않았다(이미 최신 등). 되돌리고 알린다.
            self.clear_conflict_checkpoint();
            anyhow::bail!("base를 머지할 것이 없습니다 — 충돌 해소로 풀 수 있는 상태가 아닙니다");
        }
        self.collect_conflicts()
    }

    /// 아직 해소되지 않은 충돌 파일 경로.
    pub fn unresolved_conflict_paths(&self) -> anyhow::Result<Vec<String>> {
        let unmerged = run_git(&self.path, &["diff", "--name-only", "--diff-filter=U"])?;
        Ok(unmerged
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect())
    }

    fn collect_conflicts(&self) -> anyhow::Result<Vec<ConflictFile>> {
        let paths = self.unresolved_conflict_paths()?;
        Ok(paths
            .into_iter()
            .map(|path| {
                let ours = self.stage_content(2, &path);
                let theirs = self.stage_content(3, &path);
                ConflictFile {
                    base: self.stage_content(1, &path),
                    patch: self.conflict_patch(ours.as_deref(), theirs.as_deref()),
                    ours,
                    theirs,
                    path,
                }
            })
            .collect())
    }

    /// index 스테이지의 내용. 그 스테이지가 없으면 `None`.
    fn stage_content(&self, stage: u8, path: &str) -> Option<String> {
        run_git(&self.path, &["show", &format!(":{stage}:{path}")]).ok()
    }

    /// 두 스테이지를 임시 파일로 꺼내 unified diff를 뜬다.
    ///
    /// `--no-index`는 **차이가 있으면 exit 1**을 내므로 실패를 에러로 바꾸는 `run_git`을 쓸 수 없다
    /// (`conflicting_paths_against_base`와 같은 사정이다). 실패해도 삼키고 `None`을 준다 —
    /// patch는 보기 좋게 만드는 부가 정보일 뿐이고, 이것 때문에 해소 자체가 막히면 본말전도다.
    ///
    /// 출력은 **프론트가 파싱하는 데이터**이지 사람이 읽을 화면이 아니다. 그래서 사용자의 git
    /// 설정이 형식을 바꾸지 못하게 막는다 — `diff.external`(delta·difftastic 등)이 걸려 있으면
    /// unified diff가 아예 아닌 것이 나오고, `color.ui = always`면 ANSI 코드가 섞여 파서가 깨진다.
    /// 둘 다 흔한 설정이라 기본값을 믿을 수 없다.
    fn conflict_patch(&self, ours: Option<&str>, theirs: Option<&str>) -> Option<String> {
        let (ours, theirs) = (ours?, theirs?);

        // 브랜치별로 가른다 — 같은 repo의 worktree 둘이 동시에 세션을 열면 고정 경로는 서로를
        // 덮어쓴다. 체크포인트 경로와 같은 이유로 '/'를 눕힌다(feature/JH2-58-…).
        let flat = self.branch.replace(['/', '\\'], "_");
        let tmp = self.repo.join(".git").join("praxis-conflict-diff").join(flat);
        std::fs::create_dir_all(&tmp).ok()?;
        let ours_path = tmp.join("ours");
        let theirs_path = tmp.join("theirs");

        let patch = (|| -> Option<String> {
            std::fs::write(&ours_path, ours).ok()?;
            std::fs::write(&theirs_path, theirs).ok()?;
            let out = std::process::Command::new("git")
                .current_dir(&self.repo)
                .args([
                    "diff",
                    "--no-ext-diff",
                    "--no-color",
                    "--no-index",
                    ours_path.to_str()?,
                    theirs_path.to_str()?,
                ])
                .output()
                .ok()?;
            let stdout = String::from_utf8_lossy(&out.stdout);
            // 헤더(`diff --git`·`---`·`+++`)에는 임시 경로가 박혀 있다. 프론트가 무시하긴 하지만
            // 내보낼 이유가 없으므로 첫 hunk부터 자른다. `@@`가 없으면 차이가 없다는 뜻이다.
            let start = stdout.find("@@")?;
            Some(stdout[start..].to_string())
        })();

        // 성공하든 아니든 임시 파일은 남기지 않는다. 지우는 것은 이 브랜치 몫뿐이다.
        let _ = std::fs::remove_dir_all(&tmp);
        patch
    }

    /// 파일 하나를 해소하고 stage한다.
    ///
    /// 경로는 **충돌 목록에 있는 것만** 받는다. 프론트에서 온 문자열로 경로를 조립하지 않으므로
    /// 트래버설이 원천 차단된다(`skills` 모듈의 스캔 목록 매칭과 같은 관행).
    pub fn resolve_conflict(&self, path: &str, resolution: &Resolution) -> anyhow::Result<()> {
        if !self.conflict_session_open() {
            anyhow::bail!("충돌 해소 세션이 열려 있지 않습니다");
        }
        let unresolved = self.unresolved_conflict_paths()?;
        if !unresolved.iter().any(|p| p == path) {
            anyhow::bail!("충돌 목록에 없는 경로입니다: {path}");
        }
        let content = match resolution {
            Resolution::Ours => self
                .stage_content(2, path)
                .ok_or_else(|| anyhow::anyhow!("작업 쪽에 '{path}'가 없습니다"))?,
            Resolution::Theirs => self
                .stage_content(3, path)
                .ok_or_else(|| anyhow::anyhow!("base 쪽에 '{path}'가 없습니다"))?,
            Resolution::Union => self.union_merge(path)?,
            Resolution::Manual(body) => {
                if body.len() > MAX_MANUAL_BYTES {
                    anyhow::bail!("해소 본문이 2MB 한도를 초과합니다 ({}바이트)", body.len());
                }
                body.clone()
            }
        };
        std::fs::write(self.path.join(path), content)?;
        run_git(&self.path, &["add", path])?;
        Ok(())
    }

    /// 양쪽을 모두 남기는 3-way union. 스테이지 셋을 임시 파일로 꺼내 `merge-file --union`에 건다.
    fn union_merge(&self, path: &str) -> anyhow::Result<String> {
        let ours = self
            .stage_content(2, path)
            .ok_or_else(|| anyhow::anyhow!("작업 쪽에 '{path}'가 없어 union할 수 없습니다"))?;
        let theirs = self
            .stage_content(3, path)
            .ok_or_else(|| anyhow::anyhow!("base 쪽에 '{path}'가 없어 union할 수 없습니다"))?;
        // both-added면 공통 조상이 없다 — 빈 파일을 조상으로 삼으면 양쪽이 모두 추가분이 된다.
        let base = self.stage_content(1, path).unwrap_or_default();

        let tmp = self.repo.join(".git").join("praxis-conflict-union");
        std::fs::create_dir_all(&tmp)?;
        let ours_path = tmp.join("ours");
        let base_path = tmp.join("base");
        let theirs_path = tmp.join("theirs");
        std::fs::write(&ours_path, &ours)?;
        std::fs::write(&base_path, &base)?;
        std::fs::write(&theirs_path, &theirs)?;

        let merge_result = run_git(
            &self.repo,
            &[
                "merge-file",
                "--union",
                ours_path.to_str().unwrap_or_default(),
                base_path.to_str().unwrap_or_default(),
                theirs_path.to_str().unwrap_or_default(),
            ],
        );
        let merged = std::fs::read_to_string(&ours_path);
        let _ = std::fs::remove_dir_all(&tmp);
        merge_result?;
        Ok(merged?)
    }

    /// 세션을 닫고 머지 커밋을 만든다. 미해결이 남아 있으면 그 목록과 함께 거부한다.
    ///
    /// 이후 repo에서의 재머지는 fast-forward가 된다 — base가 이미 이 커밋의 조상이기 때문이다.
    pub fn finish_conflict_resolution(&self) -> anyhow::Result<String> {
        if !self.conflict_session_open() {
            anyhow::bail!("충돌 해소 세션이 열려 있지 않습니다");
        }
        let remaining = self.unresolved_conflict_paths()?;
        if !remaining.is_empty() {
            anyhow::bail!("아직 해소되지 않은 파일이 있습니다: {}", remaining.join(", "));
        }
        run_git(&self.path, &["commit", "--no-edit"])?;
        let head = run_git(&self.path, &["rev-parse", "HEAD"])?
            .trim()
            .to_string();
        self.clear_conflict_checkpoint();
        Ok(head)
    }

    /// 세션을 버리고 체크포인트로 원복한다. 체크포인트 기록이 없으면 머지만 되돌린다 —
    /// 그 경우에도 worktree를 머지 이전 상태로 되돌리는 것이 `merge --abort`의 역할이다.
    pub fn abort_conflict_resolution(&self) -> anyhow::Result<()> {
        let checkpoint = self.read_conflict_checkpoint().ok();
        let _ = run_git(&self.path, &["merge", "--abort"]);
        if let Some(sha) = checkpoint {
            self.restore_to_checkpoint(&sha)?;
        }
        self.clear_conflict_checkpoint();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn git(dir: &Path, args: &[&str]) -> String {
        run_git(dir, args).unwrap_or_else(|e| panic!("git {args:?} 실패: {e}"))
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = crate::testtmp::dir().join(format!("praxis-conflict-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// base 브랜치와 작업 worktree가 **같은 줄**을 다르게 고친 상태를 만든다.
    fn repo_with_conflict(tag: &str) -> (PathBuf, Worktree) {
        let repo = tmp_dir(tag);
        super::super::init_repository(&repo).unwrap();
        std::fs::write(repo.join("shared.txt"), "원본\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-m", "base"]);

        let base = super::super::current_branch(&repo).unwrap();
        let wt = super::super::create(&repo, "task-branch", Some(&base), false, &|_| {})
            .unwrap()
            .0;

        // 작업 쪽 변경
        std::fs::write(wt.path.join("shared.txt"), "작업이 고친 줄\n").unwrap();
        git(&wt.path, &["add", "-A"]);
        git(&wt.path, &["commit", "-m", "작업 변경"]);

        // base 쪽 변경 — 같은 줄
        std::fs::write(repo.join("shared.txt"), "base가 고친 줄\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-m", "base 변경"]);

        (repo, wt)
    }

    fn repo_merge_head_exists(repo: &Path) -> bool {
        repo.join(".git").join("MERGE_HEAD").exists()
    }

    #[cfg(unix)]
    fn install_rejecting_pre_commit(repo: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let hooks = repo.join("test-hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("pre-commit");
        std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        git(repo, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
        hooks
    }

    #[test]
    fn conflict_is_detected_before_any_merge_happens() {
        let (repo, wt) = repo_with_conflict("detect");
        let paths = wt.conflicting_paths_against_base().unwrap();
        assert_eq!(paths, vec!["shared.txt".to_string()]);
        // 미리보기는 어느 쪽도 건드리지 않는다.
        assert!(!repo_merge_head_exists(&repo));
        assert!(!wt.conflict_session_open());
    }

    #[test]
    fn ours_is_the_task_side_and_theirs_is_the_base_side() {
        let (_repo, wt) = repo_with_conflict("labels");
        let conflicts = wt.begin_conflict_resolution().unwrap();
        assert_eq!(conflicts.len(), 1);
        let file = &conflicts[0];
        assert_eq!(file.path, "shared.txt");
        assert_eq!(file.ours.as_deref(), Some("작업이 고친 줄\n"));
        assert_eq!(file.theirs.as_deref(), Some("base가 고친 줄\n"));
        assert_eq!(file.base.as_deref(), Some("원본\n"));
    }

    #[cfg(unix)]
    #[test]
    fn conflict_checkpoint_bypasses_a_rejecting_pre_commit_hook() {
        let (repo, wt) = repo_with_conflict("checkpoint-hook");
        let hooks = install_rejecting_pre_commit(&repo);
        std::fs::write(wt.path.join("uncommitted.txt"), "snapshot me").unwrap();

        assert!(wt.begin_conflict_resolution().is_ok());
        assert_eq!(
            git(&repo, &["config", "--get", "core.hooksPath"]).trim(),
            hooks.to_str().unwrap()
        );
    }

    #[test]
    fn the_repo_never_enters_a_merge_state() {
        let (repo, wt) = repo_with_conflict("repo-clean");
        wt.begin_conflict_resolution().unwrap();
        assert!(!repo_merge_head_exists(&repo), "세션 중 repo가 머지 상태");
        wt.resolve_conflict("shared.txt", &Resolution::Ours).unwrap();
        assert!(!repo_merge_head_exists(&repo), "해소 중 repo가 머지 상태");
        wt.finish_conflict_resolution().unwrap();
        assert!(!repo_merge_head_exists(&repo), "완료 후 repo가 머지 상태");
    }

    #[test]
    fn resolving_with_ours_keeps_the_task_content() {
        let (_repo, wt) = repo_with_conflict("take-ours");
        wt.begin_conflict_resolution().unwrap();
        wt.resolve_conflict("shared.txt", &Resolution::Ours).unwrap();
        wt.finish_conflict_resolution().unwrap();
        let content = std::fs::read_to_string(wt.path.join("shared.txt")).unwrap();
        assert_eq!(content, "작업이 고친 줄\n");
        assert!(!wt.conflict_session_open());
    }

    #[test]
    fn resolving_with_theirs_keeps_the_base_content() {
        let (_repo, wt) = repo_with_conflict("take-theirs");
        wt.begin_conflict_resolution().unwrap();
        wt.resolve_conflict("shared.txt", &Resolution::Theirs)
            .unwrap();
        wt.finish_conflict_resolution().unwrap();
        let content = std::fs::read_to_string(wt.path.join("shared.txt")).unwrap();
        assert_eq!(content, "base가 고친 줄\n");
    }

    #[test]
    fn manual_resolution_writes_the_given_body() {
        let (_repo, wt) = repo_with_conflict("manual");
        wt.begin_conflict_resolution().unwrap();
        wt.resolve_conflict(
            "shared.txt",
            &Resolution::Manual("사람이 합친 줄\n".to_string()),
        )
        .unwrap();
        wt.finish_conflict_resolution().unwrap();
        let content = std::fs::read_to_string(wt.path.join("shared.txt")).unwrap();
        assert_eq!(content, "사람이 합친 줄\n");
    }

    #[test]
    fn union_keeps_both_sides() {
        let (_repo, wt) = repo_with_conflict("union");
        wt.begin_conflict_resolution().unwrap();
        wt.resolve_conflict("shared.txt", &Resolution::Union).unwrap();
        wt.finish_conflict_resolution().unwrap();
        let content = std::fs::read_to_string(wt.path.join("shared.txt")).unwrap();
        assert!(content.contains("작업이 고친 줄"), "작업 쪽이 사라짐: {content}");
        assert!(content.contains("base가 고친 줄"), "base 쪽이 사라짐: {content}");
    }

    #[test]
    fn finish_refuses_while_something_is_unresolved() {
        let (_repo, wt) = repo_with_conflict("refuse");
        wt.begin_conflict_resolution().unwrap();
        let err = wt.finish_conflict_resolution().unwrap_err().to_string();
        assert!(err.contains("shared.txt"), "미해결 파일명이 없음: {err}");
    }

    #[test]
    fn abort_restores_the_worktree_and_closes_the_session() {
        let (_repo, wt) = repo_with_conflict("abort");
        let before = git(&wt.path, &["rev-parse", "HEAD"]).trim().to_string();
        wt.begin_conflict_resolution().unwrap();
        wt.resolve_conflict("shared.txt", &Resolution::Theirs)
            .unwrap();
        wt.abort_conflict_resolution().unwrap();

        assert!(!wt.conflict_session_open());
        assert_eq!(git(&wt.path, &["rev-parse", "HEAD"]).trim(), before);
        let content = std::fs::read_to_string(wt.path.join("shared.txt")).unwrap();
        assert_eq!(content, "작업이 고친 줄\n", "작업 내용이 복원되지 않음");
    }

    #[test]
    fn an_open_session_is_recovered_instead_of_restarted() {
        let (_repo, wt) = repo_with_conflict("recover");
        wt.begin_conflict_resolution().unwrap();
        // 앱 재시작을 흉내 — 같은 worktree로 핸들을 다시 만든다.
        let reopened = Worktree {
            repo: wt.repo.clone(),
            path: wt.path.clone(),
            branch: wt.branch.clone(),
            base: wt.base.clone(),
            base_revision: wt.base_revision.clone(),
        };
        assert!(reopened.conflict_session_open());
        let conflicts = reopened.begin_conflict_resolution().unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].path, "shared.txt");
    }

    #[test]
    fn resolve_rejects_a_path_outside_the_conflict_list() {
        let (_repo, wt) = repo_with_conflict("traversal");
        wt.begin_conflict_resolution().unwrap();
        let err = wt
            .resolve_conflict("../escape.txt", &Resolution::Manual("x".into()))
            .unwrap_err()
            .to_string();
        assert!(err.contains("충돌 목록에 없는"), "예상과 다른 거부: {err}");
    }

    #[test]
    fn resolve_is_rejected_without_an_open_session() {
        let (_repo, wt) = repo_with_conflict("no-session");
        let err = wt
            .resolve_conflict("shared.txt", &Resolution::Ours)
            .unwrap_err()
            .to_string();
        assert!(err.contains("세션이 열려 있지 않"), "예상과 다른 거부: {err}");
    }

    #[test]
    fn the_approval_diff_does_not_absorb_base_changes() {
        let (_repo, wt) = repo_with_conflict("diff-clean");
        // base에만 있는 무관한 파일 — 머지로 worktree에 들어오지만 승인 diff에는 없어야 한다.
        std::fs::write(wt.repo.join("unrelated.txt"), "base만의 파일\n").unwrap();
        git(&wt.repo, &["add", "-A"]);
        git(&wt.repo, &["commit", "-m", "base 무관 변경"]);

        wt.begin_conflict_resolution().unwrap();
        wt.resolve_conflict("shared.txt", &Resolution::Ours).unwrap();
        wt.finish_conflict_resolution().unwrap();

        let changed = wt.changed_paths().unwrap();
        assert!(
            !changed.iter().any(|p| p == "unrelated.txt"),
            "base 변경분이 승인 diff에 섞였다: {changed:?}"
        );
    }

    #[test]
    fn the_patch_shows_both_sides_of_the_difference() {
        let (_repo, wt) = repo_with_conflict("patch");
        let conflicts = wt.begin_conflict_resolution().unwrap();
        let patch = conflicts[0]
            .patch
            .as_deref()
            .expect("양쪽이 다 있는데 patch가 없다");

        assert!(patch.starts_with("@@"), "헤더가 잘리지 않았다: {patch}");
        assert!(
            patch.contains("-작업이 고친 줄"),
            "작업 쪽이 삭제줄로 없다: {patch}"
        );
        assert!(
            patch.contains("+base가 고친 줄"),
            "base 쪽이 추가줄로 없다: {patch}"
        );
    }

    #[test]
    fn the_patch_never_leaks_the_temporary_paths() {
        let (_repo, wt) = repo_with_conflict("patch-tmp");
        let conflicts = wt.begin_conflict_resolution().unwrap();
        let patch = conflicts[0].patch.as_deref().unwrap();
        assert!(
            !patch.contains("praxis-conflict-diff"),
            "임시 경로가 patch에 남았다: {patch}"
        );
        let flat = wt.branch.replace(['/', '\\'], "_");
        assert!(
            !wt.repo
                .join(".git")
                .join("praxis-conflict-diff")
                .join(flat)
                .exists(),
            "임시 디렉터리가 정리되지 않았다"
        );
    }

    /// patch는 사람이 읽을 화면이 아니라 프론트가 파싱하는 데이터다. 사용자의 git 설정이
    /// 형식을 바꾸면 파서가 조용히 깨지므로, 흔한 두 설정을 켜 두고도 순수 unified diff가
    /// 나오는지 못박는다.
    #[test]
    fn the_patch_ignores_the_users_diff_configuration() {
        let (repo, wt) = repo_with_conflict("patch-config");
        // 색상을 강제하고, 외부 diff 도구를 걸어 둔다(delta·difftastic을 쓰는 흔한 설정).
        git(&repo, &["config", "color.ui", "always"]);
        git(&repo, &["config", "color.diff", "always"]);
        git(&repo, &["config", "diff.external", "/bin/echo"]);

        let conflicts = wt.begin_conflict_resolution().unwrap();
        let patch = conflicts[0]
            .patch
            .as_deref()
            .expect("설정 때문에 patch가 사라졌다");

        assert!(!patch.contains('\u{1b}'), "ANSI 색상이 섞였다: {patch:?}");
        assert!(patch.starts_with("@@"), "unified diff가 아니다: {patch}");
        assert!(
            patch.contains("-작업이 고친 줄") && patch.contains("+base가 고친 줄"),
            "외부 diff 도구가 출력을 가로챘다: {patch}"
        );
    }

    /// 한쪽에 파일이 아예 없으면 비교 상대가 없다 — patch를 지어내지 않고 `None`을 준다.
    #[test]
    fn there_is_no_patch_when_one_side_deleted_the_file() {
        let repo = tmp_dir("patch-delete");
        super::super::init_repository(&repo).unwrap();
        std::fs::write(repo.join("shared.txt"), "원본\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-m", "base"]);

        let base = super::super::current_branch(&repo).unwrap();
        let wt = super::super::create(&repo, "task-branch", Some(&base), false, &|_| {})
            .unwrap()
            .0;

        // 작업 쪽은 고치고
        std::fs::write(wt.path.join("shared.txt"), "작업이 고친 줄\n").unwrap();
        git(&wt.path, &["add", "-A"]);
        git(&wt.path, &["commit", "-m", "작업 변경"]);

        // base 쪽은 지운다 → delete/modify 충돌
        std::fs::remove_file(repo.join("shared.txt")).unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-m", "base 삭제"]);

        let conflicts = wt.begin_conflict_resolution().unwrap();
        let file = conflicts
            .iter()
            .find(|f| f.path == "shared.txt")
            .expect("충돌 목록에 없다");
        assert!(file.theirs.is_none(), "base 쪽이 삭제됐는데 내용이 있다");
        assert!(file.patch.is_none(), "비교 상대가 없는데 patch가 생겼다");
    }
}
