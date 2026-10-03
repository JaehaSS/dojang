import { relativeToWorktree } from "./agent-link";

export type DocumentLinkTarget =
  | { kind: "file"; path: string }
  | { kind: "url"; url: string };

const URI_SCHEME = /^[a-z][a-z\d+.-]*:/i;
const LINE_SUFFIX = /:\d{1,7}(?::\d{1,7})?$/;

function safeDecode(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

/** 루트 밖에서 읽기 전용으로 연 탭의 경로 형태 — `useWorkspaceFiles`가 같은 판정으로 읽기 경로를 가른다. */
export function isAbsolutePath(path: string): boolean {
  return path.startsWith("/") || /^[a-z]:[\\/]/i.test(path);
}

/** 디렉터리 조각 위에 상대 링크를 얹는다. `..`가 `floor`개 조각 아래로 내려가면 실패다 —
 *  worktree 상대 경로는 루트를, 절대 경로는 파일시스템 루트(`""`·`C:`)를 지킨다. */
function walk(parts: string[], link: string, floor: number): string[] | null {
  for (const part of link.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      if (parts.length <= floor) return null;
      parts.pop();
      continue;
    }
    parts.push(part);
  }
  return parts;
}

/** 루트 밖에서 읽기 전용으로 열린 문서(탭 경로가 절대 경로)의 상대 링크는 그 문서의 디렉터리
 *  기준으로 푼다. 이미 그 디렉터리의 파일을 열어 보고 있으므로 옆 파일도 같은 자격으로 연다.
 *  결과가 루트 안이면 루트 기준 상대 경로로 돌려 보통 탭이 되게 한다. */
function absoluteSibling(link: string, sourcePath: string, rootPath?: string | null): string | null {
  const parts = sourcePath.split("/").slice(0, -1);
  // 선두 조각은 POSIX의 `""` 또는 Windows 드라이브 — 그 뒤로는 빈 조각·`.`·`..`가 없어야 한다.
  if (parts.length === 0 || parts.slice(1).some((part) => !part || part === "." || part === "..")) return null;
  const walked = walk(parts, link, 1);
  if (walked == null || walked.length < 2) return null;
  const absolute = walked.join("/");
  return (rootPath ? relativeToWorktree(absolute, rootPath) : null) ?? absolute;
}

function relativePath(link: string, sourcePath: string, rootPath?: string | null): string | null {
  const withoutFragment = link.split("#", 1)[0];
  const decoded = safeDecode(withoutFragment.replace(LINE_SUFFIX, "")).replace(/\\/g, "/");
  if (!decoded || /[\0-\x1f\x7f]/.test(decoded)) return null;
  // 절대 경로는 문서 위치와 무관하다 — 루트 안이냐만 따진다.
  if (isAbsolutePath(decoded)) {
    return rootPath ? relativeToWorktree(decoded, rootPath) : null;
  }
  if (decoded.startsWith("~") || URI_SCHEME.test(decoded)) return null;

  const source = sourcePath.replace(/\\/g, "/");
  if (isAbsolutePath(source)) return absoluteSibling(decoded, source, rootPath);

  const parts = source.split("/").slice(0, -1);
  if (sourcePath.includes("\\") || parts.some((part) => !part || part === "." || part === "..")) return null;
  const walked = walk(parts, decoded, 0);
  return walked?.join("/") || null;
}

/** Markdown 문서 링크는 브라우저 출처가 아니라 링크가 적힌 문서의 디렉터리를 기준으로 푼다.
 *  루트 안 절대 경로는 루트 기준 상대 경로로 푼다 — 루트 밖은 연다고 약속하지 않는다.
 *  단, 문서 자체가 루트 밖에서 열린 것이면 그 문서의 상대 링크는 문서 디렉터리 기준 절대 경로로 푼다. */
export function resolveDocumentLink(
  value: string,
  sourcePath: string,
  rootPath?: string | null,
): DocumentLinkTarget | null {
  const link = value.trim();
  if (!link || link.includes("\0")) return null;
  if (/^https?:\/\//i.test(link)) {
    try {
      return { kind: "url", url: new URL(link).href };
    } catch {
      return null;
    }
  }
  const path = relativePath(link, sourcePath, rootPath);
  return path ? { kind: "file", path } : null;
}
