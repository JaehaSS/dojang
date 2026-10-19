import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { PIPELINE_CHANGED_EVENT, pipelineList, type RunListItem } from "./pipeline";

export interface PipelineRuns {
  runs: RunListItem[];
  error: string | null;
  reload: () => void;
}

/** 모든 파이프라인 실행을 불러오고 `pipeline://changed`마다 다시 읽는다. 저장소로 거르지 않는다.
 * App이 한 번 호출해 사이드바 배지와 파이프라인 화면이 같은 목록을 쓴다. */
export function usePipelineRuns(): PipelineRuns {
  const [runs, setRuns] = useState<RunListItem[]>([]);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(() => {
    pipelineList().then(
      (r) => {
        setRuns(Array.isArray(r) ? r : []);
        setError(null);
      },
      (e: unknown) => setError(String(e)),
    );
  }, []);

  useEffect(() => {
    reload();
    let un: Promise<() => void> | null = null;
    try {
      un = listen(PIPELINE_CHANGED_EVENT, () => reload());
    } catch {
      un = null;
    }
    return () => {
      void un?.then((f) => f(), () => undefined);
    };
  }, [reload]);

  return { runs, error, reload };
}
