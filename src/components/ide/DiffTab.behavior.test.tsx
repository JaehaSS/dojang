// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DiffHunk } from "../../lib/ipc";

const hunk: DiffHunk = {
  id: "h1",
  path: "src/a.ts",
  old_range: [1, 2],
  new_range: [1, 2],
  protected: false,
  committed: false,
  risk: "low",
  lines: [
    { kind: "context", text: "const a = 1;" },
    { kind: "add", text: "const b = 2;" },
  ],
};

const mocks = vi.hoisted(() => ({
  annotationsList: vi.fn(async () => []),
  diffHunks: vi.fn(),
  taskDiff: vi.fn(),
  tauriEvents: new Map<string, (event: { payload: unknown }) => void>(),
}));

// 세션은 에이전트 리뷰 이벤트를 듣는다. 핸들러를 잡아 두고 테스트가 직접 발화한다.
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, handler: (event: { payload: unknown }) => void) => {
    mocks.tauriEvents.set(name, handler);
    return Promise.resolve(() => mocks.tauriEvents.delete(name));
  },
}));

vi.mock("../../lib/ipc", () => ({
  annotationSave: vi.fn(),
  annotationsList: mocks.annotationsList,
  annotationsResend: vi.fn(),
  diffHunks: mocks.diffHunks,
  partialApply: vi.fn(),
  partialRollback: vi.fn(),
  taskDiff: mocks.taskDiff,
}));

import { DiffSessionProvider } from "../DiffSessionContext";
import { DiffTab } from "./DiffTab";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** jsdom에는 ResizeObserver가 없다. 폭 전이는 관측 콜백을 손으로 발화시켜 검증한다. */
class ResizeObserverStub {
  static instances: ResizeObserverStub[] = [];
  constructor(private cb: ResizeObserverCallback) {
    ResizeObserverStub.instances.push(this);
  }
  observe() {}
  disconnect() {}
  emit(width: number) {
    this.cb([{ contentRect: { width } } as ResizeObserverEntry], this as never);
  }
}

const task = { host: "local", id: 9 };
const emit = async (width: number) => {
  await act(async () => {
    ResizeObserverStub.instances.forEach((observer) => observer.emit(width));
  });
};
const press = async (key: string) => {
  await act(async () => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
  });
};
const buttonWith = (label: string) =>
  [...(container?.querySelectorAll("button") ?? [])].find((b) => b.textContent === label);

let container: HTMLDivElement | null = null;
let root: Root | null = null;

const render = async (children: React.ReactNode) => {
  await act(async () => {
    root?.render(
      <DiffSessionProvider task={task} openDiff={() => {}}>
        {children}
      </DiffSessionProvider>,
    );
    await Promise.resolve();
  });
};

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", ResizeObserverStub);
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
    cb(0);
    return 0;
  });
  localStorage.clear();
  ResizeObserverStub.instances = [];
  mocks.diffHunks.mockResolvedValue([hunk]);
  mocks.taskDiff.mockResolvedValue({
    files: [{ path: "src/a.ts", status: "M", patch: "@@ -1 +1 @@\n+const b = 2;" }],
    baseline: { kind: "pinned" as const },
  });
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root?.unmount());
  container?.remove();
  container = null;
  root = null;
  vi.clearAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe("DiffTab 폭 강제 unified (AC-6)", () => {
  it("좁아지면 분할을 잠그고 저장 모드를 건드리지 않는다", async () => {
    localStorage.setItem("praxis:diff-mode", "split");
    await render(<DiffTab path="src/a.ts" active />);
    expect(buttonWith("분할")?.getAttribute("aria-pressed")).toBe("true");

    await emit(500);
    expect(buttonWith("통합")?.getAttribute("aria-pressed")).toBe("true");
    expect(buttonWith("분할")?.hasAttribute("disabled")).toBe(true);

    // 강제 구간의 S는 저장값을 바꾸지 않는다 — 다른 탭과 다음 세션의 기본값이 걸려 있다.
    await press("s");
    await act(async () => buttonWith("분할")?.click());
    expect(localStorage.getItem("praxis:diff-mode")).toBe("split");
    expect(buttonWith("통합")?.getAttribute("aria-pressed")).toBe("true");

    await emit(900);
    expect(buttonWith("분할")?.getAttribute("aria-pressed")).toBe("true");
  });
});

describe("DiffTab 사라진 파일 (AC-9)", () => {
  it("스냅샷에서 빠져도 탭을 닫지 않고 안내와 닫기 버튼을 보인다", async () => {
    const onClose = vi.fn();
    await render(<DiffTab path="src/gone.ts" active onClose={onClose} />);

    expect(container?.textContent).toContain("이 파일은 더 이상 바뀌지 않았습니다");
    await act(async () => buttonWith("탭 닫기")?.click());
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe("DiffTab 단축키 소유자 (AC-11)", () => {
  it("탭이 둘 마운트돼도 J는 활성 탭에서만 동작한다", async () => {
    const jump = vi.fn();
    Element.prototype.scrollIntoView = jump;
    await render(
      <>
        <DiffTab path="src/a.ts" active />
        <DiffTab path="src/a.ts" />
      </>,
    );
    expect(container?.querySelectorAll("[data-diff-tab]")).toHaveLength(2);

    await press("j");
    expect(jump).toHaveBeenCalledTimes(1);
  });
});

describe("DiffTab 세션 값 (DR-5)", () => {
  it("범위·기준점·↻·부분 적용 실행을 툴바에 두지 않는다", async () => {
    await render(<DiffTab path="src/a.ts" active />);

    expect(buttonWith("세션 전체")).toBeUndefined();
    expect(buttonWith("미커밋")).toBeUndefined();
    expect(container?.querySelector("[aria-label='Diff 새로고침']")).toBeNull();
    expect(container?.textContent).not.toContain("선택 적용");
  });
});

describe("DiffTab 단축키 (AC-5)", () => {
  it("U·S가 모드를 바꾸고 저장값에 남는다", async () => {
    await render(<DiffTab path="src/a.ts" active />);

    await press("s");
    expect(localStorage.getItem("praxis:diff-mode")).toBe("split");
    await press("u");
    expect(localStorage.getItem("praxis:diff-mode")).toBe("unified");
  });

  it("수식키 조합은 앱 전역 단축키에 넘긴다", async () => {
    await render(<DiffTab path="src/a.ts" active />);

    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "s", metaKey: true }));
    });
    expect(localStorage.getItem("praxis:diff-mode")).toBeNull();
  });

  it("V는 이 탭의 파일을 확인함으로 표시하고 작업별로 남는다", async () => {
    await render(<DiffTab path="src/a.ts" active />);

    await press("v");
    expect(localStorage.getItem("praxis:diff-viewed:9")).toContain("src/a.ts");
    await press("v");
    expect(localStorage.getItem("praxis:diff-viewed:9")).not.toContain("src/a.ts");
  });
});

describe("DiffTab 에이전트 이동 (review_navigate)", () => {
  const second: DiffHunk = { ...hunk, id: "h2", old_range: [40, 1], new_range: [41, 1] };
  const navigate = async (payload: unknown) => {
    await act(async () => {
      mocks.tauriEvents.get("review://navigate")?.({ payload });
      await Promise.resolve();
    });
  };

  it("이 작업의 이벤트면 그 파일 탭을 열고 목표 hunk 헤더로 스크롤한다", async () => {
    mocks.diffHunks.mockResolvedValue([hunk, second]);
    const scrolled: string[] = [];
    Element.prototype.scrollIntoView = function (this: HTMLElement) {
      scrolled.push(this.dataset.hunkId ?? "?");
    };
    const openDiff = vi.fn();
    await act(async () => {
      root?.render(
        <DiffSessionProvider task={task} openDiff={openDiff}>
          <DiffTab path="src/a.ts" active />
        </DiffSessionProvider>,
      );
      await Promise.resolve();
    });

    await navigate({ task_id: 9, path: "src/a.ts", hunk_id: "h2", line: 41, side: "new" });
    expect(openDiff).toHaveBeenCalledWith("src/a.ts");
    expect(scrolled).toEqual(["h2"]);

    // 같은 hunk를 다시 겨눠도 한 번 더 움직인다 — 소비한 목표는 지워지고 nonce가 새 값이다.
    await navigate({ task_id: 9, path: "src/a.ts", hunk_id: "h2", line: 41, side: "new" });
    expect(scrolled).toEqual(["h2", "h2"]);

    await navigate({ task_id: 10, path: "src/a.ts", hunk_id: "h1", line: 1, side: "new" });
    expect(scrolled).toEqual(["h2", "h2"]);
    expect(openDiff).toHaveBeenCalledTimes(2);
  });

  it("hunk id가 화면과 어긋나면 줄 번호로 되찾고, 못 찾으면 목표를 버린다", async () => {
    mocks.diffHunks.mockResolvedValue([hunk, second]);
    const scrolled: string[] = [];
    Element.prototype.scrollIntoView = function (this: HTMLElement) {
      scrolled.push(this.dataset.hunkId ?? "?");
    };
    await render(<DiffTab path="src/a.ts" active />);

    // 다른 범위에서 뜬 id지만 41번 줄은 두 번째 hunk 안이다.
    await navigate({ task_id: 9, path: "src/a.ts", hunk_id: "stale", line: 41, side: "new" });
    expect(scrolled).toEqual(["h2"]);

    // diff 밖 줄 — 스크롤하지 않고 버린다. 뒤에 범위를 바꿔도 튀지 않도록 hunk를 다시 그려 본다.
    await navigate({ task_id: 9, path: "src/a.ts", hunk_id: "stale", line: 500, side: "new" });
    expect(scrolled).toEqual(["h2"]);
    mocks.diffHunks.mockResolvedValue([{ ...second, id: "stale" }]);
    await act(async () => {
      window.dispatchEvent(new Event("focus"));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(scrolled).toEqual(["h2"]);
  });

  it("노트 이벤트는 폴링을 기다리지 않고 주석만 다시 읽는다", async () => {
    await render(<DiffTab path="src/a.ts" active />);
    const before = mocks.annotationsList.mock.calls.length;

    await act(async () => {
      mocks.tauriEvents.get("review://annotations")?.({ payload: { task_id: 9, path: "src/a.ts" } });
      await Promise.resolve();
    });
    expect(mocks.annotationsList.mock.calls.length).toBe(before + 1);
    expect(mocks.taskDiff).toHaveBeenCalledTimes(1);
  });

  it("새 hunk로 이동할 때 지연된 diff 갱신이 끝날 때까지 목표를 유지한다", async () => {
    const scrolled: string[] = [];
    Element.prototype.scrollIntoView = function (this: HTMLElement) {
      scrolled.push(this.dataset.hunkId ?? "?");
    };
    await render(<DiffTab path="src/a.ts" active />);

    const snapshot = {
      files: [{ path: "src/a.ts", status: "M", patch: "updated" }],
      baseline: { kind: "pinned" as const },
    };
    let complete!: (value: typeof snapshot) => void;
    mocks.taskDiff.mockReturnValueOnce(new Promise((resolve) => { complete = resolve; }));
    mocks.diffHunks.mockResolvedValue([hunk, second]);

    await navigate({ task_id: 9, path: "src/a.ts", hunk_id: "h2", line: 41, side: "new" });
    expect(scrolled).toEqual([]);
    await act(async () => { complete(snapshot); });
    expect(scrolled).toEqual(["h2"]);
  });

  it("갱신 중 이동 요청이 겹치면 마지막 목표만 소비한다", async () => {
    const scrolled: string[] = [];
    Element.prototype.scrollIntoView = function (this: HTMLElement) {
      scrolled.push(this.dataset.hunkId ?? "?");
    };
    await render(<DiffTab path="src/a.ts" active />);
    let complete!: () => void;
    mocks.taskDiff.mockReturnValueOnce(new Promise((resolve) => {
      complete = () => resolve({ files: [{ path: "src/a.ts", patch: "updated", status: "M" }], baseline: { kind: "pinned" } });
    }));
    mocks.diffHunks.mockResolvedValue([hunk, second]);

    await navigate({ task_id: 9, path: "src/a.ts", hunk_id: "h1", line: 1, side: "new" });
    await navigate({ task_id: 9, path: "src/a.ts", hunk_id: "h2", line: 41, side: "new" });
    expect(scrolled).toEqual([]);
    await act(async () => complete());
    expect(scrolled).toEqual(["h2"]);
  });

  it("만료 뒤에 갱신이 끝나도 이전 목표로 이동하지 않는다", async () => {
    vi.useFakeTimers();
    const scroll = vi.fn();
    Element.prototype.scrollIntoView = scroll;
    await render(<DiffTab path="src/a.ts" active />);
    let complete!: () => void;
    mocks.taskDiff.mockReturnValueOnce(new Promise((resolve) => {
      complete = () => resolve({ files: [{ path: "src/a.ts", patch: "updated", status: "M" }], baseline: { kind: "pinned" } });
    }));
    mocks.diffHunks.mockResolvedValue([hunk, second]);

    await navigate({ task_id: 9, path: "src/a.ts", hunk_id: "h2", line: 41, side: "new" });
    await act(async () => { vi.advanceTimersByTime(15_000); });
    await act(async () => complete());
    expect(scroll).not.toHaveBeenCalled();
  });

  it("이동 전에 시작된 조회가 있으면 그 응답 뒤에 새 스냅샷을 요청한다", async () => {
    const scrolled: string[] = [];
    Element.prototype.scrollIntoView = function (this: HTMLElement) {
      scrolled.push(this.dataset.hunkId ?? "?");
    };
    await render(<DiffTab path="src/a.ts" active />);
    let complete!: () => void;
    mocks.taskDiff.mockReturnValueOnce(new Promise((resolve) => {
      complete = () => resolve({
        files: [{ path: "src/a.ts", patch: "old", status: "M" }],
        baseline: { kind: "pinned" },
        review: { hunks: [hunk], annotations: [] },
      });
    }));
    // 이 요청의 스냅샷은 에이전트가 새 hunk를 만들기 전에 준비됐다.
    await act(async () => { window.dispatchEvent(new Event("focus")); });
    mocks.diffHunks.mockResolvedValue([hunk, second]);

    await navigate({ task_id: 9, path: "src/a.ts", hunk_id: "h2", line: 41, side: "new" });
    await act(async () => complete());
    expect(scrolled).toEqual(["h2"]);
  });
});
