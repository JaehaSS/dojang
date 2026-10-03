//! 격리 스코프 안에서만 파일을 여는 유틸(`scoped_file`) — 옛 증거 저장/재검증(P2)은 제거됐다
//! (설계 2026-09-13 §P2 제거). 코드그래프/Wiki/vault/skills가 이 스코프 열기를 계속 쓴다.

pub(crate) mod scoped_file;
