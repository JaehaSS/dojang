import { useEffect, useRef, useState } from "react";
import { Icon } from "./icons";

export interface BubbleAnchor {
  top: number;
  left: number;
  placement: "below" | "above";
  startLine: number;
  endLine: number;
}

interface Props {
  anchor: BubbleAnchor;
  /** 세션이 응답 중이면 보낼 수 없다 — 큐잉하면 언제 갈지 모르는 메시지를 기다리게 된다. */
  busy: boolean;
  /** 전송 실패 사유. 있으면 버블을 유지한 채 보여 준다. */
  error: string | null;
  onSubmit: (question: string) => void;
  /** ⌘L과 같은 동작 — 첨부만 하고 닫는다. */
  onAttachOnly: () => void;
  onClose: () => void;
  onAskSeparately?: () => void;
}

const BUBBLE_HEIGHT = 76;

/**
 * 드래그한 코드 옆에 떠서 그 자리에서 세션에 질문하게 하는 버블.
 *
 * ⌘L(선택 첨부)은 이 기능이 생기기 전부터 있었지만 시각적 단서가 없어 모르면 쓸 수 없었다.
 * 이 버블은 그 동선을 대체하는 것이 아니라 **보이게 만든다** — 보조 액션이 정확히 ⌘L이다.
 *
 * **두 단계로 뜬다.** 처음에는 칩 하나이고, 질문을 누를 때만 입력창이 펼쳐진다.
 * 처음부터 입력창을 띄우고 포커스까지 가져가던 때에는 **드래그한 코드를 복사할 수 없었다** —
 * Monaco의 선택은 화면에 남지만 키보드의 주인은 이미 이 입력창이라, ⌘C가 빈 입력창을
 * 복사했다. 코드를 고르는 이유는 대개 복사·편집이고 질문은 그보다 드물다. 드문 쪽이
 * 흔한 쪽의 포커스를 뺏으면 안 된다.
 */
export function SelectionAskBubble({
  anchor,
  busy,
  error,
  onSubmit,
  onAttachOnly,
  onClose,
  onAskSeparately,
}: Props) {
  const [question, setQuestion] = useState("");
  /** 입력창을 펼쳤는가. 펼치기 전에는 포커스를 건드리지 않는다. */
  const [expanded, setExpanded] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  // 새 선택은 언제나 접힌 채로 시작한다 — 드래그가 끝날 때마다 포커스를 뺏기면
  // 이어서 ⌘C를 누르는 손이 매번 빈 클립보드를 얻는다.
  useEffect(() => {
    setExpanded(false);
  }, [anchor.startLine, anchor.endLine]);

  // 펼친 것은 사용자의 뜻이므로 그때는 포커스를 가져간다.
  useEffect(() => {
    if (expanded) inputRef.current?.focus();
  }, [expanded]);

  const lineLabel =
    anchor.startLine === anchor.endLine
      ? `L${anchor.startLine}`
      : `L${anchor.startLine}–L${anchor.endLine}`;

  const submit = () => {
    if (busy || question.trim().length === 0) return;
    onSubmit(question);
    setQuestion("");
  };

  const position = {
    top: anchor.placement === "below" ? anchor.top : anchor.top - BUBBLE_HEIGHT,
    left: anchor.left,
  };

  if (!expanded) {
    return (
      <div
        role="dialog"
        aria-label="선택한 코드로 질문"
        className="absolute z-30 flex items-center gap-1.5 rounded-full border border-border-strong bg-raised px-2 py-1 text-[11px] shadow-e2"
        style={position}
      >
        <span className="text-text-muted">{lineLabel}</span>
        <button className="inline-flex items-center gap-1 text-text-secondary hover:text-text" onClick={() => setExpanded(true)}>
          <Icon name="chat" size={12} />
          질문
        </button>
        {onAskSeparately && <button className="text-text-secondary hover:text-text" onClick={onAskSeparately}>따로 질문</button>}
        <button className="text-text-muted hover:text-text" onClick={onAttachOnly}>
          ⌘L 첨부
        </button>
        <button className="inline-flex items-center px-0.5 text-text-muted hover:text-text" onClick={onClose} aria-label="질문 닫기" title="닫기">
          <Icon name="x" size={12} />
        </button>
      </div>
    );
  }

  return (
    <div
      role="dialog"
      aria-label="선택한 코드로 질문"
      className="absolute z-30 w-80 rounded-lg border border-border-strong bg-raised p-2 shadow-e2"
      style={position}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onClose();
        }
      }}
    >
      <div className="flex items-center gap-1 text-[11px] text-text-muted">
        <span className="inline-flex items-center gap-1"><Icon name="chat" size={12} />{lineLabel}</span>
        <button
          className="ml-auto inline-flex items-center px-1 hover:text-text"
          onClick={onClose}
          aria-label="질문 닫기"
          title="닫기 (Esc)"
        >
          <Icon name="x" size={12} />
        </button>
      </div>
      <textarea
        ref={inputRef}
        className="mt-1 w-full resize-none rounded border border-border bg-base px-2 py-1 text-xs outline-none focus:border-accent disabled:opacity-50"
        rows={2}
        value={question}
        disabled={busy}
        placeholder={busy ? "응답 중 — 끝나면 보낼 수 있습니다" : "무엇이든 물어보세요"}
        onChange={(e) => setQuestion(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            submit();
          }
        }}
      />
      {error && (
        <div className="mt-1 text-[11px] text-danger" role="alert">
          {error}
        </div>
      )}
      <div className="mt-1 flex items-center gap-2 text-[11px]">
        {onAskSeparately && <button className="text-text-secondary hover:text-text" onClick={onAskSeparately}>따로 질문</button>}
        <button className="text-text-muted hover:text-text" onClick={onAttachOnly}>
          ⌘L 첨부만 하고 닫기
        </button>
        <button
          className="ml-auto rounded bg-accent px-2 py-0.5 text-base-inverse disabled:opacity-40"
          onClick={submit}
          disabled={busy || question.trim().length === 0}
        >
          보내기 ⏎
        </button>
      </div>
    </div>
  );
}
