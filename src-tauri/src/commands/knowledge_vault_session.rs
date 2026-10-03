//! 창고 정리 세션 — 창고 루트로 프로젝트 에디터 창을 열고, 그 창의 PTY를 일반 셸 대신
//! 에이전트 CLI + seed 지시문으로 띄운다. 정리 규칙은 앱 밖 스킬이 가진다 — 어떤
//! 스킬인지와 위키 폴더가 어디인지는 설정에서 온다(`vault::settings`).
//!
//! 세션은 대화형이다 — 헤드리스·자동 승인 플래그는 쓰지 않는다(설계 D3).

use std::path::PathBuf;

use serde::Serialize;
use sqlx::Row;
use tauri::{AppHandle, Runtime, State, WebviewWindow};

use crate::knowledge::vault;
use crate::project_editor::{self, ProjectEditorState, ProjectLaunch};

use super::knowledge_vault::{pool_of, text};
use super::AppState;

#[derive(Serialize)]
pub struct SessionOpenDto {
    pub label: String,
    pub root: String,
    /// 어떤 스킬을 찾았는지 — 안내 문구는 화면이 이 이름으로 만든다.
    pub skill: String,
    pub skill_present: bool,
    pub warning: Option<String>,
}

fn skill_relative_path(skill: &str) -> String {
    format!(".claude/skills/{skill}/SKILL.md")
}

#[tauri::command]
pub async fn knowledge_vault_session_open<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    editor: State<'_, ProjectEditorState>,
    window: WebviewWindow<R>,
    vault_id: String,
    agent: String,
) -> Result<SessionOpenDto, String> {
    if window.label() != "main" {
        return Err("정리 세션은 메인 창에서만 열 수 있습니다".into());
    }
    let pool = pool_of(&state)?;
    let root = session_root(&pool, &vault_id).await?;
    let settings = vault::settings::load(&pool).await;
    let model = crate::db::get_setting(&pool, &format!("model:{}", agent.trim()))
        .await
        .ok()
        .flatten()
        .filter(|value| !value.trim().is_empty());
    let launch = session_launch(
        agent.trim(),
        model.as_deref(),
        &settings.organizer_skill,
        &settings.wiki_dir,
    )?;
    let info = project_editor::open_with_launch(&app, &editor, PathBuf::from(root), Some(launch))
        .await?;
    let relative = skill_relative_path(&settings.organizer_skill);
    let skill_present = skill_path(&relative).is_some_and(|path| path.is_file());
    let skill = settings.organizer_skill;
    Ok(SessionOpenDto {
        label: info.label,
        root: info.root,
        skill_present,
        warning: (!skill_present)
            .then(|| format!("{skill} 스킬이 없습니다 — ~/{relative} 에 설치하세요")),
        skill,
    })
}

async fn session_root(pool: &sqlx::SqlitePool, vault_id: &str) -> Result<String, String> {
    let row = sqlx::query("SELECT canonical_root, enabled, writable FROM vaults WHERE id = ?")
        .bind(vault_id)
        .fetch_optional(pool)
        .await
        .map_err(text)?
        .ok_or("창고를 찾을 수 없습니다")?;
    if row.get::<i64, _>("enabled") == 0 {
        return Err("연결이 끊긴 창고입니다".into());
    }
    if row.get::<i64, _>("writable") == 0 {
        return Err("읽기 전용 창고에서는 정리 세션을 열 수 없습니다".into());
    }
    Ok(row.get::<String, _>("canonical_root"))
}

/// 에이전트별 seed 지시문. 규칙 본문은 스킬이 가지므로 여기엔 스킬 호출과 경계만 남는다.
/// Claude Code만 `/이름`이 스킬 호출이고, 나머지는 평문으로 이름을 부른다.
fn session_seed(agent: &str, skill: &str, wiki_dir: &str) -> String {
    let boundary =
        format!("이 폴더는 Dojang 개인 지식창고다. 위키 문서는 {wiki_dir}/ 아래에만 쓴다.");
    match agent {
        "claude" => format!("/{skill} {boundary}"),
        _ => format!("{skill} 스킬을 사용해라. {boundary}"),
    }
}

/// (bin, args) 결정 — PTY는 PATH를 풀지 않으므로 bin은 `which`가 준 절대 경로다.
pub fn session_launch(
    agent: &str,
    model: Option<&str>,
    skill: &str,
    wiki_dir: &str,
) -> Result<ProjectLaunch, String> {
    let seed = session_seed(agent, skill, wiki_dir);
    let (bin, args) = crate::agent::agent_args_with_effort(agent, &seed, model, None)
        .ok_or("에이전트 명령을 만들 수 없습니다")?;
    let bin = crate::reviewer::which(&bin)
        .ok_or_else(|| format!("{bin} CLI를 PATH에서 찾을 수 없습니다"))?;
    Ok(ProjectLaunch { bin, args })
}

fn skill_path(relative: &str) -> Option<PathBuf> {
    crate::usage::home_dir().map(|home| home.join(relative))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKILL: &str = vault::settings::DEFAULT_ORGANIZER_SKILL;
    const WIKI: &str = vault::settings::DEFAULT_WIKI_DIR;

    #[test]
    fn seed_calls_the_skill_by_slash_only_for_claude() {
        assert!(session_seed("claude", SKILL, WIKI).starts_with("/wiki-organizer "));
        assert!(session_seed("codex", SKILL, WIKI).starts_with("wiki-organizer 스킬을 사용해라."));
        assert!(session_seed("agy", SKILL, WIKI).contains("위키 문서는 wiki/ 아래에만 쓴다."));
    }

    #[test]
    fn seed_carries_the_configured_skill_and_wiki_folder() {
        let claude = session_seed("claude", "knowledge-harness", "문서/기술-위키/wiki");
        assert!(claude.starts_with("/knowledge-harness "));
        assert!(claude.contains("이 폴더는 Dojang 개인 지식창고다."));
        assert!(claude.contains("위키 문서는 문서/기술-위키/wiki/ 아래에만 쓴다."));
        let other = session_seed("codex", "knowledge-harness", "문서/기술-위키/wiki");
        assert!(other.starts_with("knowledge-harness 스킬을 사용해라."));
        assert!(other.contains("위키 문서는 문서/기술-위키/wiki/ 아래에만 쓴다."));
        // 옛 하드코딩이 남아 있으면 여기서 걸린다.
        assert!(!other.contains("wiki-organizer"));
    }

    #[test]
    fn skill_path_follows_the_configured_skill_name() {
        assert_eq!(
            skill_relative_path("knowledge-harness"),
            ".claude/skills/knowledge-harness/SKILL.md"
        );
    }

    #[test]
    fn launch_seeds_the_interactive_agent_with_its_model() {
        // 로컬에 claude CLI가 없는 환경에서는 PATH 확인이 먼저 걸린다.
        let Ok(launch) = session_launch("claude", Some("opus"), SKILL, WIKI) else {
            return;
        };
        assert!(launch.bin.ends_with("claude"), "{}", launch.bin);
        assert_eq!(launch.args.first().map(String::as_str), Some("--model"));
        assert_eq!(launch.args.get(1).map(String::as_str), Some("opus"));
        let seed = session_seed("claude", SKILL, WIKI);
        assert_eq!(launch.args.last().map(String::as_str), Some(seed.as_str()));
        assert!(!launch
            .args
            .iter()
            .any(|arg| arg == "-p" || arg.contains("skip-permissions")));
    }

    #[test]
    fn launch_rejects_an_agent_that_is_not_on_path() {
        let error = session_launch("/nonexistent/zzz-bin {prompt}", None, SKILL, WIKI).unwrap_err();
        assert_eq!(error, "/nonexistent/zzz-bin CLI를 PATH에서 찾을 수 없습니다");
        assert_eq!(
            session_launch("", None, SKILL, WIKI).unwrap_err(),
            "에이전트 명령을 만들 수 없습니다"
        );
    }
}
