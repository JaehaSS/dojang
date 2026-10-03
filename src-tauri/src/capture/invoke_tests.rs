//! 린 인보케이션 테스트 — 설계 0055 §10의 T-3~T-4. T-1(인자 조립)·T-5(롤백 집합)·T-6(관측
//! 슬롯)은 그 대상이던 `invocation_args`·`run`·`record`·`mark_parsed`가 유일한 호출자(자동
//! 추출·수동 회고 파이프라인)와 함께 1.0에서 제거되어 함께 지웠다 — `profile`·`parse_envelope`
//! 만 남았다.
//!
//! 전부 프로세스를 띄우지 않는다. `reviewer::invocation`의 테스트(`reviewer/mod.rs:118`)와
//! 같은 이유다 — 인자 한 글자가 틀리면 CLI가 무출력으로 즉사하고, 그 실패는 `Ok(None)`에
//! 삼켜져 아무 데도 남지 않는다.

use super::invoke::*;

/// T-3 · 실패 봉투가 실패로 접히는지.
///
/// 픽스처는 실측값이다(CLI 2.1.258, `--model sonnet-typo-xyz`). **`subtype`이 `"success"`인데
/// `is_error`가 `true`다** — `subtype`으로 판정하면 이 실패를 놓치고 오류 문장이 회고가 된다.
#[test]
fn t3_is_error_envelope_with_success_subtype_is_a_failure() {
    let fixture = r#"{
      "type":"result","subtype":"success","is_error":true,
      "terminal_reason":"api_error","api_error_status":404,
      "result":"There's an issue with the selected model (sonnet-typo-xyz).",
      "total_cost_usd":0,
      "usage":{"input_tokens":0,"output_tokens":0}
    }"#;
    let env = parse_envelope(fixture).expect("봉투는 파싱된다");
    assert!(env.is_error, "is_error가 주 판정이다");
    assert_eq!(env.diagnostic.as_deref(), Some("api_error (404)"));
    // subtype이 success라는 사실 자체를 고정해 둔다 — 이것이 이 테스트의 존재 이유다.
    let v: serde_json::Value = serde_json::from_str(fixture).unwrap();
    assert_eq!(v["subtype"], "success");
}

/// T-3 · 정상 봉투에서 텍스트와 usage를 회수한다.
#[test]
fn t3_success_envelope_yields_text_and_usage() {
    let fixture = r#"{"type":"result","subtype":"success","is_error":false,
      "result":"교훈 한 문장.","total_cost_usd":0.0299,
      "usage":{"input_tokens":10,"output_tokens":537}}"#;
    let env = parse_envelope(fixture).unwrap();
    assert_eq!(env.text, "교훈 한 문장.");
    assert!(!env.is_error);
    assert_eq!(env.cost_usd, Some(0.0299));
    assert_eq!(env.input_tokens, Some(10));
    assert_eq!(env.output_tokens, Some(537));
}

/// T-3 · 봉투가 아닌 출력은 파싱 실패로 접힌다(계약 3).
#[test]
fn t3_non_envelope_output_is_rejected() {
    assert!(parse_envelope("그냥 텍스트 한 줄").is_none());
    assert!(parse_envelope("").is_none());
    // result 필드가 없는 JSON도 거부 — 부분 파싱으로 통과시키지 않는다.
    assert!(parse_envelope(r#"{"is_error":false}"#).is_none());
}

async fn settings_pool() -> (sqlx::SqlitePool, String) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = crate::testtmp::dir()
        .join(format!(
            "praxis-invoke-test-{}-{n}.sqlite",
            std::process::id()
        ))
        .to_string_lossy()
        .into_owned();
    let pool = crate::db::init_pool(&path).await.expect("init pool");
    (pool, path)
}

/// T-4 · 미설정·빈값·무효는 전부 코드 상수로 접힌다.
///
/// **CLI 기본값으로 떨어지게 두지 않는 것**이 이 작업의 요점이다 — 명시하지 않은 것은
/// 언젠가 사용자의 최상위 모델이 되고, 그 상속은 조용하다.
#[tokio::test]
async fn t4_unset_blank_and_invalid_all_fold_to_constants() {
    let (pool, path) = settings_pool().await;

    // 미설정
    let p = profile(&pool).await;
    assert_eq!(p.model, DEFAULT_MODEL);
    assert_eq!(p.effort, DEFAULT_EFFORT);
    assert!(p.lean, "미설정은 린이 기본이다");

    // 공백만 있는 값도 미설정으로 본다.
    crate::db::set_setting(&pool, KEY_MODEL, "   ").await.unwrap();
    crate::db::set_setting(&pool, KEY_EFFORT, "  ").await.unwrap();
    let p = profile(&pool).await;
    assert_eq!(p.model, DEFAULT_MODEL);
    assert_eq!(p.effort, DEFAULT_EFFORT);

    // 무효 effort는 검증기에 걸려 상수로 접힌다 — 그대로 넘기면 CLI가 즉사한다.
    crate::db::set_setting(&pool, KEY_EFFORT, "ultra").await.unwrap();
    assert_eq!(profile(&pool).await.effort, DEFAULT_EFFORT);

    // 유효값은 그대로 실린다.
    crate::db::set_setting(&pool, KEY_MODEL, "haiku").await.unwrap();
    crate::db::set_setting(&pool, KEY_EFFORT, "medium").await.unwrap();
    let p = profile(&pool).await;
    assert_eq!(p.model, "haiku");
    assert_eq!(p.effort, "medium");

    // lean은 명시적 "false"일 때만 꺼진다 — 오타가 롤백을 유발하면 안 된다.
    crate::db::set_setting(&pool, KEY_LEAN, "nope").await.unwrap();
    assert!(profile(&pool).await.lean, "\"false\"가 아니면 린 유지");
    crate::db::set_setting(&pool, KEY_LEAN, "false").await.unwrap();
    assert!(!profile(&pool).await.lean);

    pool.close().await;
    let _ = std::fs::remove_file(&path);
}

/// 기본값 상수가 카탈로그·검증기와 어긋나지 않는지.
///
/// `sonnet`은 `src/lib/models.ts`의 claude 별칭이고, `low`는 `reasoning_effort_override`가
/// 허용하는 최저값이다. 둘 중 하나가 어긋나면 첫 호출이 즉사한다.
#[test]
fn defaults_are_accepted_by_existing_validators() {
    assert_eq!(
        crate::agent::reasoning_effort_override("claude", Some(DEFAULT_EFFORT)),
        Ok(Some(DEFAULT_EFFORT.to_string()))
    );
    assert_eq!(DEFAULT_MODEL, "sonnet");
}
