import { useEffect, useRef } from "react";
import type { SkillMeta } from "../../lib/ipc";

interface Props {
  /** 열림 여부 — false면 아무것도 렌더하지 않는다. */
  open: boolean;
  /** 표시할 스킬 목록. */
  items: SkillMeta[];
  /** 현재 하이라이트된 인덱스. */
  sel: number;
  /** 마우스 hover로 인덱스 이동. */
  onHover: (i: number) => void;
  /** 스킬 선택(클릭) — 스킬 이름 전달. */
  onSelect: (name: string) => void;
}

/** /스킬 슬래시 자동완성 드롭다운 — AgentComposer/Composer 공유 프레젠테이셔널 컴포넌트. */
export function SkillDropdown({ open, items, sel, onHover, onSelect }: Props) {
  const listRef = useRef<HTMLDivElement>(null);
  // ↑↓로 옮긴 선택이 max-h 밖으로 나가면 보이는 데까지만 끌어온다(jsdom에는 scrollIntoView가 없다).
  useEffect(() => {
    if (!open) return;
    listRef.current?.querySelector<HTMLElement>('[data-selected="true"]')?.scrollIntoView?.({ block: "nearest" });
  }, [open, sel, items.length]);
  if (!open) return null;
  return (
    <div
      ref={listRef}
      className="absolute bottom-full left-0 mb-1 z-30 w-[32rem] max-h-64 overflow-auto rounded-lg border border-border-strong bg-raised py-1 shadow-xl"
    >
      <div className="px-3 py-1 text-xs text-text-muted">/ 스킬 · Tab으로 완성</div>
      {items.map((sk, i) => (
        <button
          data-selected={i === sel}
          key={sk.name}
          className={`w-full text-left px-3 py-1.5 flex items-baseline gap-2 ${
            i === sel ? "bg-surface text-text" : "text-text-secondary"
          }`}
          onMouseEnter={() => onHover(i)}
          onClick={() => onSelect(sk.name)}
        >
          <span className="font-code text-sm shrink-0">
            /{sk.name}
            {sk.argumentHint && <span className="text-text-muted"> {sk.argumentHint}</span>}
          </span>
          <span className="text-xs text-text-muted truncate flex-1">{sk.description}</span>
          {sk.global && (
            <span className="text-[10px] px-1 py-0.5 rounded bg-raised text-text-muted shrink-0 border border-border">
              global
            </span>
          )}
        </button>
      ))}
    </div>
  );
}
