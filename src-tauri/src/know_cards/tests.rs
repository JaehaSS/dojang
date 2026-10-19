use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static DB_COUNTER: AtomicUsize = AtomicUsize::new(0);

async fn test_pool() -> SqlitePool {
    let n = DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = crate::testtmp::dir().join(format!("praxis-know-cards-{}-{n}.sqlite", std::process::id()));
    let _ = std::fs::remove_file(&path);
    crate::db::init_pool(path.to_str().unwrap()).await.unwrap()
}

async fn task(pool: &SqlitePool) -> i64 {
    crate::db::insert_task(pool, "/repo", "b", "main", "/wt", "x", Some("claude"), None, "conversation", 1)
        .await
        .unwrap()
}

fn tool(name: &str, summary: &str, parent: Option<&str>) -> ConvoEvent {
    ConvoEvent::ToolUse {
        name: name.into(),
        summary: summary.into(),
        tool_id: None,
        parent_id: parent.map(str::to_string),
    }
}

fn input(tools: usize) -> JudgeInput {
    let mut digest = TurnDigest::default();
    for i in 0..tools {
        digest.observe(&tool("Edit", &format!("src/auth/file{i}.ts"), None));
    }
    JudgeInput {
        request: "  \n로그인 오류 고쳐줘\n세션이 자꾸 끊긴다".into(),
        answer: "토큰 만료를 1시간으로 바꿨습니다.".into(),
        digest,
    }
}

fn gate(vendor: Vendor, tools: usize) -> TurnGate {
    TurnGate { debate: false, vendor, result_seen: true, result_error: false, main_tool_uses: tools }
}

fn nonce_in(args: &[String]) -> String {
    let system = &args[args.iter().position(|a| a == "--system-prompt").unwrap() + 1];
    let at = system.find(NONCE_PREFIX).unwrap();
    system[at..at + NONCE_PREFIX.len() + 1 + 32].to_string()
}

fn card_json(topic: &str) -> String {
    serde_json::json!({
        "show": true,
        "kind": "heads_up",
        "topic": topic,
        "title": "세션 토큰 만료가 1시간으로 줄었습니다",
        "body": "refresh 흐름이 없어 1시간 뒤 다시 로그인해야 합니다.",
        "evidence": "Edit   src/auth/file0.ts",
    })
    .to_string()
}

/// 판정 모델 대역 — nonce 한 줄 뒤에 주어진 JSON을 낸다.
fn replies(json: String) -> impl FnOnce(Vec<String>) -> std::future::Ready<Result<String, String>> {
    move |args| std::future::ready(Ok(format!("생각 중…\n{}\n{json}", nonce_in(&args))))
}

fn material(input: &JudgeInput) -> Material {
    Material::from(input)
}

async fn outcomes(pool: &SqlitePool, task_id: i64) -> Vec<String> {
    sqlx::query_scalar("SELECT outcome FROM know_card_checks WHERE task_id = ? ORDER BY id")
        .bind(task_id)
        .fetch_all(pool)
        .await
        .unwrap()
}

// A1 — 조건을 모두 만족할 때만 판정한다.
#[test]
fn only_a_clean_tool_heavy_single_seat_turn_qualifies() {
    let cases = [
        ("claude 6회", gate(Vendor::Claude, 6), true),
        ("codex 6회", gate(Vendor::Codex, 6), true),
        ("메인 5회", gate(Vendor::Claude, 5), false),
        ("agy", gate(Vendor::Agy, 12), false),
        ("토론", TurnGate { debate: true, ..gate(Vendor::Claude, 12) }, false),
        ("중단·타임아웃", TurnGate { result_seen: false, ..gate(Vendor::Claude, 12) }, false),
        ("오류 턴", TurnGate { result_error: true, ..gate(Vendor::Claude, 12) }, false),
    ];
    for (label, gate, expected) in cases {
        assert_eq!(turn_qualifies(&gate), expected, "{label}");
    }
}

#[test]
fn subagent_calls_are_not_counted() {
    let mut digest = TurnDigest::default();
    digest.observe(&tool("Task", "조사", None));
    for _ in 0..30 {
        digest.observe(&tool("Read", "a.rs", Some("toolu_1")));
    }
    digest.observe(&ConvoEvent::ToolResult {
        summary: "서브 실패".into(),
        is_error: true,
        result_chars: None,
        tool_use_id: None,
        parent_id: Some("toolu_1".into()),
    });
    assert_eq!(digest.main_tool_uses(), 1);
    assert!(digest.failed.is_empty());
}

#[tokio::test]
async fn disabled_setting_skips_without_a_row() {
    let pool = test_pool().await;
    let id = task(&pool).await;
    assert!(enabled(&pool).await);
    set_enabled(&pool, false).await.unwrap();
    assert!(!enabled(&pool).await);
    let called = AtomicBool::new(false);
    let outcome = check(&pool, &Flights::default(), id, input(8), 10, |_| {
        called.store(true, Ordering::SeqCst);
        std::future::ready(Ok(String::new()))
    })
    .await;
    assert_eq!(outcome, None);
    assert!(!called.load(Ordering::SeqCst));
    assert!(outcomes(&pool, id).await.is_empty());
}

#[test]
fn digest_keeps_head_and_tail_and_caps_failures() {
    let mut digest = TurnDigest::default();
    for i in 0..50 {
        digest.observe(&tool("Bash", &format!("step {i}"), None));
    }
    for i in 0..15 {
        digest.observe(&ConvoEvent::ToolResult {
            summary: format!("실패 {i} {}", "x".repeat(300)),
            is_error: true,
            result_chars: None,
            tool_use_id: None,
            parent_id: None,
        });
    }
    let input = JudgeInput { request: "r".into(), answer: "a".into(), digest };
    let text = material(&input).render(input.digest.main_tool_uses());
    assert!(text.contains("도구 호출 (50회)"));
    assert!(text.contains("Bash step 19\n(… 10건 생략 …)\nBash step 30"));
    assert!(text.contains("Bash step 49"));
    assert!(!text.contains("Bash step 25"));
    assert_eq!(input.digest.failed.len(), FAILED_RESULTS);
    assert!(input.digest.failed[0].chars().count() <= FAILED_RESULT_CHARS + 1);
}

#[test]
fn invocation_is_lean_and_prompt_precedes_variadic_tools() {
    let args = invocation_args("PRAXIS-KNOW-x", "<turn>t</turn>".into());
    assert_eq!(args[0], "-p");
    assert_eq!(args[1], "<turn>t</turn>");
    let at = |flag: &str| args[args.iter().position(|a| a == flag).unwrap() + 1].clone();
    assert_eq!(at("--model"), "sonnet");
    assert_eq!(at("--effort"), "low");
    assert_eq!(at("--setting-sources"), "");
    assert_eq!(at("--tools"), "");
    assert!(at("--system-prompt").contains("PRAXIS-KNOW-x"));
    assert!(args.contains(&"--strict-mcp-config".to_string()));
    assert!(args.contains(&"--no-session-persistence".to_string()));
}

// A2 — 검증을 통과하지 못한 출력은 카드가 되지 않는다.
#[test]
fn verdict_trusts_only_json_after_the_nonce() {
    let input = input(6);
    let m = material(&input);
    let none = HashSet::new();
    let spoofed = format!("{}\nN-1\n{{\"show\": false}}", card_json("가짜"));
    assert_eq!(parse_verdict(&spoofed, "N-1", &m, &none), Ok(Verdict::Nothing));
    assert!(parse_verdict(&card_json("앞에만"), "N-1", &m, &none).is_err());
}

#[test]
fn verdict_rejects_bad_fields_and_excluded_topics() {
    let input = input(6);
    let m = material(&input);
    let none = HashSet::new();
    let with = |patch: serde_json::Value| {
        let mut v: serde_json::Value = serde_json::from_str(&card_json("세션 토큰 만료")).unwrap();
        for (k, val) in patch.as_object().unwrap() {
            v[k] = val.clone();
        }
        format!("N-1\n{v}")
    };
    let reject = |raw: String, excluded: &HashSet<String>| parse_verdict(&raw, "N-1", &m, excluded).is_err();
    assert!(reject(with(serde_json::json!({"evidence": "Edit src/other.ts"})), &none), "근거 불일치");
    assert!(reject(with(serde_json::json!({"evidence": ""})), &none), "빈 근거");
    assert!(reject(with(serde_json::json!({"kind": "tip"})), &none), "잘못된 kind");
    assert!(reject(with(serde_json::json!({"title": "가".repeat(61)})), &none), "제목 초과");
    assert!(reject(with(serde_json::json!({"body": " "})), &none), "빈 본문");
    assert!(reject(with(serde_json::json!({"topic": "t".repeat(41)})), &none), "주제 초과");
    assert!(reject(with(serde_json::json!({"show": "yes"})), &none), "show 형식");
    let excluded: HashSet<String> = ["세션 토큰 만료".to_string()].into();
    assert!(reject(with(serde_json::json!({"topic": "  세션   토큰 만료 "})), &excluded), "제외 주제");
}

#[test]
fn verdict_accepts_a_card_and_normalizes_it() {
    let input = input(6);
    let m = material(&input);
    let raw = format!("N-1\n{}", card_json("  Session   TOKEN "));
    let Ok(Verdict::Card(card)) = parse_verdict(&raw, "N-1", &m, &HashSet::new()) else {
        panic!("카드가 나와야 한다");
    };
    assert_eq!(card.kind, KIND_HEADS_UP);
    assert_eq!(card.topic, "session token");
    assert_eq!(card.evidence, "Edit src/auth/file0.ts");
}

#[test]
fn turn_label_is_the_first_nonempty_line_clipped() {
    assert_eq!(turn_label("  \n로그인 오류 고쳐줘\n둘째 줄"), "로그인 오류 고쳐줘");
    assert_eq!(turn_label(&"가".repeat(50)), format!("{}…", "가".repeat(LABEL_CHARS)));
    assert_eq!(turn_label(""), "");
}

// A3·A12 — 판정 1회에 카드 최대 1장, 6번째에서 가장 오래된 활성 카드가 밀려난다.
#[tokio::test]
async fn sixth_active_card_evicts_the_oldest() {
    let pool = test_pool().await;
    let id = task(&pool).await;
    let flights = Flights::default();
    for n in 0..6 {
        // 백오프를 피하려 직전 카드에 동작한다 — 상한만 본다.
        if let Some(card) = list_active(&pool, id).await.unwrap().first() {
            act(&pool, card.id, CardAction::Asked, 10 + n).await.unwrap();
        }
        let outcome = check(&pool, &flights, id, input(6), 10 + n, replies(card_json(&format!("주제 {n}")))).await;
        assert_eq!(outcome, Some(Outcome::Card));
    }
    let active = list_active(&pool, id).await.unwrap();
    assert_eq!(active.len(), 5);
    assert_eq!(active[0].topic, "주제 5");
    assert_eq!(active[0].turn_label, "로그인 오류 고쳐줘");
    let evicted: Vec<String> =
        sqlx::query_scalar("SELECT topic FROM know_cards WHERE task_id = ? AND action = 'evicted'")
            .bind(id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(evicted, vec!["주제 0".to_string()]);
    assert_eq!(outcomes(&pool, id).await, vec!["card"; 6]);
}

// A5·A12 — 판정 실패는 기록만 남기고 조용히 끝난다.
#[tokio::test]
async fn each_check_records_one_outcome() {
    let pool = test_pool().await;
    let id = task(&pool).await;
    let flights = Flights::default();
    let failing = |_: Vec<String>| std::future::ready(Err::<String, String>("claude 실행 실패".into()));
    assert_eq!(check(&pool, &flights, id, input(6), 1, failing).await, Some(Outcome::Failed));
    let nothing = |args: Vec<String>| std::future::ready(Ok(format!("{}\n{{\"show\": false}}", nonce_in(&args))));
    assert_eq!(check(&pool, &flights, id, input(6), 2, nothing).await, Some(Outcome::None));
    let unquoted = card_json("t").replace("Edit   src/auth/file0.ts", "지어낸 근거");
    assert_eq!(check(&pool, &flights, id, input(6), 3, replies(unquoted)).await, Some(Outcome::Rejected));
    {
        let _running = flights.claim(id).unwrap();
        assert_eq!(check(&pool, &flights, id, input(6), 4, replies(card_json("t"))).await, Some(Outcome::Busy));
    }
    assert_eq!(check(&pool, &flights, id, input(6), 5, replies(card_json("t"))).await, Some(Outcome::Card));
    assert_eq!(outcomes(&pool, id).await, ["failed", "none", "rejected", "busy", "card"]);
    let durations: Vec<Option<i64>> =
        sqlx::query_scalar("SELECT duration_ms FROM know_card_checks WHERE task_id = ? ORDER BY id")
            .bind(id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(durations[0].is_some() && durations[3].is_none());
}

#[tokio::test]
async fn a_card_for_a_deleted_task_is_not_stored() {
    let pool = test_pool().await;
    let outcome = check(&pool, &Flights::default(), 9_999, input(6), 1, replies(card_json("t"))).await;
    assert_eq!(outcome, Some(Outcome::Failed));
    assert!(list_active(&pool, 9_999).await.unwrap().is_empty());
}

// A4 — 연속 무시 3·4·5·6·7장이면 1·2·4·8·16번 건너뛴다.
#[test]
fn backoff_doubles_from_three_ignored_and_caps_at_sixteen() {
    let skips: Vec<usize> = (0..=9).map(skips_for).collect();
    assert_eq!(skips, [0, 0, 0, 1, 2, 4, 8, 16, 16, 16]);
}

async fn seed_cards(pool: &SqlitePool, id: i64, n: usize) {
    for i in 0..n {
        sqlx::query(
            "INSERT INTO know_cards (task_id, kind, topic, title, body, evidence, turn_label, created_at)
             VALUES (?, 'heads_up', ?, 't', 'b', 'e', 'l', 1)",
        )
        .bind(id)
        .bind(format!("시드 {i}"))
        .execute(pool)
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn ignored_cards_back_off_until_the_newest_one_is_acted_on() {
    let pool = test_pool().await;
    let id = task(&pool).await;
    let flights = Flights::default();
    seed_cards(&pool, id, 4).await;
    // 연속 무시 4장 → 2번 건너뛰고 세 번째에 판정한다.
    let run = |n: i64| check(&pool, &flights, id, input(6), n, replies("{\"show\": false}".to_string()));
    assert_eq!(run(1).await, Some(Outcome::Skipped));
    assert_eq!(run(2).await, Some(Outcome::Skipped));
    assert_eq!(run(3).await, Some(Outcome::None));
    assert_eq!(run(4).await, Some(Outcome::Skipped));

    // 최신 카드에 동작하면 연속이 끊겨 바로 판정한다.
    let newest = list_active(&pool, id).await.unwrap()[0].id;
    act(&pool, newest, CardAction::Dismissed, 5).await.unwrap();
    assert_eq!(run(6).await, Some(Outcome::None));
    assert_eq!(outcomes(&pool, id).await, ["skipped", "skipped", "none", "skipped", "none"]);
}

#[tokio::test]
async fn busy_rows_neither_count_nor_break_the_skip_run() {
    let pool = test_pool().await;
    let id = task(&pool).await;
    seed_cards(&pool, id, 4).await;
    for outcome in [Outcome::Skipped, Outcome::Busy] {
        store::record_check(&pool, id, outcome, None, 1).await;
    }
    assert_eq!(store::trailing_skips(&pool, id).await, 1);
    assert_eq!(store::consecutive_ignored(&pool, id).await, 4);
}

// A11 — 이미 앎·닫기는 남고, 이미 앎 주제는 다음 프롬프트의 제외 목록에 들어간다.
#[tokio::test]
async fn known_topic_persists_and_enters_the_next_prompt() {
    let pool = test_pool().await;
    let id = task(&pool).await;
    let flights = Flights::default();
    check(&pool, &flights, id, input(6), 1, replies(card_json("세션 토큰 만료"))).await;
    let card = list_active(&pool, id).await.unwrap().remove(0);
    assert_eq!(act(&pool, card.id, CardAction::Known, 2).await.unwrap(), Some(id));
    assert!(list_active(&pool, id).await.unwrap().is_empty());
    let (known, recent) = store::topics(&pool).await.unwrap();
    assert_eq!(known, vec!["세션 토큰 만료".to_string()]);
    assert!(recent.is_empty(), "이미 앎이 최근을 덮는다");

    let prompt = std::sync::Mutex::new(String::new());
    let outcome = check(&pool, &flights, id, input(6), 3, |args: Vec<String>| {
        *prompt.lock().unwrap() = args[1].clone();
        std::future::ready(Ok(format!("{}\n{}", nonce_in(&args), card_json("세션 토큰 만료"))))
    })
    .await;
    assert_eq!(outcome, Some(Outcome::Rejected));
    assert!(prompt.lock().unwrap().contains("<known_topics>\n- 세션 토큰 만료\n</known_topics>"));
}

#[tokio::test]
async fn card_actions_update_the_row() {
    let pool = test_pool().await;
    let id = task(&pool).await;
    seed_cards(&pool, id, 2).await;
    let cards = list_active(&pool, id).await.unwrap();
    act(&pool, cards[0].id, CardAction::Asked, 7).await.unwrap();
    act(&pool, cards[0].id, CardAction::Asked, 9).await.unwrap();
    mark_seen(&pool, cards[0].id, 5).await.unwrap();
    mark_seen(&pool, cards[0].id, 8).await.unwrap();
    act(&pool, cards[1].id, CardAction::Dismissed, 7).await.unwrap();
    let active = list_active(&pool, id).await.unwrap();
    assert_eq!(active.len(), 1, "자세히는 카드를 남긴다");
    assert_eq!((active[0].asked_at, active[0].seen_at), (Some(7), Some(5)));
    assert_eq!(act(&pool, 12_345, CardAction::Known, 1).await.unwrap(), None);
}

// 판정이 카드를 쓰는 중에 눌러도 기다렸다가 기록한다. 읽기로 시작한 지연 트랜잭션은 쓰기로
// 올라갈 때 busy handler를 거치지 않아, 기다리지 않고 곧바로 "database is locked"로 실패한다.
#[tokio::test]
async fn an_action_waits_for_a_concurrent_writer() {
    let pool = test_pool().await;
    let id = task(&pool).await;
    seed_cards(&pool, id, 1).await;
    let card = list_active(&pool, id).await.unwrap()[0].id;

    let writer = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let pending = tokio::spawn({
        let pool = pool.clone();
        async move { act(&pool, card, CardAction::Dismissed, 3).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    writer.commit().await.unwrap();

    assert_eq!(pending.await.unwrap().unwrap(), Some(id));
    assert!(list_active(&pool, id).await.unwrap().is_empty());
}

#[tokio::test]
async fn topic_lists_keep_the_newest_fifty() {
    let pool = test_pool().await;
    let id = task(&pool).await;
    for n in 0..55_i64 {
        let draft = CardDraft {
            kind: KIND_YOU_SHOULD_KNOW,
            topic: format!("주제 {n}"),
            title: "t".into(),
            body: "b".into(),
            evidence: "e".into(),
        };
        assert!(store::insert_card(&pool, id, &draft, "l", n).await.unwrap());
    }
    let (_, recent) = store::topics(&pool).await.unwrap();
    assert_eq!(recent.len(), 50);
    assert_eq!(recent[0], "주제 54");
    assert!(!recent.contains(&"주제 4".to_string()));
}
