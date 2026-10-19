import { useEffect, useState } from "react";
import type { SideQuestionApi } from "./side-question";

/**
 * 이 세션에서 사이드 질문을 보낼 수 있는지 — 백엔드 스냅샷의 `supported`를 그대로 쓴다.
 *
 * 지원 여부는 제공자·정책·작업 상태를 모두 본 백엔드만 안다. 프론트에서 같은 규칙을 다시 세우면
 * 어긋난다. `revision`(작업 상태처럼 판단을 바꿀 수 있는 값)이 바뀌면 다시 읽되, 응답이 올 때까지
 * 같은 세션의 앞 값을 유지한다. 첫 응답 전과 조회 실패는 "못 쓴다"로 둔다.
 */
export function useSideQuestionSupported(
  sessionKey: string | null,
  revision: string,
  api: Pick<SideQuestionApi, "read"> | null,
): boolean {
  const [state, setState] = useState<{ key: string; supported: boolean } | null>(null);
  useEffect(() => {
    if (api == null || sessionKey == null) return;
    let live = true;
    Promise.resolve()
      .then(() => api.read())
      .then((snapshot) => { if (live) setState({ key: sessionKey, supported: snapshot.supported }); })
      .catch(() => { if (live) setState({ key: sessionKey, supported: false }); });
    return () => { live = false; };
  }, [api, sessionKey, revision]);
  return api != null && state?.key === sessionKey && state.supported;
}
