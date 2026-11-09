// 모바일 PWA 라우트 — 순수 파싱/직렬화. (설계 0013 §5.3)
// 러너가 /m/* 를 SPA fallback으로 서빙하므로 history API 경로를 그대로 쓴다.

/** 모든 모바일 경로의 접두사. 러너의 정적 서빙 스코프와 같아야 한다. */
export const BASE = "/m";

export const TASK_TABS = ["review", "convo", "terminal", "files"] as const;
export type TaskTab = (typeof TASK_TABS)[number];

export type Route =
  | { name: "home" }
  | { name: "assistant" }
  /**
   * `tab: null`은 "탭을 지정하지 않았다"는 뜻이다. 어느 탭을 열지는 작업의 mode가 정한다 —
   * 대화 작업을 열었는데 diff가 먼저 뜨면 들어온 목적과 어긋난다.
   */
  | { name: "task"; id: number; tab: TaskTab | null }
  | { name: "new" }
  | { name: "schedules" }
  | { name: "settings" }
  | { name: "notfound"; path: string };

/** mode에 따라 처음 열 탭. 대화 작업은 대화부터, 나머지는 리뷰부터. */
export function defaultTabFor(mode: string): TaskTab {
  return mode === "conversation" ? "convo" : "review";
}

function isTaskTab(value: string): value is TaskTab {
  return (TASK_TABS as readonly string[]).includes(value);
}

/**
 * pathname을 Route로 해석한다. BASE 밖이거나 알 수 없는 경로는 notfound.
 * 쿼리스트링·해시는 호출 전에 제거된 pathname만 받는다.
 */
export function matchRoute(pathname: string): Route {
  if (pathname !== BASE && !pathname.startsWith(`${BASE}/`)) {
    return { name: "notfound", path: pathname };
  }
  const segments = pathname
    .slice(BASE.length)
    .split("/")
    .filter((segment) => segment.length > 0);

  if (segments.length === 0) return { name: "home" };

  switch (segments[0]) {
    case "assistant":
      return segments.length === 1 ? { name: "assistant" } : { name: "notfound", path: pathname };
    case "new":
      return segments.length === 1 ? { name: "new" } : { name: "notfound", path: pathname };
    case "schedules":
      return segments.length === 1 ? { name: "schedules" } : { name: "notfound", path: pathname };
    case "settings":
      return segments.length === 1 ? { name: "settings" } : { name: "notfound", path: pathname };
    case "t": {
      // /m/t/:id 와 /m/t/:id/:tab 만 허용. id는 양의 정수(러너 task id).
      if (segments.length < 2 || segments.length > 3) return { name: "notfound", path: pathname };
      if (!/^[1-9][0-9]*$/.test(segments[1])) return { name: "notfound", path: pathname };
      const id = Number(segments[1]);
      if (!Number.isSafeInteger(id)) return { name: "notfound", path: pathname };
      if (segments.length === 2) return { name: "task", id, tab: null };
      if (!isTaskTab(segments[2])) return { name: "notfound", path: pathname };
      return { name: "task", id, tab: segments[2] };
    }
    default:
      return { name: "notfound", path: pathname };
  }
}

/** Route를 pathname으로 되돌린다. matchRoute(hrefFor(r))는 r과 같아야 한다. */
export function hrefFor(route: Route): string {
  switch (route.name) {
    case "home":
      return `${BASE}/`;
    case "assistant":
      return `${BASE}/assistant`;
    case "new":
      return `${BASE}/new`;
    case "schedules":
      return `${BASE}/schedules`;
    case "settings":
      return `${BASE}/settings`;
    case "task":
      return route.tab === null
        ? `${BASE}/t/${route.id}`
        : `${BASE}/t/${route.id}/${route.tab}`;
    case "notfound":
      return route.path;
  }
}

/** 푸시 알림 딥링크 대상. tab을 생략하면 작업 mode가 정한다. (설계 0013 §8) */
export function taskHref(id: number, tab: TaskTab | null = null): string {
  return hrefFor({ name: "task", id, tab });
}
