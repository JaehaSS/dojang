import { Fragment, type ReactNode, useCallback, useEffect, useMemo, useState } from "react";
import { studyExpressionsOn, translateReply, useTranslateSettings, type ReplyTranslation } from "../../lib/translate";
import { ExpressionList, ExpressionMark, useExpressions, useVisibleExpressions } from "./ExpressionStudy";
import { Markdown } from "./Markdown";

/** 원문에 표현 밑줄을 긋는 손잡이 — `Markdown`의 `highlights`·`renderHighlight`로 그대로 넘긴다. */
export interface ReplyHighlights {
  phrases: string[];
  render: (index: number, children: ReactNode) => ReactNode;
}

/** 대화창 ⌥⌘J — 가장 최근 완료된 영어 답변의 번역을 펼치거나 접는다. */
export const isReplyTranslateShortcut = (e: KeyboardEvent) =>
  (e.metaKey || e.ctrlKey) && e.altKey && !e.shiftKey && e.code === "KeyJ";

type Status = { kind: "loading" } | { kind: "done"; value: ReplyTranslation } | { kind: "error"; message: string };

/**
 * 번역할 수 있는 답변 본문과 `번역` 버튼. 펼치면 영어 블록(문단·목록·표) 바로 아래 그 블록의
 * 한국어를 작고 흐리게 붙인다 — 원문을 대체하지 않고, 영어를 먼저 읽고 대조하며 읽게 하는 것이
 * 이 기능의 목적이다. 펼침 여부는 부모가 쥔다(⌥⌘J가 같은 상태를 바꾼다).
 *
 * 대역은 원문을 블록 단위로 다시 그리므로 원문 렌더링도 여기서 한다(`renderMarkdown`).
 * 버튼은 `actions`(`따로 질문`)와 같은 부모 아래 형제로 둔다 — 그 버튼이 부모 요소로 선택 영역을 판정한다.
 */
export function TranslatableReply({
  text,
  open,
  ready,
  onToggle,
  renderMarkdown,
  actions,
}: {
  text: string;
  open: boolean;
  /** 스트리밍 중이면 false — 자라는 버퍼를 번역하지 않는다. */
  ready: boolean;
  onToggle: () => void;
  /** 원문 전체 또는 원문 한 블록을 대화창의 방식(링크 처리 등)대로 그린다. 표현을 짚었으면 밑줄 손잡이가 온다. */
  renderMarkdown: (source: string, highlights?: ReplyHighlights) => ReactNode;
  actions?: ReactNode;
}) {
  const [status, setStatus] = useState<Status>({ kind: "loading" });
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    if (!open || !ready) return;
    let alive = true;
    setStatus({ kind: "loading" });
    translateReply(text).then(
      (value) => alive && setStatus({ kind: "done", value }),
      (error: unknown) => alive && setStatus({ kind: "error", message: String(error) }),
    );
    return () => {
      alive = false;
    };
  }, [open, ready, text, attempt]);

  const shown = open && ready;
  // 표현은 번역과 나란히 부른다 — 번역을 기다린 뒤 부르면 그만큼 늦게 뜬다.
  const study = studyExpressionsOn(useTranslateSettings());
  const expressions = useVisibleExpressions(useExpressions(text, shown && study));
  const phrases = useMemo(() => expressions.map((expr) => expr.phrase), [expressions]);
  const render = useCallback(
    (index: number, children: ReactNode) =>
      expressions[index] ? (
        <ExpressionMark expression={expressions[index]} context={text}>
          {children}
        </ExpressionMark>
      ) : (
        children
      ),
    [expressions, text],
  );
  const highlights = useMemo(() => (phrases.length > 0 ? { phrases, render } : undefined), [phrases, render]);
  const bilingual = shown && status.kind === "done" && status.value.kind === "bilingual" ? status.value.blocks : null;

  return (
    <>
      {bilingual
        ? bilingual.map((block, i) => (
            <Fragment key={i}>
              {renderMarkdown(block.source, highlights)}
              {block.translation !== null && (
                <div aria-label="한국어 번역" className="mb-2 border-l-2 border-primary/40 pl-3 text-sm text-text-secondary">
                  <Markdown text={block.translation} stable />
                </div>
              )}
            </Fragment>
          ))
        : // 번역이 오기 전에는 긋지 않는다 — 대역이 도착하면 블록별로 다시 그려져, 그 사이 고정한 카드가 닫힌다.
          renderMarkdown(text, status.kind === "done" && shown ? highlights : undefined)}
      {shown && <ExpressionList expressions={expressions} context={text} />}
      {actions}
      <button
        type="button"
        disabled={!ready}
        aria-expanded={open}
        title={ready ? "문단별 한국어 번역 펼치기 (⌥⌘J: 마지막 답변)" : "답변이 끝나면 번역할 수 있습니다"}
        className="mt-1 rounded px-1 py-0.5 text-xs text-text-secondary hover:bg-raised hover:text-text disabled:opacity-40 disabled:hover:bg-transparent focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary"
        onMouseDown={(event) => event.preventDefault()}
        onClick={onToggle}
      >
        {open ? "번역 접기" : "번역"}
      </button>
      {shown && !bilingual && (
        <div aria-label="한국어 번역" className="mt-1 rounded-md border-l-2 border-primary/40 bg-raised/60 px-3 py-2 text-sm">
          {status.kind === "loading" && <div role="status" className="text-xs text-text-secondary animate-pulse">번역 중…</div>}
          {status.kind === "error" && (
            <div role="alert" className="text-xs text-status-failed">
              번역하지 못했습니다: {status.message}{" "}
              <button type="button" className="underline" onClick={() => setAttempt((n) => n + 1)}>
                다시 시도
              </button>
            </div>
          )}
          {status.kind === "done" && status.value.kind === "whole" && <Markdown text={status.value.text} stable />}
        </div>
      )}
    </>
  );
}
