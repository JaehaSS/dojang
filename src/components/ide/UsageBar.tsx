import { useCallback, useEffect, useRef, useState } from "react";
import {
  usageBridgeInstall,
  usageBridgeStatus,
  usageSnapshot,
  type BridgeStatus,
  type UsageSnapshot,
  type UsageWindow,
  type VendorUsage,
} from "../../lib/ipc";
import {
  fmtAge,
  fmtReset,
  gaugeColor,
  gaugeTextClass,
  remaining,
  sourceLabel,
  summarize,
  windowLabel,
  worstRemaining,
} from "../../lib/usage";
import { ago } from "../../lib/fmt";
import { Icon } from "./icons";

/** 스냅샷 폴링 간격(ms). 백엔드가 OAuth 재조회를 자체 캐시로 눌러 준다. */
const POLL_MS = 60_000;

/** 남은 비율 게이지 — 상태바용 미니(w-10)와 팝오버용 풀(w-full) 두 가지. */
function Gauge({ percent, wide }: { percent: number; wide?: boolean }) {
  return (
    <div className={`${wide ? "w-full h-1.5" : "w-10 h-1"} rounded-full bg-raised overflow-hidden`}>
      <div
        className="h-full rounded-full"
        style={{ width: `${percent}%`, background: gaugeColor(percent) }}
      />
    </div>
  );
}

/** 상태바에 한 벤더를 나타내는 칩 — 클릭하면 상세 팝오버를 연다. */
export function VendorChip({
  usage,
  active,
  onClick,
}: {
  usage: VendorUsage;
  active: boolean;
  onClick: () => void;
}) {
  const { text, tone } = summarize(usage);
  const worst = worstRemaining(usage);
  return (
    <button
      className={`h-6 px-2 rounded flex items-center gap-1.5 hover:bg-raised ${active ? "bg-raised" : ""}`}
      onClick={onClick}
      title={usage.detail ?? `${usage.label} 사용 한도`}
      aria-expanded={active}
    >
      <span className="text-text-muted">{usage.label}</span>
      {worst != null && <Gauge percent={worst} />}
      <span className={tone === "value" ? gaugeTextClass(worst ?? 100) : "text-text-muted"}>
        {text}
      </span>
    </button>
  );
}

/** 팝오버 안 윈도 한 줄 — 라벨 + 남은 비율 + 리셋 시각. */
function WindowRow({
  title,
  window: w,
  now,
  muted,
}: {
  title: string;
  window: UsageWindow;
  now: number;
  /** 값이 낡았다 — 게이지 색은 남기고 숫자만 죽인다. */
  muted?: boolean;
}) {
  const left = remaining(w);
  const reset = fmtReset(w.resets_at, now);
  return (
    <div className="mb-2 last:mb-0">
      <div className="flex items-baseline justify-between mb-1">
        <span className="text-text-secondary">{windowLabel(w.window_minutes, title)}</span>
        <span className={muted ? "text-text-muted" : gaugeTextClass(left)}>
          {Math.round(left)}% 남음
          {reset && <span className="text-text-muted"> · {reset}</span>}
        </span>
      </div>
      <Gauge percent={left} wide />
    </div>
  );
}

/** Claude 전용 안내 — 잔량이 statusline JSON으로만 나오므로 브리지 설치 동선을 준다. 설치된 뒤에는 아무것도 내지 않는다. */
function BridgeControl({
  bridge,
  busy,
  onInstall,
}: {
  bridge: BridgeStatus | null;
  busy: boolean;
  onInstall: () => void;
}) {
  if (!bridge || bridge.installed) return null;
  return (
    <div className="mt-2 pt-2 border-t border-border text-xs text-text-muted">
      <p className="mb-1">
        Claude는 잔량을 statusline으로만 알려줍니다. 브리지를 설치하면 대화 중 자동으로 값이 채워집니다.
      </p>
      {bridge.foreign && <p className="mb-1">기존 statusline({bridge.foreign})은 그대로 실행됩니다.</p>}
      <button
        className="h-6 px-2 rounded bg-raised text-text-secondary hover:text-text disabled:opacity-40"
        disabled={busy}
        onClick={onInstall}
      >
        브리지 설치
      </button>
    </div>
  );
}

/** 벤더 상세 팝오버 본문. */
export function VendorDetail({
  usage,
  now,
  bridge,
  busy,
  onInstall,
}: {
  usage: VendorUsage;
  now: number;
  bridge: BridgeStatus | null;
  busy: boolean;
  onInstall: () => void;
}) {
  const source = sourceLabel(usage.source);
  const stale = usage.status === "stale";
  return (
    <div>
      <div className="flex items-baseline justify-between mb-2">
        <span className="text-text">{usage.label}</span>
        {usage.plan && <span className="text-xs text-text-muted">{usage.plan}</span>}
      </div>

      {!!usage.quota_groups?.length &&
        usage.quota_groups.map((group, index) => (
          <div key={index} className="mb-3 last:mb-0">
            <p className="mb-1 text-xs text-text-secondary">{group.label}</p>
            {group.five_hour && (
              <WindowRow title="5시간" window={group.five_hour} now={now} muted={stale} />
            )}
            {group.weekly && <WindowRow title="주간" window={group.weekly} now={now} muted={stale} />}
          </div>
        ))}
      {!usage.quota_groups?.length && usage.five_hour && (
        <WindowRow title="5시간" window={usage.five_hour} now={now} muted={stale} />
      )}
      {!usage.quota_groups?.length && usage.weekly && (
        <WindowRow title="주간" window={usage.weekly} now={now} muted={stale} />
      )}
      {!usage.quota_groups?.length && !usage.five_hour && !usage.weekly && (
        <p className="text-xs text-text-muted">{usage.detail ?? "표시할 한도 정보가 없습니다."}</p>
      )}
      {stale && (
        // 만료는 로그아웃이 아니다 — 값은 그대로 두고 언제 관측한 값인지만 분명히 한다.
        <p className="mt-1 text-xs text-text-muted">
          {usage.updated_at != null && `${fmtAge(Math.max(0, now - usage.updated_at))} 관측 · `}
          {usage.detail ?? "새 값을 받지 못해 마지막 관측을 보여줍니다."}
        </p>
      )}
      {usage.status === "unauthenticated" && usage.vendor !== "agy" && (
        // 여기서 바로 로그인시키지 않는 이유: 로그인은 대화형 PTY라 상태바 팝오버에 담기지
        // 않는다. 어디로 가야 하는지만 확실히 알려준다.
        <p className="mt-1 text-xs text-status-awaiting">
          설정 → 에이전트 CLI에서 로그인할 수 있습니다. 설정에서 조회용 토큰을 등록할 수도
          있습니다.
        </p>
      )}

      {(source || usage.updated_at) && (
        <div className="mt-2 text-xs text-text-muted">
          {source}
          {usage.updated_at != null && `${source ? " · " : ""}${ago(usage.updated_at)} 전 관측`}
        </div>
      )}

      {usage.vendor === "claude" && (
        <BridgeControl bridge={bridge} busy={busy} onInstall={onInstall} />
      )}
    </div>
  );
}

/** 에디터 최하단 사용량 상태바 — 벤더별 잔량을 한 줄로 보여주고, 칩을 누르면 상세를 띄운다. */
export function UsageBar() {
  const [snap, setSnap] = useState<UsageSnapshot | null>(null);
  const [openVendor, setOpenVendor] = useState<string | null>(null);
  const [bridge, setBridge] = useState<BridgeStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  const rootRef = useRef<HTMLDivElement>(null);

  const load = useCallback(async (force = false) => {
    try {
      setSnap(await usageSnapshot(force));
    } catch {
      // 상태바는 실패해도 조용히 유지한다 — 다음 폴링에서 회복.
    }
    setNow(Math.floor(Date.now() / 1000));
  }, []);

  useEffect(() => {
    void load();
    const id = window.setInterval(() => void load(), POLL_MS);
    return () => window.clearInterval(id);
  }, [load]);

  // 팝오버를 열 때만 브리지 상태를 확인한다(설정 파일 IO를 폴링에 얹지 않는다).
  useEffect(() => {
    if (openVendor !== "claude") return;
    void usageBridgeStatus()
      .then(setBridge)
      .catch(() => setBridge(null));
  }, [openVendor]);

  // 바깥 클릭 / ESC로 닫기.
  useEffect(() => {
    if (openVendor == null) return;
    const onDown = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpenVendor(null);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpenVendor(null);
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [openVendor]);

  const runBridge = async (action: () => Promise<BridgeStatus>) => {
    setBusy(true);
    try {
      setBridge(await action());
      await load(true);
    } catch {
      // 실패해도 상태만 유지 — 사용자가 다시 시도할 수 있다.
    }
    setBusy(false);
  };

  if (!snap) return null;
  const opened = snap.vendors.find((v) => v.vendor === openVendor) ?? null;

  return (
    <div
      ref={rootRef}
      className="relative border-t border-border bg-surface px-3 h-7 shrink-0 flex items-center gap-1 text-xs"
    >
      {snap.vendors.map((v) => (
        <VendorChip
          key={v.vendor}
          usage={v}
          active={openVendor === v.vendor}
          onClick={() => setOpenVendor((cur) => (cur === v.vendor ? null : v.vendor))}
        />
      ))}

      <button
        className="ml-auto h-6 px-1.5 rounded text-text-muted hover:text-text hover:bg-raised"
        onClick={() => void load(true)}
        title="사용량 새로고침"
        aria-label="사용량 새로고침"
      >
        <Icon name="refresh" size={12} />
      </button>

      {opened && (
        <div className="absolute left-3 bottom-full mb-1 z-30 w-72 rounded-lg border border-border-strong bg-raised shadow-xl p-3">
          <VendorDetail
            usage={opened}
            now={now}
            bridge={bridge}
            busy={busy}
            onInstall={() => void runBridge(usageBridgeInstall)}
          />
        </div>
      )}
    </div>
  );
}
