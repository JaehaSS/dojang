import { useCallback, useEffect, useState } from "react";
import {
  deleteVocab,
  dueVocab,
  listVocab,
  reviewVocab,
  setVocabStatus,
  type VocabEntry,
} from "../../lib/vocab";

/**
 * 단어장 — ⌘J·답변 번역에서 저장한 표현을 간격 반복으로 복습한다(`src-tauri/src/vocab.rs`).
 * 복습은 예문을 보고 뜻을 떠올린 뒤 스스로 채점한다. 뜻풀이는 문자열로 맞출 수 없다.
 */
export function VocabView() {
  const [tab, setTab] = useState<"review" | "list">("review");
  const tabCls = (on: boolean) =>
    `rounded px-2 py-1 text-xs ${on ? "bg-raised text-text" : "text-text-secondary hover:text-text"}`;
  return (
    <div className="flex h-full min-h-0 flex-col overflow-auto p-6">
      <div className="mx-auto w-full max-w-2xl">
        <div className="mb-4 flex items-center gap-2">
          <h1 className="flex-1 text-lg font-semibold">단어장</h1>
          <button type="button" className={tabCls(tab === "review")} onClick={() => setTab("review")}>
            복습
          </button>
          <button type="button" className={tabCls(tab === "list")} onClick={() => setTab("list")}>
            전체 목록
          </button>
        </div>
        {tab === "review" ? <ReviewDeck /> : <EntryList />}
      </div>
    </div>
  );
}

function ReviewDeck() {
  const [cards, setCards] = useState<VocabEntry[] | null>(null);
  const [revealed, setRevealed] = useState(false);
  const [done, setDone] = useState(0);
  const [grading, setGrading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    setError(null);
    dueVocab().then(
      (due) => {
        setCards(due);
        setRevealed(false);
      },
      (e: unknown) => setError(String(e)),
    );
  }, []);
  useEffect(load, [load]);

  const card = cards?.[0];
  const grade = (remembered: boolean) => {
    // 응답 전에 또 누르면 같은 카드가 두 번 채점되어 상자가 두 칸 오르고 다음 카드가 빠진다.
    if (!card || grading) return;
    setGrading(true);
    reviewVocab(card.id, remembered).then(
      () => {
        setCards((rest) => (rest?.[0]?.id === card.id ? rest.slice(1) : rest));
        setRevealed(false);
        setDone((n) => n + 1);
        setGrading(false);
      },
      (e: unknown) => {
        setGrading(false);
        setError(String(e));
      },
    );
  };

  if (error) return <div role="alert" className="text-sm text-status-failed">단어장을 읽지 못했습니다: {error}</div>;
  if (!cards) return <div className="text-sm text-text-secondary">불러오는 중…</div>;
  if (!card) {
    return (
      <div className="rounded-md border border-border p-6 text-center text-sm text-text-secondary">
        {done > 0 ? `${done}개를 복습했습니다. 지금 복습할 표현은 더 없습니다.` : "지금 복습할 표현이 없습니다."}
        <div className="mt-1 text-xs text-text-muted">
          답변 번역이나 ⌘J 결과에서 밑줄 친 표현에 마우스를 올려 단어장에 저장하세요.
        </div>
        <button type="button" className="mt-3 text-xs text-primary-bright underline" onClick={load}>
          다시 확인
        </button>
      </div>
    );
  }
  return (
    <div aria-label="복습 카드" className="rounded-md border border-border p-5">
      <div className="mb-3 text-[11px] text-text-muted">남은 카드 {cards.length}개</div>
      <div className="text-xl font-semibold">{card.phrase}</div>
      {card.example && <div className="mt-2 text-sm text-text-secondary">{card.example}</div>}
      {revealed ? (
        <>
          <div className="mt-4 border-t border-border pt-3 text-base">{card.meaning}</div>
          {card.note && <div className="mt-1 text-xs text-text-secondary">{card.note}</div>}
          <div className="mt-4 flex gap-2">
            <button type="button" disabled={grading} className="rounded bg-primary px-3 py-1.5 text-sm text-bg disabled:opacity-50" onClick={() => grade(true)}>
              기억났어요
            </button>
            <button type="button" disabled={grading} className="rounded border border-border px-3 py-1.5 text-sm disabled:opacity-50" onClick={() => grade(false)}>
              다시 볼게요
            </button>
          </div>
        </>
      ) : (
        <button
          type="button"
          className="mt-4 rounded border border-border px-3 py-1.5 text-sm hover:bg-raised"
          onClick={() => setRevealed(true)}
        >
          뜻 보기
        </button>
      )}
    </div>
  );
}

const DAY_MS = 24 * 60 * 60 * 1000;

function dueLabel(entry: VocabEntry, nowMs: number): string {
  if (entry.status === "known") return "알아요";
  const days = Math.ceil((entry.due_at * 1000 - nowMs) / DAY_MS);
  return days <= 0 ? "복습할 때" : `${days}일 뒤 복습`;
}

function EntryList() {
  const [entries, setEntries] = useState<VocabEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(() => {
    listVocab().then(setEntries, (e: unknown) => setError(String(e)));
  }, []);
  useEffect(load, [load]);
  const run = (action: Promise<unknown>) => action.then(load, (e: unknown) => setError(String(e)));

  if (error) return <div role="alert" className="text-sm text-status-failed">단어장을 읽지 못했습니다: {error}</div>;
  if (!entries) return <div className="text-sm text-text-secondary">불러오는 중…</div>;
  if (entries.length === 0) return <div className="text-sm text-text-secondary">아직 저장한 표현이 없습니다.</div>;
  const now = Date.now();
  return (
    <ul aria-label="저장한 표현" className="divide-y divide-border rounded-md border border-border">
      {entries.map((entry) => (
        <li key={entry.id} className="flex items-start gap-3 px-3 py-2">
          <div className="min-w-0 flex-1">
            <div className="text-sm">
              <span className="font-medium">{entry.phrase}</span>
              {entry.meaning && <span className="text-text-secondary"> · {entry.meaning}</span>}
            </div>
            {entry.example && <div className="mt-0.5 truncate text-xs text-text-muted">{entry.example}</div>}
          </div>
          <span className="shrink-0 text-[11px] text-text-muted">{dueLabel(entry, now)}</span>
          <button
            type="button"
            className="shrink-0 text-[11px] text-text-secondary hover:text-text"
            onClick={() => run(setVocabStatus(entry, entry.status === "known" ? "learning" : "known"))}
          >
            {entry.status === "known" ? "다시 복습" : "알아요"}
          </button>
          <button
            type="button"
            aria-label={`${entry.phrase} 삭제`}
            className="shrink-0 text-[11px] text-text-muted hover:text-status-failed"
            onClick={() => run(deleteVocab(entry))}
          >
            삭제
          </button>
        </li>
      ))}
    </ul>
  );
}
