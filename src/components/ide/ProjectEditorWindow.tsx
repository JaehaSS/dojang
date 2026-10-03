import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { emit } from "@tauri-apps/api/event";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { fileTabKey } from "../../lib/tab-key";
import { LOCAL_HOST } from "../../lib/transport";
import { projectEditorInfo, projectEditorOpenPath, projectEditorRead, projectEditorResolvePath, projectEditorTree, projectEditorWrite, type ProjectEditorInfo } from "../../lib/project-editor-ipc";
import { useWorkspaceFiles, type WorkspaceFileSource } from "./useWorkspaceFiles";
import { EditorSplitView } from "./EditorSplitView";
import { FileTree } from "./FileTree";
import { Icon } from "./icons";
import { ProjectTerminal, restartProjectShell } from "./ProjectTerminal";
import { PROJECT_EDITOR_READY_EVENT } from "../../lib/editor-window-events";
import { useTheme } from "../../lib/use-theme";
import { codeFontStack, uiFontStack } from "../../lib/fonts";
import { DEFAULT_EDITOR_SETTINGS, normalizeEditorSettings } from "../../lib/editor-settings";
import { editorSettingsGet, fontSettingsGet, type EditorSettings, type FontSettings } from "../../lib/ipc";

const tabsKey = (root: string) => `praxis-project-editor:${root}`;

/** A separate window for one canonical local root. Task-only editor functions receive null identity. */
export function ProjectEditorWindow() {
  const [info, setInfo] = useState<ProjectEditorInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [treeOpen, setTreeOpen] = useState(true);
  const [terminalOpen, setTerminalOpen] = useState(false);
  const [terminalStarted, setTerminalStarted] = useState(false);
  const [terminalExited, setTerminalExited] = useState(false);
  const [shellSession, setShellSession] = useState<number | null>(null);
  const [restart, setRestart] = useState(0);
  const [fontSettings, setFontSettings] = useState<FontSettings | null>(null);
  const [editorSettings, setEditorSettings] = useState<EditorSettings>(DEFAULT_EDITOR_SETTINGS);
  const [restoredRoot, setRestoredRoot] = useState<string | null>(null);
  const closingRef = useRef(false);
  const closingBusyRef = useRef(false);
  const restartingRef = useRef(false);
  const theme = useTheme();
  const source = useMemo<WorkspaceFileSource | null>(() => info == null ? null : {
    key: `project:${info.root}`,
    tree: projectEditorTree,
    read: projectEditorRead,
    write: projectEditorWrite,
  }, [info?.root]);
  const files = useWorkspaceFiles({ task: null, source, onError: setError });

  const loadInfo = useCallback(() => {
    return projectEditorInfo().then((next) => {
      setInfo(next);
      // 에이전트 CLI로 뜬 창은 터미널이 본문이다 — 사용자가 열 때까지 기다리지 않는다.
      if (next.launch) { setTerminalStarted(true); setTerminalOpen(true); }
      void emit(PROJECT_EDITOR_READY_EVENT, next.label).catch(() => {});
    }).catch((cause) => setError(String(cause)));
  }, []);
  useEffect(() => { void loadInfo(); }, [loadInfo]);
  useEffect(() => {
    void fontSettingsGet().then((settings) => {
      document.documentElement.style.setProperty("--font-ui", uiFontStack(settings.ui_family));
      document.documentElement.style.setProperty("--font-code", codeFontStack(settings.code_family));
      document.documentElement.style.setProperty("--font-ui-size", `${settings.ui_size}px`);
      setFontSettings(settings);
    }).catch(() => {});
    void editorSettingsGet().then((settings) => setEditorSettings(normalizeEditorSettings(settings))).catch(() => {});
  }, []);
  useEffect(() => { if (info) files.refreshTree(); }, [info?.root]);
  useEffect(() => {
    if (!info) return;
    const saved = localStorage.getItem(tabsKey(info.root));
    if (!saved) { setRestoredRoot(info.root); return; }
    let value: { paths?: string[]; active?: string | null };
    try {
      value = JSON.parse(saved);
      if (!Array.isArray(value.paths) || !value.paths.every((path) => typeof path === "string")) throw new Error("invalid tabs");
    } catch { setError("저장된 탭 정보를 읽지 못했습니다."); setRestoredRoot(info.root); return; }
    void (async () => {
      for (const path of value.paths ?? []) await files.openFile(path);
      if (value.active) files.setActiveKey(fileTabKey(value.active));
      setRestoredRoot(info.root);
    })();
  }, [info?.root]);
  useEffect(() => {
    if (!info || restoredRoot !== info.root) return;
    const paths = files.openFiles.filter((file) => !file.dirty && file.kind !== "diff").map((file) => file.path);
    localStorage.setItem(tabsKey(info.root), JSON.stringify({ paths, active: files.activeFile?.path ?? null }));
  }, [info?.root, restoredRoot, files.openFiles, files.activeFile?.path]);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!event.ctrlKey || event.metaKey || event.altKey || event.shiftKey || event.code !== "Backquote") return;
      event.preventDefault();
      setTerminalStarted(true); setTerminalOpen((open) => !open);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);
  useEffect(() => {
    const win = getCurrentWindow();
    const unlisten = win.onCloseRequested((event) => {
      if (closingRef.current) return;
      event.preventDefault();
      if (closingBusyRef.current) return;
      void (async () => {
        closingBusyRef.current = true;
        const result = await files.flushDirty();
        if (!result.ok) {
          setError(`${result.path}를 저장하지 못했습니다: ${result.detail ?? result.reason}`);
          closingBusyRef.current = false;
          return;
        }
        if (terminalStarted && !window.confirm("실행 중인 프로젝트 셸을 종료하고 창을 닫을까요?")) { closingBusyRef.current = false; return; }
        closingRef.current = true;
        try { await win.close(); } catch (cause) { closingRef.current = false; closingBusyRef.current = false; setError(`창을 닫지 못했습니다: ${String(cause)}`); }
      })();
    });
    return () => { void unlisten.then((stop) => stop()); };
  }, [files.flushDirty, terminalStarted]);

  const beforeCloseTab = useCallback((key: string) => {
    const file = files.openFiles.find((item) => item.key === key);
    if (!file?.dirty) return true;
    return window.confirm(`${file.path}에 저장하지 않은 변경이 있습니다. 버리시겠습니까?`);
  }, [files]);
  const toggleTerminal = () => { setTerminalStarted(true); setTerminalOpen((open) => !open); };
  const restartShell = async () => {
    if (restartingRef.current) return;
    restartingRef.current = true;
    try {
      await restartProjectShell(shellSession);
      setError(null);
      setTerminalExited(false); setRestart((value) => value + 1);
    } catch (cause) {
      setError(`터미널을 다시 시작하지 못했습니다: ${String(cause)}`);
    } finally { restartingRef.current = false; }
  };
  if (!info) return <div className="h-screen grid place-items-center gap-2 text-sm text-text-muted">{error ?? "프로젝트를 여는 중…"}{error && <button className="text-primary-bright" onClick={() => { setError(null); void loadInfo(); }}>다시 시도</button>}</div>;
  return <div className="h-screen flex flex-col bg-bg text-text">
    <header className="h-9 shrink-0 flex items-center gap-2 px-3 border-b border-border text-xs text-text-muted">
      <Icon name="folder" size={14} /><span className="truncate">{info.root}</span>
      <button className="ml-auto p-1 hover:text-text" onClick={toggleTerminal} aria-label={terminalOpen ? "터미널 닫기" : "터미널 열기"} aria-pressed={terminalOpen} title={terminalOpen ? "터미널 닫기 (⌃`)" : "터미널 열기 (⌃`)"}><Icon name="terminal" size={14} /></button>
    </header>
    <div className="flex flex-1 min-h-0">
      <aside className={`w-64 shrink-0 border-r border-border flex-col ${treeOpen ? "flex" : "hidden"}`}>
        <div className="h-8 px-2 flex items-center text-xs text-text-muted">파일<button className="ml-auto p-1 hover:text-text" onClick={() => files.refreshTree()} aria-label="파일 트리 새로고침"><Icon name="refresh" size={13} /></button><button className="p-1 hover:text-text" onClick={() => setTreeOpen(false)} aria-label="파일 트리 닫기"><Icon name="x" size={13} /></button></div>
        <div className="flex-1 overflow-auto"><FileTree nodes={files.tree} activePath={files.activeFile?.path ?? null} onOpen={(path) => void files.openFile(path, { preview: true, tree: true })} onPin={(path) => files.pinTab(fileTabKey(path))} /></div>
      </aside>
      {!treeOpen && <button className="w-8 shrink-0 border-r border-border" onClick={() => setTreeOpen(true)} aria-label="파일 트리 열기"><Icon name="folder" size={14} /></button>}
      <div className="flex min-w-0 min-h-0 flex-1 flex-col">
        <EditorSplitView taskId={null} host={LOCAL_HOST} rootPath={info.root} windowId={info.label} ownsWindow files={files.openFiles} activeKey={files.activeKey} treeOpen={files.treeOpen} onTreeOpenHandled={files.consumeTreeOpen} dark={theme.kind !== "light"} onSelect={files.setActiveKey} onClose={files.closeTab} onBeforeClose={beforeCloseTab} onOpenFile={files.openFile} onChange={files.changeFile} onSave={(path, content) => void files.saveFile(path, content)} onReload={(path) => void files.reloadFile(path)} onOpenPath={(path) => void projectEditorOpenPath(path).catch((cause) => setError(String(cause)))} onRevealPath={(path) => void projectEditorResolvePath(path).then(revealItemInDir).catch((cause) => setError(String(cause)))} onPinTab={files.pinTab} codeFontFamily={fontSettings ? codeFontStack(fontSettings.code_family) : undefined} codeFontSize={fontSettings?.code_size} editorSettings={editorSettings} />
        {terminalStarted && <section className={`h-64 shrink-0 border-t border-border flex flex-col ${terminalOpen ? "" : "hidden"}`}><div className="h-8 px-3 flex items-center gap-2 text-xs text-text-muted">터미널{terminalExited && <button className="ml-auto text-primary-bright" onClick={() => void restartShell()}>재시작</button>}</div><ProjectTerminal hidden={!terminalOpen} restart={restart} onSession={setShellSession} onExit={setTerminalExited} onError={setError} /></section>}
      </div>
    </div>
    {error && <div className="shrink-0 border-t border-border px-3 py-1 text-xs text-danger" role="alert">{error}</div>}
  </div>;
}
