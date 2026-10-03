import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  PIPELINE_CHANGED_EVENT,
  PIPELINE_VENDORS,
  availableVendors,
  boundOutput,
  canApprovePlan,
  canCancel,
  canResume,
  formatDuration,
  groupFindings,
  hasEscalation,
  parseSpec,
  synthesisOf,
  pipelineCancel,
  pipelineGet,
  pipelineResume,
  pipelineRetryTicket,
  pipelineSkipTicket,
  pipelineVendors,
  runStateInfo,
  stepDurationSecs,
  stepKindLabel,
  stepStatusLabel,
  ticketDeps,
  vendorLabel,
  type PipelineDetail,
  type StepRow,
  type TicketRow,
} from "../../../lib/pipeline";
import { reviewGet } from "../../../lib/ipc";
import { BTN_DANGER, BTN_OUTLINE, BTN_PRIMARY, ErrorLine, INPUT, Section, StateBadge, VendorTag } from "./parts";
import { PlanApproval } from "./PlanApproval";
import { SynthesisView } from "./SynthesisView";

interface Props {
  runId: number;
  onBack: () => void;
  /** 티켓·통합 작업의 기존 세션 화면으로 이동한다. */
  onOpenTask: (taskId: number) => void;
  /** 기존 `taskApprove` 경로(사이드바 머지와 동일). 모든 승인 가드가 그대로 적용된다. */
  onApproveIntegration: (taskId: number) => Promise<void> | void;
}

function Escalation({ detail, vendors, onChanged, onError }: {
  detail: PipelineDetail;
  vendors: string[];
  onChanged: () => void;
  onError: (message: string | null) => void;
}) {
  const [choice, setChoice] = useState<Record<number, string>>({});
  const [busy, setBusy] = useState(false);
  const escalated = detail.tickets.filter((t) => t.state === "escalated");
  const act = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    onError(null);
    try {
      await fn();
      onChanged();
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Section title="에스컬레이션">
      {detail.run.paused_reason && <p className="mb-2 text-sm">일시정지 사유: {detail.run.paused_reason}</p>}
      {escalated.map((t) => {
        const vendor = choice[t.id] ?? t.vendor;
        return (
          <div key={t.id} className="mb-3 rounded-md border border-status-failed p-3">
            <p className="text-sm font-medium">
              <span className="font-code">{t.key}</span> {t.title}
            </p>
            <pre className="my-2 max-h-48 overflow-auto whitespace-pre-wrap break-words font-code text-xs" aria-label={`${t.key} 마지막 오류`}>
              {boundOutput(t.last_error) || "기록된 오류가 없습니다."}
            </pre>
            <div className="flex flex-wrap items-center gap-2">
              <label className="flex items-center gap-1 text-xs">
                벤더
                <select className={INPUT} aria-label={`${t.key} 재시도 벤더`} value={vendor} onChange={(e) => setChoice({ ...choice, [t.id]: e.target.value })}>
                  {PIPELINE_VENDORS.filter((v) => vendors.includes(v) || v === t.vendor).map((v) => (
                    <option key={v} value={v} disabled={!vendors.includes(v)}>
                      {vendorLabel(v)}
                    </option>
                  ))}
                </select>
              </label>
              <button type="button" className={BTN_PRIMARY} disabled={busy} onClick={() => void act(() => pipelineRetryTicket(detail.run.id, t.id, vendor))}>
                벤더 변경 후 재시도
              </button>
              <button type="button" className={BTN_OUTLINE} disabled={busy} onClick={() => void act(() => pipelineSkipTicket(detail.run.id, t.id))}>
                티켓 제외
              </button>
            </div>
          </div>
        );
      })}
    </Section>
  );
}

function FinalApproval({ detail, onOpenTask, onApproveIntegration }: {
  detail: PipelineDetail;
  onOpenTask: Props["onOpenTask"];
  onApproveIntegration: Props["onApproveIntegration"];
}) {
  const { run } = detail;
  const [remaining, setRemaining] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    setRemaining(null);
    if (run.final_review_id == null) return;
    reviewGet(run.final_review_id).then(
      (r) => live && setRemaining(groupFindings(synthesisOf(r.result)).blocking.length),
      () => live && setRemaining(null),
    );
    return () => {
      live = false;
    };
  }, [run.final_review_id]);
  const taskId = run.integration_task_id;
  const approve = async () => {
    if (taskId == null) return;
    if (!window.confirm(`승인하면 ${run.integration_branch ?? "통합 브랜치"}가 ${run.base_branch}에 머지됩니다. 계속할까요?`)) return;
    setBusy(true);
    setError(null);
    try {
      await onApproveIntegration(taskId);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Section title="머지 승인 (G2)">
      <ErrorLine message={error} />
      {remaining != null && remaining > 0 && (
        <p role="status" className="mb-2 rounded border border-status-failed p-2 text-sm text-status-failed">
          남은 blocking 지적 {remaining}건이 있습니다. 확인 후 승인하세요.
        </p>
      )}
      {run.last_error && (
        <p role="note" className="mb-2 whitespace-pre-wrap rounded border border-status-awaiting p-2 text-sm text-status-awaiting">
          {boundOutput(run.last_error)}
        </p>
      )}
      <p className="mb-2 text-sm text-text-secondary">승인하면 통합 브랜치가 {run.base_branch}에 머지됩니다. 기존 작업 승인과 같은 검사를 거칩니다.</p>
      {run.final_review_id != null && <SynthesisView reviewId={run.final_review_id} />}
      <div className="mt-3 flex gap-2">
        <button type="button" className={BTN_OUTLINE} disabled={taskId == null} onClick={() => taskId != null && onOpenTask(taskId)}>
          통합 작업 열기 (diff)
        </button>
        <button type="button" className={BTN_PRIMARY} disabled={taskId == null || busy} onClick={() => void approve()}>
          승인하고 머지
        </button>
      </div>
    </Section>
  );
}

function StepsTimeline({ steps, tickets }: { steps: StepRow[]; tickets: TicketRow[] }) {
  const keyOf = (id: number | null) => tickets.find((t) => t.id === id)?.key;
  if (steps.length === 0) return <p className="text-sm text-text-muted">아직 단계 기록이 없습니다.</p>;
  return (
    <ol className="space-y-1">
      {steps.map((s) => (
        <li key={s.id} className="text-sm">
          <details>
            <summary className="cursor-pointer">
              {stepKindLabel(s.kind)}
              {keyOf(s.ticket_id) ? ` · ${keyOf(s.ticket_id)}` : ""}
              {s.vendor ? ` · ${vendorLabel(s.vendor)}` : ""} · {stepStatusLabel(s.status)} · {formatDuration(stepDurationSecs(s))}
            </summary>
            <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words font-code text-xs">{boundOutput(s.output) || "출력이 없습니다."}</pre>
          </details>
        </li>
      ))}
    </ol>
  );
}

/** 실행 한 건의 보드 — 상태·스펙·티켓·단계·리뷰와 단계별 승인 화면. `pipeline://changed`로 갱신한다. */
export function RunDetail({ runId, onBack, onOpenTask, onApproveIntegration }: Props) {
  const [detail, setDetail] = useState<PipelineDetail | null>(null);
  const [vendors, setVendors] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [openReview, setOpenReview] = useState<number | null>(null);

  const load = useCallback(async () => {
    try {
      setDetail(await pipelineGet(runId));
    } catch (e) {
      setError(String(e));
    }
  }, [runId]);

  useEffect(() => {
    void load();
    pipelineVendors().then((v) => setVendors(availableVendors(v)), () => setVendors([]));
    const un = listen<number>(PIPELINE_CHANGED_EVENT, (ev) => {
      if (ev.payload == null || Number(ev.payload) === runId) void load();
    });
    return () => {
      void un.then((f) => f());
    };
  }, [load, runId]);

  if (!detail) {
    return (
      <div>
        <button type="button" className={BTN_OUTLINE} onClick={onBack}>목록으로</button>
        <ErrorLine message={error} />
        {!error && <p className="mt-2 text-sm text-text-muted">불러오는 중…</p>}
      </div>
    );
  }

  const { run, tickets, steps, reviews } = detail;
  const spec = parseSpec(run.spec_json);
  const act = async (fn: () => Promise<unknown>) => {
    setError(null);
    try {
      await fn();
      await load();
    } catch (e) {
      setError(String(e));
    }
  };
  const cancel = () => {
    if (!window.confirm("이 실행을 취소할까요? 진행 중인 티켓 작업이 중단됩니다.")) return;
    void act(() => pipelineCancel(run.id));
  };

  return (
    <div>
      <div className="mb-4 flex items-center gap-2">
        <button type="button" className={BTN_OUTLINE} onClick={onBack}>목록으로</button>
        <h1 className="flex-1 truncate text-lg font-semibold">#{run.id} {run.goal}</h1>
        <StateBadge kind="run" state={run.state} />
        {canResume(run) && (
          <button type="button" className={BTN_OUTLINE} onClick={() => void act(() => pipelineResume(run.id))}>일시정지 해제</button>
        )}
        {canCancel(run) && (
          <button type="button" className={BTN_DANGER} onClick={cancel}>취소</button>
        )}
      </div>
      <ErrorLine message={error} />
      {run.state === "paused" && run.paused_reason && !hasEscalation(tickets) && (
        <p role="status" className="mb-3 text-sm text-status-awaiting">일시정지: {run.paused_reason}</p>
      )}
      {run.last_error && run.state === "failed" && <ErrorLine message={run.last_error} />}
      <p className="mb-4 text-xs text-text-muted">
        {run.repo} · base {run.base_branch}
        {run.integration_branch ? ` · 통합 ${run.integration_branch}` : ""} · 상태 {runStateInfo(run.state).label}
      </p>

      {spec && (
        <Section title="스펙">
          <p className="mb-2 text-sm">{spec.goal || run.goal}</p>
          <ul className="space-y-1 text-sm">
            {spec.requirements.map((r) => (
              <li key={r.id}>
                <span className="font-code text-xs text-text-muted">{r.id}</span> {r.text}
                {r.acceptance ? <span className="text-xs text-text-muted"> · 수용: {r.acceptance}</span> : null}
              </li>
            ))}
          </ul>
        </Section>
      )}

      {canApprovePlan(run) && (
        <PlanApproval key={run.spec_revision} run={run} tickets={tickets} vendors={vendors} onReload={() => void load()} />
      )}
      {hasEscalation(tickets) && (
        <Escalation detail={detail} vendors={vendors} onChanged={() => void load()} onError={setError} />
      )}
      {run.state === "awaiting_merge_approval" && (
        <FinalApproval detail={detail} onOpenTask={onOpenTask} onApproveIntegration={async (id) => { await onApproveIntegration(id); await load(); }} />
      )}

      <Section title="티켓">
        {tickets.length === 0 ? (
          <p className="text-sm text-text-muted">아직 티켓이 없습니다.</p>
        ) : (
          <table className="w-full text-left text-sm">
            <thead className="text-xs text-text-muted">
              <tr>
                <th scope="col">키</th>
                <th scope="col">제목</th>
                <th scope="col">벤더</th>
                <th scope="col">리뷰어</th>
                <th scope="col">상태</th>
                <th scope="col">시도</th>
                <th scope="col">재배정</th>
                <th scope="col">선행</th>
                <th scope="col">세션</th>
              </tr>
            </thead>
            <tbody>
              {tickets.map((t) => (
                <tr key={t.id} className="border-t border-border">
                  <td className="font-code text-xs">{t.key}</td>
                  <td>{t.title}</td>
                  <td><VendorTag vendor={t.vendor} /></td>
                  <td><VendorTag vendor={t.reviewer_vendor} /></td>
                  <td><StateBadge kind="ticket" state={t.state} /></td>
                  <td>{t.attempt}</td>
                  <td>{t.reassigned ? "예" : "-"}</td>
                  <td className="font-code text-xs">{ticketDeps(t).join(", ") || "-"}</td>
                  <td>
                    {t.task_id != null ? (
                      <button type="button" className="text-primary-bright" aria-label={`${t.key} 세션 열기`} onClick={() => onOpenTask(t.task_id!)}>
                        열기
                      </button>
                    ) : "-"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Section>

      <Section title="단계 기록">
        <StepsTimeline steps={steps} tickets={tickets} />
      </Section>

      <Section title="리뷰">
        {reviews.length === 0 ? (
          <p className="text-sm text-text-muted">리뷰 기록이 없습니다.</p>
        ) : (
          <ul className="space-y-2">
            {reviews.map((r) => (
              <li key={r.id} className="text-sm">
                <div className="flex items-center gap-2">
                  <span>{stepKindLabel(r.pipeline_step) || r.focus}</span>
                  <span className="text-xs text-text-muted">{r.ok_count}/{r.total} 벤더 응답</span>
                  <button type="button" className="text-primary-bright" aria-expanded={openReview === r.id} onClick={() => setOpenReview(openReview === r.id ? null : r.id)}>
                    {openReview === r.id ? "접기" : "보기"}
                  </button>
                </div>
                {openReview === r.id && <div className="mt-2"><SynthesisView reviewId={r.id} /></div>}
              </li>
            ))}
          </ul>
        )}
      </Section>
    </div>
  );
}
