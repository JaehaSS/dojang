// src/lib/session-home-tree.ts — 세션홈 이어받기 화면의 프로젝트 → 세션 트리 모델
// (설계 2026-09-18). session-navigator.ts와 같은 노드·행 문법을 쓰되, 데이터가 Task가
// 아니라 세션홈 스캔 결과이고 그룹 단이 없어(결정 1) 별도 모듈로 둔다. UI 비의존.

import type { SessionHomeSession } from "./transport";

export type SessionHomeNodeKind = "project" | "session";

export interface SessionHomeNode {
  id: string;
  kind: SessionHomeNodeKind;
  label: string;
  /** 프로젝트: 전체 경로(없으면 null). 세션: 프로젝트 경로 기준 상대 cwd(같으면 null). */
  detail: string | null;
  session?: SessionHomeSession;
  children: SessionHomeNode[];
}

export interface SessionHomeRow extends SessionHomeNode {
  depth: number;
}

/** cwd를 모르는 세션이 모이는 자리 — 버리면 승계 대상이 조용히 사라진다(결정 2). */
export const UNKNOWN_PROJECT_ID = "project:?";

const stripTrailingSlash = (path: string): string => path.replace(/\/+$/, "");

const basename = (path: string): string => path.split("/").filter(Boolean).pop() ?? path;

const isUnder = (cwd: string, root: string): boolean => cwd === root || cwd.startsWith(`${root}/`);

const sessionCwd = (session: SessionHomeSession): string | null =>
  session.cwd ?? session.last_cwd ?? null;

export const sessionVendor = (session: Pick<SessionHomeSession, "vendor">): "claude" | "codex" =>
  session.vendor === "codex" ? "codex" : "claude";

export const sessionHomeKey = (session: Pick<SessionHomeSession, "host" | "vendor" | "session_id">): string =>
  `${session.host}:${sessionVendor(session)}:${session.session_id}`;

export function sessionLabel(session: Pick<SessionHomeSession, "title" | "first_message">): string {
  return session.title?.trim() || session.first_message?.trim() || "(제목 없음)";
}

/**
 * 세션이 속할 프로젝트 경로 — 등록 프로젝트의 **최장** 접두 일치, 없으면 cwd 자체(결정 2).
 * 최장을 고르는 이유는 `/work`와 `/work/app`이 모두 등록돼 있을 때 `/work/app/x`가 `/work`로
 * 새지 않게 하기 위해서다.
 */
export function projectOf(session: SessionHomeSession, projects: readonly string[]): string | null {
  const cwd = sessionCwd(session);
  if (!cwd) return null;
  const normalized = stripTrailingSlash(cwd);
  if (!normalized) return null; // cwd가 "/"뿐이면 프로젝트라 부를 수 없다 — 경로 없음으로 모은다.
  let best: string | null = null;
  for (const project of projects) {
    const root = stripTrailingSlash(project);
    if (!root) continue;
    if (isUnder(normalized, root) && (best === null || root.length > best.length)) best = root;
  }
  return best ?? normalized;
}

/**
 * 저장소 접두 조회와 전체 조회를 host/vendor/session id로 합친다. 앞쪽 배열이 우선이고
 * 순서는 첫 등장 순을 지킨다 — 서버가 준 최근순이 곧 프로젝트 안의 세션 순서다.
 */
export function mergeSessionHomeResults(
  ...lists: readonly (readonly SessionHomeSession[])[]
): SessionHomeSession[] {
  const seen = new Set<string>();
  const out: SessionHomeSession[] = [];
  for (const list of lists) {
    for (const session of list) {
      const key = sessionHomeKey(session);
      if (seen.has(key)) continue;
      seen.add(key);
      out.push(session);
    }
  }
  return out;
}

/**
 * 프로젝트 → 세션 트리. 현재 저장소의 프로젝트가 맨 앞, 나머지는 가장 최근 세션의
 * `last_active` 내림차순, cwd를 모르는 묶음은 맨 뒤(결정 3). 프로젝트 안의 세션 순서는
 * 입력 순서 그대로다.
 */
export function buildSessionHomeTree(
  sessions: readonly SessionHomeSession[],
  repo: string,
  projects: readonly string[],
): SessionHomeNode[] {
  const currentRepo = stripTrailingSlash(repo);
  const roots = currentRepo ? [currentRepo, ...projects] : [...projects];
  const buckets = new Map<string | null, SessionHomeSession[]>();
  for (const session of sessions) {
    const key = projectOf(session, roots);
    const bucket = buckets.get(key);
    if (bucket) bucket.push(session);
    else buckets.set(key, [session]);
  }

  const nodes: SessionHomeNode[] = [];
  for (const [root, bucket] of buckets) {
    const children = bucket.map((session): SessionHomeNode => {
      const cwd = sessionCwd(session);
      const relative =
        root && cwd && stripTrailingSlash(cwd) !== root
          ? stripTrailingSlash(cwd).slice(root.length + 1)
          : null;
      return {
        id: `session:${sessionHomeKey(session)}`,
        kind: "session",
        label: sessionLabel(session),
        detail: relative,
        session,
        children: [],
      };
    });
    nodes.push({
      id: root ? `project:${root}` : UNKNOWN_PROJECT_ID,
      kind: "project",
      label: root ? basename(root) : "(경로 없음)",
      detail: root,
      children,
    });
  }

  const latest = (node: SessionHomeNode): number =>
    node.children.reduce((max, child) => Math.max(max, child.session?.last_active ?? 0), 0);
  const rank = (node: SessionHomeNode): number => {
    if (currentRepo && node.id === `project:${currentRepo}`) return 0;
    if (node.id === UNKNOWN_PROJECT_ID) return 2;
    return 1;
  };
  return nodes.sort((a, b) => rank(a) - rank(b) || latest(b) - latest(a));
}

/** 접힌 프로젝트의 세션은 감춘다. 검색 중에는 접힘을 무시하고 전부 편다(결정 5). */
export function sessionHomeRows(
  nodes: readonly SessionHomeNode[],
  collapsed: ReadonlySet<string>,
  searching: boolean,
): SessionHomeRow[] {
  const rows: SessionHomeRow[] = [];
  for (const node of nodes) {
    rows.push({ ...node, depth: 0 });
    if (searching || !collapsed.has(node.id)) {
      for (const child of node.children) rows.push({ ...child, depth: 1 });
    }
  }
  return rows;
}

/** 처음 열 때 접어 둘 프로젝트 — 현재 저장소만 펼친다(결정 3). */
export function initiallyCollapsedProjects(nodes: readonly SessionHomeNode[], repo: string): Set<string> {
  const currentId = `project:${stripTrailingSlash(repo)}`;
  return new Set(nodes.filter((node) => node.id !== currentId).map((node) => node.id));
}
