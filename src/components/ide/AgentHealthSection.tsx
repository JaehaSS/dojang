import { useCallback, useEffect, useState, type ReactElement } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  agentAuthReconcile,
  agentHealth,
  antigravityHubUpdate,
  autoUpdateGet,
  autoUpdateLast,
  autoUpdateSet,
  AUTO_UPDATE_EVENT,
  type AgentActionKind,
  type AutoUpdateReport,
  type HubUpdate,
  type VendorHealth,
} from "../../lib/ipc";
import {
  accountLine,
  authBadge,
  canUpdate,
  hubNotice,
  methodLabel,
  outcomeLine,
  updateHint,
  versionLabel,
} from "../../lib/agent-health";
import { ActionTerminal } from "./ActionTerminal";

type ActionTarget = { kind: AgentActionKind; vendor: string };

const actionButton =
  "text-[11px] px-1.5 py-0.5 rounded border border-border text-text-secondary " +
  "hover:bg-raised hover:text-text disabled:opacity-40 disabled:hover:bg-transparent";

function VendorRow({
  health,
  onAction,
}: {
  health: VendorHealth;
  onAction: (target: ActionTarget) => void;
}): ReactElement {
  const badge = authBadge(health.auth);
  const account = accountLine(health);
  const hint = updateHint(health);
  return (
    <div className="py-2 border-b border-border last:border-b-0 space-y-1">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-text">{health.label}</span>
            <span className={`text-xs ${badge.color}`}>
              {badge.mark} {badge.text}
            </span>
          </div>
          {account && <div className="text-text-muted text-xs truncate">{account}</div>}
          {health.install_method === "unknown" && health.bin_path && (
            <div className="text-text-muted text-[11px] truncate" title={health.bin_path}>
              {health.bin_path}
            </div>
          )}
        </div>
        <div className="text-right shrink-0">
          <div
            className={`text-xs ${health.update_available ? "text-status-awaiting" : "text-text-muted"}`}
          >
            {versionLabel(health)}
          </div>
          <div className="text-text-muted text-[11px]">{methodLabel(health.install_method)}</div>
        </div>
      </div>
      <div className="flex items-center gap-1.5">
        <button
          type="button"
          className={actionButton}
          onClick={() => onAction({ kind: "login", vendor: health.vendor })}
        >
          {health.auth === "ok" ? "재로그인" : "로그인"}
        </button>
        <button
          type="button"
          className={actionButton}
          disabled={!canUpdate(health)}
          title={hint ?? undefined}
          onClick={() => onAction({ kind: "update", vendor: health.vendor })}
        >
          업데이트
        </button>
        {health.vendor === "claude" && (
          <button
            type="button"
            className={actionButton}
            onClick={() => onAction({ kind: "doctor", vendor: health.vendor })}
          >
            진단
          </button>
        )}
        {hint && <span className="text-[11px] text-text-muted">{hint}</span>}
      </div>
    </div>
  );
}

const ACTION_TITLE: Record<AgentActionKind, string> = {
  login: "로그인",
  logout: "로그아웃",
  update: "업데이트",
  doctor: "진단",
  scratch: "셸",
};

/**
 * 벤더 CLI의 인증·버전 상태와 액션.
 *
 * 로그인·업데이트·자유 셸이 전부 앱 안 PTY에서 돈다. 프로세스가 끝나면 상태를 자동으로
 * 다시 읽어 배지를 갱신한다 — 로그인해 놓고 "여전히 로그인 필요"로 보이면 헛걸음을 부른다.
 */
export function AgentHealthSection(): ReactElement {
  const [vendors, setVendors] = useState<VendorHealth[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [action, setAction] = useState<ActionTarget | null>(null);
  const [resumed, setResumed] = useState<number | null>(null);
  // null = 설정을 아직 모른다(읽기 전 또는 읽기 실패). 기본값 true로 두면 꺼져 있는데도
  // 켜진 것처럼 보이고, 그 상태에서 누르면 끄려던 의도가 켜기로 뒤집힌다.
  const [autoUpdate, setAutoUpdate] = useState<boolean | null>(null);
  const [savingAuto, setSavingAuto] = useState(false);
  const [report, setReport] = useState<AutoUpdateReport | null>(null);
  const [hub, setHub] = useState<HubUpdate | null>(null);

  const load = useCallback((force: boolean) => {
    setLoading(true);
    setError(null);
    agentHealth(force)
      .then((snapshot) => setVendors(snapshot.vendors))
      .catch((cause: unknown) => setError(String(cause)))
      .finally(() => setLoading(false));
    // Hub 상태도 같이 다시 읽는다. 사용자가 Antigravity를 재시작하고 새로고침을 눌렀는데
    // 배너가 그대로면, 고친 사람에게 아직 안 고쳤다고 말하는 셈이다.
    antigravityHubUpdate()
      .then(setHub)
      .catch(() => setHub(null));
  }, []);

  /// 더 오래된 리포트가 새 것을 덮지 않게 한다. 마운트 시 조회와 이벤트 수신은 서로
  /// async라 도착 순서가 보장되지 않는다.
  const mergeReport = useCallback((next: AutoUpdateReport) => {
    setReport((prev) => (prev && prev.finished_at > next.finished_at ? prev : next));
  }, []);

  useEffect(() => {
    load(false);
    // 시작 시 자동 업데이트는 이 패널이 열리기 한참 전에 끝난다 — 이벤트만 기다리면
    // 아무것도 못 본다. 지난 결과를 먼저 읽고, 이벤트는 열려 있는 동안의 갱신용으로만 쓴다.
    autoUpdateGet()
      .then(setAutoUpdate)
      .catch(() => setAutoUpdate(null));
    autoUpdateLast().then(mergeReport).catch(() => {});
  }, [load, mergeReport]);

  useEffect(() => {
    const pending = listen<AutoUpdateReport>(AUTO_UPDATE_EVENT, (event) => {
      mergeReport(event.payload);
      load(true);
    });
    return () => {
      void pending.then((stop) => stop());
    };
  }, [load, mergeReport]);

  const toggleAutoUpdate = () => {
    if (autoUpdate === null || savingAuto) return;
    const next = !autoUpdate;
    setAutoUpdate(next);
    setSavingAuto(true);
    autoUpdateSet(next)
      // 저장이 실제로 무엇으로 끝났는지는 서버가 안다 — 빠르게 두 번 눌러도 화면이 갈리지 않는다.
      .then(setAutoUpdate)
      .catch((cause: unknown) => {
        // 저장에 실패했는데 켜진 것처럼 보이면 다음 시작에 안 도는 이유를 알 수 없다.
        setAutoUpdate(!next);
        setError(String(cause));
      })
      .finally(() => setSavingAuto(false));
  };

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <div className="text-text-muted text-xs">
          {loading ? "CLI에 상태를 묻는 중…" : "설치된 CLI의 로그인·버전 상태입니다."}
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            className="text-xs text-text-muted hover:text-text"
            onClick={() => setAction({ kind: "scratch", vendor: "" })}
          >
            셸 열기
          </button>
          <button
            type="button"
            className="text-xs text-text-muted hover:text-text disabled:opacity-50"
            disabled={loading}
            onClick={() => load(true)}
          >
            새로고침
          </button>
        </div>
      </div>
      <div className="flex items-center justify-between gap-4 py-2 border-b border-border">
        <div className="min-w-0">
          <div className="text-text text-xs">시작할 때 자동으로 업데이트</div>
          <div className="text-text-muted text-[11px]">
            앱을 켠 직후에만 돕니다 — 작업이 돌고 있으면 바이너리를 바꾸지 않고 건너뜁니다.
          </div>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={autoUpdate ?? false}
          aria-label="시작할 때 자동으로 업데이트"
          disabled={autoUpdate === null || savingAuto}
          title={autoUpdate === null ? "설정을 읽지 못했습니다" : undefined}
          onClick={toggleAutoUpdate}
          className={`w-11 h-6 shrink-0 rounded-full relative transition-colors disabled:opacity-40 ${
            autoUpdate ? "bg-primary" : "bg-border"
          }`}
        >
          <span
            className={`absolute top-0.5 w-5 h-5 rounded-full bg-bg transition-all ${
              autoUpdate ? "left-[22px]" : "left-0.5"
            }`}
          />
        </button>
      </div>
      {autoUpdate === null && (
        <div className="text-[11px] text-status-failed">
          자동 업데이트 설정을 읽지 못했습니다 — 지금은 켜짐/꺼짐을 바꿀 수 없습니다.
        </div>
      )}
      {report && (report.outcomes.length > 0 || report.skipped) && (
        <div className="space-y-0.5">
          {report.outcomes.map((outcome) => (
            <div
              key={outcome.vendor}
              className={`text-[11px] ${outcome.ok ? "text-status-done" : "text-status-failed"}`}
            >
              {outcomeLine(outcome)}
            </div>
          ))}
          {report.skipped && (
            <div className="text-[11px] text-text-muted">{report.skipped}</div>
          )}
        </div>
      )}
      {error && <div className="text-xs text-status-failed">{error}</div>}
      {resumed !== null && (
        <div className="text-xs text-status-done">
          인증이 회복돼 대기 중이던 작업 {resumed}개가 다시 큐로 돌아갔습니다.
        </div>
      )}
      {vendors?.map((health) => (
        <VendorRow key={health.vendor} health={health} onAction={setAction} />
      ))}
      {vendors?.length === 0 && (
        <div className="text-text-muted text-xs">확인할 수 있는 CLI가 없습니다.</div>
      )}
      {hub && hubNotice(hub) && (
        <div className="text-[11px] text-status-awaiting py-1">
          {hubNotice(hub)}
          <div className="text-text-muted">
            받아둔 업데이트는 앱이 꺼질 때 설치됩니다 — Dojang이 대신 종료하지는 않습니다.
          </div>
        </div>
      )}
      {action && (
        <div className="border border-border rounded overflow-hidden">
          <div className="flex items-center justify-between px-2 py-1 bg-surface">
            <span className="text-xs text-text-secondary">
              {action.vendor ? `${action.vendor} · ` : ""}
              {ACTION_TITLE[action.kind]}
            </span>
            <button
              type="button"
              className="text-xs text-text-muted hover:text-text"
              onClick={() => setAction(null)}
            >
              닫기
            </button>
          </div>
          <div className="h-64">
            <ActionTerminal
              kind={action.kind}
              vendor={action.vendor}
              // 프로세스가 끝나면 상태를 다시 읽고, 인증이 회복됐으면 막아둔 작업을 푼다.
              // 창은 닫지 않는다 — 실패 메시지가 남아 있어야 무엇이 잘못됐는지 볼 수 있다.
              onExit={() => {
                load(true);
                if (action.kind === "login" || action.kind === "logout") {
                  agentAuthReconcile()
                    .then((count) => setResumed(count > 0 ? count : null))
                    .catch(() => setResumed(null));
                }
              }}
            />
          </div>
        </div>
      )}
    </div>
  );
}
