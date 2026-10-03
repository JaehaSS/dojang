import { useEffect, useRef } from "react";
import { Icon } from "./icons";
import { mentionKey, sourceLabel, type MentionItem } from "../../lib/mention-knowledge";

interface Props {
  /** 열림 여부 — false면 아무것도 렌더하지 않는다. */
  open: boolean;
  /** 표시할 항목 — 파일 경로와 외부 지식이 섞인다(파일이 항상 앞). */
  items: MentionItem[];
  /** 현재 하이라이트된 인덱스. */
  sel: number;
  /** 현재 @토큰 — 헤더에 ` · token`으로 표시. */
  token: string;
  /** 지식 결과를 아직 기다리는 중인지 — 파일 결과는 이미 그려진 상태다. */
  knowledgePending?: boolean;
  /** 마우스 hover로 인덱스 이동. */
  onHover: (i: number) => void;
  /** 항목 선택(클릭). */
  onSelect: (item: MentionItem) => void;
}

/** @멘션 자동완성 드롭다운 — AgentComposer/Composer 공유 프레젠테이셔널 컴포넌트. */
export function MentionDropdown({
  open,
  items,
  sel,
  token,
  knowledgePending,
  onHover,
  onSelect,
}: Props) {
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
      className="absolute bottom-full left-0 mb-1 z-30 w-[28rem] max-h-56 overflow-auto rounded-lg border border-border-strong bg-raised py-1 shadow-xl"
    >
      <div className="px-3 py-1 text-xs text-text-muted flex items-center gap-1">
        <Icon name="at" size={12} /> 멘션{token && ` · ${token}`}
        {knowledgePending && <span className="ml-auto text-text-muted">지식 검색 중…</span>}
      </div>
      {items.map((item, i) => (
        <button
          data-selected={i === sel}
          key={mentionKey(item)}
          className={`w-full text-left px-3 py-1 text-sm flex items-center gap-2 ${
            i === sel ? "bg-surface text-text" : "text-text-secondary"
          }`}
          onMouseEnter={() => onHover(i)}
          onClick={() => onSelect(item)}
        >
          {item.kind === "file" ? (
            <span className="font-code truncate">{item.path}</span>
          ) : (
            <>
              {/* 출처 배지 — 어디서 온 지식인지 모르면 결과를 신뢰할 수 없다. */}
              <span className="shrink-0 rounded px-1.5 py-0.5 text-[10px] bg-surface text-text-muted">
                {sourceLabel(item.hit.source)}
              </span>
              <span className="truncate">{item.hit.title}</span>
              {item.hit.heading && (
                <span className="shrink-0 text-xs text-text-muted">§ {item.hit.heading}</span>
              )}
            </>
          )}
        </button>
      ))}
    </div>
  );
}
