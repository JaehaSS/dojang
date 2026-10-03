import { useEffect, useRef, useState, type ReactElement } from "react";
import { AGENT_PRESETS, labelFor } from "../../lib/agents";
import { contextShrinkWarning } from "../../lib/context-window";
import { hostCapabilities, LOCAL_ONLY_REASON } from "../../lib/host-capabilities";
import { LOCAL_HOST } from "../../lib/transport";
import { SpeedPicker } from "./SpeedPicker";
import { debateStart, taskAgentSet, taskModelSet, taskServiceTierSet, taskRef, type ServiceTier, type Task } from "../../lib/ipc";
import { Icon } from "./icons";
import { ModelPicker } from "./ModelPicker";

interface Props {
  task: Task;
  contextTokens?: number | null;
  observedModel?: string | null;
  onChanged: (model: string, updatedTask?: Task) => void;
  onServiceTierChanged?: (task: Task) => void;
  /** 우측 자리가 있다 — 시퀀스 사이에도 작업은 검토 대기로 돌아오므로 상태만으로는 못 가린다. */
  inDebate?: boolean;
  /** 토론이 시작됐다 — App이 우측 자리를 다시 읽어 분할 뷰로 간다. */
  onDebateStarted?: () => void;
}

const CONVERSATION_AGENTS = AGENT_PRESETS.filter(({ key }) => key === "claude" || key === "codex" || key === "agy");

function AgentDraftPicker({ agent, disabled, onChange, agents = CONVERSATION_AGENTS, heading = "다음 메시지의 에이전트", label, title, extra }: {
  agent: string;
  disabled: boolean;
  onChange: (agent: string) => void;
  /** 고를 수 있는 상대. 토론에서는 현재 에이전트를 뺀 둘이 온다. */
  agents?: typeof CONVERSATION_AGENTS;
  heading?: string;
  label?: string;
  title?: string;
  /** 목록 끝의 추가 선택지 — 토론의 "모두 (3자 토론)"이 여기 온다. */
  extra?: { label: string; hint: string; onSelect: () => void };
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (disabled) setOpen(false);
  }, [disabled]);
  useEffect(() => {
    if (!open) return;
    const close = (event: MouseEvent) => {
      if (!ref.current?.contains(event.target as Node)) setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", escape);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", escape);
    };
  }, [open]);
  return (
    <div className="relative" ref={ref}>
      <button
        className="flex items-center gap-1 rounded-md border border-border px-2 py-1 text-xs text-text-secondary hover:border-border-strong disabled:cursor-not-allowed disabled:opacity-60"
        disabled={disabled}
        onClick={() => setOpen((value) => !value)}
        title={title ?? (disabled ? "에이전트 전환은 대화 작업이 검토 대기일 때만 할 수 있습니다" : "다음 메시지를 처리할 에이전트")}
      >
        <Icon name="sparkle" size={13} />
        <span className="max-w-[150px] truncate">{label ?? labelFor(agent)}</span>
        <Icon name="chevronDown" size={12} />
      </button>
      {open && (
        <div className="absolute left-0 top-full z-20 mt-1 w-56 rounded-lg border border-border-strong bg-raised py-1 shadow-xl">
          <div className="px-3 py-1 text-xs text-text-muted">{heading}</div>
          {agents.map((preset) => (
            <button
              key={preset.key}
              className="flex w-full items-center gap-2 px-2 py-1.5 text-left text-sm text-text-secondary hover:bg-surface"
              onClick={() => {
                onChange(preset.key);
                setOpen(false);
              }}
            >
              <Icon name={agent === preset.key ? "check" : "sparkle"} size={13} />
              <span className="flex-1">{preset.label}</span>
              <span className="font-code text-xs text-text-muted">{preset.key}</span>
            </button>
          ))}
          {extra && (
            <button
              className="flex w-full items-center gap-2 border-t border-border px-2 py-1.5 text-left text-sm text-text-secondary hover:bg-surface"
              onClick={() => {
                extra.onSelect();
                setOpen(false);
              }}
            >
              <Icon name="sparkle" size={13} />
              <span className="flex-1">{extra.label}</span>
              <span className="font-code text-xs text-text-muted">{extra.hint}</span>
            </button>
          )}
        </div>
      )}
    </div>
  );
}

function CrossAgentDraft({ agent, model, busy, allowed, onModelChange, onSubmit }: { agent: string; model: string; busy: boolean; allowed: boolean; onModelChange: (model: string) => void; onSubmit: () => void }) {
  return (
    <>
      <ModelPicker agent={agent} model={model} onChange={onModelChange} placement="down" />
      <button
        type="button"
        className="rounded-md border border-primary/50 px-2 py-1 text-xs text-primary-bright hover:border-primary disabled:cursor-not-allowed disabled:border-border disabled:text-text-muted"
        disabled={busy || !allowed || !model.trim()}
        onClick={onSubmit}
      >
        전환
      </button>
    </>
  );
}

/** 세션 헤더의 모델 전환. 같은 에이전트의 모델은 즉시 저장하고, 다른 에이전트는 다음 메시지용
 * 대상 모델을 고른 뒤에만 전환한다. 작업·파일·표시 이력은 남지만 새 CLI의 네이티브 이력은 잇지 않는다. */
export function SessionModelSwitch({ task, contextTokens, observedModel, inDebate = false, onChanged, onServiceTierChanged, onDebateStarted }: Props): ReactElement | null {
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const agent = (task.agent ?? "").trim();
  const [draftAgent, setDraftAgent] = useState(agent);
  const [draftModel, setDraftModel] = useState("");
  useEffect(() => {
    setDraftAgent(agent);
    setDraftModel("");
  }, [task.id, agent]);
  if (!agent) return null;

  const isConversationAgent = CONVERSATION_AGENTS.some((preset) => preset.key === agent);
  // 모델 교체는 양쪽 호스트가 다 한다 — DB 한 칸을 고치고 다음 턴이 그 값을 읽는 것이 전부다.
  // 에이전트 전환·토론은 아니다. 전환은 벤더 세션을 버리고 핸드오프를 재조립하는 경로라
  // Runner에 대응 엔드포인트가 없고, 토론은 Runner의 대화 어댑터가 우측 자리가 있는 작업을
  // 아예 거절한다 — 원격에서 자리만 만들면 그 세션이 다음 턴부터 실행 불가가 된다.
  const canSwitchAgentHere = hostCapabilities(task.host).sessionAgentSwitch;
  // 토론 시작 게이트는 에이전트 전환과 같다 — 도는 중에는 잠금이 잡혀 벤더 세션을 새로 팔 수 없다.
  // 토론 중에는 두 면이 이미 벤더 세션을 하나씩 쥐고 있다 — 갈아타면 어느 면인지 말할 수 없다.
  const canSwitchAgent = canSwitchAgentHere && task.mode === "conversation" && task.state === "AwaitingReview" && isConversationAgent && !inDebate;
  const crossAgent = draftAgent !== agent;
  // 원격에서 막히는 이유는 "상태가 아직 아니다"가 아니라 "여기서는 안 되는 일"이다. 기본 문구를
  // 그대로 두면 검토 대기를 기다리면 열릴 것처럼 읽혀 사용자가 오지 않을 상태를 기다린다.
  const agentSwitchBlockedReason = canSwitchAgentHere
    ? null
    : `에이전트 전환·토론은 ${LOCAL_ONLY_REASON} — 모델은 원격에서도 바꿀 수 있습니다`;
  const changeModel = (raw: string) => {
    const next = raw.trim();
    if (busyRef.current || next === (task.model ?? "").trim()) return;
    const warning = contextShrinkWarning(agent, next, (task.model ?? "").trim(), contextTokens);
    if (warning != null && !window.confirm(warning)) return;
    busyRef.current = true;
    setBusy(true);
    taskModelSet(taskRef(task), next)
      .then(() => onChanged(next), (error) => window.alert(`모델을 바꾸지 못했습니다: ${error}`))
      .finally(() => {
        busyRef.current = false;
        setBusy(false);
      });
  };
  const changeServiceTier = (tier: ServiceTier) => {
    if (busyRef.current || tier === task.service_tier) return;
    busyRef.current = true;
    setBusy(true);
    taskServiceTierSet(taskRef(task), tier)
      .then((updated) => onServiceTierChanged?.(updated), (error) => window.alert(`실행 속도를 저장하지 못했습니다: ${error}`))
      .finally(() => { busyRef.current = false; setBusy(false); });
  };
  const changeAgent = (next: string) => {
    setDraftAgent(next);
    setDraftModel("");
  };
  const submitAgent = () => {
    if (!canSwitchAgent || !crossAgent || busyRef.current || !draftModel.trim()) return;
    busyRef.current = true;
    setBusy(true);
    taskAgentSet(taskRef(task), draftAgent, draftModel)
      .then((updated) => onChanged(draftModel, updated), (error) => window.alert(`에이전트를 바꾸지 못했습니다: ${error}`))
      .finally(() => {
        busyRef.current = false;
        setBusy(false);
      });
  };

  const opponents = CONVERSATION_AGENTS.filter((preset) => preset.key !== agent);
  /** 상대는 1~2명 — 자리 순서는 고른 순서다(첫째가 우측, 둘째가 셋째). */
  const startDebate = (picked: readonly string[]) => {
    if (!canSwitchAgent || busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    debateStart(taskRef(task), picked.map((opponent) => ({ agent: opponent })))
      .then(() => onDebateStarted?.(), (error) => window.alert(`토론을 시작하지 못했습니다: ${error}`))
      .finally(() => {
        busyRef.current = false;
        setBusy(false);
      });
  };

  return (
    <div className={`flex items-center gap-1 ${busy ? "pointer-events-none opacity-60" : ""}`}>
      {isConversationAgent && (
        <AgentDraftPicker
          agent={draftAgent}
          disabled={!canSwitchAgent || busy}
          title={agentSwitchBlockedReason ?? undefined}
          onChange={changeAgent}
        />
      )}
      {isConversationAgent && !inDebate && (
        <AgentDraftPicker
          agent={agent}
          disabled={!canSwitchAgent || busy}
          agents={opponents}
          heading="토론 상대"
          label="토론 시작"
          title={agentSwitchBlockedReason ?? (canSwitchAgent ? "고른 상대와 라운드를 주고받습니다 — 라운드 상한은 설정에서 바꿉니다" : "토론은 대화 작업이 검토 대기일 때만 시작할 수 있습니다")}
          onChange={(opponent) => startDebate([opponent])}
          extra={
            opponents.length >= 2
              ? { label: "모두 (3자 토론)", hint: opponents.slice(0, 2).map((preset) => preset.key).join("+"), onSelect: () => startDebate(opponents.slice(0, 2).map((preset) => preset.key)) }
              : undefined
          }
        />
      )}
      {crossAgent ? <CrossAgentDraft agent={draftAgent} model={draftModel} busy={busy} allowed={canSwitchAgent} onModelChange={setDraftModel} onSubmit={submitAgent} /> : <ModelPicker agent={agent} model={task.model ?? ""} observedModel={observedModel} onChange={changeModel} placement="down" disabled={inDebate} disabledTitle="토론 중에는 모델을 바꿀 수 없습니다" />}
      {!crossAgent && !inDebate && task.host === LOCAL_HOST && agent === "codex" && task.mode === "conversation" && !task.ensemble && (
        <SpeedPicker model={task.model ?? ""} value={task.service_tier ?? null} onChange={changeServiceTier} disabled={busy} />
      )}
      {isConversationAgent && !canSwitchAgent && (
        <span className="text-xs text-text-muted">{agentSwitchBlockedReason ?? (inDebate ? "토론이 끝나면 전환할 수 있습니다" : "대화 작업 검토 대기에서 전환 가능")}</span>
      )}
      {crossAgent && <span className="text-xs text-text-muted" title="같은 작업·파일과 표시 이력은 유지됩니다. 전체 대화 이력은 새 에이전트에 그대로 이어지지 않습니다.">작업 요약과 최근 대화를 전달해 이어갑니다</span>}
    </div>
  );
}
