//! 데스크톱이 직접 서빙하는 모바일 표면 — 설계 2026-09-13 D1·D4·D5.
//!
//! 러너가 쓰던 PWA 표면(`/m/*` 셸 + `/v1/*` API)을 **데스크톱 프로세스 안에서** 그대로 띄운다.
//! 읽기 핸들러는 `praxis.sqlite`를 그대로 읽고, 쓰기는 `TaskActions`로 데스크톱 IPC 명령에
//! 결선한다(`DesktopTaskActions`). 텔레그램 봇이 하던 "데스크톱 명령을 외부 채널에 결선"을
//! 전송 계층만 바꿔 되살린 것이다.
//!
//! 밖으로는 `tailscale serve`로만 낸다 — 여기서는 루프백에만 bind한다.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::StatusCode;
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager};

use crate::commands::AppState;
use crate::db;
use crate::runner::actions::{ApiError, SharedTaskActions, TaskActions};
use crate::runner::auth::RunnerAuth;
use crate::runner::config::{ExecutionPolicy, RunnerConfig};
use crate::runner::events::EventHub;
use crate::runner::http::{self, RunnerHttpState};
use crate::runner::queue::QueueWorker;

/// 서빙 on/off 복원용 설정 키. 기본 OFF — 네트워크 표면은 명시적으로 켠 것만 뜬다.
pub const ENABLED_KEY: &str = "mobile_surface_enabled";
/// 루프백 bind 포트. 러너 기본값(47831)과 겹치지 않게 둔다.
pub const PORT_KEY: &str = "mobile_surface_port";
/// 서빙 중 Mac 잠자기 방지(caffeinate). 잠들면 폰에서 아무것도 안 보인다(D5).
pub const PREVENT_SLEEP_KEY: &str = "mobile_surface_prevent_sleep";

pub const DEFAULT_PORT: u16 = 47832;

/// 현재 서빙 상태 — 설정 화면이 그대로 읽는 모양.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MobileSurfaceStatus {
    pub running: bool,
    pub port: u16,
    pub prevent_sleep: bool,
    /// 잠자기 방지를 켰는데 `caffeinate`를 띄우지 못한 경우(비-macOS 등) false.
    pub sleep_prevented: bool,
}

struct Running {
    port: u16,
    /// axum graceful shutdown 신호. drop만으로는 부족해 명시적으로 보낸다.
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    /// Web Push 워처. 멈출 때 함께 거두지 않으면 포트를 바꿀 때마다 하나씩 쌓여
    /// 같은 이벤트로 푸시가 여러 번 나간다.
    push_watch: Option<tauri::async_runtime::JoinHandle<()>>,
    caffeinate: Option<std::process::Child>,
}

/// Tauri managed state. 서버 수명을 앱 수명에 묶는다 — 앱이 꺼지면 표면도 꺼진다.
#[derive(Default)]
pub struct MobileSurface {
    running: Mutex<Option<Running>>,
    /// EventHub는 **한 번만** 만든다. `start`가 내부에서 50ms 폴링 태스크를 띄우는데
    /// 그 핸들을 돌려주지 않아, 껐다 켤 때마다 새로 만들면 폴러가 누적된다.
    events: Mutex<Option<EventHub>>,
}

impl MobileSurface {
    pub fn status(&self, prevent_sleep: bool, configured_port: u16) -> MobileSurfaceStatus {
        let guard = self.running.lock().unwrap_or_else(|error| error.into_inner());
        match guard.as_ref() {
            Some(running) => MobileSurfaceStatus {
                running: true,
                port: running.port,
                prevent_sleep,
                sleep_prevented: running.caffeinate.is_some(),
            },
            None => MobileSurfaceStatus {
                running: false,
                port: configured_port,
                prevent_sleep,
                sleep_prevented: false,
            },
        }
    }

    pub fn is_running(&self) -> bool {
        self.running
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_some()
    }

    /// 서버를 멈추고 잠자기 방지도 해제한다. 이미 꺼져 있으면 아무 일도 하지 않는다.
    pub fn stop(&self) {
        let taken = self
            .running
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        let Some(mut running) = taken else {
            return;
        };
        if let Some(stop) = running.stop.take() {
            let _ = stop.send(());
        }
        if let Some(watch) = running.push_watch.take() {
            watch.abort();
        }
        if let Some(child) = running.caffeinate.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// 표면에 필요한 스키마(페어링·세션·푸시 구독)를 데스크톱 DB에도 만든다.
/// 러너와 같은 `db` 모듈을 쓰므로 테이블 정의는 하나뿐이다.
pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    crate::runner::session::migrate(pool).await?;
    crate::runner::push::migrate(pool).await?;
    Ok(())
}

/// 부팅 시 설정을 읽어 켜져 있었으면 되띄운다. 실패는 부팅을 막지 않는다.
pub async fn restore(app: &AppHandle, pool: &SqlitePool) {
    if db::get_setting(pool, ENABLED_KEY).await.ok().flatten().as_deref() != Some("true") {
        return;
    }
    if let Err(error) = start(app, pool).await {
        eprintln!("모바일 표면을 되살리지 못했습니다: {error}");
    }
}

pub async fn configured_port(pool: &SqlitePool) -> u16 {
    db::get_setting(pool, PORT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|value| value.trim().parse().ok())
        .filter(|port| *port >= 1024)
        .unwrap_or(DEFAULT_PORT)
}

pub async fn prevent_sleep_enabled(pool: &SqlitePool) -> bool {
    db::get_setting(pool, PREVENT_SLEEP_KEY)
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("true")
}

/// 모바일 표면을 띄운다. 이미 떠 있으면 그대로 둔다(멱등).
pub async fn start(app: &AppHandle, pool: &SqlitePool) -> Result<MobileSurfaceStatus, String> {
    let surface = app.state::<MobileSurface>();
    let prevent_sleep = prevent_sleep_enabled(pool).await;
    let port = configured_port(pool).await;
    if surface.is_running() {
        return Ok(surface.status(prevent_sleep, port));
    }

    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("app data 디렉터리를 찾을 수 없습니다: {error}"))?;
    std::fs::create_dir_all(&data_dir).map_err(|error| error.to_string())?;

    let token_file = data_dir.join("mobile-pairing-token");
    ensure_pairing_token(&token_file)?;
    let auth = RunnerAuth::from_file(&token_file)?;

    migrate(pool).await.map_err(|error| error.to_string())?;

    let config = RunnerConfig {
        bind: SocketAddr::from(([127, 0, 0, 1], port)),
        repository_roots: repository_roots(pool).await,
        // 폰에서 만든 작업은 데스크톱이 직접 돌린다. 큐 워커가 없으므로 이 값은
        // health 표시용이다.
        max_concurrent_tasks: 1,
        // 외부기원 작업은 승인 없이 시작하지 않는다 — 폰이 원격이라는 사실은 변하지 않는다.
        execution_policy: ExecutionPolicy::RequireApproval,
        pairing_token_file: token_file,
    };

    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .map_err(|error| format!("모바일 표면 bind 실패({}): {error}", config.bind))?;

    // EventHub::start는 내부에서 폴링 태스크를 띄운다 — tokio 컨텍스트 안에서, 그리고
    // 앱 수명 동안 **한 번만** 만든다.
    let events = {
        let mut guard = surface
            .events
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        guard
            .get_or_insert_with(|| EventHub::start(pool.clone()))
            .clone()
    };

    let app_state = app.state::<AppState>();
    let state = RunnerHttpState {
        auth,
        pool: pool.clone(),
        config,
        recovered_tasks: 0,
        events: events.clone(),
        // 큐는 **잠재워 둔다**. 데스크톱에는 워커 루프가 없고, 여기서는 worktree 락
        // 핸들만 쓰인다. `spawn_next`를 부르는 곳이 없으므로 작업을 가로채지 않는다.
        queue: QueueWorker::new(pool.clone(), 1),
        started_at: crate::now(),
        // 승인·verify 점유는 IPC와 **같은 것**을 써야 한다. 갈라 두면 폰과 데스크톱이
        // 같은 작업을 동시에 종결한다.
        review_claims: app_state.review_claims.clone(),
    };

    // Web Push는 EventHub 구독으로 붙는다. 키를 못 만들면 알림만 조용히 빠진다.
    let push_watch = match crate::runner::push::VapidKeys::load_or_create(&data_dir.join("vapid.key"))
    {
        Ok(keys) => Some(tauri::async_runtime::spawn(crate::runner::push::watch(
            pool.clone(),
            state.events.clone(),
            keys,
        ))),
        Err(error) => {
            eprintln!("Web Push 비활성 — VAPID 키를 준비하지 못했습니다: {error}");
            None
        }
    };

    let actions: SharedTaskActions = Arc::new(DesktopTaskActions { app: app.clone() });
    let router = http::mobile_surface_router(state, actions);
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    tauri::async_runtime::spawn(async move {
        let served = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = stop_rx.await;
        })
        .await;
        if let Err(error) = served {
            eprintln!("모바일 표면 서버가 종료되었습니다: {error}");
        }
    });

    let caffeinate = prevent_sleep.then(spawn_caffeinate).flatten();
    let sleep_prevented = caffeinate.is_some();
    *surface
        .running
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some(Running {
        port,
        stop: Some(stop_tx),
        push_watch,
        caffeinate,
    });
    Ok(MobileSurfaceStatus {
        running: true,
        port,
        prevent_sleep,
        sleep_prevented,
    })
}

/// 폰이 열람할 수 있는 repo 경로 목록. `authorize_repository_path`가 이 목록으로 판정한다.
///
/// **알려진 한계**: 서빙 시작 시점에 한 번만 계산한다. 새 repo를 추가한 뒤에는 표면을
/// 껐다 켜야 폰에서 보인다. 매 요청 조회로 바꾸면 canonicalize가 요청마다 디스크를 친다.
async fn repository_roots(pool: &SqlitePool) -> Vec<PathBuf> {
    let repos = db::known_repos(pool).await.unwrap_or_default();
    let mut roots: Vec<PathBuf> = repos
        .iter()
        .filter_map(|repo| std::fs::canonicalize(repo).ok())
        .collect();
    roots.sort();
    roots.dedup();
    roots
}

/// 페어링 토큰 파일이 없으면 만든다. 이미 있으면 손대지 않는다 — 이미 페어링한 기기의
/// 자격을 조용히 무효화하면 안 된다.
fn ensure_pairing_token(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| format!("난수 생성 실패: {error}"))?;
    let mut hex = String::with_capacity(64);
    for byte in bytes {
        hex.push(char::from_digit((byte >> 4) as u32, 16).unwrap_or('0'));
        hex.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap_or('0'));
    }
    std::fs::write(path, &hex).map_err(|error| format!("pairing token 생성 실패: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        // `RunnerAuth::from_file`이 0600을 강제한다 — 만들 때 맞춰 둔다.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("pairing token 권한 설정 실패: {error}"))?;
    }
    Ok(())
}

/// 잠자기 방지. macOS 밖에서는 할 일이 없으므로 None을 돌려준다.
fn spawn_caffeinate() -> Option<std::process::Child> {
    #[cfg(target_os = "macos")]
    {
        // -i: 유휴 잠자기 방지, -m: 디스크 잠자기 방지. 디스플레이는 끄게 둔다.
        // -w <pid>: 우리가 죽으면 함께 끝난다 — 강제 종료로도 방지가 남지 않게.
        std::process::Command::new("caffeinate")
            .args(["-i", "-m", "-w", &std::process::id().to_string()])
            .spawn()
            .map_err(|error| eprintln!("caffeinate 실행 실패 — 잠자기 방지 없음: {error}"))
            .ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

// ── 쓰기 경로: 데스크톱 명령으로의 결선 (D4) ──────────────────────────────

/// 모바일 표면의 작업 행위를 데스크톱 IPC 명령에 결선한다.
///
/// 승인·폐기는 `remote_review_commands_enabled` 토글로 막힌다(기본 OFF). 대화 메시지는
/// **막지 않는다** — 토글이 OFF여도 폰은 읽기·대화까지는 할 수 있어야 한다는 것이 이번
/// 설계의 목적이다(설계 2026-09-13 D4). 승인만이 되돌리기 어려운 행위다.
struct DesktopTaskActions {
    app: AppHandle,
}

fn bad_request(error: String) -> ApiError {
    (StatusCode::BAD_REQUEST, error)
}

fn not_found() -> ApiError {
    (StatusCode::NOT_FOUND, "task를 찾을 수 없습니다".to_string())
}

impl DesktopTaskActions {
    /// 되돌리기 어려운 행위(승인·폐기)의 게이트. OFF면 403.
    async fn require_review_commands(&self, pool: &SqlitePool) -> Result<(), ApiError> {
        if crate::commands::remote_review_enabled(pool).await {
            Ok(())
        } else {
            Err((
                StatusCode::FORBIDDEN,
                "원격 리뷰 커맨드가 꺼져 있습니다 — 설정 > 모바일에서 켜세요".to_string(),
            ))
        }
    }
}

#[async_trait]
impl TaskActions for DesktopTaskActions {
    async fn create(
        &self,
        cx: &RunnerHttpState,
        request: crate::runner::QueuedTaskRequest,
    ) -> Result<db::Task, ApiError> {
        let state = self.app.state::<AppState>();
        // External 기원 — `create_task_internal`이 repo 화이트리스트를 검증하고
        // PENDING_APPROVAL로 멈춘다. 실행 허가는 `/v1/tasks/:id/run`이 준다.
        let mut params = crate::orchestrator::CreateTaskParams::headless_terminal(
            request.repository,
            request.instruction,
            request.agent,
            crate::orchestrator::TaskOrigin::External,
        );
        params.role = request.role;
        params.model = request.model;
        params.reasoning_effort = request.reasoning_effort;
        params.mode = request.mode;
        params.goal_contract = request.goal_contract;
        let _ = cx;
        crate::commands::create_task_internal(&self.app, &state, params)
            .await
            .map_err(bad_request)
    }

    async fn run_approve(&self, cx: &RunnerHttpState, id: i64) -> Result<db::Task, ApiError> {
        let state = self.app.state::<AppState>();
        crate::commands::approve_pending_task(&self.app, &state, id)
            .await
            .map_err(bad_request)?;
        db::get_task(&cx.pool, id)
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
            .ok_or_else(not_found)
    }

    async fn approve(&self, cx: &RunnerHttpState, id: i64) -> Result<(), ApiError> {
        self.require_review_commands(&cx.pool).await?;
        let task = db::get_task(&cx.pool, id)
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
            .ok_or_else(not_found)?;
        // 원격 승인은 로컬과 달리 verify 증거·protected paths를 **항상** 강제한다.
        crate::commands::remote_approval_gate(&cx.pool, &task)
            .await
            .map_err(|error| (StatusCode::CONFLICT, error))?;
        crate::commands::task_approve(self.app.state::<AppState>(), id)
            .await
            .map_err(bad_request)
    }

    async fn discard(&self, cx: &RunnerHttpState, id: i64) -> Result<(), ApiError> {
        self.require_review_commands(&cx.pool).await?;
        crate::commands::task_discard(self.app.clone(), self.app.state::<AppState>(), id)
            .await
            .map_err(bad_request)
    }

    async fn message(
        &self,
        cx: &RunnerHttpState,
        id: i64,
        message: &str,
    ) -> Result<(), ApiError> {
        let _ = cx;
        let state = self.app.state::<AppState>();
        crate::commands::remote_review_retry(&self.app, &state, id, message.to_string())
            .await
            .map_err(|error| (StatusCode::CONFLICT, error))
    }

    async fn verify(
        &self,
        cx: &RunnerHttpState,
        id: i64,
        preview_token: String,
    ) -> Result<crate::verify::VerifyReport, ApiError> {
        let _ = cx;
        crate::commands::task_verify(self.app.state::<AppState>(), id, preview_token)
            .await
            .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error))
    }

    async fn output(
        &self,
        cx: &RunnerHttpState,
        id: i64,
        after: i64,
        limit: i64,
    ) -> Result<Vec<db::TaskOutput>, ApiError> {
        db::list_task_output_for_task_after(&cx.pool, id, after, limit)
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
    }
}
