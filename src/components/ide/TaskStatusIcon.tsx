import type { ReactElement } from "react";
import type { StatusTone } from "../../lib/task-status";

/**
 * 작업 상태 마크 — 색과 **모양**을 함께 바꾼다. 같은 크기의 점 다섯 개는 색으로만 갈렸고,
 * 색각 이상 사용자에게는 실행 중과 완료가 같은 점이었다(DESIGN.md Do #2).
 *
 * 실행 중은 열린 고리가 돈다 — 움직이는 것은 이것 하나라 "지금 일하는 중"이 목록에서 먼저 읽힌다.
 * `prefers-reduced-motion`이면 멈춘 고리로 남는다. 완전한 문구는 호출부가 `label`로 싣는다.
 */
export function TaskStatusIcon({
  tone,
  label,
  size = 14,
  decorative = false,
}: {
  tone: StatusTone;
  label: string;
  size?: number;
  /** 옆에 같은 문구가 보이면 켠다 — 스크린 리더가 상태를 두 번 읽지 않게 이름을 떼고 숨긴다. */
  decorative?: boolean;
}): ReactElement {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={`shrink-0 ${TONE_CLASS[tone]} ${tone === "running" ? "animate-spin motion-reduce:animate-none" : ""}`}
      role={decorative ? undefined : "img"}
      aria-label={decorative ? undefined : `작업 상태: ${label}`}
      aria-hidden={decorative || undefined}
      data-tone={tone}
    >
      {!decorative && <title>{label}</title>}
      {SHAPES[tone]}
    </svg>
  );
}

const TONE_CLASS: Record<StatusTone, string> = {
  running: "text-status-running",
  awaiting: "text-status-awaiting",
  question: "text-status-question",
  done: "text-status-done",
  failed: "text-status-failed",
  muted: "text-text-muted",
};

const SHAPES: Record<StatusTone, ReactElement> = {
  // 3/4만 그린 고리 — 회전해야 진행으로 읽힌다.
  running: <circle cx="12" cy="12" r="8" strokeDasharray="38 13" />,
  // 검토 대기: 결과를 "보러" 가야 한다.
  awaiting: (
    <>
      <path d="M2.5 12s3.5-6.5 9.5-6.5 9.5 6.5 9.5 6.5-3.5 6.5-9.5 6.5S2.5 12 2.5 12z" />
      <circle cx="12" cy="12" r="2.5" />
    </>
  ),
  // 답변 대기: 에이전트가 묻고 있다.
  question: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M9.6 9.4a2.5 2.5 0 1 1 3.4 2.4c-.6.3-1 .8-1 1.4v.4" />
      <path d="M12 16.8h.01" />
    </>
  ),
  done: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="m8.5 12 2.5 2.5 4.5-5" />
    </>
  ),
  failed: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="m9.2 9.2 5.6 5.6M14.8 9.2l-5.6 5.6" />
    </>
  ),
  // 대기·생성됨: 아직 시작하지 않았다 — 비어 있는 점선 원.
  muted: <circle cx="12" cy="12" r="8" strokeDasharray="2.5 3.5" />,
};
