use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

static DIR_COUNTER: AtomicU32 = AtomicU32::new(0);

/// 저장소 픽스처 디렉터리 — `{root}/docs/lessons.json`을 원하는 내용으로 채운다.
fn fixture_repo(body: Option<&str>) -> std::path::PathBuf {
    let sequence = DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
    let root =
        crate::testtmp::dir().join(format!("praxis-lessons-{}-{sequence}", std::process::id()));
    let docs = root.join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    if let Some(body) = body {
        std::fs::write(docs.join("lessons.json"), body).unwrap();
    }
    root
}

// ── 투영 읽기 — 없음 · 손상 · schema 불일치는 전부 None ────────────────

#[test]
fn missing_file_yields_no_projection() {
    let root = fixture_repo(None);
    assert!(load_projection(&root).is_none());
}

#[test]
fn invalid_json_yields_no_projection() {
    let root = fixture_repo(Some("{ not json"));
    assert!(load_projection(&root).is_none());
}

#[test]
fn schema_2_yields_no_projection() {
    let root = fixture_repo(Some(r#"{"schema":2,"entries":[]}"#));
    assert!(load_projection(&root).is_none());
}

#[test]
fn schema_1_reads_full_lesson_and_abandoned_arrays() {
    let root = fixture_repo(Some(
        r#"{"schema":1,"entries":[
            {"number":2,"date":"2026-01-02","subjects":["a/b"],"status":"done",
             "lessons":["첫 번째","두 번째"],"abandoned":["버린 길"]}
        ]}"#,
    ));
    let entries = load_projection(&root).expect("schema 1은 읽혀야 한다");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].lessons, vec!["첫 번째", "두 번째"]);
    assert_eq!(entries[0].abandoned, vec!["버린 길"]);
}

fn entry(
    number: i64,
    date: &str,
    subjects: &[&str],
    lessons: &[&str],
    abandoned: &[&str],
) -> LedgerEntry {
    LedgerEntry {
        number,
        date: date.to_string(),
        subjects: subjects.iter().map(|s| s.to_string()).collect(),
        lessons: lessons.iter().map(|s| s.to_string()).collect(),
        abandoned: abandoned.iter().map(|s| s.to_string()).collect(),
    }
}

const NOW: i64 = 1_800_000_000; // 2027-01-15 ~ (UTC, tz=0 기준 계산에만 쓴다)

// ── 구간 필터링 — 로컬 날짜 경계 ────────────────────────────────────

#[test]
fn current_window_excludes_dates_before_start_and_includes_today() {
    let today = super::super::ts_from_epoch(NOW, 0).date;
    let w = super::super::cards::current_window("30d", NOW);
    let cw = current_date_window(w, 0);
    let start = super::super::ts_from_epoch(w.start, 0).date;
    assert!(!cw.contains("2000-01-01"));
    assert!(cw.contains(&start));
    assert!(cw.contains(&today));
}

#[test]
fn baseline_window_excludes_the_boundary_day_owned_by_current() {
    let current = super::super::cards::current_window("7d", NOW);
    let baseline = super::super::cards::baseline_window("7d", NOW).unwrap();
    let current_start = super::super::ts_from_epoch(current.start, 0).date;
    let bw = baseline_date_window(baseline, 0);
    // 직전 구간의 배타 상한은 현재 구간의 시작일과 같다 — 그 날은 현재 몫이다.
    assert!(!bw.contains(&current_start));
}

// ── subject 집계 — 교훈 줄 수 합 ────────────────────────────────────

#[test]
fn subject_count_sums_lesson_lines_not_entry_count() {
    let projection = RepoProjection {
        repo: "~/work/x".to_string(),
        entries: vec![
            entry(1, "2027-01-10", &["a/b"], &["l1", "l2"], &[]),
            entry(2, "2027-01-11", &["a/b"], &["l3"], &[]),
        ],
    };
    let out = repo_lessons(&projection, "30d", NOW, 0).expect("교훈이 있어야 한다");
    let ab = out.subjects.iter().find(|s| s.subject == "a/b").unwrap();
    // 항목1이 교훈 2줄 + 항목2가 1줄 = 3. 항목 수(2)가 아니라 줄 수 합이어야 한다.
    assert_eq!(ab.lessons, 3);
    assert_eq!(ab.numbers, vec![1, 2]);
}

#[test]
fn repo_with_no_lessons_or_abandoned_in_window_is_omitted() {
    let projection = RepoProjection {
        repo: "~/work/empty".to_string(),
        entries: vec![entry(1, "2027-01-10", &["a/b"], &[], &[])],
    };
    assert!(repo_lessons(&projection, "30d", NOW, 0).is_none());
}

#[test]
fn all_range_has_no_previous_window_so_prev_lessons_is_none() {
    let projection = RepoProjection {
        repo: "~/work/x".to_string(),
        entries: vec![entry(1, "2020-01-10", &["a/b"], &["l1"], &[])],
    };
    let out = repo_lessons(&projection, "all", NOW, 0).unwrap();
    assert_eq!(out.subjects[0].prev_lessons, None);
}

#[test]
fn prev_lessons_reflects_the_previous_window_when_present() {
    let current = super::super::cards::current_window("7d", NOW);
    let current_start_date = super::super::ts_from_epoch(current.start, 0).date;
    let baseline = super::super::cards::baseline_window("7d", NOW).unwrap();
    let baseline_mid_date =
        super::super::ts_from_epoch((baseline.start + baseline.end) / 2, 0).date;
    let projection = RepoProjection {
        repo: "~/work/x".to_string(),
        entries: vec![
            entry(1, &current_start_date, &["a/b"], &["l1"], &[]),
            entry(2, &baseline_mid_date, &["a/b"], &["l2", "l3"], &[]),
        ],
    };
    let out = repo_lessons(&projection, "7d", NOW, 0).unwrap();
    let ab = out.subjects.iter().find(|s| s.subject == "a/b").unwrap();
    assert_eq!(ab.lessons, 1);
    assert_eq!(ab.prev_lessons, Some(2));
}

// ── 버린 길 — 최신 우선, 상한 ────────────────────────────────────────

#[test]
fn abandoned_items_are_newest_first_and_flatten_one_per_line() {
    let projection = RepoProjection {
        repo: "~/work/x".to_string(),
        entries: vec![
            entry(1, "2027-01-10", &["a/b"], &[], &["오래된 것"]),
            entry(2, "2027-01-12", &["a/b"], &[], &["최근 것1", "최근 것2"]),
        ],
    };
    let out = repo_lessons(&projection, "30d", NOW, 0).unwrap();
    assert_eq!(out.abandoned.len(), 3);
    assert_eq!(out.abandoned[0].text, "최근 것1");
    assert_eq!(out.abandoned[1].text, "최근 것2");
    assert_eq!(out.abandoned[2].text, "오래된 것");
}

#[test]
fn abandoned_items_are_capped_at_twenty() {
    let entries = (0..25)
        .map(|i| entry(i, "2027-01-10", &["a/b"], &[], &["버린 길"]))
        .collect();
    let projection = RepoProjection {
        repo: "~/work/x".to_string(),
        entries,
    };
    let out = repo_lessons(&projection, "30d", NOW, 0).unwrap();
    assert_eq!(out.abandoned.len(), 20);
}

#[test]
fn subjects_are_capped_at_eight_sorted_by_lessons_desc() {
    let entries = (0..10)
        .map(|i| {
            let subject = format!("subj/{i}");
            entry(i, "2027-01-10", &[subject.as_str()], &["l"], &[])
        })
        .collect();
    let projection = RepoProjection {
        repo: "~/work/x".to_string(),
        entries,
    };
    let out = repo_lessons(&projection, "30d", NOW, 0).unwrap();
    assert_eq!(out.subjects.len(), 8);
}

// ── load_projections — 존재하지 않는 디렉터리는 건너뛴다 ────────────

#[test]
fn load_projections_skips_nonexistent_dirs_and_dirs_without_projection() {
    let with_projection = fixture_repo(Some(r#"{"schema":1,"entries":[]}"#));
    let without_projection = fixture_repo(None);
    let repos = vec![
        with_projection.display().to_string(),
        without_projection.display().to_string(),
        "/no/such/path/at/all".to_string(),
    ];
    let out = load_projections(&repos);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].repo, with_projection.display().to_string());
}
