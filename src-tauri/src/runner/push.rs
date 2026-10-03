//! Web Push — 검토 대기가 생기면 폰을 깨운다. (설계 0013 §8)
//!
//! **페이로드를 싣지 않는다.** 브라우저를 깨우기만 하고, Service Worker가 동일 출처
//! `/v1`을 다시 조회해 알림 문구를 만든다. 두 가지 이득이 있다.
//!   1. 작업 내용이 FCM/Apple 서버를 통과하지 않는다.
//!   2. ECDH·HKDF·AES-GCM 페이로드 암호화 스택이 통째로 필요 없어진다 — 남는 암호 요구는
//!      VAPID JWT(ES256) 서명 하나뿐이다.
//!
//! 발송은 EventHub 구독으로 붙는다. 상태 전이 호출부(process.rs 등)를 건드리지 않는
//! 단일 지점이다.

use std::path::{Path, PathBuf};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use sqlx::{Row, SqlitePool};

use crate::db::RunnerEvent;
use crate::runner::events::EventHub;

/// JWT 수명. 표준 권고 상한은 24시간이며, 여유를 두고 12시간으로 잡는다.
const VAPID_TTL_SECS: i64 = 12 * 60 * 60;
/// 폰이 꺼져 있어도 하루는 보관되게 한다.
const PUSH_TTL_SECS: u32 = 24 * 60 * 60;
/// 연속 실패가 이 값을 넘으면 죽은 구독으로 보고 지운다.
const MAX_FAILURES: i64 = 5;

const MIGRATION: &str = "
CREATE TABLE IF NOT EXISTS push_subscriptions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id INTEGER,
  endpoint TEXT NOT NULL UNIQUE,
  created_at INTEGER NOT NULL,
  last_ok_at INTEGER,
  failure_count INTEGER NOT NULL DEFAULT 0
);
";

pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    let mut connection = pool.acquire().await?;
    for statement in MIGRATION.split(';').filter(|s| !s.trim().is_empty()) {
        sqlx::query(statement).execute(&mut *connection).await?;
    }
    Ok(())
}

/// VAPID 키쌍. private은 P-256 스칼라 32바이트, public은 비압축 SEC1 포인트 65바이트.
#[derive(Clone)]
pub struct VapidKeys {
    signing: SigningKey,
    public: Vec<u8>,
}

impl VapidKeys {
    /// 파일이 있으면 읽고, 없으면 만들어 mode 0600으로 저장한다.
    pub fn load_or_create(path: &Path) -> anyhow::Result<Self> {
        if let Ok(encoded) = std::fs::read_to_string(path) {
            let bytes = URL_SAFE_NO_PAD
                .decode(encoded.trim())
                .map_err(|_| anyhow::anyhow!("VAPID 키 형식이 올바르지 않습니다"))?;
            let signing = SigningKey::from_slice(&bytes)
                .map_err(|_| anyhow::anyhow!("VAPID 키를 읽을 수 없습니다"))?;
            return Ok(Self::from_signing(signing));
        }
        let signing = generate_key()?;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(path, URL_SAFE_NO_PAD.encode(signing.to_bytes()))?;
        restrict(path);
        Ok(Self::from_signing(signing))
    }

    fn from_signing(signing: SigningKey) -> Self {
        let public = signing
            .verifying_key()
            .to_encoded_point(false)
            .as_bytes()
            .to_vec();
        Self { signing, public }
    }

    /// 브라우저 `applicationServerKey`에 그대로 넣는 값.
    pub fn public_key_base64url(&self) -> String {
        URL_SAFE_NO_PAD.encode(&self.public)
    }

    /// `Authorization: vapid t=<jwt>, k=<pubkey>` 헤더 값.
    pub fn authorization(&self, audience: &str, now: i64) -> anyhow::Result<String> {
        let header = URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = serde_json::json!({
            "aud": audience,
            "exp": now + VAPID_TTL_SECS,
            "sub": "mailto:praxis@localhost",
        });
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims)?);
        let signing_input = format!("{header}.{payload}");
        // JWT ES256의 서명은 DER이 아니라 r||s 고정 64바이트다.
        let signature: Signature = self.signing.sign(signing_input.as_bytes());
        let encoded = URL_SAFE_NO_PAD.encode(signature.to_bytes());
        Ok(format!(
            "vapid t={signing_input}.{encoded}, k={}",
            self.public_key_base64url()
        ))
    }
}

fn generate_key() -> anyhow::Result<SigningKey> {
    // 유효 스칼라 범위를 벗어날 확률은 무시할 수준이지만, 실패를 조용히 넘기지 않는다.
    for _ in 0..8 {
        let mut bytes = [0_u8; 32];
        getrandom::getrandom(&mut bytes)
            .map_err(|error| anyhow::anyhow!("난수 생성 실패: {error}"))?;
        if let Ok(key) = SigningKey::from_slice(&bytes) {
            return Ok(key);
        }
    }
    Err(anyhow::anyhow!("VAPID 키를 생성하지 못했습니다"))
}

#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict(_path: &Path) {}

/// 설정 디렉터리 기준 기본 키 경로.
pub fn default_key_path() -> PathBuf {
    std::env::var("PRAXIS_RUNNER_VAPID")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            crate::usage::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config/praxis/vapid.key")
        })
}

pub async fn subscribe(
    pool: &SqlitePool,
    endpoint: &str,
    session_id: Option<i64>,
    now: i64,
) -> anyhow::Result<()> {
    // 같은 endpoint 재구독은 갱신으로 본다 — 실패 카운터를 초기화해 되살린다.
    sqlx::query(
        "INSERT INTO push_subscriptions (session_id, endpoint, created_at, last_ok_at, failure_count)
         VALUES (?, ?, ?, NULL, 0)
         ON CONFLICT(endpoint) DO UPDATE SET session_id = excluded.session_id, failure_count = 0",
    )
    .bind(session_id)
    .bind(endpoint)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn unsubscribe(pool: &SqlitePool, endpoint: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM push_subscriptions WHERE endpoint = ?")
        .bind(endpoint)
        .execute(pool)
        .await?;
    Ok(())
}

/// 세션이 회수되면 그 기기의 구독도 함께 사라져야 한다.
pub async fn remove_for_session(pool: &SqlitePool, session_id: i64) -> anyhow::Result<u64> {
    let result = sqlx::query("DELETE FROM push_subscriptions WHERE session_id = ?")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

pub async fn endpoints(pool: &SqlitePool) -> anyhow::Result<Vec<String>> {
    let rows = sqlx::query("SELECT endpoint FROM push_subscriptions")
        .fetch_all(pool)
        .await?;
    rows.into_iter()
        .map(|row| row.try_get::<String, _>("endpoint").map_err(Into::into))
        .collect()
}

/// push 서비스 origin — VAPID `aud` 클레임에 들어간다.
pub fn audience(endpoint: &str) -> Option<String> {
    let rest = endpoint.split_once("://")?;
    let host = rest.1.split('/').next()?;
    (!host.is_empty()).then(|| format!("{}://{host}", rest.0))
}

/// 등록된 모든 기기를 깨운다. 실패한 구독은 정리하고, 결과는 best-effort다 —
/// 알림 발송이 상태 전이를 막아서는 안 된다.
pub async fn notify_all(pool: &SqlitePool, http: &reqwest::Client, keys: &VapidKeys, now: i64) {
    let Ok(targets) = endpoints(pool).await else {
        return;
    };
    for endpoint in targets {
        let Some(audience) = audience(&endpoint) else {
            let _ = unsubscribe(pool, &endpoint).await;
            continue;
        };
        let Ok(authorization) = keys.authorization(&audience, now) else {
            continue;
        };
        let response = http
            .post(&endpoint)
            .header("Authorization", authorization)
            .header("TTL", PUSH_TTL_SECS.to_string())
            .header("Urgency", "normal")
            .header("Content-Length", "0")
            .send()
            .await;
        match response {
            // 404/410은 브라우저가 구독을 버렸다는 뜻 — 계속 두면 매번 실패한다.
            Ok(result) if result.status() == 404 || result.status() == 410 => {
                let _ = unsubscribe(pool, &endpoint).await;
            }
            Ok(result) if result.status().is_success() => {
                let _ = sqlx::query(
                    "UPDATE push_subscriptions SET last_ok_at = ?, failure_count = 0 WHERE endpoint = ?",
                )
                .bind(now)
                .bind(&endpoint)
                .execute(pool)
                .await;
            }
            _ => {
                let _ = sqlx::query(
                    "UPDATE push_subscriptions SET failure_count = failure_count + 1 WHERE endpoint = ?",
                )
                .bind(&endpoint)
                .execute(pool)
                .await;
                let _ = sqlx::query(
                    "DELETE FROM push_subscriptions WHERE endpoint = ? AND failure_count >= ?",
                )
                .bind(&endpoint)
                .bind(MAX_FAILURES)
                .execute(pool)
                .await;
            }
        }
    }
}

/// 알림을 보낼 만한 전이인지. 출력 이벤트마다 폰을 깨우면 알림이 무의미해진다.
pub fn should_notify(kind: &str) -> bool {
    matches!(
        kind,
        "completed"
            | "failed"
            | "cancelled"
            | crate::db::runner_event_kind::AWAITING_REVIEW
            | crate::db::runner_event_kind::AWAITING_ANSWER
    )
}

/// EventHub를 구독해 종료 전이마다 폰을 깨운다. 상태 전이 호출부를 건드리지 않는
/// 단일 통합 지점이다.
pub async fn watch(pool: SqlitePool, events: EventHub, keys: VapidKeys) {
    let http = reqwest::Client::new();
    let mut after = crate::db::latest_runner_event_sequence(&pool)
        .await
        .unwrap_or(0);
    loop {
        let Ok(subscription) = events.subscribe_after(after).await else {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            continue;
        };
        after = subscription.watermark.max(after);
        let mut receiver = subscription.receiver;
        loop {
            match receiver.recv().await {
                Ok(RunnerEvent { sequence, kind, .. }) => {
                    if sequence <= after {
                        continue;
                    }
                    after = sequence;
                    if should_notify(&kind) {
                        notify_all(&pool, &http, &keys, crate::runner::now_secs()).await;
                    }
                }
                // 밀린 경우 개별 이벤트를 되짚지 않는다 — 어차피 한 번 깨우면 충분하다.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    notify_all(&pool, &http, &keys, crate::runner::now_secs()).await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
