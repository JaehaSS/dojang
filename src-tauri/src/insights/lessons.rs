//! 교훈 투영 읽기 · 교훈 주제 집계(설계 §6, §9 T3).
//!
//! `docs/lessons.json`만 읽는다 — md를 직접 파싱하지 않는다. 원장 파서는 `scripts/lib/ledger.mjs`
//! 한 벌뿐이어야 한다(§6.1, CLAUDE.md "원장·색인 파서는 한 벌뿐이다") — Rust가 두 번째 파서가
//! 되면 중복 필드에서 교훈이 사라진 전례가 반복된다.
//!
//! 파일이 없거나 읽을 수 없거나 JSON이 아니거나 `schema`가 1이 아니면, 그 저장소는 "투영이
//! 없다"로 조용히 처리한다(§6.3) — 모르는 형식을 추정으로 읽지 않는다. 에러를 구분해 알리지
//! 않는 것은 의도적이다: 앱 입장에서 셋 다 "이 저장소는 교훈 주제 섹션·L1을 낼 재료가 없다"는
//! 같은 결론으로 이어진다.

use std::fs;
use std::path::Path;

use serde::Deserialize;

use super::cards::Window;

/// 교훈 주제 섹션 — subject당 최대 이 개수만 보여준다(화면을 덮지 않는다).
const MAX_SUBJECTS: usize = 8;
/// 버린 길 목록 상한.
const MAX_ABANDONED: usize = 20;

// ── 투영 읽기 ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
struct RawProjection {
    schema: u32,
    #[serde(default)]
    entries: Vec<RawEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawEntry {
    number: i64,
    date: String,
    #[serde(default)]
    subjects: Vec<String>,
    #[serde(default)]
    lessons: Vec<String>,
    #[serde(default)]
    abandoned: Vec<String>,
}

/// `docs/lessons.json`의 항목 하나. `status`는 이 슬라이스에서 쓰지 않아 옮기지 않는다.
#[derive(Debug, Clone, PartialEq)]
pub struct LedgerEntry {
    pub number: i64,
    pub date: String,
    pub subjects: Vec<String>,
    pub lessons: Vec<String>,
    pub abandoned: Vec<String>,
}

/// 저장소 하나의 교훈 투영.
#[derive(Debug, Clone, PartialEq)]
pub struct RepoProjection {
    pub repo: String,
    pub entries: Vec<LedgerEntry>,
}

/// `{repo_dir}/docs/lessons.json`을 읽는다. 실패 사유(없음 · 읽기 실패 · JSON 아님 · schema
/// 불일치)를 구분하지 않고 전부 `None`으로 합친다 — 위 모듈 문서 참고.
pub fn load_projection(repo_dir: &Path) -> Option<Vec<LedgerEntry>> {
    let raw = fs::read_to_string(repo_dir.join("docs/lessons.json")).ok()?;
    let parsed: RawProjection = serde_json::from_str(&raw).ok()?;
    if parsed.schema != 1 {
        return None;
    }
    Some(
        parsed
            .entries
            .into_iter()
            .map(|e| LedgerEntry {
                number: e.number,
                date: e.date,
                subjects: e.subjects,
                lessons: e.lessons,
                abandoned: e.abandoned,
            })
            .collect(),
    )
}

/// 저장소 경로 목록에서 투영이 있는 것만 골라 싣는다. 디렉터리가 없으면 조용히 건너뛴다.
/// 파일 IO라 호출자가 blocking 풀로 보내야 한다(이 모듈의 다른 조사와 같은 규칙).
pub fn load_projections(repos: &[String]) -> Vec<RepoProjection> {
    repos
        .iter()
        .filter(|r| Path::new(r).is_dir())
        .filter_map(|r| {
            load_projection(Path::new(r)).map(|entries| RepoProjection {
                repo: r.clone(),
                entries,
            })
        })
        .collect()
}

// ── 구간 — 날짜 경계 ──────────────────────────────────────────────────

/// 로컬 날짜(YYYY-MM-DD) 반개구간 `[start, end)`. 원장 항목의 `date`는 이미 로컬 날짜 문자열이라
/// 사전순 비교가 곧 날짜 비교다.
pub(super) struct DateWindow {
    start: String,
    end_exclusive: String,
}

impl DateWindow {
    pub(super) fn contains(&self, date: &str) -> bool {
        date >= self.start.as_str() && date < self.end_exclusive.as_str()
    }
}

/// 현재 구간 → 날짜 경계. 상한은 "오늘"을 포함해야 하므로 다음 날을 배타 상한으로 쓴다
/// (`Window.end`는 `now` 자체라 그 날짜로 배타 비교하면 오늘 항목이 빠진다).
pub(super) fn current_date_window(w: Window, tz_offset_secs: i64) -> DateWindow {
    DateWindow {
        start: super::ts_from_epoch(w.start, tz_offset_secs).date,
        end_exclusive: super::ts_from_epoch(w.end + 86_400, tz_offset_secs).date,
    }
}

/// 직전 구간 → 날짜 경계. `cards::baseline_window`가 `end`를 현재 구간의 시작과 정확히
/// 같은 epoch로 맞춰 두므로(반개구간), 날짜도 그대로 배타 상한이 된다 — 보정이 필요 없다.
pub(super) fn baseline_date_window(w: Window, tz_offset_secs: i64) -> DateWindow {
    DateWindow {
        start: super::ts_from_epoch(w.start, tz_offset_secs).date,
        end_exclusive: super::ts_from_epoch(w.end, tz_offset_secs).date,
    }
}

// ── 교훈 주제 집계(§6, 교훈 주제 섹션) ─────────────────────────────────

/// subject 하나의 빈도 — `lessons`는 이 subject가 붙은, 이 구간 항목들의 **교훈 줄 수 합**이다.
/// 항목 하나가 교훈을 둘 남기면 2로 센다(§6.2 "배열 전체를 싣는다"를 집계에도 그대로 반영).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SubjectCount {
    pub subject: String,
    pub lessons: i64,
    /// 직전 구간의 같은 지표. "all"은 직전 구간이 없어 항상 `None`이다.
    pub prev_lessons: Option<i64>,
    /// 이 합을 재현할 수 있는 항목 번호(교훈이 있는 것만).
    pub numbers: Vec<i64>,
}

/// 버린 길 한 줄 — `abandoned` 배열의 원소 하나가 곧 한 항목이다(§6.2 "배열 전체").
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AbandonedItem {
    pub number: i64,
    pub date: String,
    pub subjects: Vec<String>,
    pub text: String,
}

/// 저장소 하나의 교훈 주제 — 투영은 있지만 이 구간에 교훈도 버린 길도 없으면 만들지 않는다
/// (`repo_lessons`가 `None`을 돌려준다. §6.3 "빈 0을 그리지 않는다").
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RepoLessons {
    pub repo: String,
    /// 이 구간에 속한 원장 항목 수(교훈 유무와 무관) — subject 막대가 보여주는 것보다 넓은,
    /// "이 저장소가 이 구간에 얼마나 활동했나"의 맥락.
    pub entries_in_window: i64,
    pub subjects: Vec<SubjectCount>,
    pub abandoned: Vec<AbandonedItem>,
}

/// subject → (교훈 줄 수 합, 항목 번호들). 한 항목이 같은 subject를 두 번 적어도 한 번만 센다.
fn subject_lesson_sums<'a>(
    entries: &[&'a LedgerEntry],
) -> std::collections::BTreeMap<&'a str, (i64, Vec<i64>)> {
    let mut out: std::collections::BTreeMap<&str, (i64, Vec<i64>)> =
        std::collections::BTreeMap::new();
    for e in entries {
        if e.lessons.is_empty() {
            continue;
        }
        let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for s in &e.subjects {
            if seen.insert(s.as_str()) {
                let bucket = out.entry(s.as_str()).or_insert((0, Vec::new()));
                bucket.0 += e.lessons.len() as i64;
                bucket.1.push(e.number);
            }
        }
    }
    out
}

/// 저장소 하나의 교훈 주제. 이 구간에 교훈도 버린 길도 없으면 `None`(§6.3).
pub fn repo_lessons(
    projection: &RepoProjection,
    range: &str,
    now: i64,
    tz_offset_secs: i64,
) -> Option<RepoLessons> {
    let current = super::cards::current_window(range, now);
    let baseline = super::cards::baseline_window(range, now);
    let cw = current_date_window(current, tz_offset_secs);
    let bw = baseline.map(|b| baseline_date_window(b, tz_offset_secs));

    let window_entries: Vec<&LedgerEntry> = projection
        .entries
        .iter()
        .filter(|e| cw.contains(&e.date))
        .collect();
    let entries_in_window = window_entries.len() as i64;

    let current_sums = subject_lesson_sums(&window_entries);
    let baseline_sums = bw.as_ref().map(|bw| {
        let baseline_entries: Vec<&LedgerEntry> = projection
            .entries
            .iter()
            .filter(|e| bw.contains(&e.date))
            .collect();
        subject_lesson_sums(&baseline_entries)
    });

    let mut subjects: Vec<SubjectCount> = current_sums
        .into_iter()
        .map(|(subject, (lessons, numbers))| {
            let prev_lessons = baseline_sums
                .as_ref()
                .map(|prev| prev.get(subject).map(|(n, _)| *n).unwrap_or(0));
            SubjectCount {
                subject: subject.to_string(),
                lessons,
                prev_lessons,
                numbers,
            }
        })
        .collect();
    subjects.sort_by(|a, b| {
        b.lessons
            .cmp(&a.lessons)
            .then_with(|| a.subject.cmp(&b.subject))
    });
    subjects.truncate(MAX_SUBJECTS);

    let mut abandoned: Vec<AbandonedItem> = window_entries
        .iter()
        .flat_map(|e| {
            e.abandoned.iter().map(move |text| AbandonedItem {
                number: e.number,
                date: e.date.clone(),
                subjects: e.subjects.clone(),
                text: text.clone(),
            })
        })
        .collect();
    // 최신 우선 — 날짜 내림차순, 같은 날짜면 번호 내림차순(항목 등록 순서에 가장 가깝다).
    abandoned.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.number.cmp(&a.number)));
    abandoned.truncate(MAX_ABANDONED);

    if subjects.is_empty() && abandoned.is_empty() {
        return None;
    }

    Some(RepoLessons {
        repo: projection.repo.clone(),
        entries_in_window,
        subjects,
        abandoned,
    })
}

/// 저장소 전체의 교훈 주제 — 투영이 없거나 이 구간에 낼 것이 없는 저장소는 조용히 빠진다.
/// 반환 순서는 저장소 경로 오름차순(결정론).
pub fn compute_lesson_themes(
    projections: &[RepoProjection],
    range: &str,
    now: i64,
    tz_offset_secs: i64,
) -> Vec<RepoLessons> {
    let mut out: Vec<RepoLessons> = projections
        .iter()
        .filter_map(|p| repo_lessons(p, range, now, tz_offset_secs))
        .collect();
    out.sort_by(|a, b| a.repo.cmp(&b.repo));
    out
}

#[cfg(test)]
mod tests;
