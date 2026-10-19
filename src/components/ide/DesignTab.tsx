import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  designCanvasStart, designCanvasStatus, designCanvasStop,
  designCanvasEdit,
  designmodeClose, designmodeOpen, designmodeHide, designmodeSetBounds,
  type DesignCanvasServerStatus,
} from "../../lib/ipc";
import { enqueueDesignTransition } from "../../lib/design-canvas/lifecycle";
import { previewBoundsChanged, previewBoundsFromRect } from "../../lib/designmode/layout";
import type { DesignBoundingRect } from "../../lib/designmode/types";
import { isDesignSelection, type DesignSelection, type DesignSelectionEvent } from "../../lib/design-canvas/selection";
import { isDesignEditRequest, type DesignEditEvent } from "../../lib/design-canvas/edit";

const starting: DesignCanvasServerStatus = { status: "server-starting", url: null, port: null, error: null, logPath: null };

/** Mounted only while Design is visible. The page owns the three canvas columns. */
export function DesignTab({ taskId }: { taskId: number }) {
  const [server, setServer] = useState(starting);
  const [attempt, setAttempt] = useState(0);
  const [loaded, setLoaded] = useState(false);
  const [selection, setSelection] = useState<DesignSelection | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const boundsRef = useRef<DesignBoundingRect | null>(null);

  useEffect(() => {
    let disposed = false;
    const listener = listen<DesignEditEvent>("design-canvas://edit-request", ({ payload }) => {
      if (disposed || payload?.taskId !== taskId || typeof payload.surface !== "string" || !isDesignEditRequest(payload.request)) return;
      void designCanvasEdit(taskId, payload.surface, payload.request).catch((cause) => console.error("[dojang:design-edit]", cause));
    });
    void listener.catch((cause) => console.error("[dojang:design-edit-listener]", cause));
    return () => { disposed = true; void listener.then((off) => off()).catch(() => {}); };
  }, [taskId, attempt]);

  // Keep the app-side validated snapshot for the next unit's Rust edit requests.
  useEffect(() => {
    let disposed = false;
    setSelection(null);
    const listener = listen<DesignSelectionEvent>("design-canvas://selection", ({ payload }) => {
      if (!disposed && payload?.taskId === taskId && isDesignSelection(payload.selection)) setSelection(payload.selection);
    });
    void listener.catch((cause) => console.error("[dojang:design-selection-listener]", cause));
    return () => { disposed = true; void listener.then((off) => off()).catch(() => {}); };
  }, [taskId, attempt]);

  useEffect(() => {
    let disposed = false;
    const owner = crypto.randomUUID();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let wake: (() => void) | undefined;
    const pause = () => new Promise<void>((resolve) => { wake = resolve; timer = setTimeout(resolve, 300); });
    const fail = (cause: unknown) => { if (!disposed) { setLoaded(false); setServer((current) => ({ ...current, status: "server-failed", error: String(cause) })); } };
    setServer(starting);
    setLoaded(false);
    void enqueueDesignTransition(taskId, async () => {
      if (disposed) return;
      // Close the prior Preview surface before borrowing its native slot.
      await designmodeClose(taskId);
      if (disposed) return;
      let current = await designCanvasStart(taskId, owner);
      while (!disposed && current.status === "server-starting") {
        setServer(current);
        await pause();
        if (disposed) return;
        current = await designCanvasStatus(taskId, owner);
      }
      if (disposed) return;
      setServer(current);
      if (current.status !== "ready" || !current.url) return;
      const bounds = containerRef.current && previewBoundsFromRect(containerRef.current.getBoundingClientRect());
      if (!bounds) throw new Error("캔버스 표시 영역을 확인할 수 없습니다.");
      await designmodeOpen(taskId, current.url, bounds, "inline");
      if (disposed) return;
      boundsRef.current = bounds;
      setLoaded(true);
      // Keep failures visible even after the native view covers the placeholder.
      const monitor = async () => {
        if (disposed) return;
        try {
          const status = await designCanvasStatus(taskId, owner);
          if (disposed) return;
          if (status.status !== "ready") {
            await enqueueDesignTransition(taskId, async () => {
              if (disposed) return;
              await designmodeClose(taskId);
              if (!disposed) { setLoaded(false); setServer(status); }
            });
            return;
          }
          timer = setTimeout(() => void monitor(), 1000);
        } catch (cause) {
          await enqueueDesignTransition(taskId, async () => {
            if (!disposed) { await designmodeClose(taskId).catch(() => {}); fail(cause); }
          });
        }
      };
      timer = setTimeout(() => void monitor(), 1000);
    }).catch(fail);
    return () => {
      disposed = true;
      clearTimeout(timer);
      wake?.();
      // Queue cleanup before Preview or a later Design mount can reuse the slot.
      void enqueueDesignTransition(taskId, async () => {
        try { await designmodeHide(taskId); await designmodeClose(taskId); }
        finally { await designCanvasStop(taskId, owner); }
      }).catch((cause) => console.error("[dojang:design-canvas-cleanup]", cause));
    };
  }, [taskId, attempt]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container || !loaded) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let disposed = false;
    const sync = () => {
      const bounds = previewBoundsFromRect(container.getBoundingClientRect());
      if (!bounds || !previewBoundsChanged(boundsRef.current, bounds)) return;
      boundsRef.current = bounds;
      void enqueueDesignTransition(taskId, async () => {
        if (!disposed) await designmodeSetBounds(taskId, bounds);
      }).catch(() => {});
    };
    const schedule = () => { clearTimeout(timer); timer = setTimeout(sync, 16); };
    const observer = new ResizeObserver(schedule);
    observer.observe(container);
    window.addEventListener("resize", schedule);
    window.addEventListener("scroll", schedule, true);
    sync();
    return () => { disposed = true; clearTimeout(timer); observer.disconnect(); window.removeEventListener("resize", schedule); window.removeEventListener("scroll", schedule, true); };
  }, [taskId, loaded]);

  return <div ref={containerRef} className="relative min-h-0 flex-1 bg-bg" aria-label="Design 워크벤치" data-design-source={selection?.source ?? undefined} data-design-class-kind={selection?.classKind}>
    {!loaded && <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 overflow-auto p-4 text-sm text-text-secondary">
      <p role={server.status === "server-failed" ? "alert" : "status"}>
        {server.status === "server-failed" ? server.error ?? "개발 서버를 시작하지 못했습니다." : `개발 서버 시작 중${server.port ? ` · 포트 ${server.port}` : ""}`}
      </p>
      {server.logPath && <p className="break-all font-code text-xs">로그: {server.logPath}</p>}
      {server.status === "server-failed" && <button className="rounded border border-border px-3 py-1 text-text hover:border-primary" onClick={() => setAttempt((value) => value + 1)}>다시 시작</button>}
    </div>}
  </div>;
}
