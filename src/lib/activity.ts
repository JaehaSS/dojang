/** ConversationView의 모든 역할을 받을 수 있는 활동 투영용 최소 구조 타입. */
export interface ActivityItem {
  role: string;
  name?: string;
  title?: string;
  summary?: string;
  is_error?: boolean;
  text?: string;
  toolId?: string;
  toolUseId?: string;
  parentId?: string;
  model?: string | null;
  /** Codex 관측 메타데이터의 카드 키. Claude toolId와 분리한다. */
  subagentId?: string;
  vendor?: "claude" | "codex";
  state?: SubagentState;
  observationStatus?: SubagentObservationStatus;
}

export type SubagentState = "running" | "done" | "failed" | "stopped" | "unknown";
export type SubagentObservationStatus = "available" | "missing" | "unreadable" | "unsupported" | "limited";

export interface ActivityEntry {
  index: number;
  state: "started" | "done" | "failed";
  title: string;
  detail: string;
}

/** 서브 에이전트(Task 도구) 1건의 진행 상태 — Task tool_use(id)와 parented 이벤트를 묶어 파생. */
export interface SubagentEntry {
  id: string;
  title: string;
  state: SubagentState;
  /** 기존 Task/Agent 카드는 claude, 자식 세션 관측 카드는 codex다. */
  vendor?: "claude" | "codex";
  /** 이 서브 에이전트가 마지막으로 수행한 도구 호출 요약. */
  lastOp: string | null;
  /** 관측된 실행 모델 — parented assistant의 message.model. 관측 전엔 null. */
  model: string | null;
  /** 상세 기록을 얻지 못한 사유. 상태와 독립이라 중단으로 바꾸지 않는다. */
  observationStatus?: SubagentObservationStatus;
}

export interface SubagentThread<T extends ActivityItem = ActivityItem> extends SubagentEntry {
  /** 토글 안에 표시할 서브 에이전트 이벤트와 최종 Task 결과. */
  items: T[];
}

/** 서브 에이전트 스폰 도구 — CLI 버전에 따라 Task/Agent 명칭이 혼재해 둘 다 인식.
 *  중첩 스폰(서브가 또 스폰, parentId 있음)은 별도 항목 없이 부모의 lastOp로만 표시된다. */
const isSubagentSpawn = (item: ActivityItem) =>
  item.role === "tool" &&
  !!item.toolId &&
  !item.parentId &&
  (item.name === "Task" || item.name === "Agent");

/** 대화 아이템에서 서브 에이전트 목록을 파생한다 (최신 스폰 먼저).
 *  스폰 tool_use → running, 부모 Task의 tool_result 도착 → done/failed,
 *  parented 도구 이벤트 → 해당 서브 에이전트의 lastOp 갱신. */
export const subagentEntries = (items: ActivityItem[]): SubagentEntry[] => {
  const byId = new Map<string, SubagentEntry>();
  const roots = subagentRootMap(items);
  for (const item of items) {
    if (isSubagentSpawn(item)) {
      const entry: SubagentEntry = {
        id: item.toolId!,
        title: item.summary || "서브 에이전트",
        state: "running",
        lastOp: null,
        model: null,
        vendor: "claude",
      };
      byId.set(entry.id, entry);
      continue;
    }
    if (item.role === "subagent_state" && item.subagentId && !item.parentId) {
      const existing = byId.get(item.subagentId);
      const entry: SubagentEntry = {
        id: item.subagentId,
        title: item.title || item.summary || existing?.title || "Codex 서브에이전트",
        state: item.state || existing?.state || "unknown",
        lastOp: existing?.lastOp ?? null,
        model: item.model ?? existing?.model ?? null,
        vendor: "codex",
        observationStatus: item.observationStatus ?? existing?.observationStatus,
      };
      byId.set(entry.id, entry);
      continue;
    }
  }
  for (const item of items) {
    // 직접 parentId로만 귀속한다 — 중첩 스폰의 모델 이벤트는 parentId가 그 스폰 id라
    // 루트만 담은 byId에서 걸리지 않는다. 루트 카드가 중첩 에이전트 모델로 덮이면 안 된다.
    if (item.role === "subagent_model" && item.parentId) {
      const owner = byId.get(item.parentId);
      if (owner && item.model) owner.model = item.model; // 마지막 관측이 이긴다
      continue;
    }
    if (item.role === "tool" && item.parentId) {
      // 중첩 자식의 작업 내용은 루트 카드의 상세 요약에 보인다. root map은 수집 순서와
      // 무관하게 부모 체인을 푼다. 모델 칩은 위의 직접 부모 규칙을 그대로 써서 덮지 않는다.
      const owner = byId.get(roots.get(item.parentId) ?? "");
      if (owner) owner.lastOp = [item.name, item.summary].filter(Boolean).join(" · ") || null;
      continue;
    }
    // Task 완료 판정은 메인 스레드 tool_result만 — parented 결과는 서브 내부 도구의 것.
    if (item.role === "tool_result" && item.toolUseId && !item.parentId) {
      const owner = byId.get(item.toolUseId);
      if (owner) owner.state = item.is_error ? "failed" : "done";
    }
  }
  return [...byId.values()].reverse();
};

/** 스폰 tool_id → 최상위 서브 에이전트 id 매핑 — 중첩 스폰(Task 안의 Task)을 루트 탭에 귀속.
 *  스트림 순서상 스폰이 자식 이벤트보다 먼저 오므로 단일 패스로 충분하다. */
export const subagentRootMap = (items: ActivityItem[]): Map<string, string> => {
  const parents = new Map<string, string | null>();
  for (const item of items) {
    if (item.role === "tool" && item.toolId && (item.name === "Task" || item.name === "Agent")) {
      parents.set(item.toolId, item.parentId ?? null);
    }
    if (item.role === "subagent_state" && item.subagentId) {
      parents.set(item.subagentId, item.parentId ?? null);
    }
  }
  const roots = new Map<string, string>();
  const findRoot = (id: string, visiting = new Set<string>()): string | null => {
    const cached = roots.get(id);
    if (cached) return cached;
    if (visiting.has(id) || !parents.has(id)) return null;
    visiting.add(id);
    const parent = parents.get(id);
    const root = parent ? findRoot(parent, visiting) : id;
    visiting.delete(id);
    if (root) roots.set(id, root);
    return root;
  };
  for (const id of parents.keys()) {
    findRoot(id);
  }
  return roots;
};

/** 아이템이 귀속되는 최상위 서브 에이전트 id — 메인 스레드/부모 미상은 null. */
export const subagentRootOf = (item: ActivityItem, roots: Map<string, string>): string | null =>
  item.parentId ? (roots.get(item.parentId) ?? null) : null;

/** 인라인 토글에 귀속될 루트 id — parented 이벤트와 최상위 Task 결과를 함께 묶는다. */
export const subagentThreadRootOf = (
  item: ActivityItem,
  roots: Map<string, string>,
): string | null => {
  // 상태 메타데이터는 인라인 카드의 앵커다. 스레드 본문으로 옮기면 카드가 사라진다.
  if (item.role === "subagent_state") return null;
  if (item.parentId) return roots.get(item.parentId) ?? null;
  if (item.role !== "tool_result" || !item.toolUseId) return null;
  const root = roots.get(item.toolUseId);
  return root === item.toolUseId ? root : null;
};

/** 대화 아이템을 루트 서브 에이전트별 인라인 토글 스레드로 투영한다. */
export const subagentThreads = <T extends ActivityItem>(items: T[]): SubagentThread<T>[] => {
  const entries = subagentEntries(items);
  const roots = subagentRootMap(items);
  const grouped = new Map(entries.map((entry) => [entry.id, [] as T[]]));
  for (const item of items) {
    // 모델 관측은 트랜스크립트가 아니라 카드 메타데이터다. 넣으면 이것뿐인 스레드가
    // "기다리는 중…" 안내 대신 빈 줄을 그린다.
    if (item.role === "subagent_model" || item.role === "subagent_state") continue;
    const root = subagentThreadRootOf(item, roots);
    if (root) grouped.get(root)?.push(item);
  }
  return entries.map((entry) => ({ ...entry, items: grouped.get(entry.id) ?? [] }));
};

/** 대화 아이템에서 에디터 활동 패널에 표시할 이벤트만 최신순으로 추린다.
 *  서브 에이전트 소속(parented) 이벤트는 제외 — 서브 에이전트 섹션이 요약한다. */
export const activityEntries = (items: ActivityItem[]): ActivityEntry[] =>
  items
    .flatMap((item, index): ActivityEntry[] => {
      if (item.parentId) return [];
      if (item.role === "tool") {
        return [{ index, state: "started", title: item.name || "도구 호출", detail: item.summary ?? "" }];
      }
      if (item.role === "tool_result") {
        return [
          {
            index,
            state: item.is_error ? "failed" : "done",
            title: item.is_error ? "도구 실행 실패" : "도구 실행 완료",
            detail: item.summary ?? "",
          },
        ];
      }
      if (item.role === "error") {
        return [{ index, state: "failed", title: "에이전트 오류", detail: item.text ?? "" }];
      }
      return [];
    })
    .reverse();

/** 현재 작업 요약은 검증된 상태 조회 값을 우선하고, 없으면 마지막 도구 호출을 사용한다.
 *  서브 에이전트 내부(parented) 이벤트는 제외 — 메인 스레드의 현재 작업만 대표한다. */
export const currentOperation = (items: ActivityItem[], lastOperation: string | null | undefined) => {
  if (lastOperation) return lastOperation;
  const tool = [...items]
    .reverse()
    .find(
      (item) =>
        !item.parentId &&
        (item.role === "tool" || item.role === "tool_result" || item.role === "error"),
    );
  if (!tool || tool.role !== "tool") return null;
  return [tool.name, tool.summary].filter(Boolean).join(" · ");
};
