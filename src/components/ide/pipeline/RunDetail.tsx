import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  PIPELINE_CHANGED_EVENT,
  PIPELINE_VENDORS,
  PROCEED_ANYWAY_ANSWER,
  STEP_BUDGET_SECS,
  availableVendors,
  baselineFailure,
  boundOutput,
  canAnswerClarification,
  canApprovePlan,
  canCancel,
  canResume,
  elapsedSecs,
  formatDuration,
  groupFindings,
  hasEscalation,
  integratedPercent,
  nextActionHint,
  parseClarification,
  parseSpec,
  repoBasename,
  runningStep,
  synthesisOf,
  pipelineAnswerClarification,
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
  ticketStateInfo,
  vendorLabel,
  type PipelineDetail,
  type RunRow,
  type StepRow,
  type TicketRow,
  type VendorAvailability,
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

function Escalation({ detail, vendors, reasons, onChanged, onError }: {
  detail: PipelineDetail;
  vendors: string[];
  /** 사용 불가 벤더의 사유(벤더 → 사유). */
  reasons: Record<string, string>;
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
                    <option key={v} value={v} disabled={!vendors.includes(v)} title={reasons[v]}>
                      {vendorLabel(v)}
                      {!vendors.includes(v) && reasons[v] ? ` (${reasons[v]})` : ""}
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

const ACTIVE_TICKET_STATES: readonly string[] = ["running", "verifying", "reviewing", "fixing", "integrating"];

/** 자동 단계가 도는 동안 무엇을 기다리는지 보인다. 분할·계획 리뷰는 10분 넘게 조용히 돌 수 있어 상태 배지만으로는 부족하다. */
function ProgressCard({ run, steps, tickets }: { run: RunRow; steps: StepRow[]; tickets: TicketRow[] }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, []);
  const step = runningStep(steps);
  const key = step ? tickets.find((t) => t.id === step.ticket_id)?.key : undefined;
  const budget = step ? STEP_BUDGET_SECS[step.kind] : undefined;
  const hint: Record<string, string> = {
    drafting: "Claude가 저장소를 읽고 스펙과 티켓을 만드는 중입니다. 구현 요청이 아니라고 판단하면 질문으로 돌아옵니다.",
    plan_review: "가용 벤더 전원이 계획을 독립 리뷰하고 Claude가 종합합니다. blocking이 남으면 한 번 자동 수정한 뒤 승인 대기로 갑니다.",
    final_review: "통합 diff를 전원이 리뷰하고 종합합니다.",
  };
  const runningIds = new Set(steps.filter((s) => s.status === "running").map((s) => s.ticket_id));
  const active = run.state === "executing" ? tickets.filter((t) => ACTIVE_TICKET_STATES.includes(t.state)) : [];
  return (
    <Section title="진행 상황">
      <p role="status" aria-label="진행 상황" className="text-sm">
        {step ? (
          <>
            {stepKindLabel(step.kind)}
            {key ? ` · ${key}` : ""}
            {step.vendor ? ` · ${vendorLabel(step.vendor)}` : ""} · 경과 {formatDuration(elapsedSecs(step, now))}
            {budget ? ` / 예산 ${formatDuration(budget)}` : ""}
          </>
        ) : (
          "다음 단계를 기다리는 중입니다."
        )}
      </p>
      {hint[run.state] && <p className="mt-1 text-xs text-text-muted">{hint[run.state]}</p>}
      {active.length > 0 && (
        <ul aria-label="티켓 진행" className="mt-2 space-y-1 text-sm">
          {active.map((t) => (
            <li key={t.id} className="flex flex-wrap items-center gap-x-2">
              <span className="font-code text-xs">{t.key}</span>
              <span className="min-w-0 truncate">{t.title}</span>
              <span className="text-xs text-status-running">{ticketStateInfo(t.state).label}</span>
              <span className="text-xs text-text-muted">{vendorLabel(t.vendor)}</span>
              {t.state === "running" && t.task_id != null && !runningIds.has(t.id) && (
                <span className="text-xs text-text-muted">에이전트 세션 진행 중</span>
              )}
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

/** 기준선(base 브랜치)에서 이미 실패하는 검사 — 티켓 판정에서 제외됨을 알린다. */
function BaselineWarning({ steps }: { steps: StepRow[] }) {
  const base = baselineFailure(steps);
  if (!base) return null;
  return (
    <section aria-label="기준선 검증 경고" className="mb-6 rounded-md border border-status-awaiting p-3">
      <h2 className="mb-1 text-sm font-medium text-status-awaiting">기준선에서 이미 실패하는 검사가 있습니다</h2>
      <p className="mb-2 text-sm text-text-secondary">
        아래 명령은 base 브랜치에서도 이미 실패하므로 티켓 검증 판정에서 제외됩니다. 저장소에 맞는 검증 명령은{" "}
        <span className="font-code text-xs">.dojang/validate.toml</span>에서 지정할 수 있습니다.
      </p>
      <ul className="mb-2 list-disc space-y-0.5 pl-5">
        {base.failed_commands.map((c) => <li key={c} className="font-code text-xs">{c}</li>)}
      </ul>
      {base.summary && (
        <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words font-code text-xs">{boundOutput(base.summary)}</pre>
      )}
    </section>
  );
}

/** 분할 심판이 티켓 대신 질문(clarify)이나 수행 불가(decline)를 냈다. 답하면 그 답을 피드백으로 다시 분할한다. */
function ClarificationCard({ run, onChanged, onError }: { run: RunRow; onChanged: () => void; onError: (e: string | null) => void }) {
  const c = parseClarification(run.clarification_json);
  const [answer, setAnswer] = useState("");
  const [busy, setBusy] = useState(false);
  if (!c) return null;
  const send = async (text: string) => {
    setBusy(true);
    onError(null);
    try {
      await pipelineAnswerClarification(run.id, text);
      onChanged();
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const isDecline = c.decision === "decline";
  return (
    <Section title={isDecline ? "분할 심판이 수행을 거절했습니다" : "분할 전 확인 질문"}>
      <p className="mb-2 text-sm text-text-secondary">
        {isDecline
          ? "심판이 이 목표를 이 저장소의 파이프라인으로 수행할 수 없다고 판단했습니다. 목표를 다시 설명해 보내거나 실행을 취소하세요."
          : "심판이 목표를 구현 요청으로 확정하지 못했습니다. 아래 질문에 답하면 그 답을 반영해 분할을 이어 갑니다."}
      </p>
      {c.reason && <p role="note" className="mb-2 whitespace-pre-wrap text-sm">{c.reason}</p>}
      {c.questions.length > 0 && (
        <ol aria-label="확인 질문" className="mb-2 list-decimal space-y-1 pl-5 text-sm">
          {c.questions.map((q, i) => <li key={i}>{q}</li>)}
        </ol>
      )}
      <label className="flex flex-col gap-1 text-xs">
        답변
        <textarea className={INPUT} rows={4} value={answer} aria-label="질문 답변" onChange={(ev) => setAnswer(ev.target.value)} />
      </label>
      <div className="mt-2 flex gap-2">
        <button type="button" className={BTN_PRIMARY} disabled={busy || !answer.trim()} onClick={() => void send(answer.trim())}>
          답변하고 분할 계속
        </button>
        <button type="button" className={BTN_OUTLINE} disabled={busy} onClick={() => void send(PROCEED_ANYWAY_ANSWER)}>
          답하지 않고 그대로 진행
        </button>
      </div>
    </Section>
  );
}

const STEPS_PAGE = 30;

function StepsTimeline({ steps, tickets }: { steps: StepRow[]; tickets: TicketRow[] }) {
  const [filter, setFilter] = useState("all");
  const [limit, setLimit] = useState(STEPS_PAGE);
  const keyOf = (id: number | null) => tickets.find((t) => t.id === id)?.key;
  if (steps.length === 0) return <p className="text-sm text-text-muted">아직 단계 기록이 없습니다.</p>;
  const sorted = [...steps].sort((a, b) => b.started_at - a.started_at || b.id - a.id);
  const filtered = sorted.filter((s) =>
    filter === "all" ? true : filter === "run" ? s.ticket_id == null : keyOf(s.ticket_id) === filter,
  );
  const shown = filtered.slice(0, limit);
  return (
    <div>
      <label className="mb-2 flex items-center gap-1 text-xs">
        보기
        <select className={INPUT} aria-label="단계 필터" value={filter} onChange={(e) => { setFilter(e.target.value); setLimit(STEPS_PAGE); }}>
          <option value="all">전체</option>
          <option value="run">실행 단계</option>
          {tickets.map((t) => <option key={t.id} value={t.key}>{t.key}</option>)}
        </select>
      </label>
      <ol className="space-y-1">
        {shown.map((s) => (
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
      {filtered.length > shown.length && (
        <button type="button" className={`${BTN_OUTLINE} mt-2`} onClick={() => setLimit(limit + STEPS_PAGE)}>
          더 보기 ({filtered.length - shown.length}건 남음)
        </button>
      )}
    </div>
  );
}

/** 실행 한 건의 보드 — 상태·스펙·티켓·단계·리뷰와 단계별 승인 화면. `pipeline://changed`로 갱신한다. */
export function RunDetail({ runId, onBack, onOpenTask, onApproveIntegration }: Props) {
  const [detail, setDetail] = useState<PipelineDetail | null>(null);
  const [vendorInfo, setVendorInfo] = useState<VendorAvailability[]>([]);
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
    pipelineVendors().then(setVendorInfo, () => setVendorInfo([]));
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
  const vendors = availableVendors(vendorInfo);
  const reasons: Record<string, string> = {};
  for (const v of vendorInfo) if (!v.available && v.reason) reasons[v.vendor] = v.reason;
  const integrated = tickets.filter((t) => t.state === "integrated").length;
  const countable = tickets.filter((t) => t.state !== "cancelled").length;
  const escalated = tickets.filter((t) => t.state === "escalated").length;
  const pct = integratedPercent(countable, integrated);
  const specOpen = run.state === "drafting" || run.state === "awaiting_plan_approval";
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
      <div className="mb-2 flex items-center gap-2">
        <button type="button" className={BTN_OUTLINE} onClick={onBack}>목록으로</button>
        <h1 className="flex-1 truncate text-lg font-semibold" title={run.goal}>#{run.id} {run.goal}</h1>
        <StateBadge kind="run" state={run.state} />
        {canResume(run) && !canAnswerClarification(run) && (
          <button type="button" className={BTN_OUTLINE} onClick={() => void act(() => pipelineResume(run.id))}>일시정지 해제</button>
        )}
        {canCancel(run) && (
          <button type="button" className={BTN_DANGER} onClick={cancel}>취소</button>
        )}
      </div>
      <div className="mb-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-text-muted">
        <span title={run.repo}>{repoBasename(run.repo)}</span>
        <span>base {run.base_branch}{run.integration_branch ? ` · 통합 ${run.integration_branch}` : ""}</span>
        {countable > 0 && (
          <span className="flex items-center gap-1.5">
            통합 {integrated}/{countable}
            <span className="h-1 w-24 overflow-hidden rounded bg-raised" role="progressbar" aria-label="통합 진행률" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct}>
              <span className="block h-full bg-primary" style={{ width: `${pct}%` }} />
            </span>
          </span>
        )}
      </div>
      <p className="mb-4 text-sm" aria-label="지금 할 일">
        지금 할 일: {nextActionHint(run, escalated)}
      </p>
      <ErrorLine message={error} />
      {run.state === "paused" && run.paused_reason && !hasEscalation(tickets) && !canAnswerClarification(run) && (
        <p role="status" className="mb-3 text-sm text-status-awaiting">일시정지: {run.paused_reason}</p>
      )}
      {run.last_error && run.state === "failed" && <ErrorLine message={run.last_error} />}

      {canAnswerClarification(run) && (
        <ClarificationCard key={run.spec_revision} run={run} onChanged={() => void load()} onError={setError} />
      )}
      {canApprovePlan(run) && (
        <PlanApproval key={run.spec_revision} run={run} tickets={tickets} vendors={vendors} reasons={reasons} onReload={() => void load()} />
      )}
      {hasEscalation(tickets) && (
        <Escalation detail={detail} vendors={vendors} reasons={reasons} onChanged={() => void load()} onError={setError} />
      )}
      {run.state === "awaiting_merge_approval" && (
        <FinalApproval detail={detail} onOpenTask={onOpenTask} onApproveIntegration={async (id) => { await onApproveIntegration(id); await load(); }} />
      )}

      {runStateInfo(run.state).tone === "running" && (
        <ProgressCard run={run} steps={steps} tickets={tickets} />
      )}
      <BaselineWarning steps={steps} />

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
                <th scope="col">최근 오류</th>
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
                  <td>시도 {t.attempt}{t.gen ? ` · 세대 ${t.gen}` : ""}</td>
                  <td className="max-w-48 truncate text-xs text-text-muted" title={t.last_error ?? undefined}>
                    {t.last_error ? t.last_error.split("\n").find((l) => l.trim()) ?? "" : "-"}
                  </td>
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

      {spec && (
        <details className="mb-6" open={specOpen}>
          <summary className="mb-2 cursor-pointer text-sm font-medium uppercase tracking-wide text-text-muted">스펙</summary>
          <p className="mb-2 text-sm">{spec.goal || run.goal}</p>
          <ul className="space-y-1 text-sm">
            {spec.requirements.map((r) => (
              <li key={r.id}>
                <span className="font-code text-xs text-text-muted">{r.id}</span> {r.text}
                {r.acceptance ? <span className="text-xs text-text-muted"> · 수용: {r.acceptance}</span> : null}
              </li>
            ))}
          </ul>
        </details>
      )}

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
