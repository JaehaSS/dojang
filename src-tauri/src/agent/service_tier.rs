//! Local Codex session speed. A missing override preserves existing CLI defaults.

use serde_json::Value;
use std::path::PathBuf;

pub fn normalize(value: Option<&str>) -> Result<Option<&str>, String> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value @ ("default" | "fast")) => Ok(Some(value)),
        Some(_) => Err("지원하지 않는 Codex 실행 속도입니다".into()),
    }
}

fn fast_models(value: &Value) -> Vec<String> {
    let mut models: Vec<String> = value["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|model| {
            model["additional_speed_tiers"]
                .as_array()
                .is_some_and(|tiers| tiers.iter().any(|tier| tier.as_str() == Some("fast")))
                || model["service_tiers"].as_array().is_some_and(|tiers| {
                    tiers
                        .iter()
                        .any(|tier| matches!(tier["id"].as_str(), Some("priority" | "fast")))
                })
        })
        .filter_map(|model| {
            model["slug"]
                .as_str()
                .filter(|id| !id.trim().is_empty())
                .map(str::to_string)
        })
        .collect();
    models.sort();
    models.dedup();
    models
}

pub fn supported_models() -> Vec<String> {
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| crate::usage::home_dir().map(|home| home.join(".codex")));
    let Some(path) = home.map(|home| home.join("models_cache.json")) else {
        return Vec::new();
    };
    // The cache is provider-owned and optional; do not fetch models or start a paid turn here.
    use std::io::Read;
    let value = std::fs::File::open(path).ok().and_then(|file| {
        let mut bytes = Vec::new();
        file.take(4 * 1024 * 1024).read_to_end(&mut bytes).ok()?;
        serde_json::from_slice(&bytes).ok()
    });
    value.as_ref().map(fast_models).unwrap_or_default()
}

pub fn validate(
    agent: &str,
    model: &str,
    tier: Option<&str>,
    models: &[String],
) -> Result<(), String> {
    let Some(tier) = normalize(tier)? else {
        return Ok(());
    };
    if agent.trim() != "codex" {
        return Err("실행 속도는 Codex 대화에서만 선택할 수 있습니다".into());
    }
    if tier == "fast" && !models.iter().any(|id| id == model.trim()) {
        return Err("Fast 지원을 확인한 Codex 모델을 직접 선택해 주세요".into());
    }
    Ok(())
}

pub fn apply(command: &mut std::process::Command, tier: Option<&str>) -> Result<(), String> {
    if let Some(tier) = normalize(tier)? {
        command.args(["-c", &format!("service_tier=\"{tier}\"")]);
        if tier == "fast" {
            command.args(["-c", "features.fast_mode=true"]);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn capabilities_require_explicit_fast_metadata() {
        let models = fast_models(&json!({"models":[
            {"slug":"a","additional_speed_tiers":["fast"]},
            {"slug":"b","service_tiers":[{"id":"priority"}]},
            {"slug":"c"}, {"slug":"a","service_tiers":[{"id":"fast"}]}
        ]}));
        assert_eq!(models, ["a", "b"]);
        assert!(validate("codex", "a", Some("fast"), &models).is_ok());
        assert!(validate("codex", "", Some("fast"), &models).is_err());
        assert!(validate("codex", "c", Some("fast"), &models).is_err());
        assert!(validate("claude", "a", Some("fast"), &models).is_err());
        assert!(validate("codex", "c", Some("default"), &[]).is_ok());
        assert!(normalize(Some("priority")).is_err());
    }

    #[test]
    fn standard_explicitly_overrides_global_fast_and_none_inherits() {
        for (tier, expected) in [
            (None, vec![]),
            (Some("default"), vec!["-c", "service_tier=\"default\""]),
            (
                Some("fast"),
                vec![
                    "-c",
                    "service_tier=\"fast\"",
                    "-c",
                    "features.fast_mode=true",
                ],
            ),
        ] {
            let mut command = std::process::Command::new("codex");
            apply(&mut command, tier).unwrap();
            assert_eq!(
                command
                    .get_args()
                    .map(|arg| arg.to_str().unwrap())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
}
