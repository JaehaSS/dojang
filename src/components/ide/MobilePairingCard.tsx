import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import qrcode from "qrcode-generator";
import type { MobilePairing, MobileSession } from "../../lib/ipc";
import type { RunnerTransport } from "../../lib/transport/runner";
import { ago } from "../../lib/fmt";
import { Icon } from "./icons";

interface Props {
  /** 작업 소스인 Runner. 원격에 붙어 있지 않으면 null — 모바일 세션은 Runner에만 있다. */
  transport: RunnerTransport | null;
}

/** 폰이 접속할 주소는 Runner가 알 수 없다(loopback에 바인딩되어 tailnet 이름을 모른다).
 * 사용자가 한 번 입력하면 기억한다. */
const BASE_URL_KEY = "praxis-mobile-base-url";

/** QR 한 변의 픽셀 크기. 폰 카메라가 30cm에서 무리 없이 읽는 정도. */
const QR_SIZE = 168;

function loadBaseUrl(): string {
  try {
    return localStorage.getItem(BASE_URL_KEY) ?? "";
  } catch {
    return "";
  }
}

/** 입력을 페어링 URL로 정규화한다. 스킴 누락·끝 슬래시·`/m` 중복을 흡수한다. */
export function pairingUrl(baseUrl: string, code: string): string | null {
  const trimmed = baseUrl.trim();
  if (!trimmed || !code) return null;
  const withScheme = /^https?:\/\//i.test(trimmed) ? trimmed : `https://${trimmed}`;
  let origin: string;
  try {
    origin = new URL(withScheme).origin;
  } catch {
    return null;
  }
  return `${origin}/m/#pair=${code}`;
}

/** QR 모듈을 행 단위로 병합한 사각형 목록으로 바꾼다 — 모듈마다 <rect>를 찍으면
 * 800개가 넘어 DOM이 불필요하게 커진다. */
function qrRects(text: string): { count: number; rects: { x: number; y: number; w: number }[] } {
  const qr = qrcode(0, "M");
  qr.addData(text);
  qr.make();
  const count = qr.getModuleCount();
  const rects: { x: number; y: number; w: number }[] = [];
  for (let y = 0; y < count; y += 1) {
    let runStart = -1;
    for (let x = 0; x <= count; x += 1) {
      const dark = x < count && qr.isDark(y, x);
      if (dark && runStart < 0) runStart = x;
      if (!dark && runStart >= 0) {
        rects.push({ x: runStart, y, w: x - runStart });
        runStart = -1;
      }
    }
  }
  return { count, rects };
}

function QrCode({ text }: { text: string }) {
  const { count, rects } = useMemo(() => qrRects(text), [text]);
  // quiet zone 4모듈은 스펙 권장값 — 없으면 스캐너가 경계를 못 잡는 경우가 있다.
  const quiet = 4;
  const span = count + quiet * 2;
  return (
    <svg
      width={QR_SIZE}
      height={QR_SIZE}
      viewBox={`0 0 ${span} ${span}`}
      shapeRendering="crispEdges"
      role="img"
      aria-label="모바일 페어링 QR 코드"
    >
      <rect width={span} height={span} fill="#ffffff" />
      {rects.map((rect, index) => (
        <rect
          key={index}
          x={rect.x + quiet}
          y={rect.y + quiet}
          width={rect.w}
          height={1}
          fill="#000000"
        />
      ))}
    </svg>
  );
}

/** 모바일 기기 페어링 — QR 발급과 연결된 기기 관리 (설계 0013 §6.1).
 *
 * 원문 pairing token은 폰에 주지 않는다. 일회용 코드를 QR로 건네고, 폰이 그것을 회수 가능한
 * 세션 쿠키로 교환한다. 그래서 폰을 잃어버려도 그 기기만 끊으면 된다. */
export function MobilePairingCard({ transport }: Props) {
  const [baseUrl, setBaseUrl] = useState(loadBaseUrl);
  const [pairing, setPairing] = useState<MobilePairing | null>(null);
  const [sessions, setSessions] = useState<MobileSession[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  /** 이 코드로 기기가 붙었는지 — 붙었으면 QR을 내리고 재발급을 안내한다. */
  const [consumed, setConsumed] = useState(false);
  /** 코드 발급 시점의 기기 수. 소비 판정의 기준선이다. */
  const sessionCountRef = useRef(0);

  const refreshSessions = useCallback(async () => {
    if (!transport) {
      setSessions([]);
      return [] as MobileSession[];
    }
    try {
      const list = await transport.mobileSessions();
      setSessions(list);
      return list;
    } catch {
      setSessions([]);
      return [] as MobileSession[];
    }
  }, [transport]);

  useEffect(() => void refreshSessions(), [refreshSessions]);

  // 코드가 살아 있는 동안만 초를 센다 — 만료 후에는 다시 그릴 이유가 없다.
  useEffect(() => {
    if (!pairing) return;
    const timer = window.setInterval(() => setNow(Math.floor(Date.now() / 1000)), 1000);
    return () => clearInterval(timer);
  }, [pairing]);

  // 코드는 한 번 쓰면 끝인데 서버가 소비 사실을 알려주지 않는다. 기기가 하나 늘면 그게
  // 곧 소비 신호다 — QR을 계속 띄워두면 이미 죽은 코드를 다시 찍게 된다.
  useEffect(() => {
    if (!pairing || consumed) return;
    const before = sessionCountRef.current;
    const timer = window.setInterval(() => {
      void refreshSessions().then((list) => {
        if (list.length > before) setConsumed(true);
      });
    }, 3000);
    return () => clearInterval(timer);
  }, [pairing, consumed, refreshSessions]);

  const remaining = pairing ? pairing.expires_at - now : 0;
  const expired = pairing != null && remaining <= 0;
  const live = pairing != null && !expired && !consumed;
  const url = live ? pairingUrl(baseUrl, pairing.code) : null;

  const issue = async () => {
    if (!transport) return;
    setBusy(true);
    setError(null);
    try {
      // 지금 기기 수를 기준선으로 잡아둔다 — 이보다 늘면 이 코드가 쓰인 것이다.
      const current = await refreshSessions();
      sessionCountRef.current = current.length;
      const issued = await transport.mobilePairingCreate();
      setNow(Math.floor(Date.now() / 1000));
      setConsumed(false);
      setPairing(issued);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const revoke = async (id: number) => {
    if (!transport) return;
    setBusy(true);
    setError(null);
    try {
      await transport.mobileSessionRevoke(id);
      void refreshSessions();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const commitBaseUrl = (value: string) => {
    setBaseUrl(value);
    try {
      localStorage.setItem(BASE_URL_KEY, value);
    } catch {
      /* 저장 실패는 이번 세션에만 영향 — 입력값은 화면에 남아 있다 */
    }
  };

  if (!transport) {
    return (
      <div className="rounded-lg border border-border bg-surface p-3 text-xs text-text-muted">
        모바일 기기는 원격 Runner에 연결된 상태에서만 페어링할 수 있습니다.
      </div>
    );
  }

  return (
    <div className="rounded-lg border border-border bg-surface p-3 space-y-3">
      <label className="block">
        <span className="mb-1 flex items-baseline gap-2">
          <span className="text-xs text-text-secondary">모바일 접속 주소</span>
          <span className="text-[11px] text-text-muted">tailnet HTTPS 주소</span>
        </span>
        <input
          aria-label="Mobile base URL"
          className="w-full bg-bg border border-border rounded px-2 py-1.5 text-sm text-text outline-none focus:border-primary font-code"
          placeholder="my-mac.example.ts.net"
          value={baseUrl}
          onChange={(event) => commitBaseUrl(event.target.value)}
        />
      </label>

      <div className="flex items-center gap-2">
        <button
          className="h-8 px-3 rounded-md border border-border text-text-secondary hover:border-border-strong disabled:opacity-50"
          disabled={busy}
          onClick={() => void issue()}
        >
          {/* 코드는 일회용이라 언제든 다시 만들 수 있어야 한다 — 버튼을 상황과 무관하게 남긴다. */}
          {busy ? "발급 중…" : pairing ? "QR 다시 만들기" : "기기 추가"}
        </button>
        {pairing && (
          <span
            className={`text-xs ${
              consumed
                ? "text-status-done"
                : expired
                  ? "text-status-failed"
                  : "text-text-muted"
            }`}
          >
            {consumed
              ? "기기가 연결되었습니다 — 이 코드는 사용되었습니다"
              : expired
                ? "코드가 만료되었습니다"
                : `${Math.floor(remaining / 60)}:${String(remaining % 60).padStart(2, "0")} 남음`}
          </span>
        )}
      </div>

      {pairing && !live && (
        <div className="rounded border border-dashed border-border px-3 py-2 text-xs text-text-muted">
          {consumed
            ? "다른 기기를 더 연결하려면 QR을 다시 만드세요. 코드 하나에 기기 하나입니다."
            : "코드는 5분간만 유효합니다. QR을 다시 만들어 스캔하세요."}
        </div>
      )}

      {live && (
        <div className="flex gap-3">
          {url ? (
            <div className="shrink-0 rounded bg-white p-2">
              <QrCode text={url} />
            </div>
          ) : (
            <div className="flex w-[168px] shrink-0 items-center justify-center rounded border border-dashed border-border p-2 text-center text-[11px] text-text-muted">
              주소를 입력하면 QR이 나타납니다
            </div>
          )}
          <div className="min-w-0 flex-1 space-y-2">
            <div className="text-xs text-text-secondary">
              폰 카메라로 QR을 찍으면 바로 연결됩니다. 스캔이 어려우면 폰 브라우저에서 위 주소의{" "}
              <span className="font-code">/m/</span>을 연 뒤 아래 코드를 입력하세요.
            </div>
            {/* 수동 입력 대비 — QR을 못 쓰는 상황에서 유일한 경로다. */}
            <div className="break-all rounded border border-border bg-bg px-2 py-1.5 text-[11px] font-code text-text-muted">
              {pairing.code}
            </div>
            <button
              className="text-xs text-text-muted hover:text-text"
              onClick={() => void navigator.clipboard?.writeText(pairing.code)}
            >
              코드 복사
            </button>
          </div>
        </div>
      )}

      <div>
        <div className="mb-1 flex items-center justify-between">
          <span className="text-xs text-text-secondary">연결된 기기</span>
          <button
            className="text-text-muted hover:text-text"
            onClick={() => void refreshSessions()}
            aria-label="기기 목록 새로고침"
            title="새로고침"
          >
            <Icon name="refresh" size={13} />
          </button>
        </div>
        {sessions.length === 0 ? (
          <div className="text-xs text-text-muted">아직 연결된 기기가 없습니다.</div>
        ) : (
          <div className="space-y-1">
            {sessions.map((session) => (
              <div
                key={session.id}
                className="flex items-center gap-2 rounded border border-border px-2 py-1.5"
              >
                <span className="min-w-0 flex-1 truncate text-sm text-text-secondary">
                  {session.label || `기기 #${session.id}`}
                </span>
                <span className="shrink-0 text-[11px] font-code text-text-muted">
                  {ago(session.last_seen_at)} 전
                </span>
                <button
                  className="shrink-0 text-xs text-text-muted hover:text-status-failed disabled:opacity-50"
                  disabled={busy}
                  onClick={() => void revoke(session.id)}
                  title="이 기기의 연결을 끊습니다"
                >
                  연결 끊기
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

      {error && <div className="text-xs text-status-failed">{error}</div>}
    </div>
  );
}
