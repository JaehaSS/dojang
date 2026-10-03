//! 오늘 할 일(today) 보드 커맨드.
//!
//! `commands.rs`에서 갈라져 나왔다 — 모든 기능이 한 파일 끝에 줄을 붙이는
//! 구조가 병행 worktree의 머지 충돌을 만들었다.

use std::path::PathBuf;

use tauri::{AppHandle, State};

use crate::db::Task;
use crate::github;
pub(crate) use crate::orchestrator::{CreateTaskParams, TaskOrigin};

use super::{AppState, create_task_internal, now, pool_of, resolve_day};

#[tauri::command]
pub async fn today_list(
    state: State<'_, AppState>,
    day: Option<String>,
) -> Result<Vec<crate::today::DayItem>, String> {
    let pool = pool_of(&state)?;
    let day = resolve_day(day)?;
    // 백로그는 날짜가 아니라 이월도 대조도 대상이 아니다. 대조를 건너뛰는 이유는 이월과
    // 다르다 — 백로그 항목의 Task가 돌고 있다면 착수 시점에 이미 오늘로 당겨졌다는 뜻이라
    // (`today_start`), 백로그에 남은 링크는 대조할 것이 없다.
    if !crate::today::day::is_backlog(&day) {
        // 지난 날의 미완료를 끌어온다. **오늘을 볼 때만** — 과거 날짜를 열람하는 것만으로
        // 그 날의 기록이 바뀌면 안 된다.
        if day == crate::today::day::local_day(now(), crate::today::day::KST_OFFSET_SECS)? {
            crate::today::carry::carry_forward(&pool, &day, now()).await?;
        }
        // 조회할 때마다 대조 — "보는 순간 정확"을 보장한다 (설계 0021 DR-4).
        // 이월 다음에 둔다: 끌어온 항목의 Task가 이미 끝나 있으면 여기서 바로 체크된다.
        crate::today::store::reconcile_tasks(&pool, &day, now()).await?;
    }
    crate::today::store::list(&pool, &day).await
}

/// 날짜 범위 조회 — 인사이트 계획 캘린더가 달 단위로 부른다 (설계 0023).
///
/// 조회 전에 범위 전체를 한 번 대조한다. `today_list`가 지키는 "보는 순간 정확"(DR-4)을
/// 범위에서도 동일하게 지킨다.
#[tauri::command]
pub async fn today_range(
    state: State<'_, AppState>,
    from: String,
    to: String,
) -> Result<Vec<crate::today::DayItem>, String> {
    let pool = pool_of(&state)?;
    crate::today::day::validate(&from)?;
    crate::today::day::validate(&to)?;
    if from > to {
        return Err(format!("범위가 뒤집혔습니다: {from} > {to}"));
    }
    crate::today::store::reconcile_tasks_range(&pool, &from, &to, now()).await?;
    crate::today::store::range(&pool, &from, &to).await
}

#[tauri::command]
pub async fn today_add(
    state: State<'_, AppState>,
    day: Option<String>,
    title: String,
    repo: Option<String>,
) -> Result<crate::today::DayItem, String> {
    let pool = pool_of(&state)?;
    let day = resolve_day(day)?;
    crate::today::store::add(&pool, &day, &title, repo.as_deref(), now()).await
}

#[tauri::command]
pub async fn today_set_status(
    state: State<'_, AppState>,
    id: i64,
    status: String,
) -> Result<crate::today::DayItem, String> {
    let pool = pool_of(&state)?;
    let parsed = crate::today::DayStatus::parse(&status)?;
    crate::today::store::set_status(&pool, id, parsed, now()).await
}

#[tauri::command]
pub async fn today_reorder(
    state: State<'_, AppState>,
    day: String,
    ordered_ids: Vec<i64>,
) -> Result<(), String> {
    let pool = pool_of(&state)?;
    crate::today::store::reorder(&pool, &day, &ordered_ids, now()).await
}

#[tauri::command]
pub async fn today_remove(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let pool = pool_of(&state)?;
    crate::today::store::remove(&pool, id).await
}

/// 항목에서 에이전트 Task를 착수한다. `repo`가 없으면 거부 — 비코딩 항목은 대상이 아니다.
/// 이미 착수한 항목의 재착수도 거부한다 (1:1, 설계 0021 DR-5. DB 유니크 인덱스가 백스톱).
#[tauri::command]
pub async fn today_start(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    agent: String,
    model: Option<String>,
    reasoning_effort: Option<String>,
    mode: String,
) -> Result<Task, String> {
    let pool = pool_of(&state)?;
    let item = crate::today::store::get(&pool, id).await?;
    let repo = item
        .repo
        .clone()
        .filter(|r| !r.trim().is_empty())
        .ok_or("레포가 지정되지 않은 항목은 착수할 수 없습니다")?;
    if item.task_id.is_some() {
        return Err("이미 착수한 항목입니다".into());
    }
    // 백로그에서 바로 착수하면 오늘로 당긴다 — "지금 한다"는 곧 오늘 일이고, 백로그에
    // 남겨 두면 착수한 일이 어느 날짜 목록에도 나타나지 않는다. 거부 검사 **뒤에** 둔다:
    // 어차피 실패할 요청이 항목을 옮겨 놓고 끝나면 안 된다.
    //
    // 이 다음의 Task 생성이 실패하면 항목은 오늘에 남는다. 되돌리지 않는 것은 의도다 —
    // 사용자는 이미 "지금 한다"를 눌렀고, 그 의사는 생성 실패와 무관하게 유효하다.
    if crate::today::day::is_backlog(&item.day) {
        let today = crate::today::day::local_day(now(), crate::today::day::KST_OFFSET_SECS)?;
        crate::today::store::move_to(&pool, id, &today, now()).await?;
    }
    // 지시문은 제목 + (있으면) 메모. UI 생성 경로(`task_create`)와 같은 파라미터 구성을
    // 쓴다 — origin=Ui라 즉시 spawn된다.
    let instruction = match item.note.as_deref().map(str::trim) {
        Some(note) if !note.is_empty() => format!("{}\n\n{}", item.title, note),
        _ => item.title.clone(),
    };
    let params = CreateTaskParams {
        repo,
        instruction,
        agent,
        role: String::new(),
        model: model.unwrap_or_default(),
        reasoning_effort: reasoning_effort.unwrap_or_default(),
        service_tier: None,
        headless: false,
        ensemble: String::new(),
        mode,
        cmd: String::new(),
        args: Vec::new(),
        cols: 100,
        rows: 30,
        origin: TaskOrigin::Ui,
        goal_contract: None,
        ambiguity: None,
        // 오늘 항목 착수는 base를 고르는 UI가 없다 — 현재 체크아웃에서 분기한다.
        base_branch: None,
        // 착수 버튼은 컴포저가 아니다 — 단계 표시를 받는 입력줄이 없다.
        client_ref: None,
        // 오늘 항목 착수는 언제나 새 대화다.
        resume_from: None,
        resume_session: None,
        resume_vendor: None,
        purpose_selection: None,
        approvals: false,
    };
    let task = create_task_internal(&app, &state, params).await?;
    // Task 생성은 성공했는데 링크만 실패하면 되돌리지 않는다 — 사용자는 이미 실행 중인
    // 에이전트를 보고 있다. 링크가 없으면 자동 체크만 안 될 뿐이므로 best-effort로 남긴다
    // (`github_create_task_from_issue`의 `task_issue_refs`와 같은 판단).
    let _ = crate::today::store::link_task(&pool, id, task.id, now()).await;
    Ok(task)
}

/// 오늘의 후보. github 소스는 `gh` CLI를 타므로 실패해도 다른 소스를 죽이지 않는다.
#[tauri::command]
pub async fn today_suggest(
    state: State<'_, AppState>,
    day: Option<String>,
    repo: Option<String>,
) -> Result<Vec<crate::today::suggest::Suggestion>, String> {
    let pool = pool_of(&state)?;
    let day = resolve_day(day)?;
    // `carry` 소스는 없다 — 지난 날의 미완료는 제안이 아니라 `today_list`가 직접 이월한다.
    let mut out = crate::today::suggest::awaiting(&pool, now()).await?;
    if let Some(repo) = repo.filter(|r| !r.trim().is_empty()) {
        out.extend(crate::today::suggest::memory(&repo));
        // gh 미설치·비 GitHub 레포는 제안이 없는 것으로 취급한다 — 에러로 올리면
        // 다른 세 소스까지 화면에서 사라진다.
        let path = PathBuf::from(&repo);
        let issues = tauri::async_runtime::spawn_blocking(move || {
            if !github::is_github_remote(&path) {
                return Vec::new();
            }
            github::list_issues(&path).unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        out.extend(
            issues
                .into_iter()
                .map(|i| crate::today::suggest::Suggestion {
                    title: format!("이슈 #{}: {}", i.number, i.title),
                    source: "github".into(),
                    source_ref: Some(i.number.to_string()),
                    repo: Some(repo.clone()),
                }),
        );
    }
    crate::today::suggest::exclude_taken(&pool, &day, out).await
}

/// 제안을 목록에 담는다. 같은 날 같은 출처는 유니크 인덱스가 막는다 —
/// 그 에러는 "이미 담음"이므로 사용자에게 그대로 노출하지 않고 `None`으로 돌려준다.
#[tauri::command]
pub async fn today_take(
    state: State<'_, AppState>,
    day: Option<String>,
    title: String,
    source: String,
    source_ref: Option<String>,
    repo: Option<String>,
) -> Result<Option<crate::today::DayItem>, String> {
    let pool = pool_of(&state)?;
    let day = resolve_day(day)?;
    match crate::today::store::add_sourced(
        &pool,
        &day,
        &title,
        repo.as_deref(),
        &source,
        source_ref.as_deref(),
        now(),
    )
    .await
    {
        Ok(item) => Ok(Some(item)),
        Err(e) if e.contains("UNIQUE") => Ok(None),
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub async fn today_close(
    state: State<'_, AppState>,
    day: Option<String>,
) -> Result<crate::today::close::DayClosing, String> {
    let pool = pool_of(&state)?;
    let day = resolve_day(day)?;
    // 백로그는 마감 대상이 아니다 — 하루가 아니기 때문이다. `resolve_day`가 레인 키까지
    // 통과시키므로 여기서 날짜로 한 번 더 좁힌다.
    crate::today::day::validate(&day)?;
    crate::today::close::close_day(&pool, &day, now()).await
}

/// 항목을 다른 레인으로 옮긴다. `to`가 `"backlog"`면 밀기, 날짜(또는 생략=오늘)면 당기기.
///
/// 방향마다 커맨드를 두지 않는다 — 같은 UPDATE가 두 벌이 될 뿐이다 (플랜 0054 DR-3).
#[tauri::command]
pub async fn today_move(
    state: State<'_, AppState>,
    id: i64,
    to: Option<String>,
) -> Result<crate::today::DayItem, String> {
    let pool = pool_of(&state)?;
    let to = resolve_day(to)?;
    crate::today::store::move_to(&pool, id, &to, now()).await
}

// ── 지식 그래프 · Gmail (설계 0020 Phase 4 / 플랜 0028) ──

