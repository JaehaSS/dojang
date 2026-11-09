import { useCallback, useEffect, useState } from "react";
import type { AssistantApproval, AssistantSnapshot } from "../lib/transport";
import { api } from "./api";
import { ConvoTab } from "./ConvoTab";
import { Empty, Spinner } from "./primitives";
import { hrefFor } from "./routes";
import { linkProps } from "./router";

const VENDORS = [
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
  { id: "agy", label: "Antigravity" },
] as const;

const QUICK_PROMPTS = [
  { label: "진행 요약", prompt: "지금 돌아가고 있거나 검토 대기 중인 작업들을 요약해 줘." },
  { label: "질문 확인", prompt: "내 답변을 기다리는 질문 대기 작업이 있으면 내용을 알려 줘." },
  { label: "오늘 일정", prompt: "오늘 예정된 스케줄과 리마인더를 정리해 줘." },
] as const;

export function AssistantScreen({ revision }: { revision: number }) {
  const [snapshot, setSnapshot] = useState<AssistantSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [switching, setSwitching] = useState(false);
  const [actionBusyId, setActionBusyId] = useState<number | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const next = await api.assistantGet();
      setSnapshot(next);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load, revision]);

  useEffect(() => {
    const stop = api.subscribeEvents(
      0,
      () => {
        void load();
      },
      () => {},
    );
    return stop;
  }, [load]);

  const handleVendorChange = async (agent: string) => {
    if (!snapshot || switching || snapshot.task.agent === agent) return;
    setSwitching(true);
    setActionError(null);
    try {
      const updated = await api.assistantUpdateConfig({
        agent,
        model: null,
        reasoning_effort: null,
      });
      setSnapshot((prev) => (prev ? { ...prev, task: updated } : prev));
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setSwitching(false);
    }
  };

  const handleApprove = async (approval: AssistantApproval) => {
    setActionBusyId(approval.id);
    setActionError(null);
    try {
      await api.assistantApprove(approval.id);
      await load();
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setActionBusyId(null);
    }
  };

  const handleReject = async (approval: AssistantApproval) => {
    setActionBusyId(approval.id);
    setActionError(null);
    try {
      await api.assistantReject(approval.id);
      await load();
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    } finally {
      setActionBusyId(null);
    }
  };

  const handleQuickPrompt = async (prompt: string) => {
    if (!snapshot) return;
    setActionError(null);
    try {
      await api.assistantMessage(prompt);
      await load();
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleCancel = async () => {
    setActionError(null);
    try {
      await api.assistantCancel();
      await load();
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    }
  };

  if (error && !snapshot) {
    return <Empty>{error}</Empty>;
  }
  if (!snapshot) {
    return <Spinner label="비서 세션을 불러오는 중…" />;
  }

  const { task, pending_approvals: approvals, overview } = snapshot;
  const isBusy =
    task.state === "Running" || task.state === "Starting" || task.state === "Queued";
  const canSend = task.state === "AwaitingReview" || task.state === "Failed";
  const blockedReason = isBusy
    ? "비서가 응답을 작성하고 있습니다. 잠시만 기다려 주세요."
    : undefined;

  return (
    <div className="flex h-full flex-col">
      {/* 상단 관제 요약 및 벤더 전환 바 */}
      <div className="shrink-0 space-y-2 border-b border-border bg-surface px-4 py-2.5">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex flex-wrap items-center gap-1.5 text-xs">
            <a
              {...linkProps(hrefFor({ name: "home" }))}
              className="rounded-full border border-border bg-raised px-2.5 py-0.5 text-text-muted"
            >
              실행 <strong className="text-text">{overview.running_count}</strong>
            </a>
            <a
              {...linkProps(hrefFor({ name: "home" }))}
              className="rounded-full border border-border bg-raised px-2.5 py-0.5 text-text-muted"
            >
              검토 <strong className="text-text">{overview.awaiting_review_count}</strong>
            </a>
            {overview.question_count > 0 ? (
              <a
                {...linkProps(hrefFor({ name: "home" }))}
                className="rounded-full bg-status-awaiting px-2.5 py-0.5 font-medium text-black"
              >
                질문 {overview.question_count}
              </a>
            ) : null}
          </div>

          <div className="flex items-center gap-1" role="group" aria-label="비서 벤더 선택">
            {VENDORS.map((vendor) => {
              const active = (task.agent ?? "claude") === vendor.id;
              return (
                <button
                  key={vendor.id}
                  type="button"
                  disabled={switching || isBusy}
                  onClick={() => void handleVendorChange(vendor.id)}
                  className={`rounded-md px-2 py-1 text-xs font-medium ${
                    active
                      ? "bg-primary text-white"
                      : "border border-border bg-raised text-text-muted"
                  } disabled:opacity-40`}
                >
                  {vendor.label}
                </button>
              );
            })}
            {isBusy ? (
              <button
                type="button"
                onClick={() => void handleCancel()}
                className="ml-1 rounded-md border border-dangerborder bg-dangerbg px-2 py-1 text-xs text-text"
              >
                중단
              </button>
            ) : null}
          </div>
        </div>

        {/* 빠른 지시 칩 */}
        <div className="flex gap-1.5 overflow-x-auto pb-0.5">
          {QUICK_PROMPTS.map((item) => (
            <button
              key={item.label}
              type="button"
              disabled={!canSend}
              onClick={() => void handleQuickPrompt(item.prompt)}
              className="shrink-0 rounded-full border border-border bg-bg px-2.5 py-1 text-xs text-text-muted disabled:opacity-40"
            >
              {item.label}
            </button>
          ))}
        </div>

        {actionError ? (
          <p className="text-xs text-status-failed">{actionError}</p>
        ) : null}
      </div>

      {/* 인라인 승인 대기 카드 */}
      {approvals.length > 0 ? (
        <div className="shrink-0 space-y-2 border-b border-border bg-raised px-4 py-2.5">
          <div className="text-xs font-medium text-status-awaiting">
            승인 대기 요청 ({approvals.length}건)
          </div>
          {approvals.map((item) => {
            const busy = actionBusyId === item.id;
            return (
              <div
                key={item.id}
                className="flex items-center justify-between gap-2 rounded-md border border-border bg-surface px-3 py-2 text-xs"
              >
                <div className="min-w-0 flex-1">
                  <div className="truncate font-medium text-text">{item.summary}</div>
                  <div className="text-[11px] text-text-muted">
                    {item.action_kind}
                    {item.target_task_id ? ` · 작업 #${item.target_task_id}` : ""}
                  </div>
                </div>
                <div className="flex shrink-0 items-center gap-1.5">
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => void handleApprove(item)}
                    className="rounded bg-primary px-2.5 py-1 font-medium text-white disabled:opacity-40"
                  >
                    승인
                  </button>
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => void handleReject(item)}
                    className="rounded border border-border bg-raised px-2.5 py-1 text-text-muted disabled:opacity-40"
                  >
                    거절
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      ) : null}

      {/* 단일 비서 대화 스트림 */}
      <div className="min-h-0 flex-1">
        <ConvoTab id={task.id} canSend={canSend} blockedReason={blockedReason} />
      </div>
    </div>
  );
}
