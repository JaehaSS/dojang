//! Versioned, app-owned Codex installations. Never replace a published runtime.
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const DEFAULT_VERSION: &str = "0.157.1";
// Keep this migration value unchanged when adding a new default.
pub const LEGACY_VERSION: &str = "0.154.0";
const APPROVED: &[&str] = &[LEGACY_VERSION, DEFAULT_VERSION];
const ENTRY: &str = "node_modules/.bin/codex";
const MANIFEST: &str = "praxis-runtime.json";
const TIMEOUT: Duration = Duration::from_secs(300);

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

pub fn validate_version(version: &str) -> Result<(), String> {
    if APPROVED.contains(&version) {
        Ok(())
    } else {
        Err(format!("이 대화의 Codex {version} 실행 계약을 지원하지 않습니다. 다른 버전으로 자동 전환하지 않았습니다"))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: String,
    sha256: String,
}

/// Blocking: call from spawn_blocking, before accepting the user's message.
pub fn ensure(root: &Path, version: &str) -> Result<String, String> {
    if !cfg!(any(target_os = "macos", target_os = "linux")) {
        return Err("Codex 질문 세션은 현재 macOS/Linux에서만 지원합니다".into());
    }
    ensure_with(root, version, install)
}

fn ensure_with(
    root: &Path,
    version: &str,
    installer: impl FnOnce(&Path, &str) -> Result<(), String>,
) -> Result<String, String> {
    validate_version(version)?; // Also keeps DB values out of filesystem paths.
    fs::create_dir_all(root).map_err(err)?;
    let root = root.canonicalize().map_err(err)?;
    let destination = root.join(version);
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(format!("{version}.lock")))
        .map_err(err)?;
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match lock.try_lock_exclusive() {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(
                        "Codex 전용 런타임 설치가 진행 중입니다. 잠시 뒤 다시 시도하세요".into(),
                    );
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(err(e)),
        }
    }
    // A corrupt installation is never silently replaced, especially while another turn uses it.
    if destination.try_exists().map_err(err)? || destination.is_symlink() {
        return verify(&destination, version).map_err(|e| {
            format!(
                "Codex {version} 전용 런타임을 확인할 수 없습니다: {e}. 보관 경로: {}",
                destination.display()
            )
        });
    }
    let stage = Staging(root.join(format!(".install-{}", super::interaction::id()?)));
    fs::create_dir(&stage.0).map_err(err)?;
    installer(&stage.0, version).map_err(|e| {
        format!("Codex {version} 전용 런타임 설치 실패: {e}. 기존 대화는 변경하지 않았습니다. 다시 시도하세요")
    })?;
    let bin = entry(&stage.0)?;
    let sha256 = tree_hash(&stage.0)?;
    super::app_server::check_exact_version(&bin, version)?;
    let manifest = Manifest {
        version: version.into(),
        sha256,
    };
    let data = serde_json::to_vec(&manifest).map_err(err)?;
    fs::write(stage.0.join(MANIFEST), data).map_err(err)?;
    // The directory is private to this installer until all checks have succeeded.
    fs::rename(&stage.0, &destination).map_err(err)?;
    entry(&destination)
}

struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn entry(root: &Path) -> Result<String, String> {
    let path = root.join(ENTRY).canonicalize().map_err(err)?;
    if !path.starts_with(root) || !path.is_file() {
        return Err("실행 파일이 전용 런타임 밖을 가리킵니다".into());
    }
    path.into_os_string()
        .into_string()
        .map_err(|_| "Codex 실행 경로를 읽을 수 없습니다".into())
}

fn verify(root: &Path, version: &str) -> Result<String, String> {
    if fs::symlink_metadata(root)
        .map_err(err)?
        .file_type()
        .is_symlink()
        || !fs::symlink_metadata(root.join(MANIFEST))
            .map_err(err)?
            .is_file()
    {
        return Err("런타임 디렉터리와 manifest는 외부 링크일 수 없습니다".into());
    }
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(root.join(MANIFEST)).map_err(err)?).map_err(err)?;
    if manifest.version != version || manifest.sha256 != tree_hash(root)? {
        return Err("저장된 버전 또는 파일 무결성이 일치하지 않습니다".into());
    }
    let bin = entry(root)?;
    super::app_server::check_exact_version(&bin, version)?;
    Ok(bin)
}

/// Include package dependencies and internal links, not just the launcher script.
fn tree_hash(root: &Path) -> Result<String, String> {
    fn visit(root: &Path, dir: &Path, hash: &mut Sha256) -> Result<(), String> {
        let mut entries = fs::read_dir(dir)
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        entries.sort_by_key(|entry| entry.file_name());
        for item in entries {
            let path = item.path();
            if path == root.join(MANIFEST) {
                continue;
            }
            let relative = path.strip_prefix(root).map_err(err)?.to_string_lossy();
            hash.update((relative.len() as u64).to_le_bytes());
            hash.update(relative.as_bytes());
            let meta = fs::symlink_metadata(&path).map_err(err)?;
            if meta.is_symlink() {
                if !path.canonicalize().map_err(err)?.starts_with(root) {
                    return Err("런타임에 외부 심볼릭 링크가 있습니다".into());
                }
                hash.update(b"link");
                let link = fs::read_link(&path).map_err(err)?;
                let link = link.to_string_lossy();
                hash.update((link.len() as u64).to_le_bytes());
                hash.update(link.as_bytes());
            } else if meta.is_dir() {
                hash.update(b"dir");
                visit(root, &path, hash)?;
            } else if meta.is_file() {
                hash.update(b"file");
                hash.update(meta.len().to_le_bytes());
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    hash.update(meta.permissions().mode().to_le_bytes());
                }
                std::io::copy(&mut File::open(&path).map_err(err)?, hash).map_err(err)?;
            } else {
                return Err("런타임에 일반 파일이 아닌 항목이 있습니다".into());
            }
        }
        Ok(())
    }
    let mut hash = Sha256::new();
    visit(root, root, &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}

fn install(stage: &Path, version: &str) -> Result<(), String> {
    let npm =
        crate::reviewer::which("npm").ok_or("첫 설치에 필요한 npm을 PATH에서 찾지 못했습니다")?;
    let mut command = Command::new(npm);
    command
        .current_dir(stage)
        .args([
            "install",
            "--global=false",
            "--ignore-scripts",
            "--include=optional",
            "--no-audit",
            "--no-fund",
            "--registry=https://registry.npmjs.org",
            "--@openai:registry=https://registry.npmjs.org",
            "--prefix",
        ])
        .arg(stage)
        .arg(format!("@openai/codex@{version}"));
    run_install(command, &stage.join("install.log"), TIMEOUT)
}

fn run_install(mut command: Command, log_path: &Path, timeout: Duration) -> Result<(), String> {
    let log = File::create(log_path).map_err(err)?;
    command
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(err)?)
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = super::child_reaper::ReapOnDrop::new(command.spawn().map_err(err)?);
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().map_err(err)? {
            if !status.success() {
                let mut log = File::open(log_path).map_err(err)?;
                let start = log.metadata().map_err(err)?.len().saturating_sub(2048);
                log.seek(SeekFrom::Start(start)).map_err(err)?;
                let mut bytes = Vec::new();
                log.take(2048).read_to_end(&mut bytes).map_err(err)?;
                return Err(format!("npm {status}: {}", String::from_utf8_lossy(&bytes)));
            }
            fs::remove_file(log_path).map_err(err)?;
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "npm 설치 제한 시간({}초)을 초과했습니다",
                timeout.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
#[path = "cli_runtime_tests.rs"]
mod tests;
