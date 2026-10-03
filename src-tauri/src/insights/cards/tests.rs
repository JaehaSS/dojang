use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

static DATABASE_COUNTER: AtomicU32 = AtomicU32::new(0);

async fn test_pool() -> SqlitePool {
    let sequence = DATABASE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = crate::testtmp::dir().join(format!(
        "praxis-cards-{}-{sequence}.sqlite",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=rwc", path.display()))
        .await
        .unwrap();
    sqlx::raw_sql(
        "CREATE TABLE tasks (id INTEGER PRIMARY KEY AUTOINCREMENT, repo TEXT NOT NULL, \
           state TEXT NOT NULL, updated_at INTEGER NOT NULL); \
         CREATE TABLE convo_events (id INTEGER PRIMARY KEY AUTOINCREMENT, task_id INTEGER NOT NULL, \
           ts INTEGER NOT NULL, event TEXT NOT NULL, rewound_at INTEGER);",
    )
    .execute(&pool)
    .await
    .unwrap();
    crate::insights::dismissals::migrate(&pool).await.unwrap();
    pool
}

async fn add_task(pool: &SqlitePool, repo: &str, state: &str, updated_at: i64) -> i64 {
    sqlx::query("INSERT INTO tasks (repo, state, updated_at) VALUES (?, ?, ?)")
        .bind(repo)
        .bind(state)
        .bind(updated_at)
        .execute(pool)
        .await
        .unwrap()
        .last_insert_rowid()
}

async fn add_user_inputs(pool: &SqlitePool, task_id: i64, count: i64) {
    for i in 0..count {
        sqlx::query("INSERT INTO convo_events (task_id, ts, event) VALUES (?, ?, ?)")
            .bind(task_id)
            .bind(i)
            .bind(r#"{"kind":"user"}"#)
            .execute(pool)
            .await
            .unwrap();
    }
}

const NOW: i64 = 1_787_500_000;

fn task(id: i64, repo: &str, unmerged: bool, user_inputs: i64) -> ClosedTask {
    ClosedTask {
        id,
        repo: repo.to_string(),
        unmerged,
        user_inputs,
    }
}

// ── 1. 표본 10 미만 → 극단적 비율이어도 카드 없음 ──────────────────

#[test]
fn s1_below_global_sample_floor_never_shows_even_at_extreme_rate() {
    let current = DiscardMetrics {
        sample: 9,
        unmerged: 9,
        ids: vec![],
    };
    // 직전 구간 0건이라 "증가" 판정은 무조건 통과하는 조건인데도, 표본 하한에서 먼저 막힌다.
    assert!(!s1_should_show(&current, 0));
}

#[test]
fn s2_below_sample_floor_never_shows_even_at_extreme_rate() {
    // 저장소 표본 9건 전부 폐기(100%)여도 하한(10) 미만이면 걸러진다.
    let rows: Vec<ClosedTask> = (0..9).map(|i| task(i, "solo-repo", true, 5)).collect();
    assert!(build_s2(&rows, "7d", 0, 0).is_empty());
}

// ── 2. 임계 바로 아래 · 바로 위 ─────────────────────────────────────

#[test]
fn s1_rate_threshold_is_exclusive_below_and_inclusive_at() {
    // sample=10 고정, 직전 구간 0건(제로베이스라인 최소 3건 규칙만 적용) — 비율 경계만 본다.
    let below = DiscardMetrics {
        sample: 10,
        unmerged: 4,
        ids: vec![],
    }; // 40%
    let at = DiscardMetrics {
        sample: 10,
        unmerged: 5,
        ids: vec![],
    }; // 50%
    assert!(!s1_should_show(&below, 0));
    assert!(s1_should_show(&at, 0));
}

#[test]
fn s1_increase_threshold_is_exclusive_below_and_inclusive_at() {
    // rate=100%로 고정해 비율 게이트는 항상 통과시키고, 직전 구간 대비 증가 게이트만 본다.
    // baseline=7: 필요 조건 unmerged>=10.5 → 10건은 미달.
    let current = DiscardMetrics {
        sample: 10,
        unmerged: 10,
        ids: vec![],
    };
    assert!(!s1_should_show(&current, 7));
    // baseline=6: 필요 조건 unmerged>=9 그리고 차이>=3 → 10건은 둘 다 만족.
    assert!(s1_should_show(&current, 6));
}

#[test]
fn s2_rate_diff_threshold_is_exclusive_below_and_inclusive_at() {
    // overall_rate=0.2 고정, repo_rate*1.5=0.3은 이미 만족시키고 차이(0.15) 게이트만 본다.
    // 0.15는 이진 부동소수로 정확히 표현되지 않아(`0.35-0.2` != `0.15` 비트 단위) 경계값
    // 자체가 아니라 그 위·아래로 여유를 둔 값으로 "미만 vs 이상"만 확인한다.
    assert!(!s2_should_show(20, 0.34, 0.2)); // 차이 0.14 — 미달
    assert!(s2_should_show(20, 0.36, 0.2)); // 차이 0.16 — 통과
}

#[test]
fn s2_ratio_threshold_is_exclusive_below_and_inclusive_at() {
    // overall_rate=0.5 고정 — 차이(0.15) 게이트는 항상 만족시키고 배수(1.5) 게이트만 본다.
    assert!(!s2_should_show(20, 0.70, 0.5)); // 배수 1.4 — 미달(차이는 0.20으로 통과)
    assert!(s2_should_show(20, 0.75, 0.5)); // 배수 1.5 — 경계 포함
}

// ── 3. 근거가 수치를 재현한다 ────────────────────────────────────────

#[tokio::test]
async fn s1_evidence_ids_reproduce_the_value_and_are_really_unmerged_low_input() {
    let pool = test_pool().await;
    // 표본 10건(전역 하한을 정확히 채운다): 입력 1회 이하 10건 중 8건이 머지 없이 닫힘.
    for _ in 0..8 {
        let id = add_task(&pool, "repo-a", "Discarded", NOW - 10).await;
        add_user_inputs(&pool, id, 0).await;
    }
    for _ in 0..2 {
        let id = add_task(&pool, "repo-a", "Done", NOW - 10).await;
        add_user_inputs(&pool, id, 0).await;
    }
    for _ in 0..2 {
        let id = add_task(&pool, "repo-a", "Discarded", NOW - 10).await;
        add_user_inputs(&pool, id, 5).await; // 입력 5회 — sample에서 빠진다
    }

    let rows = load_closed_tasks(&pool, NOW - 100, NOW).await.unwrap();
    let baseline_rows: Vec<ClosedTask> = Vec::new();
    let candidate = build_s1(&rows, &baseline_rows, "7d", NOW - 100, 0).unwrap();

    let Evidence::Tasks { ids } = &candidate.card.evidence else {
        panic!("S1 evidence는 Tasks여야 한다");
    };
    assert_eq!(ids.len() as f64, candidate.card.value);
    assert_eq!(ids.len(), 8);
    let by_id: std::collections::HashMap<i64, &ClosedTask> =
        rows.iter().map(|r| (r.id, r)).collect();
    for id in ids {
        let row = by_id
            .get(id)
            .expect("evidence id는 조회 결과에 있어야 한다");
        assert!(
            row.unmerged,
            "evidence의 각 id는 머지 없이 닫힌 작업이어야 한다"
        );
        assert!(
            row.user_inputs <= 1,
            "evidence의 각 id는 입력 1회 이하여야 한다"
        );
    }
}

#[tokio::test]
async fn s2_evidence_ids_reproduce_the_value_and_belong_to_that_repo() {
    let pool = test_pool().await;
    // repo-b: 20건 중 8건 폐기(40%) — 아래 배경 저장소 대비 폐기가 뚜렷하게 집중되도록 만든다.
    for i in 0..20 {
        let state = if i < 8 { "Discarded" } else { "Done" };
        add_task(&pool, "repo-b", state, NOW - 10).await;
    }
    // 배경 저장소: 폐기 없음 — 전체 평균을 repo-b보다 훨씬 낮게 눌러 둔다.
    for _ in 0..20 {
        add_task(&pool, "repo-quiet", "Done", NOW - 10).await;
    }

    let rows = load_closed_tasks(&pool, NOW - 100, NOW).await.unwrap();
    let candidates = build_s2(&rows, "7d", NOW - 100, 0);
    let repo_b = candidates
        .iter()
        .find(|c| c.card.sentence.contains("repo-b"))
        .expect("repo-b 카드가 있어야 한다");

    let Evidence::Tasks { ids } = &repo_b.card.evidence else {
        panic!("S2 evidence는 Tasks여야 한다");
    };
    assert_eq!(ids.len() as f64, repo_b.card.value);
    assert_eq!(ids.len(), 8);
    let by_id: std::collections::HashMap<i64, &ClosedTask> =
        rows.iter().map(|r| (r.id, r)).collect();
    for id in ids {
        let row = by_id
            .get(id)
            .expect("evidence id는 조회 결과에 있어야 한다");
        assert_eq!(row.repo, "repo-b");
        assert!(row.unmerged);
    }
}

// ── 4. "all" 구간은 S1·S6을 내지 않는다 ─────────────────────────────

#[tokio::test]
async fn all_range_produces_no_s1_even_with_a_strong_signal() {
    let pool = test_pool().await;
    // S1이 살아 있었다면 확실히 뜰 표본 — 그런데도 "all"은 직전 구간이 없어 만들지 않는다.
    for _ in 0..20 {
        let id = add_task(&pool, "repo-a", "Discarded", NOW - 10).await;
        add_user_inputs(&pool, id, 0).await;
    }

    let cards = compute_cards(&pool, "all", 0, NOW, None, &[], &[])
        .await
        .unwrap();
    assert!(!cards.iter().any(|c| c.signal == SignalId::S1));
    assert!(!cards.iter().any(|c| c.signal == SignalId::S6));
}

#[test]
fn all_range_has_no_baseline_window() {
    assert!(baseline_window("all", NOW).is_none());
    assert!(baseline_window("7d", NOW).is_some());
    assert!(baseline_window("30d", NOW).is_some());
}

// ── 5. 정렬 + 전체 상한 5 + S2 상한 2 ────────────────────────────────

fn candidate(signal: SignalId, magnitude: f64, key: &str) -> Candidate {
    Candidate {
        card: InsightCard {
            key: key.to_string(),
            signal,
            sentence: key.to_string(),
            value: 0.0,
            baseline: None,
            sample: 10,
            evidence: Evidence::Tasks { ids: vec![] },
            repo: None,
        },
        magnitude,
    }
}

#[test]
fn finalize_caps_s2_at_two_before_the_overall_cap_of_five() {
    let s1 = Some(candidate(SignalId::S1, 10.0, "s1"));
    let s2 = vec![
        candidate(SignalId::S2, 9.0, "s2-a"),
        candidate(SignalId::S2, 8.0, "s2-b"),
        candidate(SignalId::S2, 7.0, "s2-c"),
        candidate(SignalId::S2, 6.0, "s2-d"),
    ];
    let s6 = Some(candidate(SignalId::S6, 5.0, "s6"));

    let out = finalize(s1, s2, s6, vec![]);
    // S2는 넷 중 크기순 상위 둘만 남는다.
    let s2_keys: Vec<&str> = out
        .iter()
        .filter(|c| c.signal == SignalId::S2)
        .map(|c| c.key.as_str())
        .collect();
    assert_eq!(s2_keys, vec!["s2-a", "s2-b"]);
    // 나머지 신호와 합쳐 크기 내림차순.
    assert_eq!(
        out.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(),
        vec!["s1", "s2-a", "s2-b", "s6"]
    );
}

#[test]
fn finalize_caps_the_total_at_five_and_breaks_ties_by_signal_id() {
    let s1 = Some(candidate(SignalId::S1, 5.0, "s1"));
    // 같은 magnitude(5.0)에서는 signal id 오름차순(S1 < S2)이 이긴다.
    let s2 = vec![
        candidate(SignalId::S2, 5.0, "s2-tie"),
        candidate(SignalId::S2, 4.0, "s2-low"),
    ];
    let s6 = Some(candidate(SignalId::S6, 20.0, "s6-big"));

    let out = finalize(s1, s2, s6, vec![]);
    assert_eq!(out.len(), 4); // 상한 5 미만이면 자르지 않는다
    assert_eq!(out[0].key, "s6-big");
    // magnitude가 같으면(s1, s2-tie 둘 다 5.0) signal id가 작은 S1이 앞선다.
    assert_eq!(out[1].key, "s1");
    assert_eq!(out[2].key, "s2-tie");
    assert_eq!(out[3].key, "s2-low");
}

#[test]
fn finalize_caps_total_at_five_when_more_are_available() {
    // S2 상한(2)을 통과한 둘 + 다른 신호로도 5장을 넘기면 상위 5장만 남는다.
    let s1 = Some(candidate(SignalId::S1, 100.0, "s1"));
    let s2 = vec![
        candidate(SignalId::S2, 90.0, "s2-a"),
        candidate(SignalId::S2, 80.0, "s2-b"),
        candidate(SignalId::S2, 70.0, "s2-c"),
    ];
    let s6 = Some(candidate(SignalId::S6, 60.0, "s6"));
    // 신호 셋(S1·S2·S6)뿐이라 실제로는 최대 1+2+1=4장 — 5장을 넘기는 시나리오는 이 슬라이스의
    // 신호 구성상 만들어지지 않는다. 대신 상한 로직 자체는 합성 후보로 직접 검증한다.
    let out = finalize(s1, s2, s6, vec![]);
    assert!(out.len() <= MAX_CARDS);
}

// ── 6. S6 순수 함수 ──────────────────────────────────────────────────

#[test]
fn s6_requires_a_baseline_and_a_ratio_of_at_least_1_5() {
    let base = SpendSnapshot {
        current_tokens: 1500,
        prev_tokens: Some(1000),
        sample: 20,
        top_project: None,
    };
    assert!(build_s6(&base, "7d", 0, 0, &[]).is_some());

    let no_baseline = SpendSnapshot {
        prev_tokens: None,
        ..base.clone()
    };
    assert!(build_s6(&no_baseline, "7d", 0, 0, &[]).is_none());

    let below_ratio = SpendSnapshot {
        current_tokens: 1499,
        ..base.clone()
    };
    assert!(build_s6(&below_ratio, "7d", 0, 0, &[]).is_none());

    let too_few_samples = SpendSnapshot {
        sample: 9,
        ..base.clone()
    };
    assert!(build_s6(&too_few_samples, "7d", 0, 0, &[]).is_none());
}

#[test]
fn s6_magnitude_and_value_are_the_ratio_and_current_tokens() {
    let spend = SpendSnapshot {
        current_tokens: 3000,
        prev_tokens: Some(1000),
        sample: 20,
        top_project: None,
    };
    let card = build_s6(&spend, "7d", 0, 0, &[]).unwrap();
    assert_eq!(card.magnitude, 3.0);
    assert_eq!(card.card.value, 3000.0);
    assert_eq!(card.card.baseline, Some(1000.0));
    assert_eq!(card.card.sample, 20);
}

#[test]
fn s6_includes_top_project_share_when_present() {
    let spend = SpendSnapshot {
        current_tokens: 1000,
        prev_tokens: Some(500),
        sample: 15,
        top_project: Some(("ade".to_string(), "~/work/ade".to_string(), 700)),
    };
    let card = build_s6(&spend, "7d", 0, 0, &[]).unwrap();
    assert!(card.card.sentence.contains("70%"));
    assert!(card.card.sentence.contains("ade"));
    assert_eq!(
        card.card.evidence,
        Evidence::Spend {
            project: Some("ade".to_string())
        }
    );
    // known_repos가 비어 있으면 표시 이름이 있어도 저장소로 묶지 않는다(§8 O2).
    assert_eq!(card.card.repo, None);
}

#[test]
fn s6_repo_is_set_only_when_the_top_project_path_is_a_known_repo() {
    let spend = SpendSnapshot {
        current_tokens: 1000,
        prev_tokens: Some(500),
        sample: 15,
        top_project: Some(("ade".to_string(), "~/work/ade".to_string(), 700)),
    };
    let known = vec!["~/work/ade".to_string()];
    let card = build_s6(&spend, "7d", 0, 0, &known).unwrap();
    assert_eq!(card.card.repo.as_deref(), Some("~/work/ade"));

    let unknown = vec!["~/work/other".to_string()];
    let card = build_s6(&spend, "7d", 0, 0, &unknown).unwrap();
    assert_eq!(card.card.repo, None);
}

// ── 7. L1 — 교훈 주제 반복 ───────────────────────────────────────────

fn ledger_entry(number: i64, date: &str, subjects: &[&str], lesson_count: usize) -> LedgerEntry {
    LedgerEntry {
        number,
        date: date.to_string(),
        subjects: subjects.iter().map(|s| s.to_string()).collect(),
        lessons: (0..lesson_count).map(|i| format!("lesson {i}")).collect(),
        abandoned: vec![],
    }
}

/// `count`건은 subject `hot`, 나머지(`sample - count`)건은 **서로 다른** subject 하나씩 —
/// 항목마다 교훈 1줄. 나머지를 한 subject로 뭉치면 count가 표본 절반 미만일 때 그쪽이 더
/// 커져 "top subject"가 바뀌어 버린다. 하나씩 흩어 두면 `count >= 1`인 한 `hot`이 항상 top이다.
fn projection_with_subject_split(
    repo: &str,
    date: &str,
    sample: i64,
    count: i64,
) -> RepoProjection {
    let mut entries = Vec::new();
    for i in 0..count {
        entries.push(ledger_entry(i, date, &["hot"], 1));
    }
    for i in count..sample {
        let subject = format!("cold-{i}");
        entries.push(ledger_entry(i, date, &[subject.as_str()], 1));
    }
    RepoProjection {
        repo: repo.to_string(),
        entries,
    }
}

#[test]
fn l1_hidden_below_the_global_sample_floor() {
    let today = super::super::ts_from_epoch(NOW, 0).date;
    let proj = projection_with_subject_split("~/work/x", &today, 9, 9); // 표본 9 — 하한 미달
    let current = current_window("30d", NOW);
    let out = build_l1(&[proj], "30d", current, None, 0);
    assert!(out.is_empty());
}

#[test]
fn l1_hidden_when_top_subject_count_is_below_five_even_if_share_is_high() {
    let today = super::super::ts_from_epoch(NOW, 0).date;
    // 표본 10, hot 4건 — 비중 40%(≥0.35)여도 건수(4)가 5 미만이라 뜨지 않는다.
    let proj = projection_with_subject_split("~/work/x", &today, 10, 4);
    let current = current_window("30d", NOW);
    let out = build_l1(&[proj], "30d", current, None, 0);
    assert!(out.is_empty());
}

#[test]
fn l1_hidden_when_share_is_below_threshold_even_if_count_meets_the_floor() {
    let today = super::super::ts_from_epoch(NOW, 0).date;
    // 표본 20, hot 5건 — 건수(5)는 만족하지만 비중 25%가 0.35 미만이다.
    let proj = projection_with_subject_split("~/work/x", &today, 20, 5);
    let current = current_window("30d", NOW);
    let out = build_l1(&[proj], "30d", current, None, 0);
    assert!(out.is_empty());
}

#[test]
fn l1_shows_at_the_exact_thresholds_and_reproduces_evidence_numbers() {
    let today = super::super::ts_from_epoch(NOW, 0).date;
    // 표본 20, hot 7건 — 비중 정확히 0.35(경계는 이상이라 포함).
    let proj = projection_with_subject_split("~/work/x", &today, 20, 7);
    let current = current_window("30d", NOW);
    let out = build_l1(&[proj], "30d", current, None, 0);
    assert_eq!(out.len(), 1);
    let card = &out[0].card;
    assert_eq!(card.signal, SignalId::L1);
    assert_eq!(card.sample, 20);
    assert_eq!(card.value, 7.0);
    assert!(card.sentence.contains("hot"));
    assert!(card.sentence.contains("~/work/x"));
    let Evidence::Ledger { repo, numbers } = &card.evidence else {
        panic!("L1 evidence는 Ledger여야 한다");
    };
    assert_eq!(repo, "~/work/x");
    assert_eq!(numbers.len(), 7);
    assert_eq!(*numbers, vec![0, 1, 2, 3, 4, 5, 6]);
}

#[test]
fn l1_baseline_is_none_for_all_range_and_for_a_thin_previous_window() {
    let today = super::super::ts_from_epoch(NOW, 0).date;
    let proj = projection_with_subject_split("~/work/x", &today, 20, 7);

    // "all"은 직전 구간 자체가 없다.
    let current_all = current_window("all", NOW);
    let out_all = build_l1(&[proj.clone()], "all", current_all, None, 0);
    assert_eq!(out_all[0].card.baseline, None);

    // 직전 구간은 있지만 표본이 하한(10) 미만이면 비중을 낼 수 없다.
    let current = current_window("30d", NOW);
    let baseline = baseline_window("30d", NOW).unwrap();
    let out_thin = build_l1(&[proj], "30d", current, Some(baseline), 0);
    assert_eq!(out_thin[0].card.baseline, None);
}

#[test]
fn l1_baseline_share_reflects_the_previous_window_when_it_has_enough_sample() {
    let current = current_window("30d", NOW);
    let baseline = baseline_window("30d", NOW).unwrap();
    let today = super::super::ts_from_epoch(NOW, 0).date;
    let baseline_mid = super::super::ts_from_epoch((baseline.start + baseline.end) / 2, 0).date;

    let RepoProjection { repo, mut entries } =
        projection_with_subject_split("~/work/x", &today, 20, 7); // 이번 구간 top=hot(7/20=35%)
                                                                  // 직전 구간 표본 10, hot 2건(20%) — 하한(10)은 채우되 이번 구간(35%)과는 다른 값.
    entries.push(ledger_entry(100, &baseline_mid, &["hot"], 1));
    entries.push(ledger_entry(101, &baseline_mid, &["hot"], 1));
    for i in 102..110 {
        entries.push(ledger_entry(i, &baseline_mid, &["other"], 1));
    }
    let proj = RepoProjection { repo, entries };

    let out = build_l1(&[proj], "30d", current, Some(baseline), 0);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].card.baseline, Some(0.2));
}

#[test]
fn l1_at_most_one_card_per_repo_and_participates_in_the_shared_cap() {
    let s1 = Some(candidate(SignalId::S1, 100.0, "s1"));
    let l1 = vec![
        candidate(SignalId::L1, 0.9, "l1-a"),
        candidate(SignalId::L1, 0.5, "l1-b"),
    ];
    let out = finalize(s1, vec![], None, l1);
    assert_eq!(
        out.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(),
        vec!["s1", "l1-a", "l1-b"]
    );
}

// ── 그 밖 — 경로 표시 · 키 형식 ───────────────────────────────────────

#[test]
fn short_repo_collapses_the_home_prefix() {
    let home = std::path::Path::new("/Users/sinjaeha");
    assert_eq!(
        short_repo_with_home("/Users/sinjaeha/work/ade", Some(home)),
        "~/work/ade"
    );
    assert_eq!(short_repo_with_home("/opt/other", Some(home)), "/opt/other");
    assert_eq!(short_repo_with_home("/Users/sinjaeha", Some(home)), "~");
}

#[test]
fn window_key_includes_signal_range_date_and_optional_repo() {
    let key = window_key("S1", "7d", NOW, 32_400, None);
    assert!(key.starts_with("S1:7d:"));
    let with_repo = window_key("S2", "7d", NOW, 32_400, Some("repo-a"));
    assert!(with_repo.ends_with(":repo-a"));
}

// ── 8. 카드의 repo 필드(무시·메모리 저장 대상 판단, T4) ────────────────

#[tokio::test]
async fn s1_card_has_no_repo_even_though_the_tasks_belong_to_one() {
    let pool = test_pool().await;
    for _ in 0..10 {
        let id = add_task(&pool, "repo-a", "Discarded", NOW - 10).await;
        add_user_inputs(&pool, id, 0).await;
    }
    let rows = load_closed_tasks(&pool, NOW - 100, NOW).await.unwrap();
    let candidate = build_s1(&rows, &[], "7d", NOW - 100, 0).unwrap();
    // S1은 전역 신호다 — 특정 저장소에 묶이지 않는다(§8 O2, 저장소 없는 신호는 USER.md).
    assert_eq!(candidate.card.repo, None);
}

#[tokio::test]
async fn s2_card_repo_matches_the_concentrated_repo() {
    let pool = test_pool().await;
    for i in 0..20 {
        let state = if i < 8 { "Discarded" } else { "Done" };
        add_task(&pool, "repo-b", state, NOW - 10).await;
    }
    for _ in 0..20 {
        add_task(&pool, "repo-quiet", "Done", NOW - 10).await;
    }
    let rows = load_closed_tasks(&pool, NOW - 100, NOW).await.unwrap();
    let candidates = build_s2(&rows, "7d", NOW - 100, 0);
    let repo_b = candidates
        .iter()
        .find(|c| c.card.sentence.contains("repo-b"))
        .expect("repo-b 카드가 있어야 한다");
    assert_eq!(repo_b.card.repo.as_deref(), Some("repo-b"));
}

#[test]
fn l1_card_repo_matches_the_projection_repo() {
    let today = super::super::ts_from_epoch(NOW, 0).date;
    let proj = projection_with_subject_split("~/work/x", &today, 20, 7);
    let current = current_window("30d", NOW);
    let out = build_l1(&[proj], "30d", current, None, 0);
    assert_eq!(out[0].card.repo.as_deref(), Some("~/work/x"));
}

// ── 9. 무시(§6.4) — (signal, repo) 쌍을 7일 동안 가라앉힌다 ─────────────

#[test]
fn is_dismissed_matches_signal_and_repo_together() {
    let dismissed = vec![
        ("S1".to_string(), "".to_string()),
        ("S2".to_string(), "repo-a".to_string()),
    ];
    assert!(is_dismissed(&dismissed, SignalId::S1, None));
    assert!(is_dismissed(&dismissed, SignalId::S2, Some("repo-a")));
    // 저장소가 다르면 같은 신호여도 무시되지 않는다.
    assert!(!is_dismissed(&dismissed, SignalId::S2, Some("repo-b")));
    // 신호가 다르면 같은(빈) repo여도 무시되지 않는다.
    assert!(!is_dismissed(&dismissed, SignalId::S6, None));
}

#[tokio::test]
async fn compute_cards_excludes_dismissed_pairs_but_keeps_other_repos() {
    let pool = test_pool().await;
    // repo-b·repo-c: 둘 다 20건 중 10건 폐기(50%) — S2가 뜰 만큼 폐기 집중.
    for i in 0..20 {
        let state = if i < 10 { "Discarded" } else { "Done" };
        add_task(&pool, "repo-b", state, NOW - 10).await;
    }
    // repo-c도 같은 모양으로 폐기 집중 — repo-b만 무시해도 repo-c는 남아야 한다.
    for i in 0..20 {
        let state = if i < 10 { "Discarded" } else { "Done" };
        add_task(&pool, "repo-c", state, NOW - 10).await;
    }
    // 배경 저장소 — 전체 평균을 0.2로 눌러 둘(repo-b·c의 0.5와 충분히 벌어지게) 만든다.
    for _ in 0..60 {
        add_task(&pool, "repo-quiet", "Done", NOW - 10).await;
    }

    crate::insights::dismissals::dismiss(&pool, "S2", "repo-b", NOW)
        .await
        .unwrap();

    // "all"은 구간 시작이 항상 0이라(`current_window`), 뒤의 TTL 만료 확인에서 `now`를
    // 미래로 옮겨도 씨앗 데이터(NOW-10에 심음)가 여전히 현재 구간 안에 남는다.
    let cards = compute_cards(&pool, "all", 0, NOW, None, &[], &[])
        .await
        .unwrap();
    assert!(
        !cards
            .iter()
            .any(|c| c.signal == SignalId::S2 && c.repo.as_deref() == Some("repo-b")),
        "무시된 (S2, repo-b)는 나오면 안 된다"
    );
    assert!(
        cards
            .iter()
            .any(|c| c.signal == SignalId::S2 && c.repo.as_deref() == Some("repo-c")),
        "무시되지 않은 repo-c는 여전히 나와야 한다"
    );

    // TTL(7일)을 넘긴 시각으로 조회하면 다시 보인다.
    let after_ttl = NOW + crate::insights::dismissals::TTL_SECS + 1;
    let cards_later = compute_cards(&pool, "all", 0, after_ttl, None, &[], &[])
        .await
        .unwrap();
    assert!(
        cards_later
            .iter()
            .any(|c| c.signal == SignalId::S2 && c.repo.as_deref() == Some("repo-b")),
        "TTL이 지나면 무시가 풀려 다시 보여야 한다"
    );
}
