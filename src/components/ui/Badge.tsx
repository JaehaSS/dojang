import type { ReactNode } from "react";
import type { StatusTone } from "../../lib/task-status";
import { TaskStatusIcon } from "../ide/TaskStatusIcon";

const TONE: Record<StatusTone, string> = {
  running: "border-status-running/30 bg-status-running/10 text-status-running",
  awaiting: "border-status-awaiting/30 bg-status-awaiting/10 text-status-awaiting",
  question: "border-status-question/30 bg-status-question/10 text-status-question",
  done: "border-status-done/30 bg-status-done/10 text-status-done",
  failed: "border-status-failed/30 bg-status-failed/10 text-status-failed",
  muted: "border-border bg-text/[.04] text-text-muted",
};

/**
 * 작업 상태 배지 (DESIGN.md `components.Badge`). **상태 전용이다** — 개수·분류·메타에 쓰면
 * "색이 곧 상태"라는 계약이 흐려진다. 그런 표시는 MetaTag나 중립 텍스트로 한다.
 *
 * 아이콘과 문구를 항상 함께 단다(Do #2). 아이콘은 모양만 보태므로 장식으로 숨기고, 이름은
 * 보이는 문구가 맡는다.
 */
export function Badge({ tone, children }: { tone: StatusTone; children: ReactNode }) {
  return (
    <span
      className={`inline-flex shrink-0 items-center gap-1 whitespace-nowrap rounded-full border px-2 py-0.5 text-xs font-medium ${TONE[tone]}`}
      data-tone={tone}
    >
      <TaskStatusIcon tone={tone} label="" size={10} decorative />
      {children}
    </span>
  );
}
