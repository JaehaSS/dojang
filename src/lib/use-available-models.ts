import { useEffect, useState } from "react";
import { agentAvailableModels } from "./ipc";
import type { ModelOption } from "./models";

/** 메뉴를 열 때 재조회한다. 백엔드는 5분 캐시와 동시 요청 합치기를 담당한다. */
export function useAvailableModels(agent: string, enabled = true): ModelOption[] | null {
  const [snapshot, setSnapshot] = useState<{ agent: string; models: ModelOption[] } | null>(null);
  useEffect(() => {
    if (!enabled || !["agy", "gemini", "antigravity"].includes(agent)) return;
    let alive = true;
    agentAvailableModels(agent)
      .then((models) => {
        if (alive) setSnapshot({ agent, models });
      })
      .catch(() => {
        // 조회 실패 시 마지막 목록 또는 내장 목록을 유지한다.
      });
    return () => { alive = false; };
  }, [agent, enabled]);
  return snapshot?.agent === agent ? snapshot.models : null;
}
