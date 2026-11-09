import { useEffect, useRef, useState } from "react";
import { Icon } from "./icons";
import { IconButton } from "../ui";

export interface MenuItem {
  label: string;
  onClick: () => void;
  danger?: boolean;
}

/** ⋮ 더보기 메뉴 — 버튼 클릭 시 아래로 드롭다운, 바깥 클릭 시 닫힘. */
export function Menu({ items, label = "더보기" }: { items: MenuItem[]; label?: string }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [open]);

  return (
    <div className="relative" ref={ref}>
      <IconButton label={label} expanded={open} aria-haspopup="menu" onClick={() => setOpen((v) => !v)}>
        <Icon name="more" size={16} />
      </IconButton>
      {open && (
        <div className="absolute top-full right-0 mt-1 z-20 w-40 rounded-lg border border-border-strong bg-raised py-1 shadow-e2">
          {items.map((it) => (
            <button
              key={it.label}
              className={`w-full text-left px-3 py-1.5 text-sm hover:bg-surface ${
                it.danger ? "text-status-failed" : "text-text-secondary"
              }`}
              onClick={() => {
                setOpen(false);
                it.onClick();
              }}
            >
              {it.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
