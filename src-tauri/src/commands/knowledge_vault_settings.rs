use serde::Serialize;
use std::path::Path;
use tauri::State;

use crate::knowledge::vault;

use super::knowledge_vault::{
    binding_dto, pool_of, text, vaults, BindingDto, VaultDto,
};
use super::knowledge_vault_recovery::{
    binding_history, operation_conflicts, BindingHistoryDto, OperationConflictDto,
};
use super::{pool_of as app_pool, AppState};

/// 창고 채널의 설정 — 위키 폴더(창고 루트 기준 상대 경로), 정리 스킬 이름,
/// 위키를 열 때 먼저 띄울 진입 문서.
/// 필드 이름은 그대로 프론트 계약이다(serde rename 없음).
#[derive(Serialize)]
pub struct VaultSettingsDto {
    pub wiki_dir: String,
    pub organizer_skill: String,
    pub wiki_home: String,
}

impl From<vault::VaultSettings> for VaultSettingsDto {
    fn from(settings: vault::VaultSettings) -> Self {
        Self {
            wiki_dir: settings.wiki_dir,
            organizer_skill: settings.organizer_skill,
            wiki_home: settings.wiki_home,
        }
    }
}

#[tauri::command]
pub async fn knowledge_vault_settings_get(
    state: State<'_, AppState>,
) -> Result<VaultSettingsDto, String> {
    Ok(vault::settings::load(&pool_of(&state)?).await.into())
}

#[tauri::command]
pub async fn knowledge_vault_settings_set(
    state: State<'_, AppState>,
    wiki_dir: String,
    organizer_skill: String,
    wiki_home: String,
) -> Result<VaultSettingsDto, String> {
    Ok(
        vault::settings::store(&pool_of(&state)?, &wiki_dir, &organizer_skill, &wiki_home)
            .await?
            .into(),
    )
}

#[derive(Serialize)]
pub struct CaptureConsentDto {
    pub id: String,
    pub provider: String,
}
#[derive(Serialize)]
pub struct VaultStatusDto {
    pub supported: bool,
    pub vaults: Vec<VaultDto>,
    pub index_rebuild_needed: bool,
    pub provider_available: bool,
    pub current_provider: String,
    pub current_provider_display: String,
    pub project_binding: Option<BindingDto>,
    pub current_consent: Option<CaptureConsentDto>,
    pub prior_bindings: Vec<BindingHistoryDto>,
    pub operation_conflicts: Vec<OperationConflictDto>,
}

#[tauri::command]
pub async fn knowledge_vault_status(
    state: State<'_, AppState>,
    repo_root: Option<String>,
) -> Result<VaultStatusDto, String> {
    let pool = app_pool(&state)?;
    let profile = crate::capture::invoke::profile(&pool).await;
    let provider = crate::capture::invoke::provider_identity(&profile);
    let project = if cfg!(target_os = "macos") {
        match repo_root {
            Some(root) => match std::fs::metadata(&root) {
                Ok(metadata) if metadata.is_dir() => {
                    vault::resolve_project(&pool, Path::new(&root))
                        .await
                        .map_err(text)?
                }
                Ok(_) => None,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(text(error)),
            },
            None => None,
        }
    } else {
        None
    };
    let current_consent = match &project {
        Some(binding) => vault::provenance::active_consent(&pool, binding, &provider)
            .await
            .map_err(text)?
            .map(|id| CaptureConsentDto {
                id,
                provider: provider.clone(),
            }),
        None => None,
    };
    Ok(VaultStatusDto {
        supported: cfg!(target_os = "macos"),
        vaults: vaults(&pool).await.map_err(text)?,
        index_rebuild_needed: vault::rebuild_needed(&pool).await.map_err(text)?,
        provider_available: crate::reviewer::which("claude").is_some(),
        current_provider: provider,
        current_provider_display: crate::capture::invoke::provider_display(&profile),
        project_binding: project.map(binding_dto),
        current_consent,
        prior_bindings: binding_history(&pool).await.map_err(text)?,
        operation_conflicts: operation_conflicts(&pool).await.map_err(text)?,
    })
}
