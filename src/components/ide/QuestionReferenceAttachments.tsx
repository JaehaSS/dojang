import { useState } from "react";
import type { QuestionReference } from "../../lib/side-question";
import { Icon } from "./icons";
import { IconButton } from "../ui";

export interface QuestionReferenceAttachmentsProps {
  references: readonly QuestionReference[];
  onChange: (references: QuestionReference[]) => void;
}

/**
 * Main-composer attachment editor. It edits copies only: an answer selected
 * from the side thread must never mutate as that thread continues streaming.
 */
export function QuestionReferenceAttachments({
  references,
  onChange,
}: QuestionReferenceAttachmentsProps) {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draft, setDraft] = useState("");

  if (references.length === 0) return null;

  const remove = (id: string) => onChange(references.filter((reference) => reference.id !== id));
  const beginEdit = (reference: QuestionReference) => {
    setEditingId(reference.id);
    setDraft(reference.text);
  };
  const saveEdit = () => {
    if (editingId == null || draft.trim() === "") return;
    onChange(
      references.map((reference) =>
        reference.id === editingId ? { ...reference, text: draft.trim() } : reference,
      ),
    );
    setEditingId(null);
    setDraft("");
  };

  return (
    <section className="mb-2 flex flex-col gap-1.5" aria-label="메인 참고자료">
      <p className="text-xs text-text-secondary">참고자료 · 메인에 자동 전송되지 않습니다</p>
      {references.map((reference) => {
        const editing = editingId === reference.id;
        return (
          <article key={reference.id} className="rounded border border-border bg-surface px-2 py-1.5 text-xs">
            <div className="flex items-start gap-1.5">
              <Icon name="chat" size={14} />
              <div className="min-w-0 flex-1">
                <p className="truncate text-text-secondary" title={reference.question}>
                  {reference.question}
                </p>
                {reference.incomplete && <p className="mt-0.5 text-status-awaiting">미완료 답변</p>}
              </div>
              <button
                type="button"
                className="rounded px-1 text-text-secondary hover:bg-raised hover:text-text"
                onClick={() => beginEdit(reference)}
                aria-label="참고자료 편집"
              >
                편집
              </button>
              <IconButton size="sm" tone="danger" label="참고자료 제거" onClick={() => remove(reference.id)}>
                <Icon name="x" size={14} />
              </IconButton>
            </div>
            {editing ? (
              <div className="mt-1.5">
                <label className="sr-only" htmlFor={`question-reference-${reference.id}`}>참고자료 내용</label>
                <textarea
                  id={`question-reference-${reference.id}`}
                  className="min-h-20 w-full resize-y rounded border border-border bg-bg px-2 py-1 text-xs text-text outline-none focus:border-primary"
                  value={draft}
                  onChange={(event) => setDraft(event.target.value)}
                />
                <div className="mt-1 flex justify-end gap-1">
                  <button type="button" className="rounded px-1.5 py-0.5 text-text-secondary hover:bg-raised" onClick={() => setEditingId(null)}>취소</button>
                  <button type="button" className="rounded border border-primary px-1.5 py-0.5 text-primary-bright disabled:opacity-50" onClick={saveEdit} disabled={draft.trim() === ""}>저장</button>
                </div>
              </div>
            ) : (
              <details className="mt-1">
                <summary className="cursor-pointer text-text-muted">내용과 출처 보기</summary>
                <pre className="mt-1 max-h-32 overflow-auto whitespace-pre-wrap rounded bg-bg p-1.5 font-ui text-text-secondary">{reference.text}</pre>
                {reference.contexts.length > 0 && <p className="mt-1 text-text-muted">출처: {reference.contexts.map((context) => context.label).join(" · ")}</p>}
              </details>
            )}
          </article>
        );
      })}
    </section>
  );
}
