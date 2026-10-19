/**
 * 관측된 실행 모델 스냅샷 — `model_snapshot` 이벤트를 누적한 것.
 *
 * 이벤트는 두 필드를 **따로** 싣는다. `invocation`은 우리가 `--model`로 보낸 값(`requested`)만,
 * `claude_stream`·`codex_session`은 CLI가 실제로 잡은 값(`resolved`)만 준다. 그래서 마지막
 * 이벤트 하나만 보면 안 되고 순서대로 접어야 한다 — 백엔드도 같은 병합을 한다
 * (`ensemble/metrics.rs`, `ensemble/feedback/storage.rs`).
 */
export interface ModelSnapshot {
  /** CLI가 실제로 잡은 모델. 사용자에게 "지금 이 모델로 돈다"고 말할 수 있는 유일한 값. */
  resolved: string | null;
  /** 우리가 요청한 값. 관측이 아직 없을 때 컨텍스트 윈도를 추정하는 폴백으로만 쓴다. */
  requested: string | null;
}

export const EMPTY_MODEL_SNAPSHOT: ModelSnapshot = { resolved: null, requested: null };

/** 병합 대상 — `ConvoEvent`의 `model_snapshot` 변종이 만족하는 최소 형태. */
interface ModelSnapshotEvent {
  kind: string;
  resolved?: string | null;
  requested?: string | null;
  speaker?: unknown;
  parent_id?: unknown;
}

/**
 * 이벤트 열을 이전 스냅샷 위에 접는다.
 *
 * 부재 필드가 이전 값을 지우지 않는 것이 핵심이다. `invocation`(requested만) 뒤에
 * `claude_stream`(resolved만)이 오는 정상 턴에서, 덮어쓰기로 처리하면 둘 중 하나가 항상 사라진다.
 *
 * 새 세션을 여는 자리에서는 `EMPTY_MODEL_SNAPSHOT`부터 접어 이전 세션의 값이 새지 않게 한다.
 */
export function foldModelSnapshot(
  prev: ModelSnapshot,
  events: readonly ModelSnapshotEvent[],
): ModelSnapshot {
  return events.reduce<ModelSnapshot>(
    (acc, ev) => {
      if (ev.parent_id != null || (ev.speaker != null && ev.speaker !== "left")) return acc;
      if (ev.kind === "context_cleared") return EMPTY_MODEL_SNAPSHOT;
      return ev.kind === "model_snapshot"
        ? { resolved: ev.resolved ?? acc.resolved, requested: ev.requested ?? acc.requested }
        : acc;
    },
    prev,
  );
}
