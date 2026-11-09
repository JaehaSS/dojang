import { useEffect, useState } from "react";
import type { FileContent, FsNode } from "../lib/ipc";
import { api } from "./api";
import { Empty, MobileList, Row, Spinner } from "./primitives";
import { breadcrumbs, childrenAt, parentPath, previewNotice, ROOT } from "./tree";
import { Icon } from "../components/ide/icons";

// worktree 파일 보기 — 읽기 전용. (설계 0013 §10 M3)
//
// 데스크톱처럼 재귀 트리를 들여쓰기하지 않는다. 폰에서는 3단만 들어가도 가로가 남지 않아,
// 한 단계씩 들어가는 드릴다운으로 바꿨다. 쓰기는 scope=mobile이 백엔드에서 거부하므로
// UI에도 노출하지 않는다(이중).

function Breadcrumbs({ path, onNavigate }: { path: string; onNavigate: (next: string) => void }) {
  const crumbs = breadcrumbs(path);
  return (
    <div className="flex items-center gap-1 overflow-x-auto border-b border-border px-4 py-2 text-xs">
      {crumbs.map((crumb, index) => (
        <span key={crumb.path} className="flex shrink-0 items-center gap-1">
          {index > 0 ? <span className="text-text-muted">/</span> : null}
          <button
            type="button"
            onClick={() => onNavigate(crumb.path)}
            className={index === crumbs.length - 1 ? "text-text" : "text-text-muted"}
          >
            {crumb.name}
          </button>
        </span>
      ))}
    </div>
  );
}

function FileView({ content }: { content: FileContent }) {
  const notice = previewNotice(content.kind);
  if (notice) return <Empty>{notice}</Empty>;
  if (content.kind === "image") {
    return (
      <div className="p-4">
        <img src={content.content} alt="" className="max-w-full" />
      </div>
    );
  }
  const lines = content.content.split("\n");
  return (
    // 가로 스크롤은 이 안쪽에서만 일어나야 한다 — 페이지 전체가 흔들리면 읽을 수 없다.
    <div className="overflow-x-auto">
      <pre className="w-max min-w-full font-code text-xs leading-5">
        {lines.map((line, index) => (
          <div key={index} className="flex">
            <span className="sticky left-0 w-10 shrink-0 select-none bg-bg pr-2 text-right text-text-muted">
              {index + 1}
            </span>
            <span className="whitespace-pre text-text-secondary">{line || " "}</span>
          </div>
        ))}
      </pre>
    </div>
  );
}

export function FilesTab({ id }: { id: number }) {
  const [tree, setTree] = useState<FsNode[] | null>(null);
  const [path, setPath] = useState<string>(ROOT);
  const [file, setFile] = useState<FileContent | null>(null);
  const [loadingFile, setLoadingFile] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setError(null);
    api
      .fsTree(id)
      .then((nodes) => {
        if (!cancelled) setTree(nodes);
      })
      .catch((cause: unknown) => {
        if (!cancelled) setError(cause instanceof Error ? cause.message : String(cause));
      });
    return () => {
      cancelled = true;
    };
  }, [id]);

  const open = (node: FsNode) => {
    setPath(node.path);
    if (node.is_dir) {
      setFile(null);
      return;
    }
    setLoadingFile(true);
    setFile(null);
    api
      .fsRead(id, node.path)
      .then(setFile)
      .catch((cause: unknown) =>
        setError(cause instanceof Error ? cause.message : String(cause)),
      )
      .finally(() => setLoadingFile(false));
  };

  const navigate = (next: string) => {
    setPath(next);
    setFile(null);
    setError(null);
  };

  if (error) return <Empty>파일을 불러오지 못했습니다. {error}</Empty>;
  if (!tree) return <Spinner label="파일 목록을 불러오는 중" />;

  const entries = childrenAt(tree, path);
  const viewingFile = file !== null || loadingFile;

  return (
    <div>
      <Breadcrumbs path={path} onNavigate={navigate} />
      {viewingFile ? (
        <>
          {/* 파일을 열면 목록이 사라지므로 돌아갈 길을 명시한다. */}
          <button
            type="button"
            onClick={() => navigate(parentPath(path))}
            className="min-h-[44px] w-full border-b border-border px-4 text-left text-sm text-text-muted"
          >
            ← 상위 폴더
          </button>
          {loadingFile ? <Spinner label="파일을 여는 중" /> : file ? <FileView content={file} /> : null}
        </>
      ) : entries.length === 0 ? (
        <Empty>빈 폴더입니다.</Empty>
      ) : (
        <MobileList>
          {path !== ROOT ? (
            <Row ariaLabel="상위 폴더" onClick={() => navigate(parentPath(path))}>
              <span className="text-sm text-text-muted">← 상위 폴더</span>
            </Row>
          ) : null}
          {entries.map((node) => (
            <Row key={node.path} ariaLabel={node.name} onClick={() => open(node)}>
              <span className="inline-flex items-center gap-1.5 text-sm text-text">
                {node.is_dir && <span className="inline-flex shrink-0 text-text-muted"><Icon name="folder" size={14} /></span>}
                {node.name}
              </span>
            </Row>
          ))}
        </MobileList>
      )}
    </div>
  );
}
