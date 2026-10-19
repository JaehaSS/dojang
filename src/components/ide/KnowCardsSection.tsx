import { useEffect, useState, type MouseEvent, type ReactElement } from "react";
import { stamp } from "../../lib/fmt";
import type { KnowCard, KnowCardAction } from "../../lib/ipc";
import { MetaTag } from "../MetaTag";
import { ChannelCard } from "./ChannelCard";

/** 채널이 카드에 넘기는 것. App의 `useKnowCards`가 채운다. */
export interface KnowCardsBinding {
  /** 활성 카드, 최신 순. */
  cards: readonly KnowCard[];
  onAct: (cardId: number, action: KnowCardAction) => void;
  onSeen: (cardId: number) => void;
  /** 사이드 질문으로 묻기. 사이드 질문을 쓸 수 없는 세션이면 넘기지 않는다 — `자세히`가 숨는다. */
  onAsk?: (card: KnowCard) => void;
}

interface Props extends KnowCardsBinding {
  /** 지금 화면에 보이는가. 비활성 탭에 마운트된 카드는 읽음 처리하지 않는다(설계 5절 A13). */
  visible: boolean;
  /** 자리마다 다른 배치 — 플로팅 채널은 하단에 붙이고 높이를 자른다. */
  className?: string;
}

const KIND_LABEL: Record<KnowCard["kind"], string> = {
  you_should_know: "알아둘 것",
  heads_up: "주의",
};

/**
 * 어느 카드를 보고 있는가. 번호가 아니라 카드에 매단다 — 다른 페이지를 보는 동안 새 카드가
 * 와도 보던 카드가 그대로 있어야 한다(설계 2.4 "페이지 넘김").
 */
interface Page {
  /** 보고 있는 카드. null이면 1페이지에 머문다 — 새 카드가 오면 새 카드가 보인다. */
  anchor: number | null;
  index: number;
  /** 1페이지에서 마지막으로 본 카드. 지금 맨 앞 카드와 다르면 그사이 새 카드가 왔다. */
  headSeen: number | null;
}

const firstPage = (ids: readonly number[]): Page => ({ anchor: null, index: 0, headSeen: ids[0] ?? null });

/**
 * 목록이 바뀐 뒤의 페이지. 바뀐 것이 없으면 같은 객체를 돌려준다 — 렌더 중 갱신의 종료 조건이다.
 *
 * 보던 카드가 빠지면(이미 앎·닫기·밀려남) 같은 자리의 카드, 즉 더 오래된 카드를 보여 준다.
 * 마지막 카드였으면 바로 앞 카드다. 그 카드에 다시 매달아야 다음 새 카드에도 자리가 유지된다.
 */
function follow(page: Page, ids: readonly number[]): Page {
  if (page.anchor == null) return page.headSeen === (ids[0] ?? null) ? page : firstPage(ids);
  const at = ids.indexOf(page.anchor);
  const index = at >= 0 ? at : Math.min(page.index, ids.length - 1);
  if (index <= 0) return firstPage(ids);
  return ids[index] === page.anchor && index === page.index ? page : { ...page, anchor: ids[index], index };
}

/** `자세히`가 사이드 질문에 넣는 맥락 — 카드에 보이는 세 부분 그대로다. */
export function knowCardQuestion(card: KnowCard): string {
  return `${card.title}\n\n${card.body}\n\n근거: ${card.evidence}`;
}

/** 오늘이면 시각만, 아니면 날짜까지. 세션은 며칠씩 이어지기도 한다. */
function cardTime(sec: number): string {
  const d = new Date(sec * 1000);
  if (d.toDateString() !== new Date().toDateString()) return stamp(sec);
  const p2 = (n: number) => String(n).padStart(2, "0");
  return `${p2(d.getHours())}:${p2(d.getMinutes())}`;
}

const actionCls =
  "rounded px-1.5 py-0.5 text-xs text-text-secondary hover:bg-surface hover:text-text focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary";
const pagerCls =
  "relative rounded px-1 text-text-secondary hover:text-text disabled:opacity-30 disabled:hover:text-text-secondary";

/**
 * 더블클릭의 두 번째 클릭은 버린다. 첫 클릭에 카드가 빠지면 같은 자리에 다음 카드의 같은 버튼이
 * 와서, 두 번째 클릭이 보지도 않은 카드를 치운다. 키보드로 누른 클릭은 `detail`이 0이라 그대로 받는다.
 */
const once = (run: () => void) => (event: MouseEvent<HTMLButtonElement>) => {
  if (event.detail > 1) return;
  run();
};

/**
 * 알아둘 것 카드 — 도구를 많이 쓴 턴이 끝난 뒤 판정이 남긴 카드를 채널 맨 아래에 한 장씩 보인다.
 *
 * 내용 영역의 높이는 일정하고 긴 텍스트는 줄바꿈해 내부에서 스크롤한다. 카드가 하단에 붙어
 * 있어 높이가 바뀌면 머리의 페이저가 위아래로 움직이고, 연달아 누르던 `›`가 빗나간다.
 *
 * 출처 줄(시각·요청)과 근거 줄은 접지 않는다. 구석의 카드가 어디서 왔는지 모르면 앱 기능이
 * 아니라 소음으로 읽힌다(설계 1절, 교훈 #513).
 */
export function KnowCardsSection({ cards, onAct, onSeen, onAsk, visible, className }: Props): ReactElement | null {
  const ids = cards.map((card) => card.id);
  const [page, setPage] = useState<Page>(() => firstPage(ids));
  const view = follow(page, ids);
  if (view !== page) setPage(view);

  const card = cards[view.index] ?? null;
  const cardId = card?.id ?? null;
  const unseen = card != null && card.seen_at == null;

  useEffect(() => {
    if (!visible || cardId == null || !unseen) return;
    // 창이 가려져 있으면 보일 때까지 미룬다 — 백그라운드에서 온 카드를 본 것으로 치지 않는다.
    if (document.visibilityState !== "hidden") {
      onSeen(cardId);
      return;
    }
    const onChange = () => {
      if (document.visibilityState === "hidden") return;
      document.removeEventListener("visibilitychange", onChange);
      onSeen(cardId);
    };
    document.addEventListener("visibilitychange", onChange);
    return () => document.removeEventListener("visibilitychange", onChange);
  }, [visible, cardId, unseen, onSeen]);

  if (card == null) return null;

  const go = (index: number) =>
    setPage(index <= 0 ? firstPage(ids) : { anchor: ids[index], index, headSeen: view.headSeen });
  const fresh = view.index > 0 && view.headSeen !== ids[0];

  return (
    <ChannelCard label="알아둘 것" className={className}>
      <div className="flex shrink-0 items-center gap-2 border-b border-border px-3 py-2">
        <MetaTag>{KIND_LABEL[card.kind]}</MetaTag>
        {cards.length > 1 && (
          <nav aria-label="알아둘 것 페이지" className="ml-auto flex items-center gap-1 text-xs">
            <button
              type="button"
              className={pagerCls}
              aria-label={fresh ? "더 최신 카드 (새 카드 있음)" : "더 최신 카드"}
              disabled={view.index === 0}
              onClick={() => go(view.index - 1)}
            >
              ‹
              {fresh && (
                <span
                  data-testid="know-cards-fresh"
                  className="absolute -top-0.5 -left-0.5 h-1.5 w-1.5 rounded-full bg-primary-bright"
                />
              )}
            </button>
            <span className="tabular-nums text-text-muted">
              {view.index + 1}/{cards.length}
            </span>
            <button
              type="button"
              className={pagerCls}
              aria-label="더 오래된 카드"
              disabled={view.index === cards.length - 1}
              onClick={() => go(view.index + 1)}
            >
              ›
            </button>
          </nav>
        )}
      </div>

      <article
        key={card.id}
        aria-label="알아둘 것 내용"
        tabIndex={0}
        className="h-44 min-h-0 space-y-1 overflow-y-auto whitespace-pre-wrap break-words px-3 py-2 focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-primary"
      >
        <p className="text-[11px] text-text-muted">
          {cardTime(card.created_at)}
          {card.turn_label && ` · '${card.turn_label}'`}
        </p>
        <h3 className="text-sm font-medium leading-5 text-text">{card.title}</h3>
        <p className="text-xs leading-5 text-text-secondary">{card.body}</p>
        <p className="text-[11px] leading-4 text-text-muted">
          근거: {card.evidence}
        </p>
      </article>

      <div className="flex shrink-0 items-center gap-1 border-t border-border px-2 py-1.5">
        {onAsk && (
          <button
            type="button"
            className={actionCls}
            onClick={once(() => {
              onAsk(card);
              onAct(card.id, "asked");
            })}
          >
            자세히
          </button>
        )}
        <button type="button" className={actionCls} onClick={once(() => onAct(card.id, "known"))}>
          이미 앎
        </button>
        <button type="button" className={actionCls} onClick={once(() => onAct(card.id, "dismissed"))}>
          닫기
        </button>
      </div>
    </ChannelCard>
  );
}
