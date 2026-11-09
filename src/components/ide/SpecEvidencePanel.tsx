import { useCallback, useEffect, useRef, useState } from "react";
import {
  taskSpecGet,
  taskSpecList,
  taskSpecRead,
  taskSpecReference,
  taskSpecSave,
  taskSpecSearch,
  taskSpecValidateLinks,
  type SpecDocument,
  type SpecHit,
  type SpecLinkReport,
  type SpecSnapshot,
  type SpecVersion,
} from "../../lib/task-specs";

interface Props {
  taskId: number;
  disabled?: boolean;
  onOpenFile: (path: string) => void;
  onAttachInput?: (text: string) => void;
  onAttachReview: (text: string) => void;
}

const messageOf = (error: unknown) => error instanceof Error ? error.message : String(error);
const queryBytes = (query: string) => new TextEncoder().encode(query).byteLength;
const queryLength = (query: string) => Array.from(query).length;
const versionLabel = (version: Pick<SpecVersion, "created_at" | "content_hash" | "id" | "relative_path">) =>
  `${version.relative_path} · ${new Date(version.created_at * 1000).toLocaleString("ko-KR")} · ${version.content_hash.slice(0, 12)}… · #${version.id}`;
const linkStateLabel = {
  ok: "확인됨",
  missing: "파일 없음",
  outside_scope: "작업 폴더 밖",
  unsupported: "지원하지 않음",
  anchor_missing: "제목 위치 없음",
  unreadable: "읽을 수 없음",
  external: "외부 링크",
} as const;

/**
 * 검색 결과는 현재 파일이고, 보존 버전은 DB에 저장된 불변 본문이다.
 * 두 본문을 같은 상태에 넣지 않아 파일이 바뀐 뒤에도 보존 원문을 혼동하지 않는다.
 */
export function SpecEvidencePanel({ taskId, disabled = false, onOpenFile, onAttachInput, onAttachReview }: Props) {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SpecHit[]>([]);
  const [searchInfo, setSearchInfo] = useState<{ truncated: boolean; scanned: number; skipped: number } | null>(null);
  const [searching, setSearching] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [currentDocument, setCurrentDocument] = useState<SpecDocument | null>(null);
  const [reading, setReading] = useState(false);
  const [readError, setReadError] = useState<string | null>(null);
  const [versions, setVersions] = useState<SpecVersion[]>([]);
  const [listing, setListing] = useState(false);
  const [listError, setListError] = useState<string | null>(null);
  const [selectedSnapshotId, setSelectedSnapshotId] = useState<number | null>(null);
  const [snapshot, setSnapshot] = useState<SpecSnapshot | null>(null);
  const [loadingSnapshot, setLoadingSnapshot] = useState(false);
  const [snapshotError, setSnapshotError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [linkReport, setLinkReport] = useState<SpecLinkReport | null>(null);
  const [validatingLinks, setValidatingLinks] = useState(false);
  const [linkError, setLinkError] = useState<string | null>(null);
  const [attachError, setAttachError] = useState<string | null>(null);
  const scope = useRef(0);
  const searchGeneration = useRef(0);
  const readGeneration = useRef(0);
  const listGeneration = useRef(0);
  const snapshotGeneration = useRef(0);
  const saveGeneration = useRef(0);
  const linkGeneration = useRef(0);
  const currentDocumentRef = useRef<SpecDocument | null>(null);

  const replaceCurrentDocument = (document: SpecDocument | null) => {
    currentDocumentRef.current = document;
    setCurrentDocument(document);
    linkGeneration.current += 1;
    setLinkReport(null);
    setValidatingLinks(false);
    setLinkError(null);
  };

  const refreshVersions = useCallback(async (id: number, requestScope = scope.current) => {
    const generation = ++listGeneration.current;
    setListing(true);
    setListError(null);
    try {
      const next = await taskSpecList(id);
      if (requestScope !== scope.current || generation !== listGeneration.current) return;
      setVersions(next);
      setSelectedSnapshotId((selected) => {
        if (selected === null || next.some((version) => version.id === selected)) return selected;
        snapshotGeneration.current += 1;
        setSnapshot(null);
        setSnapshotError(null);
        return null;
      });
    } catch (error) {
      if (requestScope === scope.current && generation === listGeneration.current) setListError(messageOf(error));
    } finally {
      if (requestScope === scope.current && generation === listGeneration.current) setListing(false);
    }
  }, []);

  useEffect(() => {
    const requestScope = ++scope.current;
    searchGeneration.current += 1;
    readGeneration.current += 1;
    snapshotGeneration.current += 1;
    saveGeneration.current += 1;
    linkGeneration.current += 1;
    setQuery("");
    setHits([]);
    setSearchInfo(null);
    setSearching(false);
    setSearchError(null);
    replaceCurrentDocument(null);
    setReading(false);
    setReadError(null);
    setVersions([]);
    setSelectedSnapshotId(null);
    setSnapshot(null);
    setLoadingSnapshot(false);
    setSnapshotError(null);
    setSaving(false);
    setSaveError(null);
    setLinkReport(null);
    setValidatingLinks(false);
    setLinkError(null);
    setAttachError(null);
    void refreshVersions(taskId, requestScope);
    return () => { scope.current += 1; };
  }, [taskId, refreshVersions]);

  const search = async () => {
    const normalized = query.trim();
    if (queryLength(normalized) < 2 || queryBytes(normalized) > 200) return;
    const requestScope = scope.current;
    const generation = ++searchGeneration.current;
    readGeneration.current += 1;
    setReading(false);
    replaceCurrentDocument(null);
    setReadError(null);
    setSelectedSnapshotId(null);
    snapshotGeneration.current += 1;
    setLoadingSnapshot(false);
    setSnapshot(null);
    setSnapshotError(null);
    setHits([]);
    setSearchInfo(null);
    setSearching(true);
    setSearchError(null);
    try {
      const page = await taskSpecSearch(taskId, normalized);
      if (requestScope !== scope.current || generation !== searchGeneration.current) return;
      setHits(page.hits);
      setSearchInfo({ truncated: page.truncated, scanned: page.scanned_files, skipped: page.skipped_files });
    } catch (error) {
      if (requestScope === scope.current && generation === searchGeneration.current) setSearchError(messageOf(error));
    } finally {
      if (requestScope === scope.current && generation === searchGeneration.current) setSearching(false);
    }
  };

  const readCurrent = async (hit: SpecHit) => {
    const requestScope = scope.current;
    const generation = ++readGeneration.current;
    snapshotGeneration.current += 1;
    setLoadingSnapshot(false);
    setSelectedSnapshotId(null);
    setSnapshot(null);
    setSnapshotError(null);
    replaceCurrentDocument(null);
    setReading(true);
    setReadError(null);
    try {
      const document = await taskSpecRead(taskId, hit.relative_path);
      if (requestScope !== scope.current || generation !== readGeneration.current) return;
      replaceCurrentDocument(document);
    } catch (error) {
      if (requestScope === scope.current && generation === readGeneration.current) setReadError(messageOf(error));
    } finally {
      if (requestScope === scope.current && generation === readGeneration.current) setReading(false);
    }
  };

  const readSnapshot = async (version: SpecVersion) => {
    const requestScope = scope.current;
    const generation = ++snapshotGeneration.current;
    readGeneration.current += 1;
    setReading(false);
    replaceCurrentDocument(null);
    setReadError(null);
    setSelectedSnapshotId(version.id);
    setSnapshot(null);
    setLoadingSnapshot(true);
    setSnapshotError(null);
    try {
      const next = await taskSpecGet(taskId, version.id);
      if (requestScope !== scope.current || generation !== snapshotGeneration.current) return;
      setSnapshot(next);
    } catch (error) {
      if (requestScope === scope.current && generation === snapshotGeneration.current) setSnapshotError(messageOf(error));
    } finally {
      if (requestScope === scope.current && generation === snapshotGeneration.current) setLoadingSnapshot(false);
    }
  };

  const saveCurrent = async () => {
    const document = currentDocumentRef.current;
    if (disabled || !document || saving) return;
    const requestScope = scope.current;
    const generation = ++saveGeneration.current;
    setSaving(true);
    setSaveError(null);
    try {
      const saved = await taskSpecSave(taskId, { relative_path: document.relative_path, expected_hash: document.content_hash });
      if (requestScope !== scope.current || generation !== saveGeneration.current) return;
      listGeneration.current += 1;
      setVersions((current) => [saved, ...current.filter((version) => version.id !== saved.id)]);
      if (currentDocumentRef.current?.content_hash === document.content_hash && currentDocumentRef.current?.relative_path === document.relative_path) {
        setSelectedSnapshotId(saved.id);
        snapshotGeneration.current += 1;
        setSnapshot(saved);
        replaceCurrentDocument(null);
      }
      void refreshVersions(taskId, requestScope);
    } catch (error) {
      if (requestScope === scope.current && generation === saveGeneration.current) setSaveError(messageOf(error));
    } finally {
      if (requestScope === scope.current && generation === saveGeneration.current) setSaving(false);
    }
  };

  const validateLinks = async () => {
    const document = currentDocumentRef.current;
    if (disabled || !document || reading || saving || validatingLinks) return;
    const requestScope = scope.current;
    const generation = ++linkGeneration.current;
    setValidatingLinks(true);
    setLinkError(null);
    setLinkReport(null);
    try {
      const report = await taskSpecValidateLinks(taskId, document.relative_path, document.content_hash);
      const current = currentDocumentRef.current;
      if (requestScope !== scope.current || generation !== linkGeneration.current || current?.relative_path !== document.relative_path || current.content_hash !== document.content_hash) return;
      setLinkReport(report);
    } catch (error) {
      if (requestScope === scope.current && generation === linkGeneration.current) setLinkError(messageOf(error));
    } finally {
      if (requestScope === scope.current && generation === linkGeneration.current) setValidatingLinks(false);
    }
  };

  const attach = (target: "input" | "review") => {
    if (disabled || !snapshot) return;
    try {
      const reference = taskSpecReference(snapshot);
      if (target === "input") onAttachInput?.(reference);
      else onAttachReview(reference);
      setAttachError(null);
    } catch (error) {
      setAttachError(messageOf(error));
    }
  };

  const canSearch = queryLength(query.trim()) >= 2 && queryBytes(query.trim()) <= 200 && !searching;
  const selectedVersion = selectedSnapshotId === null ? null : versions.find((version) => version.id === selectedSnapshotId) ?? null;

  return <section aria-label="명세 근거" className="border-t border-border bg-surface p-3 text-sm">
    <header className="flex flex-wrap items-center gap-2">
      <div className="mr-auto"><h3 className="font-medium">명세 근거</h3><p className="text-text-muted">작업 워크트리의 Markdown을 키워드로 찾습니다. 검색 결과는 키워드 일치 후보이며 관련성을 보장하지 않습니다.</p><p className="text-xs text-text-muted">문서는 최대 64KiB까지 보존합니다. 입력은 보존 명세 4개와 확장 뒤 128KiB까지 연결할 수 있습니다. 보존·입력 연결만으로 전송되지 않습니다. 기존 전송 버튼을 누르면 저장 원문을 전달합니다.</p></div>
    </header>
    <div className="mt-2 flex flex-wrap items-center gap-2">
      <input aria-label="명세 검색어" value={query} onChange={(event) => setQuery(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") void search(); }} placeholder="제목과 본문 검색" className="bg-bg border border-border px-2" />
      <button disabled={!canSearch} onClick={() => void search()}>명세 검색</button>
      <span className="text-xs text-text-muted">2글자 이상, 200바이트 이하</span>
    </div>
    {searching && <p aria-live="polite">명세를 검색하는 중입니다.</p>}
    {searchError && <p role="alert" className="text-status-failed">명세 검색 실패: {searchError} <button onClick={() => void search()}>다시 시도</button></p>}
    {searchInfo && <p className="text-xs text-text-muted">탐색 {searchInfo.scanned}개 · 건너뜀 {searchInfo.skipped}개{searchInfo.truncated ? " · 시간 또는 문서 제한으로 일부만 표시" : ""}</p>}
    {hits.length > 0 && <div className="mt-2 border border-border"><p className="px-2 py-1 text-xs text-text-muted">키워드 일치 후보</p>{hits.map((hit) => <button key={`${hit.relative_path}:${hit.line}`} className="block w-full border-t border-border p-2 text-left hover:bg-raised" onClick={() => void readCurrent(hit)}><span className="block text-primary-bright">{hit.title || hit.relative_path}</span><span className="block text-xs text-text-muted">{hit.relative_path}:{hit.line}</span>{hit.snippet && <span className="block mt-1 text-xs text-text-secondary">{hit.snippet}</span>}</button>)}</div>}
    <div className="mt-3 grid gap-3 lg:grid-cols-2">
      <section className="border border-border" aria-label="현재 문서">
        <div className="flex items-center gap-2 border-b border-border px-2 py-1"><h4 className="mr-auto font-medium">현재 문서</h4>{currentDocument && <><button disabled={disabled || saving || reading || validatingLinks} onClick={() => void saveCurrent()}>{saving ? "보존 중입니다." : "명세 버전 보존"}</button><button disabled={disabled || saving || reading || validatingLinks} onClick={() => void validateLinks()}>{validatingLinks ? "문서 링크 검사 중입니다." : "문서 링크 검사"}</button><button onClick={() => onOpenFile(currentDocument.relative_path)}>현재 문서 열기</button></>}</div>
        {reading ? <p className="p-2" aria-live="polite">현재 문서를 불러오는 중입니다.</p> : readError ? <p role="alert" className="p-2 text-status-failed">현재 문서 조회 실패: {readError}</p> : currentDocument ? <><p className="p-2 text-xs text-text-muted">{currentDocument.relative_path} · SHA-256 {currentDocument.content_hash} · {currentDocument.byte_length}바이트</p><pre className="max-h-80 overflow-auto whitespace-pre-wrap break-words border-t border-border p-2">{currentDocument.content}</pre></> : <p className="p-2 text-text-muted">검색 결과를 선택하면 현재 원문을 확인합니다.</p>}
        {validatingLinks && <p className="p-2" aria-live="polite">현재 워크트리의 문서 링크를 검사하는 중입니다.</p>}
        {linkError && <p role="alert" className="p-2 text-status-failed">문서 링크 검사 실패: {linkError} <button disabled={disabled || saving || reading || validatingLinks} onClick={() => void validateLinks()}>다시 시도</button></p>}
        {linkReport && <div className="border-t border-border p-2" aria-label="문서 링크 검사 결과"><p className="text-xs text-text-muted">현재 워크트리 원문 · SHA-256 {linkReport.content_hash} · {new Date(linkReport.checked_at * 1000).toLocaleString("ko-KR")}</p><p className="mt-1 text-xs">검사 {linkReport.summary.total}개 · 확인 {linkReport.summary.ok}개 · 파일 없음 {linkReport.summary.missing}개 · 제목 위치 없음 {linkReport.summary.anchor_missing}개 · 작업 폴더 밖 {linkReport.summary.outside_scope}개 · 읽을 수 없음 {linkReport.summary.unreadable}개 · 지원하지 않음 {linkReport.summary.unsupported}개</p><p className="mt-1 text-xs text-text-muted">외부 링크 {linkReport.summary.external}개는 네트워크로 확인하지 않았습니다.</p>{linkReport.truncated && <p className="mt-1 text-xs text-text-muted">링크 수 제한으로 일부만 검사했습니다.</p>}<div className="mt-2 max-h-40 overflow-auto border border-border">{linkReport.links.map((link, index) => <p key={`${link.source_line}:${link.target}:${index}`} className="border-b border-border px-2 py-1 text-xs"><span>{link.source_line}행 · {link.target || "문서 안 위치"} · {linkStateLabel[link.state]}</span>{link.resolved_path && <span className="text-text-muted"> · {link.resolved_path}</span>}</p>)}</div></div>}
        {saveError && <p role="alert" className="p-2 text-status-failed">명세 보존 실패: {saveError} <button disabled={disabled || saving} onClick={() => void saveCurrent()}>다시 시도</button></p>}
      </section>
      <section className="border border-border" aria-label="보존 명세">
        <div className="flex items-center gap-2 border-b border-border px-2 py-1"><h4 className="mr-auto font-medium">보존 명세</h4><button onClick={() => void refreshVersions(taskId)}>새로고침</button></div>
        {listing ? <p className="p-2" aria-live="polite">보존 기록을 불러오는 중입니다.</p> : listError ? <p role="alert" className="p-2 text-status-failed">보존 명세 목록 조회 실패: {listError} <button onClick={() => void refreshVersions(taskId)}>다시 시도</button></p> : versions.length === 0 ? <p className="p-2 text-text-muted">보존한 명세가 없습니다.</p> : <div className="max-h-40 overflow-auto">{versions.map((version) => <button key={version.id} className={`block w-full border-b border-border p-2 text-left ${selectedSnapshotId === version.id ? "bg-raised" : "hover:bg-raised"}`} onClick={() => void readSnapshot(version)}>{versionLabel(version)}</button>)}</div>}
        {loadingSnapshot ? <p className="p-2" aria-live="polite">보존 원문을 불러오는 중입니다.</p> : snapshotError ? <p role="alert" className="p-2 text-status-failed">보존 원문 조회 실패: {snapshotError}{selectedVersion && <button onClick={() => void readSnapshot(selectedVersion)}>다시 시도</button>}</p> : snapshot ? <><p className="p-2 text-xs text-text-muted">보존 원문 · {snapshot.relative_path} · SHA-256 {snapshot.content_hash} · {snapshot.byte_length}바이트 · {new Date(snapshot.created_at * 1000).toLocaleString("ko-KR")}</p><pre className="max-h-80 overflow-auto whitespace-pre-wrap break-words border-t border-border p-2">{snapshot.content}</pre><div className="flex flex-wrap gap-2 border-t border-border p-2"><button disabled={disabled || !onAttachInput} onClick={() => attach("input")}>작업 입력에 연결</button><button disabled={disabled} onClick={() => attach("review")}>검토 사유에 연결</button></div></> : <p className="p-2 text-text-muted">보존 기록을 선택하면 저장 당시 원문을 확인합니다.</p>}
        {attachError && <p role="alert" className="p-2 text-status-failed">명세 연결 실패: {attachError}</p>}
      </section>
    </div>
  </section>;
}
