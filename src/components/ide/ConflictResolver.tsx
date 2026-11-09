import { useCallback, useEffect, useMemo, useState } from "react";
import type { ReactElement } from "react";
import {
  conflictAbort,
  conflictBegin,
  conflictFinish,
  conflictResolve,
  type ConflictFile,
  type ConflictResolution,
} from "../../lib/ipc";
import { summarizePatch } from "../../lib/diff";
import { SplitDiff } from "../DiffPresentation";
import { Icon } from "./icons";

/** 좌우 라벨 — 역방향 머지라 ours가 작업, theirs가 base다. 헷갈리기 쉬워 한 곳에 둔다. */
const OURS_LABEL = "내 작업 (ours)";
const THEIRS_LABEL = "기준 브랜치 (theirs)";

interface Props {
  taskId: number;
  /** 세션을 닫을 때 — 완료(true)면 호출부가 승인을 다시 시도한다. */
  onClose: (resolved: boolean) => void;
}

/**
 * 머지 충돌 해소.
 *
 * 충돌은 worktree 안에 갇혀 있고 repo는 깨끗한 상태다 — 중단해도 원본 체크아웃은 영향받지 않는다.
 * `ours`가 작업 쪽, `theirs`가 base 쪽이다(역방향 머지).
 */
export function ConflictResolver({ taskId, onClose }: Props): ReactElement {
  const [files, setFiles] = useState<ConflictFile[] | null>(null);
  const [unresolved, setUnresolved] = useState<string[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let live = true;
    conflictBegin(taskId)
      .then((list) => {
        if (!live) return;
        setFiles(list);
        setUnresolved(list.map((file) => file.path));
        setSelected(list[0]?.path ?? null);
      })
      .catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, [taskId]);

  const current = useMemo(
    () => files?.find((file) => file.path === selected) ?? null,
    [files, selected],
  );

  const resolve = useCallback(
    async (resolution: ConflictResolution) => {
      if (!selected) return;
      setBusy(true);
      setError(null);
      try {
        const remaining = await conflictResolve(taskId, selected, resolution);
        setUnresolved(remaining);
        setSelected(remaining[0] ?? null);
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(false);
      }
    },
    [selected, taskId],
  );

  const finish = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      await conflictFinish(taskId);
      onClose(true);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }, [onClose, taskId]);

  const abort = useCallback(async () => {
    setBusy(true);
    try {
      await conflictAbort(taskId);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      onClose(false);
    }
  }, [onClose, taskId]);

  if (error && !files) {
    return (
      <section className="m-3 rounded-lg border border-border bg-bg p-3">
        <h2 className="text-sm font-medium text-status-failed">충돌 해소를 열 수 없습니다</h2>
        <p className="mt-1 text-xs text-text-secondary">{error}</p>
        <button
          type="button"
          onClick={() => onClose(false)}
          className="mt-2 rounded border border-border px-2 py-1 text-xs text-text-secondary hover:bg-surface"
        >
          닫기
        </button>
      </section>
    );
  }

  if (!files) {
    return (
      <section className="m-3 rounded-lg border border-border bg-bg p-3 text-xs text-text-muted">
        충돌을 확인하는 중…
      </section>
    );
  }

  const done = unresolved.length === 0;

  return (
    <section className="m-3 rounded-lg border border-border bg-bg p-3">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <h2 className="text-sm font-medium text-text">머지 충돌 해소</h2>
        <span className="text-[11px] text-text-muted">
          충돌은 워크트리 안에만 있습니다 — 중단해도 원본 저장소는 그대로입니다.
        </span>
      </div>

      <div className="mt-2 flex flex-wrap gap-1.5">
        {files.map((file) => {
          const pending = unresolved.includes(file.path);
          return (
            <button
              key={file.path}
              type="button"
              onClick={() => setSelected(file.path)}
              className={`rounded border px-2 py-1 text-xs ${
                file.path === selected
                  ? "border-primary text-text"
                  : "border-border text-text-secondary hover:bg-surface"
              }`}
            >
              <span className={`mr-1 inline-flex align-[-2px] ${pending ? "text-status-failed" : "text-status-done"}`}>
                <Icon name={pending ? "alert" : "check"} size={12} />
              </span>
              {file.path}
            </button>
          );
        })}
      </div>

      {current && unresolved.includes(current.path) && (
        <div className="mt-3">
          <ConflictDiff file={current} />
          <div className="mt-2 flex flex-wrap gap-2">
            <ActionButton disabled={busy} onClick={() => resolve({ kind: "ours" })}>
              내 작업 채택
            </ActionButton>
            <ActionButton disabled={busy} onClick={() => resolve({ kind: "theirs" })}>
              기준 브랜치 채택
            </ActionButton>
            <ActionButton disabled={busy} onClick={() => resolve({ kind: "union" })}>
              양쪽 모두 남기기
            </ActionButton>
          </div>
        </div>
      )}

      {error && <p className="mt-2 text-xs text-status-failed">{error}</p>}

      <div className="mt-3 flex items-center gap-2 border-t border-border pt-2">
        <button
          type="button"
          disabled={!done || busy}
          onClick={finish}
          className="rounded border border-primary px-2 py-1 text-xs text-text disabled:opacity-40"
        >
          해소 완료
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={abort}
          className="rounded border border-border px-2 py-1 text-xs text-text-secondary hover:bg-surface disabled:opacity-40"
        >
          중단하고 되돌리기
        </button>
        <span className="text-[11px] text-text-muted">
          {done ? "모두 해소됐습니다 — 완료 후 다시 승인하세요." : `남은 파일 ${unresolved.length}개`}
        </span>
      </div>
    </section>
  );
}

/**
 * 좌우 비교.
 *
 * patch가 있으면 **다른 줄만** 짚는다. 전문 두 벌을 나란히 놓는 것은 비교처럼 보이지만
 * 실제 차이가 한 줄이고 파일이 수백 줄이면 그 차이를 찾는 일이 통째로 사람에게 떠넘겨진다.
 *
 * 한쪽에 파일이 없는 충돌(삭제/수정)은 patch가 없다 — 비교할 상대가 없으니 전문을 그대로 놓는다.
 */
function ConflictDiff({ file }: { file: ConflictFile }): ReactElement {
  if (!file.patch) {
    return (
      <div className="grid gap-2 md:grid-cols-2">
        <SidePane label={OURS_LABEL} content={file.ours} />
        <SidePane label={THEIRS_LABEL} content={file.theirs} />
      </div>
    );
  }
  const { additions, deletions } = summarizePatch(file.patch);
  return (
    <div className="min-w-0">
      <div className="flex items-baseline gap-2 text-[11px] text-text-muted">
        <span className="flex-1">{OURS_LABEL}</span>
        <span className="flex-1">{THEIRS_LABEL}</span>
        <span
          className="shrink-0 font-code"
          title={`${OURS_LABEL}에만 있는 줄 ${deletions}개 · ${THEIRS_LABEL}에만 있는 줄 ${additions}개`}
        >
          <span className="text-status-failed">−{deletions}</span>{" "}
          <span className="text-status-done">+{additions}</span>
        </span>
      </div>
      <div className="mt-1 max-h-72 overflow-auto rounded border border-border bg-raised">
        <SplitDiff patch={file.patch} />
      </div>
    </div>
  );
}

function SidePane({ label, content }: { label: string; content: string | null }): ReactElement {
  return (
    <div className="min-w-0">
      <div className="text-[11px] text-text-muted">{label}</div>
      {/* 줄바꿈을 허용한다 — 긴 한 줄이 가로 스크롤에 잘리면 비교라는 목적 자체가 무너진다. */}
      <pre className="mt-1 max-h-48 overflow-auto whitespace-pre-wrap break-all rounded border border-border bg-raised p-2 text-[11px] text-text-secondary">
        {content ?? "(이 쪽에는 파일이 없습니다)"}
      </pre>
    </div>
  );
}

function ActionButton({
  children,
  disabled,
  onClick,
}: {
  children: React.ReactNode;
  disabled: boolean;
  onClick: () => void;
}): ReactElement {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className="rounded border border-border px-2 py-1 text-xs text-text-secondary hover:bg-surface disabled:opacity-40"
    >
      {children}
    </button>
  );
}
