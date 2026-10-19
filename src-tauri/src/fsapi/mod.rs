//! 워크트리 스코프 파일시스템 API (IDE 에디터, S-?? 코드 편집).
//!
//! 보안 원칙: 모든 경로는 **워크트리 루트 하위로 제한**한다.
//! - 절대 경로 / `..`(ParentDir) / 드라이브 프리픽스 거부 (경로 탈출 차단).
//! - 대상이 이미 존재하면 canonicalize 후 루트 prefix 재확인 (심볼릭 링크 우회 차단).
//! Tauri 비의존 — `cargo test`로 검증.

pub mod guard;
pub mod mutate;
pub mod search;
pub mod table;

use std::path::{Component, Path, PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;

/// 읽기/쓰기 최대 크기 (바이너리·대용량 보호).
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// 트리 재귀 최대 깊이 (병적 디렉터리 보호).
const MAX_DEPTH: usize = 12;
/// 트리에서 제외할 디렉터리/파일 이름.
const IGNORE: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    crate::project_paths::DIR,
    crate::project_paths::LEGACY_DIR,
    ".DS_Store",
];

/// 파일 트리 노드 (디렉터리는 children 보유, 파일은 빈 벡터).
#[derive(Debug, Serialize)]
pub struct FsNode {
    pub name: String,
    /// 루트 기준 상대 경로 (항상 `/` 구분자).
    pub path: String,
    pub is_dir: bool,
    pub children: Vec<FsNode>,
}

/// 파일 뷰어가 내용을 어떻게 렌더링해야 하는지 구분.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Text,
    Image,
    Binary,
    TooLarge,
    Table,
}

/// 파일 내용 + 종류 + 수정 시각(ms) — 외부 변경 감지에 사용.
///
/// `content` 의미는 `kind`에 따라 다르다:
/// - `Text`: 파일 본문(UTF-8).
/// - `Image`: data URL (`data:{mime};base64,...`).
/// - `Table`: `table::parquet_preview`가 만든 미리보기 JSON 문자열
///   (`table::TablePreview` 형태). `.parquet` 전용 — 크기 상한(`MAX_FILE_BYTES`)을
///   적용받지 않는다(파케이는 흔히 2MB를 넘는다).
/// - `Binary` / `TooLarge`: 빈 문자열(""). 프런트는 kind로 분기해
///   "기본 앱으로 열기" 등 폴백을 제공해야 한다.
#[derive(Debug, Serialize)]
pub struct FileContent {
    pub kind: FileKind,
    pub content: String,
    pub mtime: i64,
}

/// 확장자(소문자) → 이미지 MIME. 이미지가 아니면 `None`.
fn image_mime(ext: &str) -> Option<&'static str> {
    match ext {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "ico" => Some("image/x-icon"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

/// `rel`을 루트 하위 경로로 안전 결합. 탈출 시도는 에러.
///
/// 방어 계층:
/// 1. 절대 경로 / `..`(ParentDir) / 프리픽스 거부.
/// 2. 루트를 canonicalize → 실경로 기준.
/// 3. **최종 경로가 심볼릭 링크면 무조건 거부** — target 존재 여부 무관(dangling symlink 포함).
///    (worktree에서 동시 실행되는 에이전트가 심은 심볼릭으로 밖에 쓰는 것을 차단.)
/// 4. 존재하는 가장 깊은 조상을 canonicalize → 루트 하위인지 확인(심볼릭 중간 디렉터리 우회 차단).
pub fn safe_join(root: &Path, rel: &str) -> anyhow::Result<PathBuf> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        anyhow::bail!("절대 경로는 허용되지 않습니다");
    }
    for comp in rel_path.components() {
        match comp {
            // 일반 이름과 현재 디렉터리(.)만 허용.
            Component::Normal(_) | Component::CurDir => {}
            _ => anyhow::bail!("허용되지 않는 경로 요소(.. 등)"),
        }
    }
    let root_canon = root.canonicalize()?;
    let joined = root_canon.join(rel_path);

    // (3) 최종 엔트리가 심볼릭이면 거부 — exists()는 dangling symlink에서 false라 신뢰 불가.
    if let Ok(md) = joined.symlink_metadata() {
        if md.file_type().is_symlink() {
            anyhow::bail!("심볼릭 링크 대상은 허용되지 않습니다");
        }
    }

    // (4) 존재하는 가장 깊은 조상의 실경로가 루트 하위인지 확인.
    let mut cursor = joined.as_path();
    while let Some(parent) = cursor.parent() {
        if parent.exists() {
            let pc = parent.canonicalize()?;
            if !pc.starts_with(&root_canon) {
                anyhow::bail!("워크트리 밖 접근이 차단되었습니다");
            }
            break;
        }
        cursor = parent;
    }

    Ok(joined)
}

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 워크트리 파일 트리 (디렉터리 우선, 이름 오름차순; IGNORE 제외).
pub fn build_tree(root: &Path) -> anyhow::Result<Vec<FsNode>> {
    if !root.is_dir() {
        anyhow::bail!("워크트리 경로가 디렉터리가 아닙니다");
    }
    Ok(walk(root, root, 0))
}

fn walk(dir: &Path, root: &Path, depth: usize) -> Vec<FsNode> {
    if depth > MAX_DEPTH {
        return vec![];
    }
    let mut entries: Vec<std::fs::DirEntry> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.flatten().collect(),
        Err(_) => return vec![],
    };
    // 디렉터리 먼저, 그다음 이름(소문자) 오름차순.
    entries.sort_by_key(|e| {
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        (!is_dir, e.file_name().to_string_lossy().to_lowercase())
    });
    let mut nodes = Vec::new();
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        if IGNORE.contains(&name.as_str()) {
            continue;
        }
        let ft = match e.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        // 심볼릭 링크는 트리에서 제외 — 심볼릭 디렉터리 따라가며 worktree 밖을 노출하는 것 차단.
        if ft.is_symlink() {
            continue;
        }
        let is_dir = ft.is_dir();
        let full = e.path();
        let rel = full
            .strip_prefix(root)
            .unwrap_or(&full)
            .to_string_lossy()
            .replace('\\', "/");
        let children = if is_dir {
            walk(&full, root, depth + 1)
        } else {
            vec![]
        };
        nodes.push(FsNode {
            name,
            path: rel,
            is_dir,
            children,
        });
    }
    nodes
}

/// 디렉터리 브라우저가 한 번에 반환하는 엔트리 상한 (병적 디렉터리 보호).
const MAX_BROWSE_ENTRIES: usize = 2000;

/// 표시·전달용 경로 문자열 — 항상 `/` 구분자를 쓰고, Windows `canonicalize()`가 붙이는
/// verbatim 프리픽스(`\\?\`)를 떼어낸다(사용자에게 `//?/C:/...`로 새는 것 방지).
pub fn display_path(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    s.strip_prefix("//?/UNC/")
        .map(|rest| format!("//{rest}"))
        .or_else(|| s.strip_prefix("//?/").map(str::to_string))
        .unwrap_or(s)
}

/// 디렉터리 한 단계의 엔트리 — 지연 로딩 브라우저용.
///
/// `build_tree`와 달리 하위로 재귀하지 않으므로, 홈처럼 수십만 파일이 달린
/// 디렉터리도 즉시 응답한다. `ls -lrt`처럼 크기·수정 시각을 함께 준다.
#[derive(Debug, Serialize)]
pub struct DirEntryInfo {
    pub name: String,
    /// 절대 경로 (항상 `/` 구분자).
    pub path: String,
    pub is_dir: bool,
    /// 파일 크기(바이트). 디렉터리는 0.
    pub size: u64,
    pub mtime: i64,
    /// git 저장소 루트인지 — 레포 선택에서 바로 고를 수 있게 표시한다.
    pub is_repo: bool,
    /// 엔트리를 열 수 없음(권한 등) — 목록에는 남기되 진입은 막는다.
    pub denied: bool,
}

/// 디렉터리 한 단계를 나열한다(디렉터리 우선, 이름 오름차순).
///
/// `build_tree`의 IGNORE는 적용하지 않는다 — 로컬 탐색기처럼 있는 그대로 보여주는 것이
/// 목적이라 `.git`/`node_modules`도 노출한다. 심볼릭 링크는 제외한다(모듈 보안 원칙:
/// 링크를 따라 루트 밖을 노출하지 않는다). 상한을 넘으면 잘라서 반환한다.
pub fn browse_dir(dir: &Path) -> anyhow::Result<Vec<DirEntryInfo>> {
    if !dir.is_dir() {
        anyhow::bail!("디렉터리가 아닙니다");
    }
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(dir)?.flatten().collect();
    entries.sort_by_key(|e| {
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        (!is_dir, e.file_name().to_string_lossy().to_lowercase())
    });
    let mut out = Vec::new();
    for e in entries.into_iter().take(MAX_BROWSE_ENTRIES) {
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let full = e.path();
        let is_dir = ft.is_dir();
        let meta = e.metadata().ok();
        out.push(DirEntryInfo {
            name: e.file_name().to_string_lossy().into_owned(),
            path: display_path(&full),
            is_dir,
            size: if is_dir {
                0
            } else {
                meta.as_ref().map(|m| m.len()).unwrap_or(0)
            },
            mtime: meta.as_ref().map(mtime_ms).unwrap_or(0),
            // worktree는 `.git`이 파일이므로 exists()로 판정한다(디렉터리 한정 아님).
            is_repo: is_dir && full.join(".git").exists(),
            // 목록만 뽑는 단계에서는 읽기 권한을 확인할 수 없으므로 실제 진입 시점에 판정된다.
            denied: is_dir && std::fs::read_dir(&full).is_err(),
        });
    }
    Ok(out)
}

/// 브라우저가 시작점으로 쓸 상위 경로 — 각 엔트리는 존재하는 디렉터리다.
pub fn browse_parent(dir: &Path) -> Option<String> {
    dir.parent().filter(|p| p.is_dir()).map(display_path)
}

/// 파일 읽기 — 종류(텍스트/이미지/바이너리/초과)를 판별해 반환. 파일이 아니면 에러.
///
/// 크기 초과는 이미지 여부와 무관하게 `TooLarge`로 처리한다(IPC 페이로드 보호).
/// 뷰어는 "기본 앱으로 열기" 등 폴백으로 유도해야 한다.
pub fn read_file(root: &Path, rel: &str) -> anyhow::Result<FileContent> {
    let p = safe_join(root, rel)?;
    // safe_join이 심볼릭 최종 엔트리를 이미 거부 → symlink_metadata로 일반 파일만 통과.
    let meta = std::fs::symlink_metadata(&p)?;
    if !meta.is_file() {
        anyhow::bail!("파일이 아닙니다");
    }
    let mtime = mtime_ms(&meta);
    let ext = p
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    // 크기 상한보다 먼저 분기한다 — 파케이는 흔히 2MB를 넘고, 이 경로는 전체 파일을
    // 읽는 대신 스트리밍으로 처음 몇백 행만 훑으므로 IPC 페이로드 보호가 이미 되어 있다.
    if ext == "parquet" {
        return Ok(FileContent {
            kind: FileKind::Table,
            content: table::parquet_preview(&p),
            mtime,
        });
    }
    if meta.len() > MAX_FILE_BYTES {
        return Ok(FileContent {
            kind: FileKind::TooLarge,
            content: String::new(),
            mtime,
        });
    }
    let bytes = std::fs::read(&p)?;
    if let Some(mime) = image_mime(&ext) {
        let b64 = STANDARD.encode(&bytes);
        return Ok(FileContent {
            kind: FileKind::Image,
            content: format!("data:{mime};base64,{b64}"),
            mtime,
        });
    }
    match String::from_utf8(bytes) {
        Ok(content) => Ok(FileContent {
            kind: FileKind::Text,
            content,
            mtime,
        }),
        Err(_) => Ok(FileContent {
            kind: FileKind::Binary,
            content: String::new(),
            mtime,
        }),
    }
}

/// 파일 쓰기 (기존 파일 덮어쓰기; 부모 디렉터리는 존재해야 함). 새 mtime 반환.
pub fn write_file(root: &Path, rel: &str, content: &str) -> anyhow::Result<i64> {
    let p = safe_join(root, rel)?;
    if content.len() as u64 > MAX_FILE_BYTES {
        anyhow::bail!("저장 내용이 너무 큽니다 (2MB 초과)");
    }
    // 부모 디렉터리 보장(중첩 신규 파일 저장 지원). 부모는 safe_join이 루트 하위로 검증함.
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&p, content)?;
    let meta = std::fs::metadata(&p)?;
    Ok(mtime_ms(&meta))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_root() -> PathBuf {
        // 병렬 테스트 스레드가 같은 타임스탬프를 받을 수 있어(시계 해상도) 나노초만으로는
        // 디렉터리가 충돌한다(간헐 플레이크) — 원자 카운터로 프로세스 내 유일성 보장(db_test 패턴).
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = crate::testtmp::dir().join(format!(
            "praxis-fsapi-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn safe_join_rejects_traversal() {
        let root = tmp_root();
        assert!(safe_join(&root, "../escape").is_err());
        assert!(safe_join(&root, "a/../../escape").is_err());
        assert!(safe_join(&root, "/etc/passwd").is_err());
        assert!(safe_join(&root, "ok/file.txt").is_ok());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn write_read_roundtrip_and_mtime() {
        let root = tmp_root();
        let m = write_file(&root, "hello.txt", "hi").unwrap();
        assert!(m > 0);
        let fc = read_file(&root, "hello.txt").unwrap();
        assert_eq!(fc.kind, FileKind::Text);
        assert_eq!(fc.content, "hi");
        assert!(fc.mtime >= m);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn tree_skips_ignored_and_sorts_dirs_first() {
        let root = tmp_root();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        for dir in crate::project_paths::DIRS {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("z.txt"), "z").unwrap();
        std::fs::write(root.join("src/a.rs"), "a").unwrap();
        let tree = build_tree(&root).unwrap();
        let names: Vec<&str> = tree.iter().map(|n| n.name.as_str()).collect();
        assert!(!names.contains(&".git"), "ignored dir excluded");
        assert!(!names.contains(&"node_modules"), "ignored dir excluded");
        assert_eq!(names, vec!["src", "z.txt"], "dirs first, then files");
        let src = tree.iter().find(|n| n.name == "src").unwrap();
        assert_eq!(src.children.len(), 1);
        assert_eq!(src.children[0].path, "src/a.rs");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn browse_lists_one_level_with_metadata() {
        let root = tmp_root();
        std::fs::create_dir_all(root.join("repo")).unwrap();
        std::fs::write(root.join("repo/.git"), "gitdir: /elsewhere").unwrap();
        std::fs::create_dir_all(root.join("plain/nested")).unwrap();
        std::fs::write(root.join("plain/nested/deep.txt"), "deep").unwrap();
        std::fs::write(root.join("z.txt"), "hello").unwrap();

        let entries = browse_dir(&root).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["plain", "repo", "z.txt"],
            "dirs first, then files"
        );

        let repo = entries.iter().find(|e| e.name == "repo").unwrap();
        assert!(repo.is_repo, "worktree의 .git 파일도 repo로 판정");
        assert!(repo.is_dir);

        let file = entries.iter().find(|e| e.name == "z.txt").unwrap();
        assert!(!file.is_dir);
        assert!(!file.is_repo);
        assert_eq!(file.size, 5);
        assert!(file.mtime > 0);
        assert!(
            file.path.ends_with("/z.txt"),
            "절대 경로를 그대로 반환: {}",
            file.path
        );

        // 한 단계만 — 하위 디렉터리의 내용은 포함하지 않는다.
        assert!(!names.contains(&"deep.txt"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn browse_keeps_entries_that_tree_ignores() {
        let root = tmp_root();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let names: Vec<String> = browse_dir(&root)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert!(names.contains(&"node_modules".to_string()));
        assert!(names.contains(&".git".to_string()));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn browse_rejects_non_directory() {
        let root = tmp_root();
        std::fs::write(root.join("f.txt"), "x").unwrap();
        assert!(browse_dir(&root.join("f.txt")).is_err());
        assert!(browse_dir(&root.join("missing")).is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn display_path_strips_windows_verbatim_prefix() {
        // canonicalize()가 붙이는 `\\?\`가 사용자에게 `//?/C:/...`로 새면 안 된다.
        assert_eq!(
            display_path(Path::new(r"\\?\C:\Users\me")),
            "C:/Users/me".to_string()
        );
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\server\share")),
            "//server/share".to_string()
        );
        assert_eq!(
            display_path(Path::new("/home/u/work")),
            "/home/u/work".to_string()
        );
    }

    #[test]
    fn browse_parent_returns_existing_directory() {
        let root = tmp_root();
        let child = root.join("child");
        std::fs::create_dir_all(&child).unwrap();
        let parent = browse_parent(&child).unwrap();
        assert_eq!(parent, root.to_string_lossy().replace('\\', "/"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn read_classifies_binary() {
        let root = tmp_root();
        std::fs::write(root.join("bin"), [0xff, 0xfe, 0x00]).unwrap();
        let fc = read_file(&root, "bin").unwrap();
        assert_eq!(fc.kind, FileKind::Binary);
        assert_eq!(fc.content, "");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn read_classifies_image_as_data_url() {
        let root = tmp_root();
        // 최소 PNG 바이트(내용은 실제 유효 이미지가 아니어도 됨 — 판별은 확장자 기준).
        std::fs::write(root.join("pic.png"), [0x89, 0x50, 0x4e, 0x47, 0x00, 0x00]).unwrap();
        let fc = read_file(&root, "pic.png").unwrap();
        assert_eq!(fc.kind, FileKind::Image);
        assert!(
            fc.content.starts_with("data:image/png;base64,"),
            "unexpected content prefix: {}",
            &fc.content[..fc.content.len().min(40)]
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn read_classifies_oversized_file_as_too_large() {
        let root = tmp_root();
        let big = vec![b'a'; (MAX_FILE_BYTES + 1) as usize];
        std::fs::write(root.join("big.txt"), &big).unwrap();
        let fc = read_file(&root, "big.txt").unwrap();
        assert_eq!(fc.kind, FileKind::TooLarge);
        assert_eq!(fc.content, "");
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_dangling_symlink_write() {
        use std::os::unix::fs::symlink;
        let root = tmp_root();
        // worktree 안에 밖(존재하지 않는 타겟)을 가리키는 dangling symlink.
        let outside = crate::testtmp::dir().join(format!("praxis-outside-{}", std::process::id()));
        symlink(&outside, root.join("evil")).unwrap();
        // 심볼릭을 통한 쓰기/읽기는 거부되어야 함.
        assert!(write_file(&root, "evil", "payload").is_err());
        assert!(read_file(&root, "evil").is_err());
        assert!(!outside.exists(), "심볼릭 타겟에 파일이 생기면 안 됨");
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_parent_dir() {
        use std::os::unix::fs::symlink;
        let root = tmp_root();
        // 밖의 실제 디렉터리.
        let outside = crate::testtmp::dir().join(format!("praxis-out-dir-{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();
        // worktree/link → outside (심볼릭 디렉터리). link/x 쓰기는 밖으로 새므로 거부.
        symlink(&outside, root.join("link")).unwrap();
        assert!(write_file(&root, "link/x", "data").is_err());
        // 트리에도 심볼릭은 노출되지 않음.
        let tree = build_tree(&root).unwrap();
        assert!(!tree.iter().any(|n| n.name == "link"));
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }
}
