//! 리뷰 노트·이동 툴. 브라우저 툴과 같은 서버에 살지만 웹뷰로 가지 않는다 — 상대는 주석
//! 저장소와 Diff 탭이다. 인자 검증과 hunk 찾기는 여기서 끝나고, DB·이벤트는 `ReviewBridge`
//! 뒤의 Tauri 쪽이 맡는다(hunk 터미널 뷰어의 `session comment`·`session navigate`에 해당).

use serde_json::{json, Value};

use crate::diffmodel::{DiffHunk, DiffLine};

pub const COMMENT_ADD: &str = "review_comment_add";
pub const COMMENT_LIST: &str = "review_comment_list";
pub const NAVIGATE: &str = "review_navigate";

/// 노트 본문 상한. 리뷰 노트는 한두 문단이지 보고서가 아니다.
const MAX_BODY_BYTES: usize = 8 * 1024;
const MAX_PATH_CHARS: usize = 1024;

const INVALID_PATH: (i64, &str) = (-32000, "invalid_argument: path");
const INVALID_LINE: (i64, &str) = (-32000, "invalid_argument: line");
const INVALID_SIDE: (i64, &str) = (-32000, "invalid_argument: side");
const INVALID_BODY: (i64, &str) = (-32000, "invalid_argument: body");
const INVALID_HUNK: (i64, &str) = (-32000, "invalid_argument: hunk");

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewCommand {
    CommentAdd {
        path: String,
        line: i64,
        /// `old`·`new`. 생략하면 `new` — 변경 후 파일의 줄 번호다.
        side: String,
        body: String,
    },
    CommentList {
        path: Option<String>,
    },
    Navigate {
        path: String,
        target: NavigateTarget,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigateTarget {
    /// 파일 안 순번, 1부터.
    Hunk(usize),
    Line { line: i64, side: String },
    /// 파일의 첫 hunk.
    File,
}

impl ReviewCommand {
    pub fn is_review_tool(name: &str) -> bool {
        matches!(name, COMMENT_ADD | COMMENT_LIST | NAVIGATE)
    }

    /// 인자 검증은 여기서 끝난다 — 실패는 (JSON-RPC 코드, 메시지)로, 브라우저 툴과 같은 모양이다.
    pub fn from_tool_call(name: &str, arguments: &Value) -> Result<Self, (i64, &'static str)> {
        match name {
            COMMENT_ADD => Ok(ReviewCommand::CommentAdd {
                path: path_arg(arguments)?,
                line: line_arg(arguments)?.ok_or(INVALID_LINE)?,
                side: side_arg(arguments)?,
                body: body_arg(arguments)?,
            }),
            COMMENT_LIST => Ok(ReviewCommand::CommentList {
                path: match arguments.get("path") {
                    None | Some(Value::Null) => None,
                    Some(_) => Some(path_arg(arguments)?),
                },
            }),
            NAVIGATE => Ok(ReviewCommand::Navigate {
                path: path_arg(arguments)?,
                target: navigate_target(arguments)?,
            }),
            _ => Err((-32601, "unknown tool")),
        }
    }

    pub fn op(&self) -> &'static str {
        match self {
            ReviewCommand::CommentAdd { .. } => COMMENT_ADD,
            ReviewCommand::CommentList { .. } => COMMENT_LIST,
            ReviewCommand::Navigate { .. } => NAVIGATE,
        }
    }
}

fn navigate_target(arguments: &Value) -> Result<NavigateTarget, (i64, &'static str)> {
    let hunk = match arguments.get("hunk") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .filter(|n| *n >= 1)
                .map(|n| n as usize)
                .ok_or(INVALID_HUNK)?,
        ),
    };
    let line = line_arg(arguments)?;
    match (hunk, line) {
        (Some(_), Some(_)) => Err(INVALID_HUNK),
        (Some(n), None) => Ok(NavigateTarget::Hunk(n)),
        (None, Some(line)) => Ok(NavigateTarget::Line {
            line,
            side: side_arg(arguments)?,
        }),
        (None, None) => Ok(NavigateTarget::File),
    }
}

fn path_arg(arguments: &Value) -> Result<String, (i64, &'static str)> {
    arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| (1..=MAX_PATH_CHARS).contains(&path.chars().count()))
        .map(str::to_string)
        .ok_or(INVALID_PATH)
}

/// 줄 번호는 1 이상의 정수다. 없으면 None — 필수 여부는 호출부가 정한다.
fn line_arg(arguments: &Value) -> Result<Option<i64>, (i64, &'static str)> {
    match arguments.get("line") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .filter(|line| *line >= 1)
            .map(Some)
            .ok_or(INVALID_LINE),
    }
}

fn side_arg(arguments: &Value) -> Result<String, (i64, &'static str)> {
    match arguments.get("side") {
        None | Some(Value::Null) => Ok("new".to_string()),
        Some(value) => value
            .as_str()
            .filter(|side| matches!(*side, "old" | "new"))
            .map(str::to_string)
            .ok_or(INVALID_SIDE),
    }
}

fn body_arg(arguments: &Value) -> Result<String, (i64, &'static str)> {
    arguments
        .get("body")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|body| !body.is_empty() && body.len() <= MAX_BODY_BYTES)
        .map(str::to_string)
        .ok_or(INVALID_BODY)
}

/// 한 파일의 hunk들 — diff 순서 그대로다. 순번(1부터)은 이 순서 위에서 센다.
pub fn hunks_of<'a>(hunks: &'a [DiffHunk], path: &str) -> Vec<&'a DiffHunk> {
    hunks.iter().filter(|hunk| hunk.path == path).collect()
}

/// (path, line, side)를 품은 hunk와 파일 안 순번. 범위 판정은 `annotations::rematch`와 같은
/// 규칙이다 — count 0인 범위도 시작 줄 하나는 품는다.
pub fn locate_hunk<'a>(
    hunks: &'a [DiffHunk],
    path: &str,
    line: i64,
    side: &str,
) -> Option<(usize, &'a DiffHunk)> {
    hunks_of(hunks, path)
        .into_iter()
        .enumerate()
        .find(|(_, hunk)| contains_line(hunk, line, side))
        .map(|(index, hunk)| (index + 1, hunk))
}

fn contains_line(hunk: &DiffHunk, line: i64, side: &str) -> bool {
    let (start, count) = if side == "old" {
        hunk.old_range
    } else {
        hunk.new_range
    };
    let start = start as i64;
    let end = start + (count as i64).max(1) - 1;
    (start..=end).contains(&line)
}

/// 노트가 실제로 그려질 자리. 화면(`hunkLines`)은 문맥 줄과 추가 줄을 new 번호로, 삭제 줄만 old
/// 번호로 그린다. old 번호로 가리킨 문맥 줄은 그 규칙대로 new 번호로 옮긴다 — 범위 안이라고 그대로
/// 저장하면 `rematch`는 멀쩡하다 하고 화면에는 영영 안 보인다.
pub fn note_position(hunk: &DiffHunk, line: i64, side: &str) -> Option<(i64, &'static str)> {
    let mut old_line = hunk.old_range.0 as i64;
    let mut new_line = hunk.new_range.0 as i64;
    for diff_line in &hunk.lines {
        match diff_line {
            DiffLine::Context(_) => {
                if (side == "old" && old_line == line) || (side != "old" && new_line == line) {
                    return Some((new_line, "new"));
                }
                old_line += 1;
                new_line += 1;
            }
            DiffLine::Del(_) => {
                if side == "old" && old_line == line {
                    return Some((line, "old"));
                }
                old_line += 1;
            }
            DiffLine::Add(_) => {
                if side != "old" && new_line == line {
                    return Some((line, "new"));
                }
                new_line += 1;
            }
        }
    }
    None
}

/// 파일 안 n번째(1부터) hunk.
pub fn nth_hunk<'a>(hunks: &'a [DiffHunk], path: &str, n: usize) -> Option<&'a DiffHunk> {
    hunks_of(hunks, path).into_iter().nth(n.checked_sub(1)?)
}

/// hunk id의 파일 안 순번(1부터). 목록 응답이 "hunk 2/3"처럼 보여 주려고 쓴다.
pub fn hunk_number(hunks: &[DiffHunk], path: &str, hunk_id: &str) -> Option<usize> {
    hunks_of(hunks, path)
        .iter()
        .position(|hunk| hunk.id == hunk_id)
        .map(|index| index + 1)
}

pub fn hunk_header(hunk: &DiffHunk) -> String {
    format!(
        "@@ -{},{} +{},{} @@",
        hunk.old_range.0, hunk.old_range.1, hunk.new_range.0, hunk.new_range.1
    )
}

/// 툴 설명이 곧 에이전트에게 주는 리뷰 규칙이다 — 별도 스킬 파일 없이 여기서 끝낸다.
pub fn descriptors() -> Vec<Value> {
    vec![
        json!({
            "name": COMMENT_ADD,
            "description": "Attach a review note to one line of this task's diff. The user sees it inline in their Diff view, next to that line, labelled with your name. Use it for what the user would not notice alone: a bug, a risk, a trade-off that needs their decision. Do not narrate what the diff plainly shows, and do not use it to talk to yourself — notes are not sent back to you. Only lines inside a changed hunk accept a note (what `git diff` against the session base shows, context lines included); other lines fail with line_not_in_diff. Call review_navigate first when the user should look at the spot right now.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Repository-relative path exactly as it appears in the diff." },
                    "line": { "type": "integer", "minimum": 1, "description": "1-based line number. Counts in the new file unless side is old." },
                    "side": { "type": "string", "enum": ["new", "old"], "description": "old only for a removed line, numbered in the old file. Default new." },
                    "body": { "type": "string", "description": "Markdown, up to 8 KiB. One finding per note." },
                },
                "required": ["path", "line", "body"],
                "additionalProperties": false,
            },
        }),
        json!({
            "name": COMMENT_LIST,
            "description": "List review notes on this task's diff — the user's notes (author human) and agent notes — with status (draft, sent, resolved) and whether a note is orphaned because its hunk no longer exists. Read the user's notes to learn what they asked for before changing code; a user note stays draft until they send it to you as a message.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Only notes on this repository-relative path. Omit for all files." },
                },
                "additionalProperties": false,
            },
        }),
        json!({
            "name": NAVIGATE,
            "description": "Scroll the user's Diff view to a hunk of this task's diff, opening the file's diff tab when it is not open. Target one hunk by its 1-based number within the file (diff order) or by a line inside it; with neither, the file's first hunk. Returns the hunk that was targeted. Fails with path_not_in_diff or hunk_not_found when the target is not part of the diff. Nothing moves if the user is looking at another task.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Repository-relative path exactly as it appears in the diff." },
                    "hunk": { "type": "integer", "minimum": 1, "description": "1-based hunk number within the file. Not with line." },
                    "line": { "type": "integer", "minimum": 1, "description": "1-based line inside the hunk to show. Not with hunk." },
                    "side": { "type": "string", "enum": ["new", "old"], "description": "Which file line numbers: default new." },
                },
                "required": ["path"],
                "additionalProperties": false,
            },
        }),
    ]
}

/// 리뷰 명령의 실제 구현. `Ok`는 툴 결과 본문, `Err`는 에이전트가 그대로 읽는 실패 사유
/// (`isError` 결과)다 — 둘 다 호출 자체는 성공했다.
#[async_trait::async_trait]
pub trait ReviewBridge: Send + Sync {
    async fn run(&self, task_id: i64, author: &str, cmd: ReviewCommand) -> Result<String, String>;
}

/// 리뷰 저장소가 없는 서버(테스트·축소 구성)가 쓰는 구현.
pub struct NoReview;

#[async_trait::async_trait]
impl ReviewBridge for NoReview {
    async fn run(&self, _task_id: i64, _author: &str, _cmd: ReviewCommand) -> Result<String, String> {
        Err("review_unavailable".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diffmodel::RiskLevel;

    fn hunk(id: &str, path: &str, old_range: (u32, u32), new_range: (u32, u32)) -> DiffHunk {
        DiffHunk {
            id: id.into(),
            path: path.into(),
            old_range,
            new_range,
            lines: vec![DiffLine::Add("x".into())],
            protected: false,
            committed: false,
            risk: RiskLevel::Low,
        }
    }

    fn sample() -> Vec<DiffHunk> {
        vec![
            hunk("a1", "src/a.ts", (9, 2), (9, 3)),
            hunk("b1", "src/b.ts", (1, 0), (1, 4)),
            hunk("a2", "src/a.ts", (40, 1), (41, 1)),
        ]
    }

    #[test]
    fn comment_add_defaults_side_to_new_and_trims_body() {
        let cmd = ReviewCommand::from_tool_call(
            COMMENT_ADD,
            &json!({ "path": "src/a.ts", "line": 10, "body": "  nil check missing  " }),
        )
        .unwrap();
        assert_eq!(
            cmd,
            ReviewCommand::CommentAdd {
                path: "src/a.ts".into(),
                line: 10,
                side: "new".into(),
                body: "nil check missing".into(),
            }
        );
    }

    #[test]
    fn comment_add_rejects_bad_line_side_and_body() {
        let call = |args| ReviewCommand::from_tool_call(COMMENT_ADD, &args);
        assert_eq!(call(json!({ "path": "a", "body": "x" })), Err(INVALID_LINE));
        assert_eq!(call(json!({ "path": "a", "line": 0, "body": "x" })), Err(INVALID_LINE));
        assert_eq!(call(json!({ "path": "a", "line": 1, "side": "left", "body": "x" })), Err(INVALID_SIDE));
        assert_eq!(call(json!({ "path": "a", "line": 1, "body": "   " })), Err(INVALID_BODY));
        let long = "x".repeat(MAX_BODY_BYTES + 1);
        assert_eq!(call(json!({ "path": "a", "line": 1, "body": long })), Err(INVALID_BODY));
        assert_eq!(call(json!({ "path": "", "line": 1, "body": "x" })), Err(INVALID_PATH));
    }

    #[test]
    fn navigate_takes_hunk_or_line_but_not_both() {
        let call = |args| ReviewCommand::from_tool_call(NAVIGATE, &args);
        assert!(matches!(
            call(json!({ "path": "a", "hunk": 2 })),
            Ok(ReviewCommand::Navigate { target: NavigateTarget::Hunk(2), .. })
        ));
        assert!(matches!(
            call(json!({ "path": "a", "line": 7 })),
            Ok(ReviewCommand::Navigate { target: NavigateTarget::Line { line: 7, .. }, .. })
        ));
        assert!(matches!(
            call(json!({ "path": "a" })),
            Ok(ReviewCommand::Navigate { target: NavigateTarget::File, .. })
        ));
        assert_eq!(call(json!({ "path": "a", "hunk": 1, "line": 1 })), Err(INVALID_HUNK));
        assert_eq!(call(json!({ "path": "a", "hunk": 0 })), Err(INVALID_HUNK));
    }

    #[test]
    fn comment_list_path_is_optional_but_validated_when_present() {
        assert_eq!(
            ReviewCommand::from_tool_call(COMMENT_LIST, &json!({})),
            Ok(ReviewCommand::CommentList { path: None })
        );
        assert_eq!(
            ReviewCommand::from_tool_call(COMMENT_LIST, &json!({ "path": "" })),
            Err(INVALID_PATH)
        );
    }

    #[test]
    fn locate_hunk_numbers_within_the_file_in_diff_order() {
        let hunks = sample();
        let (n, hunk) = locate_hunk(&hunks, "src/a.ts", 41, "new").unwrap();
        assert_eq!((n, hunk.id.as_str()), (2, "a2"));
        let (n, hunk) = locate_hunk(&hunks, "src/a.ts", 10, "old").unwrap();
        assert_eq!((n, hunk.id.as_str()), (1, "a1"));
        // 순수 추가 hunk의 old 범위(count 0)는 시작 줄 하나만 품는다.
        assert!(locate_hunk(&hunks, "src/b.ts", 1, "old").is_some());
        assert!(locate_hunk(&hunks, "src/b.ts", 2, "old").is_none());
        assert!(locate_hunk(&hunks, "src/a.ts", 20, "new").is_none());
        assert!(locate_hunk(&hunks, "src/zzz.ts", 1, "new").is_none());
    }

    #[test]
    fn note_position_follows_the_renderers_side_rule() {
        let mut mixed = hunk("m", "src/a.ts", (9, 3), (9, 3));
        mixed.lines = vec![
            DiffLine::Context("a".into()), // old 9 · new 9
            DiffLine::Del("b".into()),     // old 10
            DiffLine::Add("c".into()),     // new 10
            DiffLine::Context("d".into()), // old 11 · new 11
        ];
        assert_eq!(note_position(&mixed, 10, "new"), Some((10, "new")));
        assert_eq!(note_position(&mixed, 10, "old"), Some((10, "old")));
        // old 번호로 가리킨 문맥 줄은 화면이 쓰는 new 번호로 옮겨진다.
        assert_eq!(note_position(&mixed, 11, "old"), Some((11, "new")));
        assert_eq!(note_position(&mixed, 9, "old"), Some((9, "new")));
        assert_eq!(note_position(&mixed, 12, "new"), None);
        assert_eq!(note_position(&mixed, 8, "old"), None);
    }

    #[test]
    fn nth_hunk_and_hunk_number_agree() {
        let hunks = sample();
        assert_eq!(nth_hunk(&hunks, "src/a.ts", 2).map(|h| h.id.as_str()), Some("a2"));
        assert!(nth_hunk(&hunks, "src/a.ts", 3).is_none());
        assert!(nth_hunk(&hunks, "src/a.ts", 0).is_none());
        assert_eq!(hunk_number(&hunks, "src/a.ts", "a2"), Some(2));
        assert_eq!(hunk_number(&hunks, "src/b.ts", "a2"), None);
    }

    #[test]
    fn descriptors_name_the_three_review_tools() {
        let names: Vec<_> = descriptors()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec![COMMENT_ADD, COMMENT_LIST, NAVIGATE]);
        assert_eq!(descriptors()[0]["inputSchema"]["required"], json!(["path", "line", "body"]));
    }
}
