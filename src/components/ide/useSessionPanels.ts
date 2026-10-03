import { useCallback, useLayoutEffect, useRef, useState } from "react";
import type { CodeTab } from "./CodeColumnTabs";

/** 패널 상태를 붙들고 있을 세션 수 상한. 넘으면 가장 오래 손대지 않은 것부터 버린다. */
const LIMIT = 50;

/** 세션마다 따로 사는 패널 열림 상태(ADR 0163 결정 1). 치수(도크 높이·열 폭)는 여기 없다 —
 *  그건 취향이지 작업 상태가 아니라 앱 전역으로 남는다. */
export interface PanelState {
  /** 에디터 아래 워크스페이스 셸 도크(⌃`). */
  terminalDock: boolean;
  /** 코드 열(파일·프리뷰·작업정보)이 자리를 받는지(⌥⌘S). */
  codeOpen: boolean;
  /** 코드 열의 활성 탭. */
  codeTab: CodeTab;
  /** 세션 본문에 표시 중인 변경 파일. */
  centralDiffPath: string | null;
  /** 파일 트리(⌘B). */
  showTree: boolean;
  /** 플로팅 작업정보 채널의 핀. */
  channelPinned: boolean;
}

type Updater<T> = T | ((prev: T) => T);

export interface SessionPanels extends PanelState {
  /**
   * 지금 렌더된 값이 속한 세션. 전환 직후 한 렌더 동안은 옛 세션의 것이다 — 복원이 layout
   * effect에서 일어나기 때문이다. "세션에 도착한 순간"을 보는 쪽은 인자가 아니라 이것을 본다.
   */
  stateKey: string | null;
  setTerminalDock: (next: Updater<boolean>) => void;
  setCodeOpen: (next: Updater<boolean>) => void;
  setCodeTab: (next: Updater<CodeTab>) => void;
  setCentralDiffPath: (path: string | null) => void;
  openDiffPanel: () => void;
  toggleDiffPanel: () => void;
  setShowTree: (next: Updater<boolean>) => void;
  setChannelPinned: (next: Updater<boolean>) => void;
}

/**
 * seed — 아직 아무 세션에도 항목이 없을 때의 시작값. **이번 실행 내내 고정이다.**
 *
 * "마지막으로 바꾼 값"을 추종하게 두면 A에서 터미널을 연 직후 처음 가는 B도 터미널이 열려
 * 있다. 그것이 바로 이 훅이 없애려는 증상이다(ADR 0163 결정 2).
 *
 * 모든 세션은 코드 열을 닫고 플로팅 채널을 핀 상태로 시작한다. 코드 열을 연 선택은 현재
 * 세션에만 남아, 처음 여는 다른 세션의 시작 모습을 바꾸지 않는다.
 */
const seedState = (): PanelState => ({
  terminalDock: false,
  codeOpen: false,
  codeTab: "file",
  centralDiffPath: null,
  showTree: false,
  channelPinned: true,
});

/**
 * 세션별로 패널 열림 상태를 기억한다.
 *
 * 프리뷰 웹뷰도 셸 PTY도 이미 `task_id`를 키로 갈려 살고 있었다. 세션을 넘어 따라오던 것은
 * 자원이 아니라 "열려 있다"는 비트뿐이었고, 그 탓에 프리뷰·터미널을 쓰지 않는 세션에도 두 면이
 * 서 있었다. 터미널 쪽은 화면 문제로 끝나지 않는다 — `ShellTerminal`은 마운트하면 무조건
 * `shell_open`을 부르고, 그 커맨드는 맵에 없으면 새 셸을 띄운다.
 *
 * 키는 `taskKey`(host:id) 좌표다. 로컬 3번과 원격 3번은 서로 다른 세션이라 숫자 하나로 묶으면
 * 서로의 화면을 덮는다(ADR 0133 결정 4).
 *
 * 메모리에만 둔다 — 앱을 껐다 켜면 seed에서 다시 시작한다. 컴포저 초안(`useSessionDraft`)과
 * 같은 규칙을 쓰는 이유는 설명 비용이다. 세션 상태의 수명이 두 종류면 어느 것이 남는지를
 * 사용자가 외워야 한다.
 */
export function useSessionPanels(sessionKey: string | null): SessionPanels {
  const seedRef = useRef<PanelState | null>(null);
  if (seedRef.current == null) seedRef.current = seedState();

  const store = useRef(new Map<string, PanelState>());
  const [state, setState] = useState<PanelState>(seedRef.current);
  const [stateKey, setStateKey] = useState<string | null>(sessionKey);
  /** 최신 상태 — setter가 마운트 내내 같은 identity를 유지해야 해서 렌더 값을 못 읽는다. */
  const stateRef = useRef<PanelState>(state);
  /** 같은 이유로 세션 키도 ref로 읽는다. 단축키 핸들러가 마운트 시 한 번만 붙는다. */
  const keyRef = useRef<string | null>(sessionKey);

  // 전환과 같은 프레임에 되돌린다. useEffect로 미루면 새 세션 화면에 옛 세션의 패널이 한 번
  // 그려졌다가 접힌다.
  useLayoutEffect(() => {
    keyRef.current = sessionKey;
    // 아직 손대지 않은 세션은 맵에 넣지 않는다 — seed가 고정이라 넣으나 마나 같은 값이고,
    // 세션을 훑기만 해도 상한이 차는 것을 막는다.
    const restored =
      (sessionKey == null ? undefined : store.current.get(sessionKey)) ?? seedRef.current!;
    stateRef.current = restored;
    setState(restored);
    setStateKey(sessionKey);
  }, [sessionKey]);

  const apply = useCallback(
    <K extends keyof PanelState>(field: K, next: Updater<PanelState[K]>): void => {
      const prev = stateRef.current[field];
      const value =
        typeof next === "function"
          ? (next as (p: PanelState[K]) => PanelState[K])(prev)
          : next;
      if (value === prev) return;

      const merged: PanelState = { ...stateRef.current, [field]: value };
      stateRef.current = merged;
      setState(merged);

      const key = keyRef.current;
      if (key == null) return;
      const map = store.current;
      map.delete(key); // 재삽입으로 최근 사용 순서를 만든다
      map.set(key, merged);
      while (map.size > LIMIT) {
        const oldest = map.keys().next().value;
        if (oldest === undefined) break;
        map.delete(oldest);
      }
    },
    [],
  );

  const setTerminalDock = useCallback((n: Updater<boolean>) => apply("terminalDock", n), [apply]);
  const setCodeOpen = useCallback(
    (n: Updater<boolean>) => {
      const next = typeof n === "function" ? n(stateRef.current.codeOpen) : n;
      apply("codeOpen", next);
      if (!next) apply("centralDiffPath", null);
    },
    [apply],
  );
  const setCodeTab = useCallback(
    (n: Updater<CodeTab>) => {
      const next = typeof n === "function" ? n(stateRef.current.codeTab) : n;
      apply("codeTab", next);
      if (next !== "diff") apply("centralDiffPath", null);
    },
    [apply],
  );
  const setCentralDiffPath = useCallback((path: string | null) => apply("centralDiffPath", path), [apply]);
  const openDiffPanel = useCallback(() => {
    apply("codeOpen", true);
    apply("codeTab", "diff");
  }, [apply]);
  const toggleDiffPanel = useCallback(() => {
    const { codeOpen, codeTab } = stateRef.current;
    if (codeOpen && codeTab === "diff") {
      apply("codeOpen", false);
      apply("centralDiffPath", null);
      return;
    }
    apply("codeOpen", true);
    apply("codeTab", "diff");
  }, [apply]);
  const setShowTree = useCallback((n: Updater<boolean>) => apply("showTree", n), [apply]);
  const setChannelPinned = useCallback(
    (n: Updater<boolean>) => apply("channelPinned", n),
    [apply],
  );

  return {
    ...state,
    stateKey,
    setTerminalDock,
    setCodeOpen,
    setCodeTab,
    setCentralDiffPath,
    openDiffPanel,
    toggleDiffPanel,
    setShowTree,
    setChannelPinned,
  };
}
