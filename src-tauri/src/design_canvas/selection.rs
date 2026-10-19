use std::collections::BTreeMap;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

const MAX_SOURCE: usize = 1_024;
const MAX_CLASS_NAME: usize = 4_096;
const MAX_TAG_NAME: usize = 64;
const MAX_CSS_ENTRIES: usize = 64;
const MAX_CSS_VALUE: usize = 1_024;
const MAX_REQUEST_ID: usize = 128;
const MAX_RECT_VALUE: f64 = 1_000_000.0;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClassKind {
    Static,
    Dynamic,
    None,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SuggestedAncestor {
    pub source: String,
    pub class_name: String,
    pub class_kind: ClassKind,
    pub tag_name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DesignCanvasSelection {
    pub version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub source: Option<String>,
    /// 선택된 공용 컴포넌트 인스턴스의 JSX 사용처. 없으면 정의 위치만 편집 후보가 된다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_source: Option<String>,
    pub class_name: String,
    pub class_kind: ClassKind,
    pub tag_name: String,
    pub computed_css: BTreeMap<String, String>,
    pub bounding_rect: BoundingRect,
    pub suggested_ancestor: Option<SuggestedAncestor>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BoundingRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Page-controlled metadata is accepted only as bounded display data. `source` is a relative
/// source-position string, so validation never opens or follows a path supplied by the page.
pub fn validate_selection(
    worktree: &Path,
    selection: DesignCanvasSelection,
) -> Result<DesignCanvasSelection, &'static str> {
    if selection.version != 1
        || !safe_class_name(&selection.class_name)
        || !safe_tag_name(&selection.tag_name)
        || !safe_rect(&selection.bounding_rect)
        || selection.computed_css.len() > MAX_CSS_ENTRIES
    {
        return Err("invalid selection metadata");
    }
    if selection
        .request_id
        .as_deref()
        .is_some_and(|request_id| request_id.is_empty() || !safe_text(request_id, MAX_REQUEST_ID))
    {
        return Err("invalid selection request id");
    }
    if let Some(source) = &selection.source {
        validate_source_position(worktree, source)?;
    }
    if let Some(source) = &selection.instance_source {
        validate_source_position(worktree, source)?;
    }
    if let Some(ancestor) = &selection.suggested_ancestor {
        validate_source_position(worktree, &ancestor.source)?;
        if !safe_class_name(&ancestor.class_name) || !safe_tag_name(&ancestor.tag_name) {
            return Err("invalid suggested ancestor metadata");
        }
    }
    for (property, value) in &selection.computed_css {
        if !crate::designmode::CSS_WHITELIST.contains(&property.as_str())
            || !safe_text(value, MAX_CSS_VALUE)
        {
            return Err("invalid computed CSS metadata");
        }
    }
    Ok(selection)
}

fn safe_text(value: &str, max_len: usize) -> bool {
    value.len() <= max_len && !value.chars().any(char::is_control)
}

fn safe_class_name(value: &str) -> bool {
    value.len() <= MAX_CLASS_NAME
        && !value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

fn safe_tag_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TAG_NAME
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn safe_rect(rect: &BoundingRect) -> bool {
    [rect.x, rect.y, rect.width, rect.height]
        .into_iter()
        .all(|value| value.is_finite() && value.abs() <= MAX_RECT_VALUE)
        && rect.width >= 0.0
        && rect.height >= 0.0
}

fn validate_source_position(worktree: &Path, source: &str) -> Result<(), &'static str> {
    if source.is_empty() || source.len() > MAX_SOURCE || source.chars().any(char::is_control) {
        return Err("invalid source metadata");
    }
    let Some((path, line, column)) = split_source_position(source) else {
        return Err("invalid source position");
    };
    // Babel `loc.start.column` is zero-based, while lines are one-based.
    if line == 0 || line > 1_000_000 || column > 1_000_000 {
        return Err("invalid source position");
    }
    let relative = Path::new(path);
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("source is outside the current worktree");
    }
    // This lexical check deliberately does not read the page-provided target. The resulting
    // path is only a read-only selector for a later, separately authorised edit operation.
    if !worktree.join(relative).starts_with(worktree) {
        return Err("source is outside the current worktree");
    }
    Ok(())
}

fn split_source_position(source: &str) -> Option<(&str, u32, u32)> {
    let (path_and_line, column) = source.rsplit_once(':')?;
    let (path, line) = path_and_line.rsplit_once(':')?;
    Some((path, line.parse().ok()?, column.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection() -> DesignCanvasSelection {
        DesignCanvasSelection {
            version: 1,
            request_id: Some("request-1".into()),
            source: Some("src/design/card.tsx:12:4".into()),
            instance_source: None,
            class_name: "p-4 text-sm".into(),
            class_kind: ClassKind::Static,
            tag_name: "div".into(),
            computed_css: BTreeMap::from([("display".into(), "block".into())]),
            bounding_rect: BoundingRect {
                x: 1.0,
                y: 2.0,
                width: 3.0,
                height: 4.0,
            },
            suggested_ancestor: None,
        }
    }

    #[test]
    fn accepts_bounded_relative_selection_without_reading_the_source_file() {
        let value = selection();
        assert_eq!(
            validate_selection(Path::new("/worktree"), value.clone()).unwrap(),
            value
        );
    }

    #[test]
    fn rejects_external_or_malformed_source_positions() {
        for source in [
            "/etc/passwd:1:1",
            "../outside.tsx:1:1",
            "src/a.tsx:0:4",
            "src/a.tsx:1:x",
        ] {
            let mut value = selection();
            value.source = Some(source.into());
            assert!(
                validate_selection(Path::new("/worktree"), value).is_err(),
                "{source}"
            );
        }
    }

    #[test]
    fn accepts_zero_based_babel_columns() {
        let mut value = selection();
        value.source = Some("src/design/card.tsx:12:0".into());
        assert!(validate_selection(Path::new("/worktree"), value).is_ok());
    }

    #[test]
    fn allows_html_class_whitespace_but_rejects_other_controls() {
        let mut value = selection();
        value.class_name = "p-4\n\ttext-sm\r rounded".into();
        assert!(validate_selection(Path::new("/worktree"), value).is_ok());
        let mut value = selection();
        value.class_name = "p-4\0text-sm".into();
        assert!(validate_selection(Path::new("/worktree"), value).is_err());
    }

    #[test]
    fn rejects_non_finite_rect_and_unapproved_css() {
        let mut value = selection();
        value.bounding_rect.width = f64::NAN;
        assert!(validate_selection(Path::new("/worktree"), value).is_err());

        let mut value = selection();
        value
            .computed_css
            .insert("background-image".into(), "url(x)".into());
        assert!(validate_selection(Path::new("/worktree"), value).is_err());
    }

    #[test]
    fn keeps_optional_request_id_but_rejects_oversized_or_control_values() {
        let value = selection();
        assert_eq!(
            validate_selection(Path::new("/worktree"), value)
                .unwrap()
                .request_id
                .as_deref(),
            Some("request-1")
        );
        let mut value = selection();
        value.request_id = Some("\u{0000}".into());
        assert!(validate_selection(Path::new("/worktree"), value).is_err());
        let mut value = selection();
        value.request_id = Some("x".repeat(MAX_REQUEST_ID + 1));
        assert!(validate_selection(Path::new("/worktree"), value).is_err());
    }
}
