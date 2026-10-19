import type { SubagentEntry } from "../../lib/activity";
import { ConversationView, type ConvoItem } from "./ConversationView";

interface Props {
  /** 이 서브 에이전트의 파생 상태 — 이력에서 스폰 카드가 사라진 경우 null(제목만 폴백). */
  entry: SubagentEntry | null;
  /** 이 서브 에이전트 소속(parentId 일치) 이벤트만 필터된 트랜스크립트. */
  items: ConvoItem[];
  /** 부모 대화 턴 진행 여부 — 서브가 running이고 턴도 진행 중일 때만 "작업 중" 표시. */
  parentBusy: boolean;
  onOpenLink?: (link: string) => void;
  onLinkMenu?: (link: string, at: { x: number; y: number }) => void;
  hidden?: boolean;
  conversationId?: string;
}

const headerStatus = (entry: SubagentEntry | null, parentBusy: boolean) => {
  if (!entry) return { mark: "●", color: "text-text-muted", label: "기록된 서브 에이전트" };
  if (entry.state === "failed") return { mark: "×", color: "text-status-failed", label: "실패" };
  if (entry.state === "done") return { mark: "✓", color: "text-status-done", label: "완료" };
  if (entry.state === "stopped") return { mark: "○", color: "text-status-awaiting", label: "중단됨" };
  if (entry.state === "unknown") return { mark: "?", color: "text-text-muted", label: "완료 확인 불가" };
  if (entry.vendor === "codex") return { mark: "→", color: "text-primary-bright", label: "작업 중" };
  if (!parentBusy)
    return { mark: "○", color: "text-status-awaiting", label: "미완료 — 턴 종료로 중단됨" };
  return { mark: "→", color: "text-primary-bright", label: "작업 중" };
};

/** 서브 에이전트 전용 탭 — 헤더(이름·상태) + 해당 서브의 트랜스크립트(ConversationView 재사용).
 *  스트림에 서브 이벤트가 없던 구 이력에서는 빈 트랜스크립트 안내가 뜬다. */
export function SubagentView({
  entry,
  items,
  parentBusy,
  onOpenLink,
  onLinkMenu,
  hidden = false,
  conversationId,
}: Props) {
  const st = headerStatus(entry, parentBusy);
  const running = entry?.state === "running" && (entry.vendor === "codex" || parentBusy);
  const detailsUnavailable = entry?.vendor === "codex" && entry.observationStatus != null && entry.observationStatus !== "available";
  return (
    <div className="flex flex-1 min-h-0 flex-col">
      <div className="flex shrink-0 items-center gap-2 border-b border-border bg-bg px-4 py-2 text-sm">
        <span className={st.color}>{st.mark}</span>
        <span className="text-text truncate">{entry?.title ?? "서브 에이전트"}</span>
        <span className="ml-auto shrink-0 text-xs text-text-muted">{st.label}</span>
      </div>
      {detailsUnavailable && (items.length > 0 || running) && (
        <div className="px-4 py-2 text-sm text-text-muted">
          {items.length > 0 ? "일부 상세 기록을 확인할 수 없습니다." : "상세 기록을 확인할 수 없습니다."}
        </div>
      )}
      {items.length === 0 && !running ? (
        <div className="px-4 py-3 text-sm text-text-muted">
          {detailsUnavailable
            ? "이 서브 에이전트의 상세 기록을 확인할 수 없습니다."
            : "이 서브 에이전트의 상세 이벤트가 없습니다 — 스폰 요약과 결과는 메인 대화에서 확인하세요."}
        </div>
      ) : (
        // 이벤트가 아직 없어도 작업 중이면 busy 인디케이터를 보여 헤더와 본문 상태를 일치시킨다.
        <ConversationView
          conversationId={conversationId}
          items={items}
          busy={!!running}
          onOpenLink={onOpenLink}
          onLinkMenu={onLinkMenu}
          hidden={hidden}
        />
      )}
    </div>
  );
}
