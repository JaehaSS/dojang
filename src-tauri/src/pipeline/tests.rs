use super::db::*;
use super::model::*;
use super::split::*;
use super::state::*;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::collections::HashMap;
use std::str::FromStr;

const VENDORS: &[&str] = &["claude", "codex", "agy"];

fn s(v: &str) -> String {
    v.to_string()
}
fn sv(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

// ---------- state ----------

#[test]
fn state_strings_round_trip() {
    for st in RunState::ALL {
        assert_eq!(RunState::from_str(st.as_str()).unwrap(), st);
        let j = serde_json::to_string(&st).unwrap();
        assert_eq!(j, format!("\"{}\"", st.as_str()));
    }
    for st in TicketState::ALL {
        assert_eq!(TicketState::from_str(st.as_str()).unwrap(), st);
    }
    assert!(RunState::from_str("nope").is_err());
    assert_eq!(
        RunState::AwaitingPlanApproval.as_str(),
        "awaiting_plan_approval"
    );
}

#[test]
fn run_transitions_follow_table() {
    use RunState::*;
    let allowed = [
        (Drafting, PlanReview),
        (Drafting, Failed),
        (PlanReview, AwaitingPlanApproval),
        (PlanReview, Drafting),
        (AwaitingPlanApproval, Executing),
        (AwaitingPlanApproval, Drafting),
        (Executing, FinalReview),
        (Executing, Paused),
        (FinalReview, AwaitingMergeApproval),
        (AwaitingMergeApproval, Done),
        (Paused, Executing),
        (Paused, FinalReview),
        (Paused, AwaitingMergeApproval),
        (Executing, Cancelled),
        (Paused, Cancelled),
        (AwaitingPlanApproval, Cancelled),
    ];
    for (f, t) in allowed {
        assert!(run_transition_allowed(f, t), "{f:?}->{t:?}");
    }
    let forbidden = [
        (Drafting, Executing),
        (Drafting, Done),
        (PlanReview, Executing),
        (AwaitingPlanApproval, FinalReview),
        (Executing, Done),
        (Executing, AwaitingMergeApproval),
        (FinalReview, Executing),
        (AwaitingMergeApproval, Executing),
        (Done, Drafting),
        (Done, Cancelled),
        (Cancelled, Drafting),
        (Failed, Drafting),
        (Failed, Cancelled),
    ];
    for (f, t) in forbidden {
        assert!(!run_transition_allowed(f, t), "{f:?}->{t:?}");
    }
    for t in RunState::ALL {
        assert!(!run_transition_allowed(Done, t));
    }
}

#[test]
fn ticket_transitions_follow_table() {
    use TicketState::*;
    let allowed = [
        (Pending, Running),
        (Running, Verifying),
        (Verifying, Reviewing),
        (Verifying, Fixing),
        (Verifying, Escalated),
        (Reviewing, Ready),
        (Reviewing, Fixing),
        (Fixing, Verifying),
        (Fixing, Running),
        (Fixing, Escalated),
        (Ready, Integrating),
        (Integrating, Integrated),
        (Integrating, Escalated),
        (Escalated, Pending),
        (Escalated, Running),
        (Escalated, Cancelled),
        (Running, Cancelled),
        (Ready, Cancelled),
    ];
    for (f, t) in allowed {
        assert!(ticket_transition_allowed(f, t), "{f:?}->{t:?}");
    }
    let forbidden = [
        (Pending, Verifying),
        (Pending, Integrated),
        (Running, Ready),
        (Verifying, Ready),
        (Reviewing, Integrating),
        (Ready, Integrated),
        (Integrating, Ready),
        (Integrated, Pending),
        (Integrated, Cancelled),
        (Cancelled, Pending),
    ];
    for (f, t) in forbidden {
        assert!(!ticket_transition_allowed(f, t), "{f:?}->{t:?}");
    }
}

// ---------- validate_split ----------

fn ticket(key: &str, deps: &[&str], covers: &[&str]) -> TicketDraft {
    TicketDraft {
        key: s(key),
        title: format!("title {key}"),
        body: String::new(),
        vendor: s("codex"),
        vendor_reason: s("r"),
        acceptance_commands: sv(&["npm test"]),
        allowed_paths: sv(&["src/"]),
        deps: sv(deps),
        covers: sv(covers),
    }
}

fn valid_split() -> SplitOutput {
    SplitOutput {
        spec: Spec {
            goal: s("g"),
            requirements: vec![
                Requirement {
                    id: s("R1"),
                    text: s("a"),
                    acceptance: s("x"),
                },
                Requirement {
                    id: s("R2"),
                    text: s("b"),
                    acceptance: s("y"),
                },
            ],
            out_of_scope: vec![],
        },
        tickets: vec![ticket("T1", &[], &["R1"]), ticket("T2", &["T1"], &["R2"])],
    }
}

fn errs_of(o: &SplitOutput) -> Vec<String> {
    validate_split(o, VENDORS, MAX_TICKETS).unwrap_err()
}
fn has(errs: &[String], needle: &str) -> bool {
    errs.iter().any(|e| e.contains(needle))
}

#[test]
fn validate_split_accepts_valid() {
    assert!(validate_split(&valid_split(), VENDORS, MAX_TICKETS).is_ok());
}

#[test]
fn validate_split_error_kinds() {
    let mut o = valid_split();
    o.spec.requirements.clear();
    o.tickets.clear();
    let e = errs_of(&o);
    assert!(has(&e, "요구사항이 하나도"));
    assert!(has(&e, "티켓이 하나도"));

    let mut o = valid_split();
    o.tickets = (0..9)
        .map(|i| ticket(&format!("T{i}"), &[], &["R1", "R2"]))
        .collect();
    assert!(has(&errs_of(&o), "상한"));

    let mut o = valid_split();
    o.tickets[1].key = s("T1");
    assert!(has(&errs_of(&o), "티켓 key 중복"));

    let mut o = valid_split();
    o.spec.requirements[1].id = s("R1");
    assert!(has(&errs_of(&o), "요구사항 id 중복"));

    let mut o = valid_split();
    o.tickets[0].vendor = s("gpt");
    assert!(has(&errs_of(&o), "사용할 수 없는 벤더"));

    let mut o = valid_split();
    o.tickets[1].deps = sv(&["T9"]);
    assert!(has(&errs_of(&o), "존재하지 않는 의존"));

    let mut o = valid_split();
    o.tickets[1].deps = sv(&["T2"]);
    assert!(has(&errs_of(&o), "자기 자신"));

    let mut o = valid_split();
    o.tickets[1].covers = sv(&[]);
    assert!(has(&errs_of(&o), "요구사항 R2를 담당"));

    let mut o = valid_split();
    o.tickets[1].covers = sv(&["R2", "R7"]);
    assert!(has(&errs_of(&o), "존재하지 않는 요구사항 'R7'"));

    let mut o = valid_split();
    o.tickets[0].allowed_paths = vec![];
    assert!(has(&errs_of(&o), "allowed_paths가 비어"));

    for bad in ["/etc/passwd", "../x", "a/../../b", "~/x"] {
        let mut o = valid_split();
        o.tickets[0].allowed_paths = sv(&[bad]);
        assert!(has(&errs_of(&o), "허용할 수 없는 경로"), "{bad}");
    }

    let mut o = valid_split();
    o.tickets[0].acceptance_commands = vec![];
    assert!(has(&errs_of(&o), "acceptance_commands가 비어"));

    let mut o = valid_split();
    o.tickets[0].acceptance_commands = sv(&["  "]);
    assert!(has(&errs_of(&o), "빈 acceptance_command"));

    let mut o = valid_split();
    o.tickets[0].title = s(" ");
    assert!(has(&errs_of(&o), "제목이 비어"));
}

#[test]
fn validate_split_detects_cycles_and_collects_all() {
    let mut o = valid_split();
    o.tickets[0].deps = sv(&["T2"]);
    let e = errs_of(&o);
    assert!(has(&e, "의존 순환"));

    let mut o = valid_split();
    o.tickets.push(ticket("T3", &["T2"], &[]));
    o.tickets[0].deps = sv(&["T3"]);
    assert!(has(&errs_of(&o), "의존 순환"));

    // 여러 문제를 한 번에 모은다
    let mut o = valid_split();
    o.tickets[0].vendor = s("zzz");
    o.tickets[1].allowed_paths = vec![];
    assert!(errs_of(&o).len() >= 2);
}

#[test]
fn gemini_is_alias_of_agy() {
    assert_eq!(normalize_vendor(" Gemini "), "agy");
    assert_eq!(normalize_vendor("codex"), "codex");
    let mut o = valid_split();
    o.tickets[0].vendor = s("gemini");
    assert!(validate_split(&o, VENDORS, MAX_TICKETS).is_ok());
}

// ---------- scheduling ----------

fn view(id: i64, key: &str, state: TicketState, deps: &[&str]) -> TicketView {
    TicketView {
        id,
        key: s(key),
        state,
        deps: sv(deps),
        vendor: s("codex"),
        reviewer_vendor: None,
        attempt: 0,
        reassigned: false,
    }
}

#[test]
fn ready_tickets_requires_integrated_deps_and_orders_by_key() {
    use TicketState::*;
    let v = vec![
        view(3, "T3", Pending, &["T1"]),
        view(2, "T2", Pending, &["T1", "T4"]),
        view(1, "T1", Integrated, &[]),
        view(4, "T4", Ready, &[]),
        view(5, "T0", Pending, &[]),
    ];
    assert_eq!(ready_tickets(&v), vec![5, 3]);
}

#[test]
fn integration_is_serial_and_ordered() {
    use TicketState::*;
    let v = vec![view(2, "T2", Ready, &[]), view(1, "T1", Ready, &[])];
    assert_eq!(next_integration(&v), Some(1));
    let v = vec![view(2, "T2", Ready, &[]), view(1, "T1", Integrating, &[])];
    assert_eq!(next_integration(&v), None);
    assert_eq!(next_integration(&[view(1, "T1", Running, &[])]), None);
}

#[test]
fn all_integrated_rules() {
    use TicketState::*;
    assert!(all_integrated(&[
        view(1, "T1", Integrated, &[]),
        view(2, "T2", Cancelled, &[])
    ]));
    assert!(!all_integrated(&[
        view(1, "T1", Integrated, &[]),
        view(2, "T2", Escalated, &[])
    ]));
    assert!(!all_integrated(&[view(1, "T1", Cancelled, &[])]));
    assert!(!all_integrated(&[]));
}

#[test]
fn fix_decision_sequence() {
    assert_eq!(fix_decision(0, false), FixDecision::RetrySameVendor);
    assert_eq!(fix_decision(1, false), FixDecision::RetrySameVendor);
    assert_eq!(fix_decision(2, false), FixDecision::Reassign);
    assert_eq!(fix_decision(0, true), FixDecision::RetrySameVendor);
    assert_eq!(fix_decision(1, true), FixDecision::RetrySameVendor);
    assert_eq!(fix_decision(2, true), FixDecision::Escalate);
}

#[test]
fn pick_reviewer_excludes_author_and_balances() {
    let mut c = HashMap::new();
    assert_eq!(
        pick_reviewer("codex", VENDORS, &c).as_deref(),
        Some("claude")
    );
    c.insert(s("claude"), 2);
    assert_eq!(pick_reviewer("codex", VENDORS, &c).as_deref(), Some("agy"));
    c.insert(s("agy"), 2);
    assert_eq!(
        pick_reviewer("codex", VENDORS, &c).as_deref(),
        Some("claude")
    );
    assert_eq!(pick_reviewer("claude", &["claude"], &c), None);
    assert_eq!(
        pick_reviewer("gemini", &["agy", "codex"], &HashMap::new()).as_deref(),
        Some("codex")
    );
    assert_eq!(
        pick_reassign_vendor("codex", VENDORS, &HashMap::new()).as_deref(),
        Some("claude")
    );
    assert_eq!(
        pick_reassign_vendor("codex", &["codex"], &HashMap::new()),
        None
    );
}

// ---------- paths ----------

#[test]
fn paths_outside_allowed_cases() {
    let out = |c: &[&str], a: &[&str]| paths_outside_allowed(&sv(c), &sv(a));
    assert!(out(&["src/a.ts"], &["src/a.ts"]).is_empty());
    assert_eq!(out(&["src/b.ts"], &["src/a.ts"]), sv(&["src/b.ts"]));
    // 디렉터리 접두
    assert!(out(&["src/x/y.ts"], &["src/"]).is_empty());
    assert!(out(&["src/x/y.ts"], &["src"]).is_empty());
    assert_eq!(out(&["srcfoo/y.ts"], &["src"]), sv(&["srcfoo/y.ts"]));
    assert!(out(&["./src/a.ts"], &["./src/"]).is_empty());
    // glob
    assert!(out(&["src/a.ts"], &["src/*.ts"]).is_empty());
    assert_eq!(out(&["src/x/a.ts"], &["src/*.ts"]), sv(&["src/x/a.ts"]));
    assert!(out(&["src/x/y/a.ts"], &["src/**/*.ts"]).is_empty());
    assert!(out(&["src/a.ts"], &["src/**/*.ts"]).is_empty());
    assert!(out(&["a/b/c.rs"], &["**/*.rs"]).is_empty());
    assert!(out(&["c.rs"], &["**/*.rs"]).is_empty());
    assert_eq!(out(&["src/a.rs"], &["src/*.ts"]), sv(&["src/a.rs"]));
    // .. 와 절대 경로는 항상 밖
    assert_eq!(out(&["src/../etc/x"], &["src/"]), sv(&["src/../etc/x"]));
    assert_eq!(out(&["/abs/x"], &["/abs/"]), sv(&["/abs/x"]));
    assert_eq!(out(&["a"], &[]), sv(&["a"]));
}

#[test]
fn step_key_format() {
    assert_eq!(step_key(7, "split", None, 0), "run:7:split:-:0");
    assert_eq!(step_key(7, "fix", Some("T2"), 3), "run:7:fix:T2:3");
}

// ---------- split ----------

#[test]
fn split_prompt_contains_contract() {
    let p = build_split_prompt("로그인 추가", "main", VENDORS, "NONCE123", None);
    assert!(p.contains("NONCE123"));
    assert!(p.contains("로그인 추가"));
    assert!(p.contains("최대 8개"));
    assert!(p.contains("main"));
    assert!(!p.contains("=== 이전 안에 대한 지적"));
    let p2 = build_split_prompt("g", "main", VENDORS, "N", Some("T1이 너무 크다"));
    assert!(p2.contains("=== 이전 안에 대한 지적"));
    assert!(p2.contains("T1이 너무 크다"));
}

const SPLIT_JSON: &str = r#"{"spec":{"goal":"g","requirements":[{"id":"R1","text":"t","acceptance":"a"}],"out_of_scope":[]},"tickets":[{"key":"T1","title":"x","vendor":"Gemini","acceptance_commands":["true"],"allowed_paths":["src/"],"covers":["R1"]}]}"#;

#[test]
fn parse_split_output_cases() {
    let raw = format!("{{\"spec\":{{}}}} 앞 JSON\nNN\n{SPLIT_JSON}");
    let out = parse_split_output(&raw, "NN").unwrap();
    assert_eq!(out.tickets[0].vendor, "agy");
    assert_eq!(out.spec.requirements.len(), 1);
    assert!(out.tickets[0].deps.is_empty());

    assert!(parse_split_output(SPLIT_JSON, "NN")
        .unwrap_err()
        .contains("nonce"));
    assert!(parse_split_output("NN\n{not json", "NN").is_err());
    assert!(parse_split_output("NN\n{\"tickets\": 3}", "NN").is_err());
    assert!(parse_split_output("NN no json", "NN").is_err());
}

#[test]
fn render_spec_text_is_deterministic() {
    let o = valid_split();
    let a = render_spec_text(&o);
    assert_eq!(a, render_spec_text(&o));
    assert!(a.contains("R1"));
    assert!(a.contains("## T2: title T2"));
    assert!(a.contains("선행 티켓: T1"));
}

// ---------- db ----------

async fn pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    migrate(&pool).await.unwrap();
    migrate(&pool).await.unwrap(); // 멱등
    pool
}

#[tokio::test]
async fn run_round_trip_and_cas() {
    let p = pool().await;
    let id = insert_run(&p, "/repo", "main", "goal", 10).await.unwrap();
    let r = get_run(&p, id).await.unwrap().unwrap();
    assert_eq!(r.run_state(), Some(RunState::Drafting));
    assert_eq!(r.spec_revision, 0);

    assert_eq!(
        active_run_for_repo(&p, "/repo").await.unwrap().unwrap().id,
        id
    );
    assert!(active_run_for_repo(&p, "/other").await.unwrap().is_none());

    // 잘못된 expected: false, 상태 불변
    assert!(!set_run_state(
        &p,
        id,
        RunState::PlanReview,
        RunState::AwaitingPlanApproval,
        11
    )
    .await
    .unwrap());
    // 금지 전이: Err
    assert!(
        set_run_state(&p, id, RunState::Drafting, RunState::Done, 11)
            .await
            .is_err()
    );
    assert_eq!(get_run(&p, id).await.unwrap().unwrap().state, "drafting");

    assert!(
        set_run_state(&p, id, RunState::Drafting, RunState::PlanReview, 12)
            .await
            .unwrap()
    );
    set_run_spec(&p, id, "{}", 13).await.unwrap();
    set_run_spec(&p, id, "{\"a\":1}", 14).await.unwrap();
    set_run_integration(&p, id, 5, "br", "/wt", 15)
        .await
        .unwrap();
    set_run_plan_review(&p, id, 3, 15).await.unwrap();
    set_run_final_review(&p, id, 4, 15).await.unwrap();
    set_run_auto_fix_used(&p, id, true, 15).await.unwrap();
    set_run_plan_fix_used(&p, id, true, 15).await.unwrap();
    set_run_error(&p, id, Some("boom"), 16).await.unwrap();
    let r = get_run(&p, id).await.unwrap().unwrap();
    assert_eq!(r.spec_revision, 2);
    assert_eq!(r.spec_json.as_deref(), Some("{\"a\":1}"));
    assert_eq!(r.integration_task_id, Some(5));
    assert_eq!(r.integration_path.as_deref(), Some("/wt"));
    assert_eq!((r.plan_review_id, r.final_review_id), (Some(3), Some(4)));
    assert_eq!((r.auto_fix_used, r.plan_fix_used), (1, 1));
    assert_eq!(r.last_error.as_deref(), Some("boom"));

    assert_eq!(list_runs(&p, Some("/repo")).await.unwrap().len(), 1);
    assert_eq!(list_runs(&p, None).await.unwrap().len(), 1);
    assert_eq!(list_active_runs(&p).await.unwrap().len(), 1);

    assert!(
        set_run_state(&p, id, RunState::PlanReview, RunState::Cancelled, 20)
            .await
            .unwrap()
    );
    assert!(active_run_for_repo(&p, "/repo").await.unwrap().is_none());
    assert!(list_active_runs(&p).await.unwrap().is_empty());
}

#[tokio::test]
async fn pause_and_resume_returns_to_origin() {
    let p = pool().await;
    let id = insert_run(&p, "/repo", "main", "g", 1).await.unwrap();
    assert!(
        set_run_state(&p, id, RunState::Drafting, RunState::PlanReview, 2)
            .await
            .unwrap()
    );
    assert!(!pause_run(&p, id, RunState::Drafting, "x", 3).await.unwrap());
    assert!(pause_run(&p, id, RunState::PlanReview, "에스컬레이션", 3)
        .await
        .unwrap());
    let r = get_run(&p, id).await.unwrap().unwrap();
    assert_eq!(r.state, "paused");
    assert_eq!(r.paused_from.as_deref(), Some("plan_review"));
    assert_eq!(r.paused_reason.as_deref(), Some("에스컬레이션"));
    assert_eq!(
        active_run_for_repo(&p, "/repo").await.unwrap().unwrap().id,
        id
    );

    assert_eq!(
        resume_run(&p, id, 4).await.unwrap(),
        Some(RunState::PlanReview)
    );
    let r = get_run(&p, id).await.unwrap().unwrap();
    assert_eq!(r.state, "plan_review");
    assert!(r.paused_from.is_none() && r.paused_reason.is_none());
    // Paused가 아니면 재개 불가
    assert_eq!(resume_run(&p, id, 5).await.unwrap(), None);
    assert_eq!(resume_run(&p, 999, 5).await.unwrap(), None);
}

#[tokio::test]
async fn tickets_replace_cas_and_views() {
    let p = pool().await;
    let run = insert_run(&p, "/repo", "main", "g", 1).await.unwrap();
    let o = valid_split();
    replace_tickets(&p, run, &o.tickets, 2).await.unwrap();
    replace_tickets(&p, run, &o.tickets, 3).await.unwrap(); // 교체이므로 UNIQUE 충돌 없음
    let ts = list_tickets(&p, run).await.unwrap();
    assert_eq!(ts.len(), 2);
    assert_eq!(ts[0].key, "T1");
    assert_eq!(ts[1].deps(), vec!["T1"]);
    assert_eq!(ts[0].acceptance(), vec!["npm test"]);
    assert_eq!(ts[0].allowed_paths(), vec!["src/"]);
    assert_eq!(ts[0].ticket_state(), Some(TicketState::Pending));
    let views: Vec<_> = ts.iter().filter_map(|t| t.view()).collect();
    assert_eq!(ready_tickets(&views), vec![ts[0].id]);

    let t1 = ts[0].id;
    assert!(
        !set_ticket_state(&p, t1, TicketState::Running, TicketState::Verifying, 4)
            .await
            .unwrap()
    );
    assert!(
        set_ticket_state(&p, t1, TicketState::Pending, TicketState::Verifying, 4)
            .await
            .is_err()
    );
    assert!(
        set_ticket_state(&p, t1, TicketState::Pending, TicketState::Running, 4)
            .await
            .unwrap()
    );
    set_ticket_task(&p, t1, Some(42), 5).await.unwrap();
    set_ticket_reviewer(&p, t1, Some("claude"), 5)
        .await
        .unwrap();
    set_ticket_vendor(&p, t1, "claude", true, 0, 5)
        .await
        .unwrap();
    set_ticket_attempt(&p, t1, 2, 5).await.unwrap();
    set_ticket_error(&p, t1, Some("e"), 5).await.unwrap();
    let t = get_ticket(&p, t1).await.unwrap().unwrap();
    assert_eq!(t.task_id, Some(42));
    assert_eq!(t.reviewer_vendor.as_deref(), Some("claude"));
    assert_eq!(
        (t.vendor.as_str(), t.reassigned, t.attempt),
        ("claude", 1, 2)
    );
    assert_eq!(t.last_error.as_deref(), Some("e"));
    assert_eq!(
        reviewer_counts(&p, run).await.unwrap().get("claude"),
        Some(&1)
    );
}

#[tokio::test]
async fn begin_step_is_idempotent() {
    let p = pool().await;
    let run = insert_run(&p, "/repo", "main", "g", 1).await.unwrap();
    let key = step_key(run, "split", None, 0);
    let StepStart::Started(id) =
        begin_step(&p, run, None, "split", Some("claude"), &key, "prompt", 2)
            .await
            .unwrap()
    else {
        panic!("첫 시작이어야 한다");
    };
    // running 상태 재시작: 같은 행 재사용
    let StepStart::Started(id2) =
        begin_step(&p, run, None, "split", Some("claude"), &key, "prompt2", 3)
            .await
            .unwrap()
    else {
        panic!("running은 재사용");
    };
    assert_eq!(id, id2);
    finish_step(&p, id, false, "err", 4).await.unwrap();
    let StepStart::Started(id3) = begin_step(&p, run, None, "split", Some("claude"), &key, "p3", 5)
        .await
        .unwrap()
    else {
        panic!("failed는 재사용");
    };
    assert_eq!(id, id3);
    finish_step(&p, id, true, "ok", 6).await.unwrap();
    match begin_step(&p, run, None, "split", Some("claude"), &key, "p4", 7)
        .await
        .unwrap()
    {
        StepStart::AlreadySucceeded(row) => {
            assert_eq!(row.output.as_deref(), Some("ok"));
            assert_eq!(row.status, "succeeded");
        }
        other => panic!("{other:?}"),
    }
    let steps = list_steps(&p, run).await.unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].prompt.as_deref(), Some("p3"));
}

// ---------- 드라이버용 DB 연산 ----------

#[tokio::test]
async fn commit_split_is_atomic_and_consumes_feedback() {
    let p = pool().await;
    let run = insert_run(&p, "/repo", "main", "g", 1).await.unwrap();
    set_run_feedback(&p, run, Some("고쳐라"), 2).await.unwrap();
    let o = valid_split();
    commit_split(&p, run, "{\"spec\":1}", &o.tickets, 3).await.unwrap();
    let r = get_run(&p, run).await.unwrap().unwrap();
    assert_eq!(r.spec_revision, 1);
    assert_eq!(r.feedback, None);
    assert_eq!(r.spec_json.as_deref(), Some("{\"spec\":1}"));
    assert_eq!(list_tickets(&p, run).await.unwrap().len(), 2);
    commit_split(&p, run, "{\"spec\":2}", &o.tickets[..1], 4).await.unwrap();
    let r = get_run(&p, run).await.unwrap().unwrap();
    assert_eq!(r.spec_revision, 2);
    assert_eq!(list_tickets(&p, run).await.unwrap().len(), 1);
}

#[tokio::test]
async fn update_tickets_and_approve_are_revision_cas() {
    let p = pool().await;
    let run = insert_run(&p, "/repo", "main", "g", 1).await.unwrap();
    let o = valid_split();
    commit_split(&p, run, "{}", &o.tickets, 2).await.unwrap();
    // drafting 상태에서는 편집·승인 모두 거절
    assert!(!update_tickets_cas(&p, run, 1, &o.tickets, 3).await.unwrap());
    assert!(!approve_plan_cas(&p, run, 1, 3).await.unwrap());
    assert!(set_run_state(&p, run, RunState::Drafting, RunState::PlanReview, 3).await.unwrap());
    assert!(set_run_state(&p, run, RunState::PlanReview, RunState::AwaitingPlanApproval, 3).await.unwrap());
    assert!(!update_tickets_cas(&p, run, 0, &o.tickets, 4).await.unwrap(), "옛 revision");
    assert!(update_tickets_cas(&p, run, 1, &o.tickets[..1], 4).await.unwrap());
    assert_eq!(get_run(&p, run).await.unwrap().unwrap().spec_revision, 2);
    assert!(!approve_plan_cas(&p, run, 1, 5).await.unwrap(), "승인은 최신 revision만");
    assert!(approve_plan_cas(&p, run, 2, 5).await.unwrap());
    assert_eq!(get_run(&p, run).await.unwrap().unwrap().state, "executing");
}

#[tokio::test]
async fn reject_plan_returns_to_drafting_with_feedback() {
    let p = pool().await;
    let run = insert_run(&p, "/repo", "main", "g", 1).await.unwrap();
    assert!(!reject_plan(&p, run, "x", 2).await.unwrap());
    set_run_state(&p, run, RunState::Drafting, RunState::PlanReview, 2).await.unwrap();
    set_run_state(&p, run, RunState::PlanReview, RunState::AwaitingPlanApproval, 2).await.unwrap();
    assert!(reject_plan(&p, run, "티켓을 줄여라", 3).await.unwrap());
    let r = get_run(&p, run).await.unwrap().unwrap();
    assert_eq!(r.state, "drafting");
    assert_eq!(r.feedback.as_deref(), Some("티켓을 줄여라"));
}

#[tokio::test]
async fn retry_and_skip_ticket_follow_escalation() {
    let p = pool().await;
    let run = insert_run(&p, "/repo", "main", "g", 1).await.unwrap();
    let o = valid_split();
    commit_split(&p, run, "{}", &o.tickets, 2).await.unwrap();
    let ts = list_tickets(&p, run).await.unwrap();
    let (t1, t2) = (ts[0].id, ts[1].id);
    assert!(!retry_ticket(&p, t1, None, 3).await.unwrap(), "escalated가 아니면 거절");
    assert!(skip_ticket_cascade(&p, run, t1, 3).await.unwrap().is_empty());

    set_ticket_state(&p, t1, TicketState::Pending, TicketState::Running, 3).await.unwrap();
    set_ticket_state(&p, t1, TicketState::Running, TicketState::Verifying, 3).await.unwrap();
    set_ticket_state(&p, t1, TicketState::Verifying, TicketState::Escalated, 3).await.unwrap();
    set_ticket_vendor(&p, t1, "codex", true, 2, 3).await.unwrap();
    set_ticket_error(&p, t1, Some("boom"), 3).await.unwrap();
    set_ticket_task(&p, t1, Some(9), 3).await.unwrap();

    assert!(retry_ticket(&p, t1, Some("agy"), 4).await.unwrap());
    let t = get_ticket(&p, t1).await.unwrap().unwrap();
    assert_eq!((t.state.as_str(), t.vendor.as_str()), ("pending", "agy"));
    assert_eq!((t.attempt, t.reassigned, t.gen, t.task_id), (0, 0, 1, None));
    assert_eq!(t.last_error, None);

    // 다시 에스컬레이션 후 skip: T2는 T1에 의존하므로 함께 취소된다.
    set_ticket_state(&p, t1, TicketState::Pending, TicketState::Running, 5).await.unwrap();
    set_ticket_state(&p, t1, TicketState::Running, TicketState::Verifying, 5).await.unwrap();
    set_ticket_state(&p, t1, TicketState::Verifying, TicketState::Escalated, 5).await.unwrap();
    let cancelled = skip_ticket_cascade(&p, run, t1, 6).await.unwrap();
    assert_eq!(cancelled, vec![t1, t2]);
    for t in list_tickets(&p, run).await.unwrap() {
        assert_eq!(t.state, "cancelled");
    }
}

#[tokio::test]
async fn cancel_run_is_idempotent_for_terminal() {
    let p = pool().await;
    let run = insert_run(&p, "/repo", "main", "g", 1).await.unwrap();
    assert!(cancel_run(&p, run, 2).await.unwrap());
    assert!(!cancel_run(&p, run, 3).await.unwrap());
    assert_eq!(get_run(&p, run).await.unwrap().unwrap().state, "cancelled");
}

#[tokio::test]
async fn guard_blocks_pipeline_managed_tasks_until_g2() {
    let p = pool().await;
    let run = insert_run(&p, "/repo", "main", "g", 1).await.unwrap();
    let o = valid_split();
    replace_tickets(&p, run, &o.tickets, 2).await.unwrap();
    let t = list_tickets(&p, run).await.unwrap().remove(0);
    set_ticket_task(&p, t.id, Some(101), 3).await.unwrap();
    assert!(claim_run_integration(&p, run, 100, "br", "/wt", 4).await.unwrap());
    // 이미 연결됐으면 다시 연결하지 않는다.
    assert!(!claim_run_integration(&p, run, 999, "br2", "/wt2", 5).await.unwrap());

    // 일반 작업은 건드리지 않는다.
    assert_eq!(pipeline_guard_for_task(&p, 555).await.unwrap(), None);
    // 진행 중: 티켓·통합 작업 모두 막힌다.
    assert_eq!(pipeline_guard_for_task(&p, 101).await.unwrap().as_deref(), Some(PIPELINE_MANAGED_MESSAGE));
    assert_eq!(pipeline_guard_for_task(&p, 100).await.unwrap().as_deref(), Some(PIPELINE_MANAGED_MESSAGE));

    // 최종 승인 대기: 통합 작업만 풀린다.
    sqlx::query("UPDATE pipeline_runs SET state = 'awaiting_merge_approval' WHERE id = ?")
        .bind(run)
        .execute(&p)
        .await
        .unwrap();
    assert_eq!(pipeline_guard_for_task(&p, 100).await.unwrap(), None);
    assert!(pipeline_guard_for_task(&p, 101).await.unwrap().is_some());

    // 종결된 실행은 정리할 수 있도록 모두 풀린다.
    for end in ["done", "cancelled", "failed"] {
        sqlx::query("UPDATE pipeline_runs SET state = ? WHERE id = ?").bind(end).bind(run).execute(&p).await.unwrap();
        assert_eq!(pipeline_guard_for_task(&p, 101).await.unwrap(), None, "{end}");
        assert_eq!(pipeline_guard_for_task(&p, 100).await.unwrap(), None, "{end}");
    }
}

#[tokio::test]
async fn claim_integration_refuses_cancelled_run() {
    let p = pool().await;
    let run = insert_run(&p, "/repo", "main", "g", 1).await.unwrap();
    assert!(cancel_run(&p, run, 2).await.unwrap());
    assert!(!claim_run_integration(&p, run, 100, "br", "/wt", 3).await.unwrap());
    assert_eq!(get_run(&p, run).await.unwrap().unwrap().integration_task_id, None);
}
