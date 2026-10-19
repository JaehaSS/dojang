import type { ReactElement } from "react";
import { contextPercent, contextWindowFor } from "../../lib/context-window";
import type { ContextObservation } from "../../lib/context-observation";
import type { Task } from "../../lib/ipc";

interface Props {
  task: Task;
  observation: ContextObservation | null;
  /** 마지막 관측 실행 모델(model_snapshot) — `[1m]` 모델의 1M 윈도를 가르는 근거. */
  model?: string | null;
  busy?: boolean;
  onOpenDetails: () => void;
}

/** 잔량 임계 톤 — 20% 미만 경고, 50% 미만 주의. 평소엔 조용하고 임계에서만 눈에 띈다. */
export const gaugeTone = (remaining: number): string =>
  remaining < 20
    ? "text-status-failed"
    : remaining < 50
      ? "text-status-awaiting"
      : "text-text-muted";

/**
 * 컴포저 곁의 컨텍스트 잔량 게이지.
 *
 * 헤더가 아니라 여기 있는 이유는 컨텍스트를 소모하는 행위가 메시지 전송이기 때문이다 —
 * 잔량은 그 손끝에 있어야 한다(설계 0044). 사용률(`CTX 7%`)이 아니라 잔량(`93% 남음`)으로
 * 적는 이유도 같다. 낮은 숫자가 좋은 것인지 나쁜 것인지 읽는 사람이 헷갈리지 않아야 한다.
 */
export function ContextGauge({
  task,
  observation,
  model,
  busy = false,
  onOpenDetails,
}: Props): ReactElement | null {
  const window = observation?.contextWindow ?? contextWindowFor(task.agent, model);
  const used = contextPercent(observation?.contextTokens ?? null, task.agent, model, window);
  const remaining = used == null ? null : 100 - used;
  const observedAt =
    observation?.observedAt == null
      ? "기록 시각 없음"
      : new Date(observation.observedAt * 1000).toISOString();
  const label = remaining == null ? "컨텍스트 확인 불가" : `컨텍스트 약 ${remaining}% 남음${busy ? " (이전 관측)" : ""}`;
  return (
    <button
      type="button"
      className="shrink-0 self-end rounded px-1.5 pb-0.5 text-xs hover:bg-raised focus-visible:outline-primary"
      onClick={onOpenDetails}
      title={observation ?
        `${label} · 마지막 관측: ${observedAt} ` +
        `(${observation.contextTokens.toLocaleString()}/${window.toLocaleString()} 토큰)`
        : "채널에서 확보된 입력과 실행 기록의 구성 보기"}
      aria-label={`${label} · 컨텍스트 구성 보기`}
    >
      <span className={remaining == null ? "text-text-muted" : gaugeTone(remaining)}>{label}</span>
    </button>
  );
}
