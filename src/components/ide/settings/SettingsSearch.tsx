import { useEffect, useRef, useState } from "react";
import { searchSettings, tabLabel, type SettingEntry } from "./settings-catalog";

/**
 * 설정 검색 — 탭을 몰라도 이름으로 항목에 닿게 한다.
 * 고르면 패널이 탭을 바꾸고 항목 id를 강조 대상으로 넘긴다. 스크롤은 행이 스스로 한다.
 */
export function SettingsSearch({ onPick }: { onPick: (entry: SettingEntry) => void }) {
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [cursor, setCursor] = useState(0);
  const boxRef = useRef<HTMLDivElement>(null);

  const results = searchSettings(query);

  useEffect(() => {
    setCursor(0);
  }, [query]);

  // 바깥을 누르면 닫는다 — 결과 목록이 탭 내용을 덮은 채 남지 않게.
  useEffect(() => {
    if (!open) return;
    const onDown = (event: MouseEvent) => {
      if (!boxRef.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const pick = (entry: SettingEntry) => {
    onPick(entry);
    setQuery("");
    setOpen(false);
  };

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === "Escape") {
      setQuery("");
      setOpen(false);
      return;
    }
    if (results.length === 0) return;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setCursor((index) => (index + 1) % results.length);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setCursor((index) => (index - 1 + results.length) % results.length);
    } else if (event.key === "Enter") {
      event.preventDefault();
      pick(results[cursor] ?? results[0]);
    }
  };

  return (
    <div ref={boxRef} className="relative w-56 shrink-0">
      <input
        type="search"
        aria-label="설정 검색"
        placeholder="설정 검색"
        className="w-full rounded-md border border-border bg-bg px-2 py-1 text-xs text-text outline-none focus:border-primary"
        value={query}
        onChange={(event) => {
          setQuery(event.target.value);
          setOpen(true);
        }}
        onFocus={() => setOpen(true)}
        onKeyDown={onKeyDown}
      />
      {open && query.trim() !== "" && (
        <div className="absolute right-0 top-full z-20 mt-1 w-72 overflow-hidden rounded-md border border-border bg-surface shadow-e2">
          {results.length === 0 ? (
            <div className="px-3 py-2 text-xs text-text-muted">찾는 설정이 없습니다.</div>
          ) : (
            results.map((entry, index) => (
              <button
                key={entry.id}
                onMouseEnter={() => setCursor(index)}
                onClick={() => pick(entry)}
                className={`block w-full px-3 py-2 text-left ${
                  index === cursor ? "bg-raised" : ""
                }`}
              >
                <div className="flex items-baseline gap-2">
                  <span className="truncate text-xs text-text">{entry.label}</span>
                  <span className="ml-auto shrink-0 text-[11px] text-text-muted">
                    {tabLabel(entry.tab)}
                  </span>
                </div>
                {entry.hint && (
                  <div className="truncate text-[11px] text-text-muted">{entry.hint}</div>
                )}
              </button>
            ))
          )}
        </div>
      )}
    </div>
  );
}
