import { isDesignEditRequest, isDesignEditResult, type DesignEditRequest, type DesignEditResult } from "../lib/design-canvas/edit";

export function canEditCanvas() { return !!(window as Window & { __dojangCanvasNative?: boolean }).__dojangCanvasNative; }

export function requestDesignEdit(request: DesignEditRequest, signal?: AbortSignal): Promise<DesignEditResult> {
  const requestId = request.requestId;
  if (!canEditCanvas()) return Promise.resolve({ requestId: request.requestId, ok: false, reason: "app_required", message: "파일 편집은 앱의 Design 탭에서 사용할 수 있습니다." });
  if (!isDesignEditRequest(request)) return Promise.resolve({ requestId, ok: false, reason: "invalid_request", message: "편집 요청의 위치·값·해시 형식을 확인하세요." });
  return new Promise((resolve) => {
    let timer: ReturnType<typeof setTimeout>;
    const finish = (result: DesignEditResult) => { clearTimeout(timer); window.removeEventListener("dojang:design-edit-result", receive); signal?.removeEventListener("abort", abort); resolve(result); };
    const abort = () => finish({ requestId: request.requestId, ok: false, reason: "cancelled", message: "선택 또는 화면이 변경되어 요청을 취소했습니다." });
    const receive = (event: Event) => {
      const value: unknown = (event as CustomEvent).detail;
      if (isDesignEditResult(value) && value.requestId === request.requestId) finish(value);
    };
    window.addEventListener("dojang:design-edit-result", receive);
    timer = setTimeout(() => finish({ requestId: request.requestId, ok: false, reason: "write_failed", message: "앱의 기록 응답을 받지 못했습니다. 요소를 다시 선택하세요." }), 15000);
    if (signal?.aborted) { abort(); return; }
    signal?.addEventListener("abort", abort, { once: true });
    try { window.location.href = `praxis-designmode:edit?data=${encodeURIComponent(JSON.stringify(request))}`; }
    catch (cause) { finish({ requestId: request.requestId, ok: false, reason: "write_failed", message: String(cause) }); }
  });
}
