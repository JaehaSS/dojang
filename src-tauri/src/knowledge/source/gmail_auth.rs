//! Gmail OAuth — desktop **loopback + PKCE** (설계 0020 DR-10 / 플랜 0028 Task 3).
//!
//! OOB(`urn:ietf:wg:oauth:2.0:oob`)는 폐지됐다. 루프백만 쓴다.
//!
//! client는 **사용자가 소유한다.** Praxis가 번들하면 전 사용자가 한 Cloud 프로젝트를
//! 공유하게 되고, `gmail.readonly`가 restricted scope인 탓에 CASA 보안 평가(매년 갱신)가
//! 걸린다. 사용자마다 자기 프로젝트면 "개인 사용" 검증 면제가 영구히 유지된다.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

/// 단일 scope. 넓히지 않는다 — 읽기 외의 권한은 이 기능에 필요 없고,
/// restricted scope를 더 얹으면 검증 부담만 커진다.
const SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";

/// 인증 실패는 두 종류다. **섞으면 사용자가 무엇을 해야 할지 모른다** —
/// 네트워크 문제면 기다리면 되고, 승인 철회면 다시 인증해야 한다.
#[derive(Debug, PartialEq, Eq)]
pub enum AuthError {
    /// refresh token이 죽었다(승인 철회·비밀번호 변경·`Testing` 상태의 7일 만료).
    /// 재인증 외에는 방법이 없다.
    NeedsReauth(String),
    /// 일시적 실패. 재시도로 풀린다.
    Transient(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NeedsReauth(m) => write!(f, "Gmail 재인증이 필요합니다: {m}"),
            Self::Transient(m) => write!(f, "Gmail 인증 일시 실패: {m}"),
        }
    }
}

impl std::error::Error for AuthError {}

/// PKCE 쌍을 만든다.
///
/// verifier는 43~128자 unreserved 문자여야 한다(RFC 7636 §4.1). 32바이트를
/// base64url(패딩 없음)로 인코딩하면 정확히 43자가 되어 하한을 만족한다.
pub fn pkce_pair() -> anyhow::Result<(String, String)> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| anyhow::anyhow!("PKCE 난수 생성 실패: {error}"))?;
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let challenge = challenge_for(&verifier);
    Ok((verifier, challenge))
}

/// `S256` 방식. **패딩(`=`)과 `+`/`/`가 있으면 안 된다** — 표준 base64로 인코딩하면
/// Google이 challenge 불일치로 거부하는데, 그 실패는 "코드 교환 실패"로만 보여
/// 원인을 찾기 어렵다.
pub fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// CSRF 방지용 `state`. 콜백에서 이 값이 다르면 응답을 버린다.
pub fn random_state() -> anyhow::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| anyhow::anyhow!("state 난수 생성 실패: {error}"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// 사용자를 보낼 동의 화면 URL.
///
/// `access_type=offline`과 `prompt=consent`가 **둘 다** 필요하다. 빠뜨리면 두 번째
/// 인증부터 refresh token이 오지 않아, 재인증을 해도 아무것도 나아지지 않는 상태가 된다.
pub fn authorize_url(
    client_id: &str,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> anyhow::Result<String> {
    let url = reqwest::Url::parse_with_params(
        AUTH_ENDPOINT,
        &[
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("response_type", "code"),
            ("scope", SCOPE),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("state", state),
        ],
    )?;
    Ok(url.to_string())
}

/// 콜백을 받을 루프백 리스너와 그 주소를 연다.
///
/// **`127.0.0.1`에만 바인딩한다.** `0.0.0.0`에 열면 같은 네트워크의 다른 기기가
/// 인증 코드를 가로챌 수 있다. 포트는 0으로 요청해 OS가 비어 있는 것을 고르게 한다 —
/// 고정 포트를 쓰면 다른 프로세스가 점유했을 때 인증 자체가 불가능해진다.
pub async fn bind_loopback() -> anyhow::Result<(tokio::net::TcpListener, String)> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let port = listener.local_addr()?.port();
    Ok((listener, format!("http://127.0.0.1:{port}")))
}

/// 브라우저가 돌려보내는 `code`를 기다린다.
///
/// `state`가 일치하지 않으면 버린다 — 이것이 없으면 공격자가 자기 인증 코드를
/// 사용자 브라우저로 흘려보내 **남의 메일함을 사용자 계정에 붙일 수** 있다.
pub async fn wait_for_code(
    listener: tokio::net::TcpListener,
    expected_state: &str,
    timeout: std::time::Duration,
) -> anyhow::Result<String> {
    use std::sync::{Arc, Mutex};

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    let shared = Arc::new(CallbackState {
        tx: Mutex::new(Some(tx)),
        expected_state: expected_state.to_string(),
    });

    let app = axum::Router::new()
        .route("/", axum::routing::get(handle_callback))
        .with_state(shared);

    let server = axum::serve(listener, app);
    tokio::select! {
        result = server => anyhow::bail!("콜백 서버가 코드를 받기 전에 종료됐다: {result:?}"),
        _ = tokio::time::sleep(timeout) => {
            anyhow::bail!("인증 시간이 초과됐습니다. 브라우저에서 승인을 마치지 못했다면 다시 시도하세요.")
        }
        received = rx => {
            // 브라우저가 완료 화면을 받을 틈을 준다. 곧바로 리스너를 닫으면
            // 사용자는 승인에 성공하고도 "연결이 끊겼습니다"를 본다.
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            match received {
                Ok(Ok(code)) => Ok(code),
                Ok(Err(message)) => anyhow::bail!("{message}"),
                Err(_) => anyhow::bail!("콜백 채널이 닫혔다"),
            }
        }
    }
}

struct CallbackState {
    tx: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<Result<String, String>>>>,
    expected_state: String,
}

async fn handle_callback(
    axum::extract::State(shared): axum::extract::State<std::sync::Arc<CallbackState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Html<&'static str> {
    let outcome = if let Some(error) = params.get("error") {
        // 사용자가 동의 화면에서 거부한 경우가 대부분이다.
        Err(format!("Google이 인증을 거부했습니다: {error}"))
    } else if params.get("state").map(String::as_str) != Some(shared.expected_state.as_str()) {
        Err("state가 일치하지 않습니다 — 요청을 버렸습니다.".to_string())
    } else {
        match params.get("code") {
            Some(code) => Ok(code.clone()),
            None => Err("콜백에 code가 없습니다.".to_string()),
        }
    };

    let succeeded = outcome.is_ok();
    // 한 번만 보낸다. 브라우저가 프리페치 등으로 두 번 때려도 두 번째는 조용히 무시된다.
    if let Ok(mut slot) = shared.tx.lock() {
        if let Some(tx) = slot.take() {
            let _ = tx.send(outcome);
        }
    }

    if succeeded {
        axum::response::Html(
            "<h2>연결되었습니다</h2><p>이 창을 닫고 Dojang으로 돌아가세요.</p>",
        )
    } else {
        axum::response::Html("<h2>연결에 실패했습니다</h2><p>Dojang에서 다시 시도하세요.</p>")
    }
}

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    /// 갱신 응답에는 없다. 최초 인증에서 받은 값을 계속 쓴다.
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: i64,
}

/// 토큰 엔드포인트의 에러 응답을 두 갈래로 가른다.
///
/// 순수 함수로 떼어 둔 이유: 이 판별이 틀리면 "재인증하세요"를 네트워크 장애에
/// 띄우거나, 반대로 죽은 토큰을 영원히 재시도하게 된다. 둘 다 픽스처로 고정해야 한다.
pub fn classify_token_error(status: u16, body: &str) -> AuthError {
    // Google은 만료·철회된 refresh token에 `invalid_grant`를 준다.
    if body.contains("invalid_grant") || body.contains("invalid_client") {
        return AuthError::NeedsReauth(body.to_string());
    }
    // 5xx와 429는 명백히 일시적이다.
    if status >= 500 || status == 429 {
        return AuthError::Transient(format!("HTTP {status}: {body}"));
    }
    // 그 밖의 4xx는 요청 자체가 틀린 것이라 재시도해도 같다 — 재인증 쪽으로 보낸다.
    if status >= 400 {
        return AuthError::NeedsReauth(format!("HTTP {status}: {body}"));
    }
    AuthError::Transient(format!("HTTP {status}: {body}"))
}

/// 인증 코드를 토큰으로 바꾼다.
pub async fn exchange_code(
    http: &reqwest::Client,
    client_id: &str,
    client_secret: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<TokenResponse, AuthError> {
    post_token(
        http,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
        ],
    )
    .await
}

/// refresh token으로 새 access token을 받는다.
pub async fn refresh_access_token(
    http: &reqwest::Client,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<TokenResponse, AuthError> {
    post_token(
        http,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
            ("client_secret", client_secret),
        ],
    )
    .await
}

async fn post_token(
    http: &reqwest::Client,
    form: &[(&str, &str)],
) -> Result<TokenResponse, AuthError> {
    let response = http
        .post(TOKEN_ENDPOINT)
        .form(form)
        .send()
        .await
        .map_err(|error| AuthError::Transient(format!("토큰 요청 실패: {error}")))?;

    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|error| AuthError::Transient(format!("토큰 응답 읽기 실패: {error}")))?;

    if status != 200 {
        return Err(classify_token_error(status, &body));
    }
    serde_json::from_str(&body)
        .map_err(|error| AuthError::Transient(format!("토큰 응답 파싱 실패: {error}: {body}")))
}

// ── OAuth client 해석 — 번들 vs 사용자 입력 (ADR 0147) ──

/// 빌드에 미리 넣어 두는 OAuth client. **빌드 타임에만 주입된다** — 저장소에는 값이 없다.
///
/// Google OAuth는 client 없이 성립하지 않으므로 **누군가는 Cloud 프로젝트를 한 번
/// 만들어야 한다.** 우회로는 없다. 다만 그 1회를 빌드하는 쪽이 대신 치르고 결과를 여기
/// 박으면, 쓰는 쪽은 발급 절차를 영영 만나지 않는다 — 자기 client를 심어 자기 기기들에
/// 배포하는 경우가 여기 해당한다.
///
/// 조직 배포라면 consent screen을 **Internal user type**으로 만드는 선택지가 더 있다.
/// Internal은 restricted scope를 써도 Google 심사가 없고 refresh token 7일 만료와
/// 100-user cap도 적용되지 않는다. 개인 계정에는 Internal이 없으므로 External을 쓰고,
/// 그때는 `In production`으로 올려야 7일 만료를 피한다(검증 제출은 하지 않는다).
///
/// 주입하지 않고 빌드하면 `None`이 되어 **기존 BYO 경로만 남는다.** 기능이 사라지는 게
/// 아니라 예전과 똑같아진다.
const BUNDLED_CLIENT_ID: Option<&str> = option_env!("PRAXIS_GMAIL_CLIENT_ID");
const BUNDLED_CLIENT_SECRET: Option<&str> = option_env!("PRAXIS_GMAIL_CLIENT_SECRET");

/// credential이 어디서 왔는가. 화면이 입력란을 접을지 펼칠지를 이걸로 정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialOrigin {
    /// 빌드에 박힌 client. 사용자는 아무것도 입력하지 않았다.
    Bundled,
    /// 사용자가 자기 Cloud 프로젝트에서 발급해 넣은 것.
    User,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub client_id: String,
    pub client_secret: String,
    pub origin: CredentialOrigin,
}

/// 해석 실패는 **무엇이 없는지**까지 말해야 한다 — 사용자를 보낼 곳이 갈린다.
/// client_id가 없으면 발급 절차로, secret만 없으면 입력란으로 보내야 한다.
#[derive(Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// 사용자 입력도 없고 번들도 없다.
    NoClientId,
    /// client_id는 있는데 짝이 되는 secret이 없다.
    NoClientSecret,
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoClientId => write!(
                f,
                "client_id를 먼저 입력하세요. Google Cloud에서 Desktop app 클라이언트를 만들어야 합니다."
            ),
            Self::NoClientSecret => write!(f, "client_secret을 먼저 입력하세요."),
        }
    }
}

impl std::error::Error for ResolveError {}

/// 빌드에 박힌 client.
///
/// **둘 중 하나라도 비면 없는 것으로 본다.** 반쪽 주입은 인증을 반드시 실패시키는데,
/// 그 실패가 `invalid_client`로만 나타나 원인이 빌드 설정이라는 데 도달하기 어렵다.
pub fn bundled() -> Option<Credentials> {
    let client_id = BUNDLED_CLIENT_ID?.trim();
    let client_secret = BUNDLED_CLIENT_SECRET?.trim();
    if client_id.is_empty() || client_secret.is_empty() {
        return None;
    }
    Some(Credentials {
        client_id: client_id.to_string(),
        client_secret: client_secret.to_string(),
        origin: CredentialOrigin::Bundled,
    })
}

/// 사용자 값이 있으면 그것, 없으면 번들.
///
/// **번들이 BYO를 밀어내면 안 된다.** 번들된 client가 받아주지 않는 계정이 있기 때문이다 —
/// 조직 Internal client는 조직 밖 계정을 아예 차단하고, External도 소유자가 정한 테스트
/// 사용자 범위에 걸릴 수 있다. 그때 기댈 곳은 사용자가 직접 넣는 값뿐이다.
pub fn resolve(
    user_client_id: &str,
    user_secret: Option<&str>,
) -> Result<Credentials, ResolveError> {
    resolve_with(bundled(), user_client_id, user_secret)
}

/// 번들을 인자로 받는 순수 형태.
///
/// 번들 값은 컴파일 타임 상수라 테스트가 만들어 낼 수 없다. 판정 규칙만 여기로 떼어
/// 고정한다 — 규칙이 틀리면 증상이 전부 "왜인지 모르게 인증이 안 됨"이라 값이 크다.
pub fn resolve_with(
    bundled: Option<Credentials>,
    user_client_id: &str,
    user_secret: Option<&str>,
) -> Result<Credentials, ResolveError> {
    let client_id = user_client_id.trim();
    if !client_id.is_empty() {
        // **두 출처를 섞지 않는다.** 사용자 client_id에 번들 secret을 붙이면 Google이
        // `invalid_client`를 주는데, 화면에는 "재인증이 필요합니다"로만 보여
        // 자기 secret을 안 넣었다는 사실에 영영 도달하지 못한다.
        let client_secret = user_secret
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or(ResolveError::NoClientSecret)?;
        return Ok(Credentials {
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
            origin: CredentialOrigin::User,
        });
    }
    bundled.ok_or(ResolveError::NoClientId)
}
