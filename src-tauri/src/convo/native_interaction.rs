//! Normalization for app-server user input and approval requests.
//!
//! The database keeps the original wire envelope, while this module derives the small, safe
//! interaction shape the UI already knows how to render.  Never add a permissive fallback here:
//! an unknown native request must be rejected so it cannot turn into an accidental approval.
use super::interaction::{Answer, OptionItem, Question, Questions};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

pub const TOOL_INPUT: &str = "item/tool/requestUserInput";
pub const COMMAND_APPROVAL: &str = "item/commandExecution/requestApproval";
pub const FILE_APPROVAL: &str = "item/fileChange/requestApproval";
pub const PERMISSIONS_APPROVAL: &str = "item/permissions/requestApproval";
pub const MCP_ELICITATION: &str = "mcpServer/elicitation/request";
/// Host-only async user message. The lead converts its provider event to TOOL_INPUT-shaped params
/// and routes the resulting answer through `turn/steer`, never as a fabricated JSON-RPC reply.
pub const ASYNC_MESSAGE: &str = "dojang/asyncMessage";
pub const FORM_ACTION_ID: &str = "__dojang_action";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NativeRequestKind {
    Question,
    Approval,
    Form,
    Url,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NativeRequestInfo {
    pub kind: NativeRequestKind,
    pub title: String,
    pub details: String,
    pub blocking: bool,
    pub url: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NormalizedNativeRequest {
    pub info: NativeRequestInfo,
    pub questions: Questions,
}

fn string(value: &Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("native 요청의 {key}이(가) 없습니다"))
}
fn opt_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}
fn details(value: &Value) -> String {
    opt_string(value, "reason")
        .or_else(|| opt_string(value, "message"))
        .unwrap_or_default()
}
fn shown(value: &Value, key: &str) -> String {
    value
        .get(key)
        .map(Value::to_string)
        .unwrap_or_else(|| "null".into())
}
fn question(id: String, text: String, options: Vec<OptionItem>, free_text: bool) -> Question {
    Question {
        id,
        question: text,
        options,
        allow_free_text: free_text,
        is_secret: false,
    }
}
fn option(
    id: impl Into<String>,
    label: impl Into<String>,
    description: impl Into<String>,
) -> OptionItem {
    OptionItem {
        id: id.into(),
        label: label.into(),
        description: description.into(),
    }
}
fn approval_options(allowed: &[&str]) -> Vec<OptionItem> {
    allowed
        .iter()
        .filter_map(|decision| match *decision {
            "accept" => Some(option("accept", "허용", "이 요청을 허용합니다")),
            "decline" => Some(option("decline", "거절", "이 요청을 허용하지 않습니다")),
            "cancel" => Some(option("cancel", "취소", "요청을 거절하고 턴을 중단합니다")),
            _ => None,
        })
        .collect()
}
fn approval_questions(allowed: &[&str]) -> Questions {
    Questions {
        kind: "clarification".into(),
        questions: vec![question(
            "decision".into(),
            "이 요청을 허용할까요?".into(),
            approval_options(allowed),
            false,
        )],
    }
}
fn available_decisions(
    value: &Value,
    defaults: &'static [&'static str],
) -> Result<Vec<&'static str>, String> {
    let Some(values) = value.get("availableDecisions") else {
        return Ok(defaults.to_vec());
    };
    if values.is_null() {
        return Ok(defaults.to_vec());
    }
    let values = values
        .as_array()
        .ok_or("승인 선택지 형식이 올바르지 않습니다")?;
    let allowed: Vec<&'static str> = values
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|value| match value {
            "accept" if defaults.contains(&"accept") => Some("accept"),
            "decline" if defaults.contains(&"decline") => Some("decline"),
            "cancel" if defaults.contains(&"cancel") => Some("cancel"),
            _ => None,
        })
        .collect();
    if allowed.is_empty() {
        Err("지원하는 승인 선택지가 없습니다".into())
    } else {
        Ok(allowed)
    }
}
fn approval_details(method: &str, value: &Value) -> String {
    let reason = details(value);
    let mut lines = match method {
        COMMAND_APPROVAL => vec![
            format!("명령: {}", shown(value, "command")),
            format!("작업 폴더: {}", shown(value, "cwd")),
            format!(
                "네트워크 승인 문맥: {}",
                shown(value, "networkApprovalContext")
            ),
            format!("추가 권한: {}", shown(value, "additionalPermissions")),
        ],
        FILE_APPROVAL => {
            let mut lines = vec![format!("쓰기 루트: {}", shown(value, "grantRoot"))];
            for key in ["changes", "fileChanges", "patch", "diff"] {
                if value.get(key).is_some() {
                    lines.push(format!("{key}: {}", shown(value, key)));
                }
            }
            lines
        }
        PERMISSIONS_APPROVAL => vec![format!("요청 권한: {}", shown(value, "permissions"))],
        _ => Vec::new(),
    };
    if !reason.is_empty() {
        lines.push(format!("사유: {reason}"));
    }
    lines.join("\n")
}
fn approval(
    method: &str,
    title: &str,
    value: &Value,
    defaults: &'static [&'static str],
) -> Result<NormalizedNativeRequest, String> {
    let allowed = available_decisions(value, defaults)?;
    Ok(NormalizedNativeRequest {
        info: NativeRequestInfo {
            kind: NativeRequestKind::Approval,
            title: title.into(),
            details: approval_details(method, value),
            blocking: true,
            url: None,
        },
        questions: approval_questions(&allowed),
    })
}
fn reject_secrets(value: &Value) -> Result<(), String> {
    match value {
        Value::Object(map) => {
            if map.get("isSecret") == Some(&Value::Bool(true))
                || map.get("is_secret") == Some(&Value::Bool(true))
                || map.get("format") == Some(&Value::String("password".into()))
                || map.get("type") == Some(&Value::String("password".into()))
            {
                return Err("비밀 또는 비밀번호 입력은 저장할 수 없습니다".into());
            }
            for value in map.values() {
                reject_secrets(value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                reject_secrets(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn input(value: &Value) -> Result<NormalizedNativeRequest, String> {
    reject_secrets(value)?;
    let blocking = value
        .get("isBlocking")
        .and_then(Value::as_bool)
        .ok_or("native 질문의 isBlocking이 없습니다")?;
    let questions = value
        .get("questions")
        .and_then(Value::as_array)
        .ok_or("native 질문이 없습니다")?;
    if questions.is_empty() {
        return Err("native 질문이 비어 있습니다".into());
    }
    let mut ids = HashSet::new();
    let mut normalized = Vec::with_capacity(questions.len());
    for (index, raw) in questions.iter().enumerate() {
        let id = string(raw, "id")?;
        if id.len() > 128 || id.chars().any(char::is_control) || !ids.insert(id.clone()) {
            return Err("native 질문 ID가 올바르지 않습니다".into());
        }
        let text = string(raw, "question")?;
        let options = raw
            .get("options")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut choices = Vec::with_capacity(options.len());
        for (option_index, choice) in options.iter().enumerate() {
            let label = string(choice, "label")?;
            choices.push(option(
                format!("option:{index}:{option_index}"),
                label,
                opt_string(choice, "description").unwrap_or_default(),
            ));
        }
        // The app-server schema permits `options: null`; those ordinary questions are free-text
        // prompts even when older providers omit `isOther`.
        let free_text =
            raw.get("isOther").and_then(Value::as_bool).unwrap_or(false) || choices.is_empty();
        normalized.push(question(id, text, choices, free_text));
    }
    Ok(NormalizedNativeRequest {
        info: NativeRequestInfo {
            kind: NativeRequestKind::Question,
            title: "사용자 입력".into(),
            details: String::new(),
            blocking,
            url: None,
        },
        questions: Questions {
            kind: "clarification".into(),
            questions: normalized,
        },
    })
}
fn property_options(schema: &Value) -> Result<(Vec<OptionItem>, bool), String> {
    let ty = string(schema, "type")?;
    match ty.as_str() {
        "boolean" => Ok((
            vec![option("true", "true", ""), option("false", "false", "")],
            false,
        )),
        "string" if schema.get("enum").is_some() => {
            let values = schema
                .get("enum")
                .and_then(Value::as_array)
                .ok_or("enum 형식이 올바르지 않습니다")?;
            if values.is_empty() || !values.iter().all(Value::is_string) {
                return Err("enum 형식이 올바르지 않습니다".into());
            }
            let names = schema.get("enumNames").and_then(Value::as_array);
            Ok((
                values
                    .iter()
                    .enumerate()
                    .map(|(i, value)| {
                        option(
                            format!("enum:{i}"),
                            value.as_str().unwrap(),
                            names
                                .and_then(|names| names.get(i))
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        )
                    })
                    .collect(),
                false,
            ))
        }
        "string" if schema.get("oneOf").is_some() => {
            let values = schema
                .get("oneOf")
                .and_then(Value::as_array)
                .ok_or("enum 형식이 올바르지 않습니다")?;
            if values.is_empty() {
                return Err("enum 형식이 올바르지 않습니다".into());
            }
            let mut choices = Vec::with_capacity(values.len());
            for (index, value) in values.iter().enumerate() {
                choices.push(option(
                    format!("enum:{index}"),
                    string(value, "title")?,
                    string(value, "const")?,
                ));
            }
            Ok((choices, false))
        }
        "string" | "number" | "integer" => Ok((Vec::new(), true)),
        _ => Err("지원하지 않는 MCP 입력 스키마입니다".into()),
    }
}
fn form(value: &Value) -> Result<NormalizedNativeRequest, String> {
    reject_secrets(value)?;
    let schema = value
        .get("requestedSchema")
        .ok_or("MCP 입력 스키마가 없습니다")?;
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err("지원하지 않는 MCP 입력 스키마입니다".into());
    }
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or("MCP 입력 속성이 없습니다")?;
    if properties.is_empty() {
        return Err("MCP 입력 속성이 비어 있습니다".into());
    }
    if properties.contains_key(FORM_ACTION_ID) {
        return Err("MCP 입력 이름은 예약된 동작 이름을 사용할 수 없습니다".into());
    }
    let required: HashSet<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut questions = Vec::with_capacity(properties.len());
    for (name, field) in properties {
        if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
            return Err("MCP 입력 이름이 올바르지 않습니다".into());
        }
        let (mut choices, free_text) = property_options(field)?;
        if !required.contains(name.as_str()) {
            choices.push(option("skip", "건너뛰기", "선택 입력을 비워 둡니다"));
        }
        questions.push(question(
            name.clone(),
            opt_string(field, "title").unwrap_or_else(|| name.clone()),
            choices,
            free_text,
        ));
    }
    questions.push(question(
        FORM_ACTION_ID.into(),
        "입력을 제출할까요?".into(),
        approval_options(&["accept", "decline", "cancel"]),
        false,
    ));
    Ok(NormalizedNativeRequest {
        info: NativeRequestInfo {
            kind: NativeRequestKind::Form,
            title: "MCP 입력".into(),
            details: string(value, "message")?,
            blocking: true,
            url: None,
        },
        questions: Questions {
            kind: "clarification".into(),
            questions,
        },
    })
}
fn url(value: &Value) -> Result<NormalizedNativeRequest, String> {
    let url = string(value, "url")?;
    Ok(NormalizedNativeRequest {
        info: NativeRequestInfo {
            kind: NativeRequestKind::Url,
            title: "URL 열기".into(),
            details: string(value, "message")?,
            blocking: true,
            url: Some(url),
        },
        questions: approval_questions(&["accept", "decline", "cancel"]),
    })
}

pub fn normalize(method: &str, params: &Value) -> Result<NormalizedNativeRequest, String> {
    if serde_json::to_vec(params)
        .map_err(|error| error.to_string())?
        .len()
        > super::interaction::MAX_BYTES
    {
        return Err("native 요청 크기 제한을 초과했습니다".into());
    }
    reject_secrets(params)?;
    match method {
        TOOL_INPUT | ASYNC_MESSAGE => input(params),
        COMMAND_APPROVAL => approval(
            method,
            "명령 실행 승인",
            params,
            &["accept", "decline", "cancel"],
        ),
        FILE_APPROVAL => approval(
            method,
            "파일 변경 승인",
            params,
            &["accept", "decline", "cancel"],
        ),
        PERMISSIONS_APPROVAL => approval(method, "권한 승인", params, &["accept", "decline"]),
        MCP_ELICITATION => match string(params, "mode")?.as_str() {
            "form" | "openai/form" | "openaiForm" => form(params),
            "url" => url(params),
            "openai/userVerification" => Err("이 커넥터에는 장치 인증 증명이 필요합니다. Dojang은 아직 이 인증 방식을 지원하지 않습니다".into()),
            _ => Err("지원하지 않는 MCP 입력 방식입니다".into()),
        },
        _ => Err("지원하지 않는 native 사용자 요청입니다".into()),
    }
}

fn selected<'a>(answers: &'a [Answer], id: &str) -> Result<&'a Answer, String> {
    answers
        .iter()
        .find(|answer| answer.question_id == id)
        .ok_or_else(|| "native 답변이 누락되었습니다".into())
}
fn decision(answers: &[Answer], id: &str, allowed: &[&str]) -> Result<String, String> {
    let value = selected(answers, id)?
        .option_id
        .as_deref()
        .ok_or("승인 선택이 올바르지 않습니다")?;
    if allowed.contains(&value) {
        Ok(value.into())
    } else {
        Err("승인 선택이 올바르지 않습니다".into())
    }
}
pub fn is_negative_form_response(
    method: &str,
    params: &Value,
    answers: &[Answer],
) -> Result<bool, String> {
    if method != MCP_ELICITATION
        || !matches!(
            string(params, "mode")?.as_str(),
            "form" | "openai/form" | "openaiForm"
        )
    {
        return Ok(false);
    }
    let Some(action) = answers
        .iter()
        .find(|answer| answer.question_id == FORM_ACTION_ID)
    else {
        return Ok(false);
    };
    let action = action
        .option_id
        .as_deref()
        .ok_or("MCP 동작 선택이 올바르지 않습니다")?;
    if !matches!(action, "accept" | "decline" | "cancel") {
        return Err("MCP 동작 선택이 올바르지 않습니다".into());
    }
    if action == "accept" {
        return Ok(false);
    }
    if answers.len() != 1 {
        return Err("MCP 거절 또는 취소에는 동작만 답할 수 있습니다".into());
    }
    Ok(true)
}
fn input_response(params: &Value, answers: &[Answer]) -> Result<Value, String> {
    let raw_questions = params
        .get("questions")
        .and_then(Value::as_array)
        .ok_or("native 질문 기록이 손상되었습니다")?;
    let mut result = Map::new();
    for (index, raw) in raw_questions.iter().enumerate() {
        let id = string(raw, "id")?;
        let answer = selected(answers, &id)?;
        let text = match (&answer.option_id, &answer.text) {
            (Some(option_id), None) => {
                let choice_index = option_id
                    .strip_prefix(&format!("option:{index}:"))
                    .ok_or("native 선택지가 올바르지 않습니다")?
                    .parse::<usize>()
                    .map_err(|_| "native 선택지가 올바르지 않습니다")?;
                raw.get("options")
                    .and_then(Value::as_array)
                    .and_then(|choices| choices.get(choice_index))
                    .and_then(|choice| choice.get("label"))
                    .and_then(Value::as_str)
                    .ok_or("native 선택지가 올바르지 않습니다")?
                    .to_string()
            }
            (None, Some(text))
                if raw.get("isOther").and_then(Value::as_bool).unwrap_or(false)
                    || raw
                        .get("options")
                        .and_then(Value::as_array)
                        .is_none_or(Vec::is_empty) =>
            {
                text.clone()
            }
            _ => return Err("native 답변이 올바르지 않습니다".into()),
        };
        result.insert(id, json!({"answers": [text]}));
    }
    Ok(json!({"answers": result}))
}
fn form_response(params: &Value, answers: &[Answer]) -> Result<Value, String> {
    let action = decision(answers, FORM_ACTION_ID, &["accept", "decline", "cancel"])?;
    if action != "accept" {
        return Ok(json!({"action":action,"content":Value::Null}));
    }
    let schema = params
        .get("requestedSchema")
        .ok_or("MCP 입력 스키마가 없습니다")?;
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or("MCP 입력 속성이 없습니다")?;
    let required: HashSet<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut content = Map::new();
    for (name, field) in properties {
        let answer = selected(answers, name)?;
        if answer.option_id.as_deref() == Some("skip") && !required.contains(name.as_str()) {
            continue;
        }
        let ty = string(field, "type")?;
        let value = match (&answer.option_id, &answer.text, ty.as_str()) {
            (Some(id), None, "boolean") if id == "true" || id == "false" => {
                Value::Bool(id == "true")
            }
            (Some(id), None, "string") if id.starts_with("enum:") => {
                let index = id[5..]
                    .parse::<usize>()
                    .map_err(|_| "MCP enum 답변이 올바르지 않습니다")?;
                field
                    .get("enum")
                    .and_then(Value::as_array)
                    .and_then(|values| values.get(index))
                    .cloned()
                    .or_else(|| {
                        field
                            .get("oneOf")
                            .and_then(Value::as_array)
                            .and_then(|values| values.get(index))
                            .and_then(|value| value.get("const"))
                            .cloned()
                    })
                    .ok_or("MCP enum 답변이 올바르지 않습니다")?
            }
            (None, Some(text), "string") => Value::String(text.clone()),
            (None, Some(text), "number") => Value::Number(
                serde_json::Number::from_f64(
                    text.parse::<f64>()
                        .map_err(|_| "MCP 숫자 답변이 올바르지 않습니다")?,
                )
                .ok_or("MCP 숫자 답변이 올바르지 않습니다")?,
            ),
            (None, Some(text), "integer") => Value::Number(
                text.parse::<i64>()
                    .map_err(|_| "MCP 정수 답변이 올바르지 않습니다")?
                    .into(),
            ),
            _ => return Err("MCP 답변이 스키마와 다릅니다".into()),
        };
        match (&value, ty.as_str()) {
            (Value::String(text), "string") => {
                if field
                    .get("minLength")
                    .and_then(Value::as_u64)
                    .is_some_and(|minimum| text.chars().count() < minimum as usize)
                    || field
                        .get("maxLength")
                        .and_then(Value::as_u64)
                        .is_some_and(|maximum| text.chars().count() > maximum as usize)
                {
                    return Err("MCP 문자열 답변이 길이 제한을 벗어났습니다".into());
                }
            }
            (Value::Number(number), "number" | "integer") => {
                let number = number.as_f64().ok_or("MCP 숫자 답변이 올바르지 않습니다")?;
                if field
                    .get("minimum")
                    .and_then(Value::as_f64)
                    .is_some_and(|minimum| number < minimum)
                    || field
                        .get("maximum")
                        .and_then(Value::as_f64)
                        .is_some_and(|maximum| number > maximum)
                {
                    return Err("MCP 숫자 답변이 범위를 벗어났습니다".into());
                }
            }
            _ => {}
        }
        content.insert(name.clone(), value);
    }
    Ok(json!({"action":"accept", "content": content}))
}

pub fn response(method: &str, params: &Value, answers: &[Answer]) -> Result<Value, String> {
    match method {
        TOOL_INPUT | ASYNC_MESSAGE => input_response(params, answers),
        COMMAND_APPROVAL => Ok(
            json!({"decision": decision(answers, "decision", &available_decisions(params, &["accept", "decline", "cancel"])? )?}),
        ),
        FILE_APPROVAL => Ok(
            json!({"decision": decision(answers, "decision", &available_decisions(params, &["accept", "decline", "cancel"])? )?}),
        ),
        PERMISSIONS_APPROVAL => {
            let accepted = decision(answers, "decision", &["accept", "decline"])? == "accept";
            let permissions = if accepted {
                params
                    .get("permissions")
                    .cloned()
                    .ok_or("권한 요청 기록이 손상되었습니다")?
            } else {
                json!({})
            };
            Ok(json!({"permissions": permissions, "scope":"turn"}))
        }
        MCP_ELICITATION => match string(params, "mode")?.as_str() {
            "form" | "openai/form" | "openaiForm" => form_response(params, answers),
            "url" => Ok(
                json!({"action": decision(answers, "decision", &["accept", "decline", "cancel"])?}),
            ),
            _ => Err("지원하지 않는 MCP 입력 방식입니다".into()),
        },
        _ => Err("지원하지 않는 native 사용자 요청입니다".into()),
    }
}

#[cfg(test)]
#[path = "native_interaction_tests.rs"]
mod tests;
