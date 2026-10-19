/** Raw conversation fields used to derive the current main-session context report. */
export interface ContextReportEvent {
  kind: string;
  speaker?: unknown;
  parent_id?: unknown;
  inherited?: unknown;
  text?: unknown;
  item_id?: unknown;
  complete?: unknown;
  name?: unknown;
  summary?: unknown;
  tool_id?: unknown;
  tool_use_id?: unknown;
  result_chars?: unknown;
  is_error?: unknown;
  prompt_metrics?: unknown;
  vendor?: unknown;
  root_thread_id?: unknown;
  child_thread_id?: unknown;
  title?: unknown;
  nickname?: unknown;
  agent_role?: unknown;
  model?: unknown;
  state?: unknown;
  dropped?: unknown;
}

export type ToolGroup = "files" | "search" | "commands" | "mcp" | "skills" | "agents" | "other";

export const TOOL_GROUP_LABELS: Record<ToolGroup, string> = {
  files: "파일",
  search: "검색",
  commands: "명령",
  mcp: "MCP",
  skills: "스킬",
  agents: "서브에이전트",
  other: "기타",
};

export interface ContextInjection {
  userBytes: number;
  sentUserBytes: number;
  instructionBytes: number;
  skillBytes: number;
  goalBytes: number;
  capsuleBytes: number;
  vaultBytes: number;
}

export interface ContextReport {
  hasHistory: boolean;
  messages: { count: number; chars: number };
  tools: Array<{ name: string; group: ToolGroup; calls: number; resultCount: number; chars: number; unknownSizes: number }>;
  injection: ContextInjection | null;
  subagents: Array<{ id: string; title: string; model: string | null; state: string | null }>;
  /** The displayed event stream does not fully describe the current active context. */
  limitations: string[];
}

type MutableTool = { calls: number; resultCount: number; chars: number; unknownSizes: number };
type ActiveTool = { key: number; id: string | null; name: string };
type MutableSubagent = { id: string; title: string; model: string | null; state: string | null };

const UNKNOWN_TOOL = "(이름 미상)";
const CODEX_SUBAGENT_TITLE = "Codex 서브에이전트";
const CLAUDE_SUBAGENT_TITLE = "서브에이전트";

const isSafeNonNegative = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;

const isMainThread = (event: ContextReportEvent): boolean =>
  event.parent_id === null || event.parent_id === undefined;

const isInherited = (event: ContextReportEvent): boolean => event.inherited === true;

const nonEmptyString = (value: unknown): string | null =>
  typeof value === "string" && value.trim().length > 0 ? value : null;

const codePointLength = (text: string): number => Array.from(text).length;

/** Parser-emitted names and the MCP prefix are the only taxonomy inputs. */
const toolGroup = (name: string): ToolGroup => {
  if (["Read", "Write", "Edit", "ApplyPatch", "apply_patch", "edit"].includes(name)) return "files";
  if (["Grep", "Glob"].includes(name)) return "search";
  if (["Bash", "shell", "Code mode"].includes(name)) return "commands";
  if (name.startsWith("mcp__")) return "mcp";
  if (name === "Skill") return "skills";
  if (name === "Task" || name === "Agent") return "agents";
  return "other";
};

const validInjection = (value: unknown): ContextInjection | null => {
  if (value == null || typeof value !== "object" || Array.isArray(value)) return null;
  const metrics = value as Record<string, unknown>;
  if (metrics.version !== 1) return null;
  const fields = [
    "user_bytes",
    "sent_user_bytes",
    "instruction_bytes",
    "skill_added_bytes",
    "goal_added_bytes",
    "capsule_added_bytes",
    "vault_added_bytes",
  ] as const;
  if (!fields.every((field) => isSafeNonNegative(metrics[field]))) return null;
  return {
    userBytes: metrics.user_bytes as number,
    sentUserBytes: metrics.sent_user_bytes as number,
    instructionBytes: metrics.instruction_bytes as number,
    skillBytes: metrics.skill_added_bytes as number,
    goalBytes: metrics.goal_added_bytes as number,
    capsuleBytes: metrics.capsule_added_bytes as number,
    vaultBytes: metrics.vault_added_bytes as number,
  };
};

/**
 * Composes visible, current-session events without converting bytes or characters into tokens.
 * Speaker-less events are shared; a report otherwise follows the selected debate participant.
 */
export function buildContextReport(
  events: readonly ContextReportEvent[],
  speaker: string | null | undefined = "left",
): ContextReport {
  const selectedSpeaker = speaker ?? "left";
  const inScope = (event: ContextReportEvent): boolean =>
    event.speaker == null || event.speaker === selectedSpeaker;

  let lastClear = -1;
  for (let index = 0; index < events.length; index += 1) {
    const event = events[index];
    if (!isInherited(event) && isMainThread(event) && inScope(event) && event.kind === "context_cleared") lastClear = index;
  }
  const current = events.slice(lastClear + 1).filter(inScope);
  const inheritedCount = current.filter(isInherited).length;
  const truncated = current
    .filter((event) => event.kind === "history_truncated")
    .reduce((sum, event) => sum + (isSafeNonNegative(event.dropped) ? event.dropped : 0), 0);
  const resumedExternal = current.some((event) => event.kind === "resume_external");
  const active = current.filter((event) => !isInherited(event));

  const tools = new Map<string, MutableTool>();
  const getTool = (name: string): MutableTool => {
    const existing = tools.get(name);
    if (existing) return existing;
    const created: MutableTool = { calls: 0, resultCount: 0, chars: 0, unknownSizes: 0 };
    tools.set(name, created);
    return created;
  };
  const activeTools = new Map<number, ActiveTool>();
  const activeToolIds = new Map<string, Set<number>>();
  const removeActive = (tool: ActiveTool): void => {
    activeTools.delete(tool.key);
    if (tool.id == null) return;
    const keys = activeToolIds.get(tool.id);
    if (!keys) return;
    keys.delete(tool.key);
    if (keys.size === 0) activeToolIds.delete(tool.id);
  };
  let nextToolKey = 0;

  let messageCount = 0;
  let messageChars = 0;
  const snapshots = new Map<string, string>();
  let anonymousSnapshots = 0;
  let injection: ContextInjection | null = null;
  let sawUser = false;
  const subagents = new Map<string, MutableSubagent>();

  for (const event of active) {
    if (event.kind === "user" && isMainThread(event)) {
      const text = typeof event.text === "string" ? event.text : "";
      messageCount += 1;
      messageChars += codePointLength(text);
      injection = validInjection(event.prompt_metrics);
      sawUser = true;
      continue;
    }

    if (event.kind === "text" && isMainThread(event)) {
      const text = typeof event.text === "string" ? event.text : "";
      messageCount += 1;
      messageChars += codePointLength(text);
      continue;
    }

    if (event.kind === "text_update" && isMainThread(event)) {
      const text = typeof event.text === "string" ? event.text : "";
      const itemId = nonEmptyString(event.item_id);
      snapshots.set(itemId ?? `__anonymous_${anonymousSnapshots++}`, text);
      continue;
    }

    if (event.kind === "tool_use" && isMainThread(event)) {
      const name = nonEmptyString(event.name) ?? UNKNOWN_TOOL;
      const row = getTool(name);
      row.calls += 1;
      // A started call has no measured result yet. A later valid result clears this
      // marker; a missing or malformed result deliberately leaves it visible.
      row.unknownSizes += 1;
      const id = nonEmptyString(event.tool_id);
      const tool: ActiveTool = { key: nextToolKey++, id, name };
      activeTools.set(tool.key, tool);
      if (id != null) {
        const keys = activeToolIds.get(id) ?? new Set<number>();
        keys.add(tool.key);
        activeToolIds.set(id, keys);
      }
      if ((name === "Task" || name === "Agent") && id != null) {
        subagents.set(id, { id, title: nonEmptyString(event.summary) ?? CLAUDE_SUBAGENT_TITLE, model: null, state: "running" });
      }
      continue;
    }

    if (event.kind === "tool_result" && isMainThread(event)) {
      const hasExplicitId = event.tool_use_id !== null && event.tool_use_id !== undefined;
      const explicitId = nonEmptyString(event.tool_use_id);
      let matched: ActiveTool | null = null;
      if (hasExplicitId && explicitId != null) {
        const keys = activeToolIds.get(explicitId);
        if (keys?.size === 1) {
          const key = [...keys][0];
          matched = activeTools.get(key) ?? null;
        }
      } else if (!hasExplicitId && activeTools.size === 1) {
        matched = activeTools.values().next().value ?? null;
      }

      const name = matched?.name ?? UNKNOWN_TOOL;
      const row = getTool(name);
      if (matched) removeActive(matched);
      // Keep an orphan result visible without pretending it was a recorded invocation.
      row.resultCount += 1;
      if (!matched) row.unknownSizes += 1;
      if (isSafeNonNegative(event.result_chars)) {
        row.chars += event.result_chars;
        row.unknownSizes -= 1;
      }

      if (explicitId != null) {
        const subagent = subagents.get(explicitId);
        if (subagent) subagent.state = event.is_error === true ? "failed" : "done";
      }
      continue;
    }

    if (
      event.kind === "subagent_model" &&
      isMainThread(event) === false &&
      nonEmptyString(event.parent_id) != null &&
      nonEmptyString(event.model) != null
    ) {
      const subagent = subagents.get(nonEmptyString(event.parent_id)!);
      if (subagent) subagent.model = nonEmptyString(event.model);
      continue;
    }

    if (event.kind === "subagent_update" && isMainThread(event) && event.vendor === "codex") {
      const id = nonEmptyString(event.child_thread_id);
      if (id == null) continue;
      const previous = subagents.get(id);
      subagents.set(id, {
        id,
        title: nonEmptyString(event.title) ?? nonEmptyString(event.nickname) ?? nonEmptyString(event.agent_role) ?? previous?.title ?? CODEX_SUBAGENT_TITLE,
        model: nonEmptyString(event.model) ?? previous?.model ?? null,
        state: nonEmptyString(event.state) ?? previous?.state ?? null,
      });
    }
  }

  for (const text of snapshots.values()) {
    messageCount += 1;
    messageChars += codePointLength(text);
  }

  const limitations: string[] = [];
  if (inheritedCount > 0) limitations.push(`이어받은 이력 ${inheritedCount}건은 현재 컨텍스트 집계에서 제외했습니다.`);
  if (truncated > 0) limitations.push(`이력 상한으로 이전 이벤트 ${truncated}건이 생략되었습니다.`);
  if (resumedExternal) limitations.push("외부 세션의 이전 이력은 현재 보고서에 없습니다.");

  return {
    hasHistory: active.length > 0,
    messages: { count: messageCount, chars: messageChars },
    tools: [...tools.entries()].map(([name, row]) => ({ name, group: toolGroup(name), ...row })),
    injection: sawUser ? injection : null,
    subagents: [...subagents.values()],
    limitations,
  };
}
