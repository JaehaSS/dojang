import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import {
  type Expression,
  exampleSentence,
  extractExpressions,
  markExpressionKnown,
  phraseKey,
  saveExpression,
  segmentByExpressions,
  useVocabMarks,
} from "../../lib/vocab";

/**
 * 영어 표현 공부 — 모델이 짚은 표현에 점선 밑줄을 긋고, 올리면 뜻 카드를 띄운다.
 * 카드에서 단어장에 저장하거나 "알아요"로 다음부터 짚지 않게 한다. 원천: `src/lib/vocab.ts`.
 */

/** 표현을 가져온다. 꺼져 있거나 실패하면 빈 목록 — 표현은 덤이라 오류로 번역을 가리지 않는다. */
export function useExpressions(text: string | null, enabled: boolean): Expression[] {
  const [state, setState] = useState<{ text: string; expressions: Expression[] } | null>(null);
  useEffect(() => {
    if (!enabled || !text?.trim()) return;
    let alive = true;
    extractExpressions(text).then(
      (expressions) => alive && setState({ text, expressions }),
      () => alive && setState({ text, expressions: [] }),
    );
    return () => {
      alive = false;
    };
  }, [text, enabled]);
  return enabled && state && state.text === text ? state.expressions : NONE;
}

const NONE: Expression[] = [];

/**
 * "알아요"한 표현은 그 자리에서 밑줄과 목록에서 빠진다. 남는 표현이 같으면 같은 배열을 준다 —
 * 저장만 해도 배열이 바뀌면 `Markdown`이 다시 그려져 열린 카드가 닫힌다.
 */
export function useVisibleExpressions(expressions: Expression[]): Expression[] {
  const marks = useVocabMarks();
  const hidden = expressions
    .filter((expr) => marks.get(phraseKey(expr.phrase)) === "known")
    .map((expr) => phraseKey(expr.phrase))
    .join("\n");
  return useMemo(
    () => (hidden ? expressions.filter((expr) => !hidden.split("\n").includes(phraseKey(expr.phrase))) : expressions),
    [expressions, hidden],
  );
}

const CLOSE_DELAY_MS = 150;

/** 표현 한 자리 — 올리면 카드를 잠깐 보이고, 누르면 고정한다. */
export function ExpressionMark({
  expression,
  context,
  children,
  chip = false,
}: {
  expression: Expression;
  /** 예문을 뽑을 원문. */
  context: string;
  children: ReactNode;
  /** 목록의 칩으로 그린다(본문 밑줄이 아니라). */
  chip?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [pinned, setPinned] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const cancelClose = () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
  };
  const closeSoon = () => {
    cancelClose();
    if (!pinned) timer.current = setTimeout(() => setOpen(false), CLOSE_DELAY_MS);
  };
  useEffect(() => cancelClose, []);

  const markCls = chip
    ? "cursor-pointer rounded border border-border px-1.5 py-0.5 text-xs text-text hover:bg-raised"
    : "cursor-pointer rounded-sm bg-transparent text-inherit underline decoration-primary/60 decoration-dotted underline-offset-4 hover:bg-primary/10";

  return (
    <span className="relative inline" onMouseEnter={() => (cancelClose(), setOpen(true))} onMouseLeave={closeSoon}>
      <mark
        role="button"
        tabIndex={0}
        aria-label={`표현: ${expression.phrase}`}
        aria-expanded={open}
        className={markCls}
        onClick={() => {
          setPinned((p) => !p);
          setOpen(true);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            setPinned((p) => !p);
            setOpen(true);
          } else if (e.key === "Escape") {
            setPinned(false);
            setOpen(false);
          }
        }}
      >
        {children}
      </mark>
      {open && (
        <ExpressionCard
          expression={expression}
          context={context}
          onClose={() => {
            setPinned(false);
            setOpen(false);
          }}
        />
      )}
    </span>
  );
}

function ExpressionCard({ expression, context, onClose }: { expression: Expression; context: string; onClose: () => void }) {
  const mark = useVocabMarks().get(phraseKey(expression.phrase));
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const act = (run: () => Promise<unknown>) => {
    setBusy(true);
    run().then(
      () => (setBusy(false), setError(null)),
      (e: unknown) => (setBusy(false), setError(String(e))),
    );
  };
  const btn = "rounded px-1.5 py-0.5 text-[11px] hover:bg-raised disabled:opacity-50";
  return (
    <span
      role="dialog"
      aria-label={`${expression.phrase} 뜻`}
      className="absolute left-0 top-full z-50 mt-1 block w-72 cursor-default rounded-md border border-border bg-bg p-2.5 text-left text-sm font-normal not-italic text-text shadow-lg"
      onMouseDown={(e) => e.stopPropagation()}
    >
      <span className="flex items-start gap-2">
        <span className="min-w-0 flex-1 font-semibold">{expression.phrase}</span>
        <button type="button" aria-label="카드 닫기" className="text-xs text-text-muted hover:text-text" onClick={onClose}>
          ✕
        </button>
      </span>
      <span className="mt-0.5 block">{expression.meaning}</span>
      {expression.note && <span className="mt-1 block text-xs text-text-secondary">{expression.note}</span>}
      <span className="mt-2 flex items-center gap-1 border-t border-border pt-1.5">
        <button
          type="button"
          disabled={busy || mark === "saved"}
          className={`${btn} text-primary-bright`}
          onClick={() => act(() => saveExpression(expression, exampleSentence(context, expression.phrase)))}
        >
          {mark === "saved" ? "단어장에 저장됨" : "단어장에 저장"}
        </button>
        <button
          type="button"
          disabled={busy}
          title="다음부터 이 표현을 짚지 않습니다"
          className={`${btn} text-text-secondary`}
          onClick={() => act(() => markExpressionKnown(expression))}
        >
          알아요
        </button>
      </span>
      {error && <span className="mt-1 block text-[11px] text-status-failed">{error}</span>}
    </span>
  );
}

/** 평문 안의 표현에 밑줄을 긋는다 — ⌘J 영어 제안처럼 마크다운이 아닌 글에 쓴다. */
export function HighlightedText({ text, expressions }: { text: string; expressions: Expression[] }) {
  return (
    <>
      {segmentByExpressions(text, expressions).map((segment, i) =>
        typeof segment === "string" ? (
          segment
        ) : (
          <ExpressionMark key={i} expression={expressions[segment.index]} context={text}>
            {segment.text}
          </ExpressionMark>
        ),
      )}
    </>
  );
}

/**
 * 표현 목록 — 본문에 밑줄을 긋지 못한 표현(서식 경계를 넘거나, 바꿔 넣어 입력창에 들어간 글)도
 * 여기서는 빠짐없이 본다.
 */
export function ExpressionList({ expressions, context }: { expressions: Expression[]; context: string }) {
  if (expressions.length === 0) return null;
  return (
    <div aria-label="공부할 표현" className="mt-1 flex flex-wrap items-center gap-1 text-[11px] text-text-muted">
      <span>표현</span>
      {expressions.map((expr) => (
        <ExpressionMark key={expr.phrase} expression={expr} context={context} chip>
          {expr.phrase}
        </ExpressionMark>
      ))}
    </div>
  );
}
