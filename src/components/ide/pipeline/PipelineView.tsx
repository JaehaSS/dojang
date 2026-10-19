import { useState } from "react";
import {
  groupRuns,
  integratedPercent,
  relativeTime,
  repoBasename,
  stepKindLabel,
  type RunListItem,
} from "../../../lib/pipeline";
import type { PipelineRuns } from "../../../lib/usePipelineRuns";
import { BTN_OUTLINE, ErrorLine, Section, StateBadge } from "./parts";
import { PipelineCreate } from "./PipelineCreate";
import { RunDetail } from "./RunDetail";

interface Props {
  /** App이 불러 둔 실행 목록(사이드바 배지와 공유). */
  list: PipelineRuns;
  /** 열려 있는 실행 id. App이 들고 있어 화면을 벗어났다 돌아와도 유지된다. */
  runId: number | null;
  onSelectRun: (id: number | null) => void;
  /** 사이드바에 등록된 프로젝트 저장소 경로. */
  projects: string[];
  /** 홈 작성기에서 고른 저장소 — 새 실행의 기본 프로젝트. */
  defaultRepo?: string;
  onOpenTask: (taskId: number) => void;
  onApproveIntegration: (taskId: number) => Promise<void> | void;
}

function RunRowItem({ run, onOpen }: { run: RunListItem; onOpen: () => void }) {
  const pct = integratedPercent(run.tickets_total, run.tickets_integrated);
  return (
    <li className="rounded-md border border-border p-2 text-sm">
      <div className="flex items-center gap-2">
        <StateBadge kind="run" state={run.state} />
        <button type="button" className="min-w-0 flex-1 truncate text-left hover:text-text" onClick={onOpen}>
          #{run.id} {run.goal}
        </button>
        <span className="shrink-0 text-xs text-text-muted" title={run.repo}>{repoBasename(run.repo)}</span>
      </div>
      <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-text-muted">
        {run.tickets_total > 0 && (
          <span className="flex items-center gap-1.5">
            통합 {run.tickets_integrated}/{run.tickets_total}
            <span className="h-1 w-20 overflow-hidden rounded bg-raised" role="progressbar" aria-label={`#${run.id} 통합 진행률`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct}>
              <span className="block h-full bg-primary" style={{ width: `${pct}%` }} />
            </span>
          </span>
        )}
        {run.tickets_escalated > 0 && <span className="text-status-failed">에스컬레이션 {run.tickets_escalated}건</span>}
        {run.active_step_kind && <span className="text-status-running">{stepKindLabel(run.active_step_kind)} 진행 중</span>}
        <span className="ml-auto">{relativeTime(run.updated_at)}</span>
      </div>
    </li>
  );
}

function RunGroup({ title, runs, onOpen, empty }: { title: string; runs: RunListItem[]; onOpen: (id: number) => void; empty?: string }) {
  if (runs.length === 0 && !empty) return null;
  return (
    <Section title={`${title} ${runs.length}`}>
      {runs.length === 0 ? (
        <p className="text-sm text-text-muted">{empty}</p>
      ) : (
        <ul className="space-y-1.5">
          {runs.map((r) => <RunRowItem key={r.id} run={r} onOpen={() => onOpen(r.id)} />)}
        </ul>
      )}
    </Section>
  );
}

/** 멀티벤더 파이프라인 — 모든 프로젝트의 실행 목록, 새 실행 폼, 선택한 실행의 보드. */
export function PipelineView({ list, runId, onSelectRun, projects, defaultRepo, onOpenTask, onApproveIntegration }: Props) {
  const { runs, error, reload } = list;
  const [createOpen, setCreateOpen] = useState(false);

  if (runId != null) {
    return (
      <div className="flex h-full min-h-0 flex-col overflow-auto p-6">
        <div className="mx-auto w-full max-w-4xl">
          <RunDetail runId={runId} onBack={() => { onSelectRun(null); reload(); }} onOpenTask={onOpenTask} onApproveIntegration={onApproveIntegration} />
        </div>
      </div>
    );
  }

  const groups = groupRuns(runs);
  const recent = [...runs].sort((a, b) => b.updated_at - a.updated_at)[0];
  const showCreate = runs.length === 0 || createOpen;
  const started = (run: { id: number }) => {
    reload();
    setCreateOpen(false);
    onSelectRun(run.id);
  };

  return (
    <div className="flex h-full min-h-0 flex-col overflow-auto p-6">
      <div className="mx-auto w-full max-w-4xl">
        <div className="mb-4 flex items-center gap-2">
          <h1 className="flex-1 text-lg font-semibold">멀티벤더 파이프라인</h1>
          {runs.length > 0 && (
            <button type="button" className={BTN_OUTLINE} aria-expanded={createOpen} onClick={() => setCreateOpen((v) => !v)}>
              {createOpen ? "새 실행 닫기" : "새 실행"}
            </button>
          )}
        </div>
        <ErrorLine message={error} />
        {showCreate && <PipelineCreate projects={projects} defaultRepo={defaultRepo} recentRepo={recent?.repo} onStarted={started} />}
        {runs.length === 0 ? (
          <p className="text-sm text-text-muted">아직 실행이 없습니다.</p>
        ) : (
          <>
            <RunGroup title="조치 필요" runs={groups.attention} onOpen={onSelectRun} empty="지금 확인할 실행이 없습니다." />
            <RunGroup title="진행 중" runs={groups.active} onOpen={onSelectRun} />
            {groups.finished.length > 0 && (
              <details className="mb-6">
                <summary className="mb-2 cursor-pointer text-sm font-medium uppercase tracking-wide text-text-muted">종료 {groups.finished.length}</summary>
                <ul className="space-y-1.5">
                  {groups.finished.map((r) => <RunRowItem key={r.id} run={r} onOpen={() => onSelectRun(r.id)} />)}
                </ul>
              </details>
            )}
          </>
        )}
      </div>
    </div>
  );
}
