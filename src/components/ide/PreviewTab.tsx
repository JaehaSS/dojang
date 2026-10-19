import { memo, useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Icon } from "./icons";
import {
  designmodeClose,
  designmodeOpen,
  designmodeHide,
  designmodeNavigate,
  designmodeSetBounds,
  designmodeSetSelectionMode,
  designmodeShow,
  designmodeState,
  type NativePreviewState,
  type DesignCaptureRecord,
  type PreviewMode,
} from "../../lib/ipc";
import { pushCapture } from "../../lib/designmode/store";
import {
  previewBoundsChanged,
  previewBoundsFromRect,
} from "../../lib/designmode/layout";
import type { DesignBoundingRect } from "../../lib/designmode/types";
import { enqueueDesignTransition, waitForDesignTransition } from "../../lib/design-canvas/lifecycle";
import { EditorCaptureButton } from "./EditorCaptureButton";
import { PreviewWorkbenchStrip } from "./PreviewWorkbenchStrip";
import type { PreviewWorkbenchState } from "../../lib/preview-workbench/types";

/** 마지막으로 연 URL — 탭을 닫았다 다시 열어도(같은 세션 내) 입력을 잃지 않게 기억한다. */
const lastUrlByTask = new Map<number, string>();

/** 마지막으로 고른 프리뷰 모드. 전환은 웹뷰 재생성이라 페이지 상태가 날아가므로,
 *  한 번 고른 모드를 기억해 전환 자체를 기본 경로에서 뺀다. */
const lastModeByTask = new Map<number, PreviewMode>();

interface CapturePayload {
  task_id: number;
  record: DesignCaptureRecord;
}

/** Rust `preview_control::ControlEvent`와 필드가 일치한다 — 명령 시작(`active:true`)마다 하나,
 *  마지막 명령 뒤 3초 유휴에 완료 이벤트(`active:false`)가 하나 온다. */
interface PreviewControlEvent {
  task_id: number;
  active: boolean;
  op: string;
  target: string | null;
  changed: boolean | null;
  url: string;
  controllable: boolean;
}

/** 마지막 액션 한 줄(D-7 ②) — `click button "로그인" → changed`. */
export function formatLastAction(ev: PreviewControlEvent): string {
  if (ev.op === "wait_for") return ev.target ? `wait_for "${ev.target}"` : "wait_for";
  const head = ev.target ? `${ev.op} ${ev.target}` : ev.op;
  if (ev.changed == null) return head;
  return `${head} → ${ev.changed ? "changed" : "unchanged"}`;
}

interface Props {
  taskId: number;
  active: boolean;
  editorAvailable: boolean;
  /** 사이드패널 위에 DOM 오버레이(도구 피커 등)가 열린 상태. */
  overlayOpen?: boolean;
  workbench?: PreviewWorkbenchProps;
  onPreviewChanged?: () => void;
}

interface PreviewWorkbenchProps {
  state: PreviewWorkbenchState;
  onDraftChange: (draft: string) => void;
  onSubmit: (message: string) => void;
  onCancelPending: () => void;
  onTakeOver: () => void;
  onRelease: () => void;
  onRefresh: () => void;
}

/** Design Mode 프리뷰 탭(D-1, macOS 우선·실험 기능) — 내장 웹뷰 + 요소 선택 캡처. */
export const PreviewTab = memo(function PreviewTab({
  taskId,
  active,
  editorAvailable,
  overlayOpen = false,
  workbench,
  onPreviewChanged,
}: Props) {
  const [url, setUrl] = useState(lastUrlByTask.get(taskId) ?? "http://localhost:3000");
  const [mode, setMode] = useState<PreviewMode>(lastModeByTask.get(taskId) ?? "window");
  const [loaded, setLoaded] = useState(false);
  const [selecting, setSelecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [controlling, setControlling] = useState(false);
  const [lastAction, setLastAction] = useState<string | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const boundsRef = useRef<DesignBoundingRect | null>(null);
  const nativeRevision = useRef(0);
  const onChangedRef = useRef(onPreviewChanged);
  onChangedRef.current = onPreviewChanged;
  const activeRef = useRef(active);
  activeRef.current = active;

  // 구독 후 조회해 마운트 전에 열린 창과 놓친 이벤트도 복원한다.
  useEffect(() => {
    let disposed = false;
    const refresh = async () => {
      const revision = ++nativeRevision.current;
      try {
        await waitForDesignTransition(taskId);
        if (disposed) return;
        const preview = await designmodeState(taskId);
        if (disposed || revision !== nativeRevision.current) return;
        setLoaded(preview !== null);
        if (preview) {
          setUrl(preview.url);
          setMode(preview.mode);
          lastUrlByTask.set(taskId, preview.url);
          lastModeByTask.set(taskId, preview.mode);
        }
      } catch (cause) {
        if (!disposed && revision === nativeRevision.current) setError(String(cause));
      }
    };
    const changed = () => { void refresh(); onChangedRef.current?.(); };
    const listeners = [
      listen<number>("designmode://changed", ({ payload }) => { if (payload === taskId) changed(); }),
      listen<NativePreviewState>("designmode://activated", ({ payload }) => { if (payload.taskId === taskId) changed(); }),
    ];
    void Promise.all(listeners).then(() => { if (!disposed) void refresh(); });
    return () => { disposed = true; ++nativeRevision.current; listeners.forEach((listener) => void listener.then((off) => off())); };
  }, [taskId, active]);

  const readBounds = useCallback((): DesignBoundingRect | null => {
    const container = containerRef.current;
    return container ? previewBoundsFromRect(container.getBoundingClientRect()) : null;
  }, []);

  const openPreview = async (target: string): Promise<void> => enqueueDesignTransition(taskId, async () => {
    if (!activeRef.current || !containerRef.current) return;
    const nextUrl = target.trim();
    const bounds = readBounds();
    if (!bounds || !nextUrl) return;
    ++nativeRevision.current;
    setError(null);
    setSelecting(false);
    try {
      if (loaded) {
        await designmodeNavigate(taskId, nextUrl);
      } else {
        await designmodeOpen(taskId, nextUrl, bounds, mode);
      }
      boundsRef.current = bounds;
      lastUrlByTask.set(taskId, nextUrl);
      lastModeByTask.set(taskId, mode);
      setUrl(nextUrl);
      setLoaded(true);
      onPreviewChanged?.();
    } catch (cause) {
      setError(String(cause));
    }
  });

  // 네이티브 자식 웹뷰는 z-index와 무관하게 항상 메인 웹뷰의 DOM 위에 그려진다 — 사이드패널
  // 오버레이(도구 피커 등)가 열린 동안은 숨겨야 그 아래로 잘리지 않는다.
  const showWebview = active && !overlayOpen;

  // 창 모드는 메인 창과 독립이라 탭 활성/오버레이와 무관하게 그대로 둔다 —
  // 탭을 떠났다고 창을 숨기면 "따로 띄워 크게 본다"가 성립하지 않는다.
  const managesWebview = mode === "inline";

  // Orca와 같은 persistent-tab 수명주기: 활성화는 show, 비활성화는 hide, navigate는 사용자 동작만.
  // 창 모드에서는 hide하지 않고 존재 여부만 조회한다 — 창을 띄워둔 채 다른 작업을 갔다 왔을 때
  // 탭이 "URL을 입력하세요"로 보이면 안 된다(네이티브 show는 창 모드에서 no-op이다).
  useEffect(() => {
    let cancelled = false;
    if (managesWebview && !showWebview) {
      void enqueueDesignTransition(taskId, () => designmodeHide(taskId)).catch(() => {});
      return;
    }
    if (!managesWebview && !active) return;
    const bounds = readBounds();
    if (!bounds) return;
    boundsRef.current = bounds;
    void enqueueDesignTransition(taskId, async () => { if (!cancelled) await designmodeShow(taskId, bounds); })
      .catch((cause) => {
        if (!cancelled) setError(String(cause));
      });
    return () => {
      cancelled = true;
    };
  }, [managesWebview, showWebview, active, readBounds, taskId]);

  // 작업 전환으로 컴포넌트가 사라져도 WebView는 닫지 않고 숨긴다. 명시적 탭 닫기만 파괴한다.
  useEffect(() => {
    if (!managesWebview) return;
    return () => { void enqueueDesignTransition(taskId, () => designmodeHide(taskId)).catch(() => {}); };
  }, [managesWebview, taskId]);

  // 리사이즈/스크롤 burst를 한 프레임으로 합치고 동일 bounds는 네이티브 IPC로 보내지 않는다.
  // 사이드패널은 고정 폭이라 메인 창 리사이즈로 위치만 이동해도 크기는 안 바뀌어 RO가 미발화한다 —
  // window resize와 조상 스크롤(capture phase)을 같은 debounce 경로에 태워 stale 좌표를 방지한다.
  useEffect(() => {
    const container = containerRef.current;
    // 창 모드에서는 네이티브가 no-op이지만, 매 프레임 무의미한 IPC를 쏘지 않도록 여기서 끊는다.
    if (!container || !managesWebview || !showWebview || !loaded) return;
    let timer: number | undefined;
    let disposed = false;
    const sync = () => {
      const bounds = previewBoundsFromRect(container.getBoundingClientRect());
      if (!bounds || !previewBoundsChanged(boundsRef.current, bounds)) return;
      boundsRef.current = bounds;
      void enqueueDesignTransition(taskId, async () => {
        if (!disposed) await designmodeSetBounds(taskId, bounds);
      }).catch(() => {});
    };
    const scheduleSync = () => {
      if (timer != null) return;
      timer = window.setTimeout(() => {
        timer = undefined;
        sync();
      }, 16);
    };
    const ro = new ResizeObserver(scheduleSync);
    ro.observe(container);
    window.addEventListener("resize", scheduleSync);
    window.addEventListener("scroll", scheduleSync, true);
    sync();
    return () => {
      disposed = true;
      if (timer != null) clearTimeout(timer);
      ro.disconnect();
      window.removeEventListener("resize", scheduleSync);
      window.removeEventListener("scroll", scheduleSync, true);
    };
  }, [managesWebview, showWebview, loaded, taskId]);

  // 사용자가 프리뷰 창을 닫으면 웹뷰가 파괴된다 — 프론트도 빈 상태로 되돌려야
  // 다음 Enter가 navigate가 아니라 open으로 간다.
  useEffect(() => {
    const un = listen<number>("designmode://closed", (e) => {
      if (e.payload !== taskId) return;
      ++nativeRevision.current;
      setLoaded(false);
      setSelecting(false);
      setControlling(false);
      setLastAction(null);
    });
    return () => {
      un.then((f) => f());
    };
  }, [taskId]);

  // 에이전트가 MCP로 페이지를 조작하는 동안의 가시화(D-7 ①②) — 액션은 100ms 안에 끝나므로
  // 배지만으로는 "무엇을" 했는지 남지 않는다. 마지막 도착 이벤트로 한 줄을 갱신한다.
  useEffect(() => {
    const un = listen<PreviewControlEvent>("designmode://control", (e) => {
      if (e.payload.task_id !== taskId) return;
      setControlling(e.payload.active);
      setLastAction(
        formatLastAction(e.payload) +
          (e.payload.controllable === false ? " · 제어 불가 origin" : ""),
      );
    });
    return () => {
      un.then((f) => f());
    };
  }, [taskId]);

  // 캡처 도착 이벤트 — 이 태스크의 캡처만 Composer 칩 스토어에 적재한다.
  useEffect(() => {
    const un = listen<CapturePayload>("designmode://capture", (e) => {
      if (e.payload.task_id !== taskId) return;
      pushCapture(taskId, e.payload.record);
      // inject.js가 캡처 직후 선택 모드를 스스로 내린다(스크린샷까지 하이라이트를 고정하려고).
      // 버튼 상태를 실제 웹뷰 상태에 맞춘다 — 개발자도구 요소 선택기와 같은 1회성 동작이다.
      setSelecting(false);
    });
    return () => {
      un.then((f) => f());
    };
  }, [taskId]);

  // 모드 전환은 웹뷰 재생성이라 페이지 상태가 날아간다(Tauri에 reparent API가 없다).
  // 이미 떠 있는 프리뷰가 있으면 그 사실을 알리고, 확인받은 뒤에만 바꾼다.
  const toggleMode = async (): Promise<void> => enqueueDesignTransition(taskId, async () => {
    if (!activeRef.current || !containerRef.current) return;
    const next: PreviewMode = mode === "window" ? "inline" : "window";
    if (loaded && !window.confirm("프리뷰를 다시 불러옵니다. 페이지에 입력하던 내용은 사라집니다.")) {
      return;
    }
    // 남은 프리뷰는 `loaded`와 무관하게 없앤다. 프론트가 "열려 있지 않다"고 믿는 동안에도
    // 네이티브에는 창이나 웹뷰가 남아 있을 수 있고(닫힘 이벤트 유실·닫기 실패), 그 하나가
    // 조작면 위에 눌러앉으면 URL 입력줄도 선택 버튼도 누를 수 없게 된다.
    // 닫힘을 **기다린 뒤에** 모드를 바꾼다 — 새 프리뷰가 먼저 열리면 둘이 겹친다.
    ++nativeRevision.current;
    try {
      await designmodeClose(taskId);
    } catch (cause) {
      // 못 없앴으면 모드도 바꾸지 않는다. 유령을 남긴 채 새로 여느니 지금 상태로 둔다.
      setError(String(cause));
      return;
    }
    setError(null);
    setLoaded(false);
    setSelecting(false);
    lastModeByTask.set(taskId, next);
    setMode(next);
  });

  const toggleSelecting = () => {
    const next = !selecting;
    setSelecting(next);
    void enqueueDesignTransition(taskId, async () => {
      if (activeRef.current && containerRef.current) await designmodeSetSelectionMode(taskId, next);
    }).catch((e) => setError(String(e)));
  };

  return (
    <div
      className="flex-1 min-h-0 flex-col bg-bg"
      style={{ display: active ? "flex" : "none" }}
      aria-hidden={!active}
    >
      <div className="flex items-center gap-2 border-b border-border px-2 py-1.5 shrink-0">
        <input
          className="flex-1 bg-surface border border-border rounded px-2 py-1 text-xs text-text outline-none focus:border-primary"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void openPreview(url);
          }}
          placeholder="http://localhost:3000"
          aria-label="프리뷰 URL"
        />
        <button
          className="text-text-muted hover:text-text shrink-0"
          onClick={() => void openPreview(url)}
          title="새로고침"
          aria-label="새로고침"
        >
          <Icon name="refresh" size={15} />
        </button>
        <button
          className={`shrink-0 px-2 py-1 rounded text-xs border ${
            selecting ? "border-primary text-primary-bright bg-primary/10" : "border-border text-text-muted"
          }`}
          onClick={toggleSelecting}
          disabled={!loaded}
          title="요소 선택 모드"
          aria-pressed={selecting}
        >
          <Icon name="code" size={13} /> 선택
        </button>
        <button
          className={`shrink-0 px-2 py-1 rounded text-xs border ${
            mode === "window"
              ? "border-primary text-primary-bright bg-primary/10"
              : "border-border text-text-muted"
          }`}
          onClick={() => void toggleMode()}
          title={
            loaded
              ? "모드를 바꾸면 페이지를 다시 불러옵니다 — 입력하던 내용이 사라집니다"
              : "별도 창으로 열기"
          }
          aria-label="별도 창"
          aria-pressed={mode === "window"}
        >
          <Icon name="desktop" size={13} /> 창
        </button>
        <EditorCaptureButton taskId={taskId} available={editorAvailable} onError={setError} />
        {controlling && (
          <span
            className="shrink-0 px-2 py-1 rounded text-xs border border-primary text-primary-bright bg-primary/10"
            role="status"
            aria-live="polite"
          >
            에이전트 제어 중
          </span>
        )}
      </div>
      {error && (
        <div className="px-3 py-1 text-xs text-status-failed border-b border-border">{error}</div>
      )}
      {lastAction && !workbench && (
        <div
          className="px-3 py-1 text-xs text-text-muted border-b border-border truncate"
          data-testid="preview-last-action"
        >
          {lastAction}
        </div>
      )}
      {workbench && (
        <PreviewWorkbenchStrip
          {...workbench}
          state={{ ...workbench.state, lastAction: workbench.state.lastAction ?? lastAction }}
        />
      )}
      <div ref={containerRef} className="flex-1 min-h-0 relative">
        {!loaded && (
          <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 text-text-muted text-sm">
            <Icon name="desktop" size={22} />
            <span>dev 서버 URL을 입력하고 Enter — 실험 기능(macOS 우선)</span>
          </div>
        )}
        {loaded && mode === "window" && (
          <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 text-text-muted text-sm">
            <Icon name="desktop" size={22} />
            <span>프리뷰 창이 열려 있습니다</span>
          </div>
        )}
      </div>
    </div>
  );
});
