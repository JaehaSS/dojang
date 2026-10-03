//! 캡처·회고의 **린 인보케이션** — 설계 0055.
//!
//! 이 모듈이 존재하는 이유는 비용이 아니라 **명시**다. 플래그 없는 `claude -p`는
//! `~/.claude/settings.json`의 모델·effort를 조용히 상속하고, 도구 31종·MCP·스킬 열거를
//! 매 호출의 시스템 프롬프트에 싣는다. 한 문장 요약에 호출당 약 28,600토큰의 하네스
//! 오버헤드가 붙었고 그것이 비용의 79%였다(브리프 §2.2).
//!
//! 두 가지를 함께 고친다.
//! - **명시적 실행 프로파일** — 미설정·무효는 CLI 기본이 아니라 코드 상수로 접는다(AD-4).
//!   명시하지 않은 것은 언젠가 사용자의 최상위 모델이 된다.
//! - **신뢰 경계** — 트랜스크립트는 툴 결과·레포 파일을 담은 신뢰할 수 없는 입력이다.
//!   `--tools ""`가 도구 표면을 0으로 만들어, 프롬프트 본문의 가드 문구를 유일한 방어에서
//!   2차 방어로 강등한다(AD-3).
//!
//! CLI를 실제로 호출하던 경로(`run`·`invocation_args`)는 그 유일한 호출자였던 자동 추출·
//! 수동 회고 파이프라인과 함께 1.0에서 제거됐다. 관측 슬롯(`last_runs`, AD-7)도 그 경로가
//! 없어지며 채울 주체를 잃어 함께 뗐다. 남은 것은 그 계약의 나머지 절반 —
//! 실행 프로파일 해석(`profile`)과 봉투 파싱(`parse_envelope`)이다. 둘 다 설정 화면·
//! provenance 추적이 여전히 쓴다.

use sqlx::SqlitePool;

/// 설정이 비어 있을 때 떨어질 자리. **CLI 기본값으로 두지 않는다** — 그것이 이 버그였다.
pub(crate) const DEFAULT_MODEL: &str = "sonnet";
pub(crate) const DEFAULT_EFFORT: &str = "low";

pub(crate) const KEY_MODEL: &str = "capture:model";
pub(crate) const KEY_EFFORT: &str = "capture:effort";
pub(crate) const KEY_LEAN: &str = "capture:lean";

/// 이번 호출에 적용할 실행 프로파일.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CaptureProfile {
    pub model: String,
    pub effort: String,
    /// false면 §12 롤백 집합대로 되돌린다. 시스템 프롬프트·출력 형식·설정 소스만 되돌리고
    /// 모델·effort·도구 차단은 유지한다.
    pub lean: bool,
}

impl Default for CaptureProfile {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.to_string(),
            effort: DEFAULT_EFFORT.to_string(),
            lean: true,
        }
    }
}

/// 설정에서 프로파일 해석. 미설정·빈값·무효는 전부 코드 상수로 접는다(AD-4).
///
/// effort는 `reasoning_effort_override`로 검증한다 — 무효값을 그대로 넘기면 CLI가 즉사하고,
/// 그 실패는 `Ok(None)`으로 삼켜져 조용한 0건이 된다.
pub(crate) async fn profile(pool: &SqlitePool) -> CaptureProfile {
    let get = |key: &'static str| async move {
        crate::db::get_setting(pool, key)
            .await
            .ok()
            .flatten()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let model = get(KEY_MODEL).await.unwrap_or_else(|| DEFAULT_MODEL.to_string());
    let effort = get(KEY_EFFORT)
        .await
        .and_then(|e| {
            crate::agent::reasoning_effort_override("claude", Some(&e))
                .ok()
                .flatten()
        })
        .unwrap_or_else(|| DEFAULT_EFFORT.to_string());
    // 미설정은 린(기본 on). 명시적으로 "false"일 때만 롤백한다.
    let lean = get(KEY_LEAN).await.as_deref() != Some("false");
    CaptureProfile {
        model,
        effort,
        lean,
    }
}

/// Consent and queued work bind to every profile field that affects an invocation.
pub(crate) fn provider_identity(profile: &CaptureProfile) -> String {
    format!(
        "claude:{}:{}:{}",
        profile.model,
        profile.effort,
        if profile.lean { "lean" } else { "standard" }
    )
}

pub(crate) fn provider_display(profile: &CaptureProfile) -> String {
    format!(
        "Claude · {} · {}{}",
        profile.model,
        profile.effort,
        if profile.lean { " · lean" } else { "" }
    )
}

/// 봉투에서 뽑아낸 것. 호출자에게는 `text`만 나간다 — 봉투 원문은 이 모듈 밖으로 안 나간다.
#[derive(Debug, PartialEq)]
pub(crate) struct Envelope {
    pub text: String,
    pub is_error: bool,
    pub cost_usd: Option<f64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub diagnostic: Option<String>,
}

/// `--output-format json` 봉투 파싱.
///
/// **`subtype`으로 실패를 판정하지 않는다.** 실측(CLI 2.1.258)에서 잘못된 모델명은
/// `is_error: true`이면서 `subtype: "success"`인 봉투를 냈다. `subtype`만 보면 이 실패를
/// 놓치고, 사람이 읽는 오류 문장이 그대로 회고로 저장된다.
pub(crate) fn parse_envelope(stdout: &str) -> Option<Envelope> {
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    let text = v.get("result")?.as_str()?.to_string();
    let usage = v.get("usage");
    let tok = |k: &str| usage.and_then(|u| u.get(k)).and_then(|x| x.as_u64());
    let diagnostic = v
        .get("terminal_reason")
        .and_then(|x| x.as_str())
        .map(|s| {
            match v.get("api_error_status").and_then(|x| x.as_u64()) {
                Some(code) => format!("{s} ({code})"),
                None => s.to_string(),
            }
        });
    Some(Envelope {
        text,
        is_error: v.get("is_error").and_then(|x| x.as_bool()).unwrap_or(false),
        cost_usd: v.get("total_cost_usd").and_then(|x| x.as_f64()),
        input_tokens: tok("input_tokens"),
        output_tokens: tok("output_tokens"),
        diagnostic,
    })
}

