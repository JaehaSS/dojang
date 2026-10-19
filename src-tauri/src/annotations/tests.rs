use super::*;
use crate::diffmodel::{DiffHunk, DiffLine, RiskLevel};

fn hunk(
    path: &str,
    old_range: (u32, u32),
    new_range: (u32, u32),
    lines: Vec<DiffLine>,
) -> DiffHunk {
    DiffHunk {
        id: format!("{path}-{}-{}", new_range.0, new_range.1),
        path: path.to_string(),
        old_range,
        new_range,
        lines,
        protected: false,
        committed: false,
        risk: RiskLevel::Low,
    }
}

fn login_hunk() -> DiffHunk {
    hunk(
        "src/auth/login.ts",
        (11, 0),
        (11, 1),
        vec![DiffLine::Add("if (!token) throw new AuthError()".into())],
    )
}

fn mixed_hunk() -> DiffHunk {
    hunk(
        "src/auth/login.ts",
        (9, 2),
        (9, 3),
        vec![
            DiffLine::Context("function login() {".into()),
            DiffLine::Add("if (!token) throw new AuthError()".into()),
            DiffLine::Context("}".into()),
        ],
    )
}

fn annotation(hunk: &DiffHunk, hunk_id: &str, line: i64) -> ReviewAnnotation {
    ReviewAnnotation {
        id: "ann-1".into(),
        task_id: 1,
        hunk_id: hunk_id.into(),
        path: hunk.path.clone(),
        line,
        side: "new".into(),
        body_md: "x".into(),
        status: status::SENT.into(),
        created_at: 1,
        author: author::HUMAN.into(),
    }
}

#[test]
fn format_resend_matches_canonical_spec_sample() {
    let hunk = login_hunk();
    let mut annotation = annotation(&hunk, &hunk.id, 11);
    annotation.body_md = "에러 코드는 상수로 빼줘".into();
    let items = build_resend_items(
        std::slice::from_ref(&annotation),
        std::slice::from_ref(&hunk),
    );
    assert_eq!(
        format_resend(&items),
        concat!(
            "[리뷰 주석 1건 — 각 항목을 반영하고 완료 후 보고할 것]\n",
            "1. src/auth/login.ts:11 (변경 후 기준)\n",
            "   > + if (!token) throw new AuthError()\n",
            "   코멘트: 에러 코드는 상수로 빼줘\n",
        )
    );
}

#[test]
fn format_resend_omits_quote_when_hunk_missing() {
    let items = vec![ResendItem {
        path: "removed.ts".into(),
        line: 4,
        quoted: None,
        comment: "이 파일은 이제 없음".into(),
    }];
    assert_eq!(
        format_resend(&items),
        concat!(
            "[리뷰 주석 1건 — 각 항목을 반영하고 완료 후 보고할 것]\n",
            "1. removed.ts:4 (변경 후 기준)\n",
            "   코멘트: 이 파일은 이제 없음\n",
        )
    );
}

#[test]
fn quote_line_finds_add_del_and_context_by_side() {
    let mixed = mixed_hunk();
    assert_eq!(
        quote_line(&mixed, 10, "new").as_deref(),
        Some("+ if (!token) throw new AuthError()")
    );
    assert_eq!(
        quote_line(&mixed, 9, "new").as_deref(),
        Some("  function login() {")
    );
    assert_eq!(quote_line(&mixed, 99, "new"), None);
    let deleted = hunk(
        "a.ts",
        (5, 1),
        (5, 0),
        vec![DiffLine::Del("legacy()".into())],
    );
    assert_eq!(
        quote_line(&deleted, 5, "old").as_deref(),
        Some("- legacy()")
    );
}

#[test]
fn rematch_prefers_exact_hunk_id_over_approximate() {
    let hunk = login_hunk();
    let annotation = annotation(&hunk, &hunk.id, 10);
    let result = rematch(
        std::slice::from_ref(&annotation),
        std::slice::from_ref(&hunk),
    );
    assert_eq!(result[0].matched_hunk_id.as_deref(), Some(hunk.id.as_str()));
    assert!(!result[0].orphaned);
}

#[test]
fn rematch_falls_back_to_path_and_line_within_tolerance() {
    let moved = hunk(
        "src/auth/login.ts",
        (9, 2),
        (12, 3),
        vec![DiffLine::Add("if (!token) throw new AuthError()".into())],
    );
    let annotation = annotation(&moved, "stale-hunk-id", 10);
    let result = rematch(
        std::slice::from_ref(&annotation),
        std::slice::from_ref(&moved),
    );
    assert_eq!(
        result[0].matched_hunk_id.as_deref(),
        Some(moved.id.as_str())
    );
    assert!(!result[0].orphaned);
}

#[test]
fn rematch_marks_orphaned_when_no_hunk_within_tolerance_or_path() {
    let far = hunk("src/auth/login.ts", (9, 2), (50, 3), vec![]);
    let other = hunk("other.ts", (10, 2), (10, 3), vec![]);
    let annotation = annotation(&far, "stale", 10);
    let result = rematch(std::slice::from_ref(&annotation), &[far, other]);
    assert!(result[0].orphaned);
    assert!(result[0].matched_hunk_id.is_none());
}

#[test]
fn generated_ids_are_unique_across_calls() {
    let first = store::generate_id(1, "a.ts", 1, 1000);
    let second = store::generate_id(1, "a.ts", 1, 1000);
    assert_ne!(first, second);
    assert!(first.starts_with("ann-"));
}

/// author 열이 없던 DB(사람 주석만 있던 시절)를 열면 열이 생기고 기존 행은 human으로 읽힌다.
/// 에이전트 노트는 `sent`로 태어나 재전송 묶음(draft)에 섞이지 않는다.
#[tokio::test]
async fn migrate_adds_author_to_an_old_table_and_agent_notes_are_born_sent() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE review_annotations (
           id TEXT PRIMARY KEY, task_id INTEGER NOT NULL, hunk_id TEXT NOT NULL, path TEXT NOT NULL,
           line INTEGER NOT NULL, side TEXT NOT NULL, body_md TEXT NOT NULL,
           status TEXT NOT NULL DEFAULT 'draft' CHECK(status IN ('draft','sent','resolved')),
           created_at INTEGER NOT NULL);
         INSERT INTO review_annotations VALUES ('ann-old', 1, 'h1', 'a.ts', 3, 'new', 'old note', 'draft', 1);",
    )
    .execute(&pool)
    .await
    .unwrap();

    migrate(&pool).await.unwrap();
    migrate(&pool).await.unwrap(); // 두 번 열어도 ALTER가 다시 돌지 않는다.
    let note = create_agent_note(&pool, 1, "h1", "a.ts", 4, "new", "agent note", "claude", 2)
        .await
        .unwrap();
    let all = list_by_task(&pool, 1).await.unwrap();

    assert_eq!(all.len(), 2);
    assert_eq!((all[0].id.as_str(), all[0].author.as_str(), all[0].status.as_str()), ("ann-old", "human", "draft"));
    assert_eq!((all[1].id.as_str(), all[1].author.as_str(), all[1].status.as_str()), (note.id.as_str(), "claude", "sent"));
    assert!(author::is_human(&all[0].author));
    assert!(!author::is_human(&all[1].author));
}
