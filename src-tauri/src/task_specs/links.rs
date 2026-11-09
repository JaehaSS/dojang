//! Current-worktree Markdown link checks. Results are transient and never alter snapshots.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::{db, fsapi, project_paths};

const MAX_LINKS: usize = 100;
const MAX_TARGET_BYTES: usize = 2_048;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpecLinkState {
    Ok,
    Missing,
    OutsideScope,
    Unsupported,
    AnchorMissing,
    Unreadable,
    External,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecLink {
    pub source_line: u32,
    pub target: String,
    pub state: SpecLinkState,
    pub resolved_path: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecLinkSummary {
    pub total: u32,
    pub ok: u32,
    pub missing: u32,
    pub outside_scope: u32,
    pub unsupported: u32,
    pub anchor_missing: u32,
    pub unreadable: u32,
    pub external: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecLinkReport {
    pub relative_path: String,
    pub content_hash: String,
    pub checked_at: i64,
    pub links: Vec<SpecLink>,
    pub summary: SpecLinkSummary,
    pub truncated: bool,
}

/// Validate only the current file content. The hash is an optimistic lock so a report cannot
/// silently describe a newer document than the UI displayed.
pub async fn validate(
    pool: &SqlitePool,
    task_id: i64,
    relative_path: &str,
    expected_hash: &str,
) -> Result<SpecLinkReport, String> {
    super::validate_hash(expected_hash)?;
    let task = db::get_task(pool, task_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "작업을 찾을 수 없습니다".to_string())?;
    let root = PathBuf::from(task.worktree_path);
    let relative_path = relative_path.to_string();
    let expected_hash = expected_hash.to_string();
    tokio::task::spawn_blocking(move || validate_blocking(&root, &relative_path, &expected_hash))
        .await
        .map_err(|_| "문서 링크 검사 작업이 중단되었습니다".to_string())?
}

fn validate_blocking(
    root: &Path,
    relative_path: &str,
    expected_hash: &str,
) -> Result<SpecLinkReport, String> {
    let document = super::read_document(root, relative_path)?;
    if document.content_hash != expected_hash {
        return Err("원본 명세가 변경되었습니다. 새로고침한 뒤 다시 검사하세요".into());
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|_| "작업 워크트리를 해석할 수 없습니다".to_string())?;
    let source_path = fsapi::safe_join(&canonical_root, &document.relative_path)
        .map_err(|error| error.to_string())?;
    let source_parent = source_path.parent().unwrap_or(&canonical_root);
    let (targets, headings) = parse_document(&document.content);
    let mut summary = SpecLinkSummary::default();
    let mut links = Vec::with_capacity(targets.len().min(MAX_LINKS));
    let truncated = targets.len() > MAX_LINKS;
    for target in targets.into_iter().take(MAX_LINKS) {
        let (state, resolved_path) = check_target(
            &canonical_root,
            source_parent,
            &source_path,
            &headings,
            &target.target,
        );
        let link = SpecLink {
            source_line: target.line,
            target: target.target,
            state,
            resolved_path,
        };
        count(&mut summary, &link.state);
        links.push(link);
    }
    Ok(SpecLinkReport {
        relative_path: document.relative_path,
        content_hash: document.content_hash,
        checked_at: now(),
        links,
        summary,
        truncated,
    })
}

#[derive(Debug)]
struct LinkTarget {
    line: u32,
    target: String,
}

fn parse_document(content: &str) -> (Vec<LinkTarget>, HashSet<String>) {
    let mut targets = Vec::new();
    let mut headings = HashSet::new();
    let mut heading_text: Option<String> = None;
    let mut duplicate_slugs = std::collections::HashMap::<String, u32>::new();
    for (event, range) in Parser::new_ext(content, Options::all()).into_offset_iter() {
        match event {
            Event::Start(Tag::Link { dest_url, .. })
            | Event::Start(Tag::Image { dest_url, .. }) => {
                targets.push(LinkTarget {
                    line: line_at(content, range.start),
                    target: dest_url.into_string(),
                });
            }
            Event::Start(Tag::Heading { .. }) => heading_text = Some(String::new()),
            Event::Text(text) | Event::Code(text) => {
                if let Some(heading) = heading_text.as_mut() {
                    heading.push_str(&text);
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(heading) = heading_text.take() {
                    let base = heading_slug(&heading);
                    if !base.is_empty() {
                        let count = duplicate_slugs.entry(base.clone()).or_insert(0);
                        let slug = if *count == 0 {
                            base
                        } else {
                            format!("{base}-{count}")
                        };
                        *count += 1;
                        headings.insert(slug);
                    }
                }
            }
            _ => {}
        }
    }
    (targets, headings)
}

fn line_at(content: &str, byte: usize) -> u32 {
    (content.as_bytes()[..byte.min(content.len())]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        + 1) as u32
}

fn check_target(
    root: &Path,
    source_parent: &Path,
    source_path: &Path,
    source_headings: &HashSet<String>,
    raw_target: &str,
) -> (SpecLinkState, Option<String>) {
    if raw_target.len() > MAX_TARGET_BYTES {
        return (SpecLinkState::Unsupported, None);
    }
    if is_external(raw_target) {
        return (SpecLinkState::External, None);
    }
    if raw_target.contains('?') {
        return (SpecLinkState::Unsupported, None);
    }
    let (raw_path, raw_fragment) = raw_target.split_once('#').unwrap_or((raw_target, ""));
    let path = match percent_decode(raw_path) {
        Some(path) => path,
        None => return (SpecLinkState::Unsupported, None),
    };
    let fragment = match percent_decode(raw_fragment) {
        Some(fragment) => fragment,
        None => return (SpecLinkState::Unsupported, None),
    };
    if path.is_empty() {
        return check_anchor(source_headings, &fragment, None);
    }
    let Some(candidate) = resolve_local_path(root, source_parent, &path) else {
        return (SpecLinkState::OutsideScope, None);
    };
    let relative = display_path(root, &candidate);
    if protected_path(&relative) || has_symlink_component(root, &candidate) {
        return (SpecLinkState::OutsideScope, relative);
    }
    let metadata = match candidate.metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (SpecLinkState::Missing, relative)
        }
        Err(_) => return (SpecLinkState::Unreadable, relative),
    };
    if !metadata.is_file() {
        return (SpecLinkState::Unsupported, relative);
    }
    if fragment.is_empty() {
        return (SpecLinkState::Ok, relative);
    }
    if candidate == source_path {
        return check_anchor(source_headings, &fragment, relative);
    }
    if !is_markdown(&candidate) {
        return (SpecLinkState::Unsupported, relative);
    }
    match super::read_document(root, relative.as_deref().unwrap_or_default()) {
        Ok(document) => {
            let (_, headings) = parse_document(&document.content);
            check_anchor(&headings, &fragment, relative)
        }
        Err(_) => (SpecLinkState::Unreadable, relative),
    }
}

fn check_anchor(
    headings: &HashSet<String>,
    fragment: &str,
    path: Option<String>,
) -> (SpecLinkState, Option<String>) {
    if fragment.is_empty() || headings.contains(fragment) {
        (SpecLinkState::Ok, path)
    } else {
        (SpecLinkState::AnchorMissing, path)
    }
}

fn is_external(target: &str) -> bool {
    if target.starts_with("//") {
        return true;
    }
    let Some(colon) = target.find(':') else {
        return false;
    };
    let scheme = &target[..colon];
    scheme
        .as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_alphabetic())
        && scheme
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
}

fn resolve_local_path(root: &Path, source_parent: &Path, decoded: &str) -> Option<PathBuf> {
    if (decoded.contains('\0') || decoded.starts_with("//") || Path::new(decoded).has_root())
        && (!decoded.starts_with('/') || decoded.starts_with("//"))
    {
        return None;
    }
    let mut pieces: Vec<PathBuf> = if decoded.starts_with('/') {
        Vec::new()
    } else {
        source_parent
            .strip_prefix(root)
            .ok()?
            .components()
            .filter_map(|component| match component {
                Component::Normal(part) => Some(PathBuf::from(part)),
                _ => None,
            })
            .collect()
    };
    for component in Path::new(decoded.trim_start_matches('/')).components() {
        match component {
            Component::Normal(part) => pieces.push(PathBuf::from(part)),
            Component::CurDir => {}
            Component::ParentDir => {
                pieces.pop()?;
            }
            _ => return None,
        }
    }
    let mut relative = PathBuf::new();
    for part in pieces {
        relative.push(part);
    }
    fsapi::safe_join(root, &relative.to_string_lossy()).ok()
}

fn protected_path(relative: &Option<String>) -> bool {
    relative.as_deref().is_some_and(|path| {
        project_paths::is_internal_path(path) || path.split('/').any(|part| part == ".git")
    })
}

fn has_symlink_component(root: &Path, candidate: &Path) -> bool {
    let Ok(relative) = candidate.strip_prefix(root) else {
        return true;
    };
    let mut cursor = root.to_path_buf();
    for component in relative.components() {
        cursor.push(component);
        match cursor.symlink_metadata() {
            Ok(metadata) if metadata.file_type().is_symlink() => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

fn display_path(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|value| value.to_string_lossy().replace('\\', "/"))
}

fn is_markdown(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .as_deref(),
        Some("md" | "markdown" | "mdown" | "mkdn")
    )
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] != b'%' {
            output.push(bytes[cursor]);
            cursor += 1;
            continue;
        }
        if cursor + 2 >= bytes.len() {
            return None;
        }
        let hi = hex(bytes[cursor + 1])?;
        let lo = hex(bytes[cursor + 2])?;
        output.push(hi << 4 | lo);
        cursor += 3;
    }
    String::from_utf8(output).ok()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn heading_slug(value: &str) -> String {
    let mut output = String::new();
    let mut hyphen = false;
    for character in value.trim().chars() {
        if character.is_whitespace() || character == '-' {
            hyphen = !output.is_empty();
        } else if character.is_alphanumeric() || character == '_' {
            if hyphen {
                output.push('-');
                hyphen = false;
            }
            for lower in character.to_lowercase() {
                output.push(lower);
            }
        }
    }
    output
}

fn count(summary: &mut SpecLinkSummary, state: &SpecLinkState) {
    summary.total += 1;
    match state {
        SpecLinkState::Ok => summary.ok += 1,
        SpecLinkState::Missing => summary.missing += 1,
        SpecLinkState::OutsideScope => summary.outside_scope += 1,
        SpecLinkState::Unsupported => summary.unsupported += 1,
        SpecLinkState::AnchorMissing => summary.anchor_missing += 1,
        SpecLinkState::Unreadable => summary.unreadable += 1,
        SpecLinkState::External => summary.external += 1,
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_generated_heading_ids() {
        let (_, headings) = parse_document("# Present heading\n");
        assert!(headings.contains("present-heading"), "{headings:?}");
    }
}
