import { useCallback, useEffect, useRef, useState } from "react";
import { taskPrExecute, taskPrPreview, type PrDeliveryAction, type PrDeliveryPreview, type PrDeliveryResult } from "../../lib/pr-delivery";

export interface PullRequestComposerProps {
  taskId: number;
  onChanged?: (result: PrDeliveryResult) => void;
}

/** A local, explicit delivery control.  Editing either field discards the prepared token. */
export function PullRequestComposer({ taskId, onChanged }: PullRequestComposerProps) {
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [preview, setPreview] = useState<PrDeliveryPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<PrDeliveryResult | null>(null);
  const generation = useRef(0);

  useEffect(() => {
    generation.current++;
    setTitle(""); setBody(""); setPreview(null); setBusy(false); setError(null); setResult(null);
  }, [taskId]);

  const invalidate = (nextTitle: string, nextBody: string) => {
    setTitle(nextTitle); setBody(nextBody); setPreview(null); setResult(null); setError(null);
  };
  const prepare = useCallback(async () => {
    if (busy) return;
    const request = ++generation.current;
    setBusy(true); setError(null); setResult(null);
    try {
      const next = await taskPrPreview(taskId, { title: title || undefined, body: body || undefined });
      if (generation.current === request) { setPreview(next); setTitle(next.title); setBody(next.body); }
    } catch (cause) {
      if (generation.current === request) setError(String(cause));
    } finally { if (generation.current === request) setBusy(false); }
  }, [body, busy, taskId, title]);
  const execute = useCallback(async (action: PrDeliveryAction) => {
    if (busy || !preview) return;
    const request = ++generation.current;
    const token = preview.token;
    setBusy(true); setError(null);
    try {
      const next = await taskPrExecute(taskId, token, action);
      if (generation.current === request) { setPreview(null); setResult(next); onChanged?.(next); }
    } catch (cause) {
      if (generation.current === request) { setPreview(null); setError(String(cause)); }
    } finally { if (generation.current === request) setBusy(false); }
  }, [busy, onChanged, preview, taskId]);

  return <section aria-label="PR 전달" className="mt-3 border-t border-border pt-3">
    <h3 className="font-medium">PR 전달</h3>
    <p className="mt-1 text-xs text-text-muted">미리보기는 커밋된 변경과 저장된 검증 결과만 포함합니다. 제목이나 본문을 고치면 미리보기를 다시 만들어야 합니다.</p>
    <label className="mt-3 block text-sm">제목
      <input aria-label="PR 제목" value={title} disabled={busy} onChange={(event) => invalidate(event.target.value, body)} className="mt-1 w-full rounded border border-border bg-bg px-2 py-1" />
    </label>
    <label className="mt-3 block text-sm">본문
      <textarea aria-label="PR 본문" value={body} disabled={busy} onChange={(event) => invalidate(title, event.target.value)} className="mt-1 min-h-28 w-full rounded border border-border bg-bg px-2 py-1" />
    </label>
    <div className="mt-3 flex flex-wrap gap-2">
      <button type="button" disabled={busy} onClick={() => void prepare()}>PR 미리보기 만들기</button>
      <button type="button" disabled={busy || !preview} onClick={() => void execute("push")}>브랜치 push</button>
      <button type="button" disabled={busy || !preview} onClick={() => void execute("create")}>PR 만들기</button>
      <button type="button" disabled={busy || !preview} onClick={() => void execute("update")}>열린 PR 업데이트</button>
    </div>
    {busy && <p role="status" className="mt-2 text-sm text-text-muted">PR 전달 작업을 처리하는 중입니다.</p>}
    {error && <p role="alert" className="mt-2 whitespace-pre-wrap break-words text-sm text-status-failed">PR 전달 실패: {error}</p>}
    {preview && <article className="mt-3 rounded border border-border bg-bg p-3 text-sm">
      <p>저장소: {preview.repository} · {preview.branch} → {preview.base}</p>
      <p className="select-text break-all text-text-muted">HEAD: <span className="font-code">{preview.head}</span></p>
      <p className="mt-2 font-medium">{preview.title}</p>
      <pre className="mt-2 whitespace-pre-wrap break-words font-sans text-text-muted">{preview.body}</pre>
    </article>}
    {result && <p role="status" className="mt-2 text-sm text-status-done">{result.message}</p>}
  </section>;
}
