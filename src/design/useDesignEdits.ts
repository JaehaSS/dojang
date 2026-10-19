import { useEffect, useRef, useState } from "react";
import type { DesignSelection } from "../lib/design-canvas/selection";
import type { DesignEditInfo } from "../lib/design-canvas/edit";
import { canEditCanvas, requestDesignEdit } from "./edit-client";

export function useDesignEdits(selection: DesignSelection | null, onWritten: (info: DesignEditInfo) => void, revision?: number) {
  const [info, setInfo] = useState<DesignEditInfo | null>(null);
  const [scope, setScope] = useState<"instance" | "component">("instance");
  const [status, setStatus] = useState("idle");
  const [error, setError] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const generation = useRef(0);
  const infoRef = useRef(info); infoRef.current = info;
  const scopeRef = useRef(scope); scopeRef.current = scope;
  const selectionRef = useRef(selection); selectionRef.current = selection;
  const writtenRef = useRef(onWritten); writtenRef.current = onWritten;
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const pending = useRef(new Map<string, { value: string; readyAt: number }>());
  const writing = useRef(false);
  const confirmed = useRef(false);
  const abortRef = useRef(new AbortController());

  const inspect = async (nextScope: "instance" | "component", stamp: number) => {
    const target = selectionRef.current;
    if (!target?.source || !canEditCanvas()) return;
    setStatus("inspecting");
    const result = await requestDesignEdit({ requestId: crypto.randomUUID(), operation: "inspect", source: target.source, instanceSource: target.instanceSource, scope: nextScope }, abortRef.current.signal);
    if (stamp !== generation.current) return;
    if (!result.ok) { setInfo(null); setStatus("write-failed"); setError(`${result.reason}: ${result.message}`); return; }
    infoRef.current = result; setInfo(result); setStatus("idle");
  };
  useEffect(() => {
    const stamp = ++generation.current;
    abortRef.current.abort(); abortRef.current = new AbortController();
    clearTimeout(timer.current); pending.current.clear(); confirmed.current = false;
    infoRef.current = null; setInfo(null); scopeRef.current = "instance"; setScope("instance");
    setDrafts({}); setError(null); setStatus("idle");
    void inspect("instance", stamp);
    return () => { ++generation.current; abortRef.current.abort(); clearTimeout(timer.current); pending.current.clear(); };
  }, [revision ?? selection?.source, revision === undefined ? selection?.instanceSource : null, !!selection]);

  const flush = async (immediate = false) => {
    clearTimeout(timer.current);
    if (immediate) for (const item of pending.current.values()) item.readyAt = 0;
    if (writing.current || !infoRef.current?.editable) return;
    const stamp = generation.current;
    writing.current = true;
    try {
      while (pending.current.size && stamp === generation.current) {
        const eligible = [...pending.current.entries()].find(([, item]) => item.readyAt <= Date.now());
        if (!eligible) break;
        const [property, { value }] = eligible;
        pending.current.delete(property);
        const current = infoRef.current!;
        const target = selectionRef.current;
        if (!target?.source) break;
        setStatus("writing"); setError(null);
        const result = await requestDesignEdit({ requestId: crypto.randomUUID(), operation: "apply", source: target.source, instanceSource: target.instanceSource, scope: scopeRef.current, componentConfirmed: confirmed.current, property, value, expectedHash: current.hash }, abortRef.current.signal);
        if (stamp !== generation.current) return;
        if (!result.ok) {
          pending.current.clear(); setStatus(result.reason === "stale_file" ? "stale-file" : "write-failed");
          setError(`${result.reason}: ${result.message}`); infoRef.current = null; setInfo(null); return;
        }
        infoRef.current = result; setInfo(result); setStatus("waiting-hmr"); writtenRef.current(result);
      }
    } finally {
      writing.current = false;
      if (pending.current.size && infoRef.current?.editable) {
        const delay = Math.max(0, Math.min(...[...pending.current.values()].map((item) => item.readyAt)) - Date.now());
        timer.current = setTimeout(() => void flush(), delay);
      }
    }
  };
  const change = (property: string, value: string) => {
    setDrafts((current) => ({ ...current, [property]: value }));
    if (!value.trim() || !infoRef.current?.editable) { pending.current.delete(property); return; }
    const probe = document.createElement("span"); probe.style.setProperty(property, value.trim());
    if (!probe.style.getPropertyValue(property)) { pending.current.delete(property); setError(`${property}: CSS 값과 단위를 확인하세요.`); return; }
    setError(null);
    const canonical = ["width", "height", "padding-top", "padding-right", "padding-bottom", "padding-left", "margin-top", "margin-right", "margin-bottom", "margin-left", "border-radius", "font-size"].includes(property) ? probe.style.getPropertyValue(property) : value.trim();
    pending.current.set(property, { value: canonical, readyAt: Date.now() + 200 }); clearTimeout(timer.current); timer.current = setTimeout(() => void flush(), 200);
  };
  const chooseScope = async (next: "instance" | "component") => {
    if (writing.current) return;
    if (next === "component" && !window.confirm("컴포넌트 정의를 변경하면 모든 사용처에 적용됩니다. 계속하시겠습니까?")) return;
    clearTimeout(timer.current); pending.current.clear(); setDrafts({}); setError(null);
    confirmed.current = next === "component"; scopeRef.current = next; setScope(next);
    abortRef.current.abort(); abortRef.current = new AbortController();
    const stamp = ++generation.current; infoRef.current = null; setInfo(null); await inspect(next, stamp);
  };
  return { info, scope, status, error, drafts, enabled: canEditCanvas(), change, flush: () => void flush(true), chooseScope, hmrReady: () => { setStatus((current) => current === "waiting-hmr" ? "idle" : current); } };
}
