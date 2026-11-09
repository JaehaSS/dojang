import { useCallback, useEffect, useState } from "react";
import {
  multiReview,
  reviewDelete,
  reviewGet,
  reviewHistoryList,
  type MultiReviewResult,
  type ReviewDetail,
  type ReviewMeta,
} from "../../lib/ipc";
import { LOCAL_HOST, type HostId } from "../../lib/transport";
import { Icon } from "./icons";
import { ReviewPanel } from "./ReviewPanel";
import { IconButton } from "../ui";

/** 백엔드 화이트리스트와 같은 순서 — 기본 셋에 gemini를 선택지로 더 둔다. */
const VENDORS = ["claude", "codex", "agy", "gemini"];
const DEFAULT_VENDORS = ["claude", "codex", "agy"];

/** epoch초 → "2026-07-06 14:38". 이력 행은 날짜까지 보여야 "언제 물어봤나"가 산다. */
const stamp = (sec: number): string => {
  const d = new Date(sec * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
};

interface Props {
  repo: string;
  /** 리뷰 대상 작업 — `source_kind="diff"`의 `source_ref`가 이 id의 문자열이다. */
  taskId: number;
  host: HostId;
}

/**
 * 선택된 작업의 diff를 여러 벤더에게 동시에 묻는 세션 안 액션(재탐색 2026-09-13, 선택지 B).
 *
 * 리뷰 채널이 하던 일이 여기로 왔다 — 리뷰는 "따로 가서 하는 일"이 아니라 지금 보고 있는
 * 작업에 붙는 일이고, 이력도 그 작업에 붙는다. 원격 Runner의 작업은 로컬 벤더 CLI로 리뷰할
 * 수 없으므로 아예 그리지 않는다.
 */
export function TaskVendorReview({ repo, taskId, host }: Props) {
  const [open, setOpen] = useState(false);
  const [vendors, setVendors] = useState<string[]>(DEFAULT_VENDORS);
  const [focus, setFocus] = useState("");
  const [synthesize, setSynthesize] = useState(true);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [result, setResult] = useState<MultiReviewResult | null>(null);
  const [detail, setDetail] = useState<ReviewDetail | null>(null);
  const [history, setHistory] = useState<ReviewMeta[]>([]);

  const local = host === LOCAL_HOST;

  const refresh = useCallback(() => {
    if (!local) return;
    reviewHistoryList()
      .then((all) =>
        setHistory(all.filter((m) => m.source_kind === "diff" && m.source_ref === String(taskId))),
      )
      .catch(() => setHistory([]));
  }, [local, taskId]);

  // 작업이 바뀌면 앞 작업의 결과가 넘어오면 안 된다 — 패널까지 함께 비운다.
  useEffect(() => {
    setOpen(false);
    setResult(null);
    setDetail(null);
    setErr(null);
    refresh();
  }, [refresh]);

  if (!local) return null;

  const toggleVendor = (v: string) =>
    setVendors((prev) => (prev.includes(v) ? prev.filter((x) => x !== v) : [...prev, v]));

  const run = async () => {
    if (busy || vendors.length === 0) return;
    setBusy(true);
    setErr(null);
    try {
      const { result: res, detail: det } = await multiReview(
        repo,
        "diff",
        String(taskId),
        focus.trim(),
        vendors,
        synthesize,
      );
      setResult(res);
      setDetail(det);
      setOpen(false);
      refresh();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const openHistory = async (id: number) => {
    setErr(null);
    try {
      const record = await reviewGet(id);
      setResult(record.result);
      setDetail(record.detail);
      setOpen(false);
    } catch (e) {
      setErr(String(e));
    }
  };

  const removeHistory = async (id: number) => {
    try {
      await reviewDelete(id);
      refresh();
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <>
      <div className="relative">
        <button
          className="h-7 px-2.5 rounded text-text-secondary hover:text-text"
          onClick={() => setOpen((v) => !v)}
          aria-expanded={open}
          title="이 작업의 diff를 여러 벤더에게 동시에 묻습니다"
        >
          벤더 리뷰{history.length > 0 ? ` ${history.length}` : ""}
        </button>

        {open && (
          <div className="absolute bottom-full right-0 z-30 mb-1 flex w-80 flex-col gap-2 rounded-lg border border-border-strong bg-raised p-3 text-left shadow-e2">
            <div className="flex flex-wrap items-center gap-2">
              {VENDORS.map((v) => (
                <label key={v} className="flex cursor-pointer items-center gap-1 text-xs text-text-secondary">
                  <input
                    type="checkbox"
                    checked={vendors.includes(v)}
                    onChange={() => toggleVendor(v)}
                    aria-label={v}
                  />
                  <span className="font-code">{v}</span>
                </label>
              ))}
            </div>

            <input
              className="h-8 rounded border border-border bg-bg px-2 text-sm text-text outline-none placeholder:text-text-muted focus:border-primary"
              placeholder="중점 (선택) — 예: 보안 취약점 위주로"
              aria-label="중점"
              value={focus}
              onChange={(e) => setFocus(e.target.value)}
            />

            <div className="flex items-center gap-2">
              <label className="flex cursor-pointer items-center gap-1 text-xs text-text-secondary">
                <input
                  type="checkbox"
                  checked={synthesize}
                  onChange={(e) => setSynthesize(e.target.checked)}
                  aria-label="종합"
                />
                종합
              </label>
              <button
                className="ml-auto h-7 rounded px-2.5 font-medium text-primary-bright hover:opacity-80 disabled:opacity-40"
                disabled={busy || vendors.length === 0}
                onClick={() => void run()}
              >
                {busy ? "리뷰 중…" : "실행"}
              </button>
            </div>

            {busy && (
              <div className="text-xs text-text-muted">벤더별 최대 120초 병렬 실행 — 시간이 걸릴 수 있습니다.</div>
            )}
            {err && <div className="font-code text-xs text-status-failed">{err}</div>}

            {history.length > 0 && (
              <div className="flex flex-col gap-1 border-t border-border pt-2">
                <div className="text-xs text-text-muted">이 작업의 리뷰 이력</div>
                {history.map((m) => (
                  <div key={m.id} className="flex items-center gap-2">
                    <button
                      className="min-w-0 flex-1 truncate text-left text-xs text-text-secondary hover:text-text"
                      onClick={() => void openHistory(m.id)}
                    >
                      {stamp(m.created_at)} · {m.ok_count}/{m.total} 완료
                    </button>
                    <IconButton size="sm" tone="danger" label="리뷰 삭제" onClick={() => void removeHistory(m.id)}>
                      <Icon name="x" size={14} />
                    </IconButton>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
      </div>

      {result && (
        <ReviewPanel
          result={result}
          detail={detail ?? undefined}
          onClose={() => {
            setResult(null);
            setDetail(null);
          }}
        />
      )}
    </>
  );
}
