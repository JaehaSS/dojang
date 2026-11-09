import { invoke } from "@tauri-apps/api/core";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { buildHtmlPreview } from "./html-preview-assets";
import { fsRead, type FileContent } from "./ipc";
import { LOCAL_HOST } from "./transport";

export type ArtifactKind = "html" | "markdown" | "image";

/** Immutable metadata for one explicitly saved result artifact. */
export interface ArtifactVersion {
  id: number;
  task_id: number;
  source_id: string;
  result_revision: number;
  relative_path: string;
  version: number;
  kind: ArtifactKind;
  content_hash: string;
  created_at: number;
  base_revision: string | null;
  unavailable: string[];
}

/** Immutable artifact metadata and its saved source/preview content. */
export interface ArtifactSnapshot extends ArtifactVersion {
  content: string;
  preview_html: string | null;
}

export interface ArtifactResourceInput {
  relative_path: string;
  expected_content: string;
}

export interface ArtifactSaveInput {
  source_id: string;
  expected_revision: number;
  relative_path: string;
  expected_content: string;
  preview_html: string | null;
  unavailable: string[];
  resources: ArtifactResourceInput[];
}

export interface ArtifactPrepareOptions {
  taskId: number;
  sourceId: string;
  resultRevision: number;
  relativePath: string;
}

const MAX_SAVED_PAYLOAD = 8 * 1024 * 1024;
const REVIEW_EXCERPT_BYTES = 1_000;
const HTML_EXTENSION = /\.(?:html?|xhtml)$/i;
const MARKDOWN_EXTENSION = /\.(?:md|mdown|mkdn|markdown)$/i;

const unsupported = (message: string): never => { throw new Error(message); };

export function artifactKind(path: string, file: Pick<FileContent, "kind">): ArtifactKind {
  if (file.kind === "image") return "image";
  if (file.kind !== "text") return unsupported("HTML, Markdown, 이미지 파일만 저장할 수 있습니다.");
  if (HTML_EXTENSION.test(path)) return "html";
  if (MARKDOWN_EXTENSION.test(path)) return "markdown";
  return unsupported("HTML, Markdown, 이미지 파일만 저장할 수 있습니다.");
}

/** Markdown is rendered as inert static markup before the existing asset bundler adds the CSP. */
function markdownDocument(content: string): string {
  const body = renderToStaticMarkup(createElement(ReactMarkdown, { remarkPlugins: [remarkGfm] }, content));
  return `<!doctype html><html><head><meta charset="utf-8"><title>Markdown artifact</title></head><body>${body}</body></html>`;
}

/**
 * Read an artifact once and prepare the exact optimistic-CAS payload for the Rust command.
 * Captured resource contents are passed back with the preview so the backend can verify every
 * source used to construct the immutable document.
 */
export async function prepareArtifactSave(input: ArtifactPrepareOptions): Promise<ArtifactSaveInput & { kind: ArtifactKind }> {
  const ref = { host: LOCAL_HOST, id: input.taskId } as const;
  const source = await fsRead(ref, input.relativePath);
  const kind = artifactKind(input.relativePath, source);
  if (source.kind === "too_large") unsupported("파일이 미리보기 크기 제한을 넘었습니다.");

  if (kind === "image") {
    if (!/^data:image\//i.test(source.content)) unsupported("이미지 데이터 URL을 준비하지 못했습니다.");
    if (new TextEncoder().encode(source.content).length > MAX_SAVED_PAYLOAD) unsupported("저장할 산출물이 크기 제한을 넘었습니다.");
    return {
      kind,
      source_id: input.sourceId,
      expected_revision: input.resultRevision,
      relative_path: input.relativePath,
      expected_content: source.content,
      preview_html: null,
      unavailable: [],
      resources: [],
    };
  }

  const resources = new Map<string, string>();
  const readAsset = async (path: string) => {
    const file = await fsRead(ref, path);
    resources.set(path, file.content);
    return file;
  };
  const rendered = kind === "html" ? source.content : markdownDocument(source.content);
  const preview = await buildHtmlPreview(rendered, input.relativePath, readAsset);
  const resourceEntries = [...resources].map(([relative_path, expected_content]) => ({ relative_path, expected_content }));
  const size = new TextEncoder().encode(JSON.stringify({ content: source.content, preview: preview.html, resources: resourceEntries, unavailable: preview.unavailable })).length;
  if (size > MAX_SAVED_PAYLOAD) unsupported("저장할 산출물과 미리보기의 합계가 크기 제한을 넘었습니다.");
  return {
    kind,
    source_id: input.sourceId,
    expected_revision: input.resultRevision,
    relative_path: input.relativePath,
    expected_content: source.content,
    preview_html: preview.html,
    unavailable: preview.unavailable,
    resources: resourceEntries,
  };
}

/** Keep immutable version references useful without consuming the review-reason byte budget. */
export function artifactReviewContext(before: ArtifactSnapshot, after: ArtifactSnapshot): string {
  const clip = (content: string) => {
    const encoder = new TextEncoder();
    const bytes = encoder.encode(content);
    if (bytes.length <= REVIEW_EXCERPT_BYTES) return content;
    const suffix = "\n[저장된 본문 일부 생략]";
    const prefix = bytes.subarray(0, REVIEW_EXCERPT_BYTES - encoder.encode(suffix).length);
    // Streaming decode leaves an incomplete final code point buffered rather than emitting U+FFFD.
    return new TextDecoder().decode(prefix, { stream: true }) + suffix;
  };
  const reference = (label: string, snapshot: ArtifactSnapshot) => [
    `${label}: 저장 버전 #${snapshot.id} (${snapshot.source_id}; ${snapshot.relative_path})`,
    `해시: ${snapshot.content_hash}; 결과 revision: ${snapshot.result_revision}`,
    snapshot.kind === "image" ? "저장된 이미지 버전입니다. 원본 경로의 현재 내용과 다를 수 있습니다." : `저장된 본문 발췌:\n${clip(snapshot.content)}`,
  ].join("\n");
  return `\n[산출물 버전 비교]\n${reference("이전", before)}\n\n${reference("이후", after)}\n`;
}

/** Local-only Tauri contract for Phase 1 immutable review artifacts. */
export const reviewArtifactList = (id: number) => invoke<ArtifactVersion[]>("review_artifact_list", { id });
export const reviewArtifactGet = (id: number, artifactId: number) =>
  invoke<ArtifactSnapshot>("review_artifact_get", { id, artifactId });
export const reviewArtifactSave = (id: number, input: ArtifactSaveInput) =>
  invoke<ArtifactSnapshot>("review_artifact_save", { id, input });
