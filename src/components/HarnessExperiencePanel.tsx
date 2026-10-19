import { useEffect, useRef, useState } from "react";
import { copyText } from "../lib/clipboard";
import { useHostScope } from "../lib/host-scope";
import type { HostId } from "../lib/transport";
import {
  loadExperiences,
  readExperience,
  type DocumentDescriptor,
  type DocumentOwner,
  type ExperienceDocument,
  type HarnessName,
  type ListResult,
  type ProjectListing,
  type SourceListing,
} from "../lib/harness-experience";
import {
  emptyExperienceDraft,
  experienceDraft,
  type ExperienceDraftFields,
} from "./harness-experience-draft";

interface Props {
  repo: string;
  onOpenMemory: () => void;
}
type PickedDocument = {
  host: HostId;
  repo: string;
  harness: HarnessName;
  owner: DocumentOwner;
  descriptor: DocumentDescriptor;
};
const HARNESSES: { value: HarnessName; label: string }[] = [
  { value: "workflow-harness", label: "workflow-harness" },
  { value: "loop-engineering", label: "loop-engineering" },
];

function errorText(error: string): string {
  const labels: Record<string, string> = {
    "not-found": "파일이 없거나 삭제되었습니다.",
    "permission-denied": "파일을 읽을 권한이 없습니다.",
    "unsafe-path": "심볼릭 링크나 허용되지 않은 경로는 읽을 수 없습니다.",
    "not-regular": "일반 파일만 읽을 수 있습니다.",
    oversized: "128KiB를 넘는 파일입니다. 원래 에디터에서 확인하세요.",
    "invalid-utf8": "UTF-8 문서만 읽을 수 있습니다.",
    changed: "읽는 동안 원문이 변경되었습니다. 다시 확인하세요.",
    "io-error": "파일 읽기 오류가 발생했습니다.",
    "invalid-request": "조회할 원본과 프로젝트를 다시 선택하세요.",
    "unregistered-project": "이 컴퓨터에서 작업 이력이 있는 프로젝트를 선택하세요",
  };
  return labels[error] ?? error;
}

function listingText(listing: SourceListing | ProjectListing): string {
  if (listing.state === "ready")
    return listing.limited
      ? `일부 표시 · ${listing.inspectedEntries}개 확인`
      : `${listing.documents.length}개 문서`;
  if (listing.state === "error") return `조회 실패: ${errorText(listing.error)}`;
  return listing.state === "not-installed" ? "설치되지 않음" : "문서 없음";
}

function sourceKey(source: SourceListing): string {
  return `${source.source.vendor}-${source.source.scope}`;
}

function SourceList({
  result,
  page,
  source,
  onPage,
  onSource,
  onPick,
}: {
  result: ListResult;
  page: number;
  source: string;
  onPage: (page: number) => void;
  onSource: (source: string) => void;
  onPick: (owner: DocumentOwner, document: DocumentDescriptor) => void;
}) {
  if (result.state !== "ready") return <Status result={result} />;
  const sources = result.sources;
  const ready = sources.flatMap((source) =>
    source.state === "ready"
      ? source.documents.map((document) => ({
          owner: { kind: "skill", source: source.source } as DocumentOwner,
          document,
        }))
      : [],
  );
  const project =
    result.project.state === "ready"
      ? result.project.documents.map((document) => ({
          owner: {
            kind: "project",
            project: result.project.project,
          } as DocumentOwner,
          document,
        }))
      : [];
  const documents =
    source === "project"
      ? project
      : ready.filter(
          (entry) =>
            entry.owner.kind === "skill" &&
            `${entry.owner.source.vendor}-${entry.owner.source.scope}` ===
              source,
        );
  const pages = Math.max(1, Math.ceil(documents.length / 20));
  const rows = documents.slice(page * 20, page * 20 + 20);
  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap gap-1" role="group" aria-label="원본 선택">
        {sources.map((entry) => (
          <button
            key={sourceKey(entry)}
            aria-pressed={source === sourceKey(entry)}
            className={`rounded px-2 py-1 text-xs ${source === sourceKey(entry) ? "bg-primary text-bg" : "border border-border text-text-secondary"}`}
            onClick={() => onSource(sourceKey(entry))}
          >
            {entry.source.vendor} · {entry.source.scope}
          </button>
        ))}
        <button
          aria-pressed={source === "project"}
          className={`rounded px-2 py-1 text-xs ${source === "project" ? "bg-primary text-bg" : "border border-border text-text-secondary"}`}
          onClick={() => onSource("project")}
        >
          프로젝트 참고
        </button>
      </div>
      {sources.map((entry) => (
        <div key={sourceKey(entry)} className="text-xs text-text-muted">
          {entry.source.vendor} · {entry.source.scope} · {listingText(entry)}
        </div>
      ))}
      <div className="text-xs text-text-muted">
        프로젝트 참고 자료 · {listingText(result.project)}
      </div>
      {rows.map(({ owner, document }) => (
        <button
          key={`${owner.kind}-${document.key}-${document.displayPath}`}
          className="break-all rounded border border-border bg-bg px-2 py-1 text-left text-xs text-text-secondary hover:border-border-strong"
          onClick={() => onPick(owner, document)}
        >
          {document.displayPath}
        </button>
      ))}
      {pages > 1 && (
        <div className="flex gap-2 text-xs">
          <button disabled={page === 0} onClick={() => onPage(page - 1)}>
            이전
          </button>
          <span>
            {page + 1} / {pages}
          </span>
          <button
            disabled={page + 1 === pages}
            onClick={() => onPage(page + 1)}
          >
            다음
          </button>
        </div>
      )}
    </div>
  );
}

function Status({
  result,
}: {
  result: Exclude<ListResult, { state: "ready" }>;
}) {
  if (result.state === "unsupported")
    return (
      <div className="text-xs text-text-muted">
        {result.reason === "remote-host"
          ? "이 경험 조회는 이 컴퓨터의 로컬 프로젝트에서만 지원됩니다."
          : "이 운영체제에서는 안전한 경험 파일 조회를 아직 지원하지 않습니다."}
      </div>
    );
  return (
    <div className="text-xs text-status-failed">
      {errorText(result.error)}
    </div>
  );
}

function DraftEditor({
  document,
  fields,
  onChange,
  onCopy,
  copyState,
  canCopy,
}: {
  document: ExperienceDocument;
  fields: ExperienceDraftFields;
  onChange: (fields: ExperienceDraftFields) => void;
  onCopy: () => void;
  copyState: string | null;
  canCopy: boolean;
}) {
  const edit = (key: keyof ExperienceDraftFields, value: string) =>
    onChange({ ...fields, [key]: value });
  const preview = experienceDraft(document, fields);
  return (
    <div className="mt-3 border-t border-border pt-3 flex flex-col gap-2">
      <label className="text-xs text-text-muted">
        선택 구절
        <textarea
          className="mt-1 w-full bg-bg border border-border rounded p-2 text-xs text-text"
          value={fields.excerpt}
          onChange={(e) => edit("excerpt", e.target.value)}
        />
      </label>
      {(["conditions", "exceptions", "proposal", "verification"] as const).map(
        (key) => (
          <label key={key} className="text-xs text-text-muted">
            {
              {
                conditions: "적용 조건",
                exceptions: "예외·반례",
                proposal: "제안",
                verification: "확인 방법",
              }[key]
            }
            <input
              className="mt-1 w-full bg-bg border border-border rounded px-2 py-1 text-xs text-text"
              value={fields[key]}
              onChange={(e) => edit(key, e.target.value)}
            />
          </label>
        ),
      )}
      <pre className="max-h-48 overflow-auto whitespace-pre-wrap rounded border border-border bg-bg p-2 text-xs font-code text-text-secondary">
        {preview}
      </pre>
      <button
        className="h-7 rounded border border-border text-xs text-text-secondary hover:border-border-strong"
        disabled={!canCopy || copyState === "변경 확인 중…"}
        onClick={onCopy}
      >
        초안 복사
      </button>
      {copyState && <div className="text-xs text-text-muted">{copyState}</div>}
    </div>
  );
}

export function HarnessExperiencePanel({ repo, onOpenMemory }: Props) {
  const host = useHostScope();
  const [harness, setHarness] = useState<HarnessName>("workflow-harness");
  const [result, setResult] = useState<ListResult | null>(null);
  const [resultScope, setResultScope] = useState({ host, repo, harness });
  const [page, setPage] = useState(0);
  const [source, setSource] = useState("claude-project");
  const [picked, setPicked] = useState<PickedDocument | null>(null);
  const [document, setDocument] = useState<ExperienceDocument | null>(null);
  const [fields, setFields] = useState(emptyExperienceDraft);
  const [copyState, setCopyState] = useState<string | null>(null);
  const [readError, setReadError] = useState<string | null>(null);
  const request = useRef(0);
  const copyInFlight = useRef(false);
  const canCopy =
    typeof navigator !== "undefined" && !!navigator.clipboard?.writeText;
  const clearDocument = () => {
    setPicked(null);
    setDocument(null);
    setFields(emptyExperienceDraft());
    setCopyState(null);
    setReadError(null);
  };
  const resetList = () => {
    setPage(0);
    clearDocument();
  };
  useEffect(() => {
    const id = ++request.current;
    resetList();
    setResult(null);
    setResultScope({ host, repo, harness });
    void loadExperiences(host, repo, harness)
      .then((next) => {
        if (id === request.current) setResult(next);
      })
      .catch((error) => {
        if (id === request.current)
          setResult({
            state: "error",
            error: String(error) as "invalid-request",
          });
      });
  }, [host, repo, harness]);
  useEffect(
    () => () => {
      request.current += 1;
    },
    [],
  );
  const pick = (owner: DocumentOwner, descriptor: DocumentDescriptor) => {
    const id = ++request.current;
    clearDocument();
    const next = { host, repo, harness, owner, descriptor };
    setPicked(next);
    void readExperience(host, repo, owner, descriptor.key)
      .then((read) => {
        if (id === request.current && read.state === "ready")
          setDocument(read.document);
        if (id === request.current && read.state !== "ready")
          setReadError(
            read.state === "error"
              ? `읽기 실패: ${errorText(read.error)}`
              : `읽기 미지원: ${read.reason}`,
          );
      })
      .catch(() => {
        if (id === request.current) setReadError("문서를 읽지 못했습니다.");
      });
  };
  const copy = async () => {
    if (!picked || !document || !canCopy || copyInFlight.current) return;
    const excerpt = fields.excerpt.trim();
    if (!excerpt || !document.text.includes(excerpt)) {
      setCopyState("원문에서 선택한 구절을 다시 선택하세요.");
      return;
    }
    const id = request.current;
    copyInFlight.current = true;
    setCopyState("변경 확인 중…");
    try {
      const read = await readExperience(
        host,
        repo,
        picked.owner,
        picked.descriptor.key,
      );
      if (id !== request.current) return;
      if (read.state !== "ready") {
        setCopyState(
          read.state === "error"
            ? `원문을 다시 읽지 못했습니다: ${errorText(read.error)}`
            : `원문을 다시 읽을 수 없습니다: ${read.reason}`,
        );
        return;
      }
      if (read.document.contentHash !== document.contentHash) {
        setCopyState("원문이 변경되었습니다. 다시 확인한 뒤 복사하세요.");
        return;
      }
      await copyText(experienceDraft(document, fields));
      if (id === request.current) setCopyState("복사됨");
    } catch {
      if (id === request.current)
        setCopyState("클립보드에 복사하지 못했습니다.");
    } finally {
      copyInFlight.current = false;
    }
  };
  const copyPath = async () => {
    if (!document || !canCopy) return;
    const id = request.current;
    try {
      await copyText(document.path);
      if (id === request.current) setCopyState("경로 복사됨");
    } catch {
      if (id === request.current) setCopyState("경로를 복사하지 못했습니다.");
    }
  };
  const refresh = () => {
    const id = ++request.current;
    resetList();
    setResult(null);
    setResultScope({ host, repo, harness });
    void loadExperiences(host, repo, harness)
      .then((next) => {
        if (id === request.current) setResult(next);
      })
      .catch((error) => {
        if (id === request.current)
          setResult({
            state: "error",
            error: String(error) as "invalid-request",
          });
      });
  };
  const currentDocument =
    document &&
    picked?.host === host &&
    picked.repo === repo &&
    picked.harness === harness
      ? document
      : null;
  const currentResult =
    resultScope.host === host &&
    resultScope.repo === repo &&
    resultScope.harness === harness
      ? result
      : null;
  return (
    <section className="mt-4 rounded-lg border border-border bg-surface p-3">
      <div className="flex items-center justify-between gap-2">
        <div>
          <div className="text-sm text-text">하네스 경험</div>
          <div className="text-xs text-text-muted">
            원문은 읽기 전용이며 개선 초안만 복사합니다.
          </div>
        </div>
        <button className="text-xs text-primary-bright" onClick={refresh}>
          새로고침
        </button>
        <button className="text-xs text-primary-bright" onClick={onOpenMemory}>
          메모리 검토
        </button>
      </div>
      <div className="mt-3 flex gap-1" role="group" aria-label="하네스 선택">
        {HARNESSES.map((entry) => (
          <button
            key={entry.value}
            className={`rounded px-2 py-1 text-xs ${harness === entry.value ? "bg-primary text-bg" : "border border-border text-text-secondary"}`}
            aria-pressed={harness === entry.value}
            onClick={() => setHarness(entry.value)}
          >
            {entry.label}
          </button>
        ))}
      </div>
      <div className="mt-3">
        {currentResult ? (
          <SourceList
            result={currentResult}
            page={page}
            source={source}
            onPage={setPage}
            onSource={(next) => {
              request.current += 1;
              setPage(0);
              setSource(next);
              clearDocument();
            }}
            onPick={pick}
          />
        ) : (
          <div className="text-xs text-text-muted">불러오는 중…</div>
        )}
      </div>
      {picked && !currentDocument && (
        <div className="mt-2 text-xs text-text-muted">
          {readError ?? "원문을 불러오는 중…"}
        </div>
      )}
      {currentDocument && (
        <div className="mt-3">
          <div className="break-all text-xs text-text-muted">
            {currentDocument.path} · sha256:{currentDocument.contentHash} ·{" "}
            {currentDocument.observedAt}
          </div>
          <div className="text-xs text-text-muted">
            {currentDocument.generated
              ? "생성물 · docs/memory, docs/archive, docs/lessons-seed.md에서 원천을 확인하세요. 파일별 증명은 아닙니다."
              : currentDocument.sourceResolution === "unconfirmed"
                ? "원천 미확인"
                : "일반 문서"}
          </div>
          <button
            className="text-xs text-primary-bright disabled:opacity-50"
            disabled={!canCopy}
            onClick={() => void copyPath()}
          >
            경로 복사
          </button>
          <pre
            onMouseUp={() => {
              const excerpt = window.getSelection()?.toString();
              if (excerpt) setFields((current) => ({ ...current, excerpt }));
            }}
            className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap rounded border border-border bg-bg p-2 text-xs font-code text-text-secondary"
          >
            {currentDocument.text}
          </pre>
          <DraftEditor
            document={currentDocument}
            fields={fields}
            onChange={setFields}
            onCopy={() => void copy()}
            copyState={copyState}
            canCopy={canCopy}
          />
        </div>
      )}
    </section>
  );
}
