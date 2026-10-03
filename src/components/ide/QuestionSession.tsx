import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { answerQuestion, answersComplete, interactionSnapshot, questionReceipt, receiptLabel, retryQuestionCleanup, saveQuestionDraft, type Interaction, type InteractionSnapshot, type QuestionAnswer } from "../../lib/conversation-interaction";
import { safeExternalHttpUrl } from "../../lib/tool-output-media";

interface Props {
  taskId: number | null;
  linkedIds: string[];
  onBusyChange?: (busy: boolean) => void;
  children: (render: (id: string) => ReactNode, status: ReactNode) => ReactNode;
}
/** Mounted with the task key: asynchronous replies and drafts always retain their original owner. */
export function QuestionSession({ taskId, linkedIds, onBusyChange, children }: Props) {
  const [snapshot, setSnapshot] = useState<InteractionSnapshot | null>(null);
  const [error, setError] = useState("");
  const refreshRef = useRef<() => Promise<void>>(async () => {});
  useEffect(() => {
    if (taskId == null) return;
    let alive = true;
    let running=false;
    let refreshAgain=false;
    const refresh = async () => {
      if(running){refreshAgain=true;return;}
      running=true;
      try {
        const next = await interactionSnapshot(taskId);
        if (!alive) return;
        setSnapshot(next); setError("");
        if (next.enabled) onBusyChange?.(next.phase !== "idle");
      } catch (e) { if (alive) setError(String(e)); }
      finally {running=false;if(alive&&refreshAgain){refreshAgain=false;void refresh();}}
    };
    refreshRef.current = refresh;
    const un = listen<{ taskId: number }>("convo-interaction://changed", ({ payload }) => { if (payload.taskId === taskId) void refresh(); });
    void un.then(() => { if (alive) void refresh(); });
    // Snapshot recovery also covers a dropped change notification and expiry while unfocused.
    const timer = window.setInterval(() => { void refresh(); }, 2000);
    return () => { alive = false; window.clearInterval(timer); void un.then((f) => f()); };
  }, [taskId, onBusyChange]);
  const refresh = useCallback(() => refreshRef.current(), []);
  const render = (id: string) => {
    const item = snapshot?.items.find((v) => v.id === id);
    return item && taskId != null ? <QuestionCard key={item.id} taskId={taskId} item={item} phase={snapshot?.phase ?? "unknown"} refresh={refresh} /> : null;
  };
  const status = <>
    {error && <p role="alert" className="text-xs text-text-secondary">질문 상태 확인 실패: {error}</p>}
    {snapshot?.enabled && snapshot.phase === "cleanup_failed" && <div className="rounded-md border border-border p-3 text-sm" role="alert">
      실행 정리를 확인하지 못했습니다. 정리 전에는 새 입력·승인·폐기를 할 수 없습니다.
      <button className="ml-2 underline" onClick={() => { if (taskId != null) void retryQuestionCleanup(taskId).then(refresh).catch((e) => setError(String(e))); }}>정리 재시도</button>
    </div>}
    {snapshot?.enabled && ["cancelling", "finalizing"].includes(snapshot.phase) && <p className="text-xs text-text-muted">{snapshot.phase === "cancelling" ? "실행을 중단하고 있습니다…" : "응답을 마쳤습니다. 실행을 정리하고 있습니다…"}</p>}
    {/* A snapshot can arrive before its transcript anchor; never lose an actionable question. */}
    {snapshot?.items.filter((item) => !linkedIds.includes(item.id)).map((item) => render(item.id))}
  </>;
  return <>{children(render, status)}</>;
}

export function QuestionCard({ taskId, item, phase, refresh }: { taskId: number; item: Interaction; phase: string; refresh: () => Promise<void> }) {
  const [answers, setAnswers] = useState(item.draft);
  const [error, setError] = useState("");
  const [sending, setSending] = useState(false);
  const [uncertain, setUncertain] = useState(false);
  const [observedReceipt, setObservedReceipt] = useState<string | null>(null);
  const revision = useRef(item.draft_revision);
  const latest = useRef(item.draft);
  const saved = useRef(JSON.stringify(item.draft));
  const queue = useRef<Promise<void>>(Promise.resolve());
  const request = useRef<string | null>(null);
  const submitting = useRef(false);
  const mounted = useRef(true);
  useEffect(() => {mounted.current=true;return () => { mounted.current = false; };}, []);
  const pending = item.state === "pending" && phase === "running" && item.expires_at * 1000 > Date.now();
  const receipt = item.receipt?.state ?? observedReceipt;
  const locked = !pending || !!receipt || sending || uncertain;
  const nativeRequest = item.request;
  const externalUrl = safeExternalHttpUrl(nativeRequest?.url);
  const submitLabel = nativeRequest?.kind === "approval"
    ? "승인 여부 보내기"
    : nativeRequest?.kind === "form"
      ? "입력 보내기"
      : nativeRequest?.kind === "url"
        ? "선택 보내기"
        : "답변 보내기";
  // Restore externally accepted snapshots, but never overwrite a locally edited draft mid-save.
  useEffect(() => {
    if (item.receipt) { latest.current = item.draft; setAnswers(item.draft); }
  }, [item.receipt?.request_id]);
  const change = (answer: QuestionAnswer) => {
    if (locked) return;
    const next = [...latest.current.filter((a) => a.question_id !== answer.question_id), answer];
    latest.current = next; setAnswers(next); setError("");
    queue.current = queue.current.then(async () => {
      const next = latest.current;
      const encoded = JSON.stringify(next);
      if (encoded === saved.current) return;
      try { revision.current = await saveQuestionDraft(taskId, item, next, revision.current); saved.current = encoded; }
      catch (e) { if (mounted.current) setError(`초안 저장 실패: ${String(e)}. 입력한 내용은 이 화면에 남아 있습니다.`); }
    });
  };
  const submit = async () => {
    if (locked || submitting.current || !answersComplete(item, latest.current)) return;
    submitting.current = true; setSending(true); setError("");
    await queue.current;
    // The answer transaction carries its own complete snapshot, even after a draft save failure.
    request.current ??= crypto.randomUUID();
    try {
      const action = latest.current.find((a) => a.question_id === "__dojang_action");
      // Declining a form sends no field contents, even if the user already started a draft.
      const payload = item.request?.kind === "form" && action && (action.option_id === "decline" || action.option_id === "cancel") ? [action] : latest.current;
      const accepted = await answerQuestion(taskId, item, request.current, payload);
      if (mounted.current) setObservedReceipt(accepted.state);
      await refresh();
    } catch (e) {
      if (mounted.current) {
        const message = e instanceof Error ? e.message : String(e);
        if (message.startsWith("INPUT_VALIDATION: ")) {
          // The host guarantees this error is returned before claiming any receipt.
          request.current = null; setUncertain(false); setError(message.slice("INPUT_VALIDATION: ".length));
        } else { setUncertain(true); setError(`답변 접수 확인 실패: ${message}`); }
      }
    } finally { submitting.current = false; if (mounted.current) setSending(false); }
  };
  const checkReceipt = async () => {
    if (!request.current) return;
    try {
      const received = await questionReceipt(taskId, request.current);
      if (!mounted.current) return;
      if (received.state === "not_found") {
        // Explicit retry retains the same id and body; checking never dispatches a new answer.
        setError("접수 기록이 없습니다. 상태를 다시 확인하거나 실행을 중단하세요.");
      } else { setObservedReceipt(received.state); setUncertain(false); setError(""); }
      await refresh();
    } catch (e) { if (mounted.current) setError(String(e)); }
  };
  return <section className="rounded-lg border border-border-strong bg-raised p-4 space-y-3 max-w-[92%]" aria-label="에이전트 질문">
    <div className="text-sm font-medium">{nativeRequest?.kind === "approval" ? pending && !receipt ? "승인 필요" : "승인 요청" : pending && !receipt ? "답변 필요" : "에이전트 질문"}</div>
    {nativeRequest && <div className="space-y-1 rounded border border-border bg-bg p-3">
      <div className="text-sm font-medium whitespace-pre-wrap">{nativeRequest.title}</div>
      {nativeRequest.details && <p className="text-sm text-text-secondary whitespace-pre-wrap">{nativeRequest.details}</p>}
      {nativeRequest.kind === "url" && externalUrl && <><p className="text-xs break-all">{externalUrl}</p><button type="button" className="text-sm text-primary-bright underline" onClick={() => { void openUrl(externalUrl).catch((e) => setError(`링크를 열지 못했습니다: ${String(e)}`)); }}>외부 링크 열기</button></>}
      {nativeRequest.kind === "url" && nativeRequest.url && !externalUrl && <p role="alert" className="text-xs text-text-secondary">열 수 없는 주소입니다.</p>}
    </div>}
    {item.questions.questions.map((q) => {
      const answer = answers.find((a) => a.question_id === q.id);
      return <fieldset key={q.id} disabled={locked} className="space-y-2">
        <legend className="text-sm whitespace-pre-wrap">{q.question}</legend>
        {q.options.map((o) => <label key={o.id} className="flex gap-2 text-sm cursor-pointer">
          <input type="radio" name={`${item.id}-${q.id}`} checked={answer?.option_id === o.id} onChange={() => change({ question_id: q.id, option_id: o.id, text: null })} />
          <span>{o.label}{o.description && <span className="block text-xs text-text-muted">{o.description}</span>}</span>
        </label>)}
        {q.allow_free_text && <label className="block text-xs text-text-muted">직접 답변
          <textarea aria-label={`${q.question} 직접 답변`} className="mt-1 block w-full bg-surface border border-border rounded p-2 text-sm text-text" maxLength={4000} rows={2} value={answer?.text ?? ""} onChange={(e) => change({ question_id: q.id, option_id: null, text: e.target.value })} onKeyDown={(e) => {
            if (e.nativeEvent.isComposing || e.keyCode === 229) return;
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); void submit(); }
          }} />
        </label>}
      </fieldset>;
    })}
    {receipt ? <p role="status" className="text-xs text-text-secondary">{receiptLabel(receipt)}</p> : pending && !uncertain ? <button disabled={locked || !answersComplete(item, answers)} onClick={() => void submit()} className="rounded border border-border px-3 py-1.5 text-sm disabled:opacity-40">{sending ? "접수 중…" : submitLabel}</button> : null}
    {uncertain && !item.receipt && <button className="underline text-sm" onClick={() => void checkReceipt()}>접수 상태 확인</button>}
    {!pending && item.reason !== "answered" && <p className="text-xs text-text-muted">이 질문은 종료되었습니다 ({item.reason === "resolved" ? "Codex가 더 이상 답변을 기다리지 않음" : item.reason === "cancelled" ? "사용자 중단" : item.reason === "connection_lost" ? "연결 종료" : "실행 종료 또는 만료"}).</p>}
    {error && <p role="alert" className="text-xs text-text-secondary">{error}</p>}
    {pending && !receipt && nativeRequest?.kind === "question" && !nativeRequest.blocking && <p className="text-xs text-text-muted">답변을 기다리는 동안 작업은 계속됩니다.</p>}
    {pending && !receipt && <p className="text-xs text-text-muted">다른 작업은 계속 진행될 수 있습니다. 비밀번호·API 키 등 비밀값을 입력하지 마세요.</p>}
  </section>;
}
