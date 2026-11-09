import { useEffect, useRef, useState } from "react";
import { Icon } from "./icons";
import { reasoningEffortsForModel } from "../../lib/models";

interface Props {
  agent: string;
  model: string;
  /** 이 세션에만 적용할 reasoning effort 오버라이드 ("" = CLI/설정 기본). */
  effort: string;
  onChange: (effort: string) => void;
}

/** 세션 단위 reasoning effort 칩 — ModelPicker와 같은 ComposerChip 패턴(DESIGN.md).
 *  비워두면 CLI 설정 기본값을 따른다. */
export function EffortPicker({ agent, model, effort, onChange }: Props) {
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

  const options = reasoningEffortsForModel(agent, model);
  const overridden = effort.trim().length > 0;
  const chipLabel = overridden ? `Effort: ${effort}` : "Effort: 기본";

  const pick = (value: string) => {
    onChange(value);
    setOpen(false);
  };

  return (
    <div className="relative" ref={ref}>
      <button
        aria-label="Reasoning effort"
        className={`flex items-center gap-1 text-xs border rounded-md px-2 py-1 hover:border-border-strong ${
          overridden ? "text-primary-bright border-primary/50" : "text-text-secondary border-border"
        }`}
        onClick={() => setOpen((v) => !v)}
        title={
          overridden
            ? `이 세션의 reasoning effort: ${effort}`
            : "Reasoning effort: 기본값 — CLI 설정을 따름"
        }
      >
        <Icon name="chart" size={14} />
        <span className="max-w-[120px] truncate">{chipLabel}</span>
        <Icon name="chevronDown" size={12} />
      </button>

      {open && (
        <div className="absolute bottom-full left-0 mb-1 z-20 w-48 rounded-lg border border-border-strong bg-raised py-1 shadow-e2">
          <div className="px-3 py-1 text-xs text-text-muted">
            Effort <span className="text-text-secondary">— 이 세션에만 적용</span>
          </div>
          <button
            className={`w-full text-left flex items-center gap-2 px-2 py-1.5 text-sm hover:bg-surface ${
              !overridden ? "text-text" : "text-text-secondary"
            }`}
            onClick={() => pick("")}
            title="CLI 설정의 기본 reasoning effort를 따름"
          >
            <span className={`shrink-0 w-4 ${!overridden ? "text-primary-bright" : "text-text-muted"}`}>
              <Icon name={!overridden ? "check" : "chart"} size={14} />
            </span>
            <span className="truncate flex-1">기본 (설정값)</span>
          </button>
          {options.map((option) => {
            const on = effort === option;
            return (
              <button
                key={option}
                className={`w-full text-left flex items-center gap-2 px-2 py-1.5 text-sm hover:bg-surface ${
                  on ? "text-text" : "text-text-secondary"
                }`}
                onClick={() => pick(option)}
              >
                <span className={`shrink-0 w-4 ${on ? "text-primary-bright" : "text-text-muted"}`}>
                  <Icon name={on ? "check" : "chart"} size={14} />
                </span>
                <span className="truncate flex-1">{option}</span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
