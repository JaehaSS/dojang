//! 메모리의 정본은 file.rs가 다루는 창고 안 마크다운 파일이다(설계 2026-09-13, P1).
//!
//! 이 파일에 남은 것은 두 가지뿐이다.
//! - 파일형 메모리가 렌더링에 쓰는 관리 블록 마커(`MARK_START`/`MARK_END`).
//! - 벡터 유사도 유틸(`cosine`/`encode_f32`/`decode_f32`) — 지식 그래프(`knowledge::graph`/`search`)가
//!   자기 임베딩 열에 재사용한다. 옛 계층형 메모리 스토어와 무관하게 남는다.
//!
//! 옛 DB 메모리(P2)가 쓰던 `memory_usages`/`memory_injections`를 참조하던 마지막 소비자
//! (앙상블 피드백·인사이트 분석·결정 승인 provenance)를 모두 뗐으므로, `migrate`는 이제
//! 파일형 메모리의 상태 색인인 `memory_files`만 만든다. 옛 사용자 DB에 남은 표는 지우지
//! 않는다(ADR: 데이터 삭제는 사용자 명령으로만) — `memory::file::purge_legacy_once`가
//! 존재 확인 후 비우는 코드만 그대로 남는다.

use sqlx::SqlitePool;

pub(super) const MARK_START: &str = "<!-- PRAXIS MEMORY START -->";
pub(super) const MARK_END: &str = "<!-- PRAXIS MEMORY END -->";

/// 파일형 메모리(P1) — 설계 2026-09-13. 옛 추출 파이프라인을 대신한다.
pub mod file;

/// 두 f32 벡터의 코사인 유사도. 둘 중 하나가 비었거나 크기가 다르면 0.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na * nb)
}

pub(crate) fn encode_f32(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub(crate) fn decode_f32(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// 파일형 메모리(P1)의 상태 색인만 만든다 — 옛 DB 메모리(P2) 테이블은 더는 만들지 않는다.
pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE IF NOT EXISTS memory_files (\
           path TEXT PRIMARY KEY, \
           kind TEXT NOT NULL, \
           repo TEXT, \
           repo_key TEXT, \
           lines INTEGER NOT NULL DEFAULT 0, \
           bytes INTEGER NOT NULL DEFAULT 0, \
           modified_at INTEGER, \
           last_projected_at INTEGER, \
           last_task_id INTEGER)",
    )
    .execute(pool)
    .await?;
    Ok(())
}
