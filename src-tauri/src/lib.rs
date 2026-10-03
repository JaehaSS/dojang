pub mod agent;
pub mod agenthealth;
pub mod annotations;
pub mod approval;
pub mod task_results;
pub mod pipeline;
pub mod project_plan;
pub mod task_review;
pub mod baseline;
pub mod capsule;
pub mod capture;
mod commands;
pub mod convo;
pub mod db;
pub mod decision;
pub mod diffcompress;
pub mod diffmodel;
pub mod editorwindow;
pub mod embed;
pub mod ensemble;
pub mod envpath;
pub mod evidence;
pub mod followup_observation;
pub mod fonts;
pub mod fsapi;
pub mod goal_contract;
pub mod insightdeck;
pub mod insights;
pub mod interview;
pub mod jsonextract;
pub mod knowledge;
pub mod lspdetect;
pub mod managed_process;
pub mod mcp_registry;
pub mod memory;
pub mod multireview;
pub mod notifications;
#[cfg(test)]
mod notifications_tests;
pub mod orchestrator;
pub mod partial;
pub mod projector;
pub mod pty;
pub mod quiz;
pub mod repl_launch;
pub mod review_ops;
pub mod reviewer;
pub mod rewind;
pub mod risk;
pub mod rlimit;
pub mod runner;
pub mod schedule;
pub mod secret;
pub mod sessionhome;
pub mod shellreap;
pub mod side_question;
pub mod skills;
/// 유닛 테스트 전용 임시 루트 — 프로덕션 빌드에는 들어가지 않는다.
#[cfg(test)]
mod testtmp;
pub mod theme_store;
pub mod today;
pub mod translate;
pub mod vocab;
pub mod verify;
pub mod voice;
pub mod worktree;
// 알파벳 순서 밖(신규 모듈, C-2) — 위쪽 목록이 다른 작업 세션과 동시 수정 중이라
// 병합 안전을 위해 끝에 추가한다.
pub mod github;
pub mod hotkeys;
pub mod usage;
// 알파벳 순서 밖(신규 모듈, D-1) — 위와 같은 이유로 끝에 추가.
pub mod designmode;
// 알파벳 순서 밖(신규 모듈) — remote preview result의 최소 권한 IPC registry.
pub mod preview_bridge;
pub mod preview_workbench;
// 알파벳 순서 밖(신규 모듈) — 프리뷰 웹뷰를 실제로 모는 Tauri 디스패처.
pub mod preview_control;
// 알파벳 순서 밖(신규 모듈) — 위와 같은 이유로 끝에 추가.
pub mod paste;
pub mod project_editor;
// 알파벳 순서 밖(신규 모듈) — 위와 같은 이유로 끝에 추가.
pub mod lspclient;
// 알파벳 순서 밖(신규 모듈) — 데스크톱이 직접 서빙하는 모바일 표면(설계 2026-09-13).
pub mod mobile_surface;

use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use commands::AppState;
#[cfg(target_os = "macos")]
use tauri::menu::{AboutMetadata, MenuBuilder, SubmenuBuilder};
use tauri::{Emitter, Manager};

pub(crate) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 인앱 MCP 서버를 루프백의 임의 포트에 연다. 실패해도 앱은 뜬다 — 프리뷰 제어만 없다.
fn start_preview_mcp(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let pool = state
        .pool
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let Some(pool) = pool else {
        eprintln!("DB가 없어 프리뷰 MCP 서버를 띄우지 않습니다");
        return;
    };
    let Some(instance) = state.mcp_instance.clone() else {
        eprintln!("인스턴스 조각을 뽑지 못해 프리뷰 MCP 서버를 띄우지 않습니다");
        return;
    };
    let mcp = std::sync::Arc::new(preview_bridge::mcp::McpState {
        instance,
        tokens: state.control_tokens.clone(),
        dispatcher: std::sync::Arc::new(preview_control::TauriDispatcher {
            app: app.clone(),
            pool,
        }),
        tools: preview_bridge::mcp::Tools::phase_f(),
    });
    match tauri::async_runtime::block_on(preview_bridge::mcp::bind()) {
        Ok((port, listener)) => {
            // 지난 실행이 남긴 설정 파일 청소. 디렉터리는 남긴다 — dev 빌드와 /Applications가
            // 같은 데이터 디렉터리를 공유하므로 상대의 살아있는 파일을 지우면 안 된다.
            if let Ok(data_dir) = app.path().app_data_dir() {
                preview_bridge::mcp::inject::sweep_stale(
                    &preview_bridge::mcp::inject::config_root(&data_dir),
                    preview_bridge::mcp::inject::CONFIG_TTL,
                );
            }
            tauri::async_runtime::spawn(preview_bridge::mcp::serve_on(listener, mcp));
            *state
                .mcp_port
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(port);
            eprintln!("프리뷰 MCP 서버 포트 {port}");
        }
        Err(error) => eprintln!("프리뷰 MCP 서버 기동 실패: {error}"),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // launchd가 준 fd soft limit(256)은 동시 세션 spawn에 모자란다. 자식(claude·그 아래
    // MCP 서버)이 상속하도록 셸아웃보다 먼저 올린다.
    eprintln!("{}", rlimit::raise_file_limit());

    // GUI(.app)는 최소 PATH만 받으므로, 모든 셸아웃(claude/git/build 등) 전에 PATH 보강.
    envpath::augment_path();
    let state = AppState::default();
    let preview_bridge = state.preview_bridge.clone();
    tauri::Builder::default()
        .plugin(preview_bridge::tauri_plugin_with(preview_bridge))
        .plugin(preview_workbench::plugin::plugin())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(hotkeys::plugin())
        .manage(state)
        .manage(project_editor::ProjectEditorState::default())
        .manage(voice::VoiceManaged::default())
        .manage(voice::server::ServerManaged::default())
        .manage(mobile_surface::MobileSurface::default())
        .setup(|app| {
            if let Some(url) = std::env::var_os("PRAXIS_PREVIEW_AUTO_OPEN_PROBE_URL") {
                let root = std::env::var_os("PRAXIS_PREVIEW_AUTO_OPEN_PROBE_DIR")
                    .ok_or("PRAXIS_PREVIEW_AUTO_OPEN_PROBE_DIR is required")?;
                return preview_control::auto_open_probe::start(
                    app, &url.to_string_lossy(), root.into(),
                ).map_err(Into::into);
            }
            if let Some(url) = std::env::var_os("PRAXIS_PREVIEW_TOOLBAR_PROBE_URL") {
                let url = url.to_string_lossy();
                return commands::start_toolbar_probe(app, &url).map_err(Into::into);
            }
            if let Some(url) = std::env::var_os("PRAXIS_PREVIEW_PROBE_URL") {
                let url = url.to_string_lossy();
                return commands::start_preview_probe(app, &url).map_err(Into::into);
            }
            // 시작 시 SQLite 풀 초기화 + 재시작 복원(Running→Failed).
            let handle = app.handle().clone();
            let data_dir = handle
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            std::fs::create_dir_all(&data_dir).ok();
            let db_path = data_dir
                .join("praxis.sqlite")
                .to_string_lossy()
                .into_owned();
            tauri::async_runtime::block_on(async {
                match db::init_pool(&db_path).await {
                    Ok(pool) => {
                        let state = handle.state::<AppState>();
                        if let Err(error) = memory::migrate(&pool).await {
                            eprintln!("메모리 스키마 초기화 실패 — DB를 활성화하지 않음: {error}");
                            return;
                        }
                        if let Err(error) = side_question::migrate(&pool).await {
                            eprintln!(
                                "별도 질의 스키마 초기화 실패 — DB를 활성화하지 않음: {error}"
                            );
                            return;
                        }
                        if let Err(error) = side_question::recover(&pool).await {
                            eprintln!("별도 질의 실행 복구 실패 — DB를 활성화하지 않음: {error}");
                            return;
                        }
                        if let Err(error) = decision::local_approval::reconcile(&pool, now()).await
                        {
                            eprintln!("로컬 승인 ledger 복구 실패 — 다음 시작에 재시도: {error}");
                        }
                        if let Err(error)=convo::interaction_commands::recover(&handle,&pool).await {
                            eprintln!("질문 세션 복원 실패 — DB를 활성화하지 않음: {error}");return;
                        }
                        // 재시작 복원(Plan 0012): 생존 확인 후 대화 turn은 Running 유지·감시,
                        // 나머지 running은 Failed. 구 mark_stale_running_failed(무조건 Failed) 대체.
                        commands::reconcile_stale_running(
                            &handle,
                            &pool,
                            &state.direct_repo_locks,
                            state.convo_active.clone(),
                            now(),
                        )
                        .await;
                        let _ = mcp_registry::migrate(&pool).await;
                        let _ = schedule::migrate(&pool).await;
                        let _ = multireview::migrate(&pool).await;
                        let _ = annotations::migrate(&pool).await;
                        let _ = partial::migrate(&pool).await;
                        let _ = github::migrate(&pool).await;
                        let _ = knowledge::migrate(&pool).await;
                        let _ = knowledge::vault::migrate(&pool).await;
                        if let Err(error) = knowledge::vault::recover_operations(&pool, now()).await
                        {
                            eprintln!("vault operation recovery failed: {error}");
                        }
                        // quiz는 knowledge_chunks를 FK로 참조한다 — 반드시 그 뒤에.
                        let _ = quiz::migrate(&pool).await;
                        let _ = today::migrate(&pool).await;
                        let _ = vocab::migrate(&pool).await;
                        let _ = insights::migrate_dismissals(&pool).await;
                        // 옛 메모리 파이프라인이 남긴 행을 한 번만 비운다(설계 2026-09-13 §7).
                        // 실패해도 부팅을 막지 않는다 — 남은 행은 아무도 읽지 않는다.
                        match memory::file::purge_legacy_once(&pool).await {
                            Ok(true) => eprintln!("옛 메모리 데이터를 비웠습니다"),
                            Ok(false) => {}
                            Err(error) => eprintln!("옛 메모리 데이터 정리 실패(무시): {error}"),
                        }
                        // 기준점 컬럼이 생기기 전에 만들어진 작업들에 지금 시점 값을 한 번
                        // 굳힌다. 실패해도 부팅을 막지 않는다 — 못 굳힌 작업은 레거시 경로로 돈다.
                        match baseline::pin_missing_baselines(&pool).await {
                            Ok(0) => {}
                            Ok(pinned) => eprintln!("diff 기준점 {pinned}건을 고정했습니다"),
                            Err(error) => eprintln!("diff 기준점 backfill 실패(무시): {error}"),
                        }
                        // 동시 실행 상한 복원 — 미설정/손상 값은 기본값으로 접힌다.
                        let limit = commands::parse_max_concurrent(
                            db::get_setting(&pool, "max_concurrent")
                                .await
                                .ok()
                                .flatten()
                                .as_deref(),
                        );
                        state.max_concurrent.store(limit, Ordering::Relaxed);
                        // 모바일 표면 스키마(페어링·세션·푸시 구독) — 설정 화면이
                        // 서빙 전에도 페어링 코드를 낼 수 있어야 한다.
                        if let Err(error) = mobile_surface::migrate(&pool).await {
                            eprintln!("모바일 표면 스키마 초기화 실패(무시): {error}");
                        }
                        *state.pool.lock().unwrap() = Some(pool);
                    }
                    Err(e) => eprintln!("DB 초기화 실패: {e}"),
                }
            });
            // 프리뷰 제어 MCP 서버 — 계측 행을 남기려면 풀이 필요해 DB 초기화 뒤에 띄운다.
            start_preview_mcp(&handle);
            // 모바일 표면(설계 2026-09-13) — 지난 실행에서 켜져 있었으면 되띄운다.
            // 풀이 AppState에 들어간 **뒤**라야 review_claims를 IPC와 공유할 수 있다.
            // bind 실패가 부팅을 잡아두지 않도록 spawn으로 떼어 둔다.
            {
                let pool = handle
                    .state::<AppState>()
                    .pool
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone();
                if let Some(pool) = pool {
                    let app = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        mobile_surface::restore(&app, &pool).await;
                    });
                }
            }
            // 임베딩 캐시를 app data 아래로 고정한다 — fastembed 기본값은 CWD 상대 경로라
            // 앱을 어디서 띄우느냐에 따라 ~130MB를 다시 내려받는다. best-effort로 둔다:
            // 이 관심사가 부팅을 막을 이유가 없고, 실패하면 종전 동작(기본 경로)이 남는다.
            // 모델을 처음 로드하기 **전에** 주입해야 한다(주입은 1회만 유효). 시작 시 워밍업은
            // 하지 않는다 — 쓰지도 않을 기능 때문에 ~130MB를 동의 없이 내려받게 된다.
            match handle.path().app_data_dir() {
                Ok(dir) => {
                    let cache = dir.join("fastembed");
                    match std::fs::create_dir_all(&cache) {
                        Ok(()) => embed::set_cache_dir(cache),
                        Err(error) => {
                            eprintln!("임베딩 캐시 디렉터리 생성 실패(기본 경로 사용): {error}")
                        }
                    }
                }
                Err(error) => eprintln!("app data 경로 조회 실패(임베딩 기본 경로 사용): {error}"),
            }
            // 크론 틱 루프(Phase 3) — 주기적으로 due 스케줄 실행.
            let cron_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                schedule::runner::tick_loop(cron_handle).await;
            });
            // 멀티 벤더 파이프라인 드라이버 — 활성 실행을 DB 상태에서 이어 받아 단계마다 한 동작씩 진행한다.
            let pipeline_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                commands::pipeline_driver::run_loop(pipeline_handle).await;
            });
            // 유휴 워크스페이스 셸 회수(ADR 0163 결정 4) — 아무도 보고 있지 않고 프롬프트에서
            // 놀고 있는 셸만 골라 정리한다. 판정이 불가능한 플랫폼에서는 아무것도 하지 않는다.
            let reap_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(shellreap::SCAN_INTERVAL).await;
                    commands::reap_idle_shells(&reap_handle);
                }
            });
            // CLI 자동 업데이트 — 시작 직후가 바이너리를 갈아치울 수 있는 유일한 창이다.
            // 활성 작업이 있으면 스스로 건너뛴다(되살아난 대화 턴이 있을 수 있다).
            let update_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let (pool, updating, last) = {
                    let state = update_handle.state::<commands::AppState>();
                    // 각각 바인딩으로 푼다 — 튜플 표현식에 두면 MutexGuard 임시값이 블록
                    // 끝까지 살아 `state` 보다 오래 남는다.
                    let pool = state.pool.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    let updating = state.updating.clone();
                    let last = state.last_autoupdate.clone();
                    (pool, updating, last)
                };
                // 설정을 읽지 못하면 돌리지 않는다 — 껐는지 켰는지 모르는 채로 바이너리를
                // 갈아치우느니 아무것도 하지 않는 편이 낫다. 다만 조용히 지나가지는 않는다.
                let Some(pool) = pool else {
                    eprintln!("DB가 없어 자동 업데이트를 건너뜁니다");
                    return;
                };
                let enabled =
                    match db::get_setting(&pool, agenthealth::autoupdate::SETTING_KEY).await {
                        Ok(raw) => agenthealth::autoupdate::enabled_from(raw.as_deref()),
                        Err(error) => {
                            eprintln!("자동 업데이트 설정을 읽지 못해 건너뜁니다: {error}");
                            return;
                        }
                    };
                let active_handle = update_handle.clone();
                let report = agenthealth::autoupdate::execute(enabled, &updating, move || {
                    let state = active_handle.state::<commands::AppState>();
                    commands::active_work_count(&state)
                })
                .await;
                *last.lock().unwrap_or_else(|e| e.into_inner()) = report.clone();
                let _ = update_handle.emit(agenthealth::autoupdate::EVENT, report);
            });
            // 음성 핫키 — 설정을 캐시해 두어야 핫키 핸들러(동기 컨텍스트)가 모드를 판별할 수 있다.
            {
                let handle = app.handle().clone();
                let pool = handle
                    .state::<AppState>()
                    .pool
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                let settings = match pool {
                    Some(pool) => tauri::async_runtime::block_on(voice::load_settings(&pool)),
                    None => voice::VoiceSettings::default(),
                };
                handle
                    .state::<voice::VoiceManaged>()
                    .set_settings(settings.clone());
                if let Err(error) = voice::register_shortcuts(&handle, &settings) {
                    eprintln!("음성 단축키 등록 실패: {error}");
                }
            }
            // Cmd+W 재정의: 기본 메뉴의 Close Window(Cmd+W 가속기)를 제거해
            // 웹뷰(JS)가 Cmd+W keydown을 받도록 한다. 종료/복사·붙여넣기 등 표준 항목은 유지.
            #[cfg(target_os = "macos")]
            {
                let app_menu = SubmenuBuilder::new(app, "Dojang")
                    .about(Some(AboutMetadata::default()))
                    .separator()
                    .services()
                    .separator()
                    .hide()
                    .hide_others()
                    .show_all()
                    .separator()
                    .quit()
                    .build()?;
                // Cmd+Z 재정의: Undo/Redo 항목을 의도적으로 넣지 않는다 → 웹뷰(JS)가
                // ⌘Z·⇧⌘Z keydown 을 받는다. Cmd+W 와 같은 이유이고, 이쪽은 증상이 더 고약했다.
                //
                // 메뉴에 `.undo()` 가 있으면 macOS 가 그 가속기를 **메뉴 커맨드로 먼저 소비**해
                // `undo:` 셀렉터를 first responder 로 보낸다. WKWebView 는 그것을 자기
                // NSUndoManager 로 처리하는데, Monaco 는 자체 undo 스택을 쓰고 hidden textarea 를
                // 프로그래매틱하게 조작하므로 거기 아무 기록이 없다 → 눌러도 아무 일이 없다.
                //
                // 잘라내기·복사·붙여넣기가 멀쩡했던 것이 이 진단의 근거다. `cut:`·`copy:`·
                // `paste:` 는 WKWebView 가 실제로 구현하지만 `undo:` 는 Monaco 까지 닿지 않는다.
                // 항목을 빼면 키가 웹뷰로 내려가 Monaco 의 기본 바인딩이 받고, 일반 input·
                // textarea 도 브라우저 기본 undo 로 동작한다.
                let edit_menu = SubmenuBuilder::new(app, "Edit")
                    .cut()
                    .copy()
                    .paste()
                    .select_all()
                    .build()?;
                // NOTE: .close_window() 를 의도적으로 넣지 않는다 → Cmd+W 해제.
                let window_menu = SubmenuBuilder::new(app, "Window")
                    .minimize()
                    .maximize()
                    .separator()
                    .fullscreen()
                    .build()?;
                let menu = MenuBuilder::new(app)
                    .items(&[&app_menu, &edit_menu, &window_menu])
                    .build()?;
                app.set_menu(menu)?;
            }
            Ok(())
        })
        .invoke_handler(|invoke: tauri::ipc::Invoke<tauri::Wry>| {
            if invoke.message.webview().label().starts_with("previewbar-") {
                invoke.resolver.reject("not allowed");
                return true;
            }
            let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
                commands::task_create,
                commands::task_resume,
                commands::session_home_index,
                commands::session_home_preview,
                commands::today_list,
                commands::today_range,
                commands::today_add,
                commands::today_set_status,
                commands::today_reorder,
                commands::today_remove,
                commands::today_move,
                commands::today_start,
                commands::today_suggest,
                commands::today_take,
                commands::today_close,
                commands::knowledge_search,
                commands::knowledge_sync,
                commands::knowledge_vaults_get,
                commands::knowledge_vaults_set,
                commands::wiki_workspace_graph,
                commands::wiki_workspace_read,
                commands::wiki_workspace_save,
                commands::wiki_workspace_trash,
                commands::knowledge_vault_status,
                commands::knowledge_vault_connect,
                commands::knowledge_vault_disconnect,
                commands::knowledge_vault_session_open,
                commands::knowledge_vault_list_documents,
                commands::knowledge_vault_document,
                commands::knowledge_vault_archive,
                commands::knowledge_vault_scope,
                commands::knowledge_vault_create_note,
                commands::knowledge_vault_update_note,
                commands::knowledge_vault_search,
                commands::knowledge_vault_prepare_attachments,
                commands::knowledge_vault_register_project,
                commands::knowledge_vault_rebind_project,
                commands::knowledge_vault_rebind,
                commands::knowledge_vault_settings_get,
                commands::knowledge_vault_settings_set,
                commands::knowledge_vault_usage,
                commands::knowledge_vault_import_files,
                commands::knowledge_vault_text_source,
                commands::knowledge_vault_url_source,
                commands::knowledge_vault_scan,
                commands::knowledge_vault_open_original,
                commands::knowledge_vault_recover_operations,
                // 파일형 메모리(설계 2026-09-13) — 창고의 이웃 폴더라 창고 커맨드 옆에 둔다.
                commands::memory_settings_get,
                commands::memory_settings_set,
                commands::memory_files_list,
                commands::memory_file_open,
                commands::memory_session_open,
                commands::insight_memory_append,
                commands::wiki_spaces,
                commands::wiki_connect,
                commands::wiki_sync,
                commands::wiki_documents,
                commands::wiki_read_document,
                // Gmail 소스 (설계 0020 Phase 4). **runner/mobile 라우터에는 넣지 않는다** —
                // 넣는 순간 `knowledge::tests::isolation`이 실패한다 (DR-6).
                commands::knowledge_gmail_status,
                commands::knowledge_gmail_config_set,
                commands::knowledge_gmail_connect,
                commands::knowledge_gmail_disconnect,
                commands::knowledge_gmail_estimate,
                commands::knowledge_gmail_sync,
                commands::observed_models,
                commands::task_diff_stat,
                commands::project_search,
                commands::task_diff,
                commands::diff_hunks,
                commands::annotations_list,
                commands::annotation_save,
                commands::annotations_resend,
                commands::partial_apply,
                commands::partial_rollback,
                commands::task_approve,
                commands::task_approval_status,
                commands::approval_repair_status,
                commands::approval_repair_prepare,
                commands::approval_repair_run,
                commands::approval_repair_cancel,
                commands::approval_repair_accept,
                commands::conflict_begin,
                commands::conflict_resolve,
                commands::conflict_finish,
                commands::conflict_abort,
                commands::checkpoint_create,
                commands::checkpoint_list,
                commands::convo_rewind,
                commands::task_run_approve,
                commands::task_run_reject,
                commands::task_cancel,
                commands::notification_source_page,
                commands::notification_snapshot,
                commands::project_plan_get,
                commands::project_plan_save,
                commands::pipeline_vendors,
                commands::pipeline_start,
                commands::pipeline_list,
                commands::pipeline_get,
                commands::pipeline_update_tickets,
                commands::pipeline_approve_plan,
                commands::pipeline_reject_plan,
                commands::pipeline_retry_ticket,
                commands::pipeline_skip_ticket,
                commands::pipeline_resume,
                commands::pipeline_cancel,
                commands::project_plan_import,
                commands::purpose_get,
                commands::purpose_request_refresh,
                commands::task_result_get,
                commands::task_result_reference,
                commands::task_review_get,
                commands::task_review_decide,
                commands::task_review_deliver,
                commands::notification_ingest,
                commands::notification_acknowledge,
                commands::notification_reconcile,
                commands::notification_settings_set,
                commands::notification_permission,
                commands::notification_test,
                commands::task_discard,
                commands::tasks_discard_orphans,
                commands::task_list,
                commands::quickopen_search,
                commands::task_write,
                commands::knowledge_vault_local_composer_send,
                commands::task_resize,
                commands::task_pty_replay,
                commands::verify_spec,
                commands::task_verify,
                commands::task_capsule,
                commands::capsule_inject,
                commands::convo_context_reset,
                commands::ensemble_list,
                commands::ensemble_metrics,
                commands::ensemble_feedback_history,
                commands::ensemble_judge,
                commands::interview_start,
                commands::interview_crystallize,
                commands::grill_round,
                commands::grill_note,
                commands::grill_save_note,
                commands::ensemble_matrix,
                commands::ensemble_compose,
                commands::convo_send,
                commands::conversation_submit,
                commands::conversation_receipt,
                commands::side_question_read,
                commands::side_question_send,
                commands::side_question_cancel,
                commands::side_question_reset,
                commands::convo_history,
                convo::interaction_commands::interaction_snapshot,
                convo::interaction_commands::interaction_draft,
                convo::interaction_commands::interaction_pending_tasks,
                convo::interaction_commands::interaction_answer,
                convo::interaction_commands::interaction_receipt,
                convo::interaction_commands::interaction_cleanup_retry,
                commands::task_tool_cost,
                commands::convo_status,
                commands::task_activity,
                commands::quiz_next,
                commands::quiz_availability,
                commands::insight_availability,
                commands::insight_next,
                commands::insight_wiki_folders_set,
                commands::insight_enabled_get,
                commands::insight_enabled_set,
                commands::quiz_answer,
                commands::quiz_report,
                commands::quiz_approve,
                commands::quiz_pending,
                commands::convo_interrupt,
                commands::debate_sides,
                commands::debate_start,
                commands::debate_end,
                commands::debate_round_cap_get,
                commands::debate_round_cap_set,
                commands::task_delete,
                commands::lsp_autoinject_get,
                commands::lsp_autoinject_set,
                commands::use_worktree_get,
                commands::refresh_base_get,
                commands::refresh_base_set,
                commands::use_worktree_set,
                commands::use_worktree_override_get,
                commands::use_worktree_override_clear,
                commands::voice_settings_get,
                commands::voice_settings_set,
                commands::translate_settings_get,
                commands::translate_settings_set,
                commands::translate_text,
                commands::vocab_list,
                commands::vocab_due,
                commands::vocab_due_count,
                commands::vocab_save,
                commands::vocab_mark_known,
                commands::vocab_set_status,
                commands::vocab_review,
                commands::vocab_delete,
                commands::voice_stt_test,
                commands::voice_server_status,
                commands::voice_server_start,
                commands::voice_server_stop,
                commands::font_settings_get,
                commands::font_settings_set,
                commands::editor_settings_get,
                commands::editor_settings_set,
                commands::editor_window_open,
                commands::editor_window_hide,
                commands::editor_window_focus,
                commands::editor_window_alive,
                commands::editor_window_geometry_save,
                commands::editor_window_files_save,
                commands::editor_window_files_load,
                commands::system_fonts_list,
                commands::theme_list,
                commands::theme_save,
                commands::theme_delete,
                commands::theme_export,
                commands::theme_import,
                commands::agent_models_get,
                commands::agent_model_set,
                commands::task_model_set,
                commands::service_tier::codex_speed_models,
                commands::service_tier::task_service_tier_set,
                commands::task_agent_set,
                commands::block_unverified_get,
                commands::block_unverified_set,
                commands::remote_review_commands_get,
                commands::remote_review_commands_set,
                commands::mobile_surface_status,
                commands::mobile_surface_set_enabled,
                commands::mobile_surface_set_port,
                commands::mobile_surface_set_prevent_sleep,
                commands::mobile_pairing_create,
                commands::mobile_session_list,
                commands::mobile_session_revoke,
                commands::fs_tree,
                commands::fs_tree_path,
                commands::fs_browse,
                commands::fs_roots,
                commands::fs_create_file,
                commands::fs_create_dir,
                commands::fs_rename,
                commands::fs_trash,
                commands::fs_copy,
                commands::fs_open_terminal,
                commands::git_status_path,
                commands::git_init_path,
                commands::git_branches_path,
                commands::git_checkout_branch_path,
                commands::fs_read,
                commands::fs_write,
                commands::open_local_file,
                commands::read_local_file,
                commands::resolve_abs_path,
                commands::lsp_goto,
                commands::lsp_status,
                commands::lsp_semantic_tokens,
                commands::mcp_list,
                commands::mcp_add,
                commands::mcp_remove,
                commands::mcp_set_enabled,
                commands::schedule_list,
                commands::schedule_add,
                commands::schedule_remove,
                commands::schedule_set_enabled,
                commands::cron_next_runs,
                commands::reminder_add,
                commands::capture_profile_get,
                commands::capture_profile_set,
                commands::max_concurrent_get,
                commands::max_concurrent_set,
                commands::insights_compute,
                commands::insights_agent_skills,
                commands::usage_snapshot,
                commands::agent_health,
                commands::auto_update_get,
                commands::auto_update_set,
                commands::auto_update_last,
                commands::antigravity_hub_update,
                commands::agent_action_open,
                commands::agent_action_replay,
                commands::agent_action_write,
                commands::agent_action_resize,
                commands::agent_auth_reconcile,
                commands::usage_bridge_status,
                commands::usage_bridge_install,
                commands::usage_bridge_uninstall,
                commands::usage_claude_token_set,
                commands::usage_claude_token_clear,
                commands::usage_claude_token_status,
                commands::outcome_insights,
                commands::task_patterns,
                commands::insight_cards,
                commands::insight_card_dismiss,
                commands::lesson_themes,
                commands::default_shell,
                commands::shell_open,
                commands::shell_write,
                commands::shell_resize,
                commands::shell_replay,
                commands::shell_detach,
                commands::repl_open,
                commands::repl_run,
                commands::repl_write,
                commands::repl_resize,
                commands::repl_replay,
                commands::repl_detach,
                commands::repl_close,
                commands::skills_list,
                commands::skills_read,
                commands::harness_experience_list,
                commands::harness_experience_read,
                commands::multi_review,
                commands::review_history_list,
                commands::review_get,
                commands::review_delete,
                commands::github_issues_list,
                commands::github_repos_list,
                commands::github_create_task_from_issue,
                commands::github_issue_delete,
                commands::designmode_open,
                commands::designmode_set_bounds,
                commands::designmode_show,
                commands::designmode_state,
                commands::designmode_navigate,
                commands::designmode_set_selection_mode,
                commands::designmode_hide,
                commands::designmode_close,
                commands::designmode_remove_capture,
                commands::designmode_capture_editor,
                commands::preview_take_over,
                commands::preview_release,
                commands::preview_workbench_state,
                commands::preview_workbench_prepare,
                commands::preview_workbench_send,
                commands::preview_workbench_receipt,
                commands::paste_capture_save,
                commands::paste_image_save,
                project_editor::project_editor_open,
                project_editor::project_editor_info,
                project_editor::project_editor_tree,
                project_editor::project_editor_read,
                project_editor::project_editor_write,
                project_editor::project_editor_resolve_path,
                project_editor::project_editor_open_path,
                project_editor::project_editor_shell_open,
                project_editor::project_editor_shell_snapshot,
                project_editor::project_editor_shell_write,
                project_editor::project_editor_shell_resize,
                project_editor::project_editor_shell_close
            ];
            handler(invoke)
        })
        .on_window_event(|window, event| {
            if window.label()=="main" && matches!(event,tauri::WindowEvent::Focused(true)) {convo::interaction_commands::reopened();}
            if matches!(event, tauri::WindowEvent::Destroyed) {
                // 팝인은 숨김이라 에디터 창이 여기 오면 실제로 죽은 것이다. 메인 창의 팝아웃
                // 상태는 editor://closed로만 풀리는데 죽은 창은 그것을 보내지 못한다 — 대신 알린다.
                if window.label() == editorwindow::WINDOW_LABEL {
                    let _ = window.app_handle().emit(editorwindow::GONE_EVENT, ());
                }
                let state = window.state::<project_editor::ProjectEditorState>();
                // cleanup이 등록을 지우므로 그 전에 루트를 꺼내 알린다 — 창고 재스캔 시점이다.
                if let Some(root) = project_editor::root_of_window(&state, window.label()) {
                    let _ = window.app_handle().emit("project-editor://closed", root);
                }
                project_editor::cleanup_window(&state, window.label());
            }
            // R7: 창 닫기 시 PTY 자식만 정리. 대화(convo) 자식은 의도적으로 살려둔다
            // (재시작 복원 전제 — BR-2, Plan 0012). 재시작 시 reconcile_stale_running이 생존 확인 후 복원.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() != "main" {
                    return;
                }
                if convo::interaction_commands::shutdown(window.app_handle(),false) {api.prevent_close();return;}
                // 파이프라인 자식 프로세스는 여기서 죽이지 않는다. 메인 창을 닫아도 앱이 남을 수 있는데,
                // kill_everything은 실행별 취소 표시를 영구히 남겨 드라이버가 재시작 전까지 멈추기 때문이다.
                // 실제 종료(ExitRequested)에서만 정리한다.
                let state = window.state::<AppState>();
                let tasks = state.tasks.lock().unwrap();
                for a in tasks.values() {
                    if let Some(s) = &a.session {
                        s.terminate();
                    }
                }
                // 워크스페이스 셸(도구 패널 터미널)도 함께 정리 — 고아 셸 방지.
                for slot in state.shells.lock().unwrap().values() {
                    slot.session.terminate();
                }
                // 에디터 팝아웃의 Python 콘솔(IPython)도 같은 이유로 정리한다.
                for repl in state.repls.lock().unwrap().values() {
                    repl.slot.session.terminate();
                }
                let project_editor = window.state::<project_editor::ProjectEditorState>();
                project_editor::cleanup_all(&project_editor);
                // 앱이 띄운 로컬 STT 서버도 함께 죽인다 — 고아로 남으면 다음 기동이 포트 선점으로 실패한다.
                window.state::<voice::server::ServerManaged>().stop();
                side_question::shutdown_all();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building praxis")
        .run(|app,event| {
            if let tauri::RunEvent::ExitRequested {api,..}=event {
                if convo::interaction_commands::shutdown(app,true) {api.prevent_exit();}
                else { pipeline::registry::kill_everything(); }
            }
        });
}
