//! 파일형 메모리의 IPC 표면 — 설정(루트·상한), 목록, 열기, 정리 세션.
//!
//! 설계: `docs/designs/2026-09-13-memory-is-a-file-in-the-vault.md` §5·§6.
//! 본문을 자유롭게 읽고 쓰는 커맨드는 **없다** — 정본이 파일이라 에디터 창이 곧
//! 편집기다(R4). 예외 하나는 `insight_memory_append`(발견 공간 설계 §8, T4)로,
//! 한 줄만 append하는 좁은 쓰기다.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime, State, WebviewWindow};

use crate::memory::file::{self, AppendOutcome, Caps, MemoryFileInfo, MemoryTarget};
use crate::project_editor::{self, ProjectEditorState, ProjectLaunch};

use super::knowledge_vault_session::SessionOpenDto;
use super::{pool_of, AppState};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySettings {
    /// 설정에 저장된 원문. 비어 있으면 "자동"(창고 → 앱 데이터)이다.
    pub root: String,
    /// 지금 실제로 쓰이는 경로. 화면은 이것을 보여 준다.
    pub effective_root: String,
    pub cap_lines: u32,
    pub cap_bytes: u64,
    pub user_cap_lines: u32,
    pub user_cap_bytes: u64,
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

async fn current_settings(pool: &sqlx::SqlitePool) -> MemorySettings {
    let data_dir = file::data_dir(pool);
    let caps = file::caps(pool).await;
    let user_caps = file::user_caps(pool).await;
    let root = crate::db::get_setting(pool, file::SETTING_ROOT)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    MemorySettings {
        root: root.trim().to_string(),
        effective_root: file::memory_root(pool, &data_dir)
            .await
            .to_string_lossy()
            .into_owned(),
        cap_lines: caps.lines,
        cap_bytes: caps.bytes,
        user_cap_lines: user_caps.lines,
        user_cap_bytes: user_caps.bytes,
    }
}

#[tauri::command]
pub async fn memory_settings_get(state: State<'_, AppState>) -> Result<MemorySettings, String> {
    let pool = pool_of(&state)?;
    Ok(current_settings(&pool).await)
}

/// 상한 검증 — 0은 "블록을 비워라"와 같고, 상한의 상한을 넘으면 컨텍스트가 메모리로 찬다.
fn validate_caps(caps: &Caps, label: &str) -> Result<(), String> {
    if caps.lines == 0 || caps.lines > file::MAX_CAP_LINES {
        return Err(format!(
            "{label} 줄 상한은 1~{} 사이여야 합니다",
            file::MAX_CAP_LINES
        ));
    }
    if caps.bytes == 0 || caps.bytes > file::MAX_CAP_BYTES {
        return Err(format!(
            "{label} 바이트 상한은 1~{} 사이여야 합니다",
            file::MAX_CAP_BYTES
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn memory_settings_set(
    state: State<'_, AppState>,
    settings: MemorySettings,
) -> Result<MemorySettings, String> {
    let pool = pool_of(&state)?;
    validate_caps(
        &Caps {
            lines: settings.cap_lines,
            bytes: settings.cap_bytes,
        },
        "MEMORY.md",
    )?;
    validate_caps(
        &Caps {
            lines: settings.user_cap_lines,
            bytes: settings.user_cap_bytes,
        },
        "USER.md",
    )?;
    let root = settings.root.trim();
    if root.is_empty() {
        // 빈 값은 "자동으로 되돌려라"다 — 빈 문자열을 저장하면 절대 경로 ""가 된다.
        crate::db::delete_setting(&pool, file::SETTING_ROOT)
            .await
            .map_err(text)?;
    } else {
        if !Path::new(root).is_absolute() {
            return Err("메모리 루트는 절대 경로여야 합니다".into());
        }
        crate::db::set_setting(&pool, file::SETTING_ROOT, root)
            .await
            .map_err(text)?;
    }
    for (key, value) in [
        (file::SETTING_CAP_LINES, settings.cap_lines.to_string()),
        (file::SETTING_CAP_BYTES, settings.cap_bytes.to_string()),
        (
            file::SETTING_USER_CAP_LINES,
            settings.user_cap_lines.to_string(),
        ),
        (
            file::SETTING_USER_CAP_BYTES,
            settings.user_cap_bytes.to_string(),
        ),
    ] {
        crate::db::set_setting(&pool, key, &value)
            .await
            .map_err(text)?;
    }
    Ok(current_settings(&pool).await)
}

#[tauri::command]
pub async fn memory_files_list(
    state: State<'_, AppState>,
) -> Result<Vec<MemoryFileInfo>, String> {
    let pool = pool_of(&state)?;
    let data_dir = file::data_dir(&pool);
    file::list(&pool, &data_dir).await.map_err(text)
}

/// 메모리 루트를 프로젝트 에디터 창으로 연다.
///
/// 에디터는 **창 하나 = 루트 하나**라 특정 파일로 바로 들어가는 진입이 없다
/// (`project_editor_open_path`는 OS 기본 앱으로 여는 다른 길이다). 그래서 루트를 열고,
/// 그 파일이 들어앉을 디렉터리를 미리 만들어 둔다 — 없는 폴더에는 저장이 안 된다.
#[tauri::command]
pub async fn memory_file_open<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    editor: State<'_, ProjectEditorState>,
    path: String,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    let data_dir = file::data_dir(&pool);
    let root = file::memory_root(&pool, &data_dir).await;
    let target = resolve_inside(&root, &path)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(text)?;
    }
    project_editor::open_with_launch(&app, &editor, root, None).await?;
    Ok(())
}

/// 발견 카드 한 문장을 메모리에 남긴다(설계 §8 "메모리에 남기기", T4).
///
/// 경로는 여기서 만들지 않는다 — `target`이 `User`/`Repo{repo}`로만 고르고, 실제 파일
/// 경로는 `memory::file::append`가 `memory_root` + 그 모듈의 경로 규칙으로 조립한다.
/// 상한을 넘기면 쓰지 않고 `Full{path}`를 돌려주므로, 화면은 그 경로를
/// `memory_file_open`으로 열어 사람이 합치게 한다.
#[tauri::command]
pub async fn insight_memory_append(
    state: State<'_, AppState>,
    target: MemoryTarget,
    text: String,
) -> Result<AppendOutcome, String> {
    let pool = pool_of(&state)?;
    let data_dir = file::data_dir(&pool);
    file::append(&pool, &data_dir, &target, &text)
        .await
        .map_err(|error| error.to_string())
}

/// 열기 대상은 메모리 루트 안이어야 한다. 목록이 준 경로를 그대로 받지만, IPC는
/// 화면만 부르는 것이 아니므로 경계를 여기서 다시 본다.
fn resolve_inside(root: &Path, path: &str) -> Result<PathBuf, String> {
    let candidate = PathBuf::from(path.trim());
    if candidate.as_os_str().is_empty() {
        return Err("경로가 비어 있습니다".into());
    }
    if !candidate.is_absolute() {
        return Err("메모리 파일 경로는 절대 경로여야 합니다".into());
    }
    if candidate
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("허용되지 않는 경로 요소(.. 등)".into());
    }
    if !candidate.starts_with(root) {
        return Err("메모리 루트 밖의 경로입니다".into());
    }
    Ok(candidate)
}

/// 메모리 정리 세션 — 루트를 cwd로 에이전트 CLI를 띄운다(R5, ADR 0190 D3과 같은 경로).
#[tauri::command]
pub async fn memory_session_open<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    editor: State<'_, ProjectEditorState>,
    window: WebviewWindow<R>,
    agent: String,
) -> Result<SessionOpenDto, String> {
    if window.label() != "main" {
        return Err("정리 세션은 메인 창에서만 열 수 있습니다".into());
    }
    let pool = pool_of(&state)?;
    let data_dir = file::data_dir(&pool);
    let root = file::memory_root(&pool, &data_dir).await;
    // 세션이 열리기 전에 루트가 있어야 한다 — 없는 폴더를 cwd로 주면 셸이 뜨지 않는다.
    std::fs::create_dir_all(&root).map_err(text)?;
    let caps = file::caps(&pool).await;
    let user_caps = file::user_caps(&pool).await;
    let model = crate::db::get_setting(&pool, &format!("model:{}", agent.trim()))
        .await
        .ok()
        .flatten()
        .filter(|value| !value.trim().is_empty());
    let launch = session_launch(agent.trim(), model.as_deref(), &caps, &user_caps)?;
    let info = project_editor::open_with_launch(&app, &editor, root, Some(launch)).await?;
    Ok(SessionOpenDto {
        label: info.label,
        root: info.root,
        // 메모리 정리에는 앱 밖 스킬이 없다 — 규칙 전체가 아래 seed 지시문 안에 있다.
        skill: String::new(),
        skill_present: true,
        warning: None,
    })
}

/// 에이전트별 seed 지시문. 상한은 설정에서 오므로 문구에 실제 숫자를 박는다 —
/// 화면과 블록이 말하는 상한이 서로 다르면 에이전트가 어느 쪽을 믿을지 알 수 없다.
fn session_seed(agent: &str, caps: &Caps, user_caps: &Caps) -> String {
    let body = format!(
        "이 폴더는 Dojang 메모리 정본이다. USER.md(전역, 상한 {}줄/{}KB)와 \
         <repo-key>/MEMORY.md(저장소별, 상한 {}줄/{}KB)를 읽고, 중복을 합치고 낡은 항목을 지워 \
         상한 안으로 줄여라. 코드에서 다시 얻을 수 있는 것·로그·임시 경로는 지운다. \
         이 폴더 밖은 쓰지 않는다.",
        user_caps.lines,
        user_caps.bytes / 1024,
        caps.lines,
        caps.bytes / 1024,
    );
    // 창고 세션과 달리 스킬 호출이 없다 — 규칙이 이 문장 안에 다 있다. `/memory`는 Claude Code
    // 내장 명령(메모리 파일 선택기)이라 앞에 붙이면 지시문이 그 명령의 인자로 삼켜진다.
    let _ = agent;
    body
}

/// (bin, args) 결정 — PTY는 PATH를 풀지 않으므로 bin은 `which`가 준 절대 경로다.
pub fn session_launch(
    agent: &str,
    model: Option<&str>,
    caps: &Caps,
    user_caps: &Caps,
) -> Result<ProjectLaunch, String> {
    let seed = session_seed(agent, caps, user_caps);
    let (bin, args) = crate::agent::agent_args_with_effort(agent, &seed, model, None)
        .ok_or("에이전트 명령을 만들 수 없습니다")?;
    let bin = crate::reviewer::which(&bin)
        .ok_or_else(|| format!("{bin} CLI를 PATH에서 찾을 수 없습니다"))?;
    Ok(ProjectLaunch { bin, args })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps() -> (Caps, Caps) {
        (Caps::default(), Caps::user_default())
    }

    #[test]
    fn seed_carries_the_real_caps_and_the_folder_boundary() {
        let (caps, user_caps) = caps();
        let claude = session_seed("claude", &caps, &user_caps);
        // 슬래시로 시작하면 안 된다 — `/memory`는 Claude Code 내장 명령이라 지시문을 삼킨다.
        assert!(!claude.starts_with('/'), "{claude}");
        assert!(claude.starts_with("이 폴더는 Dojang 메모리 정본이다."));
        assert!(claude.contains("USER.md(전역, 상한 40줄/3KB)"));
        assert!(claude.contains("MEMORY.md(저장소별, 상한 100줄/8KB)"));
        assert!(claude.contains("이 폴더 밖은 쓰지 않는다."));
        // 벤더가 달라도 같은 문장이다 — 스킬 호출이 없으니 가를 이유가 없다.
        let codex = session_seed("codex", &caps, &user_caps);
        assert_eq!(claude, codex);
        assert_eq!(session_seed("agy", &caps, &user_caps), codex);
    }

    #[test]
    fn seed_follows_the_configured_caps() {
        let seed = session_seed(
            "codex",
            &Caps {
                lines: 250,
                bytes: 20480,
            },
            &Caps {
                lines: 10,
                bytes: 1024,
            },
        );
        assert!(seed.contains("상한 10줄/1KB"), "{seed}");
        assert!(seed.contains("상한 250줄/20KB"), "{seed}");
    }

    #[test]
    fn launch_seeds_the_interactive_agent_with_its_model() {
        let (caps, user_caps) = caps();
        // 로컬에 claude CLI가 없는 환경에서는 PATH 확인이 먼저 걸린다.
        let Ok(launch) = session_launch("claude", Some("opus"), &caps, &user_caps) else {
            return;
        };
        assert!(launch.bin.ends_with("claude"), "{}", launch.bin);
        assert_eq!(launch.args.first().map(String::as_str), Some("--model"));
        assert_eq!(launch.args.get(1).map(String::as_str), Some("opus"));
        assert_eq!(
            launch.args.last().map(String::as_str),
            Some(session_seed("claude", &caps, &user_caps).as_str())
        );
        assert!(!launch
            .args
            .iter()
            .any(|arg| arg == "-p" || arg.contains("skip-permissions")));
    }

    #[test]
    fn launch_rejects_an_agent_that_is_not_on_path() {
        let (caps, user_caps) = caps();
        let error =
            session_launch("/nonexistent/zzz-bin {prompt}", None, &caps, &user_caps).unwrap_err();
        assert_eq!(error, "/nonexistent/zzz-bin CLI를 PATH에서 찾을 수 없습니다");
        assert_eq!(
            session_launch("", None, &caps, &user_caps).unwrap_err(),
            "에이전트 명령을 만들 수 없습니다"
        );
    }

    #[test]
    fn open_target_must_live_inside_the_memory_root() {
        let root = Path::new("/vault/memory");
        assert_eq!(
            resolve_inside(root, "/vault/memory/praxis-3f9a1c2e/MEMORY.md").unwrap(),
            PathBuf::from("/vault/memory/praxis-3f9a1c2e/MEMORY.md")
        );
        assert!(resolve_inside(root, "/etc/passwd").is_err());
        assert!(resolve_inside(root, "/vault/memory/../../etc/passwd").is_err());
        assert!(resolve_inside(root, "USER.md").is_err());
        assert!(resolve_inside(root, "  ").is_err());
    }

    #[test]
    fn caps_outside_the_allowed_range_are_rejected_in_korean() {
        assert!(validate_caps(
            &Caps {
                lines: 0,
                bytes: 8192
            },
            "MEMORY.md"
        )
        .unwrap_err()
        .contains("줄 상한"));
        assert!(validate_caps(
            &Caps {
                lines: 2001,
                bytes: 8192
            },
            "MEMORY.md"
        )
        .is_err());
        assert!(validate_caps(
            &Caps {
                lines: 100,
                bytes: 262_145
            },
            "MEMORY.md"
        )
        .unwrap_err()
        .contains("바이트 상한"));
        assert!(validate_caps(
            &Caps {
                lines: 2000,
                bytes: 262_144
            },
            "MEMORY.md"
        )
        .is_ok());
    }
}
