import { invoke } from "@tauri-apps/api/core";

export type PrStatus = "ready" | "no_pr" | "unsupported" | "unavailable";
export type PrCheckStatus = "pass" | "fail" | "pending" | "skipped" | "cancelled" | "unknown";

export interface PrCheck {
  name: string;
  status: PrCheckStatus;
  description: string;
  details_url: string | null;
}

export interface PrDetails {
  number: number;
  url: string;
  title: string;
  state: "open" | "closed" | "merged";
  draft: boolean;
  head_sha: string;
  head_branch: string;
  base_branch: string;
  review_decision: string;
  review_requests: string[];
  checks: PrCheck[];
}

/** Read-only result of GitHub CLI inspection for one task worktree. */
export interface PrStatusSnapshot {
  status: PrStatus;
  message: string | null;
  repository: string | null;
  branch: string;
  local_head: string | null;
  observed_at: number;
  pr: PrDetails | null;
}

export const taskPrStatusGet = (id: number) => invoke<PrStatusSnapshot>("task_pr_status_get", { id });

const encoder = new TextEncoder();
const REVIEW_CONTEXT_BYTES = 4_000;
const OMITTED = "\n[일부 검사 세부 정보를 생략했습니다]";

const byteLength = (value: string) => encoder.encode(value).length;

/** Clips at a Unicode boundary, reserving the omission marker when truncation occurs. */
function clipUtf8(value: string, maxBytes: number, suffix = ""): string {
  if (byteLength(value) <= maxBytes) return value;
  const suffixBytes = byteLength(suffix);
  // TextDecoder with stream:true retains a partial code point.  It must be fresh for every
  // clipping operation so a previous Korean suffix cannot contaminate a later emoji excerpt.
  const decodePrefix = (source: string, limit: number) => new TextDecoder().decode(encoder.encode(source).subarray(0, limit), { stream: true });
  if (maxBytes <= suffixBytes) return decodePrefix(suffix, maxBytes);
  return decodePrefix(value, maxBytes - suffixBytes) + suffix;
}

/**
 * Builds text that can be appended to the existing human-review reason.  Only failed checks
 * are included, and the stable PR identifier, complete head SHA, and observation time precede
 * any data that may be clipped.
 */
export function prFailureReviewContext(snapshot: PrStatusSnapshot): string | null {
  const pr = snapshot.pr;
  const failed = pr?.checks.filter((check) => check.status === "fail") ?? [];
  if (snapshot.status !== "ready" || !pr || failed.length === 0) return null;
  const number = Number.isSafeInteger(pr.number) && pr.number > 0 ? String(pr.number) : "알 수 없음";
  const prHead = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/i.test(pr.head_sha) ? pr.head_sha : "알 수 없음";
  const repository = clipUtf8(snapshot.repository ?? "알 수 없음", 256, "…");
  const localHead = clipUtf8(snapshot.local_head ?? "알 수 없음", 256, "…");
  const scope = snapshot.local_head && snapshot.local_head === pr.head_sha
    ? "로컬 HEAD와 PR HEAD가 일치하지만, 미커밋 변경은 원격 검사에 포함되지 않습니다."
    : "로컬 HEAD와 PR HEAD가 다르거나 확인되지 않아, 원격 검사는 현재 로컬 결과의 검증이 아닙니다.";

  const header = [
    "",
    "[원격 PR 검사 실패]",
    `저장소: ${repository}`,
    `PR: #${number}`,
    `PR HEAD: ${prHead}`,
    `로컬 HEAD: ${localHead}`,
    `조회 시각: ${formatObservedAt(snapshot.observed_at)}`,
    `비교 범위: ${scope}`,
  ].join("\n") + "\n";
  const details = failed.map((check) => {
    const link = safePublicHttpUrl(check.details_url);
    return [
      `실패 검사: ${check.name}`,
      check.description ? `설명: ${check.description}` : "설명: 없음",
      link ? `세부 링크: ${link}` : check.details_url ? "세부 링크: 안전하지 않은 주소" : "",
    ].filter(Boolean).join("\n");
  }).join("\n\n");

  // Preserve the fixed PR metadata first, then spend the remaining review budget on
  // attacker-controlled CI descriptions and URLs.
  const remaining = REVIEW_CONTEXT_BYTES - byteLength(header);
  if (remaining <= 0) return clipUtf8(header, REVIEW_CONTEXT_BYTES, OMITTED);
  return header + clipUtf8(details, remaining, OMITTED);
}

export function formatObservedAt(seconds: number): string {
  const instant = new Date(seconds * 1000);
  return Number.isFinite(instant.getTime()) ? instant.toLocaleString() : "알 수 없음";
}

function ipv4Parts(hostname: string): number[] | null {
  if (!/^\d{1,3}(?:\.\d{1,3}){3}$/.test(hostname)) return null;
  const parts = hostname.split(".").map(Number);
  return parts.every((part) => part >= 0 && part <= 255) ? parts : null;
}

function isPrivateIpv4(parts: number[]): boolean {
  const [first, second] = parts;
  return first === 0 || first === 10 || first === 127 || first >= 224
    || (first === 100 && second >= 64 && second <= 127)
    || (first === 169 && second === 254)
    || (first === 172 && second >= 16 && second <= 31)
    || (first === 192 && (second === 0 || second === 168))
    || (first === 198 && (second === 18 || second === 19 || second === 51))
    || (first === 203 && second === 0);
}

/** Reject credentials, local names, and non-public IP literals before handing a URL to the OS. */
export function safePublicHttpUrl(value: string | null | undefined): string | null {
  if (!value || /[\u0000-\u001f\u007f]/.test(value)) return null;
  try {
    const parsed = new URL(value);
    if ((parsed.protocol !== "https:" && parsed.protocol !== "http:") || parsed.username || parsed.password) return null;
    const hostname = parsed.hostname.toLowerCase().replace(/\.$/, "");
    if (!hostname || hostname === "localhost" || hostname.endsWith(".localhost") || hostname.endsWith(".local")) return null;
    const v4 = ipv4Parts(hostname);
    // Reject IPv6 literals because URL normalisation can conceal mapped private IPv4 values
    // in hexadecimal form. This guards explicit addresses; it does not inspect DNS resolution.
    if (v4 ? isPrivateIpv4(v4) : hostname.includes(":")) return null;
    // Single-label DNS names are ordinarily intranet hosts and cannot be established as public.
    if (!v4 && !hostname.includes(".")) return null;
    return parsed.href;
  } catch {
    return null;
  }
}

/** PR links are intentionally narrower than external CI detail links. */
export function safeGithubPullRequestUrl(value: string | null | undefined, repository?: string | null, number?: number): string | null {
  const safe = safePublicHttpUrl(value);
  if (!safe) return null;
  const parsed = new URL(safe);
  if (parsed.protocol !== "https:" || parsed.port || parsed.search || parsed.hash || parsed.hostname.toLowerCase().replace(/\.$/, "") !== "github.com") return null;
  const match = /^\/([A-Za-z0-9_.-]+)\/([A-Za-z0-9_.-]+)\/pull\/([1-9]\d*)\/?$/.exec(parsed.pathname);
  if (!match) return null;
  if (repository && `${match[1]}/${match[2]}`.toLowerCase() !== repository.toLowerCase()) return null;
  if (number != null && (!Number.isSafeInteger(number) || String(number) !== match[3])) return null;
  return safe;
}
