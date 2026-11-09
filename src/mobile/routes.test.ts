import { describe, expect, it } from "vitest";
import {
  BASE,
  defaultTabFor,
  hrefFor,
  matchRoute,
  taskHref,
  TASK_TABS,
  type Route,
} from "./routes";

describe("matchRoute", () => {
  it("BASE 자체와 트레일링 슬래시를 홈으로 본다", () => {
    expect(matchRoute("/m")).toEqual({ name: "home" });
    expect(matchRoute("/m/")).toEqual({ name: "home" });
  });

  it("단일 세그먼트 화면을 해석한다", () => {
    expect(matchRoute("/m/assistant")).toEqual({ name: "assistant" });
    expect(matchRoute("/m/new")).toEqual({ name: "new" });
    expect(matchRoute("/m/schedules")).toEqual({ name: "schedules" });
    expect(matchRoute("/m/settings")).toEqual({ name: "settings" });
  });

  it("탭을 생략하면 미지정으로 남긴다", () => {
    // 어느 탭을 열지는 작업 mode가 정한다 — 여기서 review로 굳히면 대화 작업이
    // diff 화면으로 열린다.
    expect(matchRoute("/m/t/12")).toEqual({ name: "task", id: 12, tab: null });
  });

  it("모든 탭을 해석한다", () => {
    for (const tab of TASK_TABS) {
      expect(matchRoute(`/m/t/7/${tab}`)).toEqual({ name: "task", id: 7, tab });
    }
  });

  it("BASE 밖 경로는 notfound", () => {
    expect(matchRoute("/")).toEqual({ name: "notfound", path: "/" });
    expect(matchRoute("/v1/health")).toEqual({ name: "notfound", path: "/v1/health" });
    // 접두사만 같고 경계가 다른 경로에 걸리지 않아야 한다.
    expect(matchRoute("/mobile")).toEqual({ name: "notfound", path: "/mobile" });
  });

  it("잘못된 task id를 거부한다", () => {
    for (const path of ["/m/t/0", "/m/t/-1", "/m/t/1.5", "/m/t/abc", "/m/t/01", "/m/t/"]) {
      expect(matchRoute(path).name).toBe("notfound");
    }
  });

  it("알 수 없는 탭과 과잉 세그먼트를 거부한다", () => {
    expect(matchRoute("/m/t/1/editor").name).toBe("notfound");
    expect(matchRoute("/m/t/1/review/extra").name).toBe("notfound");
    expect(matchRoute("/m/new/extra").name).toBe("notfound");
    expect(matchRoute("/m/unknown").name).toBe("notfound");
  });
});

describe("hrefFor", () => {
  it("matchRoute의 역함수다", () => {
    const routes: Route[] = [
      { name: "home" },
      { name: "assistant" },
      { name: "new" },
      { name: "schedules" },
      { name: "settings" },
      { name: "task", id: 3, tab: null },
      ...TASK_TABS.map((tab) => ({ name: "task", id: 42, tab }) as Route),
    ];
    for (const route of routes) {
      expect(matchRoute(hrefFor(route))).toEqual(route);
    }
  });

  it("미지정 탭은 경로에 싣지 않는다", () => {
    expect(hrefFor({ name: "task", id: 5, tab: null })).toBe(`${BASE}/t/5`);
    expect(hrefFor({ name: "task", id: 5, tab: "review" })).toBe(`${BASE}/t/5/review`);
    expect(hrefFor({ name: "task", id: 5, tab: "terminal" })).toBe(`${BASE}/t/5/terminal`);
  });
});

describe("taskHref", () => {
  it("푸시 딥링크 경로를 만든다", () => {
    expect(taskHref(9)).toBe("/m/t/9");
    expect(matchRoute(taskHref(9, "convo"))).toEqual({ name: "task", id: 9, tab: "convo" });
  });
});

describe("defaultTabFor", () => {
  it("대화 작업은 대화 탭부터 연다", () => {
    expect(defaultTabFor("conversation")).toBe("convo");
  });

  it("그 외에는 리뷰부터 연다", () => {
    expect(defaultTabFor("terminal")).toBe("review");
    expect(defaultTabFor("")).toBe("review");
  });
});
