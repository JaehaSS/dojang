import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { PIPELINE_CHANGED_EVENT, pipelineList, type RunRow } from "../../../lib/pipeline";
import { ErrorLine, Section, StateBadge } from "./parts";
import { PipelineCreate } from "./PipelineCreate";
import { RunDetail } from "./RunDetail";

interface Props {
  repo: string;
  onOpenTask: (taskId: number) => void;
  onApproveIntegration: (taskId: number) => Promise<void> | void;
}

/** 멀티벤더 파이프라인 — 실행 목록과 생성 폼, 선택한 실행의 보드. */
export function PipelineView({ repo, onOpenTask, onApproveIntegration }: Props) {
  const [runs, setRuns] = useState<RunRow[]>([]);
  const [runId, setRunId] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    pipelineList(repo || undefined).then(setRuns, (e: unknown) => setError(String(e)));
  }, [repo]);

  useEffect(() => {
    load();
    const un = listen(PIPELINE_CHANGED_EVENT, () => load());
    return () => {
      void un.then((f) => f());
    };
  }, [load]);

  return (
    <div className="flex h-full min-h-0 flex-col overflow-auto p-6">
      <div className="mx-auto w-full max-w-4xl">
        {runId != null ? (
          <RunDetail runId={runId} onBack={() => { setRunId(null); load(); }} onOpenTask={onOpenTask} onApproveIntegration={onApproveIntegration} />
        ) : (
          <>
            <h1 className="mb-4 text-lg font-semibold">멀티벤더 파이프라인</h1>
            <ErrorLine message={error} />
            <PipelineCreate repo={repo} onStarted={(run) => { load(); setRunId(run.id); }} />
            <Section title="실행 목록">
              {runs.length === 0 ? (
                <p className="text-sm text-text-muted">아직 실행이 없습니다.</p>
              ) : (
                <ul className="space-y-1">
                  {runs.map((r) => (
                    <li key={r.id} className="flex items-center gap-2 text-sm">
                      <StateBadge kind="run" state={r.state} />
                      <button type="button" className="min-w-0 flex-1 truncate text-left hover:text-text" onClick={() => setRunId(r.id)}>
                        #{r.id} {r.goal}
                      </button>
                      <span className="text-xs text-text-muted">{r.base_branch}</span>
                    </li>
                  ))}
                </ul>
              )}
            </Section>
          </>
        )}
      </div>
    </div>
  );
}
