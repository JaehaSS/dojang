import { useEffect, useRef, type ReactNode } from "react";
import { Icon } from "./icons";

interface Props {
  /** 팝오버의 접근성 이름. 칩 라벨은 상태에 따라 바뀌지만 이 이름은 고정이다. */
  label: string;
  /** 칩에 적는 한 줄 요약. 좁아지면 잘리므로 전문은 title로 함께 준다. */
  summary: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** 확인이 필요한 상태 — 칩 색만 바꾼다. 스스로 펼치지는 않는다. */
  attention?: boolean;
  children: ReactNode;
}

/**
 * 한 줄 칩 + 위로 열리는 상세 팝오버 (DESIGN.md DetailChip).
 *
 * 승인 바는 대화 열의 높이를 깎는 자리라, 상세를 그 자리에 인라인으로 펼치면 펼친 만큼
 * 대화가 좁아지고 작업 상태에 따라 높이가 들쭉날쭉해진다. 팝오버는 대화를 **덮되 밀지 않으므로**
 * 바의 높이가 상태와 무관하게 일정해진다.
 *
 * `attention`이어도 스스로 열지 않는다 — 드문 상세가 흔한 대화의 자리를 뺏지 않는다(원장 #340).
 * 확인이 필요하다는 사실은 칩 색으로만 말하고, 펼치는 것은 사용자가 정한다.
 */
export function DetailChip({ label, summary, open, onOpenChange, attention, children }: Props) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (event: MouseEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) onOpenChange(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onOpenChange(false);
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open, onOpenChange]);

  return (
    <div className="relative" ref={ref}>
      <button
        type="button"
        aria-expanded={open}
        aria-haspopup="dialog"
        title={summary}
        onClick={() => onOpenChange(!open)}
        className={`flex max-w-[240px] items-center gap-1 rounded-md border px-2 py-1 text-xs ${
          attention
            ? "border-status-failed/50 text-status-failed"
            : "border-border text-text-secondary hover:border-border-strong"
        }`}
      >
        <span role="status" className="min-w-0 truncate">{summary}</span>
        <Icon name="chevronDown" size={12} />
      </button>
      {open && (
        <div
          role="dialog"
          aria-label={label}
          className="absolute bottom-full right-0 z-30 mb-1 max-h-[50vh] w-[min(640px,calc(100vw-2rem))] overflow-auto rounded-lg border border-border-strong bg-raised p-3 text-xs text-text-secondary shadow-e2"
        >
          {children}
        </div>
      )}
    </div>
  );
}
