//! 에이전트 × 스킬 사용 통계 — "누가 어떤 스킬을 돌렸는가".
//!
//! 메인 세션 트랜스크립트(`<proj>/*.jsonl`)와 서브에이전트 트랜스크립트
//! (`<proj>/<session>/subagents/agent-*.jsonl`)를 함께 스캔한다. 후자는 사용량 집계(`super::compute`)의
//! 스캔 대상 밖이라, 서브에이전트가 태운 토큰은 지금까지 어느 지표에도 잡히지 않았다.
//!
//! 스킬 귀속에는 신호 두 개를 쓴다. 둘은 세는 대상이 다르므로 합치지 않고 나란히 낸다.
//! - `attributionSkill`(최상위 필드): 그 메시지가 **어느 스킬 맥락에서** 나왔는가 → 메시지·토큰의 근거.
//! - `Skill` tool_use: **명시적 호출** → 호출 횟수의 근거. 슬래시 입력이나 자동 매칭으로 발동한 스킬은
//!   여기 안 잡히므로, 호출 0인데 작업량은 큰 스킬이 정상적으로 존재한다.
//!
//! 에이전트 축은 메인 세션이 `main`, 서브에이전트가 `.meta.json`의 `agentType`이다.
//! 스킬 하나가 통째로 서브에이전트로 fork된 경우(`.forked-skill.json`)에는 그 트랜스크립트 전체를
//! 해당 스킬에 귀속한다 — fork된 쪽에는 `attributionSkill`이 안 붙는 경우가 있기 때문.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use serde::Serialize;

use super::{cutoff, parse_ts};

/// 메인 세션의 에이전트 키 — 서브에이전트 `agentType`과 같은 축에 둔다.
const MAIN_AGENT: &str = "main";

/// `.meta.json`이 없거나 읽히지 않는 서브에이전트의 키.
const UNKNOWN_AGENT: &str = "unknown";

/// 에이전트 안에서 스킬 하나가 차지한 몫.
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct SkillSlice {
    pub skill: String,
    pub calls: u64,
    pub messages: u64,
    pub tokens: u64,
}

/// 스킬 안에서 에이전트 하나가 차지한 몫.
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct AgentSlice {
    pub agent: String,
    pub calls: u64,
    pub messages: u64,
    pub tokens: u64,
}

/// 에이전트 한 종의 집계.
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct AgentStat {
    pub agent: String,
    /// 스폰 횟수 — 서브에이전트는 트랜스크립트 파일 수, `main`은 세션 수.
    pub runs: u64,
    /// 이 에이전트가 낸 `Skill` 호출 수.
    pub calls: u64,
    pub messages: u64,
    pub tokens: u64,
    /// 어느 스킬에도 귀속되지 않은 메시지 — 스킬 없이 돈 분량.
    pub unattributed_messages: u64,
    /// 토큰 내림차순.
    pub skills: Vec<SkillSlice>,
}

/// 스킬 한 종의 집계.
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct SkillStat {
    pub skill: String,
    pub calls: u64,
    pub messages: u64,
    pub tokens: u64,
    /// 이 스킬이 등장한 고유 세션 수.
    pub sessions: u64,
    /// 토큰 내림차순.
    pub agents: Vec<AgentSlice>,
}

/// 에이전트 × 스킬 전체.
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct AgentSkillUsage {
    /// 토큰 내림차순.
    pub agents: Vec<AgentStat>,
    /// 토큰 내림차순.
    pub skills: Vec<SkillStat>,
    /// 스킬에 귀속된 메시지 총합.
    pub attributed_messages: u64,
    /// 어느 스킬에도 귀속되지 않은 메시지 총합.
    pub unattributed_messages: u64,
    /// 서브에이전트 스폰 총 횟수.
    pub subagent_runs: u64,
}

/// (에이전트, 스킬) 교차 누적.
#[derive(Default)]
struct PairAcc {
    calls: u64,
    messages: u64,
    tokens: u64,
}

#[derive(Default)]
struct AgentAcc {
    runs: u64,
    calls: u64,
    messages: u64,
    tokens: u64,
    unattributed_messages: u64,
    skills: HashMap<String, PairAcc>,
}

#[derive(Default)]
struct SkillAcc {
    calls: u64,
    messages: u64,
    tokens: u64,
    sessions: HashSet<String>,
    agents: HashMap<String, PairAcc>,
}

#[derive(Default)]
struct Acc {
    agents: HashMap<String, AgentAcc>,
    skills: HashMap<String, SkillAcc>,
    attributed_messages: u64,
    unattributed_messages: u64,
    subagent_runs: u64,
}

impl Acc {
    /// 메시지 한 건 — 토큰과 메시지 수를 (에이전트, 스킬)에 반영. skill이 None이면 미귀속.
    fn add_message(&mut self, agent: &str, skill: Option<&str>, session: &str, tokens: u64) {
        let a = self.agents.entry(agent.to_string()).or_default();
        a.messages += 1;
        a.tokens += tokens;
        match skill {
            Some(sk) => {
                let slice = a.skills.entry(sk.to_string()).or_default();
                slice.messages += 1;
                slice.tokens += tokens;
            }
            None => a.unattributed_messages += 1,
        }
        let Some(sk) = skill else {
            self.unattributed_messages += 1;
            return;
        };
        self.attributed_messages += 1;
        let s = self.skills.entry(sk.to_string()).or_default();
        s.messages += 1;
        s.tokens += tokens;
        if !session.is_empty() {
            s.sessions.insert(session.to_string());
        }
        let slice = s.agents.entry(agent.to_string()).or_default();
        slice.messages += 1;
        slice.tokens += tokens;
    }

    /// `Skill` 호출 한 건 — 호출한 에이전트와 **호출 대상** 스킬에 반영한다.
    /// 스킬 A가 실행 중에 스킬 B를 부르면 B의 호출로 센다.
    fn add_call(&mut self, agent: &str, target: &str) {
        let a = self.agents.entry(agent.to_string()).or_default();
        a.calls += 1;
        a.skills.entry(target.to_string()).or_default().calls += 1;
        let s = self.skills.entry(target.to_string()).or_default();
        s.calls += 1;
        s.agents.entry(agent.to_string()).or_default().calls += 1;
    }
}

/// assistant 메시지의 usage 4종 합계. usage가 없으면 0.
fn message_tokens(v: &serde_json::Value) -> u64 {
    let Some(u) = v.get("message").and_then(|m| m.get("usage")) else {
        return 0;
    };
    let g = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    g("input_tokens")
        + g("output_tokens")
        + g("cache_creation_input_tokens")
        + g("cache_read_input_tokens")
}

/// 트랜스크립트 한 줄에서 뽑은 원시 레코드 — cutoff와 무관해 파일 단위로 캐시할 수 있다.
/// 에이전트 키와 fork된 스킬은 사이드카에서 오므로 여기 담지 않고 적용 시점에 얹는다.
struct SkillRec {
    /// UTC epoch(초). 로컬 오프셋과 무관하다.
    epoch: i64,
    session: String,
    attributed: Option<String>,
    tokens: u64,
    /// 이 메시지가 부른 `Skill` 대상들.
    calls: Vec<String>,
}

/// JSONL 본문 → 원시 레코드. user/assistant 외의 줄, 깨진 줄, 시각 없는 줄은 버린다.
fn parse_skill_records(content: &str) -> Vec<SkillRec> {
    let mut out = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
        if ty != "user" && ty != "assistant" {
            continue;
        }
        let Some(epoch) = v
            .get("timestamp")
            .and_then(|x| x.as_str())
            .and_then(|s| parse_ts(s, 0))
            .map(|ts| ts.epoch)
        else {
            continue;
        };
        let session = v
            .get("sessionId")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let attributed = v
            .get("attributionSkill")
            .and_then(|x| x.as_str())
            .map(str::to_string);
        let (tokens, calls) = if ty == "assistant" {
            (message_tokens(&v), skill_calls(&v))
        } else {
            (0, Vec::new())
        };
        out.push(SkillRec {
            epoch,
            session,
            attributed,
            tokens,
            calls,
        });
    }
    out
}

/// assistant 메시지 안의 `Skill` tool_use 대상 이름들.
fn skill_calls(v: &serde_json::Value) -> Vec<String> {
    let Some(content) = v
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
    else {
        return Vec::new();
    };
    content
        .iter()
        .filter(|block| block.get("type").and_then(|x| x.as_str()) == Some("tool_use"))
        .filter(|block| block.get("name").and_then(|x| x.as_str()) == Some("Skill"))
        .filter_map(|block| {
            block
                .get("input")
                .and_then(|i| i.get("skill"))
                .and_then(|x| x.as_str())
        })
        .filter(|target| !target.is_empty())
        .map(str::to_string)
        .collect()
}

/// cutoff 이후 레코드를 누적한다. 한 줄이라도 반영했으면 true — 스폰 수를 셀지 가른다.
fn apply_skill_records(
    acc: &mut Acc,
    recs: &[SkillRec],
    agent: &str,
    forced_skill: Option<&str>,
    cutoff: i64,
) -> bool {
    let mut touched = false;
    for r in recs {
        if r.epoch < cutoff {
            continue;
        }
        touched = true;
        let skill = forced_skill.or(r.attributed.as_deref());
        acc.add_message(agent, skill, &r.session, r.tokens);
        for target in &r.calls {
            acc.add_call(agent, target);
        }
    }
    touched
}

#[cfg(test)]
fn accumulate_transcript(
    acc: &mut Acc,
    content: &str,
    agent: &str,
    forced_skill: Option<&str>,
    cutoff: i64,
) -> bool {
    apply_skill_records(
        acc,
        &parse_skill_records(content),
        agent,
        forced_skill,
        cutoff,
    )
}

/// 스캔 대상 한 건 — 경로, 에이전트 키, fork된 스킬명.
struct Target {
    path: PathBuf,
    agent: String,
    forced_skill: Option<String>,
    /// 서브에이전트 트랜스크립트인가 — 스폰 총계 카운트용.
    subagent: bool,
}

/// 서브에이전트 트랜스크립트 옆의 `.meta.json` / `.forked-skill.json`을 읽어 (에이전트 키, 스킬) 결정.
fn subagent_target(path: PathBuf) -> Target {
    let sidecar = |suffix: &str| -> Option<serde_json::Value> {
        let raw = path.to_string_lossy();
        let stem = raw.strip_suffix(".jsonl")?;
        let text = std::fs::read_to_string(format!("{stem}{suffix}")).ok()?;
        serde_json::from_str(&text).ok()
    };
    let agent = sidecar(".meta.json")
        .and_then(|m| {
            m.get("agentType")
                .and_then(|x| x.as_str())
                .map(str::to_string)
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| UNKNOWN_AGENT.to_string());
    let forced_skill = sidecar(".forked-skill.json")
        .and_then(|f| {
            f.get("skillName")
                .and_then(|x| x.as_str())
                .map(str::to_string)
        })
        .filter(|s| !s.is_empty());
    Target {
        path,
        agent,
        forced_skill,
        subagent: true,
    }
}

/// `~/.claude/projects` 아래 메인 세션 + 서브에이전트 트랜스크립트 전부.
fn transcript_targets() -> Vec<Target> {
    let mut out = Vec::new();
    let Some(root) = crate::sessionhome::projects_root() else {
        return out;
    };
    let Ok(projects) = std::fs::read_dir(&root) else {
        return out;
    };
    for proj in projects.flatten() {
        let dir = proj.path();
        if !dir.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().map(|e| e == "jsonl").unwrap_or(false) {
                out.push(Target {
                    path: p,
                    agent: MAIN_AGENT.to_string(),
                    forced_skill: None,
                    subagent: false,
                });
                continue;
            }
            // `<proj>/<sessionId>/subagents/agent-*.jsonl`
            if !p.is_dir() {
                continue;
            }
            let Ok(subagents) = std::fs::read_dir(p.join("subagents")) else {
                continue;
            };
            for sub in subagents.flatten() {
                let sp = sub.path();
                if sp.extension().map(|e| e == "jsonl").unwrap_or(false) {
                    out.push(subagent_target(sp));
                }
            }
        }
    }
    out
}

/// 토큰 → 메시지 → 이름 순으로 정렬. `key`가 그 세 값을 뽑는다.
/// 상위 목록의 순서가 어디서나 같아야 화면에서 스킬이 튀지 않는다.
fn rank_by_tokens<T, F>(mut items: Vec<T>, key: F) -> Vec<T>
where
    F: Fn(&T) -> (u64, u64, String),
{
    items.sort_by(|a, b| {
        let (at, am, an) = key(a);
        let (bt, bm, bn) = key(b);
        bt.cmp(&at)
            .then_with(|| bm.cmp(&am))
            .then_with(|| an.cmp(&bn))
    });
    items
}

fn finalize(acc: Acc) -> AgentSkillUsage {
    let mut agents: Vec<AgentStat> = acc
        .agents
        .into_iter()
        .map(|(agent, a)| AgentStat {
            agent,
            runs: a.runs,
            calls: a.calls,
            messages: a.messages,
            tokens: a.tokens,
            unattributed_messages: a.unattributed_messages,
            skills: rank_by_tokens(
                a.skills
                    .into_iter()
                    .map(|(skill, p)| SkillSlice {
                        skill,
                        calls: p.calls,
                        messages: p.messages,
                        tokens: p.tokens,
                    })
                    .collect(),
                |s| (s.tokens, s.messages, s.skill.clone()),
            ),
        })
        .collect();
    agents.sort_by(|a, b| {
        b.tokens
            .cmp(&a.tokens)
            .then_with(|| b.messages.cmp(&a.messages))
            .then_with(|| a.agent.cmp(&b.agent))
    });

    let mut skills: Vec<SkillStat> = acc
        .skills
        .into_iter()
        .map(|(skill, s)| SkillStat {
            skill,
            calls: s.calls,
            messages: s.messages,
            tokens: s.tokens,
            sessions: s.sessions.len() as u64,
            agents: rank_by_tokens(
                s.agents
                    .into_iter()
                    .map(|(agent, p)| AgentSlice {
                        agent,
                        calls: p.calls,
                        messages: p.messages,
                        tokens: p.tokens,
                    })
                    .collect(),
                |a| (a.tokens, a.messages, a.agent.clone()),
            ),
        })
        .collect();
    skills.sort_by(|a, b| {
        b.tokens
            .cmp(&a.tokens)
            .then_with(|| b.messages.cmp(&a.messages))
            .then_with(|| a.skill.cmp(&b.skill))
    });

    AgentSkillUsage {
        agents,
        skills,
        attributed_messages: acc.attributed_messages,
        unattributed_messages: acc.unattributed_messages,
        subagent_runs: acc.subagent_runs,
    }
}

/// 파일 한 개의 파싱 결과 + 그때의 mtime·size. 둘 중 하나라도 다르면 다시 읽는다.
struct CachedSkillFile {
    mtime: Option<SystemTime>,
    size: u64,
    recs: Vec<SkillRec>,
}

static CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedSkillFile>>> = OnceLock::new();

/// 파일을 실제로 읽은 횟수 — 캐시가 스캔을 건너뛰는지 테스트가 확인한다.
static FILE_READS: AtomicUsize = AtomicUsize::new(0);

/// range별 에이전트 × 스킬 집계. 베스트 에포트.
///
/// `_offset_secs`는 커맨드 계약으로 받지만 쓰지 않는다 — 집계는 UTC epoch cutoff만 보고
/// 날짜 버킷을 만들지 않는다.
pub fn compute_agent_skills(range: &str, _offset_secs: i64) -> AgentSkillUsage {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    compute_targets(&transcript_targets(), cutoff(range, now))
}

/// 대상 목록을 주입받는 본체 — 테스트가 가짜 트랜스크립트로 부른다.
///
/// 인사이트의 지출 집계(`super::compute`)처럼 파일 단위 캐시를 둔다. 없으면 range 칩을 바꿀
/// 때마다 메인·서브에이전트 트랜스크립트 전체(GB 단위)를 다시 읽는다.
fn compute_targets(targets: &[Target], cut: i64) -> AgentSkillUsage {
    let mut acc = Acc::default();
    let cell = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cell.lock().unwrap_or_else(|e| e.into_inner());
    let live: HashSet<&PathBuf> = targets.iter().map(|t| &t.path).collect();
    cache.retain(|path, _| live.contains(path));
    for t in targets {
        let recs = cached_recs(&mut cache, &t.path);
        let touched = apply_skill_records(&mut acc, recs, &t.agent, t.forced_skill.as_deref(), cut);
        if !touched {
            continue;
        }
        // 파일 하나 = 스폰 한 번. 메인은 파일 하나가 세션 하나다.
        acc.agents.entry(t.agent.clone()).or_default().runs += 1;
        if t.subagent {
            acc.subagent_runs += 1;
        }
    }
    drop(cache);
    finalize(acc)
}

/// 캐시가 파일의 현재 mtime·size와 맞으면 그대로, 아니면 읽어서 갱신한 뒤 돌려준다.
fn cached_recs<'a>(
    cache: &'a mut HashMap<PathBuf, CachedSkillFile>,
    path: &Path,
) -> &'a [SkillRec] {
    let meta = std::fs::metadata(path).ok();
    let mtime = meta.as_ref().and_then(|m| m.modified().ok());
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let fresh = cache
        .get(path)
        .map(|c| c.mtime == mtime && c.size == size)
        .unwrap_or(false);
    if !fresh {
        FILE_READS.fetch_add(1, Ordering::Relaxed);
        let recs = std::fs::read_to_string(path)
            .map(|c| parse_skill_records(&c))
            .unwrap_or_default();
        cache.insert(path.to_path_buf(), CachedSkillFile { mtime, size, recs });
    }
    cache.get(path).map(|c| c.recs.as_slice()).unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// attributionSkill과 usage를 갖춘 assistant 라인.
    fn assistant(ts: &str, session: &str, skill: Option<&str>, tokens: u64) -> String {
        let attr = skill
            .map(|s| format!(r#","attributionSkill":"{s}""#))
            .unwrap_or_default();
        format!(
            r#"{{"type":"assistant","timestamp":"{ts}","sessionId":"{session}"{attr},"message":{{"model":"claude-opus-5","usage":{{"input_tokens":{tokens},"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
        )
    }

    /// Skill tool_use를 담은 assistant 라인.
    fn skill_call(ts: &str, session: &str, from: Option<&str>, target: &str) -> String {
        let attr = from
            .map(|s| format!(r#","attributionSkill":"{s}""#))
            .unwrap_or_default();
        format!(
            r#"{{"type":"assistant","timestamp":"{ts}","sessionId":"{session}"{attr},"message":{{"model":"claude-opus-5","content":[{{"type":"tool_use","name":"Skill","input":{{"skill":"{target}"}}}}]}}}}"#
        )
    }

    fn feed(acc: &mut Acc, content: &str, agent: &str, forced: Option<&str>) -> bool {
        accumulate_transcript(acc, content, agent, forced, 0)
    }

    fn agent_of<'a>(u: &'a AgentSkillUsage, name: &str) -> &'a AgentStat {
        u.agents
            .iter()
            .find(|a| a.agent == name)
            .unwrap_or_else(|| panic!("에이전트 {name} 없음"))
    }

    fn skill_of<'a>(u: &'a AgentSkillUsage, name: &str) -> &'a SkillStat {
        u.skills
            .iter()
            .find(|s| s.skill == name)
            .unwrap_or_else(|| panic!("스킬 {name} 없음"))
    }

    #[test]
    fn attributes_messages_and_tokens_per_agent_and_skill() {
        let content = [
            assistant("2026-08-01T01:00:00.000Z", "s1", Some("code-reviewer"), 100),
            assistant("2026-08-01T01:00:01.000Z", "s1", Some("code-reviewer"), 40),
            assistant(
                "2026-08-01T01:00:02.000Z",
                "s1",
                Some("verification-loop"),
                10,
            ),
        ]
        .join("\n");
        let mut acc = Acc::default();
        assert!(feed(&mut acc, &content, MAIN_AGENT, None));
        let u = finalize(acc);

        let main = agent_of(&u, MAIN_AGENT);
        assert_eq!(main.messages, 3);
        assert_eq!(main.tokens, 150);
        // 토큰 내림차순 — code-reviewer가 앞.
        assert_eq!(main.skills[0].skill, "code-reviewer");
        assert_eq!(main.skills[0].tokens, 140);
        assert_eq!(main.skills[0].messages, 2);
        assert_eq!(main.skills[1].skill, "verification-loop");

        assert_eq!(skill_of(&u, "code-reviewer").sessions, 1);
        assert_eq!(u.attributed_messages, 3);
        assert_eq!(u.unattributed_messages, 0);
    }

    #[test]
    fn separates_subagent_from_main() {
        let mut acc = Acc::default();
        feed(
            &mut acc,
            &assistant("2026-08-01T01:00:00.000Z", "s1", Some("code-reviewer"), 100),
            MAIN_AGENT,
            None,
        );
        feed(
            &mut acc,
            &assistant("2026-08-01T01:00:01.000Z", "s1", Some("code-reviewer"), 900),
            "logic-reviewer",
            None,
        );
        let u = finalize(acc);

        assert_eq!(agent_of(&u, MAIN_AGENT).tokens, 100);
        assert_eq!(agent_of(&u, "logic-reviewer").tokens, 900);
        // 스킬 쪽에서 보면 같은 스킬이 두 에이전트로 갈린다. 토큰 큰 쪽이 앞.
        let sk = skill_of(&u, "code-reviewer");
        assert_eq!(sk.tokens, 1000);
        assert_eq!(sk.agents[0].agent, "logic-reviewer");
        assert_eq!(sk.agents[1].agent, MAIN_AGENT);
    }

    #[test]
    fn forked_skill_attributes_whole_transcript() {
        // fork된 서브에이전트에는 attributionSkill이 없어도 스킬에 귀속돼야 한다.
        let content = [
            assistant("2026-08-01T01:00:00.000Z", "s1", None, 50),
            assistant("2026-08-01T01:00:01.000Z", "s1", None, 70),
        ]
        .join("\n");
        let mut acc = Acc::default();
        feed(&mut acc, &content, "general-purpose", Some("code-explorer"));
        let u = finalize(acc);

        assert_eq!(skill_of(&u, "code-explorer").tokens, 120);
        assert_eq!(u.unattributed_messages, 0);
        assert_eq!(agent_of(&u, "general-purpose").unattributed_messages, 0);
    }

    #[test]
    fn counts_calls_against_target_skill_not_caller() {
        // 스킬 A 실행 중 스킬 B를 부르면, 호출은 B로 세고 메시지는 A에 남는다.
        let content = skill_call(
            "2026-08-01T01:00:00.000Z",
            "s1",
            Some("feature-development"),
            "verification-loop",
        );
        let mut acc = Acc::default();
        feed(&mut acc, &content, MAIN_AGENT, None);
        let u = finalize(acc);

        assert_eq!(skill_of(&u, "verification-loop").calls, 1);
        assert_eq!(skill_of(&u, "verification-loop").messages, 0);
        assert_eq!(skill_of(&u, "feature-development").calls, 0);
        assert_eq!(skill_of(&u, "feature-development").messages, 1);
        assert_eq!(agent_of(&u, MAIN_AGENT).calls, 1);
    }

    #[test]
    fn tracks_unattributed_messages() {
        let content = [
            assistant("2026-08-01T01:00:00.000Z", "s1", None, 30),
            assistant("2026-08-01T01:00:01.000Z", "s1", Some("research"), 20),
        ]
        .join("\n");
        let mut acc = Acc::default();
        feed(&mut acc, &content, MAIN_AGENT, None);
        let u = finalize(acc);

        assert_eq!(u.unattributed_messages, 1);
        assert_eq!(u.attributed_messages, 1);
        let main = agent_of(&u, MAIN_AGENT);
        assert_eq!(main.unattributed_messages, 1);
        // 미귀속 메시지는 에이전트 합계에는 들어가되 스킬 분해에는 안 들어간다.
        assert_eq!(main.messages, 2);
        assert_eq!(main.tokens, 50);
        assert_eq!(main.skills.len(), 1);
    }

    #[test]
    fn cutoff_excludes_older_lines_and_untouched_files() {
        let old = assistant("2026-08-01T01:00:00.000Z", "s1", Some("research"), 100);
        let mut acc = Acc::default();
        // cutoff를 라인 시각보다 뒤로 두면 아무것도 반영되지 않는다 → 스폰도 세지 않는다.
        let touched = accumulate_transcript(
            &mut acc,
            &old,
            MAIN_AGENT,
            None,
            2_000_000_000, // 2033년
        );
        assert!(!touched);
        assert!(finalize(acc).skills.is_empty());
    }

    #[test]
    fn ignores_malformed_lines_and_non_message_types() {
        let content = [
            "{ not json".to_string(),
            r#"{"type":"summary","timestamp":"2026-08-01T01:00:00.000Z"}"#.to_string(),
            r#"{"type":"assistant"}"#.to_string(), // timestamp 없음
            assistant("2026-08-01T01:00:03.000Z", "s1", Some("research"), 5),
        ]
        .join("\n");
        let mut acc = Acc::default();
        assert!(feed(&mut acc, &content, MAIN_AGENT, None));
        let u = finalize(acc);
        assert_eq!(u.attributed_messages, 1);
        assert_eq!(skill_of(&u, "research").tokens, 5);
    }

    #[test]
    fn cache_reuses_parsed_records_across_ranges_until_file_changes() {
        let dir = crate::testtmp::dir().join(format!("agent-skills-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("main.jsonl");
        let first_line = assistant("2026-08-01T01:00:00.000Z", "s1", Some("research"), 10);
        std::fs::write(&path, &first_line).unwrap();
        let targets = [Target {
            path: path.clone(),
            agent: MAIN_AGENT.to_string(),
            forced_skill: None,
            subagent: false,
        }];

        let reads = || FILE_READS.load(Ordering::Relaxed);
        let first = compute_targets(&targets, 0);
        let after_first = reads();
        let narrowed = compute_targets(&targets, 2_000_000_000);
        assert_eq!(
            reads(),
            after_first,
            "range를 바꿔도 파일을 다시 읽지 않는다"
        );
        assert!(narrowed.skills.is_empty());
        assert_eq!(skill_of(&first, "research").tokens, 10);

        let grown = [
            first_line,
            assistant("2026-08-01T02:00:00.000Z", "s1", Some("research"), 5),
        ]
        .join("\n");
        std::fs::write(&path, grown).unwrap();
        let third = compute_targets(&targets, 0);
        assert_eq!(reads(), after_first + 1, "바뀐 파일만 다시 읽는다");
        assert_eq!(skill_of(&third, "research").tokens, 15);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn user_lines_carry_attribution_without_tokens() {
        let content = r#"{"type":"user","timestamp":"2026-08-01T01:00:00.000Z","sessionId":"s1","attributionSkill":"github-operator","message":{"content":"hi"}}"#;
        let mut acc = Acc::default();
        feed(&mut acc, content, MAIN_AGENT, None);
        let u = finalize(acc);
        let sk = skill_of(&u, "github-operator");
        assert_eq!(sk.messages, 1);
        assert_eq!(sk.tokens, 0);
    }
}
