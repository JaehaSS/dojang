// 에이전트가 MCP 리뷰 툴을 부르면 Rust가 Tauri 이벤트를 쏜다. 이 훅이 그 이벤트를
// 작업의 diff 세션에 잇는다 — 노트가 붙으면 주석만 다시 읽고, 이동 요청은 목표로 남긴다.
// 5초 폴링을 기다리게 하면 "방금 남겼다"는 에이전트의 말과 화면이 어긋난다.

import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import type { ReviewAnnotationsChangedPayload, ReviewNavigatePayload } from "../lib/ipc";
import { LOCAL_HOST, type TaskRef } from "../lib/transport";

export const REVIEW_ANNOTATIONS_EVENT = "review://annotations";
export const REVIEW_NAVIGATE_EVENT = "review://navigate";

/** DiffTab이 소비하는 이동 목표. `nonce`는 같은 hunk로 두 번 보내도 다시 스크롤하게 한다. */
export interface ReviewNavigation {
  path: string;
  hunkId: string;
  line: number;
  side: string;
  nonce: number;
}

export function navigationFrom(payload: ReviewNavigatePayload, nonce: number): ReviewNavigation {
  return { path: payload.path, hunkId: payload.hunk_id, line: payload.line, side: payload.side, nonce };
}

/** 목표가 가리키는 hunk id. 없으면 줄 번호로 되찾는다 — 화면이 다른 범위를 보고 있거나 파일이
 *  그새 바뀌면 id가 전부 갈리지만, 줄은 대개 같은 hunk 안에 남아 있다(Rust `contains_line`과 같은 규칙). */
export function resolveNavigationHunk(
  target: ReviewNavigation,
  hunks: Array<{ id: string; path: string; old_range: [number, number]; new_range: [number, number] }>,
): string | null {
  const exact = hunks.find((hunk) => hunk.id === target.hunkId);
  if (exact) return exact.id;
  const byLine = hunks.find((hunk) => {
    if (hunk.path !== target.path) return false;
    const [start, count] = target.side === "old" ? hunk.old_range : hunk.new_range;
    return start <= target.line && target.line < start + Math.max(count, 1);
  });
  return byLine?.id ?? null;
}

/** 이벤트가 이 세션의 작업을 겨눴는가. MCP 서버는 로컬 앱 안에 있으므로 원격 호스트의 작업은
 *  같은 id라도 남의 것이다. */
export function targetsTask(task: TaskRef, taskId: number): boolean {
  return task.host === LOCAL_HOST && task.id === taskId;
}

interface ReviewEventHandlers {
  onAnnotationsChanged: () => void;
  onNavigate: (payload: ReviewNavigatePayload) => void;
}

/** jsdom·브라우저처럼 Tauri 내부가 없는 곳에서는 `listen`이 거부되고, 그걸로 끝이다 —
 *  폴링이 남아 있으니 기능이 죽지는 않는다(`use-notification-snapshot`과 같은 모양). */
export function useReviewEvents(task: TaskRef, handlers: ReviewEventHandlers): void {
  const ref = useRef(handlers);
  ref.current = handlers;

  useEffect(() => {
    let active = true;
    const stops: Array<() => void> = [];
    const subscribe = <T,>(name: string, handle: (payload: T) => void) =>
      Promise.resolve()
        .then(() => listen<T>(name, (event) => handle(event.payload)))
        .then((unlisten) => {
          if (!active) return unlisten();
          stops.push(unlisten);
        })
        .catch(() => {});
    void subscribe<ReviewAnnotationsChangedPayload>(REVIEW_ANNOTATIONS_EVENT, (payload) => {
      if (active && targetsTask(task, payload.task_id)) ref.current.onAnnotationsChanged();
    });
    void subscribe<ReviewNavigatePayload>(REVIEW_NAVIGATE_EVENT, (payload) => {
      if (active && targetsTask(task, payload.task_id)) ref.current.onNavigate(payload);
    });
    return () => {
      active = false;
      for (const stop of stops) void Promise.resolve(stop()).catch(() => {});
    };
  }, [task.host, task.id]);
}
