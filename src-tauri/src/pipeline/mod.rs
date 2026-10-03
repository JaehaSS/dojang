//! 멀티 벤더 파이프라인 — 목표를 스펙+티켓으로 분할하고 벤더별로 실행·검증·통합하는 오케스트레이션.
//!
//! 이 모듈의 `model`·`state`·`split`은 Tauri와 무관한 순수 로직이며, `db`만 SQLite를 만진다.
//! 설계 정본: `docs/plans/2026-10-01-multi-vendor-pipeline.md`.

pub mod checks;
pub mod db;
pub mod git_ops;
pub mod model;
pub mod plan;
pub mod prompts;
pub mod registry;
pub mod review_run;
pub mod split;
pub mod state;
pub mod steps;

#[cfg(test)]
mod tests;
