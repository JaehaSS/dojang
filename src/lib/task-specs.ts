import { invoke } from "@tauri-apps/api/core";

export interface SpecHit {
  relative_path: string;
  title: string;
  snippet: string;
  line: number;
}

export interface SpecSearchPage {
  hits: SpecHit[];
  truncated: boolean;
  scanned_files: number;
  skipped_files: number;
}

export interface SpecDocument {
  relative_path: string;
  title: string;
  content_hash: string;
  byte_length: number;
  content: string;
}

export interface SpecVersion {
  id: number;
  task_id: number;
  relative_path: string;
  title: string;
  content_hash: string;
  byte_length: number;
  created_at: number;
  base_revision: string | null;
}

export interface SpecSnapshot extends SpecVersion {
  content: string;
}

export interface SpecSaveInput {
  relative_path: string;
  expected_hash: string;
}

export type SpecLinkState = "ok" | "missing" | "outside_scope" | "unsupported" | "anchor_missing" | "unreadable" | "external";

export interface SpecLink {
  source_line: number;
  target: string;
  state: SpecLinkState;
  resolved_path: string | null;
}

export interface SpecLinkSummary {
  total: number;
  ok: number;
  missing: number;
  outside_scope: number;
  unsupported: number;
  anchor_missing: number;
  unreadable: number;
  external: number;
}

/** 현재 워크트리 원문에서만 계산한 링크 검사 결과이다. 보존 명세와 연결하지 않는다. */
export interface SpecLinkReport {
  relative_path: string;
  content_hash: string;
  checked_at: number;
  links: SpecLink[];
  summary: SpecLinkSummary;
  truncated: boolean;
}

export const taskSpecSearch = (id: number, query: string) =>
  invoke<SpecSearchPage>("task_spec_search", { id, query });

export const taskSpecRead = (id: number, relativePath: string) =>
  invoke<SpecDocument>("task_spec_read", { id, relativePath });

export const taskSpecSave = (id: number, input: SpecSaveInput) =>
  invoke<SpecSnapshot>("task_spec_save", { id, input });

export const taskSpecList = (id: number) =>
  invoke<SpecVersion[]>("task_spec_list", { id });

export const taskSpecGet = (id: number, snapshotId: number) =>
  invoke<SpecSnapshot>("task_spec_get", { id, snapshotId });

export const taskSpecValidateLinks = (id: number, relativePath: string, expectedHash: string) =>
  invoke<SpecLinkReport>("task_spec_validate_links", { id, relativePath, expectedHash });

const MAX_EXPANDED_INPUT_BYTES = 128 * 1024;
const hashPattern = /^[0-9a-f]{64}$/;

function validPath(path: string): boolean {
  return path.length > 0 && path.length <= 1024 && !/[\r\n\0]/.test(path);
}

/**
 * 사용자가 읽을 수 있는 보존 기록과 전송 시 확장할 단일 참조 토큰을 만든다.
 * 경로나 해시가 신뢰할 수 없으면 입력 초안에 넣지 않는다.
 */
export function taskSpecReference(snapshot: Pick<SpecVersion, "id" | "relative_path" | "content_hash" | "created_at">): string {
  if (!Number.isSafeInteger(snapshot.id) || snapshot.id <= 0) throw new Error("유효하지 않은 보존 명세 ID입니다.");
  if (!hashPattern.test(snapshot.content_hash)) throw new Error("유효하지 않은 보존 명세 해시입니다.");
  if (!validPath(snapshot.relative_path)) throw new Error("유효하지 않은 보존 명세 경로입니다.");
  if (!Number.isFinite(snapshot.created_at) || snapshot.created_at < 0) throw new Error("유효하지 않은 보존 시각입니다.");
  const savedAt = new Date(snapshot.created_at * 1000).toLocaleString("ko-KR");
  return `명세 근거: ${snapshot.relative_path} · 보존 ${savedAt}\n[[dojang-spec:${snapshot.id}:${snapshot.content_hash}]]`;
}

/** 기존 초안을 변경하지 않고 검증된 명세 참조만 덧붙인다. */
export function appendSpecReference(draft: string, reference: string): string {
  const next = draft ? `${draft}\n\n${reference}` : reference;
  if (new TextEncoder().encode(next).byteLength > MAX_EXPANDED_INPUT_BYTES) {
    throw new Error("명세 참조를 연결하면 입력이 128KiB 제한을 넘습니다. 기존 입력을 줄인 뒤 다시 시도하세요.");
  }
  return next;
}
