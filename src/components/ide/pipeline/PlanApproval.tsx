import { useEffect, useMemo, useState } from "react";
import { reviewGet } from "../../../lib/ipc";
import {
  PIPELINE_VENDORS,
  groupFindings,
  hasTicketErrors,
  isRevisionConflict,
  linesToList,
  listToLines,
  synthesisOf,
  pipelineApprovePlan,
  pipelineRejectPlan,
  pipelineUpdateTickets,
  ticketToDraft,
  ticketsEqual,
  validateTickets,
  vendorLabel,
  type RunRow,
  type TicketDraft,
  type TicketRow,
} from "../../../lib/pipeline";
import { BTN_OUTLINE, BTN_PRIMARY, ErrorLine, INPUT, Section } from "./parts";
import { SynthesisView } from "./SynthesisView";

interface Props {
  run: RunRow;
  tickets: TicketRow[];
  vendors: string[];
  /** 사용 불가 벤더의 사유(벤더 → 사유). */
  reasons?: Record<string, string>;
  /** 갱신된 상태를 다시 불러온다 — 승인·반려·개정 충돌 뒤에 부른다. */
  onReload: () => void;
}

/** 계획 리뷰가 남긴 것을 한 줄로 요약한다 — 남은 blocking 수, 응답하지 못한 리뷰어. 종합 전문은 아래 SynthesisView에 있다. */
function PlanReviewBanner({ run }: { run: RunRow }) {
  const [summary, setSummary] = useState<{ blocking: number; failed: string[] } | null>(null);
  useEffect(() => {
    let live = true;
    setSummary(null);
    if (run.plan_review_id == null) return;
    reviewGet(run.plan_review_id).then(
      (r) => {
        if (!live) return;
        const failed = r.result.items.filter((i) => !i.ok).map((i) => vendorLabel(i.vendor));
        setSummary({ blocking: groupFindings(synthesisOf(r.result)).blocking.length, failed });
      },
      () => live && setSummary(null),
    );
    return () => {
      live = false;
    };
  }, [run.plan_review_id]);
  if (!summary || (summary.blocking === 0 && summary.failed.length === 0)) return null;
  return (
    <div role="status" aria-label="계획 리뷰 요약" className="mb-3 rounded border border-status-awaiting p-2 text-sm">
      {summary.blocking > 0 && (
        <p className="text-status-failed">
          계획 리뷰의 blocking 지적 {summary.blocking}건이 {run.plan_fix_used ? "한 번의 자동 수정 뒤에도 " : ""}남아 있습니다.
          아래 종합을 확인하고, 반영이 필요하면 반려 코멘트로 다시 분할시키세요. 승인하면 지적을 그대로 두고 실행합니다.
        </p>
      )}
      {summary.failed.length > 0 && (
        <p className="text-text-secondary">응답하지 못한 리뷰어: {summary.failed.join(", ")} (사유는 아래 종합의 벤더별 항목에 있습니다)</p>
      )}
    </div>
  );
}

/** G1 — 스펙·티켓 배정을 검토·편집하고 승인하거나 코멘트와 함께 반려한다. */
export function PlanApproval({ run, tickets, vendors, reasons = {}, onReload }: Props) {
  const original = useMemo(() => tickets.map(ticketToDraft), [tickets]);
  const [drafts, setDrafts] = useState<TicketDraft[]>(original);
  const [comment, setComment] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const errors = useMemo(() => validateTickets(drafts), [drafts]);
  const reviewerOf = (key: string) => tickets.find((t) => t.key === key)?.reviewer_vendor;

  const patch = (key: string, p: Partial<TicketDraft>) =>
    setDrafts((ds) => ds.map((d) => (d.key === key ? { ...d, ...p } : d)));

  const guard = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      if (isRevisionConflict(e)) {
        setError("다른 곳에서 계획이 바뀌어 최신 내용을 다시 불러옵니다. 편집 내용은 사라집니다.");
        onReload();
      } else {
        setError(String(e));
      }
    } finally {
      setBusy(false);
    }
  };

  const approve = () =>
    guard(async () => {
      let revision = run.spec_revision;
      if (!ticketsEqual(drafts, original)) {
        const updated = await pipelineUpdateTickets(run.id, revision, drafts);
        revision = updated.spec_revision;
      }
      await pipelineApprovePlan(run.id, revision);
      onReload();
    });

  const reject = () =>
    guard(async () => {
      await pipelineRejectPlan(run.id, comment.trim());
      setComment("");
      onReload();
    });

  const blocked = busy || hasTicketErrors(errors);

  return (
    <Section title="계획 승인 (G1)">
      <ErrorLine message={error} />
      <PlanReviewBanner run={run} />
      <div className="space-y-3">
        {drafts.map((d) => {
          const e = errors[d.key] ?? {};
          const others = drafts.filter((o) => o.key !== d.key);
          return (
            <fieldset key={d.key} className="rounded-md border border-border p-3">
              <legend className="px-1 font-code text-xs text-text-muted">{d.key}</legend>
              <div className="grid grid-cols-2 gap-2">
                <label className="col-span-2 flex flex-col gap-1 text-xs">
                  제목
                  <input className={INPUT} value={d.title} aria-label={`${d.key} 제목`} onChange={(ev) => patch(d.key, { title: ev.target.value })} />
                  {e.title && <span role="alert" className="text-status-failed">{e.title}</span>}
                </label>
                <label className="col-span-2 flex flex-col gap-1 text-xs">
                  설명
                  <textarea className={INPUT} rows={2} value={d.body} aria-label={`${d.key} 설명`} onChange={(ev) => patch(d.key, { body: ev.target.value })} />
                </label>
                <label className="flex flex-col gap-1 text-xs">
                  구현 벤더
                  <select className={INPUT} value={d.vendor} aria-label={`${d.key} 벤더`} onChange={(ev) => patch(d.key, { vendor: ev.target.value })}>
                    {PIPELINE_VENDORS.filter((v) => vendors.includes(v) || v === d.vendor).map((v) => (
                      <option key={v} value={v} disabled={!vendors.includes(v)} title={reasons[v]}>
                        {vendorLabel(v)}
                        {!vendors.includes(v) && reasons[v] ? ` (${reasons[v]})` : ""}
                      </option>
                    ))}
                  </select>
                </label>
                <div className="flex flex-col gap-1 text-xs">
                  리뷰 벤더 (읽기 전용)
                  <span className="py-1 text-sm">{reviewerOf(d.key) ? vendorLabel(reviewerOf(d.key)) : "벤더에 맞춰 자동 지정"}</span>
                </div>
                <label className="flex flex-col gap-1 text-xs">
                  허용 경로 (줄마다 하나)
                  <textarea className={`${INPUT} font-code`} rows={3} value={listToLines(d.allowed_paths)} aria-label={`${d.key} 허용 경로`} onChange={(ev) => patch(d.key, { allowed_paths: linesToList(ev.target.value) })} />
                  {e.allowed_paths && <span role="alert" className="text-status-failed">{e.allowed_paths}</span>}
                </label>
                <label className="flex flex-col gap-1 text-xs">
                  수용 명령 (줄마다 하나)
                  <textarea className={`${INPUT} font-code`} rows={3} value={listToLines(d.acceptance_commands)} aria-label={`${d.key} 수용 명령`} onChange={(ev) => patch(d.key, { acceptance_commands: linesToList(ev.target.value) })} />
                  {e.acceptance_commands && <span role="alert" className="text-status-failed">{e.acceptance_commands}</span>}
                </label>
                <div className="col-span-2 text-xs">
                  <span id={`deps-${d.key}`}>선행 티켓</span>
                  <div role="group" aria-labelledby={`deps-${d.key}`} className="flex flex-wrap gap-3 py-1">
                    {others.length === 0 && <span className="text-text-muted">없음</span>}
                    {others.map((o) => (
                      <label key={o.key} className="flex items-center gap-1">
                        <input
                          type="checkbox"
                          checked={d.deps.includes(o.key)}
                          onChange={(ev) => patch(d.key, { deps: ev.target.checked ? [...d.deps, o.key] : d.deps.filter((k) => k !== o.key) })}
                        />
                        {o.key}
                      </label>
                    ))}
                  </div>
                  {e.deps && <span role="alert" className="text-status-failed">{e.deps}</span>}
                </div>
              </div>
            </fieldset>
          );
        })}
      </div>

      {run.plan_review_id != null && (
        <div className="mt-4">
          <h3 className="mb-1 text-sm font-medium">계획 리뷰 종합</h3>
          <SynthesisView reviewId={run.plan_review_id} />
        </div>
      )}

      <div className="mt-4 flex flex-col gap-2">
        <label className="flex flex-col gap-1 text-xs">
          반려 코멘트 (반려 시 필수 — 스펙 수정을 다시 요청합니다)
          <textarea className={INPUT} rows={2} value={comment} onChange={(ev) => setComment(ev.target.value)} />
        </label>
        <div className="flex gap-2">
          <button type="button" className={BTN_PRIMARY} disabled={blocked} onClick={() => void approve()}>
            승인
          </button>
          <button type="button" className={BTN_OUTLINE} disabled={busy || !comment.trim()} onClick={() => void reject()}>
            반려
          </button>
        </div>
      </div>
    </Section>
  );
}
