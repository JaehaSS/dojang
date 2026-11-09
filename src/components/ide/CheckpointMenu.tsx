import { useCallback, useEffect, useRef, useState } from "react";
import type { ReactElement } from "react";
import {
  checkpointCreate,
  checkpointList,
  convoRewind,
  type ConvoCheckpoint,
  type RewindSummary,
} from "../../lib/ipc";
import { Icon } from "./icons";

interface Props {
  taskId: number;
  /** 되감기가 성공하면 — 호출부가 대화 화면을 다시 읽는다. */
  onRewound: () => void;
}

/**
 * 체크포인트와 되감기 — 세션 툴바의 아이콘 + 팝오버(Menu·EffortPicker와 같은 패턴).
 *
 * 되감기는 **파일을 진짜로** 원복하고(worktree 커밋), 대화는 요약만 남긴 새 세션으로 재구성한다.
 * 벤더 CLI가 컨텍스트 절단을 제공하지 않아 대화 축은 재구성이 유일한 방법이다.
 *
 * 대화 위에 상시 펼쳐 두지 않는 이유: 체크포인트는 **수동 생성뿐이고 되감기는 파괴적·저빈도**다.
 * 목록이 비어 있는 시간이 대부분인 UI에 대화 영역의 상단을 상시 내줄 가치가 없다.
 */
export function CheckpointMenu({ taskId, onRewound }: Props): ReactElement {
  const [open, setOpen] = useState(false);
  const [points, setPoints] = useState<ConvoCheckpoint[]>([]);
  const [label, setLabel] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [summary, setSummary] = useState<RewindSummary | null>(null);
  const [confirming, setConfirming] = useState<number | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  const reload = useCallback(() => {
    checkpointList(taskId)
      .then(setPoints)
      .catch((e) => setError(String(e)));
  }, [taskId]);

  // 아이콘 옆 개수가 정직해야 하므로 목록은 팝오버를 열기 전에도 한 번 읽는다.
  useEffect(reload, [reload]);

  // 작업을 바꾸면 이전 작업의 확인 상태·결과를 끌고 가지 않는다.
  useEffect(() => {
    setConfirming(null);
    setError(null);
    setSummary(null);
  }, [taskId]);

  const close = useCallback(() => {
    setOpen(false);
    setConfirming(null);
  }, []);

  // 바깥 클릭·Esc로 닫힘 (Menu·EffortPicker와 같은 규약).
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [close, open]);

  const create = useCallback(async () => {
    if (!label.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await checkpointCreate(taskId, label.trim());
      setLabel("");
      reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [label, reload, taskId]);

  const rewind = useCallback(
    async (checkpointId: number) => {
      setBusy(true);
      setError(null);
      try {
        const result = await convoRewind(taskId, checkpointId);
        setSummary(result);
        setConfirming(null);
        onRewound();
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(false);
      }
    },
    [onRewound, taskId],
  );

  return (
    <div className="relative" ref={ref}>
      <button
        type="button"
        className={`flex items-center gap-0.5 rounded p-1 ${
          open ? "bg-primary/10 text-primary-bright" : "text-text-secondary hover:text-text"
        }`}
        onClick={() => {
          if (open) {
            close();
            return;
          }
          setOpen(true);
          reload();
        }}
        title="체크포인트 · 되감기"
        aria-label="체크포인트"
        aria-haspopup="dialog"
        aria-expanded={open}
      >
        <Icon name="clock" size={16} />
        {points.length > 0 && (
          <span className="text-[11px] leading-none tabular-nums">{points.length}</span>
        )}
      </button>

      {open && (
        <div className="absolute right-0 top-full z-20 mt-1 w-80 rounded-lg border border-border-strong bg-raised p-3 shadow-e2">
          <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
            <h2 className="text-sm font-medium text-text">체크포인트</h2>
            <span className="text-[11px] text-text-muted">
              되감으면 파일은 그 시점으로 돌아가고, 대화는 요약만 남습니다.
            </span>
          </div>

          <div className="mt-2 flex gap-2">
            <input
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && void create()}
              placeholder="이 시점의 이름 (예: 파서 교체 직전)"
              className="min-w-0 flex-1 rounded border border-border bg-surface px-2 py-1 text-xs text-text placeholder:text-text-muted"
            />
            <button
              type="button"
              disabled={busy || !label.trim()}
              onClick={create}
              className="shrink-0 rounded border border-primary px-2 py-1 text-xs text-text disabled:opacity-40"
            >
              지금 표시
            </button>
          </div>

          {points.length === 0 ? (
            <div className="mt-2 text-xs text-text-muted">아직 체크포인트가 없습니다.</div>
          ) : (
            <ul className="mt-2 grid max-h-64 gap-1.5 overflow-auto">
              {points.map((point) => (
                <li
                  key={point.id}
                  className="flex items-center justify-between gap-2 rounded border border-border px-2 py-1"
                >
                  <span className="min-w-0 truncate text-xs text-text-secondary">{point.label}</span>
                  {confirming === point.id ? (
                    <span className="flex shrink-0 items-center gap-1.5">
                      <span className="text-[11px] text-status-failed">이후 기록을 버립니다</span>
                      <button
                        type="button"
                        disabled={busy}
                        onClick={() => rewind(point.id)}
                        className="rounded border border-dangerborder px-1.5 py-0.5 text-[11px] text-status-failed disabled:opacity-40"
                      >
                        되감기
                      </button>
                      <button
                        type="button"
                        onClick={() => setConfirming(null)}
                        className="rounded border border-border px-1.5 py-0.5 text-[11px] text-text-muted"
                      >
                        취소
                      </button>
                    </span>
                  ) : (
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() => setConfirming(point.id)}
                      className="shrink-0 rounded border border-border px-1.5 py-0.5 text-[11px] text-text-secondary hover:bg-surface disabled:opacity-40"
                    >
                      여기로 되감기
                    </button>
                  )}
                </li>
              ))}
            </ul>
          )}

          {error && <p className="mt-2 text-xs text-status-failed">{error}</p>}

          {summary && (
            <div className="mt-2 rounded border border-border bg-surface p-2">
              <div className="text-[11px] text-text-muted">되감기 요약</div>
              {summary.kept && <p className="mt-1 text-xs text-text-secondary">{summary.kept}</p>}
              {summary.abandoned.length > 0 && (
                <>
                  <div className="mt-1.5 text-[11px] text-text-muted">
                    시도했으나 버린 접근 (다시 시도하지 말 것)
                  </div>
                  <ul className="mt-0.5 grid gap-0.5">
                    {summary.abandoned.map((item) => (
                      <li key={item} className="text-xs text-text-secondary">
                        · {item}
                      </li>
                    ))}
                  </ul>
                </>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
