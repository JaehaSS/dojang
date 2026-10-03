import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import {
  mobileSurfaceStatus,
  mobileSurfaceSetEnabled,
  mobileSurfaceSetPort,
  mobileSurfaceSetPreventSleep,
  mobilePairingCreate,
  mobileSessionList,
  mobileSessionRevoke,
  remoteReviewCommandsGet,
  remoteReviewCommandsSet,
  type MobileSurfaceStatus,
  type MobilePairing,
  type MobileSession,
} from "../lib/ipc";
import { ago } from "../lib/fmt";
import { getTransport, getTransportRevision, listHosts, subscribeTransportChange } from "../lib/transport";
import { RunnerTransport } from "../lib/transport/runner";
import { inputCls } from "./ide/formStyles";
import { ToggleBadge, ResourceRow, DeleteButton } from "./ide/ResourceRow";
import { MobilePairingCard } from "./ide/MobilePairingCard";
import { useHighlight } from "./ide/settings/SettingRow";
import { MetaTag } from "./MetaTag";

/** 페어링 코드 수명 — 백엔드 `session::PAIRING_TTL_SECS`와 같은 값이다. */
const PAIRING_HINT = "코드는 한 번만 쓸 수 있고 만료되면 다시 발급해야 합니다.";

function Section({
  id,
  title,
  hint,
  children,
}: {
  /** 설정 카탈로그의 항목 id. 검색이 지목할 수 있는 자리에만 준다. */
  id?: string;
  title: string;
  hint?: string;
  children: React.ReactNode;
}) {
  const { ref, ring } = useHighlight(id);
  return (
    <div
      ref={ref}
      data-setting-id={id}
      className={`bg-surface border border-border rounded-lg p-3 mb-4 ${ring}`}
    >
      <div className="text-sm font-medium mb-1">{title}</div>
      {hint && <div className="text-text-muted text-xs mb-2">{hint}</div>}
      {children}
    </div>
  );
}

/**
 * 지금 붙어 있는 Runner — 없으면 null.
 * Runner에 연결된 폰은 Runner가 세션을 들고 있으므로 여기서만 페어링할 수 있다.
 */
function useRunnerTransport(): RunnerTransport | null {
  // revision이 바뀌면 붙어 있는 Runner도 바뀌었을 수 있다 — 연결/해제 때마다 다시 고른다.
  useSyncExternalStore(subscribeTransportChange, getTransportRevision, getTransportRevision);

  for (const host of listHosts()) {
    const found = getTransport(host);
    if (found instanceof RunnerTransport) return found;
  }
  return null;
}

/** 서빙 on/off·포트·잠자기 방지. 폰에서 보이는 것은 전부 이 서버 하나에 달려 있다. */
function SurfaceSection({
  status,
  setStatus,
  onError,
}: {
  status: MobileSurfaceStatus | null;
  setStatus: (next: MobileSurfaceStatus) => void;
  onError: (message: string | null) => void;
}) {
  const [port, setPort] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (status) setPort(String(status.port));
  }, [status?.port]);

  const run = async (action: () => Promise<MobileSurfaceStatus>) => {
    setBusy(true);
    onError(null);
    try {
      setStatus(await action());
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (!status) return <Section title="모바일 표면">확인 중…</Section>;

  const portChanged = port.trim() !== String(status.port);

  return (
    <Section
      title="모바일 표면"
      hint="켜면 이 Mac이 폰용 PWA를 루프백에 서빙합니다. 밖으로 내려면 `tailscale serve`로 이 포트를 tailnet에 붙이세요."
    >
      <div className="flex flex-wrap gap-2 items-center mb-2">
        <span
          className={`text-xs font-medium ${status.running ? "text-status-done" : "text-text-muted"}`}
        >
          {status.running ? `● 서빙 중 · 127.0.0.1:${status.port}` : "○ 꺼짐"}
        </span>
        <div className="flex-1" />
        <ToggleBadge
          enabled={status.running}
          onToggle={() => void run(() => mobileSurfaceSetEnabled(!status.running))}
        />
      </div>
      <div className="flex flex-wrap gap-2 items-center">
        <input
          className={`${inputCls} w-28`}
          inputMode="numeric"
          placeholder="포트"
          value={port}
          onChange={(e) => setPort(e.target.value)}
        />
        <button
          className="h-9 px-3 rounded-md bg-surface border border-border text-text-secondary text-sm hover:border-border-strong disabled:opacity-40"
          disabled={busy || !portChanged || !/^\d+$/.test(port.trim())}
          onClick={() => void run(() => mobileSurfaceSetPort(Number(port.trim())))}
        >
          포트 적용
        </button>
        <div className="flex-1" />
        <label className="flex items-center gap-2 text-xs text-text-secondary">
          <input
            type="checkbox"
            checked={status.prevent_sleep}
            disabled={busy}
            onChange={(e) => void run(() => mobileSurfaceSetPreventSleep(e.target.checked))}
          />
          서빙 중 잠자기 방지
        </label>
      </div>
      {status.prevent_sleep && status.running && !status.sleep_prevented && (
        <div className="text-status-failed text-xs font-code mt-2">
          caffeinate를 띄우지 못했습니다 — Mac이 잠들면 폰에서 아무것도 보이지 않습니다.
        </div>
      )}
    </Section>
  );
}

/** 폰에서의 승인·되돌리기 허용. 읽기·대화는 이 토글과 무관하게 항상 된다. */
function ReviewCommandsSection({ onError }: { onError: (message: string | null) => void }) {
  const [enabled, setEnabled] = useState<boolean | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        setEnabled(await remoteReviewCommandsGet());
      } catch (e) {
        onError(String(e));
      }
    })();
  }, [onError]);

  const toggle = async () => {
    if (enabled === null) return;
    onError(null);
    try {
      await remoteReviewCommandsSet(!enabled);
      setEnabled(!enabled);
    } catch (e) {
      onError(String(e));
    }
  };

  return (
    <Section
      title="폰에서 승인·되돌리기 허용"
      hint="꺼져 있어도 폰에서 보고 대화하는 것은 됩니다. 원격 승인은 verify 증거와 protected paths를 항상 강제합니다."
    >
      <div className="flex items-center gap-2">
        <span className="text-xs text-text-muted flex-1">
          {enabled === null ? "확인 중…" : enabled ? "허용됨" : "차단됨 (기본값)"}
        </span>
        <ToggleBadge enabled={enabled === true} onToggle={() => void toggle()} />
      </div>
    </Section>
  );
}

/**
 * 페어링 코드 발급 + 세션 목록. 코드 원문은 발급 응답에만 존재한다.
 *
 * 두 경로가 한 자리에 있다 — 이 Mac이 직접 서빙할 때와 원격 Runner에 붙어 있을 때.
 * 전에는 후자가 설정 > 연결에 따로 있어 "폰을 붙인다"가 두 화면으로 갈려 있었다.
 */
function PairingSection({ onError }: { onError: (message: string | null) => void }) {
  const runner = useRunnerTransport();
  const [sessions, setSessions] = useState<MobileSession[]>([]);
  const [pairing, setPairing] = useState<MobilePairing | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setSessions(await mobileSessionList());
    } catch (e) {
      onError(String(e));
    }
  }, [onError]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const create = async () => {
    setBusy(true);
    onError(null);
    try {
      setPairing(await mobilePairingCreate());
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const revoke = async (id: number) => {
    onError(null);
    try {
      await mobileSessionRevoke(id);
      await refresh();
    } catch (e) {
      onError(String(e));
    }
  };

  return (
    <Section id="mobile-pairing" title="기기 페어링" hint={PAIRING_HINT}>
      <div className="text-text-secondary text-xs mb-2">이 Mac에 직접 연결</div>
      <div className="flex flex-wrap gap-2 items-center mb-2">
        <button
          className="h-9 px-3 rounded-md bg-primary text-bg text-sm font-medium disabled:bg-border disabled:text-text-muted"
          disabled={busy}
          onClick={() => void create()}
        >
          페어링 코드 발급
        </button>
        {pairing && (
          <span className="font-code text-sm text-text-secondary break-all">
            {pairing.code}
            <span className="text-text-muted text-xs ml-2">
              {new Date(pairing.expires_at * 1000).toLocaleTimeString()}까지
            </span>
          </span>
        )}
      </div>
      {sessions.length === 0 ? (
        <div className="text-text-muted text-xs">페어링된 기기가 없습니다.</div>
      ) : (
        <div className="flex flex-col gap-2">
          {sessions.map((s) => (
            <ResourceRow key={s.id}>
              <span className="font-code text-text-secondary text-xs flex-1 truncate">
                {s.label || `세션 ${s.id}`}
              </span>
              <span className="text-text-muted text-xs shrink-0">{ago(s.last_seen_at)} 전</span>
              <DeleteButton onRemove={() => void revoke(s.id)} />
            </ResourceRow>
          ))}
        </div>
      )}

      <div className="border-t border-border mt-3 pt-3">
        <div className="text-text-secondary text-xs mb-1">원격 Runner에 연결 (QR)</div>
        <div className="text-text-muted text-xs mb-2">
          폰에서 QR을 찍어 연결합니다. 원문 pairing token은 폰에 저장되지 않고, 기기별로 따로
          연결을 끊을 수 있습니다.
        </div>
        <MobilePairingCard transport={runner} />
      </div>
    </Section>
  );
}

/** 설정 > 모바일 — 데스크톱이 직접 서빙하는 PWA 표면의 운영 화면(설계 2026-09-13 P2). */
export function MobileView() {
  const [status, setStatus] = useState<MobileSurfaceStatus | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        setStatus(await mobileSurfaceStatus());
      } catch (e) {
        setErr(String(e));
      }
    })();
  }, []);

  return (
    <div className="flex-1 overflow-auto p-4">
      <div className="max-w-3xl mx-auto">
        <div className="flex items-center gap-2 mb-2">
          <span className="text-sm font-medium">모바일 PWA</span>
          <MetaTag title="실제 기기 페어링은 아직 충분히 검증되지 않았습니다">베타</MetaTag>
        </div>
        <div className="text-text-muted text-xs mb-3">
          데스크톱에서 돌린 작업을 폰에서 보고, 대화하고, 승인합니다. 에이전트가 질문하거나 검토 대기에
          들어가면 Web Push가 갑니다. 실제 기기로 페어링하는 경로는 아직 충분히 검증되지 않았고, 이
          Mac 밖에서 접속하려면 `tailscale serve`가 필요합니다.
        </div>
        {err && <div className="text-status-failed text-sm font-code mb-2">{err}</div>}
        <SurfaceSection status={status} setStatus={setStatus} onError={setErr} />
        <PairingSection onError={setErr} />
        <ReviewCommandsSection onError={setErr} />
      </div>
    </div>
  );
}
