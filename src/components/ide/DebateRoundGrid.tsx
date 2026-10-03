import { type ReactElement, type ReactNode } from "react";
import { badgeLabelFor, labelFor } from "../../lib/agents";
import { DEBATE_SEATS, type DebateSpeaker } from "../../lib/ipc";
import type { ConvoItem } from "./ConversationView";
import { ToolOutput } from "./ToolOutput";
import type { DebateRound } from "./debate-rounds";
import { Icon } from "./icons";
import { Markdown } from "./Markdown";
import { MIN_DEBATE_PANE_WIDTH } from "./workspace-split-width";
import type { DebatePane } from "./DebateView";

/** 면 개수별 위치 이름 — 자리 id(`right`)는 이름일 뿐이라 3자에서 가운데 면을 "우측"이라 읽으면 틀린다. */
const POSITION: Record<number, readonly string[]> = { 2: ["좌측", "우측"], 3: ["첫째", "둘째", "셋째"] };

/** 면 격자 — 자리 수만큼 같은 폭으로 나눈다. 클래스 이름을 조립하면 Tailwind가 못 찾는다. */
export const paneGrid = (count: number) => ({ gridTemplateColumns: `repeat(${count}, minmax(0, 1fr))` });

export function PaneHeader({ pane, seat, count, active }: { pane: DebatePane; seat: number; count: number; active: boolean }): ReactElement {
  const name = labelFor(pane.agent);
  return (
    <div
      className="flex items-center gap-2 border-b border-border px-3 py-1.5 text-xs"
      aria-label={`${POSITION[count]?.[seat] ?? `${seat + 1}번째`} 발화자 ${name}`}
    >
      <Icon name="sparkle" size={12} />
      <span className={active ? "text-text" : "text-text-secondary"}>{name}</span>
      {pane.model && <span className="rounded border border-border px-1.5 py-0.5 font-code text-[11px] text-text-muted">{pane.model}</span>}
      {/* 토론 중 세션 교체는 그쪽 벤더 맥락을 통째로 버린다 — 손잡이는 있되 눌리지 않는다. */}
      <button
        type="button"
        className="ml-auto text-text-muted disabled:cursor-not-allowed disabled:opacity-50"
        disabled
        title="토론 중에는 에이전트·모델을 바꿀 수 없습니다"
      >
        <Icon name="chevronDown" size={12} />
      </button>
    </div>
  );
}

/** 라운드 안 면마다 다는 발화자 표지 — 고정 헤더는 긴 발화를 스크롤하면 눈에서 멀어진다. */
function SpeakerTag({ pane }: { pane: DebatePane }): ReactElement {
  const name = badgeLabelFor(pane.agent) ?? labelFor(pane.agent);
  return (
    <div className="flex items-center gap-1.5 px-3 pt-2 text-[11px] text-text-secondary" data-testid="debate-speaker">
      <Icon name="sparkle" size={11} />
      <span>{name}</span>
      {pane.model && <span className="font-code text-text-muted">{pane.model}</span>}
    </div>
  );
}

export function Items({ items, onOpenLink, renderQuestion }: { items: ConvoItem[]; onOpenLink?: (link: string) => void; renderQuestion?: (id: string) => ReactNode }): ReactElement {
  return (
    <div className="space-y-2 px-3 py-2">
      {items.map((item, i) => {
        if (item.role === "text") return <Markdown key={i} text={item.text} onOpenLink={onOpenLink} />;
        if (item.role === "user") return <div key={i} className="rounded-lg bg-primary/15 px-3 py-2 text-md text-text whitespace-pre-wrap">{item.text}</div>;
        if (item.role === "error") return <div key={i} className="font-code text-sm text-status-failed whitespace-pre-wrap">{item.text}</div>;
        if (item.role === "divider") return <div key={i} role="separator" className="py-1 text-[11px] text-text-muted">{item.text}</div>;
        if (item.role === "interaction") return <div key={i}>{renderQuestion?.(item.interactionId)}</div>;
        if (item.role === "tool_output") return <ToolOutput key={i} contents={item.contents} onOpenLink={onOpenLink} />;
        if (item.role === "tool" || item.role === "tool_result")
          return (
            <div key={i} className="truncate rounded-md border border-border px-2 py-1 font-code text-xs text-text-muted">
              {item.role === "tool" ? `${item.name} ${item.summary}` : item.summary}
            </div>
          );
        return null;
      })}
    </div>
  );
}

export function Round({ round, cap, panes, active, busy, onOpenLink, renderQuestion }: {
  round: DebateRound;
  /** 자리 순서의 면 정체. 길이가 곧 자리 수다. */
  panes: readonly DebatePane[];
  cap: number;
  active: DebateSpeaker;
  busy: boolean;
  onOpenLink?: (link: string) => void;
  renderQuestion?: (id: string) => ReactNode;
}): ReactElement {
  const cursor = (side: DebateSpeaker) =>
    busy && active === side ? <div className="px-3 pb-2 text-xs text-primary-bright">▌ 응답 생성 중…</div> : null;
  const orphans = round.panes.slice(panes.length).flat();
  return (
    <div>
      {/* 구분선은 모든 면을 가로지른다 — 면마다 그으면 어느 발화가 어느 발화의 답인지 읽을 수 없다. */}
      <div className="flex items-center gap-2 border-y border-border bg-raised px-3 py-1" role="heading" aria-level={3}>
        <span className="h-px flex-1 bg-border" />
        <span className="text-[11px] text-text-secondary">라운드 {round.no}/{cap}</span>
        <span className="h-px flex-1 bg-border" />
      </div>
      {round.user && <Items items={[{ role: "user", text: round.user }]} onOpenLink={onOpenLink} renderQuestion={renderQuestion} />}
      {round.common.length > 0 && <Items items={round.common} onOpenLink={onOpenLink} renderQuestion={renderQuestion} />}
      <div className="grid divide-x divide-border" style={paneGrid(panes.length)}>
        {panes.map((pane, seat) => (
          <div key={DEBATE_SEATS[seat]} style={{ minWidth: MIN_DEBATE_PANE_WIDTH }}>
            <SpeakerTag pane={pane} />
            <Items items={round.panes[seat] ?? []} onOpenLink={onOpenLink} renderQuestion={renderQuestion} />
            {cursor(DEBATE_SEATS[seat])}
          </div>
        ))}
      </div>
      {/* 지금보다 자리가 많던 이전 토론의 발화 — 면이 없다고 버리면 이력이 화면에서 사라진다. */}
      {orphans.length > 0 && (
        <>
          <div className="border-t border-border px-3 pt-2 text-[11px] text-text-muted">이전 토론의 다른 참여자</div>
          <Items items={orphans} onOpenLink={onOpenLink} renderQuestion={renderQuestion} />
        </>
      )}
    </div>
  );
}
