import { useMemo, useState } from "react";
import { RepoPicker, type RecentRepo } from "./RepoPicker";
import { Icon } from "./icons";

const RECENT_KEY = "praxis-project-editor-recent";

export function recentProjectRoots(): RecentRepo[] {
  try {
    const raw = localStorage.getItem(RECENT_KEY);
    const entries = raw == null ? [] : JSON.parse(raw) as RecentRepo[];
    return entries.filter((entry) => typeof entry.path === "string" && Number.isFinite(entry.lastUsed));
  } catch { return []; }
}

export function rememberProjectRoot(root: string): void {
  const entries = recentProjectRoots().filter((entry) => entry.path !== root);
  localStorage.setItem(RECENT_KEY, JSON.stringify([{ path: root, lastUsed: Math.floor(Date.now() / 1000) }, ...entries].slice(0, 20)));
}

interface Props {
  localRoots: string[];
  onOpen: (root: string) => Promise<string>;
}

/** Home entry deliberately accepts only local roots; remote task paths cannot become project roots. */
export function HomeProjectEditor({ localRoots, onOpen }: Props) {
  const [recent, setRecent] = useState(recentProjectRoots);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const candidates = useMemo(() => {
    const seen = new Map<string, RecentRepo>();
    for (const entry of recent) seen.set(entry.path, entry);
    for (const path of localRoots) if (!seen.has(path)) seen.set(path, { path, lastUsed: 0 });
    return [...seen.values()].sort((a, b) => b.lastUsed - a.lastUsed);
  }, [localRoots, recent]);
  const open = async (root: string) => {
    if (busy) return;
    setBusy(true); setError(null);
    try {
      const canonical = await onOpen(root);
      rememberProjectRoot(canonical);
      setRecent(recentProjectRoots());
    } catch (cause) { setError(String(cause)); }
    finally { setBusy(false); }
  };
  return <section className="mb-5 rounded-md border border-border bg-surface px-3 py-2.5">
    <div className="flex items-center gap-2">
      <Icon name="folder" size={16} />
      <span className="text-sm text-text">프로젝트 에디터</span>
      <span className="text-xs text-text-muted">작업 없이 원본 폴더를 엽니다</span>
      <div className="ml-auto"><RepoPicker repo="" triggerLabel="에디터 열기" recentRepos={candidates} onPick={(path) => void open(path)} label="최근 프로젝트" onOpen={() => setRecent(recentProjectRoots())} placement="down" align="right" /></div>
    </div>
    {busy && <div className="mt-2 text-xs text-text-muted">프로젝트를 여는 중…</div>}
    {error && <div className="mt-2 text-xs text-danger" role="alert">{error}</div>}
  </section>;
}
