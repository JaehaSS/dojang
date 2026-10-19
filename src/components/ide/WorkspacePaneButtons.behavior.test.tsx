// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { WorkspacePaneButtons } from "./WorkspacePaneButtons";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const onOpen = vi.fn();
const onClose = vi.fn();
const onTogglePopOut = vi.fn();
const onToggleTerminal = vi.fn();
const onToggleChanges = vi.fn();

type Overrides = Partial<Parameters<typeof WorkspacePaneButtons>[0]>;

let host: HTMLDivElement;
let root: Root | null = null;

const render = async (props: Overrides = {}) => {
  await act(async () => {
    root?.render(
      <WorkspacePaneButtons
        open={false}
        active="file"
        changes={{ on: false, count: null, onToggle: onToggleChanges }}
        previewAvailable
        onOpen={onOpen}
        onClose={onClose}
        popOut={{ poppedOut: false, onToggle: onTogglePopOut }}
        terminal={{ available: true, open: false, onToggle: onToggleTerminal }}
        showLabels
        {...props}
      />,
    );
  });
};

/** aria-label은 상태에 따라 "…보기"/"…닫기"로 갈리므로 접두사로 찾는다. */
const button = (label: string): HTMLButtonElement | undefined =>
  [...host.querySelectorAll("button")].find((b) =>
    (b.getAttribute("aria-label") ?? "").startsWith(label),
  ) as HTMLButtonElement | undefined;

const click = async (label: string) => {
  await act(async () => {
    button(label)?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
};

beforeEach(() => {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  onOpen.mockClear();
  onClose.mockClear();
  onTogglePopOut.mockClear();
  onToggleTerminal.mockClear();
  onToggleChanges.mockClear();
});

afterEach(async () => {
  await act(async () => root?.unmount());
  host.remove();
  root = null;
});

describe("WorkspacePaneButtons", () => {
  it("로컬 작업에서 Design을 열 수 있고 원격에서는 버튼을 숨긴다", async () => {
    await render({ designAvailable: true });
    await click("Design");
    expect(onOpen).toHaveBeenCalledWith("design");
    await render({ designAvailable: true, previewAvailable: false });
    expect(button("Design")).toBeUndefined();
  });
  it("파일·프리뷰·변경·에디터 창·터미널을 각각의 버튼으로 낸다", async () => {
    await render();
    expect(button("변경")).toBeTruthy();
    expect(button("파일 보기")).toBeTruthy();
    expect(button("프리뷰")).toBeTruthy();
    expect(button("에디터를 새 창으로")).toBeTruthy();
    expect(button("터미널")).toBeTruthy();
    // 트리 칩은 헤더에서 뺐다 — "파일" 칩이 트리까지 연다(ADR 0188).
    expect(button("파일 트리")).toBeUndefined();
  });

  it("변경 칩은 코드 열 탭이 아니라 트리 렌즈 손잡이다 — 누르면 onToggle만 부른다", async () => {
    await render({ open: true, active: "file" });
    await click("변경");
    expect(onToggleChanges).toHaveBeenCalledTimes(1);
    expect(onOpen).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("변경 렌즈가 켜져 있으면 변경 칩만 눌림이다", async () => {
    await render({ open: false, active: "file", changes: { on: true, count: 2, onToggle: onToggleChanges } });
    expect(button("변경")?.getAttribute("aria-pressed")).toBe("true");
    expect(button("파일 보기")?.getAttribute("aria-pressed")).toBe("false");
  });

  it("닫힌 열에서 버튼을 누르면 그 탭으로 연다", async () => {
    await render({ open: false, active: "file" });
    await click("프리뷰");
    expect(onOpen).toHaveBeenCalledWith("preview");
    expect(onClose).not.toHaveBeenCalled();
  });

  it("변경 파일 수를 변경 배지로 보인다", async () => {
    await render({ changes: { on: false, count: 4, onToggle: onToggleChanges } });
    expect(button("변경")?.textContent).toContain("4");
  });

  it("열려 있어도 다른 탭 버튼은 닫지 않고 그 탭으로 옮긴다", async () => {
    await render({ open: true, active: "file" });
    await click("프리뷰");
    expect(onOpen).toHaveBeenCalledWith("preview");
    expect(onClose).not.toHaveBeenCalled();
  });

  it("보고 있던 탭을 다시 누르면 닫는다", async () => {
    await render({ open: true, active: "preview" });
    await click("프리뷰");
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onOpen).not.toHaveBeenCalled();
  });

  it("보고 있는 탭만 눌림 상태로 표시한다", async () => {
    await render({ open: true, active: "preview" });
    expect(button("프리뷰")?.getAttribute("aria-pressed")).toBe("true");
    expect(button("파일 보기")?.getAttribute("aria-pressed")).toBe("false");
  });

  it("열이 닫혀 있으면 활성 탭이라도 눌림이 아니다", async () => {
    await render({ open: false, active: "preview" });
    expect(button("프리뷰")?.getAttribute("aria-pressed")).toBe("false");
  });

  it("원격 작업에서는 프리뷰 버튼을 만들지 않는다", async () => {
    await render({ previewAvailable: false });
    expect(button("프리뷰")).toBeUndefined();
    expect(button("파일 보기")).toBeTruthy();
  });

  it("에디터가 팝아웃되어 있으면 파일 버튼을 만들지 않는다", async () => {
    // 팝아웃 여부는 `popOut.poppedOut` 하나로만 들어온다 — 손잡이와 필터가 어긋날 자리가 없다.
    await render({ popOut: { poppedOut: true, onToggle: onTogglePopOut } });
    expect(button("파일 보기")).toBeUndefined();
    expect(button("코드 창 앞으로")).toBeTruthy();
  });

  it("팝아웃 손잡이는 코드 열이 닫혀 있어도 보이고, 누르면 빼낸다", async () => {
    await render({ open: false });
    const popOut = button("에디터를 새 창으로");
    expect(popOut).toBeTruthy();
    expect(popOut?.getAttribute("aria-pressed")).toBe("false");
    await click("에디터를 새 창으로");
    expect(onTogglePopOut).toHaveBeenCalledTimes(1);
    expect(onOpen).not.toHaveBeenCalled();
  });

  it("팝아웃 칩은 라벨을 병기하고 좁으면 접는다", async () => {
    await render();
    expect(button("에디터를 새 창으로")?.textContent).toContain("에디터 창");
    await render({ showLabels: false });
    expect(button("에디터를 새 창으로")?.textContent).not.toContain("에디터 창");
  });

  it("팝아웃 칩은 코드 열 묶음 안 변경 바로 다음이다", async () => {
    await render();
    expect(button("변경")?.nextElementSibling).toBe(button("에디터를 새 창으로"));
  });

  it("나가 있으면 눌림 상태로 그 창을 앞으로 부른다", async () => {
    await render({ popOut: { poppedOut: true, onToggle: onTogglePopOut } });
    const popOut = button("코드 창 앞으로");
    expect(popOut?.getAttribute("aria-pressed")).toBe("true");
    await click("코드 창 앞으로");
    expect(onTogglePopOut).toHaveBeenCalledTimes(1);
  });

  it("터미널은 코드 열과 따로 토글한다", async () => {
    await render({
      open: true,
      active: "file",
      terminal: { available: true, open: false, onToggle: onToggleTerminal },
    });
    await click("터미널");
    expect(onToggleTerminal).toHaveBeenCalledTimes(1);
    expect(onOpen).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("터미널 칩은 열림 상태를 눌림으로 보이고 (⌃`)를 일러 준다", async () => {
    await render({ terminal: { available: true, open: false, onToggle: onToggleTerminal } });
    expect(button("터미널")?.getAttribute("aria-label")).toBe("터미널 열기");
    expect(button("터미널")?.title).toBe("터미널 열기 (⌃`)");
    await click("터미널");
    expect(onToggleTerminal).toHaveBeenCalledTimes(1);

    await render({ terminal: { available: true, open: true, onToggle: onToggleTerminal } });
    expect(button("터미널")?.getAttribute("aria-label")).toBe("터미널 닫기");
    expect(button("터미널")?.getAttribute("aria-pressed")).toBe("true");
  });

  it("원격 작업에서는 터미널 칩을 사유와 함께 비활성화하고 토글하지 않는다", async () => {
    await render({
      terminal: {
        available: false,
        open: false,
        reason: "로컬 세션에서만 쓸 수 있습니다",
        onToggle: onToggleTerminal,
      },
    });
    const chip = button("터미널 사용 불가");
    expect(chip?.disabled).toBe(true);
    expect(chip?.title).toBe("로컬 세션에서만 쓸 수 있습니다");
    await click("터미널 사용 불가");
    expect(onToggleTerminal).not.toHaveBeenCalled();
  });

  it("라벨 모드에서는 목적지 이름을 그대로 보여준다", async () => {
    await render({ showLabels: true });
    expect(button("파일 보기")?.textContent).toContain("파일");
    expect(button("변경")?.textContent).toContain("변경");
    expect(button("프리뷰")?.textContent).toContain("프리뷰");
    expect(button("터미널")?.textContent).toContain("터미널");
  });

  it("좁으면 라벨을 접고 아이콘만 남긴다 — 버튼과 그 이름표는 사라지지 않는다", async () => {
    await render({ showLabels: false });
    for (const label of ["파일 보기", "프리뷰", "터미널"]) {
      // 손잡이 자체는 남는다. 눌림 상태와 스크린리더용 이름도 그대로다.
      expect(button(label)).toBeTruthy();
      expect(button(label)?.textContent).toBe("");
    }
  });

  it("라벨을 접어도 누르면 같은 곳으로 간다", async () => {
    await render({ showLabels: false, open: false, active: "file" });
    await click("프리뷰");
    expect(onOpen).toHaveBeenCalledWith("preview");
  });
});
