import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { interactionPendingTasks } from "./conversation-interaction";

const EMPTY: readonly number[] = [];

/**
 * 답변을 기다리는 질문이 열린 로컬 작업 id 목록.
 *
 * 구조화 질문은 `tasks.state`를 바꾸지 않으므로 사이드바가 `tasks`만 읽어서는 드러나지 않는다.
 * 질문이 열리고 닫힐 때는 `convo-interaction://changed`가, 턴이 끝나 질문이 함께 닫힐 때는
 * `task://state`가 오므로 둘 다 듣고 다시 읽는다. 늦게 돌아온 응답이 새 응답을 덮지 않도록
 * 요청 번호로 가른다.
 */
export function usePendingQuestions(): readonly number[] {
  const [ids, setIds] = useState<readonly number[]>(EMPTY);
  const request = useRef(0);
  useEffect(() => {
    let active = true;
    const stops: (() => void)[] = [];
    const load = async () => {
      const current = ++request.current;
      try {
        const next = await interactionPendingTasks();
        if (!active || current !== request.current) return;
        setIds((prev) =>
          prev.length === next.length && prev.every((id, i) => id === next[i]) ? prev : next,
        );
      } catch {
        // 조회 실패는 표시를 비우지 않는다 — 다음 이벤트에서 다시 읽는다.
      }
    };
    void load();
    for (const event of ["convo-interaction://changed", "task://state"]) {
      void Promise.resolve()
        .then(() => listen(event, () => void load()))
        .then((unlisten) => {
          if (!active) return unlisten();
          stops.push(unlisten);
        })
        .catch(() => {});
    }
    return () => {
      active = false;
      request.current += 1;
      for (const stop of stops) void Promise.resolve(stop()).catch(() => {});
    };
  }, []);
  return ids;
}
