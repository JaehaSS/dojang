//! 단발 실행 벤더(Claude)의 질문 세션. 질문은 인앱 MCP 툴 호출이 되고, 그 호출은 답이 올 때까지
//! **서버 쪽에서 멈춘다** — 그래서 턴 프로세스는 양방향 stdin을 열 필요가 없다.
//!
//! 세션 등록이 곧 툴의 존재다. 등록이 없으면 `ask_user`는 목록에도 없고 호출도 거절된다.

use super::app_server::Context;
use super::{interaction as ledger, ConvoEvent, TurnOutcome};
use crate::preview_bridge::mcp::PreviewMcpLease;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Claude 질문 계약은 완료 계약과 한 번 합성해 append-system-prompt로 보낸다.
pub const TOOL_INSTRUCTIONS: &str = "# Praxis clarification contract\n- To ask the user a question, call the MCP tool `mcp__praxis-preview__ask_user` with kind=clarification and 1-3 questions (id, question, options with id/label/description, allow_free_text, is_secret=false).\n- Ask only ordinary clarifications. Never ask for secrets, credentials, or tool execution approvals, and never treat an answer as a permission grant.\n- The tool blocks until the user replies and returns their answers. Do not ask the user anything in plain prose and then stop — a question asked outside this tool never reaches the user.\n- While it is pending, continue only work that does not depend on the reply.\n- Leave no question unanswered when the turn ends; the turn fails if one is still open.";

const POLL: Duration = Duration::from_millis(200);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// 등록된 턴. `anchors`는 질문이 트랜스크립트에 남길 이벤트의 통로다 — `ask`는 벤더 스트림을
/// 읽는 스레드가 아니라 MCP 핸들러(tokio)에서 돌고, 스트림 콜백은 그 스레드에 묶여 있다.
#[derive(Clone)]
struct Session {
    ctx: Context,
    anchors: Sender<ConvoEvent>,
    /// 이 턴이 도구 실행을 승인에 맡겼는가. 켜져 있을 때만 `approve`가 존재한다.
    approvals: bool,
}

fn sessions() -> &'static Mutex<HashMap<i64, Session>> {
    static MAP: OnceLock<Mutex<HashMap<i64, Session>>> = OnceLock::new();
    MAP.get_or_init(Default::default)
}

/// 등록의 수명이 툴의 수명이다 — 턴이 어떻게 끝나든(패닉 포함) 여기서 거둬진다.
struct Registered(i64);
impl Drop for Registered {
    fn drop(&mut self) {
        sessions()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

/// 이 작업의 턴이 지금 질문을 받을 수 있는가. `tools/list`가 이것으로 갈린다.
pub fn active(task: i64) -> bool {
    sessions()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(&task)
}

/// 이 작업의 턴이 실행 승인을 받는가. `tools/list`의 `approve`가 이것으로 갈린다.
pub fn approvals_active(task: i64) -> bool {
    session(task).is_some_and(|s| s.approvals)
}

/// "이 세션에서 항상 허용"한 도구 이름. 턴을 넘어 작업 단위로 살고, 앱을 다시 켜면 비워진다 —
/// 영속 허용 목록은 CLI 설정의 몫이고, 여기는 대화 한 벌의 편의다.
fn allowed() -> &'static Mutex<HashMap<i64, HashSet<String>>> {
    static MAP: OnceLock<Mutex<HashMap<i64, HashSet<String>>>> = OnceLock::new();
    MAP.get_or_init(Default::default)
}

fn session(task: i64) -> Option<Session> {
    sessions()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&task)
        .cloned()
}

/// 밀린 질문 앵커를 벤더 이벤트 **앞에** 싣는다. 답을 실은 `tool_result`보다 먼저 자리를 잡아야
/// 그 뒤의 도구 카드가 질문 아래로 쌓인다 — Codex 경로가 `open` 직후 `on_event`하는 것과 같은 순서다.
fn drain_anchors(anchors: &Receiver<ConvoEvent>, on_event: &mut impl FnMut(ConvoEvent)) {
    while let Ok(anchor) = anchors.try_recv() {
        on_event(anchor);
    }
}

/// MCP 핸들러가 부르는 진입점. **`Context::db`를 쓰지 않는다** — 그것은 `block_on`이고
/// 여기는 이미 tokio 런타임 안이다.
pub async fn ask(task: i64, arguments: &Value) -> Result<String, String> {
    let Session { ctx, anchors, .. } = session(task).ok_or("이 작업에는 열린 질문 세션이 없습니다")?;
    let execution = ctx.control.execution.clone();
    // 원장의 UNIQUE(execution, wire_id)를 지키려면 우리가 id를 만들어야 한다 — MCP JSON-RPC id는
    // 세션마다 1부터 되풀이된다. 응답은 같은 프로세스 안에서 돌려주므로 상관 id가 필요 없다.
    let wire = Value::String(ledger::id()?);
    let call = ledger::id()?;
    let interaction =
        ledger::open(&ctx.pool, &execution, &wire, &call, arguments, crate::now()).await?;
    // 트랜스크립트 앵커. 없으면 카드는 늘 대화 맨 아래 폴백에 그려져, 답한 뒤의 도구 카드가
    // 질문 **위로** 쌓인다. 수신 쪽이 이미 닫혔으면(턴 종료 경합) 앵커만 잃고 질문은 그대로다.
    let _ = anchors.send(ConvoEvent::Interaction {
        interaction_id: interaction.clone(),
    });
    ctx.changed();
    // Codex 경로(`Context::open`)와 같은 부수 효과 — 한쪽만 알리면 벤더에 따라 알림이 오고 안 온다(#511).
    ctx.question_opened().await;
    loop {
        if ctx.control.cancelled.load(Ordering::SeqCst) {
            return Err("사용자가 턴을 중단했습니다".into());
        }
        if let Some(answer) =
            ledger::take_dispatch_for(&ctx.pool, &execution, &interaction, crate::now()).await?
        {
            ledger::written(&ctx.pool, &answer.answer_id).await?;
            ledger::settle(&ctx.pool, &execution, &answer.call_id).await?;
            ctx.changed();
            return Ok(answer.output);
        }
        if !ledger::interaction_open(&ctx.pool, &interaction, crate::now()).await? {
            return Err("질문이 응답 없이 닫혔습니다".into());
        }
        tokio::time::sleep(POLL).await;
    }
}

const ALLOW: &str = "allow";
const ALLOW_ALWAYS: &str = "allow_always";
const DENY: &str = "deny";
const DETAIL_CHARS: usize = 1500;

/// CLI `--permission-prompt-tool`의 진입점. 승인 요청을 질문 원장의 카드로 올리고, 답을
/// CLI가 읽는 결정(`{"behavior":"allow"|"deny",…}`)으로 옮긴다. 대기·중단·턴 종료 정리는
/// 질문과 같은 경로를 탄다.
pub async fn approve(task: i64, arguments: &Value) -> Result<String, String> {
    let tool = arguments
        .get("tool_name")
        .and_then(Value::as_str)
        .ok_or("승인 요청에 도구 이름이 없습니다")?;
    let input = arguments.get("input").cloned().unwrap_or(Value::Null);
    if !approvals_active(task) {
        return Err("이 작업에는 열린 승인 세션이 없습니다".into());
    }
    // 모델이 승인 도구를 직접 부르면 실제 실행과 무관한 카드로 "항상 허용"을 받아낼 수 있다.
    if tool == crate::preview_bridge::mcp::inject::approval_tool_name() {
        return Ok(deny("승인 도구는 직접 호출할 수 없습니다"));
    }
    let remembered = allowed()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&task)
        .is_some_and(|tools| tools.contains(tool));
    if remembered || crate::preview_bridge::mcp::inject::is_own_tool(tool) {
        return Ok(allow(input));
    }
    let output = ask(task, &approval_questions(tool, &input)).await?;
    let decision = decide(&output)?;
    if decision == Decision::AllowAlways {
        allowed()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(task)
            .or_default()
            .insert(tool.to_string());
    }
    Ok(match decision {
        Decision::Allow | Decision::AllowAlways => allow(input),
        Decision::Deny(message) => deny(&message),
    })
}

fn allow(input: Value) -> String {
    serde_json::json!({"behavior": "allow", "updatedInput": input}).to_string()
}

fn deny(message: &str) -> String {
    serde_json::json!({"behavior": "deny", "message": message}).to_string()
}

#[derive(Debug, PartialEq)]
enum Decision {
    Allow,
    AllowAlways,
    Deny(String),
}

fn approval_detail(input: &Value) -> String {
    let picked = ["command", "file_path", "notebook_path", "path", "url", "pattern"]
        .iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str).map(str::to_string));
    let detail = picked.unwrap_or_else(|| input.to_string());
    let mut chars = detail.chars();
    let head: String = chars.by_ref().take(DETAIL_CHARS).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

fn approval_questions(tool: &str, input: &Value) -> Value {
    let option = |id: &str, label: String, description: &str| {
        serde_json::json!({"id": id, "label": label, "description": description})
    };
    serde_json::json!({
        "kind": "clarification",
        "questions": [{
            "id": "approval",
            "question": format!("Claude가 {tool} 실행 승인을 요청합니다.\n\n{}", approval_detail(input)),
            "options": [
                option(ALLOW, "허용".into(), "이번 한 번만 실행합니다."),
                option(ALLOW_ALWAYS, format!("이 세션에서 {tool} 항상 허용"), "이 대화가 끝날 때까지 같은 도구는 다시 묻지 않습니다."),
                option(DENY, "거부".into(), "실행하지 않습니다. 직접 입력하면 거부 사유로 Claude에게 전달됩니다."),
            ],
            "allow_free_text": true,
            "is_secret": false,
        }],
    })
}

/// `ask`가 돌려준 답(`answer_output`)을 결정으로 옮긴다. 자유 입력은 거부 사유다 — CLI의
/// "아니요, 대신 이렇게 하세요"와 같은 자리다.
fn decide(output: &str) -> Result<Decision, String> {
    let value: Value = serde_json::from_str(output).map_err(|e| e.to_string())?;
    let answer = value
        .pointer("/answers/0")
        .ok_or("승인 답이 비어 있습니다")?;
    if let Some(text) = answer.get("text").and_then(Value::as_str) {
        return Ok(Decision::Deny(format!("사용자가 실행을 거부하고 이렇게 전했습니다: {text}")));
    }
    Ok(match answer.get("option_id").and_then(Value::as_str) {
        Some(ALLOW) => Decision::Allow,
        Some(ALLOW_ALWAYS) => Decision::AllowAlways,
        _ => Decision::Deny("사용자가 실행을 거부했습니다.".into()),
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    ctx: &Context,
    cwd: &str,
    message: &str,
    resume: Option<&str>,
    idle_timeout: u64,
    vendor: super::Vendor,
    bin: &str,
    model: Option<&str>,
    effort: Option<&str>,
    service_tier: Option<&str>,
    images: &[String],
    session_name: Option<&str>,
    mcp: Option<&PreviewMcpLease>,
    on_spawn: impl FnOnce(u32),
    mut on_event: impl FnMut(ConvoEvent),
) -> Result<TurnOutcome, String> {
    let execution = ctx.control.execution.clone();
    let (anchor_tx, anchor_rx) = mpsc::channel::<ConvoEvent>();
    // spawn **전에** 실행을 올린다. 세션 id는 `system/init`에서야 오는데 `ask_user`는 그보다
    // 먼저 올 수 있고, `open`은 실행이 `running`이 아니면 질문을 받지 않는다.
    ctx.db(ledger::started_local(&ctx.pool, &execution, &ledger::id()?))?;
    ctx.changed();
    let registered = {
        sessions().lock().unwrap_or_else(|e| e.into_inner()).insert(
            ctx.task_id,
            Session {
                ctx: ctx.clone(),
                anchors: anchor_tx,
                approvals: mcp.is_some_and(|lease| lease.injection().approvals),
            },
        );
        Registered(ctx.task_id)
    };

    let spawn_ctx = ctx.clone();
    let result = super::run_turn_with_contract(
        cwd,
        message,
        resume,
        idle_timeout,
        vendor,
        bin,
        model,
        effort,
        service_tier,
        images,
        session_name,
        mcp.map(|lease| lease.injection()),
        true,
        move |pid| {
            // 중단이 읽을 자리. 플래그만으로는 이 런타임을 멈출 수 없다.
            spawn_ctx.control.pgid.store(pid, Ordering::SeqCst);
            // 회수 근거. 남기지 못해도 턴은 계속한다 — 없으면 크래시 복구가 약해질 뿐이다.
            if let Ok(Some(identity)) = crate::runner::process_identity::observe_group_leader(pid) {
                let _ = spawn_ctx.db(ledger::spawned_local(
                    &spawn_ctx.pool,
                    &spawn_ctx.control.execution,
                    pid,
                    &identity,
                ));
            }
            on_spawn(pid);
        },
        |event| {
            drain_anchors(&anchor_rx, &mut on_event);
            if let ConvoEvent::SessionInit { session_id } = &event {
                let _ = ctx.db(ledger::thread_bound(&ctx.pool, &execution, session_id));
            }
            on_event(event);
        },
    );
    // The child and its process group have returned. A later round's cancellation must never
    // signal this old numeric pid after the OS has reused it.
    ctx.control.pgid.store(0, Ordering::SeqCst);

    // 등록을 먼저 거둔다 — 이 아래로는 새 질문이 열리지 않는다.
    drop(registered);
    // 스트림이 끝난 뒤 열린 질문(중단·미응답)도 앵커는 남긴다 — 위치는 어차피 맨 끝이다.
    drain_anchors(&anchor_rx, &mut on_event);
    let interrupted = ctx.control.cancelled.load(Ordering::SeqCst);
    // 미응답 판정은 닫기보다 **먼저다**. 닫고 나면 셀 대상이 사라진다.
    let (unanswered, _) = ctx
        .db(ledger::pending(&ctx.pool, &execution, crate::now()))
        .unwrap_or((0, false));
    // 닫기가 배수보다 **먼저다**. 뒤집으면 대기 중인 MCP 호출이 스스로 풀리지 못해 배수가
    // 5초를 다 쓰고 실패하고, 멀쩡한 턴이 `cleanup_failed`로 잠긴다.
    let closed = ctx.db(ledger::close_questions(
        &ctx.pool,
        &execution,
        if interrupted {
            "cancelled"
        } else {
            "turn_ended"
        },
    ));
    let drained = mcp.is_none_or(|lease| lease.revoke_and_drain(CLOSE_TIMEOUT));
    if closed.is_err() || !drained {
        ctx.control.cleanup_failed.store(true, Ordering::SeqCst);
        let _ = ctx.db(ledger::phase(&ctx.pool, &execution, "cleanup_failed"));
        ctx.changed();
        return Err("질문 정리를 확인하지 못했습니다. 승인·폐기가 잠겨 있습니다".into());
    }

    let outcome = result.and_then(|turn| {
        if interrupted || unanswered == 0 {
            Ok(turn)
        } else {
            Err("응답되지 않은 질문이 남은 채 실행이 종료되었습니다".into())
        }
    });
    let failure = outcome.as_ref().err().cloned();
    if let Err(error) = ctx.db(ledger::finish(
        &ctx.pool,
        &execution,
        if interrupted {
            "cancelled"
        } else if failure.is_none() {
            "completed"
        } else {
            "failed"
        },
        failure.as_deref(),
    )) {
        ctx.control.cleanup_failed.store(true, Ordering::SeqCst);
        return Err(error);
    }
    ctx.changed();
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convo::app_server::Control;
    use serde_json::json;
    use std::sync::Arc;

    async fn pool() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE tasks(id INTEGER PRIMARY KEY,convo_session_id TEXT,pending_capsule TEXT);INSERT INTO tasks(id) VALUES(7),(8)")
            .execute(&pool)
            .await
            .unwrap();
        ledger::migrate(&pool).await.unwrap();
        ledger::bind(&pool, 7).await.unwrap();
        ledger::bind(&pool, 8).await.unwrap();
        pool
    }

    fn args() -> Value {
        json!({"kind":"clarification","questions":[{"id":"color","question":"색상?","options":[{"id":"blue","label":"파랑","description":""}],"allow_free_text":true,"is_secret":false}]})
    }

    /// 질문 카드가 대화 맨 아래 폴백이 아니라 제자리에 그려지려면 `ask`가 트랜스크립트 앵커를
    /// 남겨야 한다 — Codex 경로의 `open`과 같은 계약. 답이 실린 벤더 이벤트보다 앞에 선다.
    #[tokio::test]
    async fn ask_anchors_the_question_before_the_vendor_event_that_carries_the_answer() {
        const TASK: i64 = 7;
        let pool = pool().await;
        let execution = ledger::begin(&pool, TASK, 100).await.unwrap();
        ledger::started_local(&pool, &execution, "turn")
            .await
            .unwrap();
        let (anchor_tx, anchor_rx) = mpsc::channel::<ConvoEvent>();
        let ctx = Context {
            pool: pool.clone(),
            task_id: TASK,
            control: Arc::new(Control::new(execution.clone(), true)),
            changed: Arc::new(|| {}),
        };
        sessions().lock().unwrap_or_else(|e| e.into_inner()).insert(
            TASK,
            Session {
                ctx,
                anchors: anchor_tx,
                approvals: false,
            },
        );
        let _registered = Registered(TASK);

        let asking = tokio::spawn(async move { ask(TASK, &args()).await });
        let anchor = tokio::task::spawn_blocking(move || {
            anchor_rx.recv_timeout(Duration::from_secs(5)).unwrap()
        })
        .await
        .unwrap();
        let ConvoEvent::Interaction { interaction_id } = &anchor else {
            panic!("앵커가 질문 이벤트가 아니다: {anchor:?}");
        };
        let snapshot = ledger::snapshot(&pool, TASK).await.unwrap();
        assert_eq!(snapshot.items.len(), 1);
        assert_eq!(&snapshot.items[0].id, interaction_id);

        let answer = vec![ledger::Answer {
            question_id: "color".into(),
            option_id: Some("blue".into()),
            text: None,
        }];
        ledger::submit(
            &pool,
            TASK,
            &execution,
            interaction_id,
            "request",
            &answer,
            101,
        )
        .await
        .unwrap();
        let output = asking.await.unwrap().unwrap();
        assert!(output.contains("blue"), "{output}");

        // 앵커는 콜백이 벤더 이벤트를 넘기기 **전에** 배수된다.
        let (tx, rx) = mpsc::channel::<ConvoEvent>();
        tx.send(ConvoEvent::Interaction {
            interaction_id: "q".into(),
        })
        .unwrap();
        let mut seen = Vec::new();
        let mut sink = |event: ConvoEvent| seen.push(event);
        drain_anchors(&rx, &mut sink);
        sink(ConvoEvent::Text {
            text: "answer".into(),
            parent_id: None,
        });
        assert!(matches!(seen[0], ConvoEvent::Interaction { .. }));
        assert!(matches!(seen[1], ConvoEvent::Text { .. }));
    }

    #[test]
    fn approval_cards_pass_the_ledger_question_contract() {
        let long = json!({"command": "x".repeat(DETAIL_CHARS + 10)});
        let questions = approval_questions("Bash", &long);
        let parsed = ledger::validate_questions(&questions).unwrap();
        let item = &parsed.questions[0];
        assert!(item.question.contains("Bash"));
        assert!(item.question.ends_with('…'));
        let ids: Vec<_> = item.options.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, [ALLOW, ALLOW_ALWAYS, DENY]);
        assert!(item.allow_free_text);
    }

    #[test]
    fn answers_become_cli_permission_decisions() {
        let answer = |option: Option<&str>, text: Option<&str>| {
            ledger::answer_output(&[ledger::Answer {
                question_id: "approval".into(),
                option_id: option.map(str::to_string),
                text: text.map(str::to_string),
            }])
        };
        assert_eq!(decide(&answer(Some(ALLOW), None)).unwrap(), Decision::Allow);
        assert_eq!(decide(&answer(Some(ALLOW_ALWAYS), None)).unwrap(), Decision::AllowAlways);
        assert!(matches!(decide(&answer(Some(DENY), None)).unwrap(), Decision::Deny(_)));
        // 자유 입력은 거부 사유로 Claude에게 간다.
        let Decision::Deny(message) = decide(&answer(None, Some("테스트만 돌려"))).unwrap() else {
            panic!("자유 입력이 거부가 아니다");
        };
        assert!(message.contains("테스트만 돌려"));

        let allowed: Value = serde_json::from_str(&allow(json!({"command": "ls"}))).unwrap();
        assert_eq!(allowed["behavior"], "allow");
        assert_eq!(allowed["updatedInput"]["command"], "ls");
    }

    /// 승인 세션이 아니면 `approve`는 거절한다. 켜진 세션에서도 우리 툴과 "항상 허용"한 도구는
    /// 카드 없이 통과한다 — 카드가 열리면 원장에 interaction이 남으므로 그것으로 가른다.
    #[tokio::test]
    async fn approve_skips_the_card_for_own_and_remembered_tools() {
        const TASK: i64 = 8; // 전역 세션 맵을 쓰는 다른 테스트(7)와 겹치지 않게.
        let pool = pool().await;
        let execution = ledger::begin(&pool, TASK, 100).await.unwrap();
        ledger::started_local(&pool, &execution, "turn").await.unwrap();
        let (anchor_tx, _anchor_rx) = mpsc::channel::<ConvoEvent>();
        let ctx = Context {
            pool: pool.clone(),
            task_id: TASK,
            control: Arc::new(Control::new(execution.clone(), true)),
            changed: Arc::new(|| {}),
        };
        let register = |approvals| {
            sessions().lock().unwrap_or_else(|e| e.into_inner()).insert(
                TASK,
                Session { ctx: ctx.clone(), anchors: anchor_tx.clone(), approvals },
            );
            Registered(TASK)
        };
        let bash = json!({"tool_name": "Bash", "input": {"command": "ls"}});
        {
            let _registered = register(false);
            assert!(!approvals_active(TASK));
            assert!(approve(TASK, &bash).await.is_err());
        }
        let _registered = register(true);
        assert!(approvals_active(TASK));
        let own = json!({"tool_name": "mcp__praxis-preview__ask_user", "input": {}});
        assert!(approve(TASK, &own).await.unwrap().contains("\"allow\""));
        // 승인 도구 자신은 우리 도구여도 허용하지 않는다 — 가짜 카드로 "항상 허용"을 받아내지 못한다.
        let itself = json!({"tool_name": "mcp__praxis-preview__approve", "input": {"tool_name": "Bash"}});
        assert!(approve(TASK, &itself).await.unwrap().contains("\"deny\""));
        allowed().lock().unwrap().entry(TASK).or_default().insert("Bash".into());
        let decided = approve(TASK, &bash).await.unwrap();
        assert!(decided.contains("\"allow\"") && decided.contains("ls"));
        allowed().lock().unwrap().remove(&TASK);
        assert!(ledger::snapshot(&pool, TASK).await.unwrap().items.is_empty());
    }
}
