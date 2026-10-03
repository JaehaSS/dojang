//! 사용량 인사이트 — 로컬 Claude 프로젝트와 Codex 세션의 관측 가능한 사용량을 집계.
//! Tauri 비의존(단위 테스트 가능). 여러 모델을 사용하는 환경을 가정해 **모델별 분해**를 함께 제공.
//!
//! 시간 버킷(날짜/시간/요일)은 로컬 시간대 기준. 외부 의존성(chrono 등) 없이 ISO8601 타임스탬프를
//! 직접 파싱한다. 프로젝트별 분해는 메시지의 `cwd` 필드를 키로 삼는다.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use serde::Serialize;

mod agent_skills;
mod cards;
pub mod dismissals;
mod lessons;
mod outcomes;
mod patterns;
pub use agent_skills::{
    compute_agent_skills, AgentSkillUsage, AgentSlice, AgentStat, SkillSlice, SkillStat,
};
pub use cards::{compute_cards, Evidence, InsightCard, SignalId, SpendSnapshot};
pub use dismissals::{dismiss as dismiss_card, migrate as migrate_dismissals};
pub use lessons::{
    compute_lesson_themes, load_projections as load_lesson_projections, AbandonedItem,
    LedgerEntry, RepoLessons, RepoProjection, SubjectCount,
};
pub use outcomes::{compute_outcomes, OutcomeInsights};
pub use patterns::{compute_patterns, TaskPatterns};

/// 모델 한 종에 대한 사용량 집계.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ModelStat {
    pub model: String,
    /// 모델 제공자 — 로컬 CLI 기록에서 정규화한 "claude" | "codex".
    pub provider: String,
    pub messages: u64,
    pub sessions: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// 캐시 쓰기(생성) 토큰 — 비용 계산 시 입력가×1.25.
    pub cache_creation_tokens: u64,
    /// 캐시 읽기 토큰 — 비용 계산 시 입력가×0.1.
    pub cache_read_tokens: u64,
    /// 캐시 비율의 분모·분자를 모두 확인한 사용량 표본 수.
    pub cache_observed_messages: u64,
    pub cache_observed_input_tokens: u64,
    pub cache_observed_read_tokens: u64,
    /// 사용량은 있었지만 캐시 비율을 계산할 수 없던 표본 수.
    pub cache_unknown_messages: u64,
    pub total_tokens: u64,
    /// 24칸 — 이 모델의 시간대별 메시지 수.
    pub hours: Vec<u64>,
}

/// 하루치 활동량(히트맵용).
#[derive(Debug, Clone, Serialize)]
pub struct DayStat {
    /// YYYY-MM-DD (로컬)
    pub date: String,
    pub messages: u64,
    pub tokens: u64,
}

/// 일별 캐시 관측치. `cache_read_tokens / input_tokens`는 observed 표본만의 비율이다.
#[derive(Debug, Clone, Serialize, Default)]
pub struct CacheDayStat {
    pub date: String,
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub observed_messages: u64,
    pub unknown_messages: u64,
}

/// 프로젝트(cwd) 한 곳의 사용량 집계.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ProjectStat {
    /// cwd 원본 경로.
    pub path: String,
    /// 표시명 — 마지막 세그먼트. 다른 프로젝트와 충돌하면 상위 세그먼트를 붙여 구분한다.
    pub name: String,
    pub sessions: u64,
    pub messages: u64,
    pub total_tokens: u64,
    /// 마지막 활동일 YYYY-MM-DD.
    pub last_active: String,
}

/// 직전 동일 길이 구간 요약 — 증감(Δ) 표시용.
#[derive(Debug, Clone, Serialize, Default)]
pub struct PeriodSummary {
    pub sessions: u64,
    pub messages: u64,
    pub total_tokens: u64,
    pub active_days: u64,
}

/// 인사이트 전체 — 개요 지표 + 히트맵(days) + 시간대(hours) + 모델별/프로젝트별 분해.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Insights {
    pub sessions: u64,
    pub messages: u64,
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// 캐시 생성 토큰 전체 합계.
    pub cache_creation_tokens: u64,
    /// 캐시 읽기 토큰 전체 합계.
    pub cache_read_tokens: u64,
    pub cache_observed_messages: u64,
    pub cache_observed_input_tokens: u64,
    pub cache_observed_read_tokens: u64,
    pub cache_unknown_messages: u64,
    /// 날짜 오름차순. 전체 로컬 CLI 기록의 캐시 관측치이며 Praxis 전용 측정이 아니다.
    pub cache_days: Vec<CacheDayStat>,
    /// Praxis 주 대화 접수 입력 측정. 과거 행이나 명령 경로는 None으로 남는다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_injection: Option<crate::convo::prompt_metrics::PromptInjectionSummary>,
    pub active_days: u64,
    pub current_streak: u64,
    pub longest_streak: u64,
    /// 메시지가 가장 많은 시간대(0~23, 로컬). 데이터 없으면 None.
    pub peak_hour: Option<u32>,
    /// 메시지 기준 가장 많이 쓴 모델.
    pub favorite_model: Option<String>,
    /// 날짜 오름차순 — 히트맵.
    pub days: Vec<DayStat>,
    /// 24칸 — 시간대별 메시지 수.
    pub hours: Vec<u64>,
    /// 168칸 — 요일(일=0) × 시간. 인덱스 = weekday * 24 + hour.
    pub weekday_hours: Vec<u64>,
    /// 메시지 많은 순 — 모델별 분해.
    pub models: Vec<ModelStat>,
    /// 토큰 많은 순 — 프로젝트별 분해.
    pub projects: Vec<ProjectStat>,
    /// 직전 동일 길이 구간. range="all"이면 비교 대상이 없어 None.
    pub prev: Option<PeriodSummary>,
}

/// 1970-01-01부터의 일수 (Howard Hinnant days_from_civil).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// days_from_civil의 역 — 일수 → (year, month, day).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 로컬 day 번호 → "YYYY-MM-DD".
fn date_string(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// day 번호 → 요일(일=0 … 토=6). 1970-01-01(day 0)은 목요일이므로 +4.
fn weekday(day: i64) -> usize {
    (day + 4).rem_euclid(7) as usize
}

/// 로컬 시간대로 환산된 타임스탬프 — day_num / hour / "YYYY-MM-DD" + 원본 UTC epoch.
struct Ts {
    /// UTC epoch(초) — cutoff 비교용(시간대 무관).
    epoch: i64,
    day: i64,
    hour: u32,
    date: String,
}

/// 메시지 한 건에서 뽑은 집계 입력.
struct Msg<'a> {
    session: &'a str,
    cwd: &'a str,
    model: Option<&'a str>,
    provider: &'a str,
    input: u64,
    output: u64,
    cache_c: u64,
    cache_r: u64,
    cache_observed: bool,
    cache_unknown: bool,
}

impl Msg<'_> {
    fn input_total(&self) -> u64 {
        if self.provider == "codex" { self.input } else { self.input + self.cache_c + self.cache_r }
    }

    fn total(&self) -> u64 {
        // Claude reports cache reads/writes outside input_tokens. Codex's cached input is already
        // a subset of total input, so adding it here would inflate both totals and model shares.
        if self.provider == "codex" {
            self.input + self.output
        } else {
            self.input + self.output + self.cache_c + self.cache_r
        }
    }
}

/// "2026-06-30T01:03:44.931Z" → UTC epoch(초). 형식이 어긋나면 None(베스트 에포트).
/// 시간대는 여기서 다루지 않는다 — 캐시에 담기는 값이라 호출 시점의 오프셋과 무관해야 한다.
fn parse_epoch(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<i64> { s.get(a..z)?.parse().ok() };
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, se) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se)
}

/// 타임스탬프 문자열 → 로컬 버킷. 캐시를 쓰지 않는 `agent_skills`가 이 짝을 그대로 쓴다.
fn parse_ts(s: &str, offset_secs: i64) -> Option<Ts> {
    parse_epoch(s).map(|e| ts_from_epoch(e, offset_secs))
}

/// UTC epoch → 로컬 버킷(offset_secs = 로컬 UTC 오프셋, KST=+32400).
fn ts_from_epoch(epoch: i64, offset_secs: i64) -> Ts {
    let local = epoch + offset_secs;
    let day = local.div_euclid(86400);
    Ts {
        epoch,
        day,
        hour: (local.rem_euclid(86400) / 3600) as u32,
        date: date_string(day),
    }
}

/// range 문자열("7d"|"30d"|"all"…) → 구간 길이(초). all/미인식이면 None.
fn range_span(range: &str) -> Option<i64> {
    match range {
        "7d" => Some(7 * 86400),
        "30d" => Some(30 * 86400),
        _ => None,
    }
}

/// range별 cutoff epoch(이상만 포함). all/미인식이면 0.
fn cutoff(range: &str, now: i64) -> i64 {
    range_span(range).map(|span| now - span).unwrap_or(0)
}

/// `~/.claude/projects` 하위 모든 *.jsonl 경로. 세션 하위의 서브에이전트 트랜스크립트는
/// 대상이 아니다 — 사용량 개요는 메인 세션 기준이고, 서브에이전트는 `agent_skills`가 따로 센다.
fn transcript_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(projects) = std::fs::read_dir(root) else {
        return out;
    };
    for proj in projects.flatten() {
        let p = proj.path();
        if !p.is_dir() {
            continue;
        }
        if let Ok(files) = std::fs::read_dir(&p) {
            for f in files.flatten() {
                let fp = f.path();
                if fp.extension().map(|e| e == "jsonl").unwrap_or(false) {
                    out.push(fp);
                }
            }
        }
    }
    out
}

/// 프로젝트 한 곳의 누적 상태.
#[derive(Default)]
struct ProjectAcc {
    messages: u64,
    total_tokens: u64,
    sessions: HashSet<String>,
    /// 마지막 활동 day 번호.
    last_day: i64,
}

/// 누적기 — 파일 스캔 중 상태. 테스트에서 직접 호출 가능하도록 분리.
#[derive(Default)]
struct Acc {
    messages: u64,
    total_tokens: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
    cache_observed_messages: u64,
    cache_observed_input_tokens: u64,
    cache_observed_read_tokens: u64,
    cache_unknown_messages: u64,
    cache_days: BTreeMap<i64, CacheDayStat>,
    sessions: HashSet<String>,
    day_msgs: BTreeMap<i64, (String, u64, u64)>, // day_num → (date, messages, tokens)
    active_days: BTreeSet<i64>,
    hours: [u64; 24],
    /// [요일(일=0)][시간] — 고정 크기 168 배열은 Default가 없어 2차원으로 둔다.
    weekday_hours: [[u64; 24]; 7],
    models: HashMap<String, ModelStat>,
    model_sessions: HashMap<String, HashSet<String>>,
    projects: HashMap<String, ProjectAcc>,
    /// cwd 원본 → 저장소 루트. compute 한 번 동안만 유효(파일시스템 상태가 바뀔 수 있다).
    repo_roots: HashMap<String, String>,
}

/// 직전 구간 누적기 — 요약 4지표만 필요하므로 경량.
#[derive(Default)]
struct PrevAcc {
    messages: u64,
    total_tokens: u64,
    sessions: HashSet<String>,
    active_days: BTreeSet<i64>,
}

impl PrevAcc {
    fn add(&mut self, ts: &Ts, m: &Msg) {
        self.messages += 1;
        self.total_tokens += m.total();
        if !m.session.is_empty() {
            self.sessions.insert(m.session.to_string());
        }
        self.active_days.insert(ts.day);
    }

    fn finish(self) -> PeriodSummary {
        PeriodSummary {
            sessions: self.sessions.len() as u64,
            messages: self.messages,
            total_tokens: self.total_tokens,
            active_days: self.active_days.len() as u64,
        }
    }
}

impl Acc {
    /// 메시지 한 건 반영.
    fn add(&mut self, ts: &Ts, m: &Msg) {
        let total = m.total();
        self.messages += 1;
        self.input_tokens += m.input;
        self.output_tokens += m.output;
        self.cache_creation_tokens += m.cache_c;
        self.cache_read_tokens += m.cache_r;
        if m.cache_observed {
            self.cache_observed_messages += 1;
            self.cache_observed_input_tokens += m.input_total();
            self.cache_observed_read_tokens += m.cache_r;
            let day = self
                .cache_days
                .entry(ts.day)
                .or_insert_with(|| CacheDayStat {
                    date: ts.date.clone(),
                    ..Default::default()
                });
            day.input_tokens += m.input_total();
            day.cache_read_tokens += m.cache_r;
            day.cache_write_tokens += m.cache_c;
            day.observed_messages += 1;
        } else if m.cache_unknown {
            self.cache_unknown_messages += 1;
            let day = self
                .cache_days
                .entry(ts.day)
                .or_insert_with(|| CacheDayStat {
                    date: ts.date.clone(),
                    ..Default::default()
                });
            day.unknown_messages += 1;
        }
        self.total_tokens += total;
        if !m.session.is_empty() {
            self.sessions.insert(m.session.to_string());
        }
        self.active_days.insert(ts.day);
        if (ts.hour as usize) < 24 {
            self.hours[ts.hour as usize] += 1;
            self.weekday_hours[weekday(ts.day)][ts.hour as usize] += 1;
        }
        let e = self
            .day_msgs
            .entry(ts.day)
            .or_insert_with(|| (ts.date.clone(), 0, 0));
        e.1 += 1;
        e.2 += total;
        if !m.cwd.is_empty() {
            let root = memo_repo_root(&mut self.repo_roots, m.cwd);
            let p = self.projects.entry(root).or_default();
            p.messages += 1;
            p.total_tokens += total;
            p.last_day = p.last_day.max(ts.day);
            if !m.session.is_empty() {
                p.sessions.insert(m.session.to_string());
            }
        }
        if let Some(name) = m.model {
            let key = format!("{}\u{1f}{name}", m.provider);
            let ms = self.models.entry(key.clone()).or_insert_with(|| ModelStat {
                model: name.to_string(),
                provider: m.provider.to_string(),
                hours: vec![0; 24],
                ..Default::default()
            });
            ms.messages += 1;
            ms.input_tokens += m.input;
            ms.output_tokens += m.output;
            ms.cache_creation_tokens += m.cache_c;
            ms.cache_read_tokens += m.cache_r;
            ms.cache_observed_messages += u64::from(m.cache_observed);
            if m.cache_observed {
                ms.cache_observed_input_tokens += m.input_total();
                ms.cache_observed_read_tokens += m.cache_r;
            }
            ms.cache_unknown_messages += u64::from(m.cache_unknown);
            ms.total_tokens += total;
            if (ts.hour as usize) < 24 {
                ms.hours[ts.hour as usize] += 1;
            }
            if !m.session.is_empty() {
                self.model_sessions
                    .entry(key)
                    .or_default()
                    .insert(m.session.to_string());
            }
        }
    }
}

/// 경로를 구분자(`/`, `\`)로 쪼갠 세그먼트. 빈 조각은 버린다.
fn split_segments(path: &str) -> Vec<&str> {
    path.split(['/', '\\']).filter(|s| !s.is_empty()).collect()
}

/// `<repo>/.praxis/worktrees/<name>/…` → `<repo>`. 그 패턴이 없으면 원본 그대로.
fn strip_worktree_suffix(cwd: &str) -> &str {
    let bytes = cwd.as_bytes();
    let mut segs: Vec<(usize, usize)> = Vec::new();
    let mut start = 0usize;
    for i in 0..=bytes.len() {
        if i == bytes.len() || bytes[i] == b'/' || bytes[i] == b'\\' {
            if i > start {
                segs.push((start, i));
            }
            start = i + 1;
        }
    }
    for w in segs.windows(2) {
        if &cwd[w[0].0..w[0].1] != ".praxis" || &cwd[w[1].0..w[1].1] != "worktrees" {
            continue;
        }
        let head = &cwd[..w[0].0];
        let trimmed = head.trim_end_matches(['/', '\\']);
        return if trimmed.is_empty() { head } else { trimmed };
    }
    cwd
}

/// cwd → 그 작업이 속한 저장소 루트. worktree 경로를 원본 체크아웃으로 접고,
/// 거기서 조상으로 올라가며 첫 저장소 루트를 고른다. 못 찾으면 접기만 한 경로.
pub fn normalize_repo_root(cwd: &str, is_repo_root: &dyn Fn(&Path) -> bool) -> String {
    if cwd.is_empty() {
        return String::new();
    }
    let base = strip_worktree_suffix(cwd);
    for anc in Path::new(base).ancestors() {
        if is_repo_root(anc) {
            return anc.to_string_lossy().into_owned();
        }
    }
    base.to_string()
}

/// 같은 cwd를 반복해 stat하지 않도록 메모를 낀 `normalize_repo_root`.
fn memo_repo_root(memo: &mut HashMap<String, String>, cwd: &str) -> String {
    if let Some(hit) = memo.get(cwd) {
        return hit.clone();
    }
    let root = normalize_repo_root(cwd, &|p| p.join(".git").exists());
    memo.insert(cwd.to_string(), root.clone());
    root
}

/// 뒤에서 depth개 세그먼트를 `/`로 이어붙인 표시명.
fn tail_name(segs: &[&str], depth: usize) -> String {
    let start = segs.len().saturating_sub(depth.max(1));
    segs[start..].join("/")
}

/// 경로 목록 → 표시명 목록. 마지막 세그먼트로 시작해, 충돌한 항목만
/// 더 붙일 세그먼트가 남아 있는 한 상위 세그먼트를 한 단계씩 앞에 붙인다.
fn display_names(paths: &[String]) -> Vec<String> {
    let segs: Vec<Vec<&str>> = paths.iter().map(|p| split_segments(p)).collect();
    let mut depth = vec![1usize; paths.len()];
    loop {
        let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, s) in segs.iter().enumerate() {
            groups.entry(tail_name(s, depth[i])).or_default().push(i);
        }
        let mut changed = false;
        for idxs in groups.values() {
            if idxs.len() < 2 {
                continue;
            }
            for &i in idxs {
                if depth[i] < segs[i].len() {
                    depth[i] += 1;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    segs.iter()
        .enumerate()
        .map(|(i, s)| tail_name(s, depth[i]))
        .collect()
}

/// 파일 한 줄에서 뽑은 원시 레코드 — cutoff·시간대와 무관해 파일 단위로 캐시할 수 있다.
#[derive(Clone)]
struct Rec {
    /// UTC epoch(초).
    epoch: i64,
    session: String,
    /// 원본 cwd. 저장소 루트 해석은 파일시스템 상태에 달려 있어 캐시에 담지 않는다.
    cwd: String,
    model: Option<String>,
    provider: String,
    input: u64,
    output: u64,
    cache_c: u64,
    cache_r: u64,
    /// 사용량 줄이었지만 캐시 비율에 필요한 필드가 빠졌음을 구분한다.
    cache_observed: bool,
    cache_unknown: bool,
    /// Claude 재기록을 합칠 때 사용하는 안정 메시지 ID. request id는 합치지 않는다.
    stable_id: Option<String>,
}

/// 파일 한 개의 파싱 결과 + 그때의 mtime·size. 둘 중 하나라도 다르면 다시 읽는다.
struct CachedFile {
    mtime: Option<SystemTime>,
    size: u64,
    recs: Vec<Rec>,
}

static CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedFile>>> = OnceLock::new();

/// 파일을 실제로 읽은 횟수 — 캐시가 스캔을 건너뛰는지 테스트가 확인한다.
static FILE_READS: AtomicUsize = AtomicUsize::new(0);

/// 지금까지 읽은 파일 수 — 테스트 전용 관측점.
pub fn file_read_count() -> usize {
    FILE_READS.load(Ordering::Relaxed)
}

/// range별 인사이트 집계. offset_secs = 로컬 UTC 오프셋(KST=32400). 베스트 에포트.
pub fn compute(range: &str, offset_secs: i64) -> Insights {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    compute_with_roots(
        &crate::sessionhome::projects_root().unwrap_or_default(),
        crate::sessionhome::codex_sessions_root().as_deref(),
        range,
        offset_secs,
        now,
    )
}

/// 트랜스크립트 루트와 현재 시각을 주입받는 본체 — 테스트가 가짜 루트로 부른다.
#[cfg(test)]
fn compute_in(root: &Path, range: &str, offset_secs: i64, now: i64) -> Insights {
    compute_with_roots(root, None, range, offset_secs, now)
}

/// Claude 프로젝트와 Codex rollout roots를 함께 집계한다. 테스트는 실제 사용자 세션을
/// 섞지 않도록 roots를 주입한다.
fn compute_with_roots(
    root: &Path,
    codex_root: Option<&Path>,
    range: &str,
    offset_secs: i64,
    now: i64,
) -> Insights {
    let cut = cutoff(range, now);
    // 직전 구간 = 현재 구간과 같은 길이로 그 앞. all이면 비교 대상 없음.
    let prev_cut = range_span(range).map(|span| cut - span).unwrap_or(0);
    let mut prev = range_span(range).map(|_| PrevAcc::default());
    let mut acc = Acc::default();
    let files = transcript_files(root);
    let codex_files = codex_root.map(codex_rollout_files).unwrap_or_default();
    let cell = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cell.lock().unwrap_or_else(|e| e.into_inner());
    let live: HashSet<&PathBuf> = files.iter().chain(&codex_files).collect();
    cache.retain(|path, _| live.contains(path));
    for path in &files {
        let recs = cached_recs(&mut cache, path, read_claude_file);
        apply_records(&mut acc, &mut prev, recs, cut, prev_cut, offset_secs);
    }
    // Codex usage is cumulative per rollout. Keep only small numeric records; the scanner
    // never retains message text, tool payloads, or the rest of the JSON object.
    // 레코드는 파일 전체의 누적값에서 나오고 cutoff와 무관하므로 Claude와 같은 캐시에 담는다 —
    // 없으면 range 칩을 바꿀 때마다 rollout 전체(GB 단위)를 다시 파싱한다.
    let mut seen = HashSet::new();
    for path in &codex_files {
        let recs: Vec<Rec> = cached_recs(&mut cache, path, read_codex_file)
            .iter()
            .filter(|r| {
                r.stable_id
                    .as_ref()
                    .is_none_or(|id| seen.insert((r.session.clone(), id.clone())))
            })
            .cloned()
            .collect();
        apply_records(&mut acc, &mut prev, &recs, cut, prev_cut, offset_secs);
    }
    drop(cache);
    // 로컬 오늘 = (now + offset)의 day.
    let today = (now + offset_secs).div_euclid(86400);
    finalize(acc, prev, today)
}

/// 캐시가 파일의 현재 mtime·size와 맞으면 그대로, 아니면 읽어서 갱신한 뒤 돌려준다.
fn cached_recs<'a>(
    cache: &'a mut HashMap<PathBuf, CachedFile>,
    path: &Path,
    read: fn(&Path) -> Vec<Rec>,
) -> &'a [Rec] {
    let meta = std::fs::metadata(path).ok();
    let mtime = meta.as_ref().and_then(|m| m.modified().ok());
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let fresh = cache
        .get(path)
        .map(|c| c.mtime == mtime && c.size == size)
        .unwrap_or(false);
    if !fresh {
        FILE_READS.fetch_add(1, Ordering::Relaxed);
        let recs = read(path);
        cache.insert(path.to_path_buf(), CachedFile { mtime, size, recs });
    }
    cache.get(path).map(|c| c.recs.as_slice()).unwrap_or(&[])
}

fn read_claude_file(path: &Path) -> Vec<Rec> {
    std::fs::read_to_string(path)
        .map(|c| parse_records(&c))
        .unwrap_or_default()
}

fn read_codex_file(path: &Path) -> Vec<Rec> {
    std::fs::File::open(path)
        .map(|f| parse_codex_reader(BufReader::new(f)))
        .unwrap_or_default()
}

/// JSONL 본문 → 원시 Claude 레코드. user/assistant 외의 줄과 깨진 줄은 버린다.
fn parse_records(content: &str) -> Vec<Rec> {
    let mut out = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
        if ty != "user" && ty != "assistant" {
            continue;
        }
        let Some(epoch) = v
            .get("timestamp")
            .and_then(|x| x.as_str())
            .and_then(parse_epoch)
        else {
            continue;
        };
        let str_of = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let (mut input, mut output, mut cache_c, mut cache_r) = (0u64, 0u64, 0u64, 0u64);
        let mut model: Option<String> = None;
        let mut cache_observed = false;
        let mut cache_unknown = ty == "assistant";
        if ty == "assistant" {
            model = Some("unknown".into());
            if let Some(m) = v.get("message") {
                model = Some(m.get("model").and_then(|x| x.as_str()).unwrap_or("unknown").to_string());
                if let Some(u) = m.get("usage") {
                    let g = |k: &str| u.get(k).and_then(|x| x.as_u64());
                    input = g("input_tokens").unwrap_or(0);
                    output = g("output_tokens").unwrap_or(0);
                    cache_c = g("cache_creation_input_tokens").unwrap_or(0);
                    cache_r = g("cache_read_input_tokens").unwrap_or(0);
                    // Explicit zero is observed; a missing, negative, or wrong-typed field is not.
                    cache_observed = g("input_tokens").is_some()
                        && g("cache_creation_input_tokens").is_some()
                        && g("cache_read_input_tokens").is_some();
                    cache_unknown = !cache_observed;
                }
            }
        }
        out.push(Rec {
            epoch,
            session: str_of("sessionId"),
            cwd: str_of("cwd"),
            model,
            provider: "claude".to_string(),
            input,
            output,
            cache_c,
            cache_r,
            cache_observed,
            cache_unknown,
            stable_id: v
                .get("message")
                .and_then(|m| m.get("id"))
                .and_then(|x| x.as_str())
                .or_else(|| v.get("uuid").and_then(|x| x.as_str()))
                .map(|id| {
                    // Some transcript writers reuse a message wrapper across provider requests.
                    // Do not collapse those separate requests merely because their wrapper ID is
                    // stable; absent requestId preserves the normal single-message identity.
                    match v.get("requestId").and_then(|x| x.as_str()) {
                        Some(request) => format!("{id}\u{1f}{request}"),
                        None => id.to_string(),
                    }
                }),
        });
    }
    dedup_claude_records(out)
}

/// Rewritten Claude chunks may repeat the same assistant message. The stable message ID is
/// intentionally scoped to a session, while `requestId` remains distinct because providers can
/// legitimately use it for separate requests. Last valid usage snapshot wins.
fn dedup_claude_records(records: Vec<Rec>) -> Vec<Rec> {
    let mut selected: HashMap<(String, String), usize> = HashMap::new();
    let mut out: Vec<Rec> = Vec::with_capacity(records.len());
    for rec in records {
        if rec.provider != "claude" || rec.stable_id.is_none() {
            out.push(rec);
            continue;
        }
        let key = (
            rec.session.clone(),
            rec.stable_id.clone().unwrap_or_default(),
        );
        if let Some(&index) = selected.get(&key) {
            if rec.cache_observed || !out[index].cache_observed { out[index] = rec; }
        } else {
            selected.insert(key, out.len());
            out.push(rec);
        }
    }
    out
}

const MAX_CODEX_FILES: usize = 10_000;

/// Rollouts live beneath date directories. Bound both traversal and retained data: each returned
/// record contains timestamps and numeric token counters only.
fn codex_rollout_files(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut out = Vec::new();
    let mut visited = 0;
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if out.len() >= MAX_CODEX_FILES || visited > 20_000 {
                return out;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file()
                && path.extension().is_some_and(|ext| ext == "jsonl")
                && entry.file_name().to_string_lossy().starts_with("rollout-")
            {
                out.push(path);
            }
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct CodexTotals {
    input: u64,
    cached: u64,
    output: u64,
    cache_write: Option<u64>,
}

fn codex_usage(v: &serde_json::Value) -> Option<CodexTotals> {
    let totals = v.pointer("/payload/info/total_token_usage")?;
    let parsed = CodexTotals {
        input: totals.get("input_tokens")?.as_u64()?,
        cached: totals.get("cached_input_tokens")?.as_u64()?,
        output: totals.get("output_tokens")?.as_u64()?,
        // The field is newer than the other totals, so it is optional rather than making the
        // otherwise complete cache read observation unknown.
        cache_write: totals
            .get("cache_write_input_tokens")
            .and_then(|v| v.as_u64()),
    };
    (parsed.cached <= parsed.input).then_some(parsed)
}

/// Cumulative Codex snapshots become per-delta records. A first snapshot, an incomplete vector,
/// or a counter rollback establishes a new baseline but is deliberately unknown rather than a
/// synthetic usage amount. turn_context supplies the active model for following deltas.
#[cfg(test)]
fn parse_codex_records(content: &str) -> Vec<Rec> {
    parse_codex_reader(BufReader::new(std::io::Cursor::new(content)))
}

enum CodexKind {
    Session(String),
    Turn { model: Option<String>, cwd: String },
    Token(Option<CodexTotals>),
}

struct CodexEvent {
    epoch: i64,
    index: usize,
    kind: CodexKind,
}

fn parse_codex_reader(reader: impl BufRead) -> Vec<Rec> {
    // Parse one line at a time and retain only the timestamp, active context, and counters. A
    // rollout can contain image/tool payloads; they are discarded after each line.
    let mut events = Vec::new();
    for (index, line) in reader.lines().map_while(Result::ok).enumerate() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let Some(epoch) = v
            .get("timestamp")
            .and_then(|v| v.as_str())
            .and_then(parse_epoch)
        else {
            continue;
        };
        let kind = match v.get("type").and_then(|v| v.as_str()) {
            Some("session_meta") => CodexKind::Session(
                v.pointer("/payload/id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            ),
            Some("turn_context") => CodexKind::Turn {
                model: v
                    .pointer("/payload/model")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                cwd: v
                    .pointer("/payload/cwd")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            },
            Some("event_msg")
                if v.pointer("/payload/type").and_then(|v| v.as_str()) == Some("token_count") =>
            {
                CodexKind::Token(codex_usage(&v))
            }
            _ => continue,
        };
        events.push(CodexEvent { epoch, index, kind });
    }
    events.sort_by_key(|event| (event.epoch, event.index));

    let mut model: Option<String> = None;
    let mut baseline: Option<CodexTotals> = None;
    let mut session = String::new();
    let mut cwd = String::new();
    let mut out = Vec::new();
    for event in events {
        let epoch = event.epoch;
        let current = match event.kind {
            CodexKind::Session(id) => {
                if session != id { baseline = None; model = Some("unknown".into()); }
                session = id;
                continue;
            }
            CodexKind::Turn {
                model: next_model,
                cwd: next_cwd,
            } => {
                model = Some(next_model.unwrap_or_else(|| "unknown".into()));
                cwd = next_cwd;
                continue;
            }
            CodexKind::Token(current) => if session.is_empty() { None } else { current },
        };
        let (input, cache_r, output, cache_c, observed, unknown) = match (baseline, current) {
            (_, None) => {
                // An incomplete vector cannot safely advance any fieldwise counter. Force the
                // next complete snapshot to become a fresh baseline as well.
                baseline = None;
                (0, 0, 0, 0, false, true)
            }
            (None, Some(current)) => {
                baseline = Some(current);
                (0, 0, 0, 0, false, true)
            }
            (Some(previous), Some(current)) => {
                if current == previous { continue; }
                let valid = current.input >= previous.input
                    && current.cached >= previous.cached
                    && current.output >= previous.output
                    && current.cached <= current.input
                    && current.cached.saturating_sub(previous.cached) <= current.input.saturating_sub(previous.input)
                    && !matches!((previous.cache_write, current.cache_write), (Some(before), Some(after)) if after < before);
                if !valid {
                    // Counter resets can happen at compaction/rollout boundaries. Do not
                    // attribute the following full counter as a delta from a stale total.
                    baseline = None;
                    (0, 0, 0, 0, false, true)
                } else {
                    baseline = Some(current);
                    let cache_c = match (previous.cache_write, current.cache_write) {
                        (Some(before), Some(after)) if after >= before => after - before,
                        _ => 0,
                    };
                    (
                        current.input - previous.input,
                        current.cached - previous.cached,
                        current.output - previous.output,
                        cache_c,
                        true,
                        false,
                    )
                }
            }
        };
        out.push(Rec {
            epoch,
            session: session.clone(),
            cwd: cwd.clone(),
            model: model.clone(),
            provider: "codex".to_string(),
            input,
            output,
            cache_c,
            cache_r,
            cache_observed: observed,
            cache_unknown: unknown,
            stable_id: current.map(|c| format!("{epoch}:{}:{}:{}:{:?}", c.input, c.cached, c.output, c.cache_write)),
        });
    }
    out
}

/// 레코드를 누적기에 반영. cutoff 이상은 acc로, [prev_cutoff, cutoff)는 prev로 분배한다.
fn apply_records(
    acc: &mut Acc,
    prev: &mut Option<PrevAcc>,
    recs: &[Rec],
    cutoff: i64,
    prev_cutoff: i64,
    offset_secs: i64,
) {
    for r in recs {
        if r.epoch < prev_cutoff {
            continue;
        }
        let ts = ts_from_epoch(r.epoch, offset_secs);
        let msg = Msg {
            session: &r.session,
            cwd: &r.cwd,
            model: r.model.as_deref(),
            provider: &r.provider,
            input: r.input,
            output: r.output,
            cache_c: r.cache_c,
            cache_r: r.cache_r,
            cache_observed: r.cache_observed,
            cache_unknown: r.cache_unknown,
        };
        if r.epoch >= cutoff {
            acc.add(&ts, &msg);
        } else if let Some(p) = prev.as_mut() {
            p.add(&ts, &msg);
        }
    }
}

/// 누적기 → 최종 Insights (스트릭/피크/정렬 계산). today = 로컬 오늘의 day 번호.
fn finalize(acc: Acc, prev: Option<PrevAcc>, today: i64) -> Insights {
    // 스트릭: 연속 활동일.
    let set = &acc.active_days;
    let mut current_streak = 0u64;
    let mut d = today;
    if !set.contains(&d) {
        d -= 1; // 오늘 미활동이면 어제부터 카운트.
    }
    while set.contains(&d) {
        current_streak += 1;
        d -= 1;
    }
    let mut longest_streak = 0u64;
    let mut run = 0u64;
    let mut prev_day: Option<i64> = None;
    for &dn in set.iter() {
        run = match prev_day {
            Some(p) if dn == p + 1 => run + 1,
            _ => 1,
        };
        longest_streak = longest_streak.max(run);
        prev_day = Some(dn);
    }

    let peak_hour = acc
        .hours
        .iter()
        .enumerate()
        .filter(|(_, &c)| c > 0)
        .max_by_key(|(_, &c)| c)
        .map(|(h, _)| h as u32);

    let days: Vec<DayStat> = acc
        .day_msgs
        .values()
        .map(|(date, m, t)| DayStat {
            date: date.clone(),
            messages: *m,
            tokens: *t,
        })
        .collect();

    let mut models: Vec<ModelStat> = acc
        .models
        .into_iter()
        .map(|(name, mut s)| {
            s.sessions = acc
                .model_sessions
                .get(&name)
                .map(|set| set.len() as u64)
                .unwrap_or(0);
            s
        })
        .collect();
    models.sort_by(|a, b| b.messages.cmp(&a.messages));
    let favorite_model = models.first().map(|m| m.model.clone());

    // 프로젝트: 토큰 내림차순 정렬 후 표시명 부여(충돌 시 상위 세그먼트 추가).
    let mut projects: Vec<ProjectStat> = acc
        .projects
        .into_iter()
        .map(|(path, p)| ProjectStat {
            path,
            name: String::new(),
            sessions: p.sessions.len() as u64,
            messages: p.messages,
            total_tokens: p.total_tokens,
            last_active: date_string(p.last_day),
        })
        .collect();
    projects.sort_by(|a, b| {
        b.total_tokens
            .cmp(&a.total_tokens)
            .then_with(|| a.path.cmp(&b.path))
    });
    let paths: Vec<String> = projects.iter().map(|p| p.path.clone()).collect();
    for (p, name) in projects.iter_mut().zip(display_names(&paths)) {
        p.name = name;
    }

    Insights {
        sessions: acc.sessions.len() as u64,
        messages: acc.messages,
        total_tokens: acc.total_tokens,
        input_tokens: acc.input_tokens,
        output_tokens: acc.output_tokens,
        cache_creation_tokens: acc.cache_creation_tokens,
        cache_read_tokens: acc.cache_read_tokens,
        cache_observed_messages: acc.cache_observed_messages,
        cache_observed_input_tokens: acc.cache_observed_input_tokens,
        cache_observed_read_tokens: acc.cache_observed_read_tokens,
        cache_unknown_messages: acc.cache_unknown_messages,
        cache_days: acc.cache_days.into_values().collect(),
        prompt_injection: None,
        active_days: acc.active_days.len() as u64,
        current_streak,
        longest_streak,
        peak_hour,
        favorite_model,
        days,
        hours: acc.hours.to_vec(),
        weekday_hours: acc.weekday_hours.iter().flatten().copied().collect(),
        models,
        projects,
        prev: prev.map(PrevAcc::finish),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::MutexGuard;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// 캐시와 FILE_READS는 전역이다 — 파일을 읽는 테스트는 전부 직렬화한다.
    fn guard() -> MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 가짜 트랜스크립트 루트(`…/projects`). 테스트마다 다른 경로를 쓴다.
    fn tmp_projects(name: &str) -> PathBuf {
        let root = crate::testtmp::dir()
            .join(format!("insights-{}-{name}", std::process::id()))
            .join("projects");
        std::fs::create_dir_all(root.join("proj")).unwrap();
        root
    }

    fn write_jsonl(root: &Path, file: &str, content: &str) {
        std::fs::write(root.join("proj").join(file), content).unwrap();
    }

    fn line(ty: &str, ts: &str, session: &str, model: &str, input: u64, output: u64) -> String {
        if ty == "assistant" {
            format!(
                r#"{{"type":"assistant","timestamp":"{ts}","sessionId":"{session}","message":{{"model":"{model}","usage":{{"input_tokens":{input},"output_tokens":{output},"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
            )
        } else {
            format!(
                r#"{{"type":"user","timestamp":"{ts}","sessionId":"{session}","message":{{"content":"hi"}}}}"#
            )
        }
    }

    /// cwd 포함 assistant 라인.
    fn line_cwd(ts: &str, session: &str, cwd: &str, tokens: u64) -> String {
        let cwd = cwd.replace('\\', "\\\\");
        format!(
            r#"{{"type":"assistant","timestamp":"{ts}","sessionId":"{session}","cwd":"{cwd}","message":{{"model":"claude-opus-4-8","usage":{{"input_tokens":{tokens},"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
        )
    }

    /// 테스트 기본값 — 직전 구간 비활성.
    fn feed(acc: &mut Acc, content: &str, cutoff: i64, offset_secs: i64) {
        apply_records(
            acc,
            &mut None,
            &parse_records(content),
            cutoff,
            0,
            offset_secs,
        );
    }

    #[test]
    fn aggregates_messages_tokens_models() {
        let content = [
            line("user", "2026-06-30T08:00:00.000Z", "s1", "", 0, 0),
            line(
                "assistant",
                "2026-06-30T08:00:01.000Z",
                "s1",
                "claude-opus-4-8",
                100,
                50,
            ),
            line(
                "assistant",
                "2026-06-30T08:30:01.000Z",
                "s1",
                "claude-opus-4-8",
                20,
                10,
            ),
            line(
                "assistant",
                "2026-06-29T08:00:01.000Z",
                "s2",
                "claude-sonnet-4-6",
                10,
                5,
            ),
        ]
        .join("\n");
        let mut acc = Acc::default();
        feed(&mut acc, &content, 0, 0);
        let today = days_from_civil(2026, 6, 30);
        let ins = finalize(acc, None, today);
        assert_eq!(ins.messages, 4);
        assert_eq!(ins.sessions, 2);
        assert_eq!(ins.total_tokens, 150 + 30 + 15);
        assert_eq!(ins.models.len(), 2);
        assert_eq!(ins.favorite_model.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(ins.active_days, 2);
        assert_eq!(ins.current_streak, 2); // 6/30, 6/29 연속
        assert_eq!(ins.peak_hour, Some(8));
        assert!(ins.prev.is_none());
    }

    #[test]
    fn cutoff_filters_old_messages() {
        let content = [
            line("assistant", "2026-06-30T08:00:01.000Z", "s1", "m", 100, 50),
            line("assistant", "2020-01-01T08:00:01.000Z", "s2", "m", 100, 50),
        ]
        .join("\n");
        let now = days_from_civil(2026, 6, 30) * 86400 + 9 * 3600;
        let mut acc = Acc::default();
        feed(&mut acc, &content, cutoff("7d", now), 0);
        let ins = finalize(acc, None, days_from_civil(2026, 6, 30));
        assert_eq!(ins.messages, 1);
        assert_eq!(ins.sessions, 1);
    }

    #[test]
    fn local_offset_shifts_day_and_hour() {
        // UTC 16:00 + KST(+9h) = 익일 01:00 로컬.
        let content = line("assistant", "2026-06-29T16:00:00.000Z", "s1", "m", 1, 1);
        let mut acc = Acc::default();
        feed(&mut acc, &content, 0, 9 * 3600);
        let ins = finalize(acc, None, days_from_civil(2026, 6, 30));
        assert_eq!(ins.days[0].date, "2026-06-30");
        assert_eq!(ins.peak_hour, Some(1));
    }

    #[test]
    fn civil_roundtrip() {
        for &d in &[0i64, 1, 10957, 20000, -100] {
            let (y, m, day) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, day), d);
        }
    }

    #[test]
    fn days_from_civil_epoch() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1970, 1, 2), 1);
        assert_eq!(days_from_civil(2000, 1, 1), 10957);
    }

    #[test]
    fn aggregates_projects_by_cwd() {
        let content = [
            line_cwd("2026-06-30T08:00:00.000Z", "s1", "/home/me/alpha", 100),
            line_cwd("2026-06-30T09:00:00.000Z", "s1", "/home/me/alpha", 50),
            line_cwd("2026-06-28T09:00:00.000Z", "s2", "/home/me/beta", 400),
        ]
        .join("\n");
        let mut acc = Acc::default();
        feed(&mut acc, &content, 0, 0);
        let ins = finalize(acc, None, days_from_civil(2026, 6, 30));
        assert_eq!(ins.projects.len(), 2);
        // 토큰 내림차순 — beta(400)가 먼저.
        assert_eq!(ins.projects[0].name, "beta");
        assert_eq!(ins.projects[0].total_tokens, 400);
        assert_eq!(ins.projects[0].sessions, 1);
        assert_eq!(ins.projects[0].last_active, "2026-06-28");
        assert_eq!(ins.projects[1].name, "alpha");
        assert_eq!(ins.projects[1].messages, 2);
        assert_eq!(ins.projects[1].last_active, "2026-06-30");
    }

    #[test]
    fn project_names_disambiguate_on_collision() {
        // 같은 마지막 세그먼트(app) 두 개 + 충돌 없는 하나.
        let names = display_names(&[
            "C:\\Users\\me\\OneDrive\\work\\app".to_string(),
            "/home/me/personal/app".to_string(),
            "/home/me/praxis-main".to_string(),
        ]);
        assert_eq!(names[0], "work/app");
        assert_eq!(names[1], "personal/app");
        // 충돌하지 않은 항목은 짧은 이름 유지.
        assert_eq!(names[2], "praxis-main");
    }

    #[test]
    fn project_names_go_deeper_until_unique() {
        // 마지막 두 세그먼트까지 같아 세 번째까지 가야 갈린다.
        let names = display_names(&["/a/one/src/app".to_string(), "/b/two/src/app".to_string()]);
        assert_eq!(names[0], "one/src/app");
        assert_eq!(names[1], "two/src/app");
    }

    #[test]
    fn weekday_hours_bucket() {
        // 2026-06-30은 화요일 → weekday 인덱스 2. 08시.
        let content = line("assistant", "2026-06-30T08:00:00.000Z", "s1", "m", 1, 1);
        let mut acc = Acc::default();
        feed(&mut acc, &content, 0, 0);
        let ins = finalize(acc, None, days_from_civil(2026, 6, 30));
        assert_eq!(ins.weekday_hours.len(), 168);
        assert_eq!(ins.weekday_hours[2 * 24 + 8], 1);
        assert_eq!(ins.weekday_hours.iter().sum::<u64>(), 1);
    }

    #[test]
    fn weekday_of_known_dates() {
        assert_eq!(weekday(days_from_civil(2026, 6, 28)), 0); // 일
        assert_eq!(weekday(days_from_civil(2026, 6, 30)), 2); // 화
        assert_eq!(weekday(days_from_civil(1970, 1, 1)), 4); // 목
    }

    #[test]
    fn prev_window_separates_from_current() {
        // now = 2026-06-30 09:00 UTC, range 7d → 현재 [6/23, 6/30], 직전 [6/16, 6/23].
        let now = days_from_civil(2026, 6, 30) * 86400 + 9 * 3600;
        let content = [
            line("assistant", "2026-06-29T08:00:00.000Z", "s1", "m", 100, 0), // 현재
            line("assistant", "2026-06-20T08:00:00.000Z", "s2", "m", 70, 0),  // 직전
            line("assistant", "2026-06-01T08:00:00.000Z", "s3", "m", 999, 0), // 둘 다 아님
        ]
        .join("\n");
        let cut = cutoff("7d", now);
        let prev_cut = cut - 7 * 86400;
        let mut acc = Acc::default();
        let mut prev = Some(PrevAcc::default());
        apply_records(
            &mut acc,
            &mut prev,
            &parse_records(&content),
            cut,
            prev_cut,
            0,
        );
        let ins = finalize(acc, prev, days_from_civil(2026, 6, 30));
        assert_eq!(ins.messages, 1);
        assert_eq!(ins.total_tokens, 100);
        let p = ins.prev.expect("7d는 직전 구간이 있어야 한다");
        assert_eq!(p.messages, 1);
        assert_eq!(p.total_tokens, 70);
        assert_eq!(p.sessions, 1);
        assert_eq!(p.active_days, 1);
    }

    #[test]
    fn cache_tokens_roll_up() {
        let content = r#"{"type":"assistant","timestamp":"2026-06-30T08:00:00.000Z","sessionId":"s1","message":{"model":"claude-opus-4-8","usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":700,"cache_creation_input_tokens":30}}}"#.to_string();
        let mut acc = Acc::default();
        feed(&mut acc, &content, 0, 0);
        let ins = finalize(acc, None, days_from_civil(2026, 6, 30));
        assert_eq!(ins.cache_read_tokens, 700);
        assert_eq!(ins.cache_creation_tokens, 30);
        assert_eq!(ins.total_tokens, 10 + 5 + 700 + 30);
    }

    #[test]
    fn cache_zero_is_observed_but_missing_fields_are_unknown() {
        let known = r#"{"type":"assistant","timestamp":"2026-06-30T08:00:00Z","sessionId":"s","message":{"id":"one","model":"m","usage":{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;
        let missing = r#"{"type":"assistant","timestamp":"2026-06-30T08:01:00Z","sessionId":"s","message":{"id":"two","model":"m","usage":{"input_tokens":0,"output_tokens":0}}}"#;
        let mut acc = Acc::default();
        feed(&mut acc, &format!("{known}\n{missing}"), 0, 0);
        let ins = finalize(acc, None, days_from_civil(2026, 6, 30));
        assert_eq!(ins.cache_observed_messages, 1);
        assert_eq!(ins.cache_unknown_messages, 1);
        assert_eq!(ins.models[0].cache_observed_messages, 1);
        assert_eq!(ins.models[0].cache_unknown_messages, 1);
    }

    #[test]
    fn claude_rewritten_message_keeps_latest_usage_but_request_ids_stay_distinct() {
        let line = |request: &str, input: u64| {
            format!(
                r#"{{"type":"assistant","timestamp":"2026-06-30T08:00:00Z","sessionId":"s","requestId":"{request}","message":{{"id":"same","model":"m","usage":{{"input_tokens":{input},"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
            )
        };
        let same_request = [line("r1", 10), line("r1", 20)].join("\n");
        let once = parse_records(&same_request);
        assert_eq!(once.len(), 1);
        assert_eq!(once[0].input, 20);
        let separate_requests = [line("r1", 10), line("r2", 20)].join("\n");
        assert_eq!(parse_records(&separate_requests).len(), 2);
    }

    #[test]
    fn codex_deltas_use_pre_range_baseline_model_and_skip_repeats_or_resets() {
        let content = [
            r#"{"timestamp":"2026-06-20T00:00:00Z","type":"session_meta","payload":{"id":"s"}}"#,
            r#"{"timestamp":"2026-06-20T00:00:01Z","type":"turn_context","payload":{"model":"gpt-a","cwd":"/tmp/a"}}"#,
            r#"{"timestamp":"2026-06-20T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":50,"cache_write_input_tokens":5,"output_tokens":10}}}}"#,
            r#"{"timestamp":"2026-06-30T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":160,"cached_input_tokens":90,"cache_write_input_tokens":7,"output_tokens":16}}}}"#,
            r#"{"timestamp":"2026-06-30T00:00:04Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":160,"cached_input_tokens":90,"cache_write_input_tokens":7,"output_tokens":16}}}}"#,
            r#"{"timestamp":"2026-06-30T00:00:05Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":1}}}}"#,
        ].join("\n");
        let recs = parse_codex_records(&content);
        assert_eq!(recs.len(), 3);
        assert!(recs[0].cache_unknown);
        assert!(recs[1].cache_observed);
        assert_eq!(
            (recs[1].input, recs[1].cache_r, recs[1].cache_c),
            (60, 40, 2)
        );
        assert!(recs[2].cache_unknown);
        let mut acc = Acc::default();
        let mut prev = None;
        apply_records(
            &mut acc,
            &mut prev,
            &recs,
            cutoff("7d", days_from_civil(2026, 6, 30) * 86400 + 3600),
            0,
            0,
        );
        let ins = finalize(acc, None, days_from_civil(2026, 6, 30));
        assert_eq!(
            ins.input_tokens, 60,
            "the pre-range snapshot is baseline only"
        );
        assert_eq!(ins.models[0].provider, "codex");
        assert_eq!(ins.models[0].model, "gpt-a");
        assert_eq!(ins.cache_days[0].date, "2026-06-30");
    }

    #[test]
    fn observed_denominators_exclude_unknown_claude_samples_and_include_cache_input() {
        let known = r#"{"type":"assistant","timestamp":"2026-06-30T08:00:00Z","sessionId":"s","message":{"id":"a","model":"m","usage":{"input_tokens":10,"output_tokens":2,"cache_read_input_tokens":80,"cache_creation_input_tokens":10}}}"#;
        let unknown = r#"{"type":"assistant","timestamp":"2026-06-30T08:01:00Z","sessionId":"s","message":{"id":"b","model":"m","usage":{"input_tokens":900,"output_tokens":1,"cache_read_input_tokens":900}}}"#;
        let mut acc = Acc::default();
        feed(&mut acc, &format!("{known}\n{unknown}"), 0, 0);
        let ins = finalize(acc, None, days_from_civil(2026, 6, 30));
        assert_eq!(ins.cache_observed_input_tokens, 100);
        assert_eq!(ins.cache_observed_read_tokens, 80);
        assert_eq!(ins.models[0].cache_observed_input_tokens, 100);
        assert_eq!(ins.models[0].cache_observed_read_tokens, 80);
        assert_eq!(ins.cache_days[0].input_tokens, 100);
        assert_eq!(ins.cache_days[0].cache_read_tokens, 80);
        assert_eq!(ins.cache_unknown_messages, 1);
        let partial_same = unknown.replace("\"b\"", "\"a\"");
        let recs = parse_records(&format!("{partial_same}\n{known}\n{partial_same}"));
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].input, 10);
        assert!(recs[0].cache_observed);
    }

    #[test]
    fn codex_delta_cannot_cache_more_than_its_input() {
        let content = r#"{"timestamp":"2026-06-20T00:00:00Z","type":"session_meta","payload":{"id":"s"}}
{"timestamp":"2026-06-20T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":1}}}}
{"timestamp":"2026-06-20T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":101,"cached_input_tokens":50,"output_tokens":2}}}}"#;
        let recs = parse_codex_records(content);
        assert_eq!(recs.len(), 2);
        assert!(recs.iter().all(|r| r.cache_unknown && !r.cache_observed));
        assert_eq!(recs[0].model.as_deref(), Some("unknown"));
    }

    #[test]
    fn normalize_repo_root_folds_worktree_path() {
        let never = |_: &Path| false;
        assert_eq!(
            normalize_repo_root("/home/me/app/.praxis/worktrees/wt-1/src", &never),
            "/home/me/app"
        );
    }

    #[test]
    fn normalize_repo_root_climbs_to_repo_ancestor() {
        let is_root = |p: &Path| p == Path::new("/home/me/app");
        assert_eq!(
            normalize_repo_root("/home/me/app/src/insights", &is_root),
            "/home/me/app"
        );
    }

    #[test]
    fn normalize_repo_root_folds_then_climbs() {
        let is_root = |p: &Path| p == Path::new("/home/me/mono");
        assert_eq!(
            normalize_repo_root("/home/me/mono/pkg/.praxis/worktrees/wt-1/src", &is_root),
            "/home/me/mono"
        );
    }

    #[test]
    fn normalize_repo_root_keeps_unmatched_path() {
        let never = |_: &Path| false;
        assert_eq!(
            normalize_repo_root("/home/me/plain", &never),
            "/home/me/plain"
        );
    }

    #[test]
    fn codex_rollouts_are_cached_until_file_changes() {
        let _g = guard();
        let root = tmp_projects("codex-cache-claude");
        let codex = tmp_projects("codex-cache-rollouts");
        let now = days_from_civil(2026, 6, 30) * 86400 + 9 * 3600;
        let meta = [
            r#"{"timestamp":"2026-06-30T00:00:00Z","type":"session_meta","payload":{"id":"s"}}"#,
            r#"{"timestamp":"2026-06-30T00:00:01Z","type":"turn_context","payload":{"model":"gpt-a","cwd":"/tmp/a"}}"#,
            r#"{"timestamp":"2026-06-30T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":10}}}}"#,
        ];
        write_jsonl(&codex, "rollout-a.jsonl", &meta.join("\n"));

        let first = compute_with_roots(&root, Some(&codex), "all", 0, now);
        let after_first = file_read_count();
        let second = compute_with_roots(&root, Some(&codex), "7d", 0, now);
        assert_eq!(
            file_read_count(),
            after_first,
            "range를 바꿔도 rollout을 다시 읽지 않는다"
        );
        assert_eq!(first.total_tokens, second.total_tokens);

        let grown = [
            meta.join("\n"),
            r#"{"timestamp":"2026-06-30T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":150,"cached_input_tokens":0,"output_tokens":20}}}}"#.to_string(),
        ]
        .join("\n");
        write_jsonl(&codex, "rollout-a.jsonl", &grown);
        let third = compute_with_roots(&root, Some(&codex), "all", 0, now);
        assert_eq!(
            file_read_count(),
            after_first + 1,
            "바뀐 rollout 하나만 다시 읽는다"
        );
        assert_eq!(third.total_tokens, first.total_tokens + 60);
    }

    #[test]
    fn cache_reuses_parsed_records_until_file_changes() {
        let _g = guard();
        let root = tmp_projects("cache");
        let now = days_from_civil(2026, 6, 30) * 86400 + 9 * 3600;
        write_jsonl(
            &root,
            "a.jsonl",
            &line("assistant", "2026-06-30T08:00:00.000Z", "s1", "m", 10, 5),
        );
        write_jsonl(
            &root,
            "b.jsonl",
            &line("assistant", "2026-06-30T09:00:00.000Z", "s2", "m", 1, 1),
        );

        let first = compute_in(&root, "all", 0, now);
        let after_first = file_read_count();
        let second = compute_in(&root, "all", 0, now);
        assert_eq!(
            file_read_count(),
            after_first,
            "캐시 히트는 파일을 열지 않는다"
        );
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        );

        // 줄 추가 — mtime이 같아도 크기가 달라져 그 파일만 다시 읽힌다.
        write_jsonl(
            &root,
            "a.jsonl",
            &[
                line("assistant", "2026-06-30T08:00:00.000Z", "s1", "m", 10, 5),
                line("assistant", "2026-06-30T10:00:00.000Z", "s3", "m", 100, 0),
            ]
            .join("\n"),
        );
        let third = compute_in(&root, "all", 0, now);
        assert_eq!(
            file_read_count(),
            after_first + 1,
            "바뀐 파일 하나만 다시 읽는다"
        );
        assert_eq!(third.messages, first.messages + 1);
        assert_eq!(third.total_tokens, first.total_tokens + 100);
    }

    #[test]
    fn dropped_file_leaves_the_result() {
        let _g = guard();
        let root = tmp_projects("dropped");
        let now = days_from_civil(2026, 6, 30) * 86400 + 9 * 3600;
        write_jsonl(
            &root,
            "a.jsonl",
            &line("assistant", "2026-06-30T08:00:00.000Z", "s1", "m", 10, 5),
        );
        write_jsonl(
            &root,
            "b.jsonl",
            &line("assistant", "2026-06-30T09:00:00.000Z", "s2", "m", 7, 0),
        );
        let before = compute_in(&root, "all", 0, now);
        assert_eq!(before.messages, 2);

        std::fs::remove_file(root.join("proj").join("b.jsonl")).unwrap();
        let after = compute_in(&root, "all", 0, now);
        assert_eq!(after.messages, 1);
        assert_eq!(after.sessions, 1);
        assert_eq!(after.total_tokens, 15);
    }

    #[test]
    fn model_stat_carries_provider() {
        let _g = guard();
        let root = tmp_projects("provider");
        let now = days_from_civil(2026, 6, 30) * 86400 + 9 * 3600;
        write_jsonl(
            &root,
            "a.jsonl",
            &line(
                "assistant",
                "2026-06-30T08:00:00.000Z",
                "s1",
                "claude-opus-4-8",
                1,
                1,
            ),
        );
        let ins = compute_in(&root, "all", 0, now);
        assert_eq!(ins.models[0].provider, "claude");
    }
}
