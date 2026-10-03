//! 드라이버가 git 작업 트리에 대해 수행하는 단계의 핵심 로직. Tauri·DB와 무관하게 디렉터리만 받으므로 단위 시험이 가능하다.
//! 공통 규칙은 "에이전트의 작업을 먼저 커밋하고, 커밋된 diff로 경로를 판정하고, 검증이 남긴 산출물은 커밋 상태로 되돌려 버린다"이다.

use super::{checks, git_ops, model};
use crate::managed_process::SharedProcessRegistrar;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Eq)]
pub struct VerifyOutput {
    pub passed: bool,
    pub summary: String,
}

fn checks_summary(outcomes: &[checks::CheckOutcome], empty_msg: &str) -> String {
    if checks::checks_passed(outcomes) {
        String::new()
    } else if outcomes.is_empty() {
        empty_msg.to_string()
    } else {
        checks::failure_summary(outcomes)
    }
}

/// 검증이 남긴 추적되지 않는 산출물을 지워 worktree를 현재 HEAD 커밋과 같게 만든다(ignored는 보존).
fn restore_to_head(dir: &Path) -> Result<(), String> {
    let head = git_ops::head(dir).map_err(|e| e.to_string())?;
    git_ops::reset_to(dir, &head).map_err(|e| e.to_string())
}

/// 티켓 worktree 검증: 에이전트 작업을 커밋하고, fork point..HEAD 변경 경로가 허용 범위인지 본 뒤 검증 명령을 돌린다.
/// 검증이 만든 산출물은 마지막에 버리므로 다음 판정이나 통합 커밋에 섞이지 않는다.
pub fn verify_ticket(
    dir: &Path,
    fork: &str,
    allowed: &[String],
    acceptance: &[String],
    registrar: Option<&SharedProcessRegistrar>,
) -> Result<VerifyOutput, String> {
    git_ops::commit_all(dir, "티켓 작업 (파이프라인)").map_err(|e| e.to_string())?;
    let changed = git_ops::committed_paths(dir, fork).map_err(|e| e.to_string())?;
    let outside = model::paths_outside_allowed(&changed, allowed);
    if !outside.is_empty() {
        return Ok(VerifyOutput {
            passed: false,
            summary: format!("허용 경로 밖을 변경했습니다(되돌려라): {}", outside.join(", ")),
        });
    }
    if changed.is_empty() {
        return Ok(VerifyOutput { passed: false, summary: "변경이 하나도 없습니다. 티켓을 구현하라.".into() });
    }
    let outcomes = checks::run_ticket_checks(dir, acceptance, registrar);
    let restored = restore_to_head(dir);
    let out = VerifyOutput {
        passed: checks::checks_passed(&outcomes),
        summary: checks_summary(
            &outcomes,
            "실행할 검증 명령이 없습니다(저장소 build/test 설정과 수용 명령이 모두 비어 있음)",
        ),
    };
    restored?;
    Ok(out)
}

/// 최종 리뷰 자동 수정(R17). 에이전트 → 커밋 → 변경 경로가 허용 범위 안인지(통합된 티켓의 허용 경로 합집합) → 검증 순서이고,
/// 어느 단계든 실패하면 `pre`로 되돌린다. 성공하면 검증 산출물을 버린 커밋 상태가 남는다.
pub fn apply_final_fix(
    dir: &Path,
    pre: &str,
    allowed: &[String],
    acceptance: &[String],
    registrar: Option<&SharedProcessRegistrar>,
    agent: impl FnOnce() -> Result<String, String>,
) -> Result<(), String> {
    let attempt = (|| -> Result<(), String> {
        agent()?;
        git_ops::commit_all(dir, "최종 리뷰 지적 자동 수정").map_err(|e| e.to_string())?;
        let changed = git_ops::committed_paths(dir, pre).map_err(|e| e.to_string())?;
        if changed.is_empty() {
            return Err("자동 수정이 아무 변경도 만들지 못했습니다".into());
        }
        let outside = model::paths_outside_allowed(&changed, allowed);
        if !outside.is_empty() {
            return Err(format!("자동 수정이 허용 경로 밖을 변경했습니다: {}", outside.join(", ")));
        }
        let outcomes = checks::run_ticket_checks(dir, acceptance, registrar);
        if !checks::checks_passed(&outcomes) {
            return Err(format!("자동 수정 뒤 검증 실패:\n{}", checks_summary(&outcomes, "실행할 검증 명령이 없습니다")));
        }
        restore_to_head(dir)
    })();
    if attempt.is_err() {
        let _ = git_ops::reset_to(dir, pre);
    }
    attempt
}

/// 읽기 전용 단계(분할 심판)가 worktree를 바꿨는가. 단계 전후 `status --porcelain`을 비교한다.
/// worktree 부트스트랩(`.worktreeinclude` 복사·셋업 스크립트)이 시작 시점부터 남긴 미추적 파일은 전후가 같으므로 변경으로 보지 않는다.
pub fn tree_changed_during(before: &str, after: &str) -> bool {
    before != after
}

/// 통합 단계 재개 때 무엇을 할지.
#[derive(Debug, PartialEq, Eq)]
pub enum IntegrationRecovery {
    /// 통합 자체는 이미 성공했다. 되돌리지 말고 티켓 상태 전이만 마무리한다.
    FinishTransition,
    /// 중단된 통합이다. 기록해 둔 통합 전 head로 되돌리고 처음부터 다시 한다.
    ResetTo(String),
    /// 처음 시작이거나 되돌릴 기록이 없다. 현재 head에서 시작한다.
    FromHead,
}

pub fn integration_recovery(recovering: bool, step_status: Option<&str>, recorded_pre: Option<String>) -> IntegrationRecovery {
    match (recovering, step_status, recorded_pre) {
        (true, Some("succeeded"), _) => IntegrationRecovery::FinishTransition,
        (true, _, Some(pre)) => IntegrationRecovery::ResetTo(pre),
        _ => IntegrationRecovery::FromHead,
    }
}

/// 단계 prompt 첫 줄에 `pre:<rev>`로 기록한 되돌림 지점. 이어지는 줄은 실제 프롬프트다.
pub fn with_pre(pre: &str, prompt: &str) -> String {
    if prompt.is_empty() { format!("pre:{pre}") } else { format!("pre:{pre}\n{prompt}") }
}

pub fn recorded_pre(prompt: &str) -> Option<String> {
    let first = prompt.lines().next()?;
    first.strip_prefix("pre:").map(str::trim).filter(|p| !p.is_empty()).map(String::from)
}

#[cfg(test)]
#[path = "steps_tests.rs"]
mod tests;
