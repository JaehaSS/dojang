import { describe, expect, it } from "vitest";
import { buildContextReport, type ContextReportEvent } from "./context-breakdown";

const event = (kind: string, fields: Omit<ContextReportEvent, "kind"> = {}): ContextReportEvent => ({ kind, ...fields });

describe("buildContextReport", () => {
  it("counts only main-thread messages and tool results, retaining Codex and Claude subagent metadata", () => {
    const report = buildContextReport([
      event("user", { text: "질문" }),
      event("text", { text: "답변" }),
      event("tool_use", { name: "Read", tool_id: "read-1" }),
      event("tool_result", { tool_use_id: "read-1", result_chars: 12 }),
      event("text", { text: "자식", parent_id: "task-1" }),
      event("tool_use", { name: "Bash", tool_id: "child-1", parent_id: "task-1" }),
      event("tool_result", { tool_use_id: "child-1", result_chars: 99, parent_id: "task-1" }),
      event("subagent_update", { vendor: "codex", child_thread_id: "child-thread", title: "검토", model: "gpt-5", state: "done" }),
      event("tool_use", { name: "Task", summary: "조사", tool_id: "task-1" }),
      event("subagent_model", { parent_id: "task-1", model: "claude-sonnet" }),
      event("tool_result", { tool_use_id: "task-1", result_chars: 0 }),
    ]);

    expect(report.messages).toEqual({ count: 2, chars: 4 });
    expect(report.tools).toEqual([
      { name: "Read", group: "files", calls: 1, resultCount: 1, chars: 12, unknownSizes: 0 },
      { name: "Task", group: "agents", calls: 1, resultCount: 1, chars: 0, unknownSizes: 0 },
    ]);
    expect(report.subagents).toEqual([
      { id: "child-thread", title: "검토", model: "gpt-5", state: "done" },
      { id: "task-1", title: "조사", model: "claude-sonnet", state: "done" },
    ]);
  });

  it("resets the report at the last current context boundary and excludes inherited display history", () => {
    const report = buildContextReport([
      event("user", { text: "이전" }),
      event("tool_use", { name: "Read", tool_id: "old" }),
      event("tool_result", { tool_use_id: "old", result_chars: 100 }),
      event("context_cleared"),
      event("text", { text: "표시만", inherited: true }),
      event("history_truncated", { inherited: true, dropped: 8 }),
      event("user", { text: "새" }),
    ]);

    expect(report.hasHistory).toBe(true);
    expect(report.messages).toEqual({ count: 1, chars: 1 });
    expect(report.tools).toEqual([]);
    expect(report.limitations).toEqual([
      "이어받은 이력 2건은 현재 컨텍스트 집계에서 제외했습니다.",
      "이력 상한으로 이전 이벤트 8건이 생략되었습니다.",
    ]);
  });

  it("keeps malformed result sizes unknown and rejects malformed latest prompt metrics", () => {
    const report = buildContextReport([
      event("user", {
        text: "첫 요청",
        prompt_metrics: {
          version: 1,
          user_bytes: 1,
          sent_user_bytes: 1,
          instruction_bytes: 1,
          skill_added_bytes: 1,
          goal_added_bytes: 1,
          capsule_added_bytes: 1,
          vault_added_bytes: 1,
        },
      }),
      event("tool_use", { name: "Bash", tool_id: "bash" }),
      event("tool_result", { tool_use_id: "bash", result_chars: -1 }),
      event("user", { text: "둘째 요청", prompt_metrics: { version: 1, user_bytes: 2 } }),
    ]);

    expect(report.tools).toEqual([{ name: "Bash", group: "commands", calls: 1, resultCount: 1, chars: 0, unknownSizes: 1 }]);
    expect(report.injection).toBeNull();
  });

  it("does not clear main-session records when a child or peer context is cleared", () => {
    const report = buildContextReport([
      event("user", { text: "메인" }),
      event("tool_use", { name: "Read", tool_id: "read" }),
      event("tool_result", { tool_use_id: "read", result_chars: 12 }),
      event("context_cleared", { parent_id: "child" }),
      event("context_cleared", { speaker: "right" }),
    ]);
    expect(report.messages).toEqual({ count: 1, chars: 2 });
    expect(report.tools[0]).toMatchObject({ name: "Read", chars: 12 });
  });

  it("does not fall back from an explicit unmatched ID and avoids ambiguous id-less parallel results", () => {
    const report = buildContextReport([
      event("tool_use", { name: "Read", tool_id: "read" }),
      event("tool_result", { tool_use_id: "missing", result_chars: 5 }),
      event("tool_use", { name: "Bash", tool_id: "bash" }),
      event("tool_result", { result_chars: 7 }),
    ]);

    expect(report.tools).toEqual([
      { name: "Read", group: "files", calls: 1, resultCount: 0, chars: 0, unknownSizes: 1 },
      { name: "(이름 미상)", group: "other", calls: 0, resultCount: 2, chars: 12, unknownSizes: 0 },
      { name: "Bash", group: "commands", calls: 1, resultCount: 0, chars: 0, unknownSizes: 1 },
    ]);
  });

  it("uses one final streaming snapshot and counts Unicode code points", () => {
    const report = buildContextReport([
      event("user", { text: "A😀" }),
      event("text_update", { item_id: "reply", text: "첫", complete: false }),
      event("text_update", { item_id: "reply", text: "첫😀", complete: true }),
    ]);

    expect(report.messages).toEqual({ count: 2, chars: 4 });
  });

  it("defaults to the left debate speaker while retaining untagged shared user events", () => {
    const report = buildContextReport([
      event("user", { text: "공통" }),
      event("text", { text: "왼쪽", speaker: "left" }),
      event("text", { text: "오른쪽", speaker: "right" }),
      event("tool_use", { name: "Grep", tool_id: "right-search", speaker: "right" }),
      event("tool_result", { tool_use_id: "right-search", result_chars: 30, speaker: "right" }),
    ]);

    expect(report.messages).toEqual({ count: 2, chars: 4 });
    expect(report.tools).toEqual([]);
  });

  it("uses the newest valid prompt metrics without summing them", () => {
    const report = buildContextReport([
      event("user", { text: "첫", prompt_metrics: { version: 1, user_bytes: 1, sent_user_bytes: 2, instruction_bytes: 3, skill_added_bytes: 4, goal_added_bytes: 5, capsule_added_bytes: 6, vault_added_bytes: 7 } }),
      event("user", { text: "둘", prompt_metrics: { version: 1, user_bytes: 8, sent_user_bytes: 9, instruction_bytes: 10, skill_added_bytes: 11, goal_added_bytes: 12, capsule_added_bytes: 13, vault_added_bytes: 14 } }),
    ]);

    expect(report.injection).toEqual({ userBytes: 8, sentUserBytes: 9, instructionBytes: 10, skillBytes: 11, goalBytes: 12, capsuleBytes: 13, vaultBytes: 14 });
  });

  it("keeps a pending call's size unknown and clears old injection when the newest user has no metrics", () => {
    const report = buildContextReport([
      event("user", { text: "첫", prompt_metrics: { version: 1, user_bytes: 1, sent_user_bytes: 1, instruction_bytes: 1, skill_added_bytes: 1, goal_added_bytes: 1, capsule_added_bytes: 1, vault_added_bytes: 1 } }),
      event("tool_use", { name: "Code mode", tool_id: "pending" }),
      event("user", { text: "둘" }),
    ]);

    expect(report.tools).toEqual([{ name: "Code mode", group: "commands", calls: 1, resultCount: 0, chars: 0, unknownSizes: 1 }]);
    expect(report.injection).toBeNull();
  });
});
