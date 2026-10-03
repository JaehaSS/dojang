use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use sqlx::SqlitePool;

use crate::runner::session::{self, MobileSession};

/// Runner pairing secret. 파일 원문은 절대 로그·DB·응답에 노출하지 않는다.
#[derive(Clone)]
pub struct RunnerAuth {
    token: Arc<[u8]>,
}

impl RunnerAuth {
    pub fn from_file(path: &Path) -> Result<Self, String> {
        ensure_secret_file_mode(path)?;
        let encoded = std::fs::read_to_string(path)
            .map_err(|_| "pairing token 파일을 읽을 수 없습니다".to_string())?;
        let token = decode_hex_32(encoded.trim())?;
        Ok(Self {
            token: token.into(),
        })
    }

    pub fn matches_bearer(&self, value: Option<&header::HeaderValue>) -> bool {
        let Some(value) = value.and_then(|value| value.to_str().ok()) else {
            return false;
        };
        let Some(candidate) = value.strip_prefix("Bearer ") else {
            return false;
        };
        decode_hex_32(candidate)
            .is_ok_and(|candidate| constant_time_eq(self.token.as_ref(), &candidate))
    }

    pub fn matches_request(&self, headers: &header::HeaderMap) -> bool {
        self.matches_bearer(headers.get(header::AUTHORIZATION))
            || headers
                .get(header::SEC_WEBSOCKET_PROTOCOL)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|protocols| {
                    protocols.split(',').map(str::trim).any(|candidate| {
                        decode_hex_32(candidate)
                            .is_ok_and(|token| constant_time_eq(self.token.as_ref(), &token))
                    })
                })
    }
}

/// 인증 미들웨어가 필요로 하는 상태. 모바일 세션 검증에 DB가 필요해 pool을 함께 든다.
#[derive(Clone)]
pub struct AuthState {
    pub auth: RunnerAuth,
    pub pool: SqlitePool,
}

/// 요청을 통과시킨 자격. 핸들러가 필요하면 extension으로 꺼내 쓴다.
#[derive(Clone, Debug)]
pub enum AuthContext {
    /// pairing token 보유자(Desktop). 전 권한.
    Pairing,
    /// QR로 페어링한 모바일 기기. scope 제한을 받는다.
    Mobile(MobileSession),
}

/// 모든 HTTP/WS upgrade 요청은 loopback peer와 자격(pairing token 또는 모바일 세션 쿠키)을
/// 모두 만족해야 한다.
///
/// `tailscale serve` 경유 요청은 프록시가 loopback에서 넣으므로 peer 검사를 통과한다.
/// 즉 이 경계는 tailnet 경계로 대체되며, 실질 방어선은 tailnet ACL + 자격 + scope 3겹이다.
pub async fn require_auth(
    State(state): State<AuthState>,
    mut request: Request,
    next: Next,
) -> Response {
    let peer_is_loopback = request
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .is_some_and(|peer| peer.0.ip().is_loopback());
    if !peer_is_loopback {
        return StatusCode::FORBIDDEN.into_response();
    }
    if state.auth.matches_request(request.headers()) {
        request.extensions_mut().insert(AuthContext::Pairing);
        return next.run(request).await;
    }

    let Some(token) = session_token(request.headers()) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let now = crate::runner::now_secs();
    let session = match session::authenticate(&state.pool, &token, now).await {
        Ok(Some(session)) => session,
        Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    // 쿠키는 자동 첨부되므로 CSRF 표면이 생긴다. SameSite=Strict와 함께 Origin을
    // 두 번째 겹으로 둔다 — 상태 변경 요청은 자기 오리진에서 온 것만 받는다.
    if is_mutating(request.method()) && !origin_is_self(request.headers()) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if session.scope == session::SCOPE_MOBILE
        && mobile_scope_denies(request.method(), request.uri().path())
    {
        return (
            StatusCode::FORBIDDEN,
            "모바일 세션은 파일을 수정할 수 없습니다",
        )
            .into_response();
    }

    request
        .extensions_mut()
        .insert(AuthContext::Mobile(session));
    next.run(request).await
}

fn is_mutating(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::DELETE | Method::PATCH
    )
}

/// 모바일 세션이 접근할 수 없는 경로. **deny-list**이므로 쓰기 라우트를 새로 추가하면
/// 여기에도 넣어야 한다. (설계 0013 §6.4 — 편집은 모바일 범위 밖)
pub fn mobile_scope_denies(method: &Method, path: &str) -> bool {
    // 파일 편집은 모바일 범위 밖이다.
    (method == Method::PUT && path == "/v1/files/write")
        // 기기 관리는 Desktop 전용. 폰이 새 페어링 코드를 찍어낼 수 있으면 회수가 무의미해지고,
        // 탈취된 세션 하나가 스스로 기기를 늘리는 권한 상승이 된다.
        || path.starts_with("/v1/mobile/")
        // 세션홈 목록은 페어링 자격 전용이다(설계 2026-09-17 제약 5) — 승계(POST /v1/tasks의
        // resume_session)는 본문을 봐야 하므로 여기서 못 막는다. `task_create` 핸들러가 대신 막는다.
        || (method == Method::GET && (path == "/v1/sessions" || path.starts_with("/v1/sessions/")))
}

/// 요청이 자기 출처에서 왔는지 본다. 두 신호 중 하나면 통과한다.
///
/// 1. `Origin`의 authority == `Host` (스킴은 프록시 종단에 따라 달라져 비교하지 않는다)
/// 2. `Sec-Fetch-Site: same-origin`
///
/// 2번을 함께 받는 이유: 브라우저가 동일 출처 POST에 `Origin`을 생략하는 경우가 있고,
/// 그러면 승인이 조용히 403이 된다 — 모바일에서 가장 치명적인 실패다. 둘 다 JS로 설정할 수
/// 없는 forbidden header라 위조 경로가 되지는 않는다.
fn origin_is_self(headers: &header::HeaderMap) -> bool {
    if let Some(site) = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        if site.eq_ignore_ascii_case("same-origin") {
            return true;
        }
        // 명시적으로 cross-site라고 알려온 요청은 Origin을 볼 것도 없이 거절한다.
        if site.eq_ignore_ascii_case("cross-site") || site.eq_ignore_ascii_case("same-site") {
            return false;
        }
    }
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    origin_authority(origin).is_some_and(|authority| authority == host)
}

/// `https://host:port/...` → `host:port`
pub fn origin_authority(origin: &str) -> Option<&str> {
    let rest = origin.split_once("://").map(|(_, rest)| rest)?;
    if rest.is_empty() {
        return None;
    }
    Some(rest.split('/').next().unwrap_or(rest))
}

/// 세션 토큰은 쿠키로만 받는다. HttpOnly라 XSS로도 원문을 꺼낼 수 없다.
fn session_token(headers: &header::HeaderMap) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie_value(cookies, session::COOKIE_NAME).map(str::to_string)
}

pub fn cookie_value<'a>(cookies: &'a str, name: &str) -> Option<&'a str> {
    cookies.split(';').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key.trim() == name).then(|| value.trim())
    })
}

/// API가 받는 repository path는 configured root의 실경로 하위여야 한다.
pub fn authorize_repository_path(roots: &[PathBuf], requested: &Path) -> Result<PathBuf, String> {
    let canonical = requested
        .canonicalize()
        .map_err(|_| "허용되지 않는 repository 경로입니다".to_string())?;
    if roots.iter().any(|root| canonical.starts_with(root)) {
        return Ok(canonical);
    }
    Err("허용되지 않는 repository 경로입니다".to_string())
}

fn ensure_secret_file_mode(path: &Path) -> Result<(), String> {
    let metadata =
        std::fs::metadata(path).map_err(|_| "pairing token 파일을 읽을 수 없습니다".to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if metadata.permissions().mode() & 0o777 != 0o600 {
            return Err("pairing token 파일 권한은 0600이어야 합니다".to_string());
        }
    }
    Ok(())
}

pub(crate) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(left_byte ^ right_byte);
    }
    difference == 0
}

fn decode_hex_32(value: &str) -> Result<[u8; 32], String> {
    if value.len() != 64 || !value.is_ascii() {
        return Err("pairing token은 32-byte hex여야 합니다".to_string());
    }
    let mut decoded = [0_u8; 32];
    for (index, slot) in decoded.iter_mut().enumerate() {
        let offset = index * 2;
        let high = hex_nibble(value.as_bytes()[offset])?;
        let low = hex_nibble(value.as_bytes()[offset + 1])?;
        *slot = (high << 4) | low;
    }
    Ok(decoded)
}

fn hex_nibble(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err("pairing token은 32-byte hex여야 합니다".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn comparison_requires_equal_length_and_content() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }

    #[test]
    fn token_decoder_requires_32_byte_hex() {
        assert!(super::decode_hex_32(&"ab".repeat(32)).is_ok());
        assert!(super::decode_hex_32("ab").is_err());
    }
}
