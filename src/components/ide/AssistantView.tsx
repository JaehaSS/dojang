import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import {
  getTransportRevision,
  subscribeTransportChange,
  type AssistantApproval,
  type AssistantSnapshot,
  type TaskRef,
} from "../../lib/transport";
import { getRemoteRunnerTransport } from "../../lib/transport/remote-host";
import {
  appendItems,
  parseConvoEvent,
  toItems,
  type MobileConvoItem,
} from "../../mobile/convo";
import { Icon } from "./icons";

const VENDORS = [
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
  { id: "agy", label: "Antigravity" },
] as const;

const QUICK_PROMPTS = [
  { label: "진행 요약", prompt: "지금 돌아가고 있거나 검토 대기 중인 작업들을 요약해 줘." },
  { label: "질문 확인", prompt: "내 답변을 기다리는 질문 대기 작업이 있으면 내용을 알려 줘." },
  { label: "오늘 일정", prompt: "오늘 예정된 스케줄과 리마인더를 정리해 줘." },
  { label: "운영 상태", prompt: "현재 미니 PC 러너의 작업 현황과 주의할 실패 건을 점검해 줘." },
] as const;

interface Props {
  onOpenSettings: () => void;
  onOpenTask?: (ref: TaskRef) => void;
}

export function AssistantView({ onOpenSettings, onOpenTask }: Props) {
  useSyncExternalStore(subscribeTransportChange, getTransportRevision);
  const transport = getRemoteRunnerTransport();

  const [snapshot, setSnapshot] = useState<AssistantSnapshot | null>(null);
  const [items, setItems] = useState<MobileConvoItem[]>([]);
  const [draft, setDraft] = useState("");
  const [modelInput, setModelInput] = useState("");
  const [loading, setLoading] = useState(false);
  const [sending, setSending] = useState(false);
  const [switching, setSwitching] = useState(false);
  const [busyApprovalId, setBusyApprovalId] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  const loadSnapshot = useCallback(async () => {
    if (!transport) {
      setSnapshot(null);
      return;
    }
    setLoading(true);
    try {
      const next = await transport.assistantGet();
      setSnapshot(next);
      setModelInput(next.task.model ?? "");
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [transport]);

  useEffect(() => {
    void loadSnapshot();
  }, [loadSnapshot]);

  const taskId = snapshot?.task.id ?? null;

  useEffect(() => {
    if (!transport || taskId === null) {
      setItems([]);
      return;
    }
    let cancelled = false;
    let after = 0;
    setItems([]);

    const pullOutput = async () => {
      try {
        for (;;) {
          if (cancelled) return;
          const rows = await transport.taskOutput(taskId, after);
          if (cancelled || rows.length === 0) return;
          after = rows[rows.length - 1].sequence;
          const parsed = rows
            .map((r) => parseConvoEvent(r.data))
            .filter((e): e is NonNullable<typeof e> => e !== null);
          const nextItems = toItems(parsed);
          setItems((prev) =>
            prev.length === 0 ? nextItems : appendItems(prev, nextItems),
          );
          if (rows.length < 200) return;
        }
      } catch {
        // 일시적 폴링 오류는 다음 이벤트에서 복구한다.
      }
    };

    void pullOutput();
    const stop = transport.subscribeEvents(
      0,
      (ev) => {
        if (ev.task_id === taskId) {
          void pullOutput();
        }
        void loadSnapshot();
      },
      () => {},
    );
    return () => {
      cancelled = true;
      stop();
    };
  }, [transport, taskId, loadSnapshot]);

  useEffect(() => {
    const el = scrollRef.current;
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
  }, [items.length]);

  if (!transport) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center p-8 text-center">
        <div className="max-w-md space-y-4 rounded-xl border border-border bg-surface p-6">
          <div className="mx-auto flex h-10 w-10 items-center justify-center rounded-full bg-raised text-primary-bright">
            <Icon name="sparkle" size={20} />
          </div>
          <h2 className="text-base font-semibold text-text">상시 비서 (미니 PC 러너)</h2>
          <p className="text-xs leading-relaxed text-text-secondary">
            상시 비서는 미니 PC에서 항시 구동되는 <code className="font-code">praxis-runner</code>에
            연결되어 코딩 에이전트 관제(Claude · Codex · Antigravity)와 일상 일정·지식베이스
            요청을 한 대화 스레드에서 처리합니다.
          </p>
          <button
            type="button"
            onClick={onOpenSettings}
            className="inline-flex items-center gap-1.5 rounded-md bg-primary px-4 py-2 text-xs font-medium text-white"
          >
            <Icon name="settings" size={14} /> 상시 러너 연결 설정 열기
          </button>
        </div>
      </div>
    );
  }

  const task = snapshot?.task ?? null;
  const overview = snapshot?.overview;
  const approvals = snapshot?.pending_approvals ?? [];
  const isBusy =
    task?.state === "Running" || task?.state === "Starting" || task?.state === "Queued";
  const canSend = Boolean(
    task && (task.state === "AwaitingReview" || task.state === "Failed") && !sending,
  );

  const handleSend = async (textOverride?: string) => {
    const message = (textOverride ?? draft).trim();
    if (!message || !canSend) return;
    setSending(true);
    setError(null);
    if (textOverride === undefined) setDraft("");
    try {
      await transport.assistantMessage(message);
      await loadSnapshot();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSending(false);
    }
  };

  const handleVendorSwitch = async (agent: string, nextModel?: string) => {
    if (!task || switching || isBusy) return;
    setSwitching(true);
    setError(null);
    try {
      const updated = await transport.assistantUpdateConfig({
        agent,
        model: nextModel !== undefined ? nextModel : modelInput.trim() || null,
        reasoning_effort: task.reasoning_effort ?? null,
      });
      setSnapshot((prev) => (prev ? { ...prev, task: updated } : prev));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSwitching(false);
    }
  };

  const handleApprove = async (item: AssistantApproval) => {
    setBusyApprovalId(item.id);
    setError(null);
    try {
      await transport.assistantApprove(item.id);
      await loadSnapshot();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyApprovalId(null);
    }
  };

  const handleReject = async (item: AssistantApproval) => {
    setBusyApprovalId(item.id);
    setError(null);
    try {
      await transport.assistantReject(item.id);
      await loadSnapshot();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyApprovalId(null);
    }
  };

  const handleCancel = async () => {
    setError(null);
    try {
      await transport.assistantCancel();
      await loadSnapshot();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <div className="flex flex-1 flex-col min-h-0 bg-bg">
      {/* 상단 헤더: 호스트 상태 · 관제 지표 · 벤더/모델 스위처 */}
      <header className="shrink-0 border-b border-border bg-surface px-5 py-3 space-y-2.5">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            <span className="inline-flex h-2 w-2 rounded-full bg-status-ready" />
            <h1 className="text-sm font-semibold text-text">
              상시 비서 ({transport.hostId})
            </h1>
            {overview ? (
              <div className="flex items-center gap-1.5 text-xs text-text-secondary ml-2">
                <span className="rounded-full border border-border bg-raised px-2.5 py-0.5">
                  실행 <strong className="text-text">{overview.running_count}</strong>
                </span>
                <span className="rounded-full border border-border bg-raised px-2.5 py-0.5">
                  대기 <strong className="text-text">{overview.queued_count}</strong>
                </span>
                <span className="rounded-full border border-border bg-raised px-2.5 py-0.5">
                  검토 <strong className="text-text">{overview.awaiting_review_count}</strong>
                </span>
                {overview.question_count > 0 ? (
                  <span className="rounded-full bg-status-awaiting px-2.5 py-0.5 font-medium text-black">
                    질문 {overview.question_count}
                  </span>
                ) : null}
              </div>
            ) : null}
          </div>

          <div className="flex items-center gap-2">
            <div className="flex items-center gap-1" role="group" aria-label="비서 벤더 선택">
              {VENDORS.map((v) => {
                const active = (task?.agent ?? "claude") === v.id;
                return (
                  <button
                    key={v.id}
                    type="button"
                    disabled={switching || isBusy}
                    onClick={() => void handleVendorSwitch(v.id)}
                    className={`rounded-md px-2.5 py-1 text-xs font-medium ${
                      active
                        ? "bg-primary text-white"
                        : "border border-border bg-raised text-text-secondary hover:text-text"
                    } disabled:opacity-40`}
                  >
                    {v.label}
                  </button>
                );
              })}
            </div>
            <input
              type="text"
              value={modelInput}
              disabled={switching || isBusy}
              onChange={(e) => setModelInput(e.target.value)}
              onBlur={() => {
                if (task && (task.model ?? "") !== modelInput.trim()) {
                  void handleVendorSwitch(task.agent ?? "claude", modelInput.trim());
                }
              }}
              placeholder="기본 모델"
              aria-label="비서 모델 오버라이드"
              className="w-36 rounded-md border border-border bg-bg px-2 py-1 font-code text-xs text-text outline-none focus:border-primary disabled:opacity-40"
            />
            {isBusy ? (
              <button
                type="button"
                onClick={() => void handleCancel()}
                className="rounded-md border border-dangerborder bg-dangerbg px-2.5 py-1 text-xs text-text"
              >
                턴 중단
              </button>
            ) : null}
            <button
              type="button"
              onClick={() => void loadSnapshot()}
              title="새로고침"
              className="rounded-md border border-border bg-raised p-1.5 text-text-muted hover:text-text"
            >
              <Icon name="refresh" size={14} />
            </button>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-1.5">
          {QUICK_PROMPTS.map((chip) => (
            <button
              key={chip.label}
              type="button"
              disabled={!canSend}
              onClick={() => void handleSend(chip.prompt)}
              className="rounded-full border border-border bg-bg px-2.5 py-1 text-xs text-text-secondary hover:border-border-strong hover:text-text disabled:opacity-40"
            >
              {chip.label}
            </button>
          ))}
        </div>

        {error ? <p className="text-xs text-status-failed">{error}</p> : null}
      </header>

      {/* 인라인 승인 요청 배너 */}
      {approvals.length > 0 ? (
        <div className="shrink-0 border-b border-border bg-raised px-5 py-3 space-y-2">
          <div className="text-xs font-semibold text-status-awaiting">
            인라인 승인 대기 ({approvals.length}건)
          </div>
          <div className="space-y-1.5">
            {approvals.map((item) => {
              const busy = busyApprovalId === item.id;
              return (
                <div
                  key={item.id}
                  className="flex items-center justify-between gap-3 rounded-md border border-border bg-surface px-3.5 py-2 text-xs"
                >
                  <div className="min-w-0 flex-1">
                    <div className="font-medium text-text">{item.summary}</div>
                    <div className="text-[11px] text-text-muted">
                      동작: <code className="font-code">{item.action_kind}</code>
                      {item.target_task_id ? ` · 대상 작업 #${item.target_task_id}` : ""}
                    </div>
                  </div>
                  <div className="flex items-center gap-1.5 shrink-0">
                    {item.target_task_id && onOpenTask ? (
                      <button
                        type="button"
                        onClick={() =>
                          onOpenTask({ host: transport.hostId, id: item.target_task_id! })
                        }
                        className="rounded border border-border bg-raised px-2.5 py-1 text-text-secondary hover:text-text"
                      >
                        작업 열기
                      </button>
                    ) : null}
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() => void handleApprove(item)}
                      className="rounded bg-primary px-3 py-1 font-medium text-white disabled:opacity-40"
                    >
                      승인
                    </button>
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() => void handleReject(item)}
                      className="rounded border border-border bg-raised px-2.5 py-1 text-text-muted hover:text-text disabled:opacity-40"
                    >
                      거절
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      ) : null}

      {/* 대화 스트림 */}
      <div ref={scrollRef} className="flex-1 overflow-y-auto px-5 py-4 space-y-3">
        {loading && items.length === 0 ? (
          <p className="text-xs text-text-muted">비서 세션을 불러오는 중…</p>
        ) : items.length === 0 ? (
          <div className="rounded-lg border border-border bg-surface p-4 text-xs text-text-secondary">
            상시 비서 세션이 준비되었습니다. 원격 코딩 작업 위임, 진행 중인 에이전트 상태 점검,
            스케줄·리마인더 등록을 지시하세요.
          </div>
        ) : (
          items.map((item, idx) => {
            if (item.role === "user") {
              return (
                <div
                  key={idx}
                  className="ml-16 rounded-lg border border-border bg-raised px-3.5 py-2.5 text-sm text-text"
                >
                  {item.text}
                </div>
              );
            }
            if (item.role === "text") {
              return (
                <div
                  key={idx}
                  className="mr-12 whitespace-pre-wrap rounded-lg border border-border bg-surface px-3.5 py-2.5 text-sm text-text"
                >
                  {item.text}
                </div>
              );
            }
            if (item.role === "tool") {
              return (
                <div key={idx} className="font-code text-xs text-text-muted">
                  → {item.name}
                  {item.summary ? ` ${item.summary}` : ""}
                </div>
              );
            }
            if (item.role === "result") {
              return (
                <div
                  key={idx}
                  className={`font-code text-xs ${
                    item.error ? "text-status-failed" : "text-text-muted"
                  }`}
                >
                  {item.error ? "×" : "✓"} {item.summary}
                </div>
              );
            }
            if (item.role === "error") {
              return (
                <div
                  key={idx}
                  className="rounded-md border border-dangerborder bg-dangerbg px-3 py-2 text-xs text-text"
                >
                  {item.text}
                </div>
              );
            }
            return null;
          })
        )}
      </div>

      {/* 하단 입력창 */}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void handleSend();
        }}
        className="shrink-0 border-t border-border bg-surface px-5 py-3 flex items-end gap-2"
      >
        <textarea
          rows={2}
          value={draft}
          disabled={!canSend}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              void handleSend();
            }
          }}
          placeholder={
            isBusy
              ? "비서가 응답을 작성 중입니다…"
              : "비서에게 코딩 작업 위임·관제 또는 일정 관리를 지시하세요 (Enter 전송, Shift+Enter 줄바꿈)"
          }
          className="flex-1 resize-none rounded-md border border-border bg-bg px-3 py-2 text-sm text-text outline-none focus:border-primary disabled:opacity-50"
        />
        <button
          type="submit"
          disabled={!canSend || !draft.trim()}
          className="rounded-md bg-primary px-4 py-2 text-xs font-medium text-white disabled:opacity-40"
        >
          전송
        </button>
      </form>
    </div>
  );
}
