import { useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { b64ToBytes } from "../../lib/bytes";
import {
  projectShellClose,
  projectShellOpen,
  projectShellResize,
  projectShellSnapshot,
  projectShellWrite,
  type ProjectShellExit,
  type ProjectShellOutput,
} from "../../lib/project-editor-ipc";
import { useXtermTheme } from "../TerminalView";
import { disposeTerminal, disposeWebgl, tryLoadWebgl } from "../../lib/xterm-webgl";

interface Props {
  /** 공용 에디터에서는 늦은 요청을 원래 프로젝트에 한정한다. */
  root?: string;
  hidden: boolean;
  restart: number;
  onSession: (session: number | null) => void;
  onExit: (exited: boolean) => void;
  onError: (message: string) => void;
}

/** Root-bound PTY. It stays mounted while hidden so the xterm buffer and shell survive a toggle. */
export function ProjectTerminal({ root, hidden, restart, onSession, onExit, onError }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const theme = useXtermTheme();
  const themeRef = useRef(theme);
  themeRef.current = theme;
  const [session, setSession] = useState<number | null>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    let disposed = false;
    let token: number | null = null;
    let ready = false;
    let writable = false;
    let lastSequence = 0;
    let pendingExit: ProjectShellExit | null = null;
    setSession(null);
    const queued: ProjectShellOutput[] = [];
    const term = new Terminal({ cursorBlink: true, fontSize: 13, theme: themeRef.current });
    termRef.current = term;
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    const webgl = tryLoadWebgl(term);
    const write = (data: string) => term.write(b64ToBytes(data));
    const receive = (event: ProjectShellOutput) => {
      if (disposed) return;
      if (token == null) return queued.push(event);
      if (event.session !== token || event.sequence <= lastSequence) return;
      if (!ready) return queued.push(event);
      lastSequence = event.sequence;
      write(event.data);
    };
    const receiveExit = (event: ProjectShellExit) => {
      if (token == null) { pendingExit = event; return; }
      if (event.session !== token || disposed) return;
      if (!ready) { pendingExit = event; return; }
      writable = false;
      onExit(true);
      term.writeln(`\r\n\x1b[90m[shell exited: ${event.code}]\x1b[0m`);
    };
    let unOut: UnlistenFn | undefined;
    let unExit: UnlistenFn | undefined;
    const stop = () => { unOut?.(); unExit?.(); unOut = undefined; unExit = undefined; };
    const start = async () => {
      try {
        unOut = await listen<ProjectShellOutput>("project-shell://output", ({ payload }) => receive(payload));
        if (disposed) { stop(); return; }
        unExit = await listen<ProjectShellExit>("project-shell://exit", ({ payload }) => receiveExit(payload));
        if (disposed) { stop(); return; }
        try { fit.fit(); } catch { /* next resize supplies dimensions */ }
        const opened = await projectShellOpen(term.cols, term.rows, root);
        if (disposed) {
          // The native window owns this PTY. Destroyed cleans it up; a view
          // remount may already be attaching to the same session.
          stop();
          return;
        }
        token = opened.session;
        setSession(token);
        onSession(token);
        const snapshot = await projectShellSnapshot(token, root);
        if (disposed || snapshot.session !== token) return;
        if (snapshot.data) write(snapshot.data);
        lastSequence = snapshot.sequence;
        queued
          .filter((event) => event.session === token && event.sequence > snapshot.sequence)
          .sort((a, b) => a.sequence - b.sequence)
          .forEach((event) => { lastSequence = event.sequence; write(event.data); });
        queued.length = 0;
        ready = true;
        if (snapshot.exited || pendingExit?.session === token) receiveExit(pendingExit ?? { session: token, code: snapshot.exit_code ?? 0 });
        else { writable = true; onExit(false); resize(); }
      } catch (error) {
        stop();
        writable = false;
        if (!disposed) {
          onError(String(error));
          // Keep any opened session token: explicit retry closes it. A failed
          // snapshot may be attaching to an already-running user shell.
          onExit(true);
        }
      }
    };
    void start();
    const input = term.onData((data) => { if (writable && !disposed && token != null) void projectShellWrite(token, data, root).catch((error) => { if (!disposed) onError(String(error)); }); });
    const resize = () => {
      try {
        fit.fit();
        if (writable && !disposed && token != null) void projectShellResize(token, term.cols, term.rows, root).catch((error) => { if (!disposed) onError(String(error)); });
      } catch { /* hidden terminal has no layout */ }
    };
    const observer = new ResizeObserver(resize);
    observer.observe(host);
    return () => {
      disposed = true;
      stop(); observer.disconnect(); input.dispose(); disposeWebgl(webgl); disposeTerminal(term);
      termRef.current = null;
      onSession(null);
    };
  }, [restart, root]);

  useEffect(() => { if (termRef.current) termRef.current.options.theme = theme; }, [theme]);
  useEffect(() => { if (!hidden) window.dispatchEvent(new Event("resize")); }, [hidden]);

  return <div ref={(node) => { hostRef.current = node; }} data-project-shell={session ?? undefined} className={`flex-1 min-h-0 bg-bg ${hidden ? "hidden" : ""}`} />;
}

export async function restartProjectShell(session: number | null, root?: string): Promise<void> {
  if (session != null) await projectShellClose(session, root);
}
