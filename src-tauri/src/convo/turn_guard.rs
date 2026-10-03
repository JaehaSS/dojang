use super::process_cleanup::Survivor;
use super::{ConvoEvent, Vendor};
use std::borrow::Cow;
use std::collections::BTreeMap;

pub(crate) const TURN_COMPLETION_GUARD: &str = "\
# Praxis synchronous turn contract
- Run delegated work in foreground; collect every terminal result before ending this turn.
- Never leave background processes or servers running after the turn.
- Keep builds/tests in foreground. If a tool yields a running process, stay in this turn and collect its result.
- Before the final answer, verify the actual diff and relevant tests; do not claim completion or promise later notification while work remains active.";

pub(crate) fn completion_system_prompt(vendor: Vendor) -> Option<&'static str> {
    (vendor == Vendor::Claude).then_some(TURN_COMPLETION_GUARD)
}

/// 가드 경고 뒤에 원 응답 전문을 붙일 때의 구분자.
///
/// Result.text를 턴의 최종 답변으로 읽는 소비자(runner/process.rs)가 있어 전문을 버릴 수 없다.
/// 대화 뷰는 같은 응답을 이미 스트리밍 text 이벤트로 렌더링했으므로 이 마커에서 잘라 중복
/// 노출을 막는다 — src/components/ide/ConversationView.tsx의 GUARD_LAST_RESPONSE_MARKER와
/// 문자 단위로 같아야 한다.
pub(crate) const LAST_RESPONSE_MARKER: &str = "\n\n에이전트의 마지막 응답:\n";

pub(crate) fn guarded_message<'a>(vendor: Vendor, message: &'a str) -> Cow<'a, str> {
    if vendor != Vendor::Codex {
        return Cow::Borrowed(message);
    }
    Cow::Owned(format!(
        "{TURN_COMPLETION_GUARD}\n\n# User request\n{message}"
    ))
}

/// 한 턴 안의 백그라운드 작업 회수 여부를 센다.
///
/// 판정은 `result` 줄마다가 아니라 **프로세스가 끝난 뒤 한 번**이다(`run_turn_with_contract`가
/// 마지막 Result를 붙들었다가 EOF에서 판정한다). claude 2.1.28x의 print 모드는 모델이 응답을
/// 마쳐도 백그라운드 작업이 살아 있으면 프로세스를 끝내지 않고, 알림이 오면 모델을 다시 불러
/// `result`를 또 낸다 — `result`마다 판정하면 아직 회수 중인 작업이 미완료로 찍히고, 같은 경고가
/// `result` 수만큼 반복된다(실제로 한 턴에 세 번 찍혔다).
#[derive(Default)]
pub(crate) struct SubagentTurnGuard {
    pending: BTreeMap<String, String>,
    /// 모델이 응답을 마친 뒤 하니스가 끊은 작업. 결과는 아무도 읽지 못했다.
    cut: BTreeMap<String, String>,
    /// 마지막 Result 이후 새 도구 호출이 없는 상태 — 이때 온 `stopped`는 하니스가 끊은 것이다.
    answered: bool,
}

impl SubagentTurnGuard {
    pub(crate) fn observe_event(&mut self, event: &ConvoEvent) {
        match event {
            ConvoEvent::ToolUse {
                name,
                summary,
                tool_id: Some(id),
                parent_id: None,
            } if matches!(name.as_str(), "Task" | "Agent" | "Bash" | "Monitor") => {
                // 포그라운드 Bash도 일단 담기지만 결과가 곧바로 와서 빠진다. 백그라운드로
                // 넘어간 것만 접수 응답에 걸려 남는다 — 호출 시점에는 둘을 구분할 수 없다.
                self.pending.insert(id.clone(), summary.clone());
                self.answered = false;
            }
            ConvoEvent::ToolUse {
                parent_id: None, ..
            } => self.answered = false,
            ConvoEvent::ToolResult {
                summary,
                tool_use_id: Some(id),
                parent_id: None,
                ..
            } if !is_launch_receipt(summary) => {
                self.pending.remove(id);
            }
            ConvoEvent::Result { .. } => self.answered = true,
            _ => {}
        }
    }

    pub(crate) fn observe_raw_line(&mut self, line: &str) -> Option<ConvoEvent> {
        let (tool_id, status) = terminal_notification(line)?;
        let title = self.pending.remove(&tool_id);
        let completed = status == "completed";
        // 모델이 응답을 마친 뒤의 stopped/killed는 에이전트의 TaskStop이 아니라 하니스가
        // 종료하면서 끊은 것이다 — 접수만 되고 결과는 증발했으므로 미완료와 같은 무게로 센다.
        if self.answered && matches!(status.as_str(), "stopped" | "killed") {
            if let Some(title) = title {
                self.cut.insert(tool_id.clone(), title);
            }
        }
        Some(ConvoEvent::ToolResult {
            summary: if completed {
                "백그라운드 작업 완료".into()
            } else {
                format!("백그라운드 작업 종료 ({status})")
            },
            is_error: !completed,
            // 가드가 합성한 이벤트라 대응하는 원문이 없다 — 0이 아니라 미상.
            result_chars: None,
            tool_use_id: Some(tool_id),
            parent_id: None,
        })
    }

    pub(crate) fn sanitize_event(&self, event: ConvoEvent) -> ConvoEvent {
        match &event {
            ConvoEvent::ToolResult { summary, .. } if is_async_agent_receipt(summary) => {
                ConvoEvent::Other
            }
            _ => event,
        }
    }

    pub(crate) fn enforce_result(&self, event: ConvoEvent) -> ConvoEvent {
        let ConvoEvent::Result {
            text,
            is_error: false,
            session_id,
            cost_usd,
            num_turns,
            tokens_in,
            tokens_out,
        } = event
        else {
            return event;
        };
        if self.pending.is_empty() && self.cut.is_empty() {
            return ConvoEvent::Result {
                text,
                is_error: false,
                session_id,
                cost_usd,
                num_turns,
                tokens_in,
                tokens_out,
            };
        }
        let mut sections = Vec::new();
        if !self.pending.is_empty() {
            sections.push(format!("미완료 작업: {}", titles(&self.pending)));
        }
        if !self.cut.is_empty() {
            sections.push(format!(
                "응답 뒤 하니스가 끊은 작업(결과 미회수): {}",
                titles(&self.cut)
            ));
        }
        let text = format!(
            "에이전트가 백그라운드 작업 {}건을 완료 회수하기 전에 턴을 종료했습니다. \
턴이 끝나면 그 프로세스도 함께 사라지므로, 이 응답은 정상 완료로 처리되지 않았습니다.\
\n\n{}{LAST_RESPONSE_MARKER}{text}",
            self.pending.len() + self.cut.len(),
            sections.join("\n")
        );
        ConvoEvent::Result {
            text,
            is_error: true,
            session_id,
            cost_usd,
            num_turns,
            tokens_in,
            tokens_out,
        }
    }

    /// 프로세스 관측을 겹친 판정.
    ///
    /// `pending`이 비었을 때만 `survivors`를 본다 — pending이 차 있으면 기존 경고가 이미
    /// **무엇이** 미완료인지 도구 호출 이름으로 말해주고, 그쪽이 사용자의 요청에 더 가깝다.
    /// 둘 다 울리면 사용자가 어느 쪽을 봐야 할지 모른다. 이 경로는 1차가 놓친 것만 잡는다.
    ///
    /// 각 생존자는 pgid와 함께 명령줄을 싣는다. 경고 직후 그룹이 죽으므로 번호만으로는
    /// 사후에 아무것도 확인할 수 없다 — 유실된 작업인지 무해한 잔여물인지 여기서 갈린다.
    pub(crate) fn enforce_result_with_survivors(
        &self,
        event: ConvoEvent,
        survivors: &[Survivor],
    ) -> ConvoEvent {
        if !self.pending.is_empty() || !self.cut.is_empty() || survivors.is_empty() {
            return self.enforce_result(event);
        }
        let ConvoEvent::Result {
            text,
            is_error: false,
            session_id,
            cost_usd,
            num_turns,
            tokens_in,
            tokens_out,
        } = event
        else {
            return event;
        };
        let groups = survivors
            .iter()
            .map(|survivor| {
                let command = survivor
                    .arguments
                    .as_deref()
                    .and_then(command_line)
                    .unwrap_or_else(|| "(명령줄 확인 불가)".to_string());
                format!("- {} · {command}", survivor.group)
            })
            .collect::<Vec<_>>()
            .join("\n");
        let text = format!(
            "에이전트가 살아 있는 프로세스 그룹 {}건을 남기고 턴을 종료했습니다. \
하니스가 접수 문구를 남기지 않아 대화만으로는 드러나지 않았습니다 — 턴이 끝나면 함께 사라집니다.\
\n\n남은 프로세스 그룹:\n{groups}{LAST_RESPONSE_MARKER}{text}",
            survivors.len()
        );
        ConvoEvent::Result {
            text,
            is_error: true,
            session_id,
            cost_usd,
            num_turns,
            tokens_in,
            tokens_out,
        }
    }

    #[cfg(test)]
    pub(crate) fn pending_count(&self) -> usize {
        self.pending.len()
    }

    #[cfg(test)]
    pub(crate) fn cut_count(&self) -> usize {
        self.cut.len()
    }
}

fn titles(entries: &BTreeMap<String, String>) -> String {
    entries
        .values()
        .map(|title| {
            if title.trim().is_empty() {
                "이름 없는 작업"
            } else {
                title.as_str()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// 경고 한 줄에 실을 명령줄 길이 상한(문자). 넘으면 뒤를 자르고 `…`를 붙인다.
///
/// 판별에 필요한 것은 "무엇인지 알아볼 수 있는 만큼"이지 전문이 아니다. node로 띄운 MCP
/// 서버처럼 인터프리터 + 절대 경로 조합이 흔해, 짧으면 파일명까지 닿지 못한다.
pub(crate) const COMMAND_LIMIT: usize = 160;
/// 이 길이를 넘는 실행 파일 경로는 뒤 두 마디만 남긴다.
pub(crate) const PROGRAM_LIMIT: usize = 40;

/// argv를 경고 한 줄로 만든다. 남길 것이 없으면 `None`.
///
/// 표시 규칙이 여기 있는 것은 이 줄을 만드는 곳이 여기이기 때문이다. 관측 쪽
/// (`process_cleanup`)은 argv를 다듬지 않은 채로 넘긴다 — 거기서 잘라 버리면 다른 소비자가
/// 생겼을 때 원본이 어디에도 남지 않는다.
///
/// 인자에 개행이 섞이면 그룹당 한 줄인 목록이 무너지므로 공백으로 눕힌다.
pub(crate) fn command_line(arguments: &[String]) -> Option<String> {
    let joined = arguments
        .iter()
        .enumerate()
        .map(|(index, argument)| {
            let argument = argument.replace(['\n', '\r', '\t'], " ");
            if index == 0 {
                shorten_program(&argument)
            } else {
                argument
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let joined = joined.trim();
    if joined.is_empty() {
        return None;
    }
    if joined.chars().count() <= COMMAND_LIMIT {
        return Some(joined.to_string());
    }
    let mut short: String = joined.chars().take(COMMAND_LIMIT).collect();
    short.push('…');
    Some(short)
}

/// 길게 늘어진 실행 파일 경로를 뒤 두 마디로 줄인다. 짧으면 그대로 둔다.
///
/// 인터프리터 경로는 길고, 정작 판별에 쓰이는 것은 그 뒤의 스크립트·서브커맨드다. 통째로
/// 실으면 길이 상한이 경로에서 다 소진돼 "무엇을 하는 프로세스인가"가 잘린다 — macOS의
/// `/usr/bin/python3`는 Xcode 안 115자짜리 실체로 exec되어 실제로 그렇게 잘렸다.
pub(crate) fn shorten_program(program: &str) -> String {
    let marks = program.split('/').filter(|mark| !mark.is_empty()).count();
    if program.chars().count() <= PROGRAM_LIMIT || marks <= 2 || !program.starts_with('/') {
        return program.to_string();
    }
    let tail = program
        .rsplit('/')
        .filter(|mark| !mark.is_empty())
        .take(2)
        .collect::<Vec<_>>();
    format!(".../{}/{}", tail[1], tail[0])
}

/// 도구 결과가 **완료가 아니라 접수**인가. 실제 종료는 `task-notification`으로 따로 온다.
///
/// 서브 에이전트뿐 아니라 백그라운드 셸·Monitor도 여기 걸려야 한다. 이 호스트는 턴이 끝나면
/// 벤더 프로세스가 죽고(프린트 모드), 백그라운드 셸은 그 자식이라 함께 사라진다 — 회수하지
/// 않은 채 턴을 마치면 명령이 한 줄도 실행되지 않은 채 증발한다(실제로 릴리스 빌드가 그렇게
/// 날아갔다). Agent만 세던 시절에는 이 경로가 `pending`에 잡히지 않아 경고도 뜨지 않았다.
///
/// 판정이 하니스 문구에 의존하는 것은 약점이다. 다만 tool_result에는 구조화된 "백그라운드로
/// 넘어감" 표식이 없어 현재로선 이것이 유일한 단서다 — 느슨하게(고유 어절만) 맞춘다.
fn is_launch_receipt(summary: &str) -> bool {
    is_async_agent_receipt(summary)
        // Bash run_in_background — 명시적으로 띄운 경우
        || summary.contains("in background with ID")
        // 도구 타임아웃(600s)에 걸려 하니스가 자동으로 넘긴 경우
        || summary.contains("moved to the background")
        || summary.contains("Monitor started")
}

/// 화면에서 감출 접수 응답 — 서브 에이전트 것만이다.
///
/// 백그라운드 셸 접수는 감추지 않는다. 무엇을 언제 띄웠는지가 대화에서 사라지면, 사용자가
/// 미완료를 추적할 단서도 같이 없어진다.
fn is_async_agent_receipt(summary: &str) -> bool {
    summary.contains("Async agent launched successfully")
        || summary.contains("resumed from transcript in the background")
}

/// 백그라운드 작업의 종료 알림에서 `(tool_use_id, status)`를 꺼낸다.
///
/// 형태가 둘이다. claude 2.1.28x의 stream-json은 구조화된 `system/task_notification` 한 줄로
/// 알린다(2026-09-26 실측). 그 전에는 `user` 메시지 본문의 `<task-notification>` 태그였고,
/// 트랜스크립트 파일에는 지금도 그 형태가 남아 있어 둘 다 받는다. 문자열 쪽만 읽던 동안
/// 새 형태를 통째로 놓쳐, 회수가 끝난 작업까지 미완료로 찍혔다.
fn terminal_notification(line: &str) -> Option<(String, String)> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if value.get("type").and_then(|item| item.as_str()) == Some("system")
        && value.get("subtype").and_then(|item| item.as_str()) == Some("task_notification")
    {
        let status = value.get("status").and_then(|item| item.as_str())?;
        if !is_terminal_status(status) {
            return None;
        }
        let tool_id = value.get("tool_use_id").and_then(|item| item.as_str())?;
        return Some((tool_id.to_string(), status.to_string()));
    }
    let content = value
        .get("content")
        .and_then(|item| item.as_str())
        .or_else(|| {
            value
                .pointer("/message/content")
                .and_then(|item| item.as_str())
        })?;
    let status = tag_value(content, "status")?;
    if !is_terminal_status(&status) {
        return None;
    }
    Some((tag_value(content, "tool-use-id")?, status))
}

fn is_terminal_status(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "killed" | "stopped")
}

fn tag_value(content: &str, tag: &str) -> Option<String> {
    let start_tag = format!("<{tag}>");
    let end_tag = format!("</{tag}>");
    let start = content.find(&start_tag)? + start_tag.len();
    let end = content[start..].find(&end_tag)? + start;
    Some(content[start..end].trim().to_string())
}
