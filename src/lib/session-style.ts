import { useSyncExternalStore } from "react";

/**
 * 세션 방식 — 새 작업을 대화(턴마다 `-p`, 앱 컴포저로 입력)로 만들지, 터미널(실제 CLI를 PTY로,
 * 터미널 안에서만 입력)로 만들지 정하는 앱 전역 설정(설정 › 작업 실행).
 *
 * 작업마다 고르던 때는 터미널 작업에도 앱 컴포저가 함께 떠 입력창이 둘로 보였다. 방식은 여기서만
 * 정하고, 작업 화면은 각 작업에 기록된 `mode`를 보고 컴포저를 그릴지 정한다 — 이 값을 바꿔도
 * 이미 떠 있는 세션의 입력 경로는 바뀌지 않는다.
 *
 * localStorage인 이유는 `experimental-features.ts`와 같다 — 공유할 도메인 사실이 아니라 UI 취향이다.
 * 기본은 대화다(기존 기본 동작).
 */
export type SessionStyle = "conversation" | "terminal";

const STORAGE_KEY = "praxis:session-style";

const listeners = new Set<() => void>();

function readStored(): SessionStyle {
  try {
    return window.localStorage.getItem(STORAGE_KEY) === "terminal" ? "terminal" : "conversation";
  } catch {
    return "conversation";
  }
}

let current = readStored();

export function sessionStyle(): SessionStyle {
  return current;
}

export function setSessionStyle(next: SessionStyle): void {
  current = next;
  try {
    window.localStorage.setItem(STORAGE_KEY, next);
  } catch {
    // storage 거부 시 현재 창 상태만 유지한다(experimental-features와 같은 방침).
  }
  listeners.forEach((cb) => cb());
}

function subscribe(cb: () => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

export function useSessionStyle(): SessionStyle {
  return useSyncExternalStore(subscribe, sessionStyle, sessionStyle);
}

/** 테스트 전용 — 모듈 상태를 리셋한다. */
export function resetSessionStyleForTest(next: SessionStyle = "conversation"): void {
  current = next;
  try {
    window.localStorage.removeItem(STORAGE_KEY);
  } catch {
    /* 무시 */
  }
  listeners.forEach((cb) => cb());
}
