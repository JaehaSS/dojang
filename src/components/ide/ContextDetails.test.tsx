import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { ContextReportEvent } from "../../lib/context-breakdown";
import { ContextDetails } from "./ContextDetails";

const render = (events: ContextReportEvent[] = []) => renderToStaticMarkup(<ContextDetails events={events} />);

describe("ContextDetails", () => {
  it("keeps character sizes and pending calls separate from tokens", () => {
    const html = render([
      { kind: "user", text: "사용자" },
      { kind: "tool_use", name: "Read", tool_id: "read" },
      { kind: "tool_result", tool_use_id: "read", result_chars: 12_000 },
      { kind: "tool_use", name: "Bash", tool_id: "pending" },
    ]);
    expect(html).toContain("12,000자");
    expect(html).toContain("크기 미상");
    expect(html).not.toContain("12,000토큰");
  });

  it("uses bytes only for the latest app-owned injection, never the whole context", () => {
    const html = render([{ kind: "user", text: "안녕", prompt_metrics: { version: 1, user_bytes: 6, sent_user_bytes: 1600, instruction_bytes: 200, skill_added_bytes: 30, goal_added_bytes: 10, capsule_added_bytes: 20, vault_added_bytes: 0 } }]);
    expect(html).toContain("200 B");
    expect(html).toContain("1,600 B");
    expect(html).toContain("지식창고 자료");
    expect(html).toContain("전체 토큰 구성은 제공되지 않습니다");
  });

  it("shows subagents separately and excludes child output from main records", () => {
    const html = render([
      { kind: "subagent_update", vendor: "codex", child_thread_id: "child", title: "검토", model: "child-model", state: "done" },
      { kind: "tool_result", parent_id: "child", result_chars: 99_999 },
    ]);
    expect(html).toContain("child-model");
    expect(html).toContain("메인 사용량에 더하지 않습니다");
    expect(html).not.toContain("99,999");
  });

  it("labels orphan results as records rather than fabricated calls", () => {
    const html = render([{ kind: "tool_result", tool_use_id: "missing", result_chars: 100 }]);
    expect(html).toContain("결과 1건");
    expect(html).not.toContain("1회");
  });
});
