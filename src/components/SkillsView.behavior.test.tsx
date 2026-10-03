// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  list: vi.fn(),
  read: vi.fn(),
}));

vi.mock("../lib/ipc", () => ({
  skillsList: mocks.list,
  skillsRead: mocks.read,
}));

import { SkillsView } from "./SkillsView";
import { resetExperimentalFeaturesForTest } from "../lib/experimental-features";

let container: HTMLDivElement;
let root: Root;

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

interface HomeSpec {
  vendor: string;
  project?: boolean;
}

const skill = (
  name: string,
  homes: HomeSpec[],
  extra: { bytes?: number; description?: string; argumentHint?: string } = {},
) => ({
  name,
  description: extra.description ?? `${name} 설명`,
  global: homes.every((h) => !h.project),
  source: homes[0]?.vendor ?? "praxis",
  homes: homes.map((h) => ({
    vendor: h.vendor,
    path: `/home/.${h.vendor}/${name}`,
    project: h.project ?? false,
  })),
  bytes: extra.bytes ?? 1024,
  argumentHint: extra.argumentHint ?? "",
  orphan: homes.length === 0,
});

const mount = async () => {
  await act(async () => {
    root.render(<SkillsView repo="/repo" />);
  });
};

const segments = () => [...container.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
const segmentLabels = () =>
  segments().map((b) => (b.textContent ?? "").replace(/\d+$/, "").trim());
const cardNames = () =>
  [...container.querySelectorAll("span.font-code")]
    .map((el) => (el.textContent ?? "").trim())
    .filter((t) => t.startsWith("/"));

const typeSearch = async (value: string) => {
  const input = container.querySelector<HTMLInputElement>('input[placeholder="이름·설명 검색"]')!;
  const setter = Object.getOwnPropertyDescriptor(
    window.HTMLInputElement.prototype,
    "value",
  )!.set!;
  await act(async () => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
};

beforeEach(() => {
  resetExperimentalFeaturesForTest(false);
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  mocks.list.mockReset();
  mocks.read.mockReset().mockResolvedValue("본문");
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("SkillsView — 뷰어", () => {
  it("벤더 디렉터리에서 온 목록을 그대로 보여 준다", async () => {
    mocks.list.mockResolvedValue([
      skill("alpha", [{ vendor: "claude" }]),
      skill("beta", [{ vendor: "codex" }]),
    ]);
    await mount();

    expect(cardNames()).toEqual(["/alpha", "/beta"]);
  });

  it("액션 버튼이 없다 — 저장하지 않으므로 추가·삭제·가져오기가 없다", async () => {
    mocks.list.mockResolvedValue([skill("alpha", [{ vendor: "claude" }])]);
    await mount();

    const labels = [...container.querySelectorAll("button")].map((b) => b.textContent ?? "");
    for (const forbidden of ["추가", "삭제", "가져오기"]) {
      expect(labels.some((l) => l.includes(forbidden))).toBe(false);
    }
  });

  it("렌즈를 고르면 그 벤더에 사는 것만 남는다", async () => {
    mocks.list.mockResolvedValue([
      skill("alpha", [{ vendor: "claude" }]),
      skill("beta", [{ vendor: "codex" }]),
      skill("gamma", [{ vendor: "antigravity" }]),
    ]);
    await mount();

    expect(segmentLabels()).toContain("Codex");
    const codex = segments().find((b) => (b.textContent ?? "").startsWith("Codex"))!;
    await act(async () => codex.click());

    expect(cardNames()).toEqual(["/beta"]);
  });

  it("아무도 살지 않는 벤더 칸은 렌더하지 않는다", async () => {
    mocks.list.mockResolvedValue([
      skill("alpha", [{ vendor: "claude" }]),
      skill("beta", [{ vendor: "claude" }]),
    ]);
    await mount();

    expect(segmentLabels()).not.toContain("Codex");
    expect(segmentLabels()).not.toContain("Agy");
  });

  it("렌즈를 고르면 발동 방식이 붙는다 — 다리로 도는 것은 Dojang 확장", async () => {
    mocks.list.mockResolvedValue([
      skill("owned", [{ vendor: "codex" }]),
      skill("borrowed", [{ vendor: "claude" }]),
    ]);
    await mount();

    const codex = segments().find((b) => (b.textContent ?? "").startsWith("Codex"))!;
    await act(async () => codex.click());

    expect(container.textContent).toContain("네이티브");
    expect(container.textContent).not.toContain("Dojang 확장");
  });

  it("검색은 이름과 설명을 대소문자 무시로 훑는다", async () => {
    mocks.list.mockResolvedValue([
      skill("alpha", [{ vendor: "claude" }], { description: "디자인 시스템" }),
      skill("beta", [{ vendor: "claude" }], { description: "코드 리뷰" }),
    ]);
    await mount();

    await typeSearch("ALPH");
    expect(cardNames()).toEqual(["/alpha"]);

    await typeSearch("리뷰");
    expect(cardNames()).toEqual(["/beta"]);
  });

  it("정렬을 바꾸면 순서가 바뀐다", async () => {
    mocks.list.mockResolvedValue([
      skill("small", [{ vendor: "claude" }], { bytes: 100 }),
      skill("big", [{ vendor: "claude" }], { bytes: 9000 }),
    ]);
    await mount();

    expect(cardNames()).toEqual(["/big", "/small"]); // 이름순

    const chip = [...container.querySelectorAll("button")].find(
      (b) => (b.textContent ?? "").includes("이름순"),
    )!;
    await act(async () => chip.click());
    const bySize = [...container.querySelectorAll('[role="option"]')].find(
      (b) => (b.textContent ?? "").includes("큰 순"),
    )! as HTMLButtonElement;
    await act(async () => bySize.click());

    expect(cardNames()).toEqual(["/big", "/small"]);
  });

  it("고아가 있으면 배너로 옮길 경로를 알려 준다", async () => {
    mocks.list.mockResolvedValue([
      skill("alpha", [{ vendor: "claude" }]),
      skill("stranded", []),
    ]);
    await mount();

    expect(container.textContent).toContain("stranded");
    expect(container.textContent).toContain("~/.claude/skills/<이름>/SKILL.md");
    expect(container.textContent).toContain("경로 복사");
  });

  it("고아가 없으면 배너를 띄우지 않는다", async () => {
    mocks.list.mockResolvedValue([skill("alpha", [{ vendor: "claude" }])]);
    await mount();

    expect(container.textContent).not.toContain("경로 복사");
  });

  // 스킬 사용량은 인사이트 "방식"에만 둔다 — ADR 0191 결정 2.
  it("실적 수치를 스킬 카드에 싣지 않는다", async () => {
    mocks.list.mockResolvedValue([skill("alpha", [{ vendor: "claude" }])]);
    await mount();

    expect(container.textContent).not.toContain("호출 기준");
    expect(container.textContent).not.toContain("많이 쓴 순");
  });

  it("카드를 누르면 본문을 펼친다 — 여는 동작은 이것 하나뿐이다", async () => {
    mocks.list.mockResolvedValue([skill("alpha", [{ vendor: "claude" }])]);
    mocks.read.mockResolvedValue("스킬 본문입니다");
    await mount();

    const card = [...container.querySelectorAll("button")].find((b) =>
      (b.textContent ?? "").includes("/alpha"),
    )!;
    await act(async () => card.click());

    expect(container.textContent).toContain("스킬 본문입니다");
    expect(mocks.read).toHaveBeenCalledWith("/repo", "alpha");
  });

  it("고급·실험 기능이 꺼져 있으면(기본값) 경험 패널을 렌더하지 않는다", async () => {
    mocks.list.mockResolvedValue([skill("alpha", [{ vendor: "claude" }])]);
    await mount();

    expect(container.querySelector('[aria-label="하네스 선택"]')).toBeNull();
  });

  it("고급·실험 기능을 켜면 경험 패널이 나타난다", async () => {
    resetExperimentalFeaturesForTest(true);
    mocks.list.mockResolvedValue([skill("alpha", [{ vendor: "claude" }])]);
    await mount();

    expect(container.querySelector('[aria-label="하네스 선택"]')).not.toBeNull();
  });
});
