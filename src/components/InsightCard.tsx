import { useCallback, useEffect, useState } from "react";

import { insightNext, type InsightCard as Card } from "../lib/ipc";
import { MetaTag } from "./MetaTag";
import { Icon } from "./ide/icons";
import { IconButton } from "./ui";

interface Props {
  onClose: () => void;
}

/**
 * 대기 인사이트 카드 — 에이전트를 기다리는 동안 도메인 지식 한 조각을 읽는다.
 *
 * `QuizPanel`과 **같은 자리·같은 크기**다. 대기 화면이 무엇을 띄우든 배치가 흔들리면
 * 사용자는 매번 눈으로 다시 찾아야 한다.
 *
 * **출처를 접거나 숨기지 않는다.** 접으면 아무도 펴지 않고, 그러면 출처를 필수로 만든
 * 이유가 사라진다(설계 0044의 `source_excerpt`와 같은 규약).
 */
export function InsightCard({ onClose }: Props) {
  const [card, setCard] = useState<Card | null>(null);
  const [busy, setBusy] = useState(true);
  const [failed, setFailed] = useState(false);

  const load = useCallback(async () => {
    setBusy(true);
    setFailed(false);
    try {
      setCard(await insightNext());
    } catch {
      setFailed(true);
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <section
      aria-label="대기 인사이트"
      className="flex w-full flex-col gap-3 rounded-lg border border-border bg-surface p-4 text-sm text-text"
    >
      <header className="flex items-center justify-between gap-2">
        {/* 덱 이름은 상태가 아니라 분류다 — Badge가 아니라 MetaTag다(DESIGN.md). */}
        {card ? <MetaTag>{card.deck}</MetaTag> : <span className="text-text-secondary">대기 인사이트</span>}
        <IconButton size="sm" label="인사이트 닫기" onClick={onClose}>
          <Icon name="x" size={14} />
        </IconButton>
      </header>

      {busy ? (
        <p className="text-text-muted">불러오는 중…</p>
      ) : failed ? (
        <p className="text-text-secondary">카드를 불러오지 못했습니다.</p>
      ) : card ? (
        <>
          <h3 className="text-base font-medium text-text">{card.title}</h3>
          <p className="leading-relaxed text-text-secondary">{card.body}</p>
          <footer className="flex items-end justify-between gap-3 pt-1">
            {/* 출처는 항상 보인다. 이것이 이 화면을 믿을 수 있게 하는 유일한 장치다. */}
            <span className="min-w-0 text-xs text-text-muted">
              출처 · <span className="break-words">{card.source}</span>
            </span>
            <button
              className="shrink-0 rounded-md border border-border px-2 py-1 text-xs text-text-secondary hover:border-border-strong"
              onClick={() => void load()}
            >
              다음 →
            </button>
          </footer>
        </>
      ) : (
        <p className="text-text-secondary">띄울 카드가 없습니다.</p>
      )}
    </section>
  );
}
