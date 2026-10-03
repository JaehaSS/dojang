//! 신호 카드(설계 §5) — "이번 구간에 무엇이 달라졌나"를 문장으로 만든다.
//!
//! LLM은 쓰지 않는다 — 수치도 문장도 템플릿이다(§5.1). T2가 카드 프레임과 S1·S2·S6을,
//! T3(`lessons` 모듈)이 교훈 투영을 얹어 L1을 더했다. 행동(메모리 남기기 · 무시 · 조사 작업,
//! §8)은 T4다 — 여기서는 무시 필터링(`is_dismissed`)과 그 판단에 쓰는 `InsightCard::repo`만
//! 더한다. 메모리 저장 API는 `memory::file`, 조사 작업 프리필·근거 열기는 프런트 몫이다.
//!
//! 규칙 상수·판정·문장 렌더링·정렬은 DB 없이 테스트할 수 있는 순수 함수로 두고,
//! `compute_cards`만 조회를 담당하는 얇은 비동기 층이다.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use super::dismissals;
use super::lessons::{LedgerEntry, RepoProjection};

// ── 상수 ──────────────────────────────────────────────────────────

/// 전역 규칙 — 분모(sample)가 이 미만이면 어떤 신호도 카드를 내지 않는다. 작은 표본의
/// 비율은 소음이다(§5.1).
const MIN_SAMPLE: i64 = 10;

/// S1 — "입력 1회 이하로 닫힌 작업" 중 머지 없이 닫힌 비율이 이 이상이어야 한다.
const S1_MIN_RATE: f64 = 0.5;
/// S1 — 직전 구간 대비 배수 임계(직전 구간이 0건이 아닐 때).
const S1_MIN_RATIO: f64 = 1.5;
/// S1 — 직전 구간 대비 최소 절대 증가 건수(직전 구간이 0건이 아닐 때 배수와 함께 요구).
const S1_MIN_ABS_INCREASE: i64 = 3;
/// S1 — 직전 구간이 0건이면 배수를 계산할 수 없으므로, 이번 구간이 이 건수 이상이면
/// "늘었다"로 본다.
const S1_ZERO_BASELINE_MIN: i64 = 3;

/// S2 — 저장소별 표본도 전역 하한(`MIN_SAMPLE`)을 따로 적용한다(변수명을 붙여 의도를 명시).
const S2_MIN_SAMPLE: i64 = MIN_SAMPLE;
/// S2 — 저장소 폐기율이 전체 평균의 이 배 이상이어야 한다.
const S2_MIN_RATE_RATIO: f64 = 1.5;
/// S2 — 저장소 폐기율과 전체 평균의 차이(비율 포인트, 0.15 = 15%p)가 이 이상이어야 한다.
const S2_MIN_RATE_DIFF: f64 = 0.15;
/// S2 — 카드는 최대 이 개수만 낸다. 저장소가 여럿 걸려도 화면을 덮지 않는다.
const S2_MAX_CARDS: usize = 2;

/// S6 — 이번 구간 토큰 총량이 직전 구간의 이 배 이상이어야 "급증"으로 본다.
const S6_MIN_RATIO: f64 = 1.5;

/// L1 — 반복 신호가 뜨려면 가장 많이 걸린 subject의 항목 수가 이 이상이어야 한다.
const L1_MIN_SUBJECT_COUNT: i64 = 5;
/// L1 — 그 subject가 표본(교훈 있는 항목 수) 대비 이 비중 이상을 차지해야 한다.
const L1_MIN_SHARE: f64 = 0.35;

/// 전체 카드 상한(§5.1) — 크기순 정렬 후 이 개수만 남긴다.
const MAX_CARDS: usize = 5;

// ── 공개 타입 ──────────────────────────────────────────────────────

/// 첫 신호 세트(T2) + L1(교훈 주제 반복, T3). 그 밖의 후보는 아직 선택되지 않았다(§5.2 D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalId {
    S1,
    S2,
    S6,
    L1,
}

impl SignalId {
    /// 무시 테이블(`insight_dismissals.signal`)에 저장·비교할 때 쓰는 안정된 문자열이다.
    pub fn as_str(&self) -> &'static str {
        match self {
            SignalId::S1 => "S1",
            SignalId::S2 => "S2",
            SignalId::S6 => "S6",
            SignalId::L1 => "L1",
        }
    }
}

/// 카드 수치를 손으로 다시 셀 수 있게 하는 근거(§5.1 성공 기준 2). `Ledger`(교훈 근거, T3)는
/// 원장 번호를 담는다 — 작업 id가 아니라서 `Tasks`와 분리한다.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind")]
pub enum Evidence {
    Tasks { ids: Vec<i64> },
    Spend { project: Option<String> },
    Ledger { repo: String, numbers: Vec<i64> },
}

/// 신호 카드 한 장. `actions`(§8 메모리 남기기 · 무시 · 조사 작업)는 T4 — 이 슬라이스에는 없다.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct InsightCard {
    /// signal + range + 구간 시작일 — 무시 판정 키(T4에서 쓴다. 이 슬라이스는 무시 테이블이 없다).
    pub key: String,
    pub signal: SignalId,
    pub sentence: String,
    pub value: f64,
    /// 직전 동일 길이 구간의 같은 지표. S2는 예외로 "구간 평균"을 담는다(아래 `build_s2` 주석).
    pub baseline: Option<f64>,
    pub sample: i64,
    pub evidence: Evidence,
    /// 저장소에 묶인 신호만 `Some`(S2·L1, S6은 top project가 알려진 저장소일 때만).
    /// 무시(§6.4)·메모리 저장 대상(§8 O2) 판단이 이 필드를 쓴다(T4).
    pub repo: Option<String>,
}

/// `insights::compute`가 이미 낸 결과에서 S6에 필요한 값만 추린 것.
///
/// **한계**: 실제 USD 비용은 여기 없다. 달러 단가(`src/lib/pricing.ts`)는 프런트 전용 로직이고,
/// 백엔드가 노출하는 직전 구간 요약(`PeriodSummary`)에는 모델별 토큰 종류 분해가 없어(총 토큰
/// 하나뿐) 여기서 재현하면 입력·출력·캐시 단가가 다른데도 뭉뚱그린 값이 된다. 그래서 이 신호는
/// **달러 지출이 아니라 토큰 총량**의 구간 대비 배수를 쓴다 — "얼마나 늘었나"의 근사치이지
/// "얼마를 더 썼나"는 아니다. 사용자에게도 문장에서 "지출"이 아니라 "토큰 사용량"이라 적는다.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpendSnapshot {
    pub current_tokens: u64,
    /// "all" 구간처럼 직전 구간이 없으면 None — 이때 S6은 만들지 않는다.
    pub prev_tokens: Option<u64>,
    /// 이 수치를 뒷받침하는 표본. 날짜 수(`active_days`)는 7일 구간에서 최대 7이라
    /// 전역 하한(10)을 넘기지 못한다 — 세션 수를 쓴다.
    pub sample: i64,
    /// 이번 구간 토큰이 가장 많은 프로젝트(이름, 경로, 토큰). `Insights::projects`가 이미
    /// 토큰 내림차순이라 `first()`가 곧 최댓값이다. 경로는 S6이 "실제 작업 저장소"인지
    /// (`known_repos`에 있는지)를 가리는 데 쓴다(§8 O2) — 표시용 이름과는 다를 수 있다.
    pub top_project: Option<(String, String, u64)>,
}

impl SpendSnapshot {
    pub fn from_insights(insights: &super::Insights) -> Self {
        Self {
            current_tokens: insights.total_tokens,
            prev_tokens: insights.prev.as_ref().map(|p| p.total_tokens),
            sample: insights.sessions as i64,
            top_project: insights
                .projects
                .first()
                .map(|p| (p.name.clone(), p.path.clone(), p.total_tokens)),
        }
    }
}

/// 신호 카드 계산 — S1·S2는 작업 DB만, S6은 `spend`(파일 기반 `insights::compute` 결과)를,
/// L1은 `projections`(저장소별 교훈 투영, §6)을 쓴다. `spend`가 None이면(예: "all" 구간이라
/// 호출자가 무거운 파일 스캔을 건너뛴 경우) S6은 만들지 않는다. L1은 변화가 아니라 반복을 재는
/// 신호라 "all"에서도 만든다(§9 T3).
///
/// range 규칙은 `patterns::cutoff`·`outcomes::cutoff`와 같다(7d/30d/그 밖은 all).
pub async fn compute_cards(
    pool: &SqlitePool,
    range: &str,
    tz_offset_secs: i64,
    now: i64,
    spend: Option<&SpendSnapshot>,
    projections: &[RepoProjection],
    known_repos: &[String],
) -> anyhow::Result<Vec<InsightCard>> {
    let current = current_window(range, now);
    let baseline = baseline_window(range, now);

    let current_rows = load_closed_tasks(pool, current.start, current.end).await?;

    let s1 = if let Some(bw) = &baseline {
        let baseline_rows = load_closed_tasks(pool, bw.start, bw.end).await?;
        build_s1(
            &current_rows,
            &baseline_rows,
            range,
            current.start,
            tz_offset_secs,
        )
    } else {
        None
    };

    let s2 = build_s2(&current_rows, range, current.start, tz_offset_secs);

    let s6 = spend.and_then(|s| build_s6(s, range, current.start, tz_offset_secs, known_repos));

    let l1 = build_l1(projections, range, current, baseline, tz_offset_secs);

    // 무시(§6.4)는 후보 단계에서 걸러낸다 — `finalize`의 절단(S2 2장·전체 5장) *뒤*에
    // 걸러내면 무시된 카드가 자리를 차지해, 무시로 비운 자리에 다른 후보가 못 들어온다.
    let dismissed = dismissals::active(pool, now).await?;
    let keep = |c: &Candidate| !is_dismissed(&dismissed, c.card.signal, c.card.repo.as_deref());
    let s1 = s1.filter(keep);
    let s2: Vec<Candidate> = s2.into_iter().filter(|c| keep(c)).collect();
    let s6 = s6.filter(keep);
    let l1: Vec<Candidate> = l1.into_iter().filter(|c| keep(c)).collect();

    Ok(finalize(s1, s2, s6, l1))
}

/// `(signal, repo)`가 지금 무시 목록에 있는지 — 저장소 없는 카드는 빈 문자열로 비교한다
/// (`dismissals` 테이블의 `repo TEXT NOT NULL DEFAULT ''`와 짝을 맞춘다).
fn is_dismissed(dismissed: &[(String, String)], signal: SignalId, repo: Option<&str>) -> bool {
    let repo = repo.unwrap_or("");
    dismissed
        .iter()
        .any(|(s, r)| s == signal.as_str() && r == repo)
}

// ── 구간 ──────────────────────────────────────────────────────────

/// `insights` 하위 모듈(`lessons`)도 같은 구간 정의를 쓴다 — L1·교훈 주제 섹션이 S1·S2·S6과
/// 다른 구간을 쓰면 "이번 구간"의 의미가 신호마다 갈린다.
#[derive(Debug, Clone, Copy)]
pub(super) struct Window {
    pub(super) start: i64,
    pub(super) end: i64,
}

fn cutoff(range: &str, now: i64) -> i64 {
    match range {
        "7d" => now.saturating_sub(7 * 86_400),
        "30d" => now.saturating_sub(30 * 86_400),
        _ => 0,
    }
}

/// 현재 구간.
pub(super) fn current_window(range: &str, now: i64) -> Window {
    Window {
        start: cutoff(range, now),
        end: now,
    }
}

/// 직전 동일 길이 구간. "all"은 비교 대상이 없다 — S1·S6은 이 구간이 없으면 만들지 않는다
/// (설계 §5.1, T2 슬라이스 지시).
pub(super) fn baseline_window(range: &str, now: i64) -> Option<Window> {
    let span = match range {
        "7d" => 7 * 86_400,
        "30d" => 30 * 86_400,
        _ => return None,
    };
    let current_start = now.saturating_sub(span);
    Some(Window {
        start: current_start.saturating_sub(span),
        end: current_start,
    })
}

/// signal + range + 구간 시작일(로컬) [+ repo]. 무시 판정 키(T4)로 재사용된다.
fn window_key(
    signal: &str,
    range: &str,
    window_start: i64,
    tz_offset_secs: i64,
    repo: Option<&str>,
) -> String {
    // `super::ts_from_epoch`는 insights 모듈 전체가 이미 쓰는 로컬 시간대 변환이다 —
    // 여기서 날짜 계산을 다시 구현하면 두 번째 파서가 생긴다.
    let date = super::ts_from_epoch(window_start, tz_offset_secs).date;
    match repo {
        Some(r) => format!("{signal}:{range}:{date}:{r}"),
        None => format!("{signal}:{range}:{date}"),
    }
}

// ── 조회(DB) ──────────────────────────────────────────────────────

/// 종료된 작업 한 건 — S1·S2가 함께 쓰는 원재료.
#[derive(Debug, Clone, FromRow, PartialEq)]
struct ClosedTask {
    id: i64,
    repo: String,
    /// `state <> 'Done'` — "머지 없이 닫힘". `Failed`도 포함한다: 설계 §3.3의 "Discarded는
    /// 실패가 아니다"는 DB 상태 이름과 무관하게 **이 카드가 재는 것은 '머지했는가'뿐**이라는
    /// 뜻이다. 문장도 "실패"가 아니라 "머지 없이 닫음"이라 쓴다.
    unmerged: bool,
    /// 이 작업의 `convo_events` 중 `kind='user'` 총 건수(구간과 무관 — 작업 전체 생애).
    user_inputs: i64,
}

/// "이 구간에 닫혔다"의 기준 시각으로 `updated_at`을 쓴다. `created_at`을 쓰면 "이전 구간에
/// 시작해 이번 구간에 닫힌" 작업이 시작 구간으로 새어 들어가 카드의 질문("이번 구간에 무엇이
/// 달라졌나")과 어긋난다. `patterns::load_durations`도 종료 작업의 소요를 잴 때 같은 컬럼을
/// 종료 시각으로 이미 쓰고 있다 — 이 테이블에 별도 `closed_at`이 없는 한 가장 신뢰할 수 있는
/// 근사다.
async fn load_closed_tasks(
    pool: &SqlitePool,
    start: i64,
    end: i64,
) -> anyhow::Result<Vec<ClosedTask>> {
    sqlx::query_as(
        "SELECT id, repo, (state <> 'Done') AS unmerged, \
                (SELECT COUNT(*) FROM convo_events c WHERE c.task_id = tasks.id \
                    AND c.rewound_at IS NULL AND json_valid(c.event) \
                    AND json_extract(c.event, '$.kind') = 'user') AS user_inputs \
           FROM tasks \
          WHERE state IN ('Done', 'Failed', 'Discarded') \
            AND updated_at >= ? AND updated_at < ?",
    )
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

// ── 순수 판정 · 렌더링 ────────────────────────────────────────────

struct Candidate {
    card: InsightCard,
    /// 정렬 전용 크기 지표. 신호마다 정의가 다르다(아래 각 build_* 참고).
    magnitude: f64,
}

/// 분모·분자 쌍 — "이 필터를 만족하는 작업 중 몇 건이 머지 없이 닫혔나".
#[derive(Debug, Default, Clone, PartialEq)]
struct DiscardMetrics {
    sample: i64,
    unmerged: i64,
    ids: Vec<i64>,
}

/// S1 분모 — 입력 1회 이하로 닫힌 작업.
fn s1_metrics(rows: &[ClosedTask]) -> DiscardMetrics {
    let mut m = DiscardMetrics::default();
    for r in rows {
        if r.user_inputs > 1 {
            continue;
        }
        m.sample += 1;
        if r.unmerged {
            m.unmerged += 1;
            m.ids.push(r.id);
        }
    }
    m
}

/// S1 노출 규칙 — 전역 표본 하한 + 비율 하한 + 직전 구간 대비 증가(§5.2 S1 지시).
fn s1_should_show(current: &DiscardMetrics, baseline_unmerged: i64) -> bool {
    if current.sample < MIN_SAMPLE {
        return false;
    }
    let rate = current.unmerged as f64 / current.sample as f64;
    if rate < S1_MIN_RATE {
        return false;
    }
    if baseline_unmerged == 0 {
        current.unmerged >= S1_ZERO_BASELINE_MIN
    } else {
        current.unmerged as f64 >= baseline_unmerged as f64 * S1_MIN_RATIO
            && current.unmerged - baseline_unmerged >= S1_MIN_ABS_INCREASE
    }
}

/// 정렬용 크기 — 직전 구간이 있으면 배수, 0건이면(배수를 정의할 수 없다) 건수 자체.
fn s1_magnitude(current_unmerged: i64, baseline_unmerged: i64) -> f64 {
    if baseline_unmerged > 0 {
        current_unmerged as f64 / baseline_unmerged as f64
    } else {
        current_unmerged as f64
    }
}

fn s1_sentence(current: &DiscardMetrics, baseline_unmerged: i64) -> String {
    let rate_pct = (current.unmerged as f64 / current.sample as f64 * 100.0).round() as i64;
    format!(
        "입력 1회 이하로 머지 없이 닫은 작업이 {}건({rate_pct}%)이다. 직전 구간 {}건",
        current.unmerged, baseline_unmerged
    )
}

fn build_s1(
    current_rows: &[ClosedTask],
    baseline_rows: &[ClosedTask],
    range: &str,
    window_start: i64,
    tz_offset_secs: i64,
) -> Option<Candidate> {
    let current = s1_metrics(current_rows);
    let baseline = s1_metrics(baseline_rows);
    if !s1_should_show(&current, baseline.unmerged) {
        return None;
    }
    let key = window_key("S1", range, window_start, tz_offset_secs, None);
    let magnitude = s1_magnitude(current.unmerged, baseline.unmerged);
    let card = InsightCard {
        key,
        signal: SignalId::S1,
        sentence: s1_sentence(&current, baseline.unmerged),
        value: current.unmerged as f64,
        baseline: Some(baseline.unmerged as f64),
        sample: current.sample,
        evidence: Evidence::Tasks { ids: current.ids },
        repo: None,
    };
    Some(Candidate { card, magnitude })
}

/// 저장소 경로를 짧게 — 홈 디렉터리를 `~`로 접는다. 이 이상은 하지 않는다: 사용자가
/// 여러 저장소를 겹쳐 쓰는 폭이 넓지 않고, `insights::display_names`(프로젝트 충돌 분해)를
/// 그대로 끌어오면 이 함수 하나를 위해 전체 저장소 목록이 필요해진다.
fn short_repo(path: &str) -> String {
    short_repo_with_home(path, crate::usage::home_dir().as_deref())
}

fn short_repo_with_home(path: &str, home: Option<&std::path::Path>) -> String {
    if let Some(home) = home {
        if let Ok(rest) = std::path::Path::new(path).strip_prefix(home) {
            let rest = rest.display().to_string();
            return if rest.is_empty() {
                "~".to_string()
            } else {
                format!("~/{rest}")
            };
        }
    }
    path.to_string()
}

/// S2 노출 규칙 — 저장소 표본 하한 + 전체 평균 대비 배수 + 전체 평균 대비 차이(§5.2 S2 지시).
fn s2_should_show(repo_sample: i64, repo_rate: f64, overall_rate: f64) -> bool {
    repo_sample >= S2_MIN_SAMPLE
        && repo_rate >= overall_rate * S2_MIN_RATE_RATIO
        && repo_rate - overall_rate >= S2_MIN_RATE_DIFF
}

/// 정렬용 크기 — 전체 평균 대비 배수. 평균이 0이면(전체가 전부 머지됨) 배수를 정의할 수
/// 없으니 저장소 비율 자체를 쓴다.
fn s2_magnitude(repo_rate: f64, overall_rate: f64) -> f64 {
    if overall_rate > 0.0 {
        repo_rate / overall_rate
    } else {
        repo_rate
    }
}

/// S2 — 저장소별 폐기 집중. **`baseline`은 직전 구간이 아니라 "이번 구간의 전체 평균"이다**
/// (§5.2 지시 — 다른 신호와 의미가 다르니 프런트에서 그대로 "직전 구간"이라 읽지 않게 한다).
fn build_s2(
    rows: &[ClosedTask],
    range: &str,
    window_start: i64,
    tz_offset_secs: i64,
) -> Vec<Candidate> {
    let total_sample = rows.len() as i64;
    if total_sample == 0 {
        return Vec::new();
    }
    let total_unmerged = rows.iter().filter(|r| r.unmerged).count() as i64;
    let overall_rate = total_unmerged as f64 / total_sample as f64;

    let mut by_repo: BTreeMap<&str, DiscardMetrics> = BTreeMap::new();
    for r in rows {
        let m = by_repo.entry(r.repo.as_str()).or_default();
        m.sample += 1;
        if r.unmerged {
            m.unmerged += 1;
            m.ids.push(r.id);
        }
    }

    let mut candidates = Vec::new();
    for (repo, m) in by_repo {
        let repo_rate = m.unmerged as f64 / m.sample as f64;
        if !s2_should_show(m.sample, repo_rate, overall_rate) {
            continue;
        }
        let key = window_key("S2", range, window_start, tz_offset_secs, Some(repo));
        let avg_pct = (overall_rate * 100.0).round() as i64;
        let sentence = format!(
            "`{}`은 {}건 중 {}건을 머지 없이 닫았다(평균 {avg_pct}%)",
            short_repo(repo),
            m.sample,
            m.unmerged
        );
        let magnitude = s2_magnitude(repo_rate, overall_rate);
        candidates.push(Candidate {
            card: InsightCard {
                key,
                signal: SignalId::S2,
                sentence,
                value: m.unmerged as f64,
                baseline: Some(overall_rate),
                sample: m.sample,
                evidence: Evidence::Tasks { ids: m.ids.clone() },
                repo: Some(repo.to_string()),
            },
            magnitude,
        });
    }
    candidates
}

/// S6 — 토큰 사용량 급증(달러가 아니다, `SpendSnapshot` 문서 참고). 순수 함수라 DB 없이
/// 테스트한다.
fn build_s6(
    spend: &SpendSnapshot,
    range: &str,
    window_start: i64,
    tz_offset_secs: i64,
    known_repos: &[String],
) -> Option<Candidate> {
    let prev_tokens = spend.prev_tokens?;
    if prev_tokens == 0 || spend.current_tokens == 0 {
        return None;
    }
    if spend.sample < MIN_SAMPLE {
        return None;
    }
    let ratio = spend.current_tokens as f64 / prev_tokens as f64;
    if ratio < S6_MIN_RATIO {
        return None;
    }
    let mut sentence = format!("이번 구간 토큰 사용량이 직전 구간의 {ratio:.1}배다.");
    let mut project = None;
    let mut repo = None;
    if let Some((name, path, tokens)) = &spend.top_project {
        if spend.current_tokens > 0 {
            let pct = (*tokens as f64 / spend.current_tokens as f64 * 100.0).round() as i64;
            // 설계 §5.2 예시는 "증가분의 {p}%"지만, 백엔드에는 프로젝트별 직전 구간
            // 분해가 없어(`PeriodSummary`가 총합만 낸다) 진짜 증가분을 계산할 수 없다.
            // 대신 "이번 구간 비중"을 쓴다 — 없는 수치를 있는 것처럼 문장에 넣지 않는다.
            sentence.push_str(&format!(" 이 중 {pct}%가 `{name}`에서 발생했다."));
            project = Some(name.clone());
            // 표시 이름(name)이 아니라 실제 경로(path)가 작업 저장소 목록에 있을 때만
            // 이 카드를 "저장소에 묶인 것"으로 본다(설계 §8 O2) — 무관한 경로를
            // 메모리 저장 대상으로 내밀지 않는다.
            if known_repos.iter().any(|r| r == path) {
                repo = Some(path.clone());
            }
        }
    }
    let key = window_key("S6", range, window_start, tz_offset_secs, None);
    Some(Candidate {
        card: InsightCard {
            key,
            signal: SignalId::S6,
            sentence,
            value: spend.current_tokens as f64,
            baseline: Some(prev_tokens as f64),
            sample: spend.sample,
            evidence: Evidence::Spend { project },
            repo,
        },
        magnitude: ratio,
    })
}

/// L1 문장의 구간 표현 — "최근 7일"·"최근 30일"·"all"은 "전체 구간"(§9 T3, "L1은 'all'에서도
/// 작동한다").
fn window_desc(range: &str) -> &'static str {
    match range {
        "7d" => "최근 7일",
        "30d" => "최근 30일",
        _ => "전체 구간",
    }
}

/// L1 — 교훈 주제 반복(§5.2, §9 T3). 표본은 "이 구간에 교훈이 하나 이상 있는 항목" 수이고,
/// 분자는 그중 가장 많이 걸린 subject의 항목 수다. 변화가 아니라 반복을 재는 신호라 baseline이
/// 없어도(= "all") 카드를 낸다 — `baseline` 필드만 None이 된다.
///
/// 저장소당 최대 한 장(최다 subject 하나만 본다) — 나머지 신호와 같은 `finalize` 정렬·5장
/// 상한에 그대로 들어간다.
fn build_l1(
    projections: &[RepoProjection],
    range: &str,
    current: Window,
    baseline: Option<Window>,
    tz_offset_secs: i64,
) -> Vec<Candidate> {
    let cw = super::lessons::current_date_window(current, tz_offset_secs);
    let bw = baseline.map(|b| super::lessons::baseline_date_window(b, tz_offset_secs));

    let mut out = Vec::new();
    for proj in projections {
        let with_lessons: Vec<&LedgerEntry> = proj
            .entries
            .iter()
            .filter(|e| cw.contains(&e.date) && !e.lessons.is_empty())
            .collect();
        let sample = with_lessons.len() as i64;
        if sample < MIN_SAMPLE {
            continue;
        }

        // subject별 항목 번호 — 한 항목이 같은 subject를 두 번 적어도 한 번만 센다.
        let mut by_subject: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
        for e in &with_lessons {
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            for s in &e.subjects {
                if seen.insert(s.as_str()) {
                    by_subject.entry(s.as_str()).or_default().push(e.number);
                }
            }
        }
        // 최댓값 우선, 동률이면 나중에 오는(이름이 늦은) subject가 이긴다 — BTreeMap 순회가
        // 이름 오름차순이므로 `>` 대신 `>=`로 갱신하면 그 규칙이 된다. 어느 쪽이든 §9 지시는
        // "top subject"만 요구할 뿐 동률 규칙을 정하지 않아, 결정론만 확보하면 된다.
        let top = by_subject
            .iter()
            .fold(None::<(&str, usize)>, |acc, (subject, ids)| match acc {
                Some((_, best)) if best > ids.len() => acc,
                _ => Some((subject, ids.len())),
            });
        let Some((subject, top_count)) = top else {
            continue;
        };
        let top_count = top_count as i64;
        if top_count < L1_MIN_SUBJECT_COUNT {
            continue;
        }
        let share = top_count as f64 / sample as f64;
        if share < L1_MIN_SHARE {
            continue;
        }

        let baseline_share = bw.as_ref().and_then(|bw| {
            let baseline_with_lessons: Vec<&LedgerEntry> = proj
                .entries
                .iter()
                .filter(|e| bw.contains(&e.date) && !e.lessons.is_empty())
                .collect();
            let baseline_sample = baseline_with_lessons.len() as i64;
            if baseline_sample < MIN_SAMPLE {
                return None;
            }
            let baseline_count = baseline_with_lessons
                .iter()
                .filter(|e| e.subjects.iter().any(|s| s == subject))
                .count() as f64;
            Some(baseline_count / baseline_sample as f64)
        });

        let numbers = by_subject.get(subject).cloned().unwrap_or_default();
        let key = window_key("L1", range, current.start, tz_offset_secs, Some(&proj.repo));
        let sentence = format!(
            "`{}` {} 교훈 {sample}건 중 {top_count}건이 `{subject}`이다",
            short_repo(&proj.repo),
            window_desc(range),
        );
        out.push(Candidate {
            card: InsightCard {
                key,
                signal: SignalId::L1,
                sentence,
                value: top_count as f64,
                baseline: baseline_share,
                sample,
                evidence: Evidence::Ledger {
                    repo: proj.repo.clone(),
                    numbers,
                },
                repo: Some(proj.repo.clone()),
            },
            magnitude: share,
        });
    }
    out
}

// ── 정렬 · 상한 ───────────────────────────────────────────────────

fn signal_rank(id: SignalId) -> u8 {
    match id {
        SignalId::S1 => 1,
        SignalId::S2 => 2,
        SignalId::S6 => 6,
        SignalId::L1 => 7,
    }
}

fn cmp_candidates(a: &Candidate, b: &Candidate) -> std::cmp::Ordering {
    b.magnitude
        .partial_cmp(&a.magnitude)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| signal_rank(a.card.signal).cmp(&signal_rank(b.card.signal)))
}

/// S2를 먼저 2장으로 자르고(§5.2), 그다음 전체를 크기순으로 합쳐 5장으로 자른다(§5.1).
/// 순서를 반대로 하면(전체 5장부터) S2가 자리를 셋 이상 차지할 수 있어 설계 지시와 어긋난다.
/// L1은 저장소당 이미 한 장뿐이라(`build_l1`) 별도 상한이 없다 — 전체 상한만 함께 적용한다.
fn finalize(
    s1: Option<Candidate>,
    mut s2: Vec<Candidate>,
    s6: Option<Candidate>,
    l1: Vec<Candidate>,
) -> Vec<InsightCard> {
    s2.sort_by(cmp_candidates);
    s2.truncate(S2_MAX_CARDS);

    let mut all: Vec<Candidate> = s1.into_iter().chain(s2).chain(s6).chain(l1).collect();
    all.sort_by(cmp_candidates);
    all.truncate(MAX_CARDS);
    all.into_iter().map(|c| c.card).collect()
}

#[cfg(test)]
mod tests;
