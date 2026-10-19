import { useCallback, useEffect, useId, useMemo, useRef, useState, type ReactElement } from "react";
import { buildContextReport, TOOL_GROUP_LABELS, type ContextReportEvent } from "../../lib/context-breakdown";
import type { ContextObservation } from "../../lib/context-observation";
import { contextPercent, contextWindowFor } from "../../lib/context-window";
import { ChannelCard } from "./ChannelCard";
import { ContextDetails } from "./ContextDetails";
import { Icon } from "./icons";

export interface ContextChannelBinding {
  agent: string | null | undefined;
  model?: string | null;
  observation: ContextObservation | null;
  events: readonly ContextReportEvent[];
  busy: boolean;
  onRefresh?: () => Promise<void>;
  /** The compact channel opens the existing activity column for details. */
  onOpenDetails: () => void;
  focusRequest?: number;
  onFocusHandled?: (request: number) => void;
}

interface Props {
  context: ContextChannelBinding;
  density: "rail" | "panel";
  /** Retained but hidden tabs must neither refresh nor receive focus. */
  active?: boolean;
}

const number = (value: number): string => value.toLocaleString("ko-KR");
const sourceLabel = (source: string): string => ({
  codex_session: "Codex 세션 로그", app_server: "Codex 실행 이벤트",
  claude_message: "Claude 응답", claude_legacy: "Claude 이전 기록",
  agy_step: "Antigravity 응답",
}[source] ?? "실행 기록");

/** Same records in both placements: a compact middle card and a sticky panel summary. */
export function ContextChannel({ context, density, active = true }: Props): ReactElement {
  const { agent, model, observation, events, busy, onRefresh, onOpenDetails, focusRequest = 0, onFocusHandled } = context;
  const panel = density === "panel";
  const summary = useRef<HTMLDivElement>(null);
  const handledFocus = useRef(0);
  const detailsId = useId();
  const [expanded, setExpanded] = useState(false);
  const report = useMemo(() => buildContextReport(events), [events]);
  const [refreshState, setRefreshState] = useState<"loading" | "error" | null>(null);
  const refreshVersion = useRef(0);
  const refresh = useCallback(async () => {
    if (!onRefresh) return;
    const version = ++refreshVersion.current;
    setRefreshState("loading");
    try {
      await onRefresh();
      if (version === refreshVersion.current) setRefreshState(null);
    } catch {
      if (version === refreshVersion.current) setRefreshState("error");
    }
  }, [onRefresh]);
  useEffect(() => {
    if (active && !busy) void refresh();
    else setRefreshState(null);
    return () => { refreshVersion.current += 1; };
  }, [active, busy, refresh]);
  useEffect(() => {
    if (!active || focusRequest === 0 || handledFocus.current === focusRequest) return;
    handledFocus.current = focusRequest;
    if (panel) setExpanded(true);
    summary.current?.scrollIntoView?.({ block: "nearest" });
    summary.current?.focus({ preventScroll: true });
    onFocusHandled?.(focusRequest);
  }, [active, focusRequest, panel, onFocusHandled]);

  const capacity = observation?.contextWindow ?? contextWindowFor(agent, model);
  const used = contextPercent(observation?.contextTokens ?? null, agent, model, capacity);
  const remaining = used == null ? null : 100 - used;
  const tone = remaining == null ? "text-text-muted" : remaining < 20 ? "text-status-failed" : remaining < 50 ? "text-status-awaiting" : "text-primary-bright";
  const bar = remaining != null && remaining < 20 ? "bg-status-failed" : remaining != null && remaining < 50 ? "bg-status-awaiting" : "bg-primary";
  const calls = report.tools.reduce((sum, tool) => sum + tool.calls, 0);
  const toolGroups = Object.entries(TOOL_GROUP_LABELS).flatMap(([group, label]) => {
    const count = report.tools.filter(tool => tool.group === group).reduce((sum, tool) => sum + tool.calls, 0);
    return count > 0 ? [`${label} ${number(count)}회`] : [];
  }).join(" · ");
  const provider = agent === "codex" ? "Codex" : agent === "claude" ? "Claude" : agent || "기본 에이전트";
  const observedAt = observation?.observedAt == null ? "기록 시각 없음" : new Date(observation.observedAt * 1000).toLocaleString("ko-KR");

  return <>
    <ChannelCard label="컨텍스트 요약" className={panel ? "sticky top-0 z-20 shrink-0" : "shrink-0"}>
      <div ref={summary} data-context-summary role="region" aria-label="컨텍스트 요약" tabIndex={-1}
        className="rounded-xl px-3 py-3 text-xs text-text-secondary outline-none focus:ring-2 focus:ring-inset focus:ring-primary">
        <div className="flex items-center justify-between gap-2">
          <h2 className="font-medium text-text">컨텍스트</h2>
          <div className="flex items-center gap-2">
            <span className={`tabular-nums ${tone}`}>{remaining == null ? "확인 불가" : `약 ${remaining}% 남음`}</span>
            {onRefresh && <button type="button" onClick={() => void refresh()} disabled={busy || refreshState === "loading"}
              aria-label="컨텍스트 기록 새로고침" title="저장된 입력과 실행 기록 새로고침"
              className="rounded p-1 hover:bg-surface hover:text-text disabled:opacity-40 focus-visible:outline-primary"><Icon name="refresh" size={14} /></button>}
          </div>
        </div>
        <p className="mt-1 truncate text-text-muted" title={`${provider}${model ? ` · ${model}` : ""}`}>{provider}{model ? ` · ${model}` : ""}</p>
        {used != null && observation ? <>
          <p className="mt-2 tabular-nums"><strong className="font-medium text-text">{number(observation.contextTokens)}</strong> / {number(capacity)} 토큰</p>
          <div className="mt-1.5 h-1 overflow-hidden rounded-full bg-border" role="progressbar" aria-label="컨텍스트 사용률" aria-valuemin={0} aria-valuemax={100} aria-valuenow={used}>
            <div className={`h-full ${bar}`} style={{ width: `${used}%` }} />
          </div>
          <p className="mt-1.5 text-text-muted" title={`${sourceLabel(observation.source)} 관측 · ${observedAt}`}>
            {sourceLabel(observation.source)} 관측{observation.contextWindow == null && " · 용량은 모델 기준 추정"}
          </p>
        </> : <p className="mt-2 text-text-muted">사용량 미관측 · 확보된 기록은 상세에서 확인할 수 있습니다.</p>}
        <p className="mt-2 tabular-nums">입력 {report.injection ? `${number(report.injection.sentUserBytes)} B` : "크기 미상"} · 도구 {number(calls)}회</p>
        {toolGroups && <p className="mt-1 truncate text-text-muted" title={`수집된 도구 기록: ${toolGroups}`}>{toolGroups}</p>}
        {busy && <p className="mt-1 text-status-awaiting">작업 중 · 사용량은 이전 관측</p>}
        {refreshState === "loading" && <p role="status" className="mt-1 text-text-muted">입력과 실행 기록을 조회하고 있습니다.</p>}
        {refreshState === "error" && <p role="alert" className="mt-1 text-status-awaiting">기록을 불러오지 못했습니다. 새로고침으로 다시 조회할 수 있습니다.</p>}
        <button type="button" className="mt-2 flex w-full items-center justify-between border-t border-border pt-2 text-text-secondary hover:text-text focus-visible:outline-primary"
          onClick={panel ? () => setExpanded(value => !value) : onOpenDetails}
          aria-label={panel && expanded ? "컨텍스트 상세 접기" : "컨텍스트 상세 펼치기"}
          aria-expanded={panel ? expanded : undefined} aria-controls={panel ? detailsId : undefined}>
          <span>{panel && expanded ? "상세 접기" : panel ? "상세 펼치기" : "상세 보기 · 사이드바 펼치기"}</span>
          <Icon name={panel ? "chevronDown" : "chevronRight"} size={14} />
        </button>
      </div>
    </ChannelCard>
    {panel && expanded && <div id={detailsId} className="shrink-0">
      <ChannelCard label="컨텍스트 기록"><ContextDetails events={events} /></ChannelCard>
    </div>}
  </>;
}
