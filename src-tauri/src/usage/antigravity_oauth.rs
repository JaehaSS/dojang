//! 앱을 꺼둔 상태에서도 Antigravity/agy가 저장한 로그인으로 quota를 조회한다.
//! Keychain(service=gemini, account=antigravity)은 읽기 전용이고 갱신 토큰은 메모리에서만 보관한다.

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use std::time::Duration;

use super::VendorUsage;

const CLOUD_URLS: [&str; 2] = [
    "https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary",
    "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary",
];
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
// Google의 installed-app OAuth client(설치된 agy/Hub 바이너리에 같은 값이 있다). 사용자의 비밀키는
// 아니지만 저장소에는 두지 않는다 — 빌드 타임에 주입하고, 없으면 refresh만 꺼진다(ADR 0147과 같은 방식).
const CLIENT_ID: Option<&str> = option_env!("PRAXIS_ANTIGRAVITY_CLIENT_ID");
const CLIENT_SECRET: Option<&str> = option_env!("PRAXIS_ANTIGRAVITY_CLIENT_SECRET");

// 사용자 토큰을 Debug/Serialize로 노출하지 않는다.
struct Credentials {
    access: Option<String>,
    refresh: Option<String>,
    expires_at: Option<i64>,
}

struct CachedToken {
    credential: [u8; 32],
    access: String,
    expires_at: i64,
}

static TOKEN_CACHE: Mutex<Option<CachedToken>> = Mutex::new(None);

impl Credentials {
    fn fingerprint(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        if let Some(refresh) = &self.refresh {
            hash.update(b"refresh:");
            hash.update(refresh);
        } else if let Some(access) = &self.access {
            hash.update(b"access:");
            hash.update(access);
        }
        hash.finalize().into()
    }

    fn usable_access(&self, now: i64) -> Option<&str> {
        self.access.as_deref().filter(|_| {
            self.expires_at
                .map(|expires| expires > now + 60)
                .unwrap_or(true)
        })
    }
}

fn parse_credentials(raw: &str) -> Option<Credentials> {
    let raw = raw.trim().trim_start_matches('\u{feff}');
    let decoded;
    let raw = if let Some(encoded) = raw.strip_prefix("go-keyring-base64:") {
        decoded = String::from_utf8(STANDARD.decode(encoded).ok()?).ok()?;
        decoded.as_str()
    } else {
        raw
    };
    let envelope: Value = serde_json::from_str(raw).ok()?;
    let token = envelope.get("token").unwrap_or(&envelope);
    let text = |key| {
        token
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let access = text("access_token");
    let refresh = text("refresh_token");
    if access.is_none() && refresh.is_none() {
        return None;
    }
    let expires_at = match token.get("expiry") {
        None => None,
        Some(value) => Some(
            chrono::DateTime::parse_from_rfc3339(value.as_str()?)
                .ok()?
                .timestamp(),
        ),
    };
    Some(Credentials {
        access,
        refresh,
        expires_at,
    })
}

fn cached_access<'a>(
    cache: &'a Option<CachedToken>,
    credentials: &Credentials,
    now: i64,
) -> Option<&'a str> {
    cache
        .as_ref()
        .filter(|token| {
            token.credential == credentials.fingerprint() && token.expires_at > now + 60
        })
        .map(|token| token.access.as_str())
}

fn blank(status: &str, detail: &str) -> VendorUsage {
    VendorUsage::blank("agy", "Antigravity", status, Some(detail.into()))
}

enum CredentialError {
    Missing,
    Unreadable,
    Malformed,
}

fn credential_error(error: CredentialError) -> VendorUsage {
    match error {
        CredentialError::Missing => blank(
            "unauthenticated",
            "저장된 Antigravity 로그인이 없습니다. 앱 또는 agy에서 한 번 로그인하세요",
        ),
        CredentialError::Unreadable => {
            blank("error", "Antigravity Keychain 로그인에 접근하지 못했습니다")
        }
        CredentialError::Malformed => {
            blank("error", "저장된 Antigravity 로그인 형식을 읽지 못했습니다")
        }
    }
}

#[cfg(target_os = "macos")]
async fn load_credentials() -> Result<Credentials, CredentialError> {
    let mut command = tokio::process::Command::new("/usr/bin/security");
    command
        .args([
            "find-generic-password",
            "-s",
            "gemini",
            "-a",
            "antigravity",
            "-w",
        ])
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(5), command.output())
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or(CredentialError::Unreadable)?;
    if !output.status.success() {
        return Err(if output.status.code() == Some(44) {
            CredentialError::Missing
        } else {
            CredentialError::Unreadable
        });
    }
    let raw = String::from_utf8(output.stdout).ok();
    raw.as_deref()
        .and_then(parse_credentials)
        .ok_or(CredentialError::Malformed)
}

enum ApiError {
    Rejected,
    Unavailable,
    NoData,
    NoClient,
}

fn status_error(status: u16) -> Option<ApiError> {
    match status {
        200..=299 => None,
        401 | 403 => Some(ApiError::Rejected),
        _ => Some(ApiError::Unavailable),
    }
}

async fn quota(client: &reqwest::Client, access: &str, now: i64) -> Result<VendorUsage, ApiError> {
    for url in CLOUD_URLS {
        let Ok(response) = client
            .post(url)
            .bearer_auth(access)
            .header("User-Agent", "antigravity")
            .json(&serde_json::json!({}))
            .send()
            .await
        else {
            continue;
        };
        match status_error(response.status().as_u16()) {
            Some(ApiError::Rejected) => return Err(ApiError::Rejected),
            Some(_) => continue,
            None => {}
        }
        let value = response
            .json::<Value>()
            .await
            .map_err(|_| ApiError::NoData)?;
        // 빈 quota도 이 소스의 응답이다. 다른 API나 가득 남은 값으로 대체하지 않는다.
        let mut usage = super::antigravity::parse_summary(&value, now).ok_or(ApiError::NoData)?;
        usage.source = Some("antigravity-oauth".into());
        return Ok(usage);
    }
    Err(ApiError::Unavailable)
}

fn refresh_response(
    status: u16,
    value: &Value,
    credential: [u8; 32],
    now: i64,
) -> Result<CachedToken, ApiError> {
    if !(200..300).contains(&status) {
        // invalid_grant만 로그인 만료로 처리한다. 429·5xx·client 설정 오류는 일시 실패다.
        return Err(
            if status == 400 && value.get("error").and_then(Value::as_str) == Some("invalid_grant")
            {
                ApiError::Rejected
            } else {
                ApiError::Unavailable
            },
        );
    }
    let access = value
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or(ApiError::Unavailable)?;
    let expires_in = value
        .get("expires_in")
        .and_then(Value::as_i64)
        .filter(|seconds| (61..=86400).contains(seconds))
        .ok_or(ApiError::Unavailable)?;
    Ok(CachedToken {
        credential,
        access: access.into(),
        expires_at: now.saturating_add(expires_in),
    })
}

async fn refresh(
    client: &reqwest::Client,
    credentials: &Credentials,
    now: i64,
) -> Result<CachedToken, ApiError> {
    let refresh = credentials.refresh.as_deref().ok_or(ApiError::Rejected)?;
    let (Some(client_id), Some(client_secret)) = (CLIENT_ID, CLIENT_SECRET) else {
        return Err(ApiError::NoClient);
    };
    let response = client
        .post(TOKEN_URL)
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("refresh_token", refresh),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|_| ApiError::Unavailable)?;
    let status = response.status().as_u16();
    let value = response
        .json::<Value>()
        .await
        .map_err(|_| ApiError::Unavailable)?;
    refresh_response(status, &value, credentials.fingerprint(), now)
}

fn error_card(error: ApiError) -> VendorUsage {
    match error {
        ApiError::Rejected => blank(
            "unauthenticated",
            "Antigravity 로그인이 만료됐습니다. 앱 또는 agy에서 다시 로그인하세요",
        ),
        ApiError::Unavailable => blank(
            "error",
            "Antigravity 사용량 API에 일시적으로 접근하지 못했습니다",
        ),
        ApiError::NoData => blank(
            "no_data",
            "Antigravity 사용량 API 응답에 표시할 한도 정보가 없습니다",
        ),
        ApiError::NoClient => blank(
            "unauthenticated",
            "이 빌드는 Antigravity 토큰을 갱신하지 못합니다. agy를 한 번 실행해 토큰을 새로 받으세요",
        ),
    }
}

async fn fetch_with_credentials(
    client: &reqwest::Client,
    credentials: &Credentials,
    now: i64,
) -> VendorUsage {
    let cached = TOKEN_CACHE
        .lock()
        .ok()
        .and_then(|cache| cached_access(&cache, credentials, now).map(str::to_string));
    let access = cached.or_else(|| credentials.usable_access(now).map(str::to_string));
    if let Some(access) = access {
        match quota(client, &access, now).await {
            Ok(usage) => return usage,
            // 일시 장애에는 refresh를 반복하지 않는다. 인증 거절일 때만 갱신한다.
            Err(ApiError::Rejected) => {}
            Err(error) => return error_card(error),
        }
    }
    let refreshed = match refresh(client, credentials, now).await {
        Ok(token) => token,
        Err(error) => return error_card(error),
    };
    let result = quota(client, &refreshed.access, now).await;
    if let Ok(mut cache) = TOKEN_CACHE.lock() {
        // 거절된 token은 다음 폴링에 남기지 않는다. 네트워크 장애 시에는 유효한 token을 재사용한다.
        *cache = if matches!(&result, Err(ApiError::Rejected)) {
            None
        } else {
            Some(refreshed)
        };
    }
    result.unwrap_or_else(error_card)
}

#[cfg(target_os = "macos")]
pub(super) async fn fetch(now: i64) -> VendorUsage {
    // 매번 Keychain을 확인해 로그아웃·계정 변경 후 이전 token이 재사용되는 것을 막는다.
    let credentials = match load_credentials().await {
        Ok(credentials) => credentials,
        Err(error) => {
            if let Ok(mut cache) = TOKEN_CACHE.lock() {
                *cache = None;
            }
            return credential_error(error);
        }
    };
    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(6))
        .build()
    {
        Ok(client) => client,
        Err(_) => return error_card(ApiError::Unavailable),
    };
    fetch_with_credentials(&client, &credentials, now).await
}

#[cfg(not(target_os = "macos"))]
pub(super) async fn fetch(_now: i64) -> VendorUsage {
    blank(
        "unsupported",
        "이 플랫폼의 Antigravity 사용량 조회는 아직 지원하지 않습니다",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn credentials() -> Credentials {
        parse_credentials(r#"{"token":{"access_token":"dummy-access","refresh_token":"dummy-refresh","expiry":"2030-10-08T08:52:18+09:00"}}"#).unwrap()
    }

    #[test]
    fn reads_installed_keychain_envelope_and_expiry_without_logging_tokens() {
        let raw = r#"{"token":{"access_token":"dummy-access","refresh_token":"dummy-refresh","expiry":"2030-10-08T08:52:18+09:00"},"id_token":"unused"}"#;
        let wrapped = format!("go-keyring-base64:{}", STANDARD.encode(raw));
        let token = parse_credentials(&wrapped).unwrap();
        assert_eq!(token.access.as_deref(), Some("dummy-access"));
        assert_eq!(token.refresh.as_deref(), Some("dummy-refresh"));
        let expiry = token.expires_at.unwrap();
        assert!(token.usable_access(expiry - 61).is_some());
        assert!(token.usable_access(expiry - 60).is_none());
        assert!(token.usable_access(expiry + 1).is_none());
    }

    #[test]
    fn malformed_credentials_never_become_bearer_tokens() {
        for raw in [
            "",
            "raw-token",
            "{}",
            "[]",
            "go-keyring-base64:!",
            "{\"id_token\":\"unused\"}",
            "{\"token\":{\"access_token\":\"dummy\",\"expiry\":\"bad\"}}",
        ] {
            assert!(parse_credentials(raw).is_none());
        }
        assert!(parse_credentials(r#"{"token":{"refresh_token":"dummy"}}"#).is_some());
    }

    #[test]
    fn refreshed_token_cache_is_bound_to_current_login_and_expiry() {
        let source = credentials();
        let cache = Some(CachedToken {
            credential: source.fingerprint(),
            access: "derived".into(),
            expires_at: 500,
        });
        assert_eq!(cached_access(&cache, &source, 400), Some("derived"));
        assert!(cached_access(&cache, &source, 440).is_none());
        let changed =
            parse_credentials(r#"{"token":{"refresh_token":"another-account"}}"#).unwrap();
        assert!(cached_access(&cache, &changed, 400).is_none());
        let renewed = parse_credentials(
            r#"{"token":{"access_token":"renewed","refresh_token":"dummy-refresh"}}"#,
        )
        .unwrap();
        assert_eq!(cached_access(&cache, &renewed, 400), Some("derived"));
    }

    #[test]
    fn validates_refresh_response_and_distinguishes_expiry_from_outages() {
        let response = json!({"access_token":"derived","expires_in":3600});
        let token = refresh_response(200, &response, [1; 32], 1000)
            .ok()
            .unwrap();
        assert_eq!(token.expires_at, 4600);
        assert!(matches!(
            refresh_response(400, &json!({"error":"invalid_grant"}), [0; 32], 0),
            Err(ApiError::Rejected)
        ));
        for status in [400, 401, 429, 503] {
            assert!(matches!(
                refresh_response(
                    status,
                    &json!({"error":"temporarily_unavailable"}),
                    [0; 32],
                    0
                ),
                Err(ApiError::Unavailable)
            ));
        }
        for value in [
            json!({}),
            json!({"access_token":"","expires_in":3600}),
            json!({"access_token":"derived","expires_in":0}),
        ] {
            assert!(matches!(
                refresh_response(200, &value, [0; 32], 0),
                Err(ApiError::Unavailable)
            ));
        }
        assert!(matches!(status_error(401), Some(ApiError::Rejected)));
        assert!(matches!(status_error(429), Some(ApiError::Unavailable)));
    }

    /// 앱의 로컬 API를 호출하지 않고 Google API로 실환경을 확인한다.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "requires existing Antigravity Keychain login and network"]
    async fn live_remote_quota_without_app() {
        let usage = fetch(super::super::now_secs()).await;
        assert_eq!(usage.status, "ok", "{:?}", usage.detail);
        assert_eq!(usage.source.as_deref(), Some("antigravity-oauth"));
        assert_eq!(usage.quota_groups.len(), 2);
        assert!(usage.five_hour.is_some());
        assert!(usage.weekly.is_some());
        println!("Antigravity remote quota: 2 groups, 5h and weekly present; no app RPC");
    }

    /// 실제 refresh grant로 만료 경로를 확인한다. 원래 Keychain은 변경하지 않는다.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "performs an OAuth refresh using existing Antigravity login"]
    async fn live_refresh_then_remote_quota() {
        let mut credentials = load_credentials().await.ok().expect("Keychain login");
        credentials.access = None;
        credentials.expires_at = Some(0);
        *TOKEN_CACHE.lock().unwrap() = None;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(6))
            .build()
            .unwrap();
        let usage = fetch_with_credentials(&client, &credentials, super::super::now_secs()).await;
        assert_eq!(usage.status, "ok", "{:?}", usage.detail);
        assert_eq!(usage.source.as_deref(), Some("antigravity-oauth"));
        assert!(!usage.quota_groups.is_empty());
        println!("Antigravity OAuth refresh and remote quota verified; Keychain unchanged");
    }
}
