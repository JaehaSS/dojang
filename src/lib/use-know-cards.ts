import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { knowCardAct, knowCardSeen, knowCardsList, type KnowCard, type KnowCardAction } from "./ipc";

/** 카드 생성·동작 처리 뒤 백엔드가 보내는 이벤트. 페이로드는 `{ taskId }`. */
export const KNOW_CARDS_CHANGED = "know-cards://changed";

const EMPTY: readonly KnowCard[] = [];

export interface KnowCards {
  /** 활성 카드, 최신 순. */
  cards: readonly KnowCard[];
  /** 아직 화면에 보인 적 없는 카드 수 — 접힌 채널 핸들의 배지. */
  unread: number;
  act: (cardId: number, action: KnowCardAction) => void;
  markSeen: (cardId: number) => void;
}

/**
 * 선택한 세션의 알아둘 것 카드.
 *
 * App이 부른다. 채널의 카드와 접힌 채널 핸들의 배지가 같은 목록을 봐야 하기 때문이다.
 * `taskId`는 로컬 대화 세션일 때만 넘긴다. 원격 작업은 판정이 돌지 않으므로 null을 받아
 * 조회하지 않는다(설계 3.8 "명령과 이벤트").
 *
 * 목록은 어느 세션의 것인지와 함께 들고, 세션이 바뀌면 새 목록이 오기 전까지 비워 둔다.
 * 앞 세션의 카드가 새 세션 화면에 한 번이라도 비치면 그 세션의 카드로 읽힌다. 늦게 돌아온
 * 응답이 새 응답을 덮지 않도록 요청 번호로 가른다(`usePendingQuestions`와 같은 방식).
 */
export function useKnowCards(taskId: number | null): KnowCards {
  const [state, setState] = useState<{ taskId: number | null; cards: readonly KnowCard[] }>({
    taskId: null,
    cards: EMPTY,
  });
  const request = useRef(0);
  // 지금 보고 있는 세션. 앞 세션에서 시작한 동작이 늦게 실패해도 새 세션의 목록을 덮지 않게 한다.
  const live = useRef(taskId);
  // 이미 앎·닫기로 뺀 카드. 이후에 도착하는 목록에서도 뺀다 — 조회는 동작보다 먼저 떠났을 수도,
  // 백엔드가 동작을 기록하기 전에 저장소를 읽었을 수도 있다. 카드 id는 AUTOINCREMENT라 다시
  // 쓰이지 않으므로 세션이 바뀌어도 비우지 않는다. 동작이 실패하면 지우고 다시 읽는다.
  const removed = useRef(new Set<number>());

  const load = useCallback(async (id: number) => {
    const current = ++request.current;
    try {
      const cards = await knowCardsList(id);
      if (current !== request.current) return;
      setState({ taskId: id, cards: cards.filter((card) => !removed.current.has(card.id)) });
    } catch {
      // 조회 실패는 표시를 비우지 않는다 — 다음 이벤트에서 다시 읽는다.
    }
  }, []);

  useEffect(() => {
    live.current = taskId;
    if (taskId == null) return;
    let active = true;
    let stop: (() => void) | null = null;
    void load(taskId);
    void Promise.resolve()
      .then(() =>
        listen<{ taskId?: number }>(KNOW_CARDS_CHANGED, (event) => {
          if (active && event.payload?.taskId === taskId) void load(taskId);
        }),
      )
      .then((unlisten) => {
        if (!active) return unlisten();
        stop = unlisten;
      })
      .catch(() => {});
    return () => {
      active = false;
      request.current += 1;
      if (stop) void Promise.resolve(stop()).catch(() => {});
    };
  }, [taskId, load]);

  const act = useCallback(
    (cardId: number, action: KnowCardAction) => {
      if (action !== "asked") {
        // 빼는 동작은 응답을 기다리지 않는다 — 누른 카드가 그대로 있으면 안 눌린 줄 안다.
        // 성공하면 백엔드의 변경 이벤트가 다시 읽게 한다.
        removed.current.add(cardId);
        setState((s) => ({ ...s, cards: s.cards.filter((card) => card.id !== cardId) }));
      }
      knowCardAct(cardId, action).catch(() => {
        // 실패하면 저장된 상태로 되돌린다 — 뺀 카드가 다시 보인다.
        removed.current.delete(cardId);
        if (taskId != null && live.current === taskId) void load(taskId);
      });
    },
    [taskId, load],
  );

  const markSeen = useCallback((cardId: number) => {
    const now = Math.floor(Date.now() / 1000);
    setState((s) =>
      s.cards.some((card) => card.id === cardId && card.seen_at == null)
        ? { ...s, cards: s.cards.map((card) => (card.id === cardId ? { ...card, seen_at: now } : card)) }
        : s,
    );
    knowCardSeen(cardId).catch(() => {});
  }, []);

  const cards = taskId != null && state.taskId === taskId ? state.cards : EMPTY;
  const unread = cards.reduce((n, card) => (card.seen_at == null ? n + 1 : n), 0);
  return useMemo(() => ({ cards, unread, act, markSeen }), [cards, unread, act, markSeen]);
}
