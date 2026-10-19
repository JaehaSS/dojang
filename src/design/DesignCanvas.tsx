import { useEffect, useRef, useState } from "react";
import { ErrorBoundary } from "../components/ErrorBoundary";
import type { ScreenEntry } from "./registry";
import { isDesignSelection, type DesignSelection } from "../lib/design-canvas/selection";
import { bindCanvasDocument, sendSelectionToApp, type CanvasSelector } from "./selection-client";
import { DesignInspector, selectionTokenMatches } from "./DesignInspector";
import type { DesignTokens } from "./token-match";
import { useDesignEdits } from "./useDesignEdits";
import type { DesignEditInfo } from "../lib/design-canvas/edit";

const resumeKey = "dojang-design-canvas-resume";
function readResume(): { screen?: string; state?: string; viewport?: string; zoom?: number; source?: string; instanceSource?: string | null } {
  try { const value: unknown = JSON.parse(sessionStorage.getItem(resumeKey) ?? "null"); return value && typeof value === "object" ? value : {}; } catch { return {}; }
}

export function ScreenFrame({ screens, search }: { screens: ScreenEntry[]; search: string }) {
  const params = new URLSearchParams(search);
  const screen = screens.find((entry) => entry.id === params.get("screen"));
  if (!screen || screen.error !== null) return <p role="alert">{screen?.error ?? "화면을 찾을 수 없습니다."}</p>;
  const state = params.get("state") ?? Object.keys(screen.meta.states)[0];
  if (!Object.prototype.hasOwnProperty.call(screen.meta.states, state)) return <p role="alert">상태를 찾을 수 없습니다.</p>;
  return <ErrorBoundary key={`${screen.id}:${state}`}><screen.Screen {...screen.meta.states[state]} /></ErrorBoundary>;
}

export function DesignCanvas({ screens, tokens = {} }: { screens: ScreenEntry[]; tokens?: DesignTokens }) {
  const [resume] = useState(readResume);
  const [selected, setSelected] = useState<string | null>(typeof resume.screen === "string" ? resume.screen : null);
  const [selectedState, setSelectedState] = useState<string | null>(typeof resume.state === "string" ? resume.state : null);
  const [viewport, setViewport] = useState(resume.viewport && ["meta", "390x844", "960x640", "1440x900"].includes(resume.viewport) ? resume.viewport : "meta");
  const [zoom, setZoom] = useState(resume.zoom && [50, 75, 100].includes(resume.zoom) ? resume.zoom : 100);
  const [selection, setSelection] = useState<DesignSelection | null>(null);
  const [selectionRevision, setSelectionRevision] = useState(0);
  const [inlineProperties, setInlineProperties] = useState<string[]>([]);
  const [selecting, setSelecting] = useState(false);
  const [selectionError, setSelectionError] = useState<string | null>(null);
  const [matches, setMatches] = useState<ReturnType<typeof selectionTokenMatches>>({});
  const [selectorReady, setSelectorReady] = useState(false);
  const frameRef = useRef<HTMLIFrameElement>(null);
  const selectorRef = useRef<CanvasSelector | null>(null);
  const pendingRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const pendingIdRef = useRef<string | null>(null);
  const pendingDocRef = useRef<Document | null>(null);
  const pendingFreshRef = useRef(false);
  const restoringRef = useRef(false);
  const restoreFreshRef = useRef(false);
  const restoreRef = useRef<{ source: string; instanceSource?: string | null } | null>(typeof resume.source === "string" ? { source: resume.source, instanceSource: resume.instanceSource } : null);
  const hmrTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const tokensRef = useRef(tokens);
  tokensRef.current = tokens;
  const screen = screens.find((entry) => entry.id === selected) ?? screens.find((entry) => entry.error === null) ?? screens[0];
  const meta = screen?.error === null ? screen.meta : null;
  const states = meta ? Object.keys(meta.states) : [];
  const state = selectedState && states.includes(selectedState) ? selectedState : states[0];
  const size = meta ? (viewport === "meta" ? meta.viewport : { width: Number(viewport.split("x")[0]), height: Number(viewport.split("x")[1]) }) : null;
  const frameUrl = screen && state ? `/design.html?${new URLSearchParams({ view: "screen", screen: screen.id, state })}` : null;
  const choose = (id: string, nextState?: string) => { restoreRef.current = null; setSelected(id); setSelectedState(nextState ?? null); setViewport("meta"); };
  const persist = () => { try { sessionStorage.setItem(resumeKey, JSON.stringify({ screen: screen?.id, state, viewport, zoom, ...restoreRef.current })); } catch { /* Canvas still works when storage is unavailable. */ } };
  const restoreSelection = () => {
    const target = restoreRef.current;
    if (!target) return;
    restoringRef.current = true;
    const restored = selectorRef.current?.reselectSource?.(target.source, target.instanceSource);
    restoringRef.current = false;
    restoreFreshRef.current = false;
    if (!restored) {
      restoreRef.current = null; setSelection(null); setMatches({});
      setSelectionError("갱신 후 선택 요소를 찾을 수 없거나 같은 소스 위치의 요소가 여러 개입니다. 다시 선택하세요."); persist();
    } else edit.hmrReady();
  };
  const written = (info: DesignEditInfo) => {
    restoreRef.current = { source: info.selectionSource, instanceSource: info.shared && edit.scope === "instance" ? info.source : selection?.instanceSource };
    persist(); clearTimeout(hmrTimer.current); hmrTimer.current = setTimeout(restoreSelection, 200);
  };
  const edit = useDesignEdits(selection, written, selectionRevision);
  const receiveSelection = (next: DesignSelection, fresh = false) => {
    const document = frameRef.current?.contentDocument;
    const element = selectorRef.current?.getSelectedElement?.() as HTMLElement | null | undefined;
    const style = element?.style;
    setInlineProperties(style ? [...new Set([...Object.keys(next.computedCss), "width", "height", "border-radius"])].filter((property) => !!style.getPropertyValue(property)
      || property === "border-radius" && ["top-left", "top-right", "bottom-left", "bottom-right"].some((corner) => !!style.getPropertyValue(`border-${corner}-radius`))) : []);
    setMatches(document ? selectionTokenMatches(next, tokensRef.current, document, selectorRef.current?.getSelectedElement?.()) : {});
    setSelection(next);
    if (fresh) setSelectionRevision((value) => value + 1);
    restoreRef.current = next.source ? { source: next.source, instanceSource: next.instanceSource } : null;
    persist();
  };
  const receiveRef = useRef(receiveSelection); receiveRef.current = receiveSelection;
  const priorFrameUrl = useRef(frameUrl);
  const priorViewport = useRef(viewport);

  const restoreCallback = useRef(restoreSelection); restoreCallback.current = restoreSelection;
  useEffect(() => {
    const refresh = () => {
      // Fast Refresh retains the iframe Document, so document identity alone
      // cannot reject a pre-refresh ACK. Cancel it and measure the new DOM.
      if (pendingIdRef.current) restoreFreshRef.current = pendingFreshRef.current;
      clearTimeout(pendingRef.current); pendingIdRef.current = null; pendingDocRef.current = null;
      clearTimeout(hmrTimer.current); hmrTimer.current = setTimeout(() => restoreCallback.current(), 100);
    };
    window.addEventListener("dojang:design-hmr", refresh);
    return () => { window.removeEventListener("dojang:design-hmr", refresh); clearTimeout(hmrTimer.current); };
  }, []);
  useEffect(persist, [screen?.id, state, viewport, zoom]);

  useEffect(() => {
    const accept = (event: Event) => {
      const data: unknown = (event as CustomEvent<unknown>).detail;
      if (!isDesignSelection(data)) return;
      if (!pendingIdRef.current || data.requestId !== pendingIdRef.current) return;
      if (pendingDocRef.current !== frameRef.current?.contentDocument || !pendingDocRef.current?.body) return;
      clearTimeout(pendingRef.current);
      pendingIdRef.current = null;
      pendingDocRef.current = null;
      setSelectionError(null);
      receiveRef.current(data, pendingFreshRef.current);
    };
    window.addEventListener("dojang:design-selection", accept);
    return () => { window.removeEventListener("dojang:design-selection", accept); clearTimeout(pendingRef.current); selectorRef.current?.dispose(); };
  }, []);

  useEffect(() => {
    if (priorFrameUrl.current !== frameUrl) { restoreRef.current = null; persist(); }
    priorFrameUrl.current = frameUrl;
    selectorRef.current?.dispose();
    selectorRef.current = null;
    clearTimeout(pendingRef.current);
    pendingIdRef.current = null;
    pendingDocRef.current = null;
    setSelection(null);
    setMatches({});
    setSelectorReady(false);
    setSelecting(false);
    setSelectionError(null);
  }, [frameUrl]);

  useEffect(() => {
    // Viewport changes reflow the same iframe without a new load event. Clear
    // measured geometry but keep its bound controller ready for another pick.
    selectorRef.current?.setEnabled(false);
    if (priorViewport.current !== viewport) { restoreRef.current = null; persist(); }
    priorViewport.current = viewport;
    clearTimeout(pendingRef.current);
    pendingIdRef.current = null;
    pendingDocRef.current = null;
    setSelection(null);
    setMatches({});
    setSelecting(false);
    setSelectionError(null);
  }, [viewport]);

  const bindFrame = () => {
    selectorRef.current?.dispose();
    selectorRef.current = null;
    clearTimeout(pendingRef.current);
    pendingIdRef.current = null;
    pendingDocRef.current = null;
    setSelection(null);
    setMatches({});
    setSelectorReady(false);
    setSelecting(false);
    try {
      const document = frameRef.current?.contentDocument;
      if (!document) throw new Error("화면 문서에 접근할 수 없습니다.");
      selectorRef.current = bindCanvasDocument(document, (next) => {
        if (document !== frameRef.current?.contentDocument) return;
        setSelecting(false);
        setSelectionError(null);
        if (!isDesignSelection(next)) { setSelectionError("선택한 요소 정보를 확인할 수 없습니다."); return; }
        const request = { ...next, requestId: crypto.randomUUID() };
        restoreRef.current = next.source ? { source: next.source, instanceSource: next.instanceSource } : null;
        persist();
        pendingFreshRef.current = restoreFreshRef.current || !restoringRef.current;
        if (pendingFreshRef.current) { setSelection(null); setMatches({}); setInlineProperties([]); }
        pendingIdRef.current = request.requestId;
        pendingDocRef.current = document;
        if (!sendSelectionToApp(request)) { pendingIdRef.current = null; pendingDocRef.current = null; receiveRef.current(next, pendingFreshRef.current); }
        else {
          clearTimeout(pendingRef.current);
          pendingRef.current = setTimeout(() => { pendingIdRef.current = null; pendingDocRef.current = null; setSelectionError("앱에서 선택 정보를 확인하지 못했습니다. 요소를 다시 선택하세요."); }, 3000);
        }
      });
      setSelectorReady(true);
      if (restoreRef.current) { clearTimeout(hmrTimer.current); hmrTimer.current = setTimeout(() => restoreCallback.current(), 100); }
    } catch (cause) { setSelectionError(String(cause)); }
  };
  const toggleSelection = () => {
    const next = !selecting;
    selectorRef.current?.setEnabled(next);
    setSelecting(next);
  };

  return (
    <main className="grid h-screen min-w-[700px] grid-cols-[160px_minmax(0,1fr)_260px] bg-bg font-ui text-sm text-text">
      <nav className="overflow-auto border-r border-border bg-surface p-3" aria-label="화면 목록">
        <h1 className="mb-3 font-medium">화면</h1>
        {screens.length === 0 && <p className="text-text-secondary">등록된 화면이 없습니다.</p>}
        {screens.map((entry) => (
          <div key={entry.id} className="mb-3">
            <button type="button" onClick={() => choose(entry.id)} aria-pressed={screen?.id === entry.id} className={`w-full break-words rounded px-2 py-1 text-left ${screen?.id === entry.id ? "bg-primary/10 text-primary-bright" : "text-text-secondary hover:text-text"}`}>
              {entry.error === null ? entry.meta.title : entry.id}
            </button>
            {entry.error !== null ? <p role="alert" className="mt-1 break-words text-xs text-status-failed">{entry.error}</p> : Object.keys(entry.meta.states).map((name) => (
              <button key={name} type="button" onClick={() => choose(entry.id, name)} aria-pressed={screen?.id === entry.id && state === name} className="block w-full rounded px-4 py-1 text-left text-xs text-text-secondary hover:bg-raised">{name}</button>
            ))}
          </div>
        ))}
      </nav>
      <section className="flex min-h-0 min-w-0 flex-col" aria-label="디자인 캔버스">
        <header className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border bg-surface px-3 py-2">
          <span>{meta?.title ?? "화면을 선택하세요"}</span>
          {meta && <>
            <select aria-label="화면 상태" value={state} onChange={(event) => setSelectedState(event.target.value)} className="rounded border border-border bg-raised px-2 py-1">{states.map((name) => <option key={name}>{name}</option>)}</select>
            <select aria-label="뷰포트" value={viewport} onChange={(event) => setViewport(event.target.value)} className="rounded border border-border bg-raised px-2 py-1">
              <option value="meta">{meta.viewport.width}×{meta.viewport.height} · 화면 기본</option>
              <option value="390x844">390×844</option><option value="960x640">960×640</option><option value="1440x900">1440×900</option>
            </select>
            <select aria-label="배율" value={zoom} onChange={(event) => setZoom(Number(event.target.value))} className="rounded border border-border bg-raised px-2 py-1">{[50, 75, 100].map((value) => <option key={value} value={value}>{value}%</option>)}</select>
            <button type="button" disabled={!selectorReady} aria-pressed={selecting} onClick={toggleSelection} className={`rounded border px-2 py-1 disabled:opacity-50 ${selecting ? "border-primary text-primary-bright" : "border-border text-text-secondary"}`}>요소 선택</button>
          </>}
        </header>
        {selectionError && <p role="alert" className="border-b border-border px-3 py-2 text-xs text-status-failed">{selectionError}</p>}
        <div className="min-h-0 flex-1 overflow-auto p-4">
          {screen?.error && <p role="alert" className="text-status-failed">{screen.error}</p>}
          {size && frameUrl && <div style={{ width: size.width * zoom / 100, height: size.height * zoom / 100 }}>
            <iframe ref={frameRef} onLoad={bindFrame} title={`${meta?.title} · ${state}`} src={frameUrl} width={size.width} height={size.height} className="block origin-top-left border-0 bg-bg outline outline-1 outline-border" style={{ width: size.width, height: size.height, maxWidth: "none", transform: `scale(${zoom / 100})` }} />
          </div>}
        </div>
        <footer className="shrink-0 border-t border-border px-3 py-2 text-xs text-text-secondary">개발 서버 {window.location.host} · 연결됨</footer>
      </section>
      <DesignInspector selection={selection} matches={matches} onSelectAncestor={() => selectorRef.current?.selectSuggestedAncestor()} edit={edit} inlineProperties={inlineProperties} />
    </main>
  );
}
