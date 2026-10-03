import { useCallback, useEffect, useState } from "react";
import { taskRef } from "../../lib/ipc";
import { listen } from "@tauri-apps/api/event";
import {
  ensembleList,
  ensembleMetrics,
  ensembleFeedbackHistory,
  ensembleJudge,
  taskDiffStat,
  convoHistory,
  convoSend,
  type CandidateBenchmarkMetrics,
  type EnsembleFeedbackHistory,
  type Task,
  type Judgment,
} from "../../lib/ipc";
import { Icon } from "./icons";
import { AGENT_PRESETS } from "../../lib/agents";
import { taskDotColor } from "../../lib/task-status";
import { ConversationView, eventToItems, type ConvoEventLike, type ConvoItem } from "./ConversationView";
import { EnsembleCompare } from "./EnsembleCompare";
import { EnsembleFeedback } from "./EnsembleFeedback";
import { EnsembleMetrics } from "./EnsembleMetrics";
import { useExperimentalFeatures } from "../../lib/experimental-features";

interface Props {
  ensemble: string;
  onOpenTask: (task: Task) => void;
  onApprove: (id: number) => void;
  onHome: () => void;
}

/** 앙상블 비교/교차검증 뷰 — N개 후보 diffstat 나란히 + 독립 심판이 최선 추천. */
export function EnsembleView({ ensemble, onOpenTask, onApprove, onHome }: Props) {
  const advancedFeatures = useExperimentalFeatures();
  const [cands, setCands] = useState<Task[]>([]);
  const [stats, setStats] = useState<Record<number, string>>({});
  const [convos, setConvos] = useState<Record<number, ConvoItem[]>>({});
  const [metrics, setMetrics] = useState<CandidateBenchmarkMetrics[]>([]);
  const [metricsUnavailable, setMetricsUnavailable] = useState(false);
  const [feedback, setFeedback] = useState<EnsembleFeedbackHistory | null>(null);
  const [feedbackUnavailable, setFeedbackUnavailable] = useState(false);
  const [mode, setMode] = useState<"diff" | "convo" | "compose">("convo"); // 대화 앙상블 기본 = 트랜스크립트 비교
  const [judg, setJudg] = useState<Judgment | null>(null);
  const [judging, setJudging] = useState(false);
  const [judgePref, setJudgePref] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [followup, setFollowup] = useState("");
  const [sending, setSending] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [list, metricResult, feedbackResult] = await Promise.all([
        ensembleList(ensemble),
        ensembleMetrics(ensemble)
          .then((rows) => ({ rows, unavailable: false }))
          .catch(() => ({ rows: [], unavailable: true })),
        ensembleFeedbackHistory()
          .then((history) => ({ history, unavailable: false }))
          .catch(() => ({ history: null, unavailable: true })),
      ]);
      setCands(list);
      setMetrics(metricResult.rows);
      setMetricsUnavailable(metricResult.unavailable);
      setFeedback(feedbackResult.history);
      setFeedbackUnavailable(feedbackResult.unavailable);
      const entries = await Promise.all(
        list.map(async (t) => {
          try {
            return [t.id, await taskDiffStat(taskRef(t))] as const;
          } catch {
            return [t.id, ""] as const;
          }
        }),
      );
      setStats(Object.fromEntries(entries));
      // 후보별 대화 트랜스크립트(있으면) — 벤더 응답 비교용. 터미널 후보는 빈 결과.
      const convoEntries = await Promise.all(
        list.map(async (t) => {
          try {
            const h = await convoHistory(t.id);
            return [t.id, (h.items as ConvoEventLike[]).flatMap(eventToItems)] as const;
          } catch {
            return [t.id, [] as ConvoItem[]] as const;
          }
        }),
      );
      setConvos(Object.fromEntries(convoEntries));
    } catch (e) {
      setErr(String(e));
    }
  }, [ensemble]);

  useEffect(() => {
    setJudg(null);
    void refresh();
    const un = listen("task://state", () => void refresh());
    return () => {
      void un.then((f) => f());
    };
  }, [refresh]);

  const judge = async () => {
    setErr(null);
    setJudging(true);
    try {
      setJudg(await ensembleJudge(ensemble, judgePref));
    } catch (e) {
      setErr(String(e));
    } finally {
      setJudging(false);
    }
  };

  const instruction = cands[0]?.instruction ?? "";
  const allReady =
    cands.length > 0 && cands.every((c) => c.state === "AwaitingReview" || c.state === "Done");
  const concernsFor = (label: string) => judg?.concerns.find(([k]) => k === label)?.[1];
  const cols = Math.min(Math.max(cands.length, 1), 3);
  const isConvoEnsemble = cands.some((c) => c.mode === "conversation");
  const winner = judg ? cands.find((c) => (c.agent ?? c.branch) === judg.winner) : undefined;
  const winnerId = winner?.id;

  // 같은 후속 질의를 모든 후보에 전송 — 각자 자기 세션으로 --resume(멀티턴 앙상블).
  const broadcast = async () => {
    const msg = followup.trim();
    if (!msg || !allReady || sending) return;
    setSending(true);
    setErr(null);
    try {
      await Promise.all(cands.map((c) => convoSend(c.id, msg).catch(() => {})));
      setFollowup("");
      setJudg(null); // 새 턴 → 이전 심판 무효화
      await refresh();
    } finally {
      setSending(false);
    }
  };

  return (
    <div className="flex-1 flex flex-col min-h-0">
      <header className="h-11 border-b border-border flex items-center gap-2 px-3 shrink-0">
        <button className="text-text-muted hover:text-text shrink-0" onClick={onHome} title="홈" aria-label="홈">
          <Icon name="home" size={16} />
        </button>
        <Icon name="scale" size={16} />
        <span className="font-medium truncate max-w-[38%]" title={instruction}>
          {instruction || "비교 실행"}
        </span>
        <span className="text-xs px-2 py-0.5 rounded bg-raised text-text-secondary shrink-0">
          {cands.length} 후보 · 비교 실행
        </span>
        <div className="ml-auto shrink-0 flex items-center gap-1.5">
          <div className="flex rounded border border-border overflow-hidden text-xs">
            <button
              className={`px-2 py-1 ${mode === "convo" ? "bg-raised text-text" : "text-text-secondary hover:text-text"}`}
              onClick={() => setMode("convo")}
            >
              대화
            </button>
            <button
              className={`px-2 py-1 ${mode === "diff" ? "bg-raised text-text" : "text-text-secondary hover:text-text"}`}
              onClick={() => setMode("diff")}
            >
              Diff
            </button>
            <button
              className={`px-2 py-1 ${mode === "compose" ? "bg-raised text-text" : "text-text-secondary hover:text-text"} disabled:opacity-40`}
              disabled={winnerId == null || cands.length < 2}
              onClick={() => setMode("compose")}
              title={
                winnerId == null
                  ? "교차검증으로 추천 후보를 먼저 정하면 조합할 수 있습니다"
                  : "후보별 hunk를 골라 하나로 조합"
              }
            >
              조합
            </button>
          </div>
          <select
            className="text-xs bg-bg border border-border rounded px-1.5 py-1 text-text-secondary outline-none focus:border-primary"
            value={judgePref}
            onChange={(e) => setJudgePref(e.target.value)}
            title="평가 모델 (자동 = 후보에 없는 모델)"
          >
            <option value="">평가 모델: 자동</option>
            {AGENT_PRESETS.map((p) => (
              <option key={p.key} value={p.key}>
                평가 모델: {p.key}
              </option>
            ))}
          </select>
          <button
            className="flex items-center gap-1 text-sm px-3 py-1 rounded-md bg-primary/15 text-primary-bright disabled:opacity-40"
            disabled={judging || cands.length < 2 || !allReady}
            onClick={judge}
            title={allReady ? "후보 비교 평가" : "후보 자율수행 완료를 기다리는 중"}
          >
            <Icon name="scale" size={14} />
            {judging ? "평가 중…" : "교차검증 (평가 모델)"}
          </button>
        </div>
      </header>

      <div className="flex-1 min-h-0 overflow-auto">
      {err && (
        <div className="bg-dangerbg border-b border-dangerborder text-status-failed text-sm px-3 py-1 font-code shrink-0">
          {err}
          <button className="ml-2 text-text-muted hover:text-text" onClick={() => setErr(null)}>
            닫기
          </button>
        </div>
      )}

      {advancedFeatures && <EnsembleMetrics metrics={metrics} unavailable={metricsUnavailable} />}
      <EnsembleFeedback
        history={feedback}
        currentEnsemble={ensemble}
        unavailable={feedbackUnavailable}
      />

      {judg && (
        <div className="m-3 mb-0 p-3 rounded-lg border border-primary/40 bg-primary/5 shrink-0">
          <div className="text-sm">
            <span className="text-text-muted">평가 모델 추천: </span>
            <span className="font-medium text-primary-bright font-code">{judg.winner}</span>
            {judg.ranking.length > 1 && (
              <span className="text-text-muted text-xs"> · 순위 {judg.ranking.join(" › ")}</span>
            )}
          </div>
          {judg.rationale && <div className="text-sm text-text-secondary mt-1">{judg.rationale}</div>}
          {winnerId != null && (
            <button
              className="mt-2 text-xs px-2.5 py-1 rounded-md bg-primary/15 text-primary-bright"
              onClick={() => winner && onOpenTask(winner)}
              title="선택한 후보의 대화를 열어 이어서 진행"
            >
              선택한 후보로 계속 →
            </button>
          )}
        </div>
      )}

      {mode === "compose" && winnerId != null ? (
        <EnsembleCompare
          candidates={cands}
          winnerTaskId={winnerId}
          ensemble={ensemble}
          onComposed={() => void refresh()}
        />
      ) : (
      <div
        className="p-3 grid gap-3 items-start"
        style={{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }}
      >
        {cands.map((c) => {
          const label = c.agent ?? c.branch;
          const isWinner = judg?.winner === label;
          return (
            <div
              key={c.id}
              className={`rounded-lg border p-3 flex flex-col min-h-0 ${
                isWinner ? "border-primary-bright" : "border-border"
              }`}
            >
              <div className="flex items-center gap-2 mb-2">
                <span
                  className="w-2 h-2 rounded-full shrink-0"
                  style={{ background: taskDotColor(c) }}
                />
                <span className="font-medium text-sm font-code truncate">{label}</span>
                {isWinner && (
                  <span className="text-[11px] px-1.5 rounded bg-primary/20 text-primary-bright shrink-0">
                    추천
                  </span>
                )}
                <span className="ml-auto text-xs text-text-muted shrink-0">{c.state}</span>
              </div>
              {mode === "convo" && (convos[c.id]?.length ?? 0) > 0 ? (
                <div className="max-h-72 overflow-auto bg-bg rounded border border-border">
                  <ConversationView items={convos[c.id]} busy={c.state === "Running"} />
                </div>
              ) : (
                <pre className="text-xs font-code text-text-secondary whitespace-pre-wrap max-h-44 overflow-auto bg-bg rounded p-2">
                  {mode === "convo" && c.state === "Running"
                    ? "(대화 수행 중…)"
                    : stats[c.id]?.trim() || (c.state === "Running" ? "(자율 수행 중…)" : "(변경 없음)")}
                </pre>
              )}
              {concernsFor(label) && (
                <div className="text-xs mt-2" style={{ color: "var(--c-awaiting)" }}>
                  ⚠ {concernsFor(label)}
                </div>
              )}
              <div className="flex gap-1.5 mt-2">
                <button
                  className="text-xs px-2 py-1 rounded border border-border text-text-secondary hover:text-text"
                  onClick={() => onOpenTask(c)}
                >
                  열기
                </button>
                <button
                  className="text-xs px-2 py-1 rounded bg-status-done/15 text-status-done disabled:opacity-40"
                  disabled={c.state !== "AwaitingReview"}
                  onClick={() => onApprove(c.id)}
                  title="이 후보를 승인해 머지하고 나머지 후보는 자동으로 버립니다"
                >
                  이 후보로 머지
                </button>
              </div>
            </div>
          );
        })}
      </div>
      )}
      </div>

      {isConvoEnsemble && (
        <footer className="border-t border-border px-3 py-2 shrink-0">
          <div className="flex items-center gap-2">
            <input
              className="flex-1 bg-bg border border-border rounded px-3 py-1.5 text-sm text-text outline-none focus:border-primary placeholder:text-text-muted disabled:opacity-50"
              placeholder={
                allReady ? "모든 후보에게 후속 질의… (Enter)" : "후보 자율수행 완료 후 후속 질의 가능"
              }
              value={followup}
              disabled={!allReady || sending}
              onChange={(e) => setFollowup(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void broadcast();
              }}
            />
            <button
              className="text-sm px-3 py-1.5 rounded-md bg-primary/15 text-primary-bright disabled:opacity-40 shrink-0"
              disabled={!allReady || sending || !followup.trim()}
              onClick={() => void broadcast()}
            >
              {sending ? "전송 중…" : `${cands.length}개 후보에 전송`}
            </button>
          </div>
        </footer>
      )}
    </div>
  );
}
