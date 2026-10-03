import { useSyncExternalStore } from "react";

/**
 * 고급·실험 기능 표시 — 설정 › 모양새의 스위치 하나로 여러 화면의 개발자용·실험 기능
 * 노출을 한 번에 켜고 끈다(캐시 관측치, 앙상블 실험 지표, Design Mode 등).
 *
 * DB가 아니라 localStorage인 이유는 이것이 순수 UI 취향이지 공유할 도메인 사실이
 * 아니기 때문이다(`diff-mode.ts`와 같은 방침). 기본은 OFF다 — 처음 쓰는
 * 사람에게 실험 지표가 먼저 보이면 이 앱이 뭘 하는 도구인지 헷갈린다.
 *
 * 같은 창 안에 이 값을 보는 화면이 여럿(설정·인사이트·앙상블·입력창…) 동시에 떠 있을 수
 * 있어, 모듈 전역 리스너로 값을 퍼뜨린다 — 설정에서 끄면 열려 있는 다른 탭도 즉시 접힌다.
 */
const STORAGE_KEY = "praxis:experimental-features";

const listeners = new Set<() => void>();

function readStored(): boolean {
  try {
    return window.localStorage.getItem(STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

let enabled = readStored();

/** 훅을 쓸 수 없는 곳(일반 함수·이벤트 핸들러)에서 현재 값을 그냥 읽을 때. */
export function experimentalFeaturesEnabled(): boolean {
  return enabled;
}

export function setExperimentalFeaturesEnabled(next: boolean): void {
  enabled = next;
  try {
    window.localStorage.setItem(STORAGE_KEY, next ? "1" : "0");
  } catch {
    // private mode 등 storage 거부 시 현재 세션 상태만 유지한다(praxis:diff-viewed와 같은 방침).
  }
  listeners.forEach((cb) => cb());
}

function subscribe(cb: () => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

/**
 * 화면이 이 값을 구독한다 — 다른 화면에서 바꿔도 리렌더로 즉시 따라온다.
 * 세 번째 인자(getServerSnapshot)는 실제 SSR용이 아니라, InsightsView처럼
 * `renderToStaticMarkup`으로 스냅숏 테스트를 하는 경로에서 React가 요구하기 때문이다 —
 * 이 앱에 서버 렌더는 없으므로 클라이언트 스냅숏과 같은 값을 그대로 준다.
 */
export function useExperimentalFeatures(): boolean {
  return useSyncExternalStore(subscribe, experimentalFeaturesEnabled, experimentalFeaturesEnabled);
}

/** 테스트 전용 — 모듈 상태(및 구독자)를 리셋한다. */
export function resetExperimentalFeaturesForTest(next = false): void {
  enabled = next;
  try {
    window.localStorage.setItem(STORAGE_KEY, next ? "1" : "0");
  } catch {
    /* ignore */
  }
}
