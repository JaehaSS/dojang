// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DiffHunk } from "../../lib/ipc";

const hunks: DiffHunk[] = [
  {
    id: "h1",
    path: "src/components/ide/Composer.tsx",
    old_range: [1, 2],
    new_range: [1, 2],
    protected: false,
    committed: false,
    risk: "low",
    lines: [{ kind: "add", text: "const a = 1;" }],
  },
  {
    id: "h2",
    path: "README.md",
    old_range: [1, 1],
    new_range: [1, 1],
    protected: false,
    committed: false,
    risk: "low",
    lines: [{ kind: "add", text: "hello" }],
  },
];

const files = [
  { path: "src/components/ide/Composer.tsx", status: "M", patch: "@@ -1 +1,3 @@\n-old\n+a\n+b\n+c" },
  { path: "README.md", status: "A", patch: "@@ -0,0 +1 @@\n+hello" },
];

const mocks = vi.hoisted(() => ({
  annotationsList: vi.fn(async () => []),
  diffHunks: vi.fn(),
  partialApply: vi.fn(),
  taskDiff: vi.fn(),
}));

vi.mock("../../lib/ipc", () => ({
  annotationSave: vi.fn(),
  annotationsList: mocks.annotationsList,
  annotationsResend: vi.fn(),
  diffHunks: mocks.diffHunks,
  partialApply: mocks.partialApply,
  partialRollback: vi.fn(),
  taskDiff: mocks.taskDiff,
}));

import { DiffSessionProvider } from "../DiffSessionContext";
import { ChangesList } from "./ChangesList";
import { DiffTab } from "./DiffTab";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const task = { host: "local", id: 7 };
const openDiff = vi.fn();

let container: HTMLDivElement | null = null;
let root: Root | null = null;

const render = async (extra?: React.ReactNode) => {
  await act(async () => {
    root?.render(
      <DiffSessionProvider task={task} openDiff={openDiff}>
        <ChangesList />
        {extra}
      </DiffSessionProvider>,
    );
    await Promise.resolve();
  });
};

const rows = () => [...(container?.querySelectorAll("button[title]") ?? [])];
const rowFor = (path: string) =>
  rows().find((row) => row.getAttribute("title") === path) as HTMLButtonElement | undefined;
const buttonWith = (label: string) =>
  [...(container?.querySelectorAll("button") ?? [])].find((b) => b.textContent === label);

beforeEach(() => {
  vi.useFakeTimers();
  // diff 탭과 함께 세우는 케이스가 있다 — jsdom에는 ResizeObserver가 없다.
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
  localStorage.clear();
  mocks.diffHunks.mockResolvedValue(hunks);
  mocks.taskDiff.mockResolvedValue({ files, baseline: { kind: "pinned" as const } });
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

describe("ChangesList 행", () => {
  it("파일별 증감을 파일을 열기 전에 보여준다", async () => {
    await render();
    expect(container?.textContent).toContain("+3");
    expect(container?.textContent).toContain("−1");
  });

  it("파일명을 디렉터리보다 앞에 두고 전체 경로는 툴팁으로 남긴다", async () => {
    await render();
    const row = rowFor("src/components/ide/Composer.tsx");
    expect(row?.textContent?.indexOf("Composer.tsx")).toBeLessThan(
      row?.textContent?.indexOf("src/components/ide") ?? -1,
    );
  });

  it("확인 진행률을 현재 패치 기준으로 센다", async () => {
    await render();
    expect(container?.textContent).toContain("변경 2 · 확인 0/2");

    const check = container?.querySelector<HTMLInputElement>(
      'input[aria-label="README.md 확인함"]',
    );
    await act(async () => check?.click());
    expect(container?.textContent).toContain("확인 1/2");
  });

  it("files 없는 응답이 와도 창을 죽이지 않고 골격을 유지한다", async () => {
    // 구버전 원격 Runner가 배열을 돌려주던 시절의 잔재 — `files`가 undefined인 채로 여기까지 오면
    // `.length`에서 터져 창 전체가 사라졌다(SSH 터널 crash). 모르는 상태는 로딩과 같이 다룬다.
    mocks.taskDiff.mockResolvedValue({ baseline: { kind: "legacy" as const } });
    await render();
    expect(rowFor("README.md")).toBeUndefined();
    expect(container?.querySelector('div[aria-hidden="true"]')).not.toBeNull();
    expect(container?.textContent).not.toContain("변경 없음");
  });

  it("체크박스를 선택 버튼 안에 넣지 않는다 — 클릭이 새면 파일이 바뀐다", async () => {
    await render();
    expect(rowFor("README.md")?.querySelector("input")).toBeNull();
  });
});

describe("ChangesList 행 클릭 (AC-3)", () => {
  it("한 번 클릭은 세션의 diff 선택을 연다", async () => {
    await render();
    await act(async () => rowFor("README.md")?.click());
    expect(openDiff).toHaveBeenCalledWith("README.md", { preview: true });

  });
});

describe("ChangesList 헤더 (DR-5 · DR-10)", () => {
  it("범위를 바꾸면 diff·hunk·주석을 같은 범위로 다시 부른다", async () => {
    await render();
    mocks.taskDiff.mockClear();
    mocks.diffHunks.mockClear();
    mocks.annotationsList.mockClear();

    await act(async () => buttonWith("미커밋")?.click());

    expect(mocks.taskDiff).toHaveBeenCalledWith(task, "uncommitted", true);
    expect(mocks.diffHunks).toHaveBeenCalledWith(task, "uncommitted");
    expect(mocks.annotationsList).toHaveBeenCalledWith(task, "uncommitted");
  });

  it("비어 있던 목록이 폴링으로 채워진다", async () => {
    mocks.taskDiff
      .mockResolvedValueOnce({ files: [], baseline: { kind: "pinned" as const } })
      .mockResolvedValue({ files, baseline: { kind: "pinned" as const } });
    await render();
    expect(container?.textContent).toContain("변경 없음");

    await act(async () => vi.advanceTimersByTimeAsync(5_000));
    expect(container?.textContent).toContain("Composer.tsx");
  });
});

describe("ChangesList 부분 적용 (AC-12)", () => {
  it("확인 단계가 영향 파일명을 나열하고, 적용은 목록에서만 실행된다", async () => {
    mocks.partialApply.mockResolvedValue({ kept_hunk_ids: ["h1"], discarded_hunk_ids: ["h2"] });
    // hunk 체크박스는 탭 본문에 있고 적용 버튼은 목록에 있다 — 둘을 한 세션에 세워야
    // "보이지 않는 파일의 변경을 버린다"는 상황이 재현된다(설계 DR-5).
    await render(<DiffTab path="README.md" active />);
    expect(container?.textContent).not.toContain("버림");

    const boxes = [...(container?.querySelectorAll('input[type="checkbox"]') ?? [])];
    const hunkBox = boxes[boxes.length - 1] as HTMLInputElement;
    await act(async () => hunkBox.click());
    expect(container?.textContent).toContain("hunk 1개 버림 · 파일 1개");

    await act(async () => buttonWith("선택 적용 (1 hunks)")?.click());
    const confirm = container?.textContent ?? "";
    expect(confirm).toContain("hunk 1개를 버립니다");
    expect(confirm).toContain("README.md");

    await act(async () => buttonWith("적용")?.click());
    expect(mocks.partialApply).toHaveBeenCalledWith(task, ["h1"]);
  });
});
