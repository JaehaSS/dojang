//! 에이전트의 리뷰 노트·이동 명령을 주석 저장소와 Diff 탭으로 내리는 Tauri 쪽 구현.
//! MCP 서버는 이 파일을 모르고 `ReviewBridge` 트레이트 뒤에서만 만난다(`preview_control`과 같은 모양).

use serde::Serialize;
use tauri::Emitter;

use crate::annotations;
use crate::db;
use crate::diffmodel::DiffHunk;
use crate::preview_bridge::mcp::review::{
    hunk_header, hunk_number, hunks_of, locate_hunk, note_position, nth_hunk, NavigateTarget,
    ReviewCommand,
};
use crate::preview_bridge::mcp::ReviewBridge;

/// 에이전트가 노트를 남겼다. Diff 세션은 이 작업의 주석만 다시 읽는다 — 5초 폴링을 기다리지 않는다.
pub const ANNOTATIONS_EVENT: &str = "review://annotations";
/// 에이전트가 사용자 시선을 옮기려 한다. 그 작업의 Diff 세션이 파일을 열고 hunk로 스크롤한다.
pub const NAVIGATE_EVENT: &str = "review://navigate";

#[derive(Debug, Clone, Serialize)]
pub struct AnnotationsChanged {
    pub task_id: i64,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NavigateRequest {
    pub task_id: i64,
    pub path: String,
    pub hunk_id: String,
    /// 목표 hunk 안의 줄. hunk id가 화면과 어긋날 때 되찾는 열쇠다.
    pub line: i64,
    pub side: String,
}

pub struct TauriReviewBridge {
    pub app: tauri::AppHandle,
    pub pool: sqlx::SqlitePool,
}

#[async_trait::async_trait]
impl ReviewBridge for TauriReviewBridge {
    async fn run(&self, task_id: i64, author: &str, cmd: ReviewCommand) -> Result<String, String> {
        let task = db::get_task(&self.pool, task_id)
            .await
            .map_err(|error| format!("db_error: {error}"))?
            .ok_or_else(|| "task_not_found".to_string())?;
        // diff는 git 호출이다 — 서버의 비동기 스레드를 붙들지 않는다.
        let hunks = tauri::async_runtime::spawn_blocking(move || crate::commands::task_hunks(&task))
            .await
            .map_err(|error| format!("diff_failed: {error}"))?
            .map_err(|error| format!("diff_failed: {error}"))?;
        match cmd {
            ReviewCommand::CommentAdd { path, line, side, body } => {
                self.comment_add(task_id, author, &hunks, path, line, side, body).await
            }
            ReviewCommand::CommentList { path } => self.comment_list(task_id, &hunks, path).await,
            ReviewCommand::Navigate { path, target } => self.navigate(task_id, &hunks, path, target),
        }
    }
}

impl TauriReviewBridge {
    #[allow(clippy::too_many_arguments)]
    async fn comment_add(
        &self,
        task_id: i64,
        author: &str,
        hunks: &[DiffHunk],
        path: String,
        line: i64,
        side: String,
        body: String,
    ) -> Result<String, String> {
        let Some((number, hunk)) = locate_hunk(hunks, &path, line, &side) else {
            return Err(not_in_diff(hunks, &path, line, &side));
        };
        // 화면이 그리는 자리로 맞춘다 — old 번호로 가리킨 문맥 줄은 new 번호가 된다.
        let (line, side) = note_position(hunk, line, &side)
            .ok_or_else(|| not_in_diff(hunks, &path, line, &side))?;
        let note = annotations::create_agent_note(
            &self.pool, task_id, &hunk.id, &path, line, side, &body, author, crate::now(),
        )
        .await
        .map_err(|error| format!("db_error: {error}"))?;
        let _ = self.app.emit(ANNOTATIONS_EVENT, AnnotationsChanged { task_id, path: path.clone() });
        Ok(format!(
            "added {} · {path}:{line} ({side}) · hunk {number}/{}",
            note.id,
            hunks_of(hunks, &path).len()
        ))
    }

    async fn comment_list(
        &self,
        task_id: i64,
        hunks: &[DiffHunk],
        path: Option<String>,
    ) -> Result<String, String> {
        let stored = annotations::list_by_task(&self.pool, task_id)
            .await
            .map_err(|error| format!("db_error: {error}"))?;
        let notes: Vec<_> = annotations::rematch(&stored, hunks)
            .into_iter()
            .filter(|note| path.as_deref().is_none_or(|path| note.annotation.path == path))
            .collect();
        let mut out = format!("notes: {}", notes.len());
        for note in &notes {
            let a = &note.annotation;
            let mut line = format!(
                "\n{} · {}:{} ({}) · {} · {}",
                a.id, a.path, a.line, a.side, a.author, a.status
            );
            if note.orphaned {
                line.push_str(" · orphaned");
            } else if let Some(number) = note
                .matched_hunk_id
                .as_deref()
                .and_then(|hunk_id| hunk_number(hunks, &a.path, hunk_id))
            {
                line.push_str(&format!(" · hunk {number}/{}", hunks_of(hunks, &a.path).len()));
            }
            out.push_str(&line);
            for body_line in a.body_md.lines() {
                out.push_str("\n  ");
                out.push_str(body_line);
            }
        }
        Ok(out)
    }

    fn navigate(
        &self,
        task_id: i64,
        hunks: &[DiffHunk],
        path: String,
        target: NavigateTarget,
    ) -> Result<String, String> {
        let total = hunks_of(hunks, &path).len();
        if total == 0 {
            return Err(format!("path_not_in_diff: {path} has no hunks in this task's diff"));
        }
        let (number, hunk, line, side) = match target {
            NavigateTarget::Hunk(n) => {
                let hunk = nth_hunk(hunks, &path, n)
                    .ok_or_else(|| format!("hunk_not_found: {path} has {total} hunk(s), asked for {n}"))?;
                (n, hunk, None, None)
            }
            NavigateTarget::Line { line, side } => {
                let (n, hunk) = locate_hunk(hunks, &path, line, &side)
                    .ok_or_else(|| not_in_diff(hunks, &path, line, &side))?;
                (n, hunk, Some(line), Some(side))
            }
            NavigateTarget::File => (1, hunks_of(hunks, &path)[0], None, None),
        };
        // 줄은 항상 싣는다. 화면이 다른 범위를 보고 있거나 파일이 그새 바뀌어 hunk id가 갈려도,
        // 줄 번호로 지금 그려진 hunk를 다시 찾을 수 있다.
        let (line, side) = match (line, side) {
            (Some(line), Some(side)) => (line, side),
            _ => (hunk.new_range.0.max(1) as i64, "new".to_string()),
        };
        let _ = self.app.emit(
            NAVIGATE_EVENT,
            NavigateRequest {
                task_id,
                path: path.clone(),
                hunk_id: hunk.id.clone(),
                line,
                side,
            },
        );
        Ok(format!("navigated · {path} · hunk {number}/{total} {}", hunk_header(hunk)))
    }
}

/// 어디가 diff 안인지 같이 알려 준다 — 에이전트가 줄 번호를 고쳐 다시 부를 수 있게.
fn not_in_diff(hunks: &[DiffHunk], path: &str, line: i64, side: &str) -> String {
    let file_hunks = hunks_of(hunks, path);
    if file_hunks.is_empty() {
        return format!("path_not_in_diff: {path} has no hunks in this task's diff");
    }
    let ranges: Vec<String> = file_hunks
        .iter()
        .map(|hunk| {
            let (start, count) = if side == "old" { hunk.old_range } else { hunk.new_range };
            format!("{start}-{}", start + count.max(1) - 1)
        })
        .collect();
    format!(
        "line_not_in_diff: {path}:{line} ({side}) is outside the changed hunks; {side} lines in the diff: {}",
        ranges.join(", ")
    )
}
