import type { Task } from "./ipc";
import { LOCAL_HOST } from "./transport";

/**
 * 전체 재조회(`taskListAll`)가 돌아오는 사이에 도착한 로컬 상태 전이(`task://state`)를 기억해,
 * 재조회 결과가 그 전이를 되돌리지 못하게 하는 오버레이.
 *
 * 왜 필요한가 — 재조회는 호스트마다 병렬로 묻고 **가장 느린 호스트**까지 기다린 뒤 한 번에
 * `setTasks`한다. 로컬 스냅샷은 조회 시작 시점의 것이라, 원격 호스트가 응답하지 않아 10초
 * 타임아웃까지 걸리면 그 사이 끝난 대화 턴의 검토 대기 전이가 낡은 "실행 중"으로 덮인다.
 * 반대 방향(후속 메시지로 실행 중이 됐는데 재조회가 검토 대기로 되돌림)도 같은 경로다.
 *
 * 전이 이벤트는 백엔드가 DB에 쓴 **뒤에** 보낸다. 그래서 조회 시작보다 먼저 온 전이는 스냅샷에
 * 이미 들어 있고, 조회 시작 이후에 온 전이만 스냅샷보다 새롭다 — 후자만 덧씌우고 전자는 버린다.
 *
 * 버리는 쪽의 전제: **더 먼저 시작한 재조회가 더 늦게 끝나면 호출부가 그 결과를 통째로
 * 버린다**(`App.tsx`의 `refreshAppliedAtRef`). 그 전제가 없으면 늦게 온 옛 스냅샷이 여기서
 * 이미 지운 전이를 되돌린다 — 두 자리는 한 쌍이다.
 */
export interface StateOverride {
  state: string;
  awaiting_kind: string | null;
  /** 전이 이벤트를 받은 시각. `startedAt`과 같은 시계(단조 시계 권장)여야 한다. */
  at: number;
}

export type StateOverrides = Map<number, StateOverride>;

/** 로컬 작업의 상태 전이를 기록한다. 같은 작업의 늦은 전이가 이른 전이를 대체한다. */
export function noteStateOverride(
  overrides: StateOverrides,
  id: number,
  state: string,
  awaitingKind: string | null | undefined,
  at: number,
): void {
  overrides.set(id, { state, awaiting_kind: awaitingKind ?? null, at });
}

/**
 * `startedAt`에 시작한 재조회 결과에, 그 이후 도착한 전이를 덧씌운다.
 *
 * 입력 배열은 변형하지 않는다. `startedAt`보다 오래된 기록은 스냅샷이 이미 담고 있으므로
 * 지운다 — 남겨 두면 다음 재조회에서도 헛되이 비교만 한다.
 *
 * 예외는 `stale` 행이다. 로컬 조회가 실패하면 병합기가 마지막 성공 목록의 행을 대신 넣는데,
 * 그 상태는 조회 시작보다 훨씬 전의 것이라 시작 시각이 신선도의 근거가 되지 못한다. 그 행에는
 * 시각과 무관하게 덧씌우고 기록도 남긴다 — 다음 성공한 조회가 정리한다.
 */
export function applyStateOverrides(
  tasks: Task[],
  overrides: StateOverrides,
  startedAt: number,
): Task[] {
  if (overrides.size === 0) return tasks;
  const staleLocal = new Set<number>();
  for (const task of tasks) {
    if (task.host === LOCAL_HOST && task.stale) staleLocal.add(task.id);
  }
  for (const [id, override] of overrides) {
    if (override.at < startedAt && !staleLocal.has(id)) overrides.delete(id);
  }
  if (overrides.size === 0) return tasks;
  return tasks.map((task) => {
    if (task.host !== LOCAL_HOST) return task;
    const override = overrides.get(task.id);
    if (!override || task.archive) return task;
    if (task.state === override.state && (task.awaiting_kind ?? null) === override.awaiting_kind) {
      return task;
    }
    return { ...task, state: override.state, awaiting_kind: override.awaiting_kind };
  });
}

/** 보관 여부와 경합한 상태 이벤트는 DB에서 다시 확인한다. 이벤트에는 발생 순서가 없다. */
export function archivedStateRaces(tasks: Task[], overrides: StateOverrides, startedAt: number): number[] {
  return tasks.filter((task) => task.host === LOCAL_HOST && task.archive
    && overrides.has(task.id) && (task.stale || overrides.get(task.id)!.at >= startedAt))
    .map((task) => task.id);
}
