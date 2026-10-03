use super::*;
use crate::convo::Side;

#[tokio::test]
async fn cli_pin_migration_preserves_historical_threads_and_never_rewrites_a_pin() {
    let p = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("CREATE TABLE tasks(id INTEGER PRIMARY KEY); INSERT INTO tasks VALUES(1),(2); CREATE TABLE convo_runtime_bindings(task_id INTEGER PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,runtime_kind TEXT NOT NULL,tool_schema_hash TEXT NOT NULL)")
        .execute(&p).await.unwrap();
    for (task, runtime) in [(1, RUNTIME), (2, RUNTIME_LOCAL)] {
        sqlx::query("INSERT INTO convo_runtime_bindings VALUES(?,?,?)")
            .bind(task)
            .bind(runtime)
            .bind(schema_hash(runtime))
            .execute(&p)
            .await
            .unwrap();
    }
    migrate(&p).await.unwrap();
    assert_eq!(
        cli_version_of(&p, 1).await.unwrap(),
        super::super::cli_runtime::LEGACY_VERSION
    );
    assert_eq!(
        runtime_of(&p, 2).await.unwrap().as_deref(),
        Some(RUNTIME_LOCAL)
    );
    assert!(cli_version_of(&p, 2).await.is_err());
    sqlx::query("UPDATE convo_runtime_bindings SET cli_version='9.0.0' WHERE task_id=1")
        .execute(&p)
        .await
        .unwrap();
    migrate(&p).await.unwrap();
    assert!(runtime_of(&p, 1).await.unwrap_err().contains("9.0.0"));
    assert!(cli_version_of(&p, 1).await.is_err());
    sqlx::query("UPDATE convo_runtime_bindings SET cli_version=NULL WHERE task_id=1")
        .execute(&p)
        .await
        .unwrap();
    assert!(runtime_of(&p, 1).await.is_err());
}

#[tokio::test]
async fn cli_pin_survives_database_close_and_reopen() {
    let path = std::env::temp_dir().join(format!("praxis-cli-pin-{}.db", id().unwrap()));
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let p = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .unwrap();
    sqlx::query("CREATE TABLE tasks(id INTEGER PRIMARY KEY); INSERT INTO tasks VALUES(1)")
        .execute(&p)
        .await
        .unwrap();
    migrate(&p).await.unwrap();
    bind(&p, 1).await.unwrap();
    let version = cli_version_of(&p, 1).await.unwrap();
    assert_eq!(version, super::super::cli_runtime::DEFAULT_VERSION);
    p.close().await;
    let reopened = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    migrate(&reopened).await.unwrap();
    assert_eq!(cli_version_of(&reopened, 1).await.unwrap(), version);
    assert_eq!(
        runtime_of(&reopened, 1).await.unwrap().as_deref(),
        Some(RUNTIME)
    );
    reopened.close().await;
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn claude_binding_pins_codex_only_when_a_codex_turn_needs_it() {
    let p = pool().await;
    bind_runtime(&p, 2, RUNTIME_LOCAL).await.unwrap();
    assert!(cli_version_of(&p, 2).await.is_err());

    assert_eq!(
        ensure_codex_version(&p, 2).await.unwrap(),
        super::super::cli_runtime::DEFAULT_VERSION
    );
    assert_eq!(
        runtime_of(&p, 2).await.unwrap().as_deref(),
        Some(RUNTIME_LOCAL)
    );
    assert_eq!(
        cli_version_of(&p, 2).await.unwrap(),
        super::super::cli_runtime::DEFAULT_VERSION
    );

    // A missing Codex pin is historical corruption, not a reason to silently choose a version.
    sqlx::query("UPDATE convo_runtime_bindings SET cli_version=NULL WHERE task_id=1")
        .execute(&p)
        .await
        .unwrap();
    assert!(ensure_codex_version(&p, 1).await.is_err());
    sqlx::query("UPDATE convo_runtime_bindings SET cli_version='unapproved' WHERE task_id=2")
        .execute(&p)
        .await
        .unwrap();
    assert!(ensure_codex_version(&p, 2).await.is_err());
}

#[tokio::test]
async fn rebind_keeps_the_pin_and_is_transactional() {
    let p = pool().await;
    bind_runtime(&p, 2, RUNTIME_LOCAL).await.unwrap();

    let mut tx = p.begin().await.unwrap();
    rebind_for_agent_tx(&mut tx, 2, "codex").await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        runtime_of(&p, 2).await.unwrap().as_deref(),
        Some(RUNTIME_LOCAL)
    );
    assert!(cli_version_of(&p, 2).await.is_err());

    let mut tx = p.begin().await.unwrap();
    rebind_for_agent_tx(&mut tx, 2, "codex").await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(runtime_of(&p, 2).await.unwrap().as_deref(), Some(RUNTIME));
    assert_eq!(
        cli_version_of(&p, 2).await.unwrap(),
        super::super::cli_runtime::DEFAULT_VERSION
    );

    let execution = begin(&p, 2, 100).await.unwrap();
    let mut tx = p.begin().await.unwrap();
    assert!(rebind_for_agent_tx(&mut tx, 2, "claude").await.is_err());
    tx.rollback().await.unwrap();
    assert_eq!(runtime_of(&p, 2).await.unwrap().as_deref(), Some(RUNTIME));
    finish(&p, &execution, "failed", None).await.unwrap();
}

async fn pool() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("CREATE TABLE tasks(id INTEGER PRIMARY KEY,convo_session_id TEXT,pending_capsule TEXT);INSERT INTO tasks(id) VALUES(1),(2)").execute(&pool).await.unwrap();
    migrate(&pool).await.unwrap();
    bind(&pool, 1).await.unwrap();
    pool
}
fn args() -> Value {
    json!({"kind":"clarification","questions":[{"id":"color","question":"색상?","options":[{"id":"blue","label":"파랑","description":""}],"allow_free_text":true,"is_secret":false}]})
}
fn native_input(blocking: bool) -> Value {
    json!({"isBlocking":blocking,"itemId":"tool","threadId":"thread","turnId":"turn","questions":[{"id":"colour","header":"Colour","question":"Choose a colour","isOther":false,"isSecret":false,"options":[{"label":"red","description":"Warm"},{"label":"blue","description":"Cool"},{"label":"green","description":"Calm"},{"label":"violet","description":"Bright"}]}]})
}
fn answer() -> Vec<Answer> {
    vec![Answer {
        question_id: "color".into(),
        option_id: Some("blue".into()),
        text: None,
    }]
}
async fn question(pool: &SqlitePool) -> (String, String) {
    let e = begin(pool, 1, 100).await.unwrap();
    started(pool, &e, "thread", "turn").await.unwrap();
    let q = open(pool, &e, &json!(9007199254740993u64), "call", &args(), 100)
        .await
        .unwrap();
    (e, q)
}

#[tokio::test]
async fn native_request_keeps_exact_wire_correlation_and_uses_protocol_result() {
    let p = pool().await;
    let e = begin(&p, 1, 100).await.unwrap();
    started(&p, &e, "thread", "turn").await.unwrap();
    let interaction = open_native(
        &p,
        &e,
        &json!("native-request-1"),
        super::super::native_interaction::TOOL_INPUT,
        &native_input(true),
        100,
    )
    .await
    .unwrap();
    let item = snapshot(&p, 1).await.unwrap().items.pop().unwrap();
    assert_eq!(item.call_id, "native:\"native-request-1\"");
    assert_eq!(
        item.request.unwrap().kind,
        super::super::native_interaction::NativeRequestKind::Question
    );
    let persisted: (String, String) =
        sqlx::query_as("SELECT native_method,native_params FROM convo_interactions WHERE id=?")
            .bind(&interaction)
            .fetch_one(&p)
            .await
            .unwrap();
    assert_eq!(persisted.0, super::super::native_interaction::TOOL_INPUT);
    assert_eq!(
        serde_json::from_str::<Value>(&persisted.1).unwrap(),
        native_input(true)
    );
    assert!(open_native(
        &p,
        &e,
        &json!("native-request-1"),
        super::super::native_interaction::TOOL_INPUT,
        &native_input(true),
        100
    )
    .await
    .is_err());
    let answers = vec![Answer {
        question_id: "colour".into(),
        option_id: Some("option:0:3".into()),
        text: None,
    }];
    submit(&p, 1, &e, &interaction, "native-receipt", &answers, 101)
        .await
        .unwrap();
    let dispatched = take_dispatch(&p, &e, 102).await.unwrap().unwrap();
    assert_eq!(dispatched.wire_id, json!("native-request-1"));
    assert_eq!(
        dispatched.native_result,
        Some(json!({"answers":{"colour":{"answers":["violet"]}}}))
    );
    written(&p, &dispatched.answer_id).await.unwrap();
    assert_eq!(
        receipt(&p, 1, "native-receipt").await.unwrap().state,
        "written"
    );
    assert!(resolve_native(&p, &e, &json!("native-request-1"))
        .await
        .unwrap());
    // Resolution means only that the server stopped waiting: it must not manufacture an ACK.
    assert_eq!(
        receipt(&p, 1, "native-receipt").await.unwrap().state,
        "written"
    );
    assert_eq!(
        snapshot(&p, 1).await.unwrap().items[0].reason.as_deref(),
        Some("resolved")
    );
    assert!(!resolve_native(&p, &e, &json!("native-request-1"))
        .await
        .unwrap());
}

#[tokio::test]
async fn native_nonblocking_input_does_not_block_turn_completion_and_is_closed() {
    let p = pool().await;
    let e = begin(&p, 1, 100).await.unwrap();
    started(&p, &e, "thread", "turn").await.unwrap();
    open_native(
        &p,
        &e,
        &json!(9),
        super::super::native_interaction::TOOL_INPUT,
        &native_input(false),
        100,
    )
    .await
    .unwrap();
    assert_eq!(pending(&p, &e, 101).await.unwrap().0, 1);
    assert_eq!(blocking_pending(&p, &e, 101).await.unwrap().0, 0);
    finish(&p, &e, "completed", None).await.unwrap();
    let item = snapshot(&p, 1).await.unwrap().items.pop().unwrap();
    assert_eq!(item.state, "closed");
    assert_eq!(item.reason.as_deref(), Some("turn_ended"));
}

#[tokio::test]
async fn native_mcp_form_rejects_schema_invalid_answer_before_receipt_claim() {
    let p = pool().await;
    let e = begin(&p, 1, 100).await.unwrap();
    started(&p, &e, "thread", "turn").await.unwrap();
    let form = json!({"mode":"form","message":"Count","requestedSchema":{"type":"object","required":["count"],"properties":{"count":{"type":"integer","minimum":1,"maximum":3}}}});
    let interaction = open_native(
        &p,
        &e,
        &json!(10),
        super::super::native_interaction::MCP_ELICITATION,
        &form,
        100,
    )
    .await
    .unwrap();
    let invalid = vec![
        Answer {
            question_id: "count".into(),
            option_id: None,
            text: Some("4".into()),
        },
        Answer {
            question_id: super::super::native_interaction::FORM_ACTION_ID.into(),
            option_id: Some("accept".into()),
            text: None,
        },
    ];
    let error = submit(&p, 1, &e, &interaction, "bad-form", &invalid, 101)
        .await
        .unwrap_err();
    assert!(error.starts_with("INPUT_VALIDATION: "));
    assert_eq!(receipt(&p, 1, "bad-form").await.unwrap().state, "not_found");
    let corrected = vec![
        Answer {
            question_id: "count".into(),
            option_id: None,
            text: Some("3".into()),
        },
        Answer {
            question_id: super::super::native_interaction::FORM_ACTION_ID.into(),
            option_id: Some("accept".into()),
            text: None,
        },
    ];
    assert_eq!(
        submit(&p, 1, &e, &interaction, "bad-form", &corrected, 102)
            .await
            .unwrap()
            .state,
        "claimed"
    );
}

#[tokio::test]
async fn native_mcp_optional_field_can_be_skipped_through_complete_answer_dispatch() {
    let p = pool().await;
    let e = begin(&p, 1, 100).await.unwrap();
    started(&p, &e, "thread", "turn").await.unwrap();
    let form = json!({"mode":"form","message":"Configure","requestedSchema":{"type":"object","required":["count"],"properties":{"count":{"type":"integer"},"note":{"type":"string"}}}});
    let interaction = open_native(
        &p,
        &e,
        &json!(11),
        super::super::native_interaction::MCP_ELICITATION,
        &form,
        100,
    )
    .await
    .unwrap();
    let answers = vec![
        Answer {
            question_id: "count".into(),
            option_id: None,
            text: Some("2".into()),
        },
        Answer {
            question_id: "note".into(),
            option_id: Some("skip".into()),
            text: None,
        },
        Answer {
            question_id: super::super::native_interaction::FORM_ACTION_ID.into(),
            option_id: Some("accept".into()),
            text: None,
        },
    ];
    submit(&p, 1, &e, &interaction, "optional-form", &answers, 101)
        .await
        .unwrap();
    assert_eq!(
        take_dispatch(&p, &e, 102)
            .await
            .unwrap()
            .unwrap()
            .native_result,
        Some(json!({"action":"accept","content":{"count":2}}))
    );
}

#[tokio::test]
async fn native_mcp_decline_can_skip_form_fields_but_cannot_smuggle_extra_answers() {
    let p = pool().await;
    let e = begin(&p, 1, 100).await.unwrap();
    started(&p, &e, "thread", "turn").await.unwrap();
    let form = json!({"mode":"form","message":"Configure","requestedSchema":{"type":"object","required":["comment"],"properties":{"comment":{"type":"string"}}}});
    let interaction = open_native(
        &p,
        &e,
        &json!(14),
        super::super::native_interaction::MCP_ELICITATION,
        &form,
        100,
    )
    .await
    .unwrap();
    let action = Answer {
        question_id: super::super::native_interaction::FORM_ACTION_ID.into(),
        option_id: Some("decline".into()),
        text: None,
    };
    let extra = Answer {
        question_id: "comment".into(),
        option_id: None,
        text: Some("extra answer".into()),
    };
    assert!(submit(
        &p,
        1,
        &e,
        &interaction,
        "decline-extra",
        &[action.clone(), extra],
        101
    )
    .await
    .is_err());
    submit(&p, 1, &e, &interaction, "decline-only", &[action], 101)
        .await
        .unwrap();
    assert_eq!(
        take_dispatch(&p, &e, 102)
            .await
            .unwrap()
            .unwrap()
            .native_result,
        Some(json!({"action":"decline","content":null}))
    );
}

#[tokio::test]
async fn native_secret_input_is_rejected_before_nonblocking_request_is_stored() {
    let p = pool().await;
    let e = begin(&p, 1, 100).await.unwrap();
    started(&p, &e, "thread", "turn").await.unwrap();
    let mut secret = native_input(false);
    secret["questions"][0]["isSecret"] = json!(true);
    assert!(open_native(
        &p,
        &e,
        &json!(12),
        super::super::native_interaction::TOOL_INPUT,
        &secret,
        100,
    )
    .await
    .is_err());
    assert!(snapshot(&p, 1).await.unwrap().items.is_empty());
}

#[tokio::test]
async fn rejected_native_relay_has_a_durable_receipt_without_an_acknowledgement() {
    let p = pool().await;
    let e = begin(&p, 1, 100).await.unwrap();
    started(&p, &e, "thread", "turn").await.unwrap();
    let interaction = open_native(
        &p,
        &e,
        &json!(13),
        super::super::native_interaction::ASYNC_MESSAGE,
        &native_input(false),
        100,
    )
    .await
    .unwrap();
    let answers = vec![Answer {
        question_id: "colour".into(),
        option_id: Some("option:0:0".into()),
        text: None,
    }];
    submit(&p, 1, &e, &interaction, "relay-rejected", &answers, 101)
        .await
        .unwrap();
    let dispatch = take_dispatch(&p, &e, 102).await.unwrap().unwrap();
    assert_eq!(
        dispatch.native_method.as_deref(),
        Some(super::super::native_interaction::ASYNC_MESSAGE)
    );
    reject_native_dispatch(&p, &dispatch.answer_id)
        .await
        .unwrap();
    assert_eq!(
        receipt(&p, 1, "relay-rejected").await.unwrap().state,
        "rejected"
    );
    assert!(resolve_native(&p, &e, &json!(13)).await.unwrap());
}

#[tokio::test]
async fn native_migration_adds_private_columns_without_changing_legacy_rows() {
    let p = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("CREATE TABLE tasks(id INTEGER PRIMARY KEY,convo_session_id TEXT,pending_capsule TEXT); INSERT INTO tasks(id) VALUES(1); CREATE TABLE convo_interactions(id TEXT PRIMARY KEY,execution_id TEXT NOT NULL,wire_id TEXT NOT NULL,call_id TEXT NOT NULL,questions TEXT NOT NULL,state TEXT NOT NULL DEFAULT 'pending',reason TEXT,revision INTEGER NOT NULL DEFAULT 1,created_at INTEGER NOT NULL,expires_at INTEGER NOT NULL,UNIQUE(execution_id,wire_id),UNIQUE(execution_id,call_id)); INSERT INTO convo_interactions VALUES('legacy','execution','1','call','{}','closed',NULL,1,1,2)")
        .execute(&p).await.unwrap();
    migrate(&p).await.unwrap();
    for column in ["native_method", "native_params", "native_request"] {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('convo_interactions') WHERE name=?",
        )
        .bind(column)
        .fetch_one(&p)
        .await
        .unwrap();
        assert_eq!(count, 1, "{column}");
    }
    let native: Option<String> =
        sqlx::query_scalar("SELECT native_request FROM convo_interactions WHERE id='legacy'")
            .fetch_one(&p)
            .await
            .unwrap();
    assert!(native.is_none());
}

#[tokio::test]
async fn pending_tasks_lists_open_questions_and_drops_expired_closed_or_finished_ones() {
    let pool = pool().await;
    assert!(pending_tasks(&pool, 100).await.unwrap().is_empty());
    let (e, q) = question(&pool).await;
    assert_eq!(pending_tasks(&pool, 100).await.unwrap(), vec![1]);
    // 만료 시각을 지나면 열려 있어도 답을 받지 않으므로 목록에서 빠진다.
    let expires: i64 = sqlx::query_scalar("SELECT expires_at FROM convo_interactions WHERE id=?")
        .bind(&q)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(pending_tasks(&pool, expires).await.unwrap().is_empty());
    // 턴이 끝나 질문이 닫히면 실행 상태와 무관하게 사라진다.
    finish(&pool, &e, "completed", None).await.unwrap();
    assert!(pending_tasks(&pool, 100).await.unwrap().is_empty());
    // 다른 작업의 실행이 running이 아니면(시작 전) 질문이 있어도 세지 않는다.
    bind(&pool, 2).await.unwrap();
    let e2 = begin(&pool, 2, 100).await.unwrap();
    started(&pool, &e2, "thread2", "turn2").await.unwrap();
    open(&pool, &e2, &json!(1), "call2", &args(), 100)
        .await
        .unwrap();
    assert_eq!(pending_tasks(&pool, 100).await.unwrap(), vec![2]);
    phase(&pool, &e2, "cancelling").await.unwrap();
    assert!(pending_tasks(&pool, 100).await.unwrap().is_empty());
}

#[test]
fn rejects_unknown_approval_secret_and_oversized_envelopes() {
    for (key, value) in [
        ("kind", json!("approval")),
        ("extra", json!("secret-fixture")),
    ] {
        let mut v = args();
        v[key] = value;
        assert!(validate_questions(&v).is_err());
    }
    let mut v = args();
    v["questions"][0]["is_secret"] = json!(true);
    assert!(validate_questions(&v).is_err());
    let mut v = args();
    v["questions"][0]["question"] = json!("a".repeat(2001));
    assert!(validate_questions(&v).is_err());
    let mut v = args();
    v["questions"] = json!([]);
    assert!(validate_questions(&v).is_err());
    let mut v = args();
    v["questions"][0]["options"][0]["unknown"] = json!("secret-fixture");
    assert!(validate_questions(&v).is_err());
}
#[tokio::test]
async fn typed_wire_ids_and_task_execution_isolation() {
    let p = pool().await;
    let (e, q) = question(&p).await;
    let q2 = open(
        &p,
        &e,
        &json!("9007199254740993"),
        "call-string",
        &args(),
        100,
    )
    .await
    .unwrap();
    assert_ne!(q, q2);
    assert!(submit(&p, 2, &e, &q, "request", &answer(), 101)
        .await
        .is_err());
    assert!(
        submit(&p, 1, "other-execution", &q, "request", &answer(), 101)
            .await
            .is_err()
    );
    assert_eq!(receipt(&p, 2, "request").await.unwrap().state, "not_found");
    submit(&p, 1, &e, &q, "request", &answer(), 101)
        .await
        .unwrap();
    let dispatch = take_dispatch(&p, &e, 102).await.unwrap().unwrap();
    assert_eq!(dispatch.wire_id, json!(9007199254740993u64));
    assert!(take_dispatch(&p, &e, 102).await.unwrap().is_none());
}
#[tokio::test]
async fn receipt_retries_never_redispatch_and_ack_requires_exact_output() {
    let p = pool().await;
    let (e, q) = question(&p).await;
    let a = submit(&p, 1, &e, &q, "request", &answer(), 101)
        .await
        .unwrap();
    assert_eq!(a.state, "claimed");
    assert_eq!(
        submit(&p, 1, &e, &q, "request", &answer(), 102)
            .await
            .unwrap(),
        a
    );
    let other = vec![Answer {
        question_id: "color".into(),
        option_id: None,
        text: Some("red".into()),
    }];
    assert!(submit(&p, 1, &e, &q, "request", &other, 102).await.is_err());
    assert!(submit(&p, 1, &e, &q, "different-request", &answer(), 102)
        .await
        .is_err());
    let d = take_dispatch(&p, &e, 102).await.unwrap().unwrap();
    written(&p, &d.answer_id).await.unwrap();
    assert!(!acknowledge(&p, &e, "call", &json!([]), true).await.unwrap());
    assert_eq!(receipt(&p, 1, "request").await.unwrap().state, "written");
    let contents = json!([{"type":"inputText","text":d.output}]);
    assert!(!acknowledge(&p, &e, "wrong-call", &contents, true)
        .await
        .unwrap());
    assert!(!acknowledge(&p, &e, "call", &contents, false).await.unwrap());
    assert!(acknowledge(&p, &e, "call", &contents, true).await.unwrap());
    assert_eq!(
        submit(&p, 1, &e, &q, "request", &answer(), 103)
            .await
            .unwrap()
            .state,
        "acknowledged"
    );
    assert!(take_dispatch(&p, &e, 102).await.unwrap().is_none());
    assert_eq!(
        snapshot(&p, 1).await.unwrap().items[0].reason.as_deref(),
        Some("answered")
    );
}
#[tokio::test]
async fn draft_cas_and_partial_answers_survive_snapshot() {
    let p = pool().await;
    let (e, q) = question(&p).await;
    assert_eq!(draft(&p, 1, &e, &q, &answer(), 0, 101).await.unwrap(), 1);
    assert!(draft(&p, 1, &e, &q, &[], 0, 102).await.is_err());
    assert_eq!(draft(&p, 1, &e, &q, &[], 1, 102).await.unwrap(), 2);
    assert_eq!(snapshot(&p, 1).await.unwrap().items[0].draft_revision, 2);
    draft(&p, 1, &e, &q, &answer(), 2, 103).await.unwrap();
    submit(&p, 1, &e, &q, "request", &answer(), 104)
        .await
        .unwrap();
    assert!(draft(&p, 1, &e, &q, &[], 3, 104).await.is_err());
    let restored = snapshot(&p, 1).await.unwrap();
    assert_eq!(restored.items[0].draft, answer());
    assert_eq!(restored.items[0].draft_revision, 0);
}
#[tokio::test]
async fn expiry_cancel_and_cleanup_failure_keep_admission_closed() {
    let p = pool().await;
    let (e, q) = question(&p).await;
    assert!(
        submit(&p, 1, &e, &q, "expired", &answer(), 100 + QUESTION_TTL)
            .await
            .is_err()
    );
    phase(&p, &e, "cancelling").await.unwrap();
    assert!(submit(&p, 1, &e, &q, "cancel", &answer(), 101)
        .await
        .is_err());
    phase(&p, &e, "cleanup_failed").await.unwrap();
    assert!(begin(&p, 1, 102).await.is_err());
    assert!(blocked(&p, 1).await.unwrap());
    finish(&p, &e, "failed", Some("cancelled")).await.unwrap();
    assert!(begin(&p, 1, 103).await.is_ok());
    assert!(submit(&p, 1, &e, &q, "old", &answer(), 104).await.is_err());
}
#[tokio::test]
async fn accepted_or_partial_write_crashes_become_unknown_without_replay() {
    for dispatch in [false, true] {
        let p = pool().await;
        let (e, q) = question(&p).await;
        submit(&p, 1, &e, &q, "request", &answer(), 101)
            .await
            .unwrap();
        if dispatch {
            take_dispatch(&p, &e, 102).await.unwrap().unwrap();
        }
        close_questions(&p, &e, "connection_lost").await.unwrap();
        assert_eq!(receipt(&p, 1, "request").await.unwrap().state, "unknown");
        assert!(take_dispatch(&p, &e, 102).await.unwrap().is_none());
        assert_eq!(snapshot(&p, 1).await.unwrap().items[0].draft, answer());
    }
}
#[tokio::test]
async fn failed_answer_insert_rolls_back_claim_and_keeps_draft() {
    let p = pool().await;
    let (e, q) = question(&p).await;
    draft(&p, 1, &e, &q, &answer(), 0, 101).await.unwrap();
    sqlx::query("CREATE TRIGGER deny_answer BEFORE INSERT ON convo_interaction_answers BEGIN SELECT RAISE(ABORT,'injected storage failure');END").execute(&p).await.unwrap();
    assert!(submit(&p, 1, &e, &q, "request", &answer(), 102)
        .await
        .is_err());
    assert_eq!(receipt(&p, 1, "request").await.unwrap().state, "not_found");
    assert!(take_dispatch(&p, &e, 102).await.unwrap().is_none());
    assert_eq!(snapshot(&p, 1).await.unwrap().items[0].draft, answer());
    sqlx::query("DROP TRIGGER deny_answer")
        .execute(&p)
        .await
        .unwrap();
    assert_eq!(
        submit(&p, 1, &e, &q, "request", &answer(), 103)
            .await
            .unwrap()
            .state,
        "claimed"
    );
}
#[tokio::test]
async fn deleting_task_cascades_question_draft_and_receipt() {
    let p = pool().await;
    let (e, q) = question(&p).await;
    submit(&p, 1, &e, &q, "request", &answer(), 101)
        .await
        .unwrap();
    sqlx::query("DELETE FROM tasks WHERE id=1")
        .execute(&p)
        .await
        .unwrap();
    for table in [
        "convo_runtime_bindings",
        "convo_executions",
        "convo_interactions",
        "convo_interaction_answers",
        "convo_interaction_drafts",
    ] {
        let n: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&p)
            .await
            .unwrap();
        assert_eq!(n, 0, "{table}");
    }
}
#[tokio::test]
async fn secret_fixture_is_rejected_before_it_reaches_storage() {
    let p = pool().await;
    let e = begin(&p, 1, 100).await.unwrap();
    started(&p, &e, "t", "u").await.unwrap();
    let mut v = args();
    v["questions"][0]["is_secret"] = json!(true);
    v["questions"][0]["question"] = json!("NEVER-PERSIST-SECRET");
    assert!(open(&p, &e, &json!(1), "call", &v, 100).await.is_err());
    assert!(snapshot(&p, 1).await.unwrap().items.is_empty());
}
#[tokio::test]
async fn simultaneous_duplicate_receipts_converge_to_one_claim() {
    let p = pool().await;
    let (e, q) = question(&p).await;
    let answers = answer();
    let (a, b) = tokio::join!(
        submit(&p, 1, &e, &q, "request", &answers, 101),
        submit(&p, 1, &e, &q, "request", &answers, 101)
    );
    assert_eq!(a.unwrap(), b.unwrap());
    assert!(take_dispatch(&p, &e, 102).await.unwrap().is_some());
    assert!(take_dispatch(&p, &e, 102).await.unwrap().is_none());
}
#[tokio::test]
async fn session_binding_and_capsule_clear_commit_with_turn_acceptance() {
    let p = pool().await;
    sqlx::query("UPDATE tasks SET convo_session_id='old',pending_capsule='handoff' WHERE id=1")
        .execute(&p)
        .await
        .unwrap();
    let execution = begin(&p, 1, 100).await.unwrap();
    sqlx::query("CREATE TRIGGER deny_session BEFORE UPDATE ON tasks BEGIN SELECT RAISE(ABORT,'injected session failure');END").execute(&p).await.unwrap();
    assert!(started(&p, &execution, "new", "turn").await.is_err());
    let current: (String, String) =
        sqlx::query_as("SELECT convo_session_id,pending_capsule FROM tasks WHERE id=1")
            .fetch_one(&p)
            .await
            .unwrap();
    assert_eq!(current, ("old".into(), "handoff".into()));
    let phase: String = sqlx::query_scalar("SELECT state FROM convo_executions WHERE id=?")
        .bind(&execution)
        .fetch_one(&p)
        .await
        .unwrap();
    assert_eq!(phase, "starting");
    sqlx::query("DROP TRIGGER deny_session")
        .execute(&p)
        .await
        .unwrap();
    started(&p, &execution, "new", "turn").await.unwrap();
    let current: (String, Option<String>) =
        sqlx::query_as("SELECT convo_session_id,pending_capsule FROM tasks WHERE id=1")
            .fetch_one(&p)
            .await
            .unwrap();
    assert_eq!(current, ("new".into(), None));
}

#[tokio::test]
async fn side_session_binding_never_mutates_the_main_task_and_rolls_back_without_a_seat() {
    let p = pool().await;
    sqlx::query("CREATE TABLE convo_debate_sides(task_id INTEGER NOT NULL,side TEXT NOT NULL,agent TEXT NOT NULL,model TEXT,vendor_session_id TEXT,PRIMARY KEY(task_id,side))")
        .execute(&p)
        .await
        .unwrap();
    sqlx::query("UPDATE tasks SET convo_session_id='left-session',pending_capsule='left-handoff' WHERE id=1")
        .execute(&p)
        .await
        .unwrap();
    let execution = begin(&p, 1, 100).await.unwrap();
    assert!(
        started_on_side(&p, &execution, "right-session", "turn", Some(Side::Right))
            .await
            .is_err()
    );
    let state: String = sqlx::query_scalar("SELECT state FROM convo_executions WHERE id=?")
        .bind(&execution)
        .fetch_one(&p)
        .await
        .unwrap();
    assert_eq!(state, "starting");
    let main: (String, String) =
        sqlx::query_as("SELECT convo_session_id,pending_capsule FROM tasks WHERE id=1")
            .fetch_one(&p)
            .await
            .unwrap();
    assert_eq!(main, ("left-session".into(), "left-handoff".into()));

    sqlx::query("INSERT INTO convo_debate_sides(task_id,side,agent,model,vendor_session_id) VALUES(1,'right','claude',NULL,NULL)")
        .execute(&p)
        .await
        .unwrap();
    started_on_side(&p, &execution, "right-session", "turn", Some(Side::Right))
        .await
        .unwrap();
    let side: Option<String> = sqlx::query_scalar(
        "SELECT vendor_session_id FROM convo_debate_sides WHERE task_id=1 AND side='right'",
    )
    .fetch_one(&p)
    .await
    .unwrap();
    assert_eq!(side.as_deref(), Some("right-session"));
    let main: (String, String) =
        sqlx::query_as("SELECT convo_session_id,pending_capsule FROM tasks WHERE id=1")
            .fetch_one(&p)
            .await
            .unwrap();
    assert_eq!(main, ("left-session".into(), "left-handoff".into()));
}

#[tokio::test]
async fn ensure_idle_rejects_each_active_execution_state() {
    let p = pool().await;
    for state in [
        "starting",
        "running",
        "cancelling",
        "finalizing",
        "cleanup_failed",
    ] {
        let execution = begin(&p, 1, 100).await.unwrap();
        if state != "starting" {
            phase(&p, &execution, state).await.unwrap();
        }
        assert!(ensure_idle(&p, 1).await.is_err(), "{state}");
        finish(&p, &execution, "failed", None).await.unwrap();
    }
    ensure_idle(&p, 1).await.unwrap();
}

#[tokio::test]
async fn accepted_answer_cannot_start_dispatch_at_or_after_expiry() {
    let p = pool().await;
    let (e, q) = question(&p).await;
    submit(
        &p,
        1,
        &e,
        &q,
        "near-expiry",
        &answer(),
        100 + QUESTION_TTL - 1,
    )
    .await
    .unwrap();
    assert!(take_dispatch(&p, &e, 100 + QUESTION_TTL)
        .await
        .unwrap()
        .is_none());
    assert!(take_dispatch(&p, &e, 101 + QUESTION_TTL)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        receipt(&p, 1, "near-expiry").await.unwrap().state,
        "claimed"
    );
    close_questions(&p, &e, "expired").await.unwrap();
    assert_eq!(
        receipt(&p, 1, "near-expiry").await.unwrap().state,
        "unknown"
    );
    assert!(take_dispatch(&p, &e, 102 + QUESTION_TTL)
        .await
        .unwrap()
        .is_none());
}
