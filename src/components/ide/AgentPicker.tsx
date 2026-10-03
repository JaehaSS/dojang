import { useEffect, useRef, useState } from "react";
import { Icon } from "./icons";
import { AGENT_PRESETS, labelFor } from "../../lib/agents";

interface Props {
  /** 선택된 에이전트. 1.0에서는 항상 1개다. */
  agents: string[];
  onChange: (agents: string[]) => void;
}

/** 리드 에이전트 선택 — 프리셋 + 임의 CLI용 커스텀 명령. Praxis는 에이전트 비종속.
 *  여러 에이전트를 골라 비교 실행하던 앙상블 진입점은 1.0에서 뺐다(원장 #548). 엔진·EnsembleView는
 *  남아 있어 이미 만든 비교는 홈의 최근 목록에서 연다. */
export function AgentPicker({ agents, onChange }: Props) {
  const [openMenu, setOpenMenu] = useState(false);
  const [custom, setCustom] = useState("");
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!openMenu) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpenMenu(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [openMenu]);

  const addCustom = () => {
    const c = custom.trim();
    if (!c) return;
    onChange([c]);
    setCustom("");
    setOpenMenu(false);
  };

  const current = agents[0] ?? "claude";

  return (
    <div className="relative" ref={ref}>
      <button
        className="flex items-center gap-1 text-xs border rounded-md px-2 py-1 hover:border-border-strong text-text-secondary border-border"
        onClick={() => setOpenMenu((v) => !v)}
        title={`리드 에이전트: ${current}`}
      >
        <Icon name="sparkle" size={13} />
        <span className="max-w-[170px] truncate">{labelFor(current)}</span>
        <Icon name="chevronDown" size={12} />
      </button>

      {openMenu && (
        <div className="absolute bottom-full left-0 mb-1 z-20 w-72 rounded-lg border border-border-strong bg-raised py-1 shadow-xl">
          <div className="px-3 py-1 text-xs text-text-muted">에이전트</div>
          {AGENT_PRESETS.map((p) => {
            const on = current === p.key;
            return (
              <div
                key={p.key}
                className={`w-full flex items-center px-2 py-1.5 text-sm hover:bg-surface ${
                  on ? "text-text" : "text-text-secondary"
                }`}
              >
                <button
                  className="flex items-center gap-2 flex-1 min-w-0 text-left"
                  onClick={() => { onChange([p.key]); setOpenMenu(false); }}
                >
                  <span className={`shrink-0 w-4 ${on ? "text-primary-bright" : "text-text-muted"}`}>
                    <Icon name={on ? "check" : "sparkle"} size={13} />
                  </span>
                  <span className="truncate flex-1">{p.label}</span>
                  <span className="text-text-muted text-xs font-code shrink-0">{p.key}</span>
                </button>
              </div>
            );
          })}
          {/* 선택된 커스텀 명령(있으면) */}
          {agents.slice(0, 1).filter((a) => !AGENT_PRESETS.some((p) => p.key === a)).map((c) => (
            <div
              key={c}
              className="w-full text-left flex items-center gap-2 px-3 py-1.5 text-sm text-text"
            >
              <span className="shrink-0 w-4 text-primary-bright">
                <Icon name="check" size={13} />
              </span>
              <span className="truncate flex-1 font-code text-xs">{c}</span>
              <span className="text-text-muted text-xs">커스텀</span>
            </div>
          ))}
          <div className="my-1 border-t border-border" />
          <div className="px-3 py-1.5">
            <div className="text-xs text-text-muted mb-1">커스텀 명령 추가</div>
            <input
              className="w-full bg-bg border border-border rounded px-2 py-1 text-sm text-text outline-none focus:border-primary placeholder:text-text-muted"
              placeholder="예: crush   ·   mybin -p {prompt}"
              value={custom}
              onChange={(e) => setCustom(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") addCustom();
              }}
            />
            <button
              className="mt-1 text-xs text-primary-bright disabled:text-text-muted"
              disabled={!custom.trim()}
              onClick={addCustom}
            >
              + 추가
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
