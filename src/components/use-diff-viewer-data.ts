import { useCallback, useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import type { BaselineStatus, DiffRange, TaskRef } from "../lib/transport";
import {
  annotationsList,
  diffHunks,
  taskDiff,
  type DiffHunk,
  type FileDiff,
  type RematchedAnnotation,
} from "../lib/ipc";

const DIFF_REFRESH_MS = 5_000;

export interface DiffViewerData {
  files: FileDiff[] | null;
  hunks: DiffHunk[];
  annotations: RematchedAnnotation[];
  /** 이 diff를 만든 기준점의 상태. 로드 전에는 null. */
  baseline: BaselineStatus | null;
  range: DiffRange;
  setRange: (range: DiffRange) => void;
  error: string | null;
  warning: string | null;
  refresh: () => void;
  refreshAnnotations: () => void;
}

interface DiffSnapshot {
  files: FileDiff[];
  hunks: DiffHunk[];
  annotations: RematchedAnnotation[];
  baseline: BaselineStatus;
  warning: string | null;
}

/** 세 요청에 **같은 범위**를 넘긴다. 어긋나면 화면이 그리는 hunk_id와 주석이 재매칭된
 *  hunk_id가 달라져, 붙여둔 주석이 한꺼번에 고아로 보인다. */
async function fetchDiffSnapshot(task: TaskRef, range: DiffRange): Promise<DiffSnapshot> {
  const diff = await taskDiff(task, range, true);
  // 새 계약은 같은 기준점에서 hunk와 주석을 함께 만든다. 구형 Runner는 review를 무시하므로
  // 기존 세 요청 경로를 남겨 호환한다.
  if (diff.review) {
    return {
      files: diff.files,
      hunks: diff.review.hunks,
      annotations: diff.review.annotations,
      baseline: diff.baseline,
      warning: diff.review.warning ?? null,
    };
  }
  const annotationRequest = annotationsList(task, range)
    .then((value) => ({ value, failed: false }))
    .catch(() => ({ value: [] as RematchedAnnotation[], failed: true }));
  const [hunks, annotationResult] = await Promise.all([
    diffHunks(task, range),
    annotationRequest,
  ]);
  return {
    files: diff.files,
    hunks,
    annotations: annotationResult.value,
    baseline: diff.baseline,
    warning: annotationResult.failed ? "주석을 불러오지 못했지만 diff는 최신 상태입니다." : null,
  };
}

/** 게이트가 닫혔다 열릴 때는 `load(true)`가 아니라 조용한 갱신이다. `files=null`을 거치면
 *  `hunkKey`가 흔들려 `usePartialApply`가 사용자의 선택을 기본값으로 되돌린다(설계 F-9).
 *  같은 `load`로 되돌아왔는지가 그 판정 기준이다 — 범위·작업이 바뀌면 `load`가 새 함수다. */
function useAutoDiffRefresh(
  load: (initial: boolean) => Promise<void>,
  requestId: { current: number },
  enabled: boolean,
): void {
  const lastLoad = useRef<((initial: boolean) => Promise<void>) | null>(null);
  useEffect(() => {
    if (!enabled) return;
    const initial = lastLoad.current !== load;
    lastLoad.current = load;
    void load(initial);
    const refreshVisible = () => {
      if (document.visibilityState !== "hidden") void load(false);
    };
    const interval = window.setInterval(refreshVisible, DIFF_REFRESH_MS);
    window.addEventListener("focus", refreshVisible);
    return () => {
      requestId.current += 1;
      window.clearInterval(interval);
      window.removeEventListener("focus", refreshVisible);
    };
  }, [load, requestId, enabled]);
}

function useAnnotationRefresh(
  task: TaskRef,
  range: DiffRange,
  requestId: { current: number },
  setAnnotations: Dispatch<SetStateAction<RematchedAnnotation[]>>,
  setWarning: Dispatch<SetStateAction<string | null>>,
): () => void {
  return useCallback(() => {
    const currentRequest = requestId.current;
    void annotationsList(task, range)
      .then((value) => {
        if (currentRequest === requestId.current) setAnnotations(value);
      })
      .catch(() => {
        if (currentRequest === requestId.current) {
          setWarning("주석을 불러오지 못했지만 diff는 최신 상태입니다.");
        }
      });
  }, [range, requestId, setAnnotations, setWarning, task.host, task.id]);
}

/** Diff 화면이 열린 동안 최신 worktree 상태를 유지한다. 핵심 diff와 주석 실패를 분리해,
 * 주석 API 하나가 실패해도 코드 변경 자체는 계속 표시한다.
 *
 * `enabled`가 거짓이면 폴링만 멈춘다 — 마지막 스냅샷은 그대로 남는다(설계 DR-7). */
export function useDiffViewerData(task: TaskRef, enabled = true): DiffViewerData {
  const [files, setFiles] = useState<FileDiff[] | null>(null);
  const [hunks, setHunks] = useState<DiffHunk[]>([]);
  const [annotations, setAnnotations] = useState<RematchedAnnotation[]>([]);
  const [baseline, setBaseline] = useState<BaselineStatus | null>(null);
  const [range, setRange] = useState<DiffRange>("session");
  const [error, setError] = useState<string | null>(null);
  const [warning, setWarning] = useState<string | null>(null);
  const requestId = useRef(0);
  const activeLoads = useRef(new Map<string, { promise: Promise<void>; generation: number }>());
  const currentKey = `${task.host}:${task.id}:${range}`;
  // render에서 즉시 갱신해 작업/범위 전환 직후 끝난 이전 요청도 화면에 쓰지 못하게 한다.
  const currentKeyRef = useRef(currentKey);
  currentKeyRef.current = currentKey;

  const load = useCallback(
    async (initial: boolean): Promise<void> => {
      const key = `${task.host}:${task.id}:${range}`;
      if (initial) {
        setFiles(null);
        setError(null);
      }
      const active = activeLoads.current.get(key);
      if (active) {
        if (active.generation === requestId.current) return active.promise;
        // A→B→A: wait for the obsolete request, then fetch a fresh snapshot without overlap.
        const generation = requestId.current;
        await active.promise;
        if (key === currentKeyRef.current && generation === requestId.current) {
          return load(initial);
        }
        return;
      }
      const generation = ++requestId.current;
      let request!: Promise<void>;
      request = (async () => {
        try {
          const snapshot = await fetchDiffSnapshot(task, range);
          if (key !== currentKeyRef.current || generation !== requestId.current) return;
          setFiles(snapshot.files);
          setHunks(snapshot.hunks);
          setAnnotations(snapshot.annotations);
          setBaseline(snapshot.baseline);
          setWarning(snapshot.warning);
          setError(null);
        } catch (cause) {
          if (key === currentKeyRef.current && generation === requestId.current) setError(String(cause));
        } finally {
          if (activeLoads.current.get(key)?.promise === request) activeLoads.current.delete(key);
        }
      })();
      activeLoads.current.set(key, { promise: request, generation });
      return request;
    },
    [range, task.host, task.id],
  );

  // 범위는 작업의 속성이 아니라 보기 방식이지만, 작업이 바뀌면 기본값으로 돌아간다.
  // 지금은 `DiffSessionScope`가 작업마다 세션을 다시 만들어 이 훅도 함께 새로 시작하므로
  // 이 effect가 실제로 도는 일은 없다 — 훅 하나만 떼어 쓰는 호출자를 위한 보루로 남긴다.
  useEffect(() => setRange("session"), [task.host, task.id]);

  const refreshAnnotations = useAnnotationRefresh(
    task,
    range,
    requestId,
    setAnnotations,
    setWarning,
  );
  useAutoDiffRefresh(load, requestId, enabled);

  return {
    files,
    hunks,
    annotations,
    baseline,
    range,
    setRange,
    error,
    warning,
    refresh: () => void load(false),
    refreshAnnotations,
  };
}
