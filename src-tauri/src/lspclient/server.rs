//! 에디터가 직접 띄우는 **raw LSP 서버** 스펙 — 파일 확장자로 고른다.
//!
//! `lspdetect`와 목적이 다르다. 저쪽은 에이전트에게 물릴 `lsp-mcp` 브리지를 `.mcp.json`에
//! 자동 주입하는 쪽이고(워크트리 단위·npx 경유), 여기는 IDE 에디터가 stdio로 직접 말을 거는
//! 서버다. 브리지를 한 겹 끼우면 점프 한 번에 npx 부팅이 붙어 대화형으로 못 쓴다.

use std::path::{Path, PathBuf};
use std::process::Command;

/// 서버가 "의미 분석이 끝났다"를 알리는 방식. 전부 **에지 트리거 알림**이므로
/// 받는 쪽은 최신값 저장이 아니라 래치여야 한다(설계 0065 DR-2b).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessSignal {
    /// rust-analyzer — `experimental/serverStatus`.
    ServerStatus,
    /// jdtls — `language/status`의 `ServiceReady`.
    LanguageStatus,
    /// typescript-language-server — 워크스페이스 진행 토큰의 `end`.
    Progress,
    /// pyright — 기다릴 신호가 없다.
    Unsupported,
}

/// 한 언어에 대응하는 LSP 서버 실행 스펙.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerSpec {
    /// 서버 풀의 키이자 UI 표기명.
    pub key: &'static str,
    pub command: &'static str,
    pub args: &'static [&'static str],
    /// 이 중 하나라도 워크트리 루트에 있어야 그 언어 프로젝트로 인정한다.
    pub root_markers: &'static [&'static str],
    /// 그래프 fingerprint의 입력 — 이 파일들이 바뀌면 의미 분석 결과가 달라진다.
    pub config_files: &'static [&'static str],
    pub readiness: ReadinessSignal,
    /// `initialize` 응답 상한.
    pub init_timeout_secs: u64,
    /// 의미 분석 준비 상한.
    pub ready_timeout_secs: u64,
}

const RUST: ServerSpec = ServerSpec {
    key: "rust-analyzer",
    command: "rust-analyzer",
    args: &[],
    root_markers: &["Cargo.toml"],
    config_files: &[
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain",
        "rust-toolchain.toml",
        // 루트 `.cargo/config.toml` — cfg 플래그·target을 바꾸면 답이 달라진다.
        ".cargo/config.toml",
    ],
    readiness: ReadinessSignal::ServerStatus,
    init_timeout_secs: 30,
    ready_timeout_secs: 60,
};

const TYPESCRIPT: ServerSpec = ServerSpec {
    key: "typescript-language-server",
    command: "typescript-language-server",
    args: &["--stdio"],
    root_markers: &["tsconfig.json", "jsconfig.json", "package.json"],
    config_files: &[
        "tsconfig.json",
        "jsconfig.json",
        "package.json",
        "package-lock.json",
        "yarn.lock",
        "pnpm-lock.yaml",
    ],
    readiness: ReadinessSignal::Progress,
    init_timeout_secs: 30,
    ready_timeout_secs: 60,
};

const PYTHON: ServerSpec = ServerSpec {
    key: "pyright",
    command: "pyright-langserver",
    args: &["--stdio"],
    root_markers: &["pyproject.toml", "setup.py", "requirements.txt"],
    config_files: &[
        "pyproject.toml",
        "setup.py",
        "setup.cfg",
        "requirements.txt",
        "Pipfile",
    ],
    readiness: ReadinessSignal::Unsupported,
    init_timeout_secs: 30,
    ready_timeout_secs: 60,
};

/// jdtls는 프로젝트 cwd에서 띄우되, `-data`와 runtime JVM을 명시해 worktree·설정별
/// 캐시와 Gradle import JVM을 분리한다.
const JAVA: ServerSpec = ServerSpec {
    key: "jdtls",
    command: "jdtls",
    args: &[],
    root_markers: &[
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "settings.gradle",
        "settings.gradle.kts",
        ".classpath",
    ],
    // `.classpath`는 루트 마커로만 쓰고 식별자 입력에서는 뺀다 — jdtls(Buildship)가 import
    // 때마다 다시 쓰는 파일이라, 넣으면 import가 끝날 때마다 설정 식별자가 바뀌어 서버를
    // 새 워크스페이스로 다시 띄운다(실측: 한 번의 import에서 1743→1239→845바이트로 두 번
    // 바뀌어 JVM 세 개가 떴고, 상태 조회는 옛 키를 찾아 `idle`을 돌려줬다). 서버가 쓰는
    // 파일을 식별자에 넣으면 자기 자신을 무효화한다.
    config_files: &[
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "settings.gradle",
        "settings.gradle.kts",
        "gradle.properties",
        "gradle/wrapper/gradle-wrapper.properties",
    ],
    readiness: ReadinessSignal::LanguageStatus,
    init_timeout_secs: 120,
    ready_timeout_secs: 120,
};

/// 확장자 → (서버, LSP languageId).
///
/// languageId는 서버가 파서를 고르는 근거다. tsx를 "typescript"로 보내면 JSX 구문에서
/// 파싱이 깨져 정의를 못 찾는다 — 확장자별로 정확히 구분해 보낸다.
pub fn spec_for_path(path: &Path) -> Option<(ServerSpec, &'static str)> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let pair = match ext.as_str() {
        "rs" => (RUST, "rust"),
        "ts" | "mts" | "cts" => (TYPESCRIPT, "typescript"),
        "tsx" => (TYPESCRIPT, "typescriptreact"),
        "js" | "mjs" | "cjs" => (TYPESCRIPT, "javascript"),
        "jsx" => (TYPESCRIPT, "javascriptreact"),
        "py" | "pyi" => (PYTHON, "python"),
        "java" => (JAVA, "java"),
        _ => return None,
    };
    Some(pair)
}

/// 조회 파일에서 위로 올라가 언어 프로젝트 루트를 찾는다.
///
/// 에디터가 여는 파일은 종종 worktree 바로 아래에 있지 않다. 특히 Gradle 멀티 모듈은
/// `settings.gradle(.kts)`가 있는 상위 aggregate를 jdtls의 루트로 써야 한다. symlink를
/// 해석한 worktree 밖으로는 절대 올라가지 않는다.
pub fn resolve_root(spec: &ServerSpec, worktree: &Path, file: &Path) -> Result<PathBuf, String> {
    let canonical_worktree = worktree
        .canonicalize()
        .map_err(|e| format!("워크트리 경로를 확인할 수 없습니다: {e}"))?;
    let file = if file.is_absolute() {
        file.to_path_buf()
    } else {
        worktree.join(file)
    };
    let start = if file.is_dir() {
        file.as_path()
    } else {
        file.parent().unwrap_or(worktree)
    };
    let mut closest_marker = None;
    let mut java_aggregate = None;
    for ancestor in start.ancestors() {
        let canonical = ancestor
            .canonicalize()
            .unwrap_or_else(|_| ancestor.to_path_buf());
        if !canonical.starts_with(&canonical_worktree) {
            break;
        }
        if spec.key == "jdtls"
            && (ancestor.join("settings.gradle").is_file()
                || ancestor.join("settings.gradle.kts").is_file())
        {
            // 계속 올라가도 worktree 안의 더 바깥 aggregate만 고른다.
            java_aggregate = Some(canonical.clone());
        }
        if closest_marker.is_none()
            && spec
                .root_markers
                .iter()
                .any(|marker| ancestor.join(marker).is_file())
        {
            closest_marker = Some(canonical.clone());
        }
        if canonical == canonical_worktree {
            break;
        }
    }
    java_aggregate.or(closest_marker).ok_or_else(|| {
        format!(
            "이 워크트리에서 {} 프로젝트를 찾지 못했습니다 ({} 없음)",
            spec.key,
            spec.root_markers.join(" / ")
        )
    })
}

/// 프로젝트 루트에 이 언어의 마커 파일이 있는가.
pub fn matches_root(spec: &ServerSpec, worktree: &Path) -> bool {
    spec.root_markers
        .iter()
        .any(|marker| worktree.join(marker).is_file())
}

/// 실행 파일이 PATH에 있는가. 없으면 설치 안내를 띄워야 하므로 spawn 전에 확인한다.
pub fn command_available(command: &str) -> bool {
    if command.contains('/') {
        return Path::new(command).is_file();
    }
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(command);
        candidate.is_file() && is_executable(&candidate)
    })
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    true
}

/// 서버를 못 쓰는 이유 — UI가 그대로 사용자에게 보여준다.
pub fn unavailable_reason(spec: &ServerSpec, worktree: &Path) -> Option<String> {
    if !matches_root(spec, worktree) {
        return Some(format!(
            "이 워크트리에서 {} 프로젝트를 찾지 못했습니다 ({} 없음)",
            spec.key,
            spec.root_markers.join(" / ")
        ));
    }
    if !command_available(spec.command) {
        return Some(format!(
            "{}이(가) PATH에 없습니다 — 설치 후 다시 시도하세요",
            spec.command
        ));
    }
    None
}

/// Java 프로젝트 import에 쓸 JVM. jdtls 자신을 띄우는 runtime JVM과 분리한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaImportJdk {
    pub home: PathBuf,
    pub major: Option<u32>,
}

/// jdtls를 실행할 JVM은 import JVM과 별개로 Java 21 이상이어야 한다. Homebrew의 jdtls
/// wrapper는 JAVA_HOME을 그대로 물려받으므로, 프로젝트가 JAVA_HOME=17로 실행되어도
/// `--java-executable`을 명시해 런타임을 분리한다.
pub fn resolve_jdtls_runtime() -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Some(home) = std::env::var_os("JAVA_HOME").map(PathBuf::from) {
        candidates.push(home);
    }
    if let Some(path_java) = crate::reviewer::which("java") {
        if let Some(home) = jdk_home_from_java_executable(Path::new(&path_java)) {
            candidates.push(home);
        }
    }
    #[cfg(target_os = "macos")]
    if let Ok(output) = Command::new("/usr/libexec/java_home")
        .args(["-v", "21+"])
        .output()
    {
        if output.status.success() {
            candidates.push(PathBuf::from(
                String::from_utf8_lossy(&output.stdout).trim(),
            ));
        }
    }
    for prefix in ["/opt/homebrew/opt", "/usr/local/opt"] {
        candidates.push(Path::new(prefix).join("openjdk/libexec/openjdk.jdk/Contents/Home"));
        candidates.push(Path::new(prefix).join("openjdk@21/libexec/openjdk.jdk/Contents/Home"));
    }
    candidates.into_iter().find_map(|home| {
        let executable = java_executable(&home);
        (executable.is_file() && java_major(&executable).is_some_and(|major| major >= 21)).then_some(executable)
    }).ok_or_else(|| "jdtls 실행에는 Java 21 이상이 필요합니다. Java 21 이상 JDK를 설치하거나 PATH에 추가하세요".into())
}

/// `gradle.properties`의 명시 JVM은 Gradle이 정한 값이므로 우선한다. 없으면 흔한
/// `javaVersion=17`와 toolchain 표기를 읽어 설치된 같은 major의 JDK를 고른다.
pub fn resolve_java_import_jdk(root: &Path) -> Result<JavaImportJdk, String> {
    let explicit = root.join("gradle.properties");
    if let Ok(text) = std::fs::read_to_string(&explicit) {
        if let Some(value) = property_value(&text, "org.gradle.java.home") {
            let home = PathBuf::from(value);
            if java_executable(&home).is_file() {
                return Ok(JavaImportJdk {
                    major: java_major(&java_executable(&home)),
                    home,
                });
            }
            return Err(format!(
                "gradle.properties의 org.gradle.java.home에 Java 실행 파일이 없습니다: {}",
                home.display()
            ));
        }
    }
    let requested = requested_java_major(root);
    if let Some(path_java) = crate::reviewer::which("java") {
        let executable = PathBuf::from(path_java);
        let home = jdk_home_from_java_executable(&executable);
        if let Some(home) = home.filter(|h| valid_jdk(h, requested)) {
            return Ok(JavaImportJdk {
                major: java_major(&java_executable(&home)),
                home,
            });
        }
    }
    // jdtls 런처의 JAVA_HOME은 지원되는 최신 JVM일 수 있다. 프로젝트가 요구 버전을
    // 말하지 않은 경우에도 shell PATH의 `java`를 먼저 써서 런타임 JVM과 import JVM을 섞지 않는다.
    if let Some(home) = std::env::var_os("JAVA_HOME").map(PathBuf::from) {
        if valid_jdk(&home, requested) {
            return Ok(JavaImportJdk {
                major: java_major(&java_executable(&home)),
                home,
            });
        }
    }
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("/usr/libexec/java_home");
        if let Some(major) = requested {
            command.args(["-v", &major.to_string()]);
        }
        if let Ok(output) = command.output() {
            if output.status.success() {
                let home = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
                if valid_jdk(&home, requested) {
                    return Ok(JavaImportJdk {
                        major: java_major(&java_executable(&home)),
                        home,
                    });
                }
            }
        }
    }
    if let Some(major) = requested {
        for prefix in ["/opt/homebrew/opt", "/usr/local/opt"] {
            let home = Path::new(prefix)
                .join(format!("openjdk@{major}"))
                .join("libexec/openjdk.jdk/Contents/Home");
            if valid_jdk(&home, Some(major)) {
                return Ok(JavaImportJdk {
                    home,
                    major: Some(major),
                });
            }
        }
    }
    match requested {
        Some(major) => Err(format!(
            "이 Gradle 프로젝트는 Java {major}를 요청하지만 설치된 JDK를 찾지 못했습니다. JAVA_HOME 또는 org.gradle.java.home을 Java {major} JDK로 설정하세요"
        )),
        None => Err("Java import에 쓸 JDK를 찾지 못했습니다. JAVA_HOME 또는 org.gradle.java.home을 설정하세요".to_string()),
    }
}

fn property_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with('!') {
            return None;
        }
        let (left, value) = line.split_once('=').or_else(|| line.split_once(':'))?;
        (left.trim() == key).then(|| value.trim().replace("\\ ", " "))
    })
}

fn requested_java_major(root: &Path) -> Option<u32> {
    let properties = std::fs::read_to_string(root.join("gradle.properties")).ok();
    let build = [
        "build.gradle",
        "build.gradle.kts",
        "settings.gradle",
        "settings.gradle.kts",
    ]
    .iter()
    .filter_map(|name| std::fs::read_to_string(root.join(name)).ok())
    .collect::<Vec<_>>()
    .join("\n");
    properties
        .as_deref()
        .and_then(|text| property_value(text, "javaVersion"))
        .and_then(|value| leading_major(&value))
        .or_else(|| toolchain_major(&build))
}

fn leading_major(value: &str) -> Option<u32> {
    let digits: String = value
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn toolchain_major(text: &str) -> Option<u32> {
    for marker in [
        "JavaLanguageVersion.of(",
        "JavaVersion.VERSION_",
        "languageVersion =",
    ] {
        if let Some(rest) = text.split_once(marker).map(|(_, rest)| rest) {
            if let Some(major) = immediate_major(rest) {
                return Some(major);
            }
        }
    }
    None
}

/// toolchain 표현의 인자 자리에 바로 쓴 숫자만 수용한다. `of(project.javaVersion)` 뒤의
/// 다른 의존성 버전을 잘못 JDK 버전으로 읽으면 안 된다.
fn immediate_major(value: &str) -> Option<u32> {
    let value = value.trim_start().trim_start_matches(['\'', '"']);
    let digits: String = value.chars().take_while(|c| c.is_ascii_digit()).collect();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

fn java_executable(home: &Path) -> PathBuf {
    home.join("bin")
        .join(if cfg!(windows) { "java.exe" } else { "java" })
}

/// PATH의 `java`는 Homebrew opt 경로나 macOS shim처럼 JDK 홈과 다른 위치를 가리킬 수
/// 있다. 실제 실행 파일을 먼저 정규화한 뒤, `release`를 가진 JDK 홈일 때만 수용한다.
/// 정규화가 실패한 경로는 원래 실행 파일 위치를 마지막으로 사용한다.
fn jdk_home_from_java_executable(executable: &Path) -> Option<PathBuf> {
    let executable = executable
        .canonicalize()
        .unwrap_or_else(|_| executable.to_path_buf());
    let home = executable.parent()?.parent()?.to_path_buf();
    home.join("release").is_file().then_some(home)
}

fn valid_jdk(home: &Path, expected: Option<u32>) -> bool {
    let executable = java_executable(home);
    executable.is_file() && expected.is_none_or(|major| java_major(&executable) == Some(major))
}

fn java_major(executable: &Path) -> Option<u32> {
    if let Some(home) = executable.parent().and_then(Path::parent) {
        if let Ok(release) = std::fs::read_to_string(home.join("release")) {
            if let Some(version) = property_value(&release, "JAVA_VERSION") {
                if let Some(major) = immediate_major(version.trim_matches('"')) {
                    return Some(major);
                }
            }
        }
    }
    let output = Command::new(executable).arg("-version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stderr);
    let quote = text.split('"').nth(1)?;
    immediate_major(quote)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn tsx_gets_react_language_id() {
        let (spec, lang) = spec_for_path(&PathBuf::from("src/App.tsx")).unwrap();
        assert_eq!(spec.key, "typescript-language-server");
        assert_eq!(lang, "typescriptreact");
    }

    #[test]
    fn plain_ts_is_not_react() {
        assert_eq!(
            spec_for_path(&PathBuf::from("a/b.ts")).unwrap().1,
            "typescript"
        );
    }

    #[test]
    fn rust_and_python_map_to_own_servers() {
        assert_eq!(
            spec_for_path(&PathBuf::from("src/main.rs")).unwrap().0.key,
            "rust-analyzer"
        );
        assert_eq!(
            spec_for_path(&PathBuf::from("app.py")).unwrap().0.key,
            "pyright"
        );
    }

    #[test]
    fn java_maps_to_jdtls() {
        let (spec, lang) = spec_for_path(&PathBuf::from("src/Main.java")).unwrap();
        assert_eq!(spec.key, "jdtls");
        assert_eq!(lang, "java");
        assert_eq!(spec.readiness, ReadinessSignal::LanguageStatus);
    }

    #[test]
    fn unknown_extension_has_no_server() {
        assert!(spec_for_path(&PathBuf::from("README.md")).is_none());
        assert!(spec_for_path(&PathBuf::from("Makefile")).is_none());
    }

    #[test]
    fn extension_match_is_case_insensitive() {
        assert!(spec_for_path(&PathBuf::from("Main.RS")).is_some());
    }

    #[test]
    fn missing_marker_reports_project_not_found() {
        let dir = crate::testtmp::dir().join("praxis-lsp-spec-test-empty");
        std::fs::create_dir_all(&dir).unwrap();
        let reason = unavailable_reason(&RUST, &dir).unwrap();
        assert!(reason.contains("Cargo.toml"), "{reason}");
    }

    #[test]
    fn absolute_command_checks_the_file_itself() {
        assert!(!command_available("/nonexistent/bin/rust-analyzer"));
        assert!(command_available("/bin/sh"));
    }

    #[test]
    fn java_root_prefers_containing_settings_aggregate_without_escaping_worktree() {
        let base = crate::testtmp::dir().join(format!("lsp-java-root-{}", std::process::id()));
        let root = base.join("worktree");
        let nested = root.join("module/src/Main.java");
        std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
        std::fs::write(
            root.join("settings.gradle"),
            "rootProject.name = 'sample'\n",
        )
        .unwrap();
        std::fs::write(root.join("build.gradle"), "\n").unwrap();
        std::fs::write(&nested, "class Main {}\n").unwrap();
        let (spec, _) = spec_for_path(&nested).unwrap();
        assert_eq!(
            resolve_root(&spec, &root, &nested).unwrap(),
            root.canonicalize().unwrap()
        );
        // 준비 대기 경로는 조회 파일 대신 프로젝트 디렉터리 자체를 전달한다.
        assert_eq!(
            resolve_root(&spec, &root, &root).unwrap(),
            root.canonicalize().unwrap()
        );

        let outside = base.join("settings.gradle");
        std::fs::write(&outside, "// must not be selected\n").unwrap();
        assert_eq!(
            resolve_root(&spec, &root, &nested).unwrap(),
            root.canonicalize().unwrap()
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn toolchain_major_accepts_literal_but_not_a_variable_or_later_dependency_version() {
        assert_eq!(
            toolchain_major("languageVersion = JavaLanguageVersion.of(17)"),
            Some(17)
        );
        assert_eq!(
            toolchain_major(
                "JavaLanguageVersion.of(project.javaVersion)\nimplementation 'x:y:3.0'"
            ),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn path_java_symlink_uses_the_canonical_jdk_home() {
        use std::os::unix::fs::symlink;

        let base = crate::testtmp::dir().join(format!(
            "lsp-java-path-home-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let real_home = base.join("cellar/openjdk@17/Contents/Home");
        let path_java = base.join("opt/openjdk@17/bin/java");
        std::fs::create_dir_all(real_home.join("bin")).unwrap();
        std::fs::create_dir_all(path_java.parent().unwrap()).unwrap();
        std::fs::write(real_home.join("release"), "JAVA_VERSION=\"17.0.13\"\n").unwrap();
        std::fs::write(real_home.join("bin/java"), "fake java").unwrap();
        symlink(real_home.join("bin/java"), &path_java).unwrap();

        let home = jdk_home_from_java_executable(&path_java).unwrap();
        assert_eq!(home, real_home.canonicalize().unwrap());
        assert!(valid_jdk(&home, Some(17)));

        let _ = std::fs::remove_dir_all(base);
    }
}
