import { useEffect, useState } from "react";
import type { FileContent } from "../../lib/ipc";
import { resolveDocumentImagePath } from "../../lib/document-link";

export type DocumentImageReader = (path: string) => Promise<FileContent>;

interface Props {
  src?: string;
  alt?: string;
  title?: string;
  documentPath?: string;
  rootPath?: string | null;
  readFile?: DocumentImageReader;
  className?: string;
}

/** 로컬 참조는 문서를 연 workspace와 같은 읽기 경계에서 data URL로 가져온다. */
export function MarkdownImage({ src, alt, title, documentPath, rootPath, readFile, className = "max-w-full h-auto", ...rest }: Props) {
  const inline = !!src && /^(?:https?:\/\/|\/\/|data:image\/)/i.test(src);
  const path = src && !inline && documentPath ? resolveDocumentImagePath(src, documentPath, rootPath) : null;
  const [loaded, setLoaded] = useState<{
    path: string;
    reader: DocumentImageReader;
    content?: string;
    error?: string;
  } | null>(null);

  useEffect(() => {
    if (!path || !readFile) return;
    let cancelled = false;
    setLoaded(null);
    void (async () => {
      try {
        const file = await readFile(path);
        if (file.kind !== "image" || !/^data:image\//i.test(file.content)) {
          throw new Error(file.kind === "too_large" ? "이미지가 미리보기 크기 제한(2MB)을 넘었습니다." : "이미지 파일이 아닙니다.");
        }
        if (!cancelled) setLoaded({ path, reader: readFile, content: file.content });
      } catch (error) {
        if (!cancelled) setLoaded({ path, reader: readFile, error: String(error) });
      }
    })();
    return () => { cancelled = true; };
  }, [path, readFile]);

  const current = loaded?.path === path && loaded?.reader === readFile ? loaded : null;
  if (inline || current?.content) {
    return <img {...rest} src={inline ? src : current?.content} alt={alt} title={title} className={className} />;
  }
  const pending = path && readFile && !current?.error;
  return <span {...rest} className="text-text-muted" title={current?.error ?? src}>
    [{pending ? "이미지 로딩 중" : "이미지를 불러오지 못했습니다"}: {alt || src || "이미지"}]
  </span>;
}
