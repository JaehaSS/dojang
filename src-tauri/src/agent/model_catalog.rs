//! 설치된 agy의 모델 목록. 동시 요청은 합치고, 조회 실패 시 마지막 성공 목록을 보존한다.

use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::Mutex;

const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const RETRY_AFTER: Duration = Duration::from_secs(30);
const QUERY_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
}

#[derive(Default)]
struct Cache {
    models: Vec<ModelOption>,
    result: Option<Result<Vec<ModelOption>, String>>,
    expires_at: Option<Instant>,
}

impl Cache {
    fn fresh(&self, now: Instant) -> Option<Result<Vec<ModelOption>, String>> {
        self.expires_at
            .filter(|expires| now < *expires)
            .and_then(|_| self.result.clone())
    }

    fn store(
        &mut self,
        result: Result<Vec<ModelOption>, String>,
        now: Instant,
    ) -> Result<Vec<ModelOption>, String> {
        let ttl = if result.is_ok() {
            CACHE_TTL
        } else {
            RETRY_AFTER
        };
        let result = match result {
            Ok(models) => {
                self.models = models.clone();
                Ok(models)
            }
            Err(_) if !self.models.is_empty() => Ok(self.models.clone()),
            Err(error) => Err(error),
        };
        self.expires_at = Some(now + ttl);
        self.result = Some(result.clone());
        result
    }
}

/// `agy models`는 진행 안내와 함께 탭으로 구분한 ID/표시명을 출력한다.
fn parse_models(raw: &str) -> Result<Vec<ModelOption>, String> {
    let mut models = Vec::new();
    for line in raw.lines() {
        let Some((id, label)) = line.split_once('\t') else {
            continue;
        };
        let (id, label) = (id.trim(), label.trim());
        if id.is_empty()
            || label.is_empty()
            || id.chars().any(char::is_whitespace)
            || models.iter().any(|model: &ModelOption| model.id == id)
        {
            continue;
        }
        models.push(ModelOption {
            id: id.into(),
            label: label.into(),
        });
    }
    if models.is_empty() {
        Err("agy에서 모델 목록을 읽지 못했습니다".into())
    } else {
        Ok(models)
    }
}

async fn query_models(bin: &str, timeout: Duration) -> Result<Vec<ModelOption>, String> {
    let mut command = tokio::process::Command::new(bin);
    command
        .arg("models")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(timeout, command.output())
        .await
        .map_err(|_| "agy 모델 조회 시간이 초과되었습니다".to_string())?
        .map_err(|error| format!("agy 모델 조회에 실패했습니다: {error}"))?;
    if !output.status.success() {
        return Err("agy 모델 조회에 실패했습니다".into());
    }
    parse_models(&String::from_utf8_lossy(&output.stdout))
}

pub async fn available_models(agent: &str) -> Result<Vec<ModelOption>, String> {
    if !matches!(agent.trim(), "agy" | "gemini" | "antigravity") {
        return Ok(Vec::new());
    }
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    // 잠금을 유지해 여러 모델 선택창이 동시에 열려도 CLI는 한 번만 실행한다.
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(Cache::default()))
        .lock()
        .await;
    if let Some(result) = cache.fresh(Instant::now()) {
        return result;
    }
    let result = match crate::reviewer::which("agy") {
        Some(bin) => query_models(&bin, QUERY_TIMEOUT).await,
        None => Err("agy 실행 파일을 찾지 못했습니다".into()),
    };
    cache.store(result, Instant::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_models_and_skips_progress_malformed_rows_and_duplicates() {
        let models = parse_models(
            "Fetching available models...\n\
            gemini-4-argon-high\tGemini 4 Argon (High)\r\n\
            gemini-4-argon-high\tDuplicate\ninvalid id\tInvalid\n\tEmpty ID\nempty-label\t\n\
            gemini-4-argon-low\tGemini 4 Argon (Low)\n",
        )
        .unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gemini-4-argon-high");
        assert_eq!(models[0].label, "Gemini 4 Argon (High)");
        assert_eq!(models[1].id, "gemini-4-argon-low");
        assert!(parse_models("Fetching available models...\n").is_err());
    }

    #[test]
    fn cache_expires_and_retries_failures_without_losing_last_success() {
        let now = Instant::now();
        let mut cache = Cache::default();
        assert!(cache.store(Err("offline".into()), now).is_err());
        assert!(cache.fresh(now).unwrap().is_err());
        assert!(cache.fresh(now + RETRY_AFTER).is_none());
        let models = parse_models("gemini-4-argon-high\tArgon").unwrap();
        assert_eq!(cache.store(Ok(models.clone()), now).unwrap(), models);
        assert_eq!(cache.fresh(now).unwrap().unwrap(), models);
        assert!(cache.fresh(now + CACHE_TTL).is_none());
        assert_eq!(cache.store(Err("offline".into()), now).unwrap(), models);
        assert!(cache.fresh(now + RETRY_AFTER).is_none());
    }

    #[tokio::test]
    async fn other_agents_do_not_query_agy() {
        assert!(available_models("codex").await.unwrap().is_empty());
    }

    #[cfg(unix)]
    fn stub(name: &str, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = crate::testtmp::dir().join(format!("model-catalog-{name}"));
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn command_checks_exit_status_and_model_output() {
        let bin = stub(
            "success",
            "test \"$1\" = models || exit 2\nprintf 'gemini-4-argon-high\\tArgon\\n'",
        );
        assert_eq!(
            query_models(&bin, QUERY_TIMEOUT).await.unwrap()[0].id,
            "gemini-4-argon-high"
        );
        let bin = stub("failure", "printf 'gemini-4-argon-high\\tArgon\\n'\nexit 1");
        assert!(query_models(&bin, QUERY_TIMEOUT).await.is_err());
        let bin = stub("empty", "echo 'Fetching available models...'");
        assert!(query_models(&bin, QUERY_TIMEOUT).await.is_err());
        assert!(query_models("/nonexistent/dojang-agy", QUERY_TIMEOUT)
            .await
            .is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_the_cli_process() {
        let bin = stub("timeout", "echo $$ > \"$0.pid\"\nexec /bin/sleep 30");
        // macOS의 첫 실행 검사까지 포함해 스텁이 PID를 기록할 시간을 준다.
        let result = query_models(&bin, Duration::from_secs(2)).await;
        assert!(result.unwrap_err().contains("초과"));
        let pid: i32 = std::fs::read_to_string(format!("{bin}.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        for _ in 0..100 {
            if nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGKILL,
        );
        panic!("timed-out model query process {pid} is still alive");
    }

    #[tokio::test]
    #[ignore = "requires installed and authenticated agy; performs a read-only model lookup"]
    async fn installed_agy_models_are_readable() {
        let models = available_models("agy").await.unwrap();
        assert!(!models.is_empty());
        assert!(models.iter().any(|model| model.id.starts_with("gemini-")));
        println!("{}", serde_json::to_string(&models).unwrap());
    }
}
