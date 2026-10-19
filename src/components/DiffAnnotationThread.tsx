import { useState } from "react";
import type { RematchedAnnotation } from "../lib/ipc";
import { isAgentAnnotation, positionKey } from "../lib/annotations";
import { Markdown } from "./ide/Markdown";

const statusLabel: Record<string, string> = { draft: "초안", sent: "전송됨", resolved: "해결됨" };
const statusColor: Record<string, string> = {
  draft: "text-text-muted",
  sent: "text-status-running",
  resolved: "text-status-done",
};

interface ComposerTarget {
  key: string;
  hunkId: string;
  line: number;
  side: string;
  /** 기존 draft 수정이면 id, 새 주석이면 null. */
  id: string | null;
}

export interface AnnotationComposerState {
  /** 현재 열린 작성기의 위치 키 — 호출부는 자기 행의 positionKey와 비교한다. */
  activeKey: string | null;
  text: string;
  /** 직전 저장 실패 사유 — 작성기가 다시 열린 이유를 사용자에게 밝힌다. */
  error: string | null;
  setText: (text: string) => void;
  openNew: (hunkId: string, line: number, side: string) => void;
  openEdit: (hunkId: string, line: number, side: string, annotation: RematchedAnnotation) => void;
  save: () => Promise<void>;
}

/** 인라인 주석 작성 상태 머신 — 통합·분할 모드가 같은 한 벌을 쓴다.
 *  저장은 blur에서 일어나므로 먼저 작성기를 닫지만, 실패하면 되돌린다 — 닫힌 채로 두면
 *  사용자가 쓴 글이 아무 신호 없이 사라진다. */
export function useAnnotationComposer(
  onCreate: (input: { hunk_id: string; line: number; side: string; body_md: string }) => Promise<void>,
  onUpdateBody: (id: string, body_md: string) => Promise<void>,
): AnnotationComposerState {
  const [target, setTarget] = useState<ComposerTarget | null>(null);
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);

  const open = (next: ComposerTarget, body: string) => {
    setTarget(next);
    setText(body);
    setError(null);
  };

  return {
    activeKey: target?.key ?? null,
    text,
    error,
    setText,
    openNew: (hunkId, line, side) =>
      open({ key: positionKey(hunkId, line, side), hunkId, line, side, id: null }, ""),
    openEdit: (hunkId, line, side, annotation) =>
      open(
        { key: positionKey(hunkId, line, side), hunkId, line, side, id: annotation.id },
        annotation.body_md,
      ),
    save: async () => {
      if (!target) return;
      const body = text.trim();
      const current = target;
      setTarget(null);
      if (!body) return;
      try {
        if (current.id) await onUpdateBody(current.id, body);
        else
          await onCreate({
            hunk_id: current.hunkId,
            line: current.line,
            side: current.side,
            body_md: body,
          });
        setError(null);
      } catch (cause) {
        setTarget(current);
        setText(body);
        setError(String(cause));
      }
    },
  };
}

/** diff 재생성으로 원래 위치를 잃은 주석 — 삭제되지 않았음을 알린다. */
export function OrphanedAnnotationsNotice({ orphaned }: { orphaned: RematchedAnnotation[] }) {
  if (orphaned.length === 0) return null;
  return (
    <div className="m-2 px-2 py-1.5 rounded bg-dangerbg text-status-failed text-[11px] font-ui">
      고아 주석 {orphaned.length}건 — diff 재생성으로 원래 위치를 찾지 못했습니다(삭제되지 않음).
      {orphaned.map((a) => (
        <div key={a.id} className="mt-1 text-text-secondary">
          {a.path}:{a.line} — {a.body_md}
        </div>
      ))}
    </div>
  );
}

/** 한 위치(hunk, line, side)에 달린 주석 스레드 — 통합·분할 모드가 공유한다. */
export function AnnotationThread({
  thread,
  indent,
  onEdit,
}: {
  thread: RematchedAnnotation[];
  indent: string;
  onEdit: (annotation: RematchedAnnotation) => void;
}) {
  return (
    <>
      {thread.map((a) => (
        <div key={a.id} className={`${indent} my-1 p-2 rounded bg-raised border border-border`}>
          <div className="flex items-center gap-2 mb-0.5">
            {isAgentAnnotation(a) ? (
              // 에이전트 노트에는 상태 대신 작성자가 붙는다 — 사람 주석의 "전송됨"과 같은 색이면
              // 자기가 보낸 글로 읽힌다. 수정 버튼도 없다(저장소가 draft만 고친다).
              <span className="text-[10px] text-accent" data-annotation-author={a.author}>
                에이전트 · {a.author}
              </span>
            ) : (
              <span className={`text-[10px] uppercase ${statusColor[a.status]}`}>
                {statusLabel[a.status] ?? a.status}
              </span>
            )}
            {a.status === "draft" && !isAgentAnnotation(a) && (
              <button className="text-text-muted hover:text-text text-[10px]" onClick={() => onEdit(a)}>
                수정
              </button>
            )}
          </div>
          <Markdown text={a.body_md} />
        </div>
      ))}
    </>
  );
}

/** 인라인 주석 입력창 — 포커스를 벗어나면 draft로 자동 저장된다.
 *  `data-diff-composer`는 키보드 내비게이션 훅이 "입력 중"을 식별하는 표식이다. */
export function AnnotationComposer({
  indent,
  value,
  error,
  onChange,
  onSave,
}: {
  indent: string;
  value: string;
  error?: string | null;
  onChange: (text: string) => void;
  onSave: () => void;
}) {
  return (
    <div className={`${indent} my-1`}>
      {error && (
        <div className="mb-1 px-2 py-1 rounded bg-dangerbg text-status-failed text-[11px] font-ui">
          주석을 저장하지 못했습니다 — 내용은 그대로 두었습니다. {error}
        </div>
      )}
      <textarea
        autoFocus
        rows={2}
        data-diff-composer
        className="w-full bg-bg border border-border rounded px-2 py-1 text-xs font-ui text-text resize-none outline-none focus:border-primary"
        placeholder="마크다운 주석 — 포커스를 벗어나면 draft로 자동 저장됩니다"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onBlur={onSave}
      />
    </div>
  );
}
