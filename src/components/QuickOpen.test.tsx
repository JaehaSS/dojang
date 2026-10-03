// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  projectSearch: vi.fn(),
  quickopenSearch: vi.fn(async () => []),
  skillsList: vi.fn(async () => []),
  /** 랭킹 호출 관찰용 — 실제 구현으로 위임한다. */
  merge: vi.fn(),
}));

vi.mock("../lib/host-scope", () => ({ useHostScope: () => "local" }));
vi.mock("../lib/ipc", () => mocks);
vi.mock("./ide/icons", () => ({ Icon: () => null }));
vi.mock("../lib/quickopen", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/quickopen")>();
  return {
    ...actual,
    mergeQuickOpenResults: (...args: Parameters<typeof actual.mergeQuickOpenResults>) => {
      mocks.merge(...args);
      return actual.mergeQuickOpenResults(...args);
    },
  };
});

import { QuickOpen } from "./QuickOpen";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
const close = vi.fn();
const select = vi.fn();

/** 앱과 같이 참조가 안정된 목록 — 렌더마다 새 배열이면 메모가 무효화돼 랭킹 재계산 검사가 의미를 잃는다. */
const FILES = ["src/alpha.ts", "src/beta.ts"];
const SCOPES = ["file", "code"] as const;

function render(open = true, contentAvailable = true, taskId: number | null = 42, initialEditorTab?: "all" | "file" | "code") {
  root.render(
    <QuickOpen
      open={open}
      editorSearch={{ scopeLabel: "feature/search · local · 세션 #42", contentAvailable }}
      initialEditorTab={initialEditorTab}
      scopes={[...SCOPES]}
      files={FILES}
      repo=""
      taskId={taskId}
      onClose={close}
      onSelect={select}
    />,
  );
}

const key = (target: HTMLElement, value: string, shiftKey = false) =>
  target.dispatchEvent(new KeyboardEvent("keydown", { key: value, shiftKey, bubbles: true }));

const search = async (value: string) => {
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
    setter?.call(host.querySelector("input"), value);
    host.querySelector("input")?.dispatchEvent(new Event("input", { bubbles: true }));
  });
};

const flushSearch = async () => {
  await act(async () => vi.advanceTimersByTime(200));
};

const result = (text: string) => ({ matches: [{ path: "src/a.ts", line: 1, column: 1, text }], truncated: false });

const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  return { promise: new Promise<T>((done) => { resolve = done; }), resolve };
};

describe("QuickOpen editor search", () => {
  beforeEach(() => {
    HTMLElement.prototype.scrollIntoView = vi.fn();
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
    close.mockClear();
    select.mockClear();
    mocks.projectSearch.mockReset();
    mocks.merge.mockClear();
    vi.useFakeTimers();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
    vi.useRealTimers();
  });

  it("Tab과 Shift+Tab으로 전체·파일·내용 범위를 바꾸고 Esc 뒤 편집 포커스를 돌려준다", async () => {
    const editor = document.createElement("textarea");
    document.body.append(editor);
    editor.focus();
    await act(async () => render());
    const input = host.querySelector("input") as HTMLInputElement;

    await act(async () => key(input, "Tab"));
    expect(host.querySelector('[role="tab"][aria-selected="true"]')?.textContent).toBe("파일");
    await act(async () => key(input, "Tab", true));
    expect(host.querySelector('[role="tab"][aria-selected="true"]')?.textContent).toBe("전체");
    await act(async () => key(input, "Escape"));
    await act(async () => render(false));
    expect(document.activeElement).toBe(editor);
    editor.remove();
  });

  it("시작 범위를 받으면 그 탭으로 열리고, 다시 열 때도 그 범위로 돌아온다", async () => {
    await act(async () => render(true, true, 42, "file"));
    expect(host.querySelector('[role="tab"][aria-selected="true"]')?.textContent).toBe("파일");
    const input = host.querySelector("input") as HTMLInputElement;
    await act(async () => key(input, "Tab"));
    expect(host.querySelector('[role="tab"][aria-selected="true"]')?.textContent).toBe("내용");

    await act(async () => render(false, true, 42, "file"));
    await act(async () => render(true, true, 42, "file"));
    expect(host.querySelector('[role="tab"][aria-selected="true"]')?.textContent).toBe("파일");
  });

  it("원격 창은 내용 탭을 막고 로컬 검색 IPC를 부르지 않는다", async () => {
    await act(async () => render());
    const input = host.querySelector("input") as HTMLInputElement;
    await act(async () => key(input, "Tab"));
    await act(async () => key(input, "Tab"));
    expect(host.querySelector('[role="tab"][aria-selected="true"]')?.textContent).toBe("내용");
    await act(async () => render(true, false));
    expect(host.textContent).toContain("내용 검색은 이 원격 워크트리에서 사용할 수 없습니다");
    expect(host.querySelector('[role="tab"][aria-selected="true"]')?.textContent).toBe("전체");
    expect(mocks.projectSearch).not.toHaveBeenCalled();
  });

  it("빈 결과에서 Enter와 Tab은 선택 콜백을 부르지 않는다", async () => {
    await act(async () => render());
    const input = host.querySelector("input") as HTMLInputElement;

    await act(async () => key(input, "Tab"));
    await act(async () => key(input, "Tab"));
    await act(async () => key(input, "Enter"));
    expect(select).not.toHaveBeenCalled();
  });

  it("같은 질의로 돌아오면 새 요청을 내고 이전 결과를 재사용하지 않는다", async () => {
    mocks.projectSearch
      .mockResolvedValueOnce(result("alpha first"))
      .mockResolvedValueOnce(result("alpha final"));
    await act(async () => render());
    await search("alpha");
    await flushSearch();
    expect(host.textContent).toContain("alpha first");
    await search("alphax");
    expect(host.textContent).not.toContain("alpha first");
    await search("alpha");
    await flushSearch();

    expect(mocks.projectSearch.mock.calls.map((call) => call[1])).toEqual(["alpha", "alpha"]);
    expect(host.textContent).toContain("alpha final");
  });

  it("작업이 바뀌면 이전 내용 결과를 표시하지 않는다", async () => {
    const old = deferred<ReturnType<typeof result>>();
    const next = deferred<ReturnType<typeof result>>();
    mocks.projectSearch.mockReturnValueOnce(old.promise).mockReturnValueOnce(next.promise);
    await act(async () => render());
    await search("alpha");
    await flushSearch();
    await act(async () => render(true, true, 43));
    await act(async () => old.resolve(result("alpha old result")));

    expect(mocks.projectSearch).toHaveBeenLastCalledWith(43, "alpha");
    expect(host.textContent).not.toContain("old result");
    await act(async () => {
      next.resolve(result("alpha new result"));
      await Promise.resolve();
    });
    expect(host.textContent).toContain("new result");
  });

  it("원격 작업은 질의가 있어도 로컬 내용 검색을 호출하지 않는다", async () => {
    const local = deferred<ReturnType<typeof result>>();
    mocks.projectSearch.mockReturnValue(local.promise);
    await act(async () => render());
    await search("alpha");
    await flushSearch();
    expect(mocks.projectSearch).toHaveBeenCalledWith(42, "alpha");
    await act(async () => render(true, false, null));
    await act(async () => local.resolve(result("alpha local result")));

    expect(mocks.projectSearch).toHaveBeenCalledTimes(1);
    expect(host.textContent).not.toContain("local result");
  });

  it("닫혀 있는 동안은 부모가 다시 렌더돼도 랭킹을 계산하지 않는다", async () => {
    // 원장 #448 — 큰 원격 저장소에서 팔레트가 닫힌 채 렌더마다 파일 전체를 정렬해 앱이 멈췄다.
    await act(async () => render(false));
    await act(async () => render(false, true, 43));
    expect(mocks.merge).not.toHaveBeenCalled();

    await act(async () => render(true, true, 43));
    expect(mocks.merge).toHaveBeenCalled();
    // 열리면서 시작된 세션·스킬 조회가 끝나 상태가 잠잠해진 뒤의 호출 수를 기준으로 잡는다.
    await act(async () => {
      await Promise.resolve();
    });
    const calls = mocks.merge.mock.calls.length;
    await act(async () => render(true, true, 43));
    expect(mocks.merge).toHaveBeenCalledTimes(calls);
  });
});
