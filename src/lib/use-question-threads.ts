import { useCallback, useEffect, useState } from "react";
import type { SideQuestionApi } from "./side-question";

/**
 * "따로 질문"을 쓴 적이 있는 작업을 기억한다.
 *
 * 헤더 칩·코드 열 탭은 기록이 있는 작업에서만 보인다 — 처음 여는 길은 선택 버블과 메시지의
 * "따로 질문"이고, 한 번 연 뒤에는 돌아올 손잡이로 남는다. 패널은 열려 있을 때만 서버를 읽으므로,
 * 작업을 고를 때 한 번 읽어 앱 재시작 전의 기록도 찾는다. 조회 실패는 "없음"으로 둔다.
 */
export function useQuestionThreads(
  sessionKey: string | null,
  conversation: boolean,
  api: Pick<SideQuestionApi, "read"> | null,
): { has: boolean; mark: (key: string) => void } {
  const [threads, setThreads] = useState<ReadonlySet<string>>(() => new Set());
  const mark = useCallback((key: string) => {
    setThreads((current) => current.has(key) ? current : new Set(current).add(key));
  }, []);
  const known = sessionKey != null && threads.has(sessionKey);
  useEffect(() => {
    if (api == null || sessionKey == null || !conversation || known) return;
    let live = true;
    Promise.resolve()
      .then(() => api.read())
      .then((snapshot) => { if (live && snapshot.turns.length > 0) mark(sessionKey); })
      .catch(() => {});
    return () => { live = false; };
  }, [api, sessionKey, conversation, known, mark]);
  return { has: known, mark };
}
