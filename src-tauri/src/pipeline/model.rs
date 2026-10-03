//! 분할 산출물 타입·검증, 스케줄링 보조, 변경 경로 검사. 모두 순수 함수다.

use super::state::TicketState;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// 티켓 수 상한.
pub const MAX_TICKETS: usize = 8;
/// 같은 벤더로 재시도하는 수정 횟수 상한.
pub const MAX_SAME_VENDOR_FIXES: u32 = 2;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Requirement {
    pub id: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub acceptance: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Spec {
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub requirements: Vec<Requirement>,
    #[serde(default)]
    pub out_of_scope: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TicketDraft {
    pub key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub vendor_reason: String,
    #[serde(default)]
    pub acceptance_commands: Vec<String>,
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub deps: Vec<String>,
    #[serde(default)]
    pub covers: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SplitOutput {
    #[serde(default)]
    pub spec: Spec,
    #[serde(default)]
    pub tickets: Vec<TicketDraft>,
}

/// 벤더 id 정규화. `gemini`는 `agy`의 별칭이다.
pub fn normalize_vendor(v: &str) -> String {
    let t = v.trim().to_lowercase();
    match t.as_str() {
        "gemini" => "agy".to_string(),
        _ => t,
    }
}

/// 분할 결과를 검증하고 모든 문제를 모아 돌려준다.
pub fn validate_split(
    out: &SplitOutput,
    available_vendors: &[&str],
    max_tickets: usize,
) -> Result<(), Vec<String>> {
    let mut errs: Vec<String> = Vec::new();
    if out.spec.requirements.is_empty() {
        errs.push("요구사항이 하나도 없다".into());
    }
    if out.tickets.is_empty() {
        errs.push("티켓이 하나도 없다".into());
    }
    if out.tickets.len() > max_tickets {
        errs.push(format!(
            "티켓이 {}개로 상한 {max_tickets}개를 넘는다",
            out.tickets.len()
        ));
    }

    let mut req_ids: HashSet<&str> = HashSet::new();
    for r in &out.spec.requirements {
        if r.id.trim().is_empty() {
            errs.push("요구사항 id가 비어 있다".into());
        } else if !req_ids.insert(r.id.as_str()) {
            errs.push(format!("요구사항 id 중복: {}", r.id));
        }
    }
    let mut keys: HashSet<&str> = HashSet::new();
    for t in &out.tickets {
        if t.key.trim().is_empty() {
            errs.push("티켓 key가 비어 있다".into());
        } else if !keys.insert(t.key.as_str()) {
            errs.push(format!("티켓 key 중복: {}", t.key));
        }
    }

    let mut covered: HashSet<&str> = HashSet::new();
    for t in &out.tickets {
        let k = &t.key;
        if t.title.trim().is_empty() {
            errs.push(format!("{k}: 제목이 비어 있다"));
        }
        let v = normalize_vendor(&t.vendor);
        if !available_vendors.iter().any(|a| normalize_vendor(a) == v) {
            errs.push(format!("{k}: 사용할 수 없는 벤더 '{}'", t.vendor));
        }
        for d in &t.deps {
            if d == &t.key {
                errs.push(format!("{k}: 자기 자신에 의존한다"));
            } else if !keys.contains(d.as_str()) {
                errs.push(format!("{k}: 존재하지 않는 의존 티켓 '{d}'"));
            }
        }
        for c in &t.covers {
            if req_ids.contains(c.as_str()) {
                covered.insert(c.as_str());
            } else {
                errs.push(format!(
                    "{k}: 존재하지 않는 요구사항 '{c}'를 covers에 담았다"
                ));
            }
        }
        if t.allowed_paths.is_empty() {
            errs.push(format!("{k}: allowed_paths가 비어 있다"));
        }
        for p in &t.allowed_paths {
            if !is_safe_relative(p) {
                errs.push(format!("{k}: 허용할 수 없는 경로 '{p}'"));
            }
        }
        if t.acceptance_commands.is_empty() {
            errs.push(format!("{k}: acceptance_commands가 비어 있다"));
        }
        if t.acceptance_commands.iter().any(|c| c.trim().is_empty()) {
            errs.push(format!("{k}: 빈 acceptance_command가 있다"));
        }
    }
    for r in &out.spec.requirements {
        if !r.id.trim().is_empty() && !covered.contains(r.id.as_str()) {
            errs.push(format!("요구사항 {}를 담당하는 티켓이 없다", r.id));
        }
    }
    if let Some(cycle) = find_cycle(&out.tickets) {
        errs.push(format!("의존 순환: {}", cycle.join(" -> ")));
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

fn is_safe_relative(p: &str) -> bool {
    let t = p.trim();
    if t.is_empty() || t.starts_with('/') || t.starts_with('\\') || t.starts_with('~') {
        return false;
    }
    // 윈도 드라이브 문자
    if t.len() >= 2 && t.as_bytes()[1] == b':' {
        return false;
    }
    !t.split(['/', '\\']).any(|seg| seg == "..")
}

/// 의존 그래프의 순환 하나를 key 경로로 돌려준다. 없으면 None.
fn find_cycle(tickets: &[TicketDraft]) -> Option<Vec<String>> {
    let deps: HashMap<&str, Vec<&str>> = tickets
        .iter()
        .map(|t| {
            (
                t.key.as_str(),
                t.deps
                    .iter()
                    .map(|d| d.as_str())
                    .filter(|d| *d != t.key)
                    .collect(),
            )
        })
        .collect();
    // 0 미방문, 1 방문 중, 2 완료
    let mut mark: HashMap<&str, u8> = HashMap::new();
    fn visit<'a>(
        n: &'a str,
        deps: &HashMap<&'a str, Vec<&'a str>>,
        mark: &mut HashMap<&'a str, u8>,
        stack: &mut Vec<&'a str>,
    ) -> Option<Vec<String>> {
        match mark.get(n).copied().unwrap_or(0) {
            2 => return None,
            1 => {
                let at = stack.iter().position(|s| *s == n).unwrap_or(0);
                let mut c: Vec<String> = stack[at..].iter().map(|s| s.to_string()).collect();
                c.push(n.to_string());
                return Some(c);
            }
            _ => {}
        }
        mark.insert(n, 1);
        stack.push(n);
        for d in deps.get(n).into_iter().flatten() {
            if deps.contains_key(d) {
                if let Some(c) = visit(d, deps, mark, stack) {
                    return Some(c);
                }
            }
        }
        stack.pop();
        mark.insert(n, 2);
        None
    }
    let mut order: Vec<&str> = deps.keys().copied().collect();
    order.sort();
    for n in order {
        let mut stack = Vec::new();
        if let Some(c) = visit(n, &deps, &mut mark, &mut stack) {
            return Some(c);
        }
    }
    None
}

/// 스케줄러가 보는 티켓의 가벼운 투영.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TicketView {
    pub id: i64,
    pub key: String,
    pub state: TicketState,
    pub deps: Vec<String>,
    pub vendor: String,
    pub reviewer_vendor: Option<String>,
    pub attempt: u32,
    pub reassigned: bool,
}

/// 선행 티켓이 모두 Integrated인 Pending 티켓 id (key 순).
pub fn ready_tickets(tickets: &[TicketView]) -> Vec<i64> {
    let done: HashSet<&str> = tickets
        .iter()
        .filter(|t| t.state == TicketState::Integrated)
        .map(|t| t.key.as_str())
        .collect();
    let mut v: Vec<&TicketView> = tickets
        .iter()
        .filter(|t| {
            t.state == TicketState::Pending && t.deps.iter().all(|d| done.contains(d.as_str()))
        })
        .collect();
    v.sort_by(|a, b| a.key.cmp(&b.key));
    v.into_iter().map(|t| t.id).collect()
}

/// 통합은 한 번에 하나만 한다. 통합 중인 티켓이 있으면 None, 아니면 key 순 첫 Ready.
pub fn next_integration(tickets: &[TicketView]) -> Option<i64> {
    if tickets.iter().any(|t| t.state == TicketState::Integrating) {
        return None;
    }
    tickets
        .iter()
        .filter(|t| t.state == TicketState::Ready)
        .min_by(|a, b| a.key.cmp(&b.key))
        .map(|t| t.id)
}

/// 모든 티켓이 Integrated 또는 Cancelled이고 Integrated가 하나 이상이면 true.
pub fn all_integrated(tickets: &[TicketView]) -> bool {
    tickets
        .iter()
        .all(|t| matches!(t.state, TicketState::Integrated | TicketState::Cancelled))
        && tickets.iter().any(|t| t.state == TicketState::Integrated)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixDecision {
    RetrySameVendor,
    Reassign,
    Escalate,
}

/// 수정 루프 결정. `attempt`는 현재 벤더에서 이미 수행한 수정 횟수다.
pub fn fix_decision(attempt: u32, reassigned: bool) -> FixDecision {
    if attempt < MAX_SAME_VENDOR_FIXES {
        FixDecision::RetrySameVendor
    } else if !reassigned {
        FixDecision::Reassign
    } else {
        FixDecision::Escalate
    }
}

fn pick_least_assigned(
    exclude: &str,
    available: &[&str],
    counts: &HashMap<String, u32>,
) -> Option<String> {
    let ex = normalize_vendor(exclude);
    let mut best: Option<(&str, u32)> = None;
    for v in available {
        if normalize_vendor(v) == ex {
            continue;
        }
        let c = counts.get(*v).copied().unwrap_or(0);
        if best.is_none_or(|(_, bc)| c < bc) {
            best = Some((v, c));
        }
    }
    best.map(|(v, _)| v.to_string())
}

/// 작성자를 제외하고 배정 수가 가장 적은 벤더(동률이면 `available` 순).
pub fn pick_reviewer(
    author: &str,
    available: &[&str],
    assigned_counts: &HashMap<String, u32>,
) -> Option<String> {
    pick_least_assigned(author, available, assigned_counts)
}

/// 재배정 대상 벤더. 현재 벤더를 제외하고 같은 규칙으로 고른다.
pub fn pick_reassign_vendor(
    current: &str,
    available: &[&str],
    counts: &HashMap<String, u32>,
) -> Option<String> {
    pick_least_assigned(current, available, counts)
}

/// 허용 경로 밖의 변경 경로를 돌려준다.
/// 허용 항목은 정확 일치, 디렉터리 접두(`dir/` 또는 `dir`), `*`·`**` glob을 지원한다.
/// 절대 경로·`..` 포함 변경 경로는 항상 밖으로 본다.
pub fn paths_outside_allowed(changed: &[String], allowed: &[String]) -> Vec<String> {
    let allowed: Vec<String> = allowed
        .iter()
        .map(|a| a.trim().trim_start_matches("./").to_string())
        .filter(|a| !a.is_empty())
        .collect();
    changed
        .iter()
        .filter(|p| {
            let p = p.trim().trim_start_matches("./");
            !(is_safe_relative(p) && allowed.iter().any(|a| path_allowed(p, a)))
        })
        .cloned()
        .collect()
}

fn path_allowed(path: &str, entry: &str) -> bool {
    if entry.contains('*') {
        return glob_match(entry.as_bytes(), path.as_bytes());
    }
    if path == entry {
        return true;
    }
    let dir = entry.trim_end_matches('/');
    path.starts_with(&format!("{dir}/"))
}

/// `**`는 `/`를 포함해 무엇이든(`**/`는 빈 디렉터리 포함), `*`는 `/`를 제외한 무엇이든 맞춘다.
fn glob_match(pat: &[u8], text: &[u8]) -> bool {
    match pat.first() {
        None => text.is_empty(),
        Some(b'*') => {
            if pat.get(1) == Some(&b'*') {
                let mut rest = &pat[2..];
                if rest.first() == Some(&b'/') {
                    // `**/`는 빈 경로도 허용
                    if glob_match(&rest[1..], text) {
                        return true;
                    }
                    rest = &pat[2..];
                }
                (0..=text.len()).any(|i| glob_match(rest, &text[i..]))
            } else {
                let rest = &pat[1..];
                let mut i = 0;
                loop {
                    if glob_match(rest, &text[i..]) {
                        return true;
                    }
                    if i >= text.len() || text[i] == b'/' {
                        return false;
                    }
                    i += 1;
                }
            }
        }
        Some(c) => text.first() == Some(c) && glob_match(&pat[1..], &text[1..]),
    }
}

/// 재시작 멱등 키: `run:{id}:{kind}:{ticket|-}:{attempt}`.
pub fn step_key(run_id: i64, kind: &str, ticket_key: Option<&str>, attempt: u32) -> String {
    format!(
        "run:{run_id}:{kind}:{}:{attempt}",
        ticket_key.unwrap_or("-")
    )
}
