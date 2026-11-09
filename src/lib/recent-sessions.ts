import type { Task } from "./ipc";
import { isTaskArchived } from "./task-archive";

/**
 * 목록에서 감추는 종료 상태. 사이드바 트리(`SessionTaskNavigation`)와 최근 세션이 같은 집합을
 * 봐야 한다 — 두 벌로 두면 상태가 하나 늘 때 한쪽만 고쳐져 같은 작업이 한쪽에만 남는다.
 */
export const HIDDEN_STATES = new Set(["Done", "Discarded"]);

/** "최근"의 창 — 1시간. 설정으로 빼지 않는다(discovery brief 결정 3). */
export const RECENT_SESSION_WINDOW_SEC = 60 * 60;

/**
 * 지난 `windowSec` 안에 갱신된 비종료 작업을 최신순으로. `nowSec`·`updated_at`은 모두 **초 단위**
 * epoch다(`fmt.ts`의 `ago()`가 `Date.now()/1000 - sec`로 쓰는 것과 같은 단위).
 *
 * 전용 "마지막 대화 시각" 필드는 없고 `updated_at`을 대신 쓴다 — 대화 모드에서는 메시지 전송과
 * 응답 완료가 모두 상태 전환이라 사실상 마지막 턴 경계 시각이다(brief "확인할 사항").
 *
 * 입력 배열은 변형하지 않는다. 호출부가 목록 상태를 그대로 넘기기 때문이다.
 */
export function recentSessions(
  tasks: Task[],
  nowSec: number,
  windowSec = RECENT_SESSION_WINDOW_SEC,
): Task[] {
  return tasks
    .filter((task) => !isTaskArchived(task) && !HIDDEN_STATES.has(task.state) && nowSec - task.updated_at <= windowSec)
    // 같은 시각이면 id 내림차순 — 목록 갱신마다 순서가 흔들리지 않게 전순서로 못 박는다.
    .sort((left, right) => right.updated_at - left.updated_at || right.id - left.id);
}
