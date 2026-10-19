import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type { TaskRef } from "../lib/transport";
import type { FileDiff } from "../lib/ipc";
import {
  clearViewed,
  isViewed,
  loadViewed,
  markViewed,
  saveViewed,
  type ViewedState,
} from "../lib/diff-viewed";
import { useDiffReviewActions } from "./use-diff-review-actions";
import { useDiffViewerData, type DiffViewerData } from "./use-diff-viewer-data";
import { usePartialApply, type PartialApplyState } from "./use-partial-apply";
import { navigationFrom, useReviewEvents, type ReviewNavigation } from "./use-review-events";

export type OpenDiff = (path: string, opts?: { preview?: boolean }) => void;

/** 에이전트 이동 목표가 소비되지 않고 살아 있는 한도. */
export const NAVIGATION_TTL_MS = 15_000;

export interface DiffSessionValue {
  data: DiffViewerData;
  viewed: ViewedState;
  toggleViewed: (path: string) => void;
  /** 부분 적용은 선택 밖 hunk를 **모든 파일에서** 되돌린다 — 그래서 세션 값이다(ADR 0175 결정 8). */
  partial: PartialApplyState;
  actions: ReturnType<typeof useDiffReviewActions>;
  activeDiffPath: string | null;
  setActiveDiffPath: (path: string | null) => void;
  openDiff: OpenDiff;
  /** 폴링 구독 — 반환된 해제 함수를 부를 때까지 타이머가 돈다(설계 DR-7). */
  retain: () => () => void;
  /** 에이전트의 `review_navigate`가 남긴 이동 목표. 그 파일의 DiffTab이 스크롤한 뒤 지운다. */
  navigation: ReviewNavigation | null;
  consumeNavigation: () => void;
}

const DiffSession = createContext<DiffSessionValue | null>(null);

export function useDiffSession(): DiffSessionValue {
  const value = useContext(DiffSession);
  if (!value) throw new Error("useDiffSession은 DiffSessionProvider 안에서만 쓸 수 있다.");
  return value;
}

/** 구독자가 하나도 없으면 폴링을 멈춘다. 구독자는 변경 목록과 열린 diff 탭이다. */
function useSubscriberGate(): { enabled: boolean; retain: () => () => void } {
  const [count, setCount] = useState(0);
  const retain = useCallback(() => {
    setCount((n) => n + 1);
    return () => setCount((n) => n - 1);
  }, []);
  return { enabled: count > 0, retain };
}

/** 확인함은 호스트별로 갈라야 한다 — 같은 id의 다른 작업과 섞이면 안 된다. */
function useViewedFiles(taskId: number, files: FileDiff[] | null) {
  const [viewed, setViewed] = useState<ViewedState>(() => loadViewed(taskId));
  const filesRef = useRef(files);
  filesRef.current = files;
  useEffect(() => setViewed(loadViewed(taskId)), [taskId]);

  const toggleViewed = useCallback(
    (path: string) => {
      const file = filesRef.current?.find((f) => f.path === path);
      if (!file) return;
      setViewed((current) => {
        const next = isViewed(current, path, file.patch)
          ? clearViewed(current, path)
          : markViewed(current, path, file.patch);
        saveViewed(taskId, next);
        return next;
      });
    },
    [taskId],
  );
  return { viewed, toggleViewed };
}

/**
 * 세션당 하나뿐인 diff 상태. 변경 목록과 diff 탭들이 같은 스냅샷·같은 범위·같은 선택을 본다.
 *
 * 훅이 둘이면 폴링이 두 배이고, 범위가 어긋나는 순간 `hunk_id`가 달라져 붙여둔 주석이
 * 한꺼번에 고아로 보인다(설계 DR-7).
 */
export function DiffSessionProvider({
  task,
  openDiff,
  children,
}: {
  task: TaskRef;
  openDiff: OpenDiff;
  children: ReactNode;
}) {
  const { enabled, retain } = useSubscriberGate();
  const data = useDiffViewerData(task, enabled);
  const { viewed, toggleViewed } = useViewedFiles(task.id, data.files);
  const partial = usePartialApply(task, data.hunks, data.refresh);
  const [activeDiffPath, setActiveDiffPath] = useState<string | null>(null);
  const actions = useDiffReviewActions(task, activeDiffPath, data.annotations, data.refreshAnnotations);
  const [navigation, setNavigation] = useState<ReviewNavigation | null>(null);
  const consumeNavigation = useCallback(() => setNavigation(null), []);
  const openDiffRef = useRef(openDiff);
  openDiffRef.current = openDiff;
  const navigationSequence = useRef(0);
  const expiry = useRef<number | null>(null);
  useEffect(() => {
    setNavigation(null);
    return () => {
      navigationSequence.current += 1;
      if (expiry.current !== null) window.clearTimeout(expiry.current);
    };
  }, [task.host, task.id]);
  useReviewEvents(task, {
    onAnnotationsChanged: data.refreshAnnotations,
    onNavigate: (payload) => {
      // 탭이 없으면 연다. 스크롤은 그 탭이 hunk를 그린 뒤 `navigation`을 보고 한다.
      openDiffRef.current(payload.path);
      const nonce = ++navigationSequence.current;
      setNavigation(null);
      // 새 스냅샷이 도착하기 전에 DiffTab이 목표를 소비하지 않도록 갱신 완료 뒤에 전달한다.
      void data.refreshAfterCurrent().then(() => {
        if (navigationSequence.current === nonce) setNavigation(navigationFrom(payload, nonce));
      });
      // 소비할 탭이 이 창에 없을 수 있다(팝아웃 창이 가져간 파일, 다른 작업을 보는 중). 묵은 목표가
      // 나중에 탭이 열릴 때 튀지 않도록 한도를 둔다.
      if (expiry.current !== null) window.clearTimeout(expiry.current);
      expiry.current = window.setTimeout(() => {
        if (navigationSequence.current !== nonce) return;
        navigationSequence.current += 1;
        setNavigation(null);
      }, NAVIGATION_TTL_MS);
    },
  });

  return (
    <DiffSession.Provider
      value={{
        data,
        viewed,
        toggleViewed,
        partial,
        actions,
        activeDiffPath,
        setActiveDiffPath,
        openDiff,
        retain,
        navigation,
        consumeNavigation,
      }}
    >
      {children}
    </DiffSession.Provider>
  );
}

/** 작업이 선택되지 않았으면 diff 세션도 없다. 작업이 바뀌면 통째로 다시 만든다(설계 F-5). */
export function DiffSessionScope({
  task,
  openDiff,
  children,
}: {
  task: TaskRef | null;
  openDiff: OpenDiff;
  children: ReactNode;
}) {
  if (!task) return <>{children}</>;
  return (
    <DiffSessionProvider key={`${task.host}:${task.id}`} task={task} openDiff={openDiff}>
      {children}
    </DiffSessionProvider>
  );
}
