//! P10 종단 검증: stub 벤더 CLI(`claude`·`codex`·`agy`, PATH 선행)로 파이프라인 흐름 전체를 돈다.
//!
//! 이음매: 드라이버(`commands::pipeline_driver`)는 `AppHandle`·`AppState`를 요구하고 crate의 tauri에
//! `test` feature가 없어 직접 틱을 돌릴 수 없다. 그래서 다음 동작은 **실제 `plan::plan_next_action`이
//! 고른 순서대로**, 드라이버가 부르는 것과 같은 Tauri 비의존 함수(`pipeline::{split,review_run,checks,
//! git_ops,db}`, `reviewer::{run_claude_readonly_in,run_reviewer_registered,run_repair_agent}`,
//! `worktree::create_plain`, `TaskService::create_task`)로 재구성해 실행한다. 벤더 호출은 모두 실제
//! 프로세스 실행 경로를 지나 PATH의 stub 스크립트에 닿는다.
//!
//! stub이 대신하지 못하는 것: `create_task_internal`(PTY 에이전트 spawn·동시 실행 상한),
//! `close_pipeline_task`(통합된 티켓 worktree 정리), 이벤트 emit, 실제 벤더 CLI 플래그 호환성.
#![cfg(unix)] // stub 벤더 CLI가 POSIX 셸 스크립트라 Windows에서는 돌지 않는다.

#[path = "support/temp_root.rs"]
mod temp_root;

use praxis_lib::db;
use praxis_lib::managed_process::SharedProcessRegistrar;
use praxis_lib::multireview;
use praxis_lib::multireview::verdict::{has_blocking, ReviewKind, SynthDecision};
use praxis_lib::orchestrator::{TaskDraft, TaskService};
use praxis_lib::pipeline::db::{self as pdb, StepStart, TicketRow};
use praxis_lib::pipeline::model::{self, Spec, SplitOutput, TicketDraft, MAX_TICKETS};
use praxis_lib::pipeline::plan::{self, Action, RunCtx, TicketCtx};
use praxis_lib::pipeline::registry::RunRegistrar;
use praxis_lib::pipeline::review_run::{self, ReviewerFn};
use praxis_lib::pipeline::state::{RunState, TicketState};
use praxis_lib::pipeline::{self, checks, git_ops, prompts, split};
use praxis_lib::{reviewer, worktree};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const VENDORS: [&str; 3] = ["claude", "codex", "agy"];

// ---------- stub 벤더 CLI ----------

/// 세 벤더가 공유하는 stub. 프롬프트는 stdin(리뷰·분할) 또는 인자(쓰기 에이전트)로 온다.
/// nonce가 보이면 nonce 뒤에 JSON만 낸다. 쓰기 호출이면 cwd의 파일을 고친다. 모든 호출을 기록한다.
const STUB: &str = r#"#!/bin/sh
V=$(basename "$0")
IN=$(cat)
ALL="$IN
$*"
LOG="$(dirname "$0")/calls.log"
pick() { printf '%s\n' "$ALL" | grep -o -E "$1" | head -n 1; }

N=$(pick 'PRAXIS-SYNTH-[0-9]+-[0-9a-f]+')
if [ -n "$N" ]; then
  echo "synth $V" >> "$LOG"
  echo "$N"
  echo '{"decision":"pass","summary":"stub synthesis","findings":[{"severity":"advisory","summary":"stub merged note","evidence":"stub","sources":["codex"]}]}'
  exit 0
fi
N=$(pick 'PRAXIS-REVIEW-[0-9]+-[0-9a-f]+')
if [ -n "$N" ]; then
  echo "review $V" >> "$LOG"
  echo "content echo before nonce is ignored"
  echo "$N"
  echo '{"findings":[{"severity":"advisory","summary":"stub note","evidence":"stub"}]}'
  exit 0
fi
N=$(pick 'PRAXIS-SPLIT-[0-9a-f]+')
if [ -n "$N" ]; then
  echo "split $V" >> "$LOG"
  echo "$N"
  cat <<'JSON'
{"spec":{"goal":"e2e","requirements":[{"id":"R1","text":"value A","acceptance":"grep"},{"id":"R2","text":"t2 file after A","acceptance":"test"},{"id":"R3","text":"value C","acceptance":"grep"}],"out_of_scope":[]},
 "tickets":[
  {"key":"T1","title":"set A","body":"shared.txt value -> A","vendor":"codex","vendor_reason":"stub","acceptance_commands":["grep -q 'value=A' shared.txt"],"allowed_paths":["shared.txt"],"deps":[],"covers":["R1"]},
  {"key":"T2","title":"t2 after A","body":"needs T1","vendor":"agy","vendor_reason":"stub","acceptance_commands":["test -f t2.txt","grep -q 'value=A' shared.txt"],"allowed_paths":["t2.txt"],"deps":["T1"],"covers":["R2"]},
  {"key":"T3","title":"set C","body":"shared.txt value -> C","vendor":"claude","vendor_reason":"stub","acceptance_commands":["grep -q 'C$' shared.txt"],"allowed_paths":["shared.txt"],"deps":[],"covers":["R3"]}]}
JSON
  exit 0
fi
case "$ALL" in
  *"merge하다 충돌"*)
    echo "conflict $V" >> "$LOG"
    printf 'value=A+C\n' > shared.txt
    echo "resolved"; exit 0 ;;
  *"티켓 T1 구현 담당"*) echo "write $V T1" >> "$LOG"; printf 'value=A\n' > shared.txt; exit 0 ;;
  *"티켓 T2 구현 담당"*) echo "write $V T2" >> "$LOG"; printf 't2\n' > t2.txt; exit 0 ;;
  *"티켓 T3 구현 담당"*) echo "write $V T3" >> "$LOG"; printf 'value=C\n' > shared.txt; exit 0 ;;
  *"티켓 T9 구현 담당"*) echo "write $V T9" >> "$LOG"; printf 'ok\n' > allowed.txt; printf 'x\n' > rogue.txt; exit 0 ;;
esac
echo "unknown $V" >> "$LOG"
echo "stub: unrecognized prompt" >&2
exit 3
"#;

/// stub 디렉터리를 만들고 PATH 맨 앞에 둔다. PATH는 프로세스 전역이라 한 번만 설정한다.
fn stub_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_root::dir().join("pipeline-e2e-stub-bin");
        std::fs::create_dir_all(&dir).unwrap();
        for v in VENDORS {
            let p = dir.join(v);
            std::fs::write(&p, STUB).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let old = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{old}", dir.display()));
        dir
    })
}

/// 두 테스트가 같은 calls.log를 쓰므로 직렬화한다.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static M: Mutex<()> = Mutex::new(());
    M.lock().unwrap_or_else(|e| e.into_inner())
}

fn calls() -> Vec<String> {
    std::fs::read_to_string(stub_dir().join("calls.log"))
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

fn available_vendors() -> Vec<String> {
    VENDORS
        .iter()
        .filter(|v| reviewer::which(v).is_some())
        .map(|v| v.to_string())
        .collect()
}

// ---------- git 픽스처 ----------

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").current_dir(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn init_repo(name: &str) -> PathBuf {
    let repo = temp_root::dir().join(name);
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "e2e"]);
    git(&repo, &["config", "user.email", "e2e@local"]);
    std::fs::write(repo.join("shared.txt"), "value=base\n").unwrap();
    std::fs::write(repo.join(".gitignore"), ".praxis/\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    repo
}

// ---------- 하네스: 드라이버 동작의 Tauri 비의존 재구성 ----------

struct H {
    pool: SqlitePool,
    run_id: i64,
    repo: PathBuf,
    reg: SharedProcessRegistrar,
    clock: AtomicI64,
    conflicts: Mutex<Vec<String>>,
}

static DB_N: AtomicU32 = AtomicU32::new(0);

impl H {
    async fn new(repo: PathBuf) -> H {
        stub_dir();
        let n = DB_N.fetch_add(1, Ordering::SeqCst);
        let path = temp_root::dir().join(format!("pipeline-e2e-{n}.sqlite"));
        let pool = db::init_pool(&path.to_string_lossy()).await.unwrap();
        multireview::migrate(&pool).await.unwrap();
        let run_id = pdb::insert_run(&pool, &repo.to_string_lossy(), "main", "e2e goal", 1).await.unwrap();
        H {
            pool,
            run_id,
            repo,
            reg: Arc::new(RunRegistrar::default()).registrar(),
            clock: AtomicI64::new(1_000),
            conflicts: Mutex::new(Vec::new()),
        }
    }

    fn now(&self) -> i64 {
        self.clock.fetch_add(1, Ordering::SeqCst)
    }

    async fn run(&self) -> pdb::RunRow {
        pdb::get_run(&self.pool, self.run_id).await.unwrap().unwrap()
    }

    async fn ticket(&self, id: i64) -> TicketRow {
        pdb::get_ticket(&self.pool, id).await.unwrap().unwrap()
    }

    async fn tickets(&self) -> Vec<TicketRow> {
        pdb::list_tickets(&self.pool, self.run_id).await.unwrap()
    }

    async fn by_key(&self, key: &str) -> TicketRow {
        self.tickets().await.into_iter().find(|t| t.key == key).unwrap()
    }

    fn integration(run: &pdb::RunRow) -> PathBuf {
        PathBuf::from(run.integration_path.clone().unwrap())
    }

    /// 드라이버 `begin`: 같은 step_key가 이미 succeeded면 캐시를 돌려준다.
    async fn begin(&self, t: Option<&TicketRow>, kind: &str, vendor: Option<&str>, token: u32, prompt: &str) -> StepStart {
        let key = model::step_key(self.run_id, kind, t.map(|t| t.key.as_str()), token);
        pdb::begin_step(&self.pool, self.run_id, t.map(|t| t.id), kind, vendor, &key, prompt, self.now())
            .await
            .unwrap()
    }

    async fn finish(&self, id: i64, ok: bool, out: &str) {
        pdb::finish_step(&self.pool, id, ok, out, self.now()).await.unwrap();
    }

    fn token(t: &TicketRow) -> u32 {
        (t.gen.max(0) as u32) * 100 + t.attempt.max(0) as u32
    }

    fn runner(&self) -> Arc<ReviewerFn> {
        let reg = self.reg.clone();
        Arc::new(move |v, p, timeout| reviewer::run_reviewer_registered(v, p, timeout, Some(&reg)))
    }

    /// 드라이버 `build_plan`: DB 스냅샷 → 순수 결정 함수.
    async fn next_action(&self) -> Action {
        let run = self.run().await;
        let rs = run.run_state().unwrap();
        let integration_task_state = match (rs, run.integration_task_id) {
            (RunState::AwaitingMergeApproval, Some(id)) => {
                db::get_task(&self.pool, id).await.unwrap().map(|t| state_key(&t.state))
            }
            _ => None,
        };
        let rctx = RunCtx {
            state: rs,
            has_integration: run.integration_task_id.is_some() && run.integration_path.is_some(),
            has_spec: run.spec_json.is_some(),
            has_feedback: run.feedback.as_deref().is_some_and(|f| !f.trim().is_empty()),
            integration_task_state,
        };
        let mut tickets = Vec::new();
        if rs == RunState::Executing {
            for t in self.tickets().await {
                let task_state = match t.task_id {
                    Some(id) => db::get_task(&self.pool, id).await.unwrap().map(|k| state_key(&k.state)),
                    None => None,
                };
                // stub 에이전트는 spawn 안에서 동기로 끝나므로 살아 있는 세션은 없다.
                tickets.push(TicketCtx { has_task: t.task_id.is_some(), agent_alive: false, task_state, view: t.view().unwrap() });
            }
        }
        plan::plan_next_action(&rctx, &tickets)
    }

    async fn ensure_integration(&self) {
        let run = self.run().await;
        let wt = worktree::create_plain(&self.repo, "dojang/pipe-e2e-1", Some("main")).unwrap();
        let task = TaskService::new(self.pool.clone())
            .create_task(task_draft(&run.repo, &wt, "claude", &run.goal), None, None, self.now())
            .await
            .unwrap();
        db::set_base_revision(&self.pool, task.id, wt.base_revision.as_deref().unwrap()).await.unwrap();
        pdb::set_run_integration(&self.pool, self.run_id, task.id, &wt.branch, &wt.path.to_string_lossy(), self.now())
            .await
            .unwrap();
    }

    /// 드라이버 `do_split`의 첫 시도. `commit=false`는 단계 기록 직후·commit_split 전에 죽은 상황을 흉내 낸다.
    /// 반환: 이번 호출에서 stub claude를 실제로 불렀는가.
    async fn split(&self, commit: bool) -> bool {
        let run = self.run().await;
        let path = Self::integration(&run);
        let avail = available_vendors();
        let refs: Vec<&str> = avail.iter().map(String::as_str).collect();
        let nonce = format!("PRAXIS-SPLIT-{:016x}", review_run::random_u64());
        let prompt = split::build_split_prompt(&run.goal, &run.base_branch, &refs, &nonce, None);
        let token = (run.spec_revision.max(0) as u32) * 10;
        let (out, invoked) = match self.begin(None, "split", Some("claude"), token, &prompt).await {
            StepStart::AlreadySucceeded(row) => (serde_json::from_str::<SplitOutput>(row.output.as_deref().unwrap()).unwrap(), false),
            StepStart::Started(id) => {
                let raw = reviewer::run_claude_readonly_in(&path, &prompt, 600, Some(&self.reg)).unwrap();
                assert_eq!(reviewer::worktree_is_clean(&path), Ok(true), "R4: read-only split must not dirty the worktree");
                let out = split::parse_split_output(&raw, &nonce).unwrap();
                model::validate_split(&out, &refs, MAX_TICKETS).unwrap();
                self.finish(id, true, &serde_json::to_string(&out).unwrap()).await;
                (out, true)
            }
        };
        if commit {
            pdb::commit_split(&self.pool, self.run_id, &serde_json::to_string(&out.spec).unwrap(), &out.tickets, self.now())
                .await
                .unwrap();
        }
        invoked
    }

    /// 드라이버 `assign_reviewers` + Drafting → PlanReview.
    async fn advance_to_plan_review(&self) {
        let avail = available_vendors();
        let refs: Vec<&str> = avail.iter().map(String::as_str).collect();
        let mut counts = pdb::reviewer_counts(&self.pool, self.run_id).await.unwrap();
        for t in self.tickets().await {
            let pick = model::pick_reviewer(&t.vendor, &refs, &counts).unwrap();
            *counts.entry(pick.clone()).or_insert(0) += 1;
            pdb::set_ticket_reviewer(&self.pool, t.id, Some(&pick), self.now()).await.unwrap();
        }
        assert!(pdb::set_run_state(&self.pool, self.run_id, RunState::Drafting, RunState::PlanReview, self.now()).await.unwrap());
    }

    /// 드라이버 `fanout_review`: 가용 벤더 전원 병렬 리뷰 + claude 종합 + reviews 행 저장.
    async fn fanout(&self, kind: ReviewKind, step: &str, focus: &str, content: &str) -> (i64, bool) {
        let vendors = available_vendors();
        let (prompt, _nonce, reviews) = review_run::run_parallel_reviews(kind, focus, content, &vendors, 300, self.runner());
        // 드라이버와 같은 허용치: 성공 2개 이상이면 종합한다. 실패는 기록만 하고(아래 별도 테스트 참고) 흐름은 계속한다.
        let ok: Vec<String> = reviews.iter().filter(|r| r.ok).map(|r| r.vendor.clone()).collect();
        for r in reviews.iter().filter(|r| !r.ok) {
            eprintln!("[e2e] {step}: {} review failed: {:?}", r.vendor, r.error);
        }
        assert!(ok.len() >= 2, "{step}: driver would pause with only {ok:?}");
        let (sp, _raw, synth) = review_run::synthesize(kind, focus, &reviews, self.runner(), 300).unwrap();
        assert!(synth.findings[0].sources.iter().all(|v| ok.contains(v)), "R7: sources limited to successful reviewers");
        let rec = review_run::build_review_record(&reviews, Some(&synth), &|_| String::new());
        let run = self.run().await;
        let id = multireview::insert_pipeline_review(
            &self.pool, &run.repo, "text", &format!("pipeline:{}:{step}", run.id), focus, &rec.result_json,
            rec.ok_count, rec.total, content, &prompt, Some(&sp), &rec.model_info_json, run.id, step, self.now(),
        )
        .await
        .unwrap();
        (id, synth.decision == SynthDecision::Fix)
    }

    fn plan_text(run: &pdb::RunRow, tickets: &[TicketRow]) -> String {
        let spec: Spec = serde_json::from_str(run.spec_json.as_deref().unwrap()).unwrap();
        split::render_spec_text(&SplitOutput { spec, tickets: tickets.iter().map(prompts::draft_from_row).collect() })
    }

    async fn plan_review(&self) {
        let run = self.run().await;
        let text = Self::plan_text(&run, &self.tickets().await);
        let StepStart::Started(sid) = self.begin(None, "plan_review", Some("claude"), run.spec_revision as u32, "계획 리뷰").await else {
            panic!("plan review unexpectedly cached");
        };
        let (rid, blocking) = self.fanout(ReviewKind::Plan, "plan_review", "목표: e2e", &text).await;
        assert!(!blocking);
        pdb::set_run_plan_review(&self.pool, run.id, rid, self.now()).await.unwrap();
        self.finish(sid, true, &format!("{{\"review_id\":{rid}}}")).await;
        assert!(pdb::set_run_state(&self.pool, run.id, RunState::PlanReview, RunState::AwaitingPlanApproval, self.now()).await.unwrap());
    }

    /// `spawn_tickets` 대체: 통합 브랜치 head에서 티켓 worktree·task를 만들고, PTY 에이전트 대신 같은 벤더의
    /// headless 실행(`run_repair_agent`, 실제 `agent::headless_args` 인자)으로 stub을 돌린 뒤 task를 검토 대기로 올린다.
    async fn spawn(&self, ids: Vec<i64>) {
        let run = self.run().await;
        let spec: Spec = serde_json::from_str(run.spec_json.as_deref().unwrap()).unwrap();
        let integration = Self::integration(&run);
        let branch = run.integration_branch.clone().unwrap();
        for id in ids {
            let t = self.ticket(id).await;
            let head = git_ops::head(&integration).unwrap();
            let wt = worktree::create_plain(&self.repo, &format!("dojang/pipe-e2e-ticket-{}", t.key.to_lowercase()), Some(&branch)).unwrap();
            assert_eq!(git_ops::head(&wt.path).unwrap(), head, "R9: ticket forks from integration head");
            let instruction = prompts::ticket_instruction(&spec, &prompts::draft_from_row(&t));
            let vendor = model::normalize_vendor(&t.vendor);
            let task = TaskService::new(self.pool.clone())
                .create_task(task_draft(&run.repo, &wt, &vendor, &instruction), None, None, self.now())
                .await
                .unwrap();
            pdb::set_ticket_task(&self.pool, t.id, Some(task.id), self.now()).await.unwrap();
            pdb::set_ticket_spawn_head(&self.pool, t.id, Some(&head), self.now()).await.unwrap();
            assert!(pdb::set_ticket_state(&self.pool, t.id, TicketState::Pending, TicketState::Running, self.now()).await.unwrap());
            reviewer::run_repair_agent(&wt.path, &vendor, None, None, &instruction, &self.reg).unwrap();
            assert!(db::mark_awaiting_review_with_notification(&self.pool, task.id, self.now(), None, "result").await.unwrap());
        }
    }

    async fn ticket_dir(&self, t: &TicketRow) -> (PathBuf, String) {
        let task = db::get_task(&self.pool, t.task_id.unwrap()).await.unwrap().unwrap();
        (PathBuf::from(task.worktree_path), task.branch)
    }

    async fn collect(&self, id: i64) {
        assert!(pdb::set_ticket_state(&self.pool, id, TicketState::Running, TicketState::Verifying, self.now()).await.unwrap());
    }

    /// 드라이버 `verify` + `verify_decision`: 허용 경로 검사 → 검증 명령. 반환은 통과 여부.
    async fn verify(&self, id: i64) -> bool {
        let t = self.ticket(id).await;
        let StepStart::Started(sid) = self.begin(Some(&t), "verify", None, Self::token(&t), "허용 경로 검사와 검증 명령").await else {
            panic!("verify unexpectedly cached");
        };
        let (dir, _) = self.ticket_dir(&t).await;
        let out = pipeline::steps::verify_ticket(
            &dir,
            t.spawn_head.as_deref().unwrap(),
            &t.allowed_paths(),
            &t.acceptance(),
            Some(&self.reg),
        )
        .unwrap();
        let (passed, summary) = (out.passed, out.summary);
        self.finish(sid, passed, &summary).await;
        let next = if passed { TicketState::Reviewing } else { TicketState::Fixing };
        pdb::set_ticket_error(&self.pool, id, (!passed).then_some(summary.as_str()), self.now()).await.unwrap();
        assert!(pdb::set_ticket_state(&self.pool, id, TicketState::Verifying, next, self.now()).await.unwrap());
        passed
    }

    /// 드라이버 `review_ticket`: 작성 벤더가 아닌 리뷰어 1명.
    async fn review(&self, id: i64) {
        let t = self.ticket(id).await;
        let reviewer_v = t.reviewer_vendor.clone().unwrap();
        assert_ne!(model::normalize_vendor(&reviewer_v), model::normalize_vendor(&t.vendor), "R12: reviewer != author");
        let (dir, _) = self.ticket_dir(&t).await;
        let diff = git_ops::diff_text(&dir, t.spawn_head.as_deref().unwrap()).unwrap();
        let content = format!("티켓 {}: {}\n{}\n\n{diff}", t.key, t.title, t.body);
        let focus = format!("티켓 {} 수용 기준 충족 여부", t.key);
        let StepStart::Started(sid) = self.begin(Some(&t), "ticket_review", Some(&reviewer_v), Self::token(&t) * 10, "티켓 리뷰").await else {
            panic!("ticket review unexpectedly cached");
        };
        let (prompt, vr) = review_run::single_review(ReviewKind::Ticket, &focus, &content, &reviewer_v, 300, self.runner());
        assert!(vr.ok, "ticket review failed: {:?}", vr.error);
        let rec = review_run::build_review_record(std::slice::from_ref(&vr), None, &|_| String::new());
        let run = self.run().await;
        let rid = multireview::insert_pipeline_review(
            &self.pool, &run.repo, "text", &format!("pipeline:{}:ticket_review:{}", run.id, t.key), &focus,
            &rec.result_json, rec.ok_count, rec.total, &content, &prompt, None, &rec.model_info_json, run.id,
            &format!("ticket_review:{}", t.key), self.now(),
        )
        .await
        .unwrap();
        assert!(!has_blocking(&vr.findings));
        self.finish(sid, true, &format!("{{\"review_id\":{rid}}}")).await;
        assert!(pdb::set_ticket_state(&self.pool, id, TicketState::Reviewing, TicketState::Ready, self.now()).await.unwrap());
    }

    /// 드라이버 `integrate`(재시작 복구가 아닌 경로): commit → merge --no-ff → 충돌이면 claude 해소 →
    /// 변경 경로가 허용 합집합 안인지 → 수용 명령 → 정리.
    async fn integrate(&self, id: i64) {
        let run = self.run().await;
        let spec: Spec = serde_json::from_str(run.spec_json.as_deref().unwrap()).unwrap();
        let integration = Self::integration(&run);
        let t = self.ticket(id).await;
        let (dir, branch) = self.ticket_dir(&t).await;
        let pre = git_ops::head(&integration).unwrap();
        assert!(pdb::set_ticket_state(&self.pool, id, TicketState::Ready, TicketState::Integrating, self.now()).await.unwrap());
        let StepStart::Started(sid) = self.begin(Some(&t), "integrate", None, Self::token(&t), &format!("pre:{pre}")).await else {
            panic!("integrate unexpectedly cached");
        };
        let msg = format!("통합: {} {}", t.key, t.title);
        git_ops::commit_all(&dir, &format!("{msg} (파이프라인 티켓)")).unwrap();
        match git_ops::merge_branch(&integration, &branch, &msg).unwrap() {
            git_ops::MergeOutcome::Conflict { paths } => {
                self.conflicts.lock().unwrap().extend(paths.iter().map(|p| format!("{}:{p}", t.key)));
                let prompt = prompts::conflict_prompt(&t.key, &branch, &paths, &spec);
                let StepStart::Started(cid) = self.begin(Some(&t), "conflict", Some("claude"), Self::token(&t), "merge 충돌 해소").await else {
                    panic!("conflict unexpectedly cached");
                };
                reviewer::run_repair_agent(&integration, "claude", None, None, &prompt, &self.reg).unwrap();
                let rev = git_ops::finish_merge_after_resolution(&integration).unwrap();
                self.finish(cid, true, &rev).await;
            }
            git_ops::MergeOutcome::Merged { .. } => {}
            git_ops::MergeOutcome::NothingToMerge => panic!("{}: nothing to merge", t.key),
        }
        let all = self.tickets().await;
        let mut allowed: Vec<String> = all
            .iter()
            .filter(|o| o.id == t.id || o.state == TicketState::Integrated.as_str())
            .flat_map(|o| o.allowed_paths())
            .collect();
        allowed.sort();
        allowed.dedup();
        let changed = git_ops::changed_paths(&integration, &pre).unwrap();
        assert!(model::paths_outside_allowed(&changed, &allowed).is_empty(), "{}: {changed:?} vs {allowed:?}", t.key);
        let outcomes = checks::run_ticket_checks(&integration, &t.acceptance(), Some(&self.reg));
        assert!(checks::checks_passed(&outcomes), "{}: {}", t.key, checks::failure_summary(&outcomes));
        let head = git_ops::head(&integration).unwrap();
        git_ops::reset_to(&integration, &head).unwrap();
        self.finish(sid, true, "통합 완료").await;
        assert!(pdb::set_ticket_state(&self.pool, id, TicketState::Integrating, TicketState::Integrated, self.now()).await.unwrap());
    }

    /// 드라이버 `do_final_review` + blocking이 없을 때의 `final_decision`(G2 대기로).
    async fn final_review(&self) {
        let run = self.run().await;
        let integration = Self::integration(&run);
        let task_id = run.integration_task_id.unwrap();
        let fork = db::get_task(&self.pool, task_id).await.unwrap().unwrap().base_revision.unwrap();
        let diff = git_ops::diff_text(&integration, &fork).unwrap();
        let content = format!("{}\n\n{diff}", Self::plan_text(&run, &self.tickets().await));
        let StepStart::Started(sid) = self.begin(None, "final_review", Some("claude"), run.auto_fix_used as u32, "최종 리뷰").await else {
            panic!("final review unexpectedly cached");
        };
        let (rid, blocking) = self.fanout(ReviewKind::Final, "final_review", "최종 통합 결과: e2e", &content).await;
        assert!(!blocking);
        pdb::set_run_final_review(&self.pool, run.id, rid, self.now()).await.unwrap();
        self.finish(sid, true, &format!("{{\"review_id\":{rid}}}")).await;
        git_ops::commit_all(&integration, "파이프라인 최종 정리").unwrap();
        assert!(db::mark_awaiting_review_with_notification(&self.pool, task_id, self.now(), None, "result").await.unwrap());
        assert!(pdb::set_run_state(&self.pool, run.id, RunState::FinalReview, RunState::AwaitingMergeApproval, self.now()).await.unwrap());
    }
}

fn state_key(s: &str) -> String {
    match s {
        db::state::AWAITING_REVIEW => "awaiting_review".into(),
        other => other.to_lowercase(),
    }
}

fn task_draft(repo: &str, wt: &worktree::Worktree, agent: &str, instruction: &str) -> TaskDraft {
    TaskDraft {
        repo: repo.to_string(),
        branch: wt.branch.clone(),
        base: wt.base.clone(),
        worktree_path: wt.path.to_string_lossy().into_owned(),
        instruction: instruction.to_string(),
        agent: Some(agent.to_string()),
        role: praxis_lib::agent::DEFAULT_ROLE.to_string(),
        ensemble: None,
        mode: "terminal".into(),
        goal_contract: None,
        ambiguity: None,
    }
}

async fn task_count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM tasks").fetch_one(pool).await.unwrap()
}

// ---------- 시나리오 ----------

/// 3티켓: T2는 T1에 의존, T1·T3는 독립이며 같은 줄을 고쳐 T3 통합에서 충돌한다.
///
/// 계획서 P10은 "2티켓·의존 1·충돌 1"이지만, R9(선행 티켓이 통합된 뒤 통합 head에서 spawn)가 지켜지는 한
/// 의존 티켓 B는 A의 결과 위에서 시작하므로 A와 충돌할 수 없다. 충돌은 독립 티켓 T3로 만든다.
#[tokio::test]
async fn pipeline_runs_split_to_merge_gate_with_dependency_and_conflict() {
    let _g = serial();
    let repo = init_repo("e2e-main");
    let h = H::new(repo.clone()).await;
    let base_rev = git(&repo, &["rev-parse", "main"]);

    // stub이 실제 벤더보다 앞에 잡힌다.
    for v in VENDORS {
        assert_eq!(reviewer::which(v).map(PathBuf::from), Some(stub_dir().join(v)));
    }
    let split_calls_before = calls().iter().filter(|l| l.starts_with("split")).count();

    let mut trace: Vec<Action> = Vec::new();
    let mut crashed_once = false;
    let mut spawn_heads: HashMap<String, String> = HashMap::new();
    for _ in 0..100 {
        let run = h.run().await;
        let rs = run.run_state().unwrap();
        if rs == RunState::AwaitingPlanApproval {
            // G1 전: 티켓 task 없음, base 불변(R8). 그 뒤 사용자 승인.
            assert_eq!(task_count(&h.pool).await, 1, "only the integration task exists before G1");
            assert!(h.tickets().await.iter().all(|t| t.task_id.is_none()));
            assert_eq!(git(&repo, &["rev-parse", "main"]), base_rev);
            assert!(pdb::approve_plan_cas(&h.pool, h.run_id, run.spec_revision, h.now()).await.unwrap());
            trace.push(Action::Idle);
            continue;
        }
        let action = h.next_action().await;
        if action == Action::Idle {
            break;
        }
        trace.push(action.clone());
        match action {
            Action::EnsureIntegration => h.ensure_integration().await,
            Action::Split => {
                if !crashed_once {
                    // R1: 분할 단계 기록 직후 죽은 것처럼 commit_split을 건너뛴다. 다음 틱은 캐시를 써야 한다.
                    crashed_once = true;
                    assert!(h.split(false).await, "first split call must invoke claude");
                } else {
                    assert!(!h.split(true).await, "R1: succeeded split step must not be re-invoked");
                }
            }
            Action::AdvanceToPlanReview => h.advance_to_plan_review().await,
            Action::PlanReview => h.plan_review().await,
            Action::Spawn(ids) => {
                h.spawn(ids.clone()).await;
                for id in ids {
                    let t = h.ticket(id).await;
                    spawn_heads.insert(t.key.clone(), t.spawn_head.clone().unwrap());
                }
            }
            Action::Collect(id) => h.collect(id).await,
            Action::Verify(id) => assert!(h.verify(id).await, "verify failed: {:?}", h.ticket(id).await.last_error),
            Action::Review(id) => h.review(id).await,
            Action::Integrate(id) => h.integrate(id).await,
            Action::ToFinalReview => {
                assert!(pdb::set_run_state(&h.pool, h.run_id, RunState::Executing, RunState::FinalReview, h.now()).await.unwrap())
            }
            Action::FinalReview => h.final_review().await,
            other => panic!("unexpected action {other:?}; run={:?}", h.run().await),
        }
    }

    // G2 대기 상태에 도달했고, 통합 작업이 승인 대상(AwaitingReview)이다.
    let run = h.run().await;
    assert_eq!(run.run_state(), Some(RunState::AwaitingMergeApproval));
    let itask = db::get_task(&h.pool, run.integration_task_id.unwrap()).await.unwrap().unwrap();
    assert_eq!(itask.state, db::state::AWAITING_REVIEW);
    assert_eq!(h.next_action().await, Action::Idle, "G2 waits for the user");

    // 결정 함수가 고른 순서: 독립 티켓 둘을 먼저, T2는 T1 통합 뒤에 spawn.
    let (t1, t2, t3) = (h.by_key("T1").await, h.by_key("T2").await, h.by_key("T3").await);
    let spawns: Vec<&Vec<i64>> = trace.iter().filter_map(|a| if let Action::Spawn(v) = a { Some(v) } else { None }).collect();
    assert_eq!(spawns, vec![&vec![t1.id, t3.id], &vec![t2.id]]);
    let pos = |a: &Action| trace.iter().position(|x| x == a).unwrap();
    assert!(pos(&Action::Integrate(t1.id)) < pos(&Action::Spawn(vec![t2.id])));
    assert_eq!(trace.iter().filter(|a| **a == Action::Split).count(), 2, "split ran twice: crash + resume");

    // 모든 티켓 통합. T3 통합에서 충돌이 났고 stub claude가 해소했다.
    for t in [&t1, &t2, &t3] {
        assert_eq!(t.state, TicketState::Integrated.as_str(), "{}", t.key);
    }
    assert_eq!(*h.conflicts.lock().unwrap(), vec!["T3:shared.txt".to_string()]);
    let integration = H::integration(&run);
    assert_eq!(std::fs::read_to_string(integration.join("shared.txt")).unwrap(), "value=A+C\n");
    assert_eq!(std::fs::read_to_string(integration.join("t2.txt")).unwrap(), "t2\n");
    assert!(reviewer::worktree_is_clean(&integration).unwrap());
    let merges = git(&integration, &["log", "--merges", "--format=%s", "main..HEAD"]);
    assert_eq!(merges.lines().count(), 3, "three --no-ff merges: {merges}");

    // R9: T2는 T1·T3 통합 뒤 통합 head에서 갈라졌다(T1·T3는 base에서).
    assert_eq!(spawn_heads["T1"], base_rev);
    assert_eq!(spawn_heads["T3"], base_rev);
    assert_ne!(spawn_heads["T2"], base_rev);
    let t2_fork_shared = git(&integration, &["show", &format!("{}:shared.txt", spawn_heads["T2"])]);
    assert_eq!(t2_fork_shared, "value=A+C");

    // 사용자 base 브랜치는 G2 전까지 그대로다(R8).
    assert_eq!(git(&repo, &["rev-parse", "main"]), base_rev);
    assert_eq!(std::fs::read_to_string(repo.join("shared.txt")).unwrap(), "value=base\n");

    // stub 호출 기록: 분할 1회(재개는 캐시), 리뷰는 3벤더×2(계획·최종)+티켓 3, 종합 2, 충돌 1.
    let log = calls();
    let count = |p: &str| log.iter().filter(|l| l.starts_with(p)).count();
    assert_eq!(count("split") - split_calls_before, 1);
    assert!(log.iter().any(|l| l == "conflict claude"));
    for (key, author) in [("T1", "codex"), ("T2", "agy"), ("T3", "claude")] {
        assert!(log.iter().any(|l| l == &format!("write {author} {key}")), "{key} written by {author}");
    }

    // 단계 기록: 전부 succeeded이고, 같은 키로 다시 시작하면 캐시가 돌아온다(R1).
    let steps = pdb::list_steps(&h.pool, h.run_id).await.unwrap();
    let kinds: Vec<&str> = steps.iter().map(|s| s.kind.as_str()).collect();
    for k in ["split", "plan_review", "verify", "ticket_review", "integrate", "conflict", "final_review"] {
        assert!(kinds.contains(&k), "missing step {k}: {kinds:?}");
    }
    assert_eq!(kinds.iter().filter(|k| **k == "integrate").count(), 3);
    for s in &steps {
        assert_eq!(s.status, "succeeded", "{}", s.step_key);
        let again = pdb::begin_step(&h.pool, h.run_id, s.ticket_id, &s.kind, s.vendor.as_deref(), &s.step_key, "retry", h.now())
            .await
            .unwrap();
        assert!(matches!(again, StepStart::AlreadySucceeded(_)), "{}", s.step_key);
    }
    let reviews: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reviews WHERE pipeline_run_id = ?")
        .bind(h.run_id)
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(reviews, 5, "plan + 3 ticket + final");
}

/// 허용 경로 밖 파일을 만든 티켓은 수용 명령이 통과해도 검증에서 거절되어 fixing으로 간다(R10).
#[tokio::test]
async fn ticket_writing_outside_allowed_paths_is_rejected_at_verify() {
    let _g = serial();
    let repo = init_repo("e2e-rogue");
    let h = H::new(repo.clone()).await;
    h.ensure_integration().await;
    let draft = TicketDraft {
        key: "T9".into(),
        title: "allowed only".into(),
        body: "write allowed.txt".into(),
        vendor: "codex".into(),
        vendor_reason: String::new(),
        acceptance_commands: vec!["test -f allowed.txt".into()],
        allowed_paths: vec!["allowed.txt".into()],
        deps: vec![],
        covers: vec!["R1".into()],
    };
    let spec = r#"{"goal":"g","requirements":[{"id":"R1","text":"t","acceptance":"a"}],"out_of_scope":[]}"#;
    pdb::commit_split(&h.pool, h.run_id, spec, &[draft], h.now()).await.unwrap();
    let t = h.by_key("T9").await;

    h.spawn(vec![t.id]).await;
    h.collect(t.id).await;
    assert!(!h.verify(t.id).await);

    let t = h.ticket(t.id).await;
    assert_eq!(t.state, TicketState::Fixing.as_str());
    let err = t.last_error.clone().unwrap_or_default();
    assert!(err.contains("rogue.txt") && !err.contains("allowed.txt"), "{err}");
    let step = pdb::list_steps(&h.pool, h.run_id).await.unwrap().into_iter().find(|s| s.kind == "verify").unwrap();
    assert_eq!(step.status, "failed");
    // 거절 사유는 경로뿐이다: 수용 명령 자체는 통과한다.
    let (dir, _) = h.ticket_dir(&t).await;
    assert!(checks::checks_passed(&checks::run_ticket_checks(&dir, &t.acceptance(), None)));
    // 실패 단계는 캐시되지 않아 다음 검증에서 다시 실행된다.
    let again = h.begin(Some(&t), "verify", None, H::token(&t), "retry").await;
    assert!(matches!(again, StepStart::Started(_)));
}

/// 제품 결함 재현: `reviewer::process::temporary_directory`가 `praxis-reviewer-{pid}-{nanos}`로 temp 디렉터리를
/// 만든다. macOS `SystemTime`은 마이크로초 해상도라 `run_parallel_reviews`처럼 같은 순간에 뜬 리뷰어들이 같은
/// 이름을 받아 `create_dir`이 `File exists`로 실패한다. e2e에서는 약 7회 중 2회 계획·최종 리뷰의 벤더 하나가
/// 이 오류로 빠졌다. 드라이버는 성공 2개 미만이면 실행을 멈추고, 티켓 리뷰는 다른 리뷰어로 넘어간다.
#[test]
fn concurrent_reviewers_get_distinct_temp_dirs() {
    let _g = serial();
    stub_dir();
    let n = 12;
    let barrier = std::sync::Barrier::new(n);
    let errors: Vec<String> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..n)
            .map(|_| {
                let b = &barrier;
                s.spawn(move || {
                    b.wait();
                    reviewer::run_reviewer("codex", "PRAXIS-REVIEW-1-abc", 30).err()
                })
            })
            .collect();
        hs.into_iter().filter_map(|h| h.join().unwrap()).collect()
    });
    assert!(errors.is_empty(), "{} of {n} concurrent reviewer calls failed: {errors:?}", errors.len());
}
