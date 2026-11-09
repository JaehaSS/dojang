import { useCallback, useEffect, useState, type ReactElement } from "react";

import { quizApprove, quizPending, quizReport, type QuizPendingItem } from "../lib/ipc";
import { Icon } from "./ide/icons";

/**
 * 도메인 문제 검수 (설계 0044 DR-4).
 *
 * 문제와 **근거를 나란히** 보여준다 — 근거 없이는 틀린 문제를 가려낼 수 없고, 그러면 검수는
 * 형식만 남는다. 이 화면 자체가 문서를 다시 읽는 일이라 대기 시간에 하기에 알맞다.
 */
export function QuizReviewPanel({
  onDone,
  doneLabel = "퀴즈로",
}: {
  onDone: () => void;
  /** 되돌아갈 곳이 없으면(검수만 있어서 열렸으면) "닫기"다. */
  doneLabel?: string;
}): ReactElement {
  const [items, setItems] = useState<QuizPendingItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setFailed(false);
    try {
      setItems(await quizPending());
    } catch {
      setFailed(true);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // 판정한 항목은 목록에서 바로 뺀다 — 다시 불러오면 스크롤이 튀어 검수 흐름이 끊긴다.
  const settle = async (id: number, approve: boolean) => {
    try {
      await (approve ? quizApprove(id) : quizReport(id));
      setItems((current) => current.filter((item) => item.id !== id));
    } catch {
      setFailed(true);
    }
  };

  return (
    <section
      aria-label="퀴즈 검수"
      className="flex w-full flex-col gap-3 rounded-lg border border-border bg-surface p-4 text-sm text-text"
    >
      <header className="flex items-center justify-between">
        <span className="text-text-secondary">검수 대기 {items.length}건</span>
        <button
          type="button"
          onClick={onDone}
          className="rounded px-2 py-0.5 text-[11px] text-text-muted hover:bg-raised"
        >
          {doneLabel}
        </button>
      </header>

      {loading ? <p className="text-text-muted">불러오는 중…</p> : null}
      {failed ? <p className="text-status-failed">처리에 실패했습니다.</p> : null}

      {!loading && items.length === 0 ? (
        <p className="leading-relaxed text-text-muted">검수할 문제가 없습니다.</p>
      ) : null}

      <div className="flex max-h-96 flex-col gap-3 overflow-y-auto">
        {items.map((item) => (
          <article key={item.id} className="flex flex-col gap-2 rounded border border-border p-3">
            <p className="leading-relaxed">{item.question}</p>

            {item.choices ? (
              <ul className="flex flex-col gap-0.5 text-[12px] text-text-secondary">
                {item.choices.map((choice) => (
                  <li key={choice} className={`flex items-center gap-1 ${choice === item.answer ? "text-status-done" : ""}`}>
                    <span className="inline-flex w-3 shrink-0">{choice === item.answer && <Icon name="check" size={12} />}</span>
                    {choice}
                  </li>
                ))}
              </ul>
            ) : (
              <p className="text-[12px] text-status-done">정답: {item.answer}</p>
            )}

            {item.explanation ? (
              <p className="text-[12px] leading-relaxed text-text-secondary">{item.explanation}</p>
            ) : null}

            {item.source_excerpt ? (
              <blockquote className="border-l-2 border-border-strong pl-3 text-[12px] leading-relaxed text-text-muted">
                {item.doc_title ? (
                  <span className="mb-1 block text-text-secondary">
                    {item.doc_title}
                    {item.heading ? ` › ${item.heading}` : ""}
                  </span>
                ) : null}
                {item.source_excerpt}
              </blockquote>
            ) : (
              <p className="text-[12px] text-status-failed">출처가 없습니다 — 폐기를 권합니다.</p>
            )}

            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => void settle(item.id, true)}
                className="rounded bg-primary px-3 py-1 text-[12px] text-bg"
              >
                승인
              </button>
              <button
                type="button"
                onClick={() => void settle(item.id, false)}
                className="rounded px-2 py-1 text-[11px] text-text-muted hover:text-status-failed"
              >
                폐기
              </button>
            </div>
          </article>
        ))}
      </div>
    </section>
  );
}
