use std::io::BufRead;
use std::path::{Path, PathBuf};

const MAX_SESSION_ENTRIES: usize = 20_000;
const MAX_MODEL_CHARS: usize = 160;

/// claude 세션 트랜스크립트에서 마지막으로 응답한 모델을 읽는다 — 스트림 관측(`system/init`)이
/// 실패했을 때의 2차 방어선. codex와 달리 claude는 스트림에 모델이 실리므로 **평소에는 쓰이지
/// 않는다**; 스트림 파싱이 어긋난 턴에서만 호출된다.
///
/// 여기서 얻는 값은 스트림보다 덜 구체적일 수 있다(트랜스크립트는 `claude-opus-5`, 스트림은
/// `claude-opus-5[1m]`처럼 컨텍스트 변형 접미사를 포함). 없는 것보다 낫다는 전제의 폴백이다.
pub(super) fn claude_model(session_id: &str) -> Option<String> {
    let root = claude_sessions_root()?;
    claude_model_in(&root, session_id)
}

fn claude_sessions_root() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(root).join("projects"));
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join(".claude").join("projects"))
}

fn claude_model_in(root: &Path, session_id: &str) -> Option<String> {
    if !safe_session_id(session_id) {
        return None;
    }
    // claude는 파일명이 세션 id 그 자체다 — 정확 일치를 요구해 인접 세션을 잘못 읽지 않는다.
    let exact = format!("{session_id}.jsonl");
    let session_file = find_session_file(root, |name| name == exact)?;
    latest_assistant_model(&session_file)
}

fn latest_assistant_model(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut latest = None;
    for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if event.get("type").and_then(serde_json::Value::as_str) != Some("assistant") {
            continue;
        }
        let model = event
            .get("message")
            .and_then(|message| message.get("model"))
            .and_then(serde_json::Value::as_str)
            .and_then(normalize_model);
        if let Some(model) = model {
            latest = Some(model);
        }
    }
    latest
}

pub(super) fn codex_model_in(root: &Path, session_id: &str) -> Option<String> {
    if !safe_session_id(session_id) {
        return None;
    }
    // codex 파일명은 `rollout-<시각>-<id>.jsonl` — 접미사로만 식별된다.
    let suffix = format!("{session_id}.jsonl");
    let session_file = find_session_file(root, |name| name.ends_with(&suffix))?;
    latest_turn_model(&session_file, session_id)
}

pub(super) fn safe_session_id(session_id: &str) -> bool {
    (16..=64).contains(&session_id.len())
        && session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// 세션 루트를 재귀 탐색해 `matches`를 만족하는 첫 파일을 찾는다. 벤더마다 파일명 규칙이
/// 달라(codex는 접두사가 붙고 claude는 id 그 자체) 판정을 호출자에게 맡긴다.
/// 방문 상한은 병적으로 큰 세션 디렉터리에서 탐색이 눌러앉지 않게 하는 안전장치다.
pub(super) fn find_session_file(root: &Path, matches: impl Fn(&str) -> bool) -> Option<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut visited = 0usize;
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            visited = visited.saturating_add(1);
            if visited > MAX_SESSION_ENTRIES {
                return None;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() && matches(&entry.file_name().to_string_lossy()) {
                return Some(entry.path());
            }
        }
    }
    None
}

fn latest_turn_model(path: &Path, session_id: &str) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut latest = None;
    let mut identity_matches = false;
    for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if event.get("type").and_then(serde_json::Value::as_str) == Some("session_meta") {
            identity_matches |= event
                .get("payload")
                .and_then(|payload| payload.get("id"))
                .and_then(serde_json::Value::as_str)
                == Some(session_id);
        }
        if event.get("type").and_then(serde_json::Value::as_str) != Some("turn_context") {
            continue;
        }
        let model = event
            .get("payload")
            .and_then(|payload| payload.get("model"))
            .and_then(serde_json::Value::as_str)
            .and_then(normalize_model);
        if let Some(model) = model {
            latest = Some(model);
        }
    }
    if !identity_matches {
        return None;
    }
    latest
}

/// 관측한 모델 이름을 기록 가능한 형태로 검증한다. 반환값은 `ModelSnapshot`으로만 흘러가
/// DB 기록·화면 표시에 쓰인다 — 파일 탐색 경로에는 들어가지 않는다(그쪽은 `safe_session_id`).
///
/// 대괄호는 컨텍스트 변형 접미사에 쓰인다(`claude-opus-5[1m]`). 화이트리스트에 없던 탓에
/// 그 모델로 돈 턴은 관측이 통째로 사라졌다 — 스냅샷이 아예 방출되지 않아 "기록 없음"과
/// "관측 실패"가 구분되지 않았다.
pub(super) fn normalize_model(model: &str) -> Option<String> {
    let model = model.trim();
    let valid = !model.is_empty()
        && model.chars().count() <= MAX_MODEL_CHARS
        && model
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/@+-[]".contains(&byte));
    valid.then(|| model.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::{claude_model_in, codex_model_in, normalize_model};

    static FIXTURE_SEQUENCE: AtomicU32 = AtomicU32::new(0);

    #[test]
    fn reads_last_assistant_model_from_the_claude_transcript() {
        let root = fixture_root("claude");
        let session_id = "cdb77def-ea16-4e92-ad5d-96fa297e5914";
        write_claude_session(
            &root,
            session_id,
            [
                r#"{"type":"user","message":{"role":"user"}}"#,
                r#"{"type":"assistant","message":{"model":"claude-opus-5","role":"assistant"}}"#,
                // 마지막 응답 모델이 그 턴의 답이다 — 중간에 바뀌면 나중 것을 취한다.
                r#"{"type":"assistant","message":{"model":"claude-opus-5[1m]","role":"assistant"}}"#,
            ],
        );

        assert_eq!(
            claude_model_in(&root, session_id).as_deref(),
            Some("claude-opus-5[1m]")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn claude_lookup_requires_an_exact_session_file_and_safe_id() {
        let root = fixture_root("claude-exact");
        let session_id = "cdb77def-ea16-4e92-ad5d-96fa297e5914";
        // 접미사만 겹치는 이웃 파일을 읽어오면 다른 세션의 모델을 이 턴에 귀속시키게 된다.
        write_claude_session(
            &root,
            &format!("backup-{session_id}"),
            [r#"{"type":"assistant","message":{"model":"wrong-model"}}"#],
        );

        assert_eq!(claude_model_in(&root, session_id), None);
        assert_eq!(claude_model_in(&root, "../config.json"), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn claude_transcript_without_assistant_model_yields_nothing() {
        let root = fixture_root("claude-empty");
        let session_id = "aaaabbbb-ea16-4e92-ad5d-96fa297e5914";
        write_claude_session(
            &root,
            session_id,
            [
                r#"{"type":"user","message":{"role":"user"}}"#,
                r#"not json"#,
                r#"{"type":"system","subtype":"init"}"#,
            ],
        );

        assert_eq!(claude_model_in(&root, session_id), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// claude는 cwd를 인코딩한 하위 디렉터리 아래에 `<session_id>.jsonl`로 저장한다.
    fn write_claude_session<'a>(
        root: &Path,
        file_stem: &str,
        lines: impl IntoIterator<Item = &'a str>,
    ) {
        let project = root.join("-Users-someone-work-repo");
        std::fs::create_dir_all(&project).unwrap();
        let content = lines.into_iter().collect::<Vec<_>>().join("\n");
        std::fs::write(project.join(format!("{file_stem}.jsonl")), content).unwrap();
    }

    #[test]
    fn accepts_context_variant_suffixes() {
        // 회귀: 대괄호가 화이트리스트에 없어 이 모델로 돈 턴의 관측이 통째로 유실됐다.
        assert_eq!(
            normalize_model("claude-opus-5[1m]").as_deref(),
            Some("claude-opus-5[1m]")
        );
        assert_eq!(normalize_model("opus[1m]").as_deref(), Some("opus[1m]"));
        assert_eq!(
            normalize_model("claude-fable-5").as_deref(),
            Some("claude-fable-5")
        );
        assert_eq!(
            normalize_model("anthropic/claude-opus-4-8").as_deref(),
            Some("anthropic/claude-opus-4-8")
        );
    }

    #[test]
    fn rejects_empty_oversized_and_unexpected_characters() {
        assert_eq!(normalize_model("   "), None);
        assert_eq!(normalize_model(&"a".repeat(161)), None);
        // 공백·따옴표·제어문자는 관측값이 아니라 파싱이 어긋났다는 신호다.
        assert_eq!(normalize_model("claude opus"), None);
        assert_eq!(normalize_model("model\"; DROP"), None);
        assert_eq!(normalize_model("model\nname"), None);
    }

    #[test]
    fn returns_latest_model_from_the_exact_codex_thread_file() {
        let root = fixture_root("latest");
        let session_id = "019f205d-33b8-7111-93b8-63fe9eaea592";
        write_session(
            &root,
            session_id,
            [
                r#"{"type":"session_meta","payload":{"id":"019f205d-33b8-7111-93b8-63fe9eaea592"}}"#,
                r#"{"type":"turn_context","payload":{"model":"gpt-5.5"}}"#,
                r#"{"type":"response_item","payload":{"model":"ignore-output-model"}}"#,
                r#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}"#,
            ],
        );

        assert_eq!(
            codex_model_in(&root, session_id).as_deref(),
            Some("gpt-5.6-sol")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ignores_unrelated_files_and_rejects_unsafe_session_ids() {
        let root = fixture_root("unsafe");
        write_session(
            &root,
            "019f205d-33b8-7111-93b8-63fe9eaea592",
            [
                r#"{"type":"session_meta","payload":{"id":"019f205d-33b8-7111-93b8-63fe9eaea592"}}"#,
                r#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}"#,
            ],
        );
        let spoofed_id = "019f205d-33b8-7111-93b8-63fe9eaea593";
        write_session(
            &root,
            spoofed_id,
            [
                r#"{"type":"session_meta","payload":{"id":"different-thread"}}"#,
                r#"{"type":"turn_context","payload":{"model":"spoofed-model"}}"#,
            ],
        );

        assert_eq!(codex_model_in(&root, "../config.toml"), None);
        assert_eq!(codex_model_in(&root, spoofed_id), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn fixture_root(label: &str) -> PathBuf {
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::SeqCst);
        let root = crate::testtmp::dir().join(format!(
            "praxis-model-observation-{}-{sequence}-{label}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_session<'a>(root: &Path, session_id: &str, lines: impl IntoIterator<Item = &'a str>) {
        let dated = root.join("2026").join("07").join("23");
        std::fs::create_dir_all(&dated).unwrap();
        let path = dated.join(format!("rollout-2026-07-23T00-00-00-{session_id}.jsonl"));
        let content = lines.into_iter().collect::<Vec<_>>().join("\n");
        std::fs::write(path, content).unwrap();
    }
}
