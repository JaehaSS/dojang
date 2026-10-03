// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./icons", () => ({ Icon: () => null }));

import { BranchPicker, branchMatchRanges, filterBranches } from "./BranchPicker";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement | null = null;
let root: Root | null = null;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root?.unmount());
  container?.remove();
  root = null;
  container = null;
});

const MANY = [
  "feature/JH2-95-editor-popout-tree",
  "feature/JH2-88-popout-preview",
  "fix/popout-height-regression",
  "main",
  "dev",
];

async function open(
  branches: string[] = MANY,
  onPick: (b: string) => void = () => undefined,
  value = "",
  direct = false,
): Promise<void> {
  await act(async () => {
    root?.render(
      <BranchPicker value={value} current="main" branches={branches} onPick={onPick} direct={direct} />,
    );
  });
  await act(async () => chip()?.click());
}

const chip = (): HTMLButtonElement | null =>
  container?.querySelector<HTMLButtonElement>("button[aria-haspopup='listbox']") ?? null;
const search = (): HTMLInputElement | null =>
  container?.querySelector<HTMLInputElement>("input[role='combobox']") ?? null;
const options = (): HTMLElement[] => [
  ...(container?.querySelectorAll<HTMLElement>("[role='option']") ?? []),
];

/** React가 value setter를 가로채므로 native setter로 넣고 input 이벤트를 올린다. */
async function type(text: string): Promise<void> {
  const input = search();
  if (!input) throw new Error("검색 필드가 없다");
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  setter?.call(input, text);
  await act(async () => {
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function press(key: string): Promise<void> {
  await act(async () => {
    search()?.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
  });
}

describe("filterBranches", () => {
  it("공백으로 나눈 토큰을 모두 포함하는 브랜치만 남긴다", () => {
    expect(filterBranches(MANY, "jh2 tree")).toEqual(["feature/JH2-95-editor-popout-tree"]);
  });

  it("대소문자를 가리지 않는다", () => {
    expect(filterBranches(MANY, "JH2-88")).toEqual(["feature/JH2-88-popout-preview"]);
  });

  it("원래 순서(최근 커밋순)를 재정렬하지 않는다", () => {
    expect(filterBranches(MANY, "popout")).toEqual([
      "feature/JH2-95-editor-popout-tree",
      "feature/JH2-88-popout-preview",
      "fix/popout-height-regression",
    ]);
  });

  it("질의가 비면 전부 그대로 돌려준다 — 조용히 자르지 않는다", () => {
    const big = Array.from({ length: 400 }, (_, i) => `feature/b-${i}`);
    expect(filterBranches(big, "   ")).toHaveLength(400);
    expect(filterBranches(big, "b-")).toHaveLength(400);
  });
});

describe("branchMatchRanges", () => {
  it("토큰마다 첫 매치만 잡는다", () => {
    expect(branchMatchRanges("a-popout-popout", ["popout"])).toEqual([[2, 8]]);
  });

  it("겹치는 구간을 하나로 합친다", () => {
    expect(branchMatchRanges("popout", ["pop", "opo"])).toEqual([[0, 4]]);
  });
});

describe("BranchPicker 검색", () => {
  it("직접 실행에서 다른 브랜치를 고르면 생성 전에 체크아웃한다고 알린다", async () => {
    await open(MANY, () => undefined, "dev", true);
    expect(chip()?.title).toContain("작업 생성 전에");
    expect(chip()?.title).toContain("체크아웃");
    expect(chip()?.title).not.toContain("승인");
  });

  it("브랜치가 하나뿐이어도 검색 필드는 같은 자리를 지킨다", async () => {
    await open(["main"]);
    expect(search()).not.toBeNull();
  });

  it("좁혀진 정도를 표시/전체 카운트로 보여준다", async () => {
    await open();
    expect(container?.textContent).toContain("5");
    await type("popout");
    expect(container?.textContent).toContain("3/5");
  });

  it("질의에 맞는 항목만 남긴다", async () => {
    await open();
    await type("jh2 tree");
    expect(options().map((o) => o.getAttribute("title"))).toEqual([
      "feature/JH2-95-editor-popout-tree",
    ]);
  });

  it("↓와 Enter로 커서가 짚은 브랜치를 확정한다", async () => {
    const onPick = vi.fn();
    await open(MANY, onPick);
    await type("popout");
    await press("ArrowDown");
    await press("Enter");
    expect(onPick).toHaveBeenCalledWith("feature/JH2-88-popout-preview");
    expect(search()).toBeNull(); // 확정하면 닫힌다
  });

  it("커서는 목록 끝에서 순환하지 않는다", async () => {
    const onPick = vi.fn();
    await open(MANY, onPick);
    await type("jh2 tree");
    await press("ArrowDown");
    await press("ArrowDown");
    await press("Enter");
    expect(onPick).toHaveBeenCalledWith("feature/JH2-95-editor-popout-tree");
  });

  it("Esc는 질의를 비우는 단계 없이 곧바로 닫는다", async () => {
    await open();
    await type("popout");
    await press("Escape");
    expect(search()).toBeNull();
  });

  it("매치가 0건이면 왜 없는지와 다음 단서를 준다", async () => {
    await open();
    await type("없는브랜치");
    expect(options()).toHaveLength(0);
    expect(container?.textContent).toContain("일치하는 로컬 브랜치가 없습니다");
    expect(container?.textContent).toContain("원격 추적 브랜치");
  });

  it("origin/ 질의는 원격 브랜치가 base가 될 수 없다는 사실을 먼저 말한다", async () => {
    await open();
    await type("origin/dev");
    const text = container?.textContent ?? "";
    expect(text.indexOf("원격 추적 브랜치")).toBeLessThan(text.indexOf("일치하는 로컬 브랜치"));
  });

  it("Enter는 매치가 없을 때 아무것도 고르지 않는다", async () => {
    const onPick = vi.fn();
    await open(MANY, onPick);
    await type("없는브랜치");
    await press("Enter");
    expect(onPick).not.toHaveBeenCalled();
    expect(search()).not.toBeNull();
  });

  it("질의를 친 뒤 곧바로 Enter — 열 때 맞춘 커서로 되돌리지 않는다", async () => {
    const onPick = vi.fn();
    // 선택 브랜치 "dev"는 목록 끝(4)이다. 질의가 바뀔 때마다 커서를 그 자리로 되맞추는
    // 정책이었다면 커서가 매치 3건의 범위 밖에 남아 Enter가 무동작이 된다 — 브랜치는
    // `resetOn: "open"`이라 그렇게 되돌리지 않는다(설계 0062 §4.2 D-2).
    await open(MANY, onPick, "dev");
    await type("popout");
    await press("Enter");
    expect(onPick).toHaveBeenCalledWith("feature/JH2-95-editor-popout-tree");
  });

  it("다시 열면 지난 질의가 남아 있지 않다", async () => {
    await open();
    await type("popout");
    await press("Escape");
    await act(async () => chip()?.click());
    expect(search()?.value).toBe("");
    expect(options()).toHaveLength(MANY.length);
  });
});

describe("BranchPicker 체크아웃", () => {
  async function openWithCheckout(onCheckout: (b: string) => void, onPick = vi.fn()): Promise<void> {
    await act(async () => {
      root?.render(
        <BranchPicker value="" current="main" branches={MANY} onPick={onPick} onCheckout={onCheckout} />,
      );
    });
    await act(async () => chip()?.click());
  }
  const checkoutButton = (): HTMLButtonElement | null =>
    [...(container?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find(
      (b) => b.textContent === "체크아웃",
    ) ?? null;

  it("커서 행의 체크아웃 버튼은 고르지 않고 전환만 요청한다", async () => {
    const onCheckout = vi.fn();
    const onPick = vi.fn();
    await openWithCheckout(onCheckout, onPick);
    await press("ArrowDown"); // main(3) → dev(4)
    await act(async () => checkoutButton()?.click());
    expect(onCheckout).toHaveBeenCalledWith("dev");
    expect(onPick).not.toHaveBeenCalled();
    expect(search()).toBeNull();
  });

  it("이미 체크아웃된 브랜치에는 체크아웃 버튼이 없다", async () => {
    const onCheckout = vi.fn();
    await openWithCheckout(onCheckout);
    // 열면 커서가 현재 브랜치 main 위에 선다.
    expect(checkoutButton()).toBeNull();
    await act(async () => {
      search()?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, bubbles: true }));
    });
    expect(onCheckout).not.toHaveBeenCalled();
  });

  it("Shift+Enter는 커서 행으로 체크아웃한다", async () => {
    const onCheckout = vi.fn();
    const onPick = vi.fn();
    await openWithCheckout(onCheckout, onPick);
    await type("popout height");
    await act(async () => {
      search()?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, bubbles: true }));
    });
    expect(onCheckout).toHaveBeenCalledWith("fix/popout-height-regression");
    expect(onPick).not.toHaveBeenCalled();
  });

  it("onCheckout이 없으면 체크아웃 버튼을 두지 않는다", async () => {
    await open();
    await press("ArrowDown");
    expect(checkoutButton()).toBeNull();
  });
});
