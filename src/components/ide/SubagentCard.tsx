import { useId, useState } from "react";
import type { SubagentThread } from "../../lib/activity";
import type { ConvoItem } from "./ConversationView";
import { Icon } from "./icons";
import { Markdown } from "./Markdown";
import { ToolOutput } from "./ToolOutput";

interface Props {
  thread: SubagentThread<ConvoItem>;
  parentBusy: boolean;
  defaultExpanded?: boolean;
  onOpenSubagent?: (toolId: string) => void;
  onOpenLink?: (link: string) => void;
  onLinkMenu?: (link: string, at: { x: number; y: number }) => void;
}

interface StatusPresentation {
  mark: string;
  color: string;
  label: string;
}

function statusFor(thread: SubagentThread<ConvoItem>, parentBusy: boolean): StatusPresentation {
  if (thread.state === "failed") return { mark: "×", color: "text-status-failed", label: "실패" };
  if (thread.state === "done") return { mark: "✓", color: "text-status-done", label: "완료" };
  if (thread.state === "stopped") return { mark: "○", color: "text-status-awaiting", label: "중단됨" };
  // Codex의 unknown은 부모 턴 종료를 근거로 중단이라고 추정하지 않는다. 기록이 충분하지 않아
  // 완료를 확인하지 못한 상태와 명시적인 중단은 사용자가 구분할 수 있어야 한다.
  if (thread.state === "unknown") return { mark: "?", color: "text-text-muted", label: "완료 확인 불가" };
  if (thread.vendor === "codex") return { mark: "→", color: "text-primary-bright", label: "작업 중" };
  if (!parentBusy) return { mark: "○", color: "text-status-awaiting", label: "중단됨" };
  return { mark: "→", color: "text-primary-bright", label: "작업 중" };
}

const detailsUnavailable = (thread: SubagentThread<ConvoItem>) =>
  thread.vendor === "codex" && thread.observationStatus != null && thread.observationStatus !== "available";

function TranscriptItem({
  item,
  onOpenLink,
  onLinkMenu,
}: {
  item: ConvoItem;
  onOpenLink?: (link: string) => void;
  onLinkMenu?: (link: string, at: { x: number; y: number }) => void;
}) {
  if (item.role === "text") {
    return <Markdown text={item.text} onOpenLink={onOpenLink} onLinkMenu={onLinkMenu} />;
  }
  if (item.role === "tool_output") return <ToolOutput contents={item.contents} onOpenLink={onOpenLink} />;
  if (item.role === "tool") {
    return (
      <div className="flex items-center gap-1.5 font-code text-text-muted">
        <Icon name="plug" size={12} />
        <span className="shrink-0 text-text-secondary">{item.name}</span>
        {item.summary && <span className="truncate">{item.summary}</span>}
      </div>
    );
  }
  if (item.role === "tool_result") {
    return (
      <div className={`font-code whitespace-pre-wrap break-words ${item.is_error ? "text-status-failed" : "text-text-muted"}`}>
        {item.summary || "(빈 결과)"}
      </div>
    );
  }
  if (item.role === "error") {
    return <div className="text-status-failed whitespace-pre-wrap break-words">{item.text}</div>;
  }
  if (item.role === "user") {
    return <div className="text-text-secondary whitespace-pre-wrap break-words">{item.text}</div>;
  }
  // 남는 것은 meta뿐이다. 명시적으로 좁히지 않으면 ConvoItem에 role이 하나 늘 때마다
  // 이 return이 조용히 그것까지 meta로 렌더하려 든다(실제로 divider 추가에서 그랬다).
  if (item.role !== "meta") return null;
  return (
    <div className="font-code text-text-muted">
      {item.tokensIn} → {item.tokensOut} tok
    </div>
  );
}

/** 메인 대화 안에서 서브 에이전트 응답을 기본 접힘으로 보여 주는 인라인 카드. */
export function SubagentCard({
  thread,
  parentBusy,
  defaultExpanded = false,
  onOpenSubagent,
  onOpenLink,
  onLinkMenu,
}: Props) {
  const [expanded, setExpanded] = useState(defaultExpanded);
  const contentId = useId();
  const status = statusFor(thread, parentBusy);

  return (
    <section className="max-w-[92%] overflow-hidden rounded-md border border-border bg-bg text-xs">
      <button
        type="button"
        className="flex w-full items-center gap-2 px-2.5 py-2 text-left hover:bg-raised"
        aria-expanded={expanded}
        aria-controls={contentId}
        onClick={() => setExpanded((value) => !value)}
      >
        <Icon name={expanded ? "chevronDown" : "chevronRight"} size={13} />
        <span className="text-primary-bright">
          <Icon name="plug" size={13} />
        </span>
        <span className="shrink-0 text-text-secondary">서브 에이전트</span>
        <span className="truncate font-code text-text-muted">{thread.title}</span>
        {/* 관측된 모델 id를 그대로 이름한다 — 별칭으로 바꾸면 실제로 도는 모델을 가린다. */}
        {thread.model && (
          <span className="shrink-0 rounded border border-border px-1 font-code text-[10px] text-text-muted">
            {thread.model}
          </span>
        )}
        <span className={`ml-auto shrink-0 ${status.color}`}>
          {status.mark} {status.label}
        </span>
      </button>

      {expanded && (
        <div id={contentId} className="border-t border-border">
          <div className="max-h-80 space-y-2 overflow-y-auto overflow-x-hidden px-3 py-2.5 text-text-secondary">
            {thread.items.length > 0 && detailsUnavailable(thread) && (
              <div className="text-text-muted">일부 상세 기록을 확인할 수 없습니다.</div>
            )}
            {thread.items.length === 0 ? (
              <div className="text-text-muted">
                {detailsUnavailable(thread)
                  ? "상세 기록을 확인할 수 없습니다."
                  : thread.state === "running" && (thread.vendor === "codex" || parentBusy)
                  ? "서브 에이전트 응답을 기다리는 중…"
                  : "기록된 상세 응답이 없습니다."}
              </div>
            ) : (
              thread.items.map((item, index) => (
                <TranscriptItem
                  key={index}
                  item={item}
                  onOpenLink={onOpenLink}
                  onLinkMenu={onLinkMenu}
                />
              ))
            )}
          </div>
          {onOpenSubagent && (
            <button
              type="button"
              className="w-full border-t border-border px-3 py-1.5 text-right text-text-muted hover:text-text"
              onClick={() => onOpenSubagent(thread.id)}
            >
              탭에서 열기 ›
            </button>
          )}
        </div>
      )}
    </section>
  );
}
