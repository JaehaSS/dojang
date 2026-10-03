import type { Task } from "../lib/ipc";
import type { RunnerHealth } from "../lib/transport/runner";

// 상태 판독 로직 — 순수 함수만. (설계 0013 §7.2)
// 모바일의 1순위 요구는 "Runner가 살아 있는지, 일하고 있는지"를 오해 없이 보여주는 것이다.

export type Tone = "running" | "awaiting" | "question" | "done" | "failed" | "muted";

/**
 * DB state 문자열 → 표시 라벨·색. 알 수 없는 값은 원문을 그대로 보여준다(추측 금지).
 *
 * `awaitingKind`는 검토 대기의 성격(`tasks.awaiting_kind`) — 답을 기다리는 중인지
 * 결과 승인을 기다리는 중인지 가른다. 폰에서는 이 구분이 특히 중요하다: 대답은
 * 폰으로도 되지만 diff 검토는 대개 데스크톱으로 미루게 된다.
 */
export function taskStateLabel(
  state: string,
  awaitingKind?: string | null,
): { label: string; tone: Tone } {
  if (state === "AwaitingReview" && awaitingKind === "question") {
    return { label: "답변 대기", tone: "question" };
  }
  switch (state) {
    case "Created":
      return { label: "생성됨", tone: "muted" };
    case "Queued":
      return { label: "대기열", tone: "muted" };
    case "PendingApproval":
      return { label: "실행 승인 대기", tone: "awaiting" };
    case "Starting":
      return { label: "시작 중", tone: "running" };
    case "Running":
      return { label: "진행 중", tone: "running" };
    case "AwaitingReview":
      return { label: "검토 대기", tone: "awaiting" };
    case "Finalizing":
      return { label: "마무리 중", tone: "running" };
    case "Done":
      return { label: "완료", tone: "done" };
    case "Discarded":
      return { label: "버림", tone: "muted" };
    case "Failed":
      return { label: "실패", tone: "failed" };
    default:
      return { label: state, tone: "muted" };
  }
}

/**
 * 목록 정렬 순위. 폰은 한 번에 몇 줄 못 보므로 **내 행동이 필요한 것**이 위로 와야 한다.
 * 검토 대기 > 실행 승인 대기 > 진행 중 > 대기 > 종료.
 */
const ORDER: Record<string, number> = {
  AwaitingReview: 0,
  PendingApproval: 1,
  Running: 2,
  Starting: 2,
  Finalizing: 2,
  Queued: 3,
  Created: 3,
  Failed: 4,
  Done: 5,
  Discarded: 6,
};

function rank(state: string): number {
  return ORDER[state] ?? 4;
}

/** 행동이 필요한 순 → 최근 갱신 순으로 정렬한다. 원본 배열은 건드리지 않는다. */
export function sortForMobile(tasks: Task[]): Task[] {
  return [...tasks].sort((a, b) => {
    const byRank = rank(a.state) - rank(b.state);
    return byRank !== 0 ? byRank : b.updated_at - a.updated_at;
  });
}

/** 내 결정을 기다리는 작업 수 — 헤더 배지에 쓴다. */
export function actionableCount(tasks: Task[]): number {
  return tasks.filter((task) => rank(task.state) <= 1).length;
}

export type ConnectionState =
  | { kind: "ok"; health: RunnerHealth }
  /** tailnet에 없거나 DNS/TLS 실패 — 폰 쪽 문제. */
  | { kind: "offline" }
  /** 프록시는 살아있고 Runner만 죽음. 이 구분이 이 앱의 핵심 신호다. */
  | { kind: "runner-down"; status: number }
  /** 세션 만료·회수. */
  | { kind: "unauthorized" }
  | { kind: "error"; status: number };

/**
 * HTTP 응답을 원인별로 나눈다. "연결 안 됨" 하나로 뭉치면 폰에서 진단이 불가능하다.
 * 502/503/504는 tailscale serve가 살아 있는데 백엔드로 못 붙은 것 = Runner 다운.
 */
export function classifyStatus(status: number): ConnectionState["kind"] {
  if (status === 401) return "unauthorized";
  if (status === 502 || status === 503 || status === 504) return "runner-down";
  return "error";
}

export interface BannerView {
  tone: Tone;
  title: string;
  /** 보조 설명. 없으면 제목만 보여준다. */
  detail?: string;
  /** 사용자가 취할 수 있는 행동이 있으면 true — 재시도 버튼을 노출한다. */
  retryable: boolean;
}

/** 신호 나이 임계 — 이보다 오래되면 "조용함"으로 표시해 의심할 근거를 준다. */
export const QUIET_AFTER_SECS = 30 * 60;

/**
 * 연결 상태 + health를 배너 한 줄로 요약한다.
 * `nowSecs`를 인자로 받는 이유는 테스트에서 시간을 고정하기 위해서다.
 */
export function describeConnection(state: ConnectionState, nowSecs: number): BannerView {
  switch (state.kind) {
    case "offline":
      return {
        tone: "failed",
        title: "Runner에 닿지 못했습니다",
        detail: "폰이 tailnet에 연결되어 있는지 확인하세요 (Tailscale 앱).",
        retryable: true,
      };
    case "runner-down":
      return {
        tone: "failed",
        title: "Runner가 응답하지 않습니다",
        detail: `프록시는 살아 있지만 Runner가 내려가 있습니다 (${state.status}).`,
        retryable: true,
      };
    case "unauthorized":
      return {
        tone: "awaiting",
        title: "세션이 만료되었습니다",
        detail: "데스크톱 Dojang 설정에서 QR을 다시 스캔하세요.",
        retryable: true,
      };
    case "error":
      return {
        tone: "failed",
        title: `요청이 실패했습니다 (${state.status})`,
        retryable: true,
      };
    case "ok": {
      const last = state.health.last_event_at;
      if (last == null) {
        return { tone: "done", title: "연결됨", detail: "아직 기록된 활동이 없습니다.", retryable: false };
      }
      const age = Math.max(0, nowSecs - last);
      const quiet = age >= QUIET_AFTER_SECS;
      return {
        tone: quiet ? "muted" : "done",
        title: "연결됨",
        detail: `마지막 신호 ${formatAge(age)} 전`,
        retryable: false,
      };
    }
  }
}

/** "45초"/"12분"/"3시간"/"2일" — 배너 한 줄에 들어가는 짧은 한국어 표기. */
export function formatAge(seconds: number): string {
  if (seconds < 60) return `${Math.floor(seconds)}초`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}분`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}시간`;
  return `${Math.floor(seconds / 86400)}일`;
}
