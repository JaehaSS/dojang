import { useCallback, useEffect, useRef, useState } from "react";
import { HtmlDoc } from "./HtmlDoc";
import {
  prepareArtifactSave,
  artifactReviewContext,
  reviewArtifactGet,
  reviewArtifactList,
  reviewArtifactSave,
  type ArtifactSnapshot,
  type ArtifactVersion,
} from "../../lib/review-artifacts";

export interface ArtifactReviewPanelProps {
  taskId: number;
  sourceId: string;
  resultRevision: number;
  paths: string[];
  disabled?: boolean;
  onAttachReview: (text: string) => void;
  onOpenFile: (path: string) => void;
}

const versionLabel = (version: ArtifactVersion) => `${version.relative_path} · v${version.version} · ${version.content_hash.slice(0, 12)}`;

function SnapshotPane({ label, snapshot }: { label: string; snapshot: ArtifactSnapshot | null }) {
  if (!snapshot) return <section className="min-w-0 border border-border rounded p-3 text-sm text-text-muted"><h4 className="font-medium text-text">{label}</h4><p>저장 버전을 선택하세요.</p></section>;
  return <section className="min-w-0 border border-border rounded p-3 flex flex-col gap-2">
    <h4 className="font-medium">{label}: v{snapshot.version}</h4>
    <p className="text-xs text-text-muted break-all">{snapshot.relative_path} · {snapshot.content_hash}</p>
    <p className="text-xs text-text-muted">작업 결과 v{snapshot.result_revision} · 기준 {snapshot.base_revision?.slice(0, 12) ?? "알 수 없음"}</p>
    {snapshot.unavailable.length > 0 && <p role="status" className="text-xs text-text-muted" title={snapshot.unavailable.join("\n")}>저장하지 못한 리소스 {snapshot.unavailable.length}개</p>}
    {snapshot.kind === "image"
      ? <img src={snapshot.content} alt={`저장 버전 v${snapshot.version}`} className="max-w-full h-auto border border-border" />
      : snapshot.preview_html
        ? <div className="h-[28rem] min-h-0 overflow-hidden"><HtmlDoc html={snapshot.preview_html} title={`${label} 버전: ${snapshot.relative_path}`} /></div>
        : <pre className="max-h-[28rem] overflow-auto whitespace-pre-wrap break-words text-xs font-code">{snapshot.content}</pre>}
  </section>;
}

/** Immutable artifacts only read their saved snapshot; current files are read solely on explicit save. */
export function ArtifactReviewPanel({ taskId, sourceId, resultRevision, paths, disabled = false, onAttachReview, onOpenFile }: ArtifactReviewPanelProps) {
  const [path, setPath] = useState(paths[0] ?? "");
  const [versions, setVersions] = useState<ArtifactVersion[]>([]);
  const [beforeId, setBeforeId] = useState<number | null>(null);
  const [afterId, setAfterId] = useState<number | null>(null);
  const [before, setBefore] = useState<ArtifactSnapshot | null>(null);
  const [after, setAfter] = useState<ArtifactSnapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const pathsKey = paths.join("\0");
  const sessionGeneration = useRef(0);
  const listGeneration = useRef(0);
  const snapshotGeneration = useRef({ before: 0, after: 0 });
  const selectedIds = useRef<{ before: number | null; after: number | null }>({ before: null, after: null });
  const writeGeneration = useRef(0);
  const initialRevision = useRef(resultRevision);

  const reload = useCallback(async () => {
    const sessionAtStart = sessionGeneration.current;
    const listAtStart = ++listGeneration.current;
    setLoading(true);
    try {
      const next = (await reviewArtifactList(taskId)).sort((left, right) => left.id - right.id);
      if (sessionAtStart !== sessionGeneration.current || listAtStart !== listGeneration.current) return;
      setVersions(next);
      const chooseListed = (selected: number | null, fallback: number | null) => selected != null && next.some((version) => version.id === selected) ? selected : fallback;
      const nextBefore = chooseListed(selectedIds.current.before, next[0]?.id ?? null);
      const nextAfter = chooseListed(selectedIds.current.after, next.length > 1 ? next[next.length - 1].id : next[0]?.id ?? null);
      if (nextBefore !== selectedIds.current.before) { selectedIds.current.before = nextBefore; setBefore(null); setBeforeId(nextBefore); }
      if (nextAfter !== selectedIds.current.after) { selectedIds.current.after = nextAfter; setAfter(null); setAfterId(nextAfter); }
      setError(null);
    } catch (cause) {
      if (sessionAtStart === sessionGeneration.current && listAtStart === listGeneration.current) setError(String(cause));
    } finally {
      if (sessionAtStart === sessionGeneration.current && listAtStart === listGeneration.current) setLoading(false);
    }
  }, [taskId]);

  useEffect(() => {
    sessionGeneration.current++;
    listGeneration.current++;
    snapshotGeneration.current.before++;
    snapshotGeneration.current.after++;
    writeGeneration.current++;
    selectedIds.current = { before: null, after: null };
    initialRevision.current = resultRevision;
    setPath(paths[0] ?? "");
    setVersions([]); setBeforeId(null); setAfterId(null); setBefore(null); setAfter(null); setError(null); setSaving(false);
    void reload();
    return () => { sessionGeneration.current++; listGeneration.current++; snapshotGeneration.current.before++; snapshotGeneration.current.after++; writeGeneration.current++; };
  }, [taskId, sourceId, reload]);

  useEffect(() => {
    if (initialRevision.current === resultRevision) return;
    initialRevision.current = resultRevision;
    void reload();
  }, [resultRevision, reload]);
  useEffect(() => { setPath((current) => paths.includes(current) ? current : (paths[0] ?? "")); }, [pathsKey]);

  const loadSnapshot = useCallback((artifactId: number | null, side: "before" | "after") => {
    if (artifactId == null) { side === "before" ? setBefore(null) : setAfter(null); return; }
    const sessionAtStart = sessionGeneration.current;
    const loadAtStart = ++snapshotGeneration.current[side];
    void reviewArtifactGet(taskId, artifactId).then((snapshot) => {
      if (sessionAtStart !== sessionGeneration.current || loadAtStart !== snapshotGeneration.current[side]) return;
      (side === "before" ? setBefore : setAfter)(snapshot);
    }).catch((cause) => {
      if (sessionAtStart === sessionGeneration.current && loadAtStart === snapshotGeneration.current[side]) setError(String(cause));
    });
  }, [taskId]);

  useEffect(() => { loadSnapshot(beforeId, "before"); }, [beforeId, loadSnapshot]);
  useEffect(() => { loadSnapshot(afterId, "after"); }, [afterId, loadSnapshot]);

  const chooseSnapshot = (side: "before" | "after", artifactId: number | null) => {
    if (selectedIds.current[side] === artifactId) return;
    snapshotGeneration.current[side]++;
    selectedIds.current[side] = artifactId;
    if (side === "before") { setBefore(null); setBeforeId(artifactId); }
    else { setAfter(null); setAfterId(artifactId); }
  };

  const save = async () => {
    if (!path || disabled || saving) return;
    const writeAtStart = ++writeGeneration.current;
    const sessionAtStart = sessionGeneration.current;
    setSaving(true); setError(null);
    try {
      const input = await prepareArtifactSave({ taskId, sourceId, resultRevision, relativePath: path });
      const snapshot = await reviewArtifactSave(taskId, input);
      if (sessionAtStart !== sessionGeneration.current || writeAtStart !== writeGeneration.current) return;
      // A list request started before this write cannot erase the just-saved immutable version.
      listGeneration.current++;
      setLoading(false);
      setVersions((current) => [...current, snapshot].sort((left, right) => left.id - right.id));
      if (selectedIds.current.before == null) { selectedIds.current.before = snapshot.id; setBefore(snapshot); setBeforeId(snapshot.id); }
      selectedIds.current.after = snapshot.id; setAfter(snapshot); setAfterId(snapshot.id);
    } catch (cause) {
      if (sessionAtStart === sessionGeneration.current && writeAtStart === writeGeneration.current) setError(String(cause));
    } finally {
      if (sessionAtStart === sessionGeneration.current && writeAtStart === writeGeneration.current) setSaving(false);
    }
  };

  const attach = () => {
    if (!before || !after || before.id !== beforeId || after.id !== afterId || disabled) return;
    onAttachReview(artifactReviewContext(before, after));
  };

  const versionSelect = (label: string, value: number | null, side: "before" | "after") => <label className="flex min-w-0 flex-col gap-1 text-sm">
    {label}
    <select aria-label={label} value={value ?? ""} disabled={disabled || loading || versions.length === 0} onChange={(event) => chooseSnapshot(side, event.target.value ? Number(event.target.value) : null)} className="bg-bg border border-border px-2 py-1">
      {versions.length === 0 ? <option value="">저장 버전 없음</option> : versions.map((version) => <option key={version.id} value={version.id}>{versionLabel(version)}</option>)}
    </select>
  </label>;

  return <section aria-label="산출물 버전 비교" className="border-t border-border pt-3 mt-3">
    <h3 className="font-medium">산출물 버전 비교</h3>
    <p className="text-sm text-text-muted">저장된 버전은 현재 파일이 바뀌거나 삭제된 뒤에도 그대로 표시됩니다.</p>
    <div className="flex flex-wrap gap-2 mt-2">
      <select aria-label="저장할 산출물" value={path} disabled={disabled || saving || paths.length === 0} onChange={(event) => setPath(event.target.value)} className="min-w-48 bg-bg border border-border px-2 py-1">
        {paths.length === 0 ? <option value="">저장할 산출물이 없습니다</option> : paths.map((candidate) => <option key={candidate} value={candidate}>{candidate}</option>)}
      </select>
      <button disabled={disabled || saving || !path} onClick={() => void save()}>검토 버전 저장</button>
      <button disabled={disabled || !path} onClick={() => onOpenFile(path)}>현재 원본 열기</button>
    </div>
    {error && <p role="status" className="mt-2 text-sm text-status-failed">{error}</p>}
    {loading && <p role="status" className="mt-2 text-sm text-text-muted">저장 버전을 불러오는 중입니다.</p>}
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-3 mt-3">
      {versionSelect("이전 버전", beforeId, "before")}
      {versionSelect("이후 버전", afterId, "after")}
      <SnapshotPane label="이전" snapshot={before} />
      <SnapshotPane label="이후" snapshot={after} />
    </div>
    <button className="mt-3" disabled={disabled || !before || !after || before.id !== beforeId || after.id !== afterId} onClick={attach}>비교 대상을 검토 사유에 연결</button>
  </section>;
}
