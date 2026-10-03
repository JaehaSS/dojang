//! worktree 환경 부트스트랩 — `.worktreeinclude` 복사와 `.praxis-env-setup.sh` 실행.
//!
//! 새 worktree는 tracked 파일만 체크아웃한다. `.env`·로컬 인증서처럼 git이 무시하는
//! 파일은 원본에 남아 에이전트가 반쪽 환경에서 일하게 된다. 이 모듈이 그 간극을 메운다.
//!
//! 계약(리서치 0005 §2에서 채택):
//! 1. 복사하지 링크하지 않는다 — worktree 편집이 원본을 바꾸면 격리가 무의미해진다.
//! 2. worktree에 이미 있는 것을 절대 대체하지 않는다 — tracked 파일이 이긴다.
//! 3. 원본 심볼릭 링크는 건너뛰고, worktree 안 링크를 통해 쓰지 않는다.
//! 4. 부분 실패는 진행을 막지 않는다 — 전사에 남기고 계속한다.
//! 5. 복사가 셋업 스크립트보다 먼저다 — 스크립트가 복사된 `.env`를 읽어야 한다.

use std::path::Path;

/// worktree가 필요로 하는 untracked 파일 목록. gitignore 문법.
pub const INCLUDE_FILE: &str = ".worktreeinclude";

/// worktree 최초 생성 직후 1회 실행하는 셋업 훅.
pub const SETUP_SCRIPT: &str = ".praxis-env-setup.sh";

/// 후보 상한 — 이보다 많으면 매칭을 시도하지 않고 전사에 남긴다.
/// `node_modules`가 통째로 후보에 드는 저장소를 방어한다.
const CANDIDATE_LIMIT: usize = 200_000;

/// 부트스트랩 전사 — 성공·스킵·실패를 모두 남긴다.
///
/// 어떤 항목이 실패해도 `run`은 Err를 돌려주지 않는다(계약 4). 호출자는 전사를
/// 기록만 하고 작업 생성을 계속한다.
#[derive(Debug, Default, serde::Serialize)]
pub struct Transcript {
    pub copied: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
    pub setup_script: Option<String>,
}

impl Transcript {
    pub fn is_empty(&self) -> bool {
        self.copied.is_empty()
            && self.skipped.is_empty()
            && self.failed.is_empty()
            && self.setup_script.is_none()
    }
}

/// 부트스트랩 실행. `source`는 원본 체크아웃, `target`은 새 worktree.
///
/// 호출 전에 `target`이 원본과 다른 경로임을 보장해야 한다 — 직접 모드에서 부르면
/// 사용자의 메인 체크아웃에 복사하고 스크립트를 실행하게 된다.
pub fn run(source: &Path, target: &Path) -> Transcript {
    let mut transcript = copy_environment(source, target);
    run_setup_script(target, &mut transcript);
    transcript
}

/// Copy-only preparation for repair preview: never executes a setup command.
pub fn copy_environment(source: &Path, target: &Path) -> Transcript {
    let mut transcript = Transcript::default();
    let include_path = source.join(INCLUDE_FILE);
    if include_path.is_file() {
        copy_included(source, target, &include_path, &mut transcript);
    }
    transcript
}

/// untracked 파일 전량.
///
/// `--exclude-standard`를 쓰지 않는 것이 핵심이다 — 붙이면 `.gitignore`된 파일
/// (`.env` 등)이 목록에서 사라지는데, 그것이야말로 복사 대상이다.
fn collect_candidates(source: &Path) -> Vec<String> {
    let Ok(out) = super::run_git(source, &["ls-files", "--others", "-z"]) else {
        return Vec::new();
    };
    out.split('\0')
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect()
}

fn copy_included(source: &Path, target: &Path, include_path: &Path, transcript: &mut Transcript) {
    let mut builder = ignore::gitignore::GitignoreBuilder::new(source);
    if let Some(error) = builder.add(include_path) {
        transcript
            .failed
            .push(format!("{INCLUDE_FILE} 를 읽지 못했습니다: {error}"));
        return;
    }
    let matcher = match builder.build() {
        Ok(matcher) => matcher,
        Err(error) => {
            transcript
                .failed
                .push(format!("{INCLUDE_FILE} 패턴이 올바르지 않습니다: {error}"));
            return;
        }
    };

    let candidates = collect_candidates(source);
    if candidates.len() > CANDIDATE_LIMIT {
        transcript.failed.push(format!(
            "미추적 파일이 {}개를 넘어 복사를 건너뜁니다 — 큰 디렉터리는 {SETUP_SCRIPT} 에서 설치하세요",
            CANDIDATE_LIMIT
        ));
        return;
    }

    for rel in candidates {
        let from = source.join(&rel);
        // Praxis는 worktree를 `<repo>/.praxis/worktrees/<slug>` 에 만든다 — target이
        // source의 하위다. 방금 만든 worktree와 다른 작업의 worktree가 그대로 후보에
        // 드는데, gitignore 패턴은 앵커링이 없으면 임의 깊이에 매치되므로(`certs/`가
        // `.praxis/worktrees/x/certs/`에도 걸린다) 자기 자신을 복사하게 된다.
        // `.praxis`는 앱 내부 상태라 어차피 사용자가 의도할 대상이 아니다.
        if rel.starts_with(".praxis/") || from.starts_with(target) {
            continue;
        }
        // Whitelist(`!`)와 None은 둘 다 복사 대상이 아니다.
        if !matches!(
            matcher.matched_path_or_any_parents(&from, from.is_dir()),
            ignore::Match::Ignore(_)
        ) {
            continue;
        }
        // 계약 3: 원본 심볼릭 링크는 대상을 따라가지 않고 건너뛴다.
        if from
            .symlink_metadata()
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false)
        {
            transcript.skipped.push(format!("{rel} — 심볼릭 링크"));
            continue;
        }
        let to = target.join(&rel);
        // 계약 2: 이미 있는 것을 대체하지 않는다. symlink_metadata를 쓰는 이유는
        // worktree 안의 링크를 통해 쓰지 않기 위해서다(계약 3의 후반부).
        if to.symlink_metadata().is_ok() {
            transcript
                .skipped
                .push(format!("{rel} — worktree 에 이미 있습니다"));
            continue;
        }
        if let Some(parent) = to.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                // 계약 4: 하나가 실패해도 나머지는 계속한다.
                transcript
                    .failed
                    .push(format!("{rel} — 디렉터리를 만들지 못했습니다: {error}"));
                continue;
            }
        }
        match std::fs::copy(&from, &to) {
            Ok(_) => transcript.copied.push(rel),
            Err(error) => transcript
                .failed
                .push(format!("{rel} — 복사하지 못했습니다: {error}")),
        }
    }
}

/// 셋업 훅 실행. 복사 이후에 돈다(계약 5).
///
/// `sh <script>`로 부른다 — 실행 권한이 없어도 동작한다. git이 실행 비트를 잃는 일이 흔하다.
fn run_setup_script(target: &Path, transcript: &mut Transcript) {
    let script = target.join(SETUP_SCRIPT);
    if !script.is_file() {
        return;
    }
    let output = std::process::Command::new(posix_shell())
        .arg(&script)
        .current_dir(target)
        .output();
    transcript.setup_script = Some(match output {
        Ok(out) if out.status.success() => format!("{SETUP_SCRIPT} 실행을 마쳤습니다"),
        Ok(out) => format!(
            "{SETUP_SCRIPT} 가 실패했습니다(코드 {:?}) — 작업은 계속합니다: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(error) => format!("{SETUP_SCRIPT} 를 실행하지 못했습니다: {error}"),
    });
}

#[cfg(not(windows))]
fn posix_shell() -> std::path::PathBuf {
    "sh".into()
}

/// Windows에는 `sh`가 PATH에 없는 것이 보통이다. Git for Windows는 `<Git>\cmd\git.exe`만
/// PATH에 올리고 `sh.exe`는 `<Git>\bin`에 두므로, git 위치에서 거슬러 찾는다.
#[cfg(windows)]
fn posix_shell() -> std::path::PathBuf {
    if let Some(sh) = crate::reviewer::which("sh") {
        return sh.into();
    }
    crate::reviewer::which("git")
        .map(std::path::PathBuf::from)
        .and_then(|git| Some(git.parent()?.parent()?.join("bin").join("sh.exe")))
        .filter(|sh| sh.is_file())
        .unwrap_or_else(|| "sh".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_dir(tag: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = crate::testtmp::dir().join(format!(
            "praxis-bootstrap-{}-{}-{}",
            tag,
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    /// `.gitignore`와 `.worktreeinclude`를 가진 저장소 + 빈 target 디렉터리.
    fn fixture(tag: &str, gitignore: &str, include: &str) -> (PathBuf, PathBuf) {
        let source = tmp_dir(tag);
        super::super::run_git(&source, &["init"]).unwrap();
        write(&source.join(".gitignore"), gitignore);
        write(&source.join(INCLUDE_FILE), include);
        super::super::run_git(&source, &["add", "-A"]).unwrap();
        super::super::run_git(
            &source,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-m",
                "init",
            ],
        )
        .unwrap();
        let target = tmp_dir(&format!("{tag}-target"));
        (source, target)
    }

    #[test]
    fn no_include_file_yields_empty_transcript() {
        let dir = tmp_dir("none");
        let target = tmp_dir("none-target");
        let transcript = run(&dir, &target);
        assert!(transcript.is_empty(), "{transcript:?}");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&target).ok();
    }

    #[test]
    fn gitignored_file_is_copied() {
        // .env 는 .gitignore 에도 있다 — 실제 저장소의 모습이다.
        // --exclude-standard 를 쓰면 후보에서 사라져 이 테스트가 깨진다.
        let (source, target) = fixture("ignored", "node_modules/\n.env\n", ".env\n");
        write(&source.join(".env"), "SECRET=1");

        let transcript = run(&source, &target);

        assert_eq!(
            transcript.copied,
            vec![".env".to_string()],
            "{transcript:?}"
        );
        assert_eq!(
            std::fs::read_to_string(target.join(".env")).unwrap(),
            "SECRET=1"
        );
        std::fs::remove_dir_all(&source).ok();
        std::fs::remove_dir_all(&target).ok();
    }

    #[test]
    fn negated_pattern_is_excluded() {
        let (source, target) = fixture("negate", ".env*\n", ".env\n.env.*\n!.env.example\n");
        write(&source.join(".env"), "A=1");
        write(&source.join(".env.local"), "B=2");
        write(&source.join(".env.example"), "A=");

        let transcript = run(&source, &target);

        assert!(target.join(".env").exists(), "{transcript:?}");
        assert!(target.join(".env.local").exists(), "{transcript:?}");
        assert!(
            !target.join(".env.example").exists(),
            "부정 패턴(!)은 복사 대상이 아니다: {transcript:?}"
        );
        std::fs::remove_dir_all(&source).ok();
        std::fs::remove_dir_all(&target).ok();
    }

    #[test]
    fn unmatched_file_is_not_copied() {
        let (source, target) = fixture("unmatched", "", ".env\n");
        write(&source.join("notes.txt"), "hello");

        let transcript = run(&source, &target);

        assert!(!target.join("notes.txt").exists(), "{transcript:?}");
        assert!(transcript.copied.is_empty(), "{transcript:?}");
        std::fs::remove_dir_all(&source).ok();
        std::fs::remove_dir_all(&target).ok();
    }

    #[test]
    fn existing_target_file_is_never_overwritten() {
        let (source, target) = fixture("exists", ".env\n", ".env\n");
        write(&source.join(".env"), "FROM_SOURCE=1");
        write(&target.join(".env"), "ALREADY_HERE=1");

        let transcript = run(&source, &target);

        assert_eq!(
            std::fs::read_to_string(target.join(".env")).unwrap(),
            "ALREADY_HERE=1",
            "worktree 에 이미 있는 파일을 대체하면 안 된다"
        );
        assert!(
            transcript.skipped.iter().any(|s| s.contains(".env")),
            "스킵이 전사에 남아야 한다: {transcript:?}"
        );
        std::fs::remove_dir_all(&source).ok();
        std::fs::remove_dir_all(&target).ok();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_in_source_is_skipped() {
        let (source, target) = fixture("symlink", "secret*\n", "secret-link\n");
        write(&source.join("secret-real"), "REAL=1");
        std::os::unix::fs::symlink(source.join("secret-real"), source.join("secret-link")).unwrap();

        let transcript = run(&source, &target);

        assert!(
            !target.join("secret-link").exists(),
            "링크 대상을 따라가면 안 된다: {transcript:?}"
        );
        assert!(
            transcript.skipped.iter().any(|s| s.contains("심볼릭 링크")),
            "{transcript:?}"
        );
        std::fs::remove_dir_all(&source).ok();
        std::fs::remove_dir_all(&target).ok();
    }

    #[test]
    fn nested_path_is_copied_with_parent_directories() {
        let (source, target) = fixture("nested", "certs/\n", "certs/\n");
        write(&source.join("certs/dev/key.pem"), "KEY");

        let transcript = run(&source, &target);

        assert_eq!(
            std::fs::read_to_string(target.join("certs/dev/key.pem")).unwrap(),
            "KEY",
            "{transcript:?}"
        );
        std::fs::remove_dir_all(&source).ok();
        std::fs::remove_dir_all(&target).ok();
    }

    #[test]
    fn copy_failure_does_not_abort_remaining_files() {
        let (source, target) = fixture("partial", "data/\n", "data/\n");
        write(&source.join("data/a.txt"), "A");
        write(&source.join("data/b.txt"), "B");
        // target 의 data 를 "파일"로 만들어 둔다 — a/b 의 부모 디렉터리 생성이 실패한다.
        write(&target.join("data"), "블로킹");

        let transcript = run(&source, &target);

        assert!(transcript.copied.is_empty(), "{transcript:?}");
        assert_eq!(
            transcript.failed.len(),
            2,
            "실패해도 나머지 항목을 계속 시도해야 한다: {transcript:?}"
        );
        std::fs::remove_dir_all(&source).ok();
        std::fs::remove_dir_all(&target).ok();
    }

    #[test]
    fn nested_worktree_is_not_a_copy_candidate() {
        // Praxis 는 worktree 를 `<repo>/.praxis/worktrees/<slug>` 에 만든다.
        // 앵커링 없는 패턴(`certs/`)은 임의 깊이에 매치되므로, 방어가 없으면
        // 새 worktree 와 다른 작업의 worktree 안 파일을 자기 자신에게 복사한다.
        let (source, _) = fixture("nested-wt", ".praxis/\ncerts/\n", "certs/\n");
        let target = source.join(".praxis/worktrees/current");
        std::fs::create_dir_all(&target).unwrap();
        write(&source.join("certs/real.pem"), "REAL");
        write(
            &source.join(".praxis/worktrees/other/certs/stale.pem"),
            "STALE",
        );
        write(&target.join("certs/self.pem"), "SELF");

        let transcript = run(&source, &target);

        assert_eq!(
            transcript.copied,
            vec!["certs/real.pem".to_string()],
            "원본의 certs 만 복사해야 한다: {transcript:?}"
        );
        assert!(
            !target.join(".praxis").exists(),
            "worktree 안에 .praxis 를 만들면 안 된다: {transcript:?}"
        );
        std::fs::remove_dir_all(&source).ok();
    }

    #[test]
    fn setup_script_runs_after_copy() {
        let (source, target) = fixture("setup", ".env\n", ".env\n");
        write(&source.join(".env"), "VALUE=42");
        // 스크립트는 target 에 있다(worktree 에 체크아웃된 tracked 파일을 모사).
        write(
            &target.join(SETUP_SCRIPT),
            "#!/bin/sh\ncat .env > copied-proof.txt\n",
        );

        let transcript = run(&source, &target);

        assert_eq!(
            std::fs::read_to_string(target.join("copied-proof.txt")).unwrap(),
            "VALUE=42",
            "셋업 스크립트는 복사 이후에 돌아야 한다: {transcript:?}"
        );
        assert!(
            transcript
                .setup_script
                .as_deref()
                .unwrap()
                .contains("마쳤습니다"),
            "{transcript:?}"
        );
        std::fs::remove_dir_all(&source).ok();
        std::fs::remove_dir_all(&target).ok();
    }

    #[test]
    fn setup_script_failure_is_recorded_not_propagated() {
        let dir = tmp_dir("setup-fail");
        let target = tmp_dir("setup-fail-target");
        write(
            &target.join(SETUP_SCRIPT),
            "#!/bin/sh\necho 나쁨 >&2\nexit 1\n",
        );

        let transcript = run(&dir, &target);

        let note = transcript.setup_script.as_deref().unwrap();
        assert!(note.contains("실패했습니다"), "{note}");
        assert!(note.contains("작업은 계속합니다"), "{note}");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&target).ok();
    }
}
