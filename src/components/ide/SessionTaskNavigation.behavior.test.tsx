// @vitest-environment jsdom

import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Task } from "../../lib/ipc";
import {
  EMPTY_PROJECT_GROUPS,
  loadProjectGroups,
  saveProjectGroups,
  type ProjectGroups,
} from "../../lib/project-groups";
import { SessionTaskNavigation } from "./SessionTaskNavigation";
import { DROP_ZONE_CLASS, PROJECT_DRAG_MIME, PROJECT_GROUP_DRAG_MIME } from "./project-drag";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const ALPHA = "/workspace/alpha";
const BETA = "/workspace/beta";

const makeTask = (id: number, repo: string, createdAt: number): Task => ({
  id,
  host: "local",
  repo,
  branch: `feature/task-${id}`,
  base: "main",
  worktree_path: `${repo}/.praxis/worktrees/task-${id}`,
  instruction: `작업 ${id}`,
  state: "Running",
  created_at: createdAt,
  updated_at: createdAt,
  mode: "conversation",
});

// alpha가 더 최근이므로 미소속 목록은 alpha → beta 순이다.
const TASKS: Task[] = [makeTask(11, ALPHA, 30), makeTask(12, BETA, 5)];

const EMPTY: ProjectGroups = { version: 1, groups: [], assignment: {} };
const withGroup = (
  ids: [string, string][],
  assignment: Record<string, string> = {},
  collapsed: string[] = [],
): ProjectGroups => ({
  version: 1,
  groups: ids.map(([id, name]) => ({ id, name, collapsed: collapsed.includes(id) })),
  assignment,
});

/** jsdom에는 DataTransfer가 없다 — 값을 나르는 통 하나만 흉내 낸다. */
class FakeDataTransfer {
  private store = new Map<string, string>();
  dropEffect = "uninitialized";
  effectAllowed = "none";
  get types(): string[] {
    return Array.from(this.store.keys());
  }
  setData(type: string, value: string): void {
    this.store.set(type, value);
  }
  getData(type: string): string {
    return this.store.get(type) ?? "";
  }
}

const BOX_H = 100;
/** 그룹 박스는 위에서부터 100px씩 쌓인다 — jsdom의 rect는 전부 0이라 자리 판정이 결정적이지 않다. */
const stubBoxes = () => {
  const original = HTMLElement.prototype.getBoundingClientRect;
  HTMLElement.prototype.getBoundingClientRect = function rect(this: HTMLElement): DOMRect {
    const boxes = Array.from(document.querySelectorAll<HTMLElement>("[data-group-box]"));
    const top = Math.max(0, boxes.indexOf(this)) * BOX_H;
    return { top, height: BOX_H, bottom: top + BOX_H, left: 0, width: 240 } as DOMRect;
  };
  return () => {
    HTMLElement.prototype.getBoundingClientRect = original;
  };
};

let container: HTMLDivElement;
let root: Root;
let changes: ProjectGroups[];
let restoreRect: (() => void) | null = null;

class ResizeObserverStub {
  static instances: ResizeObserverStub[] = [];
  constructor(private readonly callback: ResizeObserverCallback) {
    ResizeObserverStub.instances.push(this);
  }
  disconnect() {}
  observe() {}
  trigger(): void {
    this.callback([], this as unknown as ResizeObserver);
  }
}

function Harness({ initial, projects }: { initial: ProjectGroups; projects: string[] }) {
  const [groups, setGroups] = useState<ProjectGroups>(initial);
  return (
    <SessionTaskNavigation
      tasks={TASKS}
      selectedKey={null}
      projects={projects}
      onOpenTask={() => {}}
      onNewInRepo={() => {}}
      onDeleteTask={() => {}}
      onRemoveProject={() => {}}
      onDiscardOrphans={() => {}}
      groups={groups}
      onProjectGroupsChange={(next) => {
        changes.push(next);
        saveProjectGroups(next);
        setGroups(next);
      }}
    />
  );
}

const render = async (initial: ProjectGroups, projects: string[] = [ALPHA, BETA]) => {
  await act(async () => {
    root.render(<Harness initial={initial} projects={projects} />);
  });
};

const last = (): ProjectGroups => changes[changes.length - 1];

const headers = (): HTMLElement[] =>
  Array.from(container.querySelectorAll<HTMLElement>('[draggable="true"]'));
const projectHeader = (repo: string): HTMLElement =>
  headers().filter((node) => !node.hasAttribute("aria-expanded"))
    .find((node) => node.textContent?.includes(repo.split("/").pop() ?? ""))!;
const groupHeader = (name: string): HTMLElement =>
  headers().find((node) => node.hasAttribute("aria-expanded") && node.textContent?.includes(name))!;
const box = (id: string): HTMLElement =>
  container.querySelector<HTMLElement>(`[data-group-box="${id}"]`)!;
const background = (): HTMLElement =>
  container.querySelector<HTMLElement>('section[aria-label="세션 작업"]')!;
const item = (label: string): HTMLButtonElement =>
  Array.from(document.body.querySelectorAll("button")).find((node) =>
    node.textContent?.includes(label),
  )!;

const fire = async (node: HTMLElement, type: string, init: MouseEventInit = {}) => {
  await act(async () => {
    node.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, ...init }));
  });
};

const key = async (node: HTMLElement, value: string, init: KeyboardEventInit = {}) => {
  await act(async () => {
    node.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: value, ...init }));
  });
};

const drag = async (
  node: HTMLElement,
  type: string,
  data: FakeDataTransfer,
  init: MouseEventInit = {},
) => {
  await act(async () => {
    const event = new MouseEvent(type, { bubbles: true, cancelable: true, ...init });
    Object.defineProperty(event, "dataTransfer", { value: data });
    node.dispatchEvent(event);
  });
};

/** 프로젝트 헤더를 실제로 잡는다 — dataTransfer에 값을 싣는 것도 끌던 원본을 기억하는 것도 여기서 난다. */
const grabProject = async (repo: string): Promise<FakeDataTransfer> => {
  const data = new FakeDataTransfer();
  await drag(projectHeader(repo), "dragstart", data);
  return data;
};

beforeEach(() => {
  changes = [];
  ResizeObserverStub.instances = [];
  localStorage.clear();
  vi.stubGlobal("ResizeObserver", ResizeObserverStub);
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  localStorage.clear();
  restoreRect?.();
  restoreRect = null;
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("우클릭 메뉴로 그룹을 다룬다 (A-3.3)", () => {
  it("드래그 가능한 프로젝트 헤더의 메뉴를 사이드바 스크롤 밖에 띄운다", async () => {
    await render(withGroup([["g1", "사내용"]]));

    await fire(projectHeader(ALPHA), "contextmenu", { clientX: 20, clientY: 30 });

    const menu = document.body.querySelector<HTMLElement>('[role="menu"]');
    expect(menu?.parentElement).toBe(document.body);
    expect(menu?.style.left).toBe("20px");
    expect(menu?.style.top).toBe("30px");

    await fire(projectHeader(ALPHA), "contextmenu", { clientX: -20, clientY: -20 });
    expect(menu?.style.left).toBe("8px");
    expect(menu?.style.top).toBe("8px");
  });

  it("그룹 목록으로 바뀌어도 화면 안쪽에 붙인다", async () => {
    const original = HTMLElement.prototype.getBoundingClientRect;
    HTMLElement.prototype.getBoundingClientRect = function rect(this: HTMLElement): DOMRect {
      if (this.getAttribute("role") !== "menu") return original.call(this);
      const expanded = this.textContent?.includes("새 그룹…") ?? false;
      return { height: expanded ? 384 : 100, width: 200 } as DOMRect;
    };
    restoreRect = () => {
      HTMLElement.prototype.getBoundingClientRect = original;
    };
    vi.stubGlobal("innerHeight", 400);
    vi.stubGlobal("innerWidth", 400);
    await render(withGroup([["g1", "사내용"]]));

    await fire(projectHeader(ALPHA), "contextmenu", { clientX: 390, clientY: 390 });
    await fire(item("그룹으로 이동"), "click");
    await act(async () => ResizeObserverStub.instances[0]?.trigger());

    const menu = document.body.querySelector<HTMLElement>('[role="menu"]')!;
    expect(menu.style.left).toBe("192px");
    expect(menu.style.top).toBe("8px");
  });

  it("그룹으로 이동 — 같은 박스가 그룹 목록으로 교체된다", async () => {
    await render(withGroup([["g1", "사내용"]]));

    await fire(projectHeader(ALPHA), "contextmenu");
    await fire(item("그룹으로 이동"), "click");
    await fire(item("사내용"), "click");

    expect(last().assignment).toEqual({ [ALPHA]: "g1" });
  });

  it("새 그룹… — 만들고 넣고 헤더를 바로 이름 편집으로 연다", async () => {
    await render(EMPTY);

    await fire(projectHeader(ALPHA), "contextmenu");
    await fire(item("그룹으로 이동"), "click");
    await fire(item("새 그룹…"), "click");

    const created = last().groups[0];
    expect(created.name).toBe("새 그룹");
    expect(last().assignment).toEqual({ [ALPHA]: created.id });
    expect(container.querySelector('input[aria-label="그룹 이름"]')).not.toBeNull();
  });

  it("기존 그룹의 프로젝트로 새 그룹 만들기를 취소하면 원래 그룹에 남긴다", async () => {
    await render(withGroup([["g1", "사내용"]], { [ALPHA]: "g1" }));

    await fire(projectHeader(ALPHA), "contextmenu");
    await fire(item("그룹으로 이동"), "click");
    await fire(item("새 그룹…"), "click");
    const input = container.querySelector<HTMLInputElement>('input[aria-label="그룹 이름"]')!;
    await act(async () => {
      input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Escape" }));
    });

    expect(last().groups.map((group) => group.id)).toEqual(["g1"]);
    expect(last().assignment).toEqual({ [ALPHA]: "g1" });
  });

  it("새 그룹 이름을 비워 취소해도 기존 그룹의 프로젝트를 보존한다", async () => {
    await render(withGroup([["g1", "사내용"]], { [ALPHA]: "g1" }));

    await fire(projectHeader(ALPHA), "contextmenu");
    await fire(item("그룹으로 이동"), "click");
    await fire(item("새 그룹…"), "click");
    const input = container.querySelector<HTMLInputElement>('input[aria-label="그룹 이름"]')!;
    input.value = " ";
    await act(async () => {
      input.dispatchEvent(new FocusEvent("focusout", { bubbles: true }));
    });

    expect(last().groups.map((group) => group.id)).toEqual(["g1"]);
    expect(last().assignment).toEqual({ [ALPHA]: "g1" });
  });

  it("빈 곳 우클릭 — 새 그룹 만들기가 빈 그룹을 만들고 이름 편집을 연다", async () => {
    await render(EMPTY);

    await fire(background(), "contextmenu");
    await fire(item("새 그룹 만들기"), "click");

    expect(last().groups.map((group) => group.name)).toEqual(["새 그룹"]);
    expect(last().assignment).toEqual({});
    expect(container.querySelector('input[aria-label="그룹 이름"]')).not.toBeNull();
  });

  it("빈 곳에서 만든 그룹의 이름을 비워 취소하면 그룹이 남지 않는다", async () => {
    await render(EMPTY);

    await fire(background(), "contextmenu");
    await fire(item("새 그룹 만들기"), "click");
    const input = container.querySelector<HTMLInputElement>('input[aria-label="그룹 이름"]')!;
    input.value = " ";
    await act(async () => {
      input.dispatchEvent(new FocusEvent("focusout", { bubbles: true }));
    });

    expect(last().groups).toEqual([]);
    expect(last().assignment).toEqual({});
  });

  it("그룹 헤더 우클릭은 그룹 메뉴만 열고 빈 곳 메뉴를 열지 않는다", async () => {
    await render(withGroup([["g1", "사내용"]]));

    await fire(groupHeader("사내용"), "contextmenu");

    expect(item("이름 바꾸기")).toBeDefined();
    expect(item("새 그룹 만들기")).toBeUndefined();
  });

  it("IME 조합 중 Enter와 Escape는 그룹 이름 편집을 끝내지 않는다", async () => {
    await render(withGroup([["g1", "사내용"]]));

    await fire(groupHeader("사내용"), "contextmenu");
    await fire(item("이름 바꾸기"), "click");
    const input = container.querySelector<HTMLInputElement>('input[aria-label="그룹 이름"]')!;
    const composingEnter = new KeyboardEvent("keydown", { bubbles: true, key: "Enter" });
    Object.defineProperty(composingEnter, "isComposing", { value: true });
    const composingEscape = new KeyboardEvent("keydown", { bubbles: true, key: "Escape" });
    Object.defineProperty(composingEscape, "isComposing", { value: true });
    await act(async () => {
      input.dispatchEvent(composingEnter);
      input.dispatchEvent(composingEscape);
      const compositionKey = new KeyboardEvent("keydown", { bubbles: true, key: "Enter" });
      Object.defineProperty(compositionKey, "keyCode", { value: 229 });
      input.dispatchEvent(compositionKey);
    });

    expect(container.querySelector('input[aria-label="그룹 이름"]')).not.toBeNull();
    expect(changes).toEqual([]);
    input.value = "새 이름";
    await act(async () => {
      input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Enter" }));
    });
    expect(last().groups[0].name).toBe("새 이름");
  });

  it("만들기·이름 바꾸기·옮기기·빼기·해제가 저장된 상태까지 이어진다", async () => {
    await render(EMPTY);

    await fire(projectHeader(ALPHA), "contextmenu");
    await fire(item("그룹으로 이동"), "click");
    await fire(item("새 그룹…"), "click");
    const input = container.querySelector<HTMLInputElement>('input[aria-label="그룹 이름"]')!;
    input.value = "사내용";
    await act(async () => {
      input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Enter" }));
    });

    await fire(projectHeader(BETA), "contextmenu");
    await fire(item("그룹으로 이동"), "click");
    await fire(item("사내용"), "click");
    const persisted = loadProjectGroups();
    expect(persisted).toEqual(last());
    await act(async () => root.unmount());
    root = createRoot(container);
    await render(persisted);
    expect(groupHeader("사내용")).toBeDefined();

    await fire(projectHeader(ALPHA), "contextmenu");
    await fire(item("그룹에서 빼기"), "click");
    await fire(groupHeader("사내용"), "contextmenu");
    await fire(item("그룹 해제"), "click");

    expect(loadProjectGroups()).toEqual(EMPTY_PROJECT_GROUPS);
  });

  it("그룹에서 빼기 — 배정만 지운다", async () => {
    await render(withGroup([["g1", "사내용"]], { [ALPHA]: "g1" }));

    await fire(projectHeader(ALPHA), "contextmenu");
    await fire(item("그룹에서 빼기"), "click");

    expect(last().assignment).toEqual({});
    expect(last().groups).toHaveLength(1);
  });

  // 확인 없이 실행되므로 결과(몇 개가 미소속이 되는지)를 라벨에 병기한다.
  it("그룹 해제 — 라벨에 프로젝트 수를 적고, 프로젝트는 남긴다", async () => {
    await render(withGroup([["g1", "사내용"]], { [ALPHA]: "g1", [BETA]: "g1" }));

    await fire(groupHeader("사내용"), "contextmenu");
    expect(item("그룹 해제").textContent).toContain("프로젝트 2개 미소속으로");

    await fire(item("그룹 해제"), "click");

    expect(last().groups).toEqual([]);
    expect(last().assignment).toEqual({});
  });

  it("그룹 색상 — 이름 있는 프리셋을 고르고 기본값으로 되돌린다", async () => {
    await render(withGroup([["g1", "사내용"]]));

    await fire(groupHeader("사내용"), "contextmenu");
    await fire(item("그룹 색상"), "click");
    const menu = document.body.querySelector<HTMLElement>('[role="menu"]')!;
    const colors = Array.from(menu.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]'));
    expect(menu.getAttribute("aria-label")).toBe("그룹 색상");
    expect(colors.map((button) => button.textContent?.trim())).toEqual([
      "파랑", "보라", "분홍", "주황", "초록", "회색", "기본값으로 되돌리기",
    ]);
    expect(colors.slice(0, -1).every((button) => button.getAttribute("aria-checked") === "false")).toBe(true);
    expect(colors[colors.length - 1]?.getAttribute("aria-checked")).toBe("true");

    await fire(item("파랑"), "click");
    expect(last().groups[0].color).toBe("blue");
    expect(container.querySelector('[data-group-header="g1"]')?.getAttribute("data-group-color")).toBe("blue");
    expect(container.querySelector('[data-group-color-strip="blue"]')).not.toBeNull();
    expect(container.querySelector<HTMLElement>("[data-group-body]")?.style.backgroundColor).toBe("");

    await fire(groupHeader("사내용"), "contextmenu");
    await fire(item("그룹 색상"), "click");
    expect(item("파랑").getAttribute("aria-checked")).toBe("true");
    await fire(item("기본값으로 되돌리기"), "click");
    expect(last().groups[0]).not.toHaveProperty("color");
    expect(container.querySelector("[data-group-color-strip]")).toBeNull();
  });

  it("키보드로 그룹을 접고 색상 메뉴를 열며 Escape는 헤더로 돌려보낸다", async () => {
    await render(withGroup([["g1", "사내용"]]));
    const header = groupHeader("사내용");
    header.focus();

    await key(header, "Enter");
    expect(last().groups[0].collapsed).toBe(true);
    await key(header, " ");
    expect(last().groups[0].collapsed).toBe(false);
    await key(header, "F10", { shiftKey: true });
    expect(document.activeElement).toBe(item("이름 바꾸기"));
    await fire(item("그룹 색상"), "click");
    expect(document.activeElement).toBe(item("파랑"));

    await fire(item("그룹 색상"), "click");
    expect(document.activeElement).toBe(item("이름 바꾸기"));
    await fire(item("그룹 색상"), "click");

    await key(document.activeElement as HTMLElement, "Escape");
    expect(document.body.querySelector('[role="menu"]')).toBeNull();
    expect(document.activeElement).toBe(header);
  });

  it("그룹 메뉴로 부모를 고르고 다시 루트로 뺀다", async () => {
    await render(withGroup([["g1", "사내용"], ["g2", "개인용"]]));

    await fire(groupHeader("개인용"), "contextmenu");
    await fire(item("그룹으로 이동"), "click");
    await fire(item("사내용"), "click");
    expect(last().groups.find((group) => group.id === "g2")?.parentId).toBe("g1");

    await fire(groupHeader("개인용"), "contextmenu");
    await fire(item("그룹에서 빼기"), "click");
    expect(last().groups.find((group) => group.id === "g2")?.parentId).toBeNull();
  });

  it("부모 선택은 자신과 후손을 제시하지 않는다", async () => {
    const initial = withGroup([["g1", "사내용"], ["g2", "개인용"]]);
    initial.groups[1].parentId = "g1";
    await render(initial);

    await fire(groupHeader("사내용"), "contextmenu");
    await fire(item("그룹으로 이동"), "click");

    expect(document.body.querySelector('[role="menu"]')?.textContent).not.toContain("개인용");
  });
});

describe("끌어다 놓기 (A-3.4)", () => {
  it("프로젝트를 그룹 박스에 넣는다", async () => {
    await render(withGroup([["g1", "사내용"]]));
    const data = await grabProject(ALPHA);

    await drag(box("g1"), "dragover", data);
    expect(box("g1").className).toContain(DROP_ZONE_CLASS);

    await drag(box("g1"), "drop", data);
    expect(last().assignment).toEqual({ [ALPHA]: "g1" });
  });

  it("드롭 강조는 그룹 색상 장식을 가린다", async () => {
    const initial = withGroup([["g1", "사내용"]]);
    initial.groups[0].color = "blue";
    await render(initial);
    const data = await grabProject(ALPHA);

    await drag(box("g1"), "dragover", data);
    expect(box("g1").className).toContain(DROP_ZONE_CLASS);
    expect(box("g1").querySelector("[data-group-color-strip]")).toBeNull();
    expect(box("g1").querySelector<HTMLElement>("[data-group-header]")?.style.backgroundColor).toBe("");
  });

  it("미소속 구획에 놓으면 그룹에서 빠진다", async () => {
    await render(withGroup([["g1", "사내용"]], { [ALPHA]: "g1" }));
    const data = await grabProject(ALPHA);
    const zone = container.querySelector<HTMLElement>("[data-unassigned]")!;

    await drag(zone, "dragover", data);
    expect(zone.className).toContain(DROP_ZONE_CLASS);

    await drag(zone, "drop", data);
    expect(last().assignment).toEqual({});
  });

  it("미소속이 0개면 끄는 동안만 빼낼 자리가 생긴다", async () => {
    await render(withGroup([["g1", "사내용"]], { [ALPHA]: "g1", [BETA]: "g1" }));
    expect(container.textContent).not.toContain("그룹에서 빼기");

    await grabProject(ALPHA);

    expect(container.textContent).toContain("그룹에서 빼기");
  });

  it("그룹을 끌면 삽입선이 서고, 놓으면 순서가 바뀐다", async () => {
    restoreRect = stubBoxes();
    await render(withGroup([["g1", "사내용"], ["g2", "개인용"]]));
    const data = new FakeDataTransfer();
    const list = box("g1").parentElement!;

    await drag(groupHeader("사내용"), "dragstart", data);
    expect(data.getData(PROJECT_GROUP_DRAG_MIME)).toBe("g1");

    // 두 번째 박스(100~200)의 아래 절반 — 경계는 박스 가운데다.
    await drag(list, "dragover", data, { clientY: 160 });
    expect(container.querySelectorAll("[data-drop-caret]")).toHaveLength(1);
    expect(container.querySelector("[data-drop-caret]")?.previousElementSibling?.getAttribute("data-group-box")).toBe("g2");

    await drag(list, "dragleave", data);
    expect(container.querySelectorAll("[data-drop-caret]")).toHaveLength(0);

    await drag(list, "dragover", data, { clientY: 160 });

    await drag(list, "drop", data, { clientY: 160 });
    expect(last().groups.map((group) => group.id)).toEqual(["g2", "g1"]);
  });

  it("확장된 그룹의 헤더 아래 경계는 그 그룹 뒤에 삽입한다", async () => {
    restoreRect = stubBoxes();
    await render(withGroup([["g1", "사내용"], ["g2", "개인용"]]));
    const data = new FakeDataTransfer();

    await drag(groupHeader("사내용"), "dragstart", data);
    await drag(groupHeader("개인용"), "dragover", data, { clientY: 99 });
    expect(container.querySelector("[data-drop-caret]")?.previousElementSibling?.getAttribute("data-group-box")).toBe("g2");

    await drag(groupHeader("개인용"), "drop", data, { clientY: 99 });
    expect(last().groups.map((group) => group.id)).toEqual(["g2", "g1"]);
  });

  it("그룹을 다른 그룹 본문에 놓으면 하위 그룹이 된다", async () => {
    await render(withGroup([["g1", "사내용"], ["g2", "개인용"]], { [ALPHA]: "g1" }));
    const data = new FakeDataTransfer();

    await drag(groupHeader("개인용"), "dragstart", data);
    await drag(box("g1").querySelector<HTMLElement>("[data-group-body]")!, "dragover", data);
    expect(box("g1").className).toContain(DROP_ZONE_CLASS);

    await drag(box("g1").querySelector<HTMLElement>("[data-group-body]")!, "drop", data);

    expect(last().groups.find((group) => group.id === "g2")?.parentId).toBe("g1");
  });

  it("이미 그 그룹에 있으면 강조하지 않는다", async () => {
    await render(withGroup([["g1", "사내용"]], { [ALPHA]: "g1" }));
    const data = await grabProject(ALPHA);

    await drag(box("g1"), "dragover", data);

    expect(box("g1").className).not.toContain(DROP_ZONE_CLASS);
    expect(data.getData(PROJECT_DRAG_MIME)).toBe(ALPHA);
  });

  it("자기 후손에 놓는 드래그는 상위 대상에 버블하지 않는다", async () => {
    const initial = withGroup([["g1", "사내용"], ["g2", "개인용"]]);
    initial.groups[1].parentId = "g1";
    await render(initial);
    const data = new FakeDataTransfer();

    await drag(groupHeader("사내용"), "dragstart", data);
    await drag(box("g2"), "dragover", data);
    await drag(box("g2"), "drop", data);

    expect(data.dropEffect).toBe("none");
    expect(changes).toEqual([]);
  });
});

describe("⌘1‥⌘9 (A-3.2)", () => {
  it("접힌 그룹 안의 작업은 번호를 건너뛴다", async () => {
    vi.useFakeTimers();
    await render(withGroup([["g1", "사내용"]], { [ALPHA]: "g1" }, ["g1"]));

    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Meta", metaKey: true }));
    });
    await act(async () => {
      vi.advanceTimersByTime(400);
    });

    // 접힌 그룹 안의 alpha 작업은 화면에 없다 — 번호를 세면 2, 5처럼 빈 번호가 남는다.
    const badges = Array.from(container.querySelectorAll("[data-shortcut]"));
    expect(badges.map((node) => node.getAttribute("data-shortcut"))).toEqual(["1"]);
    expect(container.textContent).toContain("작업 12");
    expect(container.textContent).not.toContain("작업 11");
    vi.useRealTimers();
  });

  it("부모를 접으면 자식 그룹과 그 작업도 단축키에서 사라진다", async () => {
    vi.useFakeTimers();
    const initial = withGroup([["g1", "사내용"], ["g2", "개인용"]], { [ALPHA]: "g2" }, ["g1"]);
    initial.groups[1].parentId = "g1";
    await render(initial);

    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Meta", metaKey: true }));
      vi.advanceTimersByTime(400);
    });

    expect(container.querySelector('[data-group-box="g2"]')).toBeNull();
    expect(container.querySelectorAll("[data-shortcut]")).toHaveLength(1);
    expect(container.textContent).not.toContain("작업 11");
    vi.useRealTimers();
  });
});

describe("최근 세션 행도 트리 카드와 같은 메뉴를 쓴다", () => {
  const renderWithRecent = async (recent: Task, onDeleteTask: (task: Task) => void) => {
    await act(async () => {
      root.render(
        <SessionTaskNavigation
          tasks={[recent, ...TASKS]}
          selectedKey={null}
          projects={[ALPHA, BETA]}
          onOpenTask={() => {}}
          onNewInRepo={() => {}}
          onDeleteTask={onDeleteTask}
          onRemoveProject={() => {}}
          onDiscardOrphans={() => {}}
        />,
      );
    });
  };
  const recentRow = (id: number): HTMLElement =>
    container.querySelector<HTMLElement>(
      `section[aria-label="최근 세션"] [data-task-key="local:${id}"]`,
    )!;

  // 최근 세션은 트리를 펼치지 않고 찾은 세션이다 — 거기서 바로 워크트리를 버릴 수 있어야 하고,
  // 그 길은 트리 카드의 우클릭과 같은 하나(onDeleteTask)여야 한다.
  it("최근 세션 행을 우클릭하면 버리기 항목이 뜨고 onDeleteTask로 그 작업을 넘긴다", async () => {
    const nowSec = Math.floor(Date.now() / 1000);
    const recent: Task = { ...makeTask(21, ALPHA, nowSec - 60), state: "AwaitingReview" };
    const deleted: number[] = [];
    await renderWithRecent(recent, (task) => deleted.push(task.id));

    await fire(recentRow(21), "contextmenu", { clientX: 20, clientY: 30 });

    const menu = document.body.querySelector<HTMLElement>('[role="menu"]');
    expect(menu?.textContent).toContain("버리기 (워크트리 정리)");

    await fire(item("버리기 (워크트리 정리)"), "click");

    expect(deleted).toEqual([21]);
    expect(document.body.querySelector('[role="menu"]')).toBeNull();
  });

  it("창 밖 작업만 있으면 최근 세션 섹션이 없고 트리는 그대로다", async () => {
    await renderWithRecent(makeTask(22, ALPHA, 10), () => {});

    expect(container.querySelector('section[aria-label="최근 세션"]')).toBeNull();
    expect(container.querySelector('section[aria-label="세션 작업"]')).not.toBeNull();
  });
});

describe("세션 우클릭 메뉴의 머지", () => {
  const renderMerge = async (task: Task, onMergeTask?: (task: Task) => void) => {
    await act(async () => {
      root.render(
        <SessionTaskNavigation
          tasks={[task]}
          selectedKey={null}
          projects={[ALPHA]}
          onOpenTask={() => {}}
          onNewInRepo={() => {}}
          onDeleteTask={() => {}}
          onMergeTask={onMergeTask}
          onRemoveProject={() => {}}
          onDiscardOrphans={() => {}}
        />,
      );
    });
  };
  // 최근 세션 행과 트리 카드는 같은 메뉴를 연다 — 좌표로 집을 수 있는 쪽을 쓴다.
  const recent = (id: number, patch: Partial<Task> = {}): Task => ({
    ...makeTask(id, ALPHA, Math.floor(Date.now() / 1000) - 60),
    ...patch,
  });
  const openMenu = async (id: number) => {
    const card = container.querySelector<HTMLElement>(
      `section[aria-label="최근 세션"] [data-task-key="local:${id}"]`,
    )!;
    await fire(card, "contextmenu", { clientX: 20, clientY: 30 });
    return document.body.querySelector<HTMLElement>('[role="menu"]');
  };

  it("검토 대기 워크트리 작업은 base 브랜치로 머지하는 항목을 띄우고 onMergeTask로 넘긴다", async () => {
    const merged: number[] = [];
    await renderMerge(recent(31, { state: "AwaitingReview" }), (task) => merged.push(task.id));

    const menu = await openMenu(31);
    expect(menu?.textContent).toContain("main에 머지");

    await fire(item("main에 머지"), "click");

    expect(merged).toEqual([31]);
    expect(document.body.querySelector('[role="menu"]')).toBeNull();
  });

  it("실행 중·직접 실행 작업이나 핸들러가 없으면 머지 항목이 없다", async () => {
    await renderMerge(recent(32), () => {});
    expect((await openMenu(32))?.textContent).not.toContain("에 머지");

    await renderMerge(recent(33, { state: "AwaitingReview", worktree_path: ALPHA }), () => {});
    expect((await openMenu(33))?.textContent).not.toContain("에 머지");

    await renderMerge(recent(34, { state: "AwaitingReview" }));
    expect((await openMenu(34))?.textContent).not.toContain("에 머지");
  });
});
