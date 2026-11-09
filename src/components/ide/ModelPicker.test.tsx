// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ModelPicker, modelChipTitle } from "./ModelPicker";
import type { ModelOption } from "../../lib/models";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// 부분 mock — 이 트리가 import하는 다른 IPC를 통째로 지우지 않는다.
const ipc = vi.hoisted(() => ({ agentModelsGet: vi.fn(), agentAvailableModels: vi.fn() }));
vi.mock("../../lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../lib/ipc")>()),
  agentModelsGet: ipc.agentModelsGet,
  agentAvailableModels: ipc.agentAvailableModels,
}));

const observed = vi.hoisted(() => ({
  rows: [] as { agent: string; model: string; last_used_at: number }[],
}));
vi.mock("../../lib/use-observed-models", () => ({ useObservedModels: () => observed.rows }));

let container: HTMLDivElement | null = null;
let root: Root | null = null;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  observed.rows = [];
  ipc.agentModelsGet.mockReset().mockResolvedValue({});
  ipc.agentAvailableModels.mockReset().mockResolvedValue([]);
});

afterEach(async () => {
  await act(async () => root?.unmount());
  container?.remove();
  root = null;
  container = null;
  vi.restoreAllMocks();
});

const render = async (node: React.ReactElement) => {
  await act(async () => root?.render(node));
};

/** 칩은 트리의 첫 버튼이다 — 드롭다운은 열기 전까지 렌더되지 않는다. */
const chip = () => container?.querySelector("button");

describe("ModelPicker 칩 라벨", () => {
  it("관측이 없으면 요청값을 보여준다", async () => {
    await render(<ModelPicker agent="claude" model="claude-opus-5" onChange={() => undefined} />);
    expect(chip()?.textContent).toContain("Opus 5");
  });

  it("관측도 요청도 없으면 기본 상태로 남는다", async () => {
    await render(<ModelPicker agent="claude" model="" onChange={() => undefined} />);
    expect(chip()?.textContent).toContain("모델: 기본");
  });

  it("관측이 있으면 요청값 대신 그것을 보여준다 — 별칭이 무엇으로 풀렸는지가 드러난다", async () => {
    await render(
      <ModelPicker
        agent="claude"
        model="opus"
        observedModel="claude-opus-5[1m]"
        onChange={() => undefined}
      />,
    );
    // 요청은 `opus`(별칭)지만 실제로 도는 것은 1M 컨텍스트 판이다. 게이지 분모가 4배 갈린다.
    expect(chip()?.textContent).toContain("Opus 5 · 1M 컨텍스트");
    expect(chip()?.textContent).not.toContain("별칭");
  });

  it("오버라이드가 없어도 관측이 있으면 실제 모델을 보여준다", async () => {
    await render(
      <ModelPicker
        agent="claude"
        model=""
        observedModel="claude-sonnet-5"
        onChange={() => undefined}
      />,
    );
    expect(chip()?.textContent).toContain("Sonnet 5");
    expect(chip()?.textContent).not.toContain("모델: 기본");
  });

  it("칩 색은 계속 요청값이 가른다 — 관측이 있다고 기본 상태가 지정처럼 보이지 않는다", async () => {
    await render(
      <ModelPicker agent="claude" model="" observedModel="claude-sonnet-5" onChange={() => undefined} />,
    );
    expect(chip()?.className).toContain("text-text-secondary");
    expect(chip()?.className).not.toContain("text-primary-bright");
  });

  it("카탈로그에 없는 관측값은 원문 그대로 보여준다", async () => {
    await render(
      <ModelPicker
        agent="claude"
        model=""
        observedModel="claude-future-9"
        onChange={() => undefined}
      />,
    );
    expect(chip()?.textContent).toContain("claude-future-9");
  });

  it("빈 문자열·공백뿐인 관측은 없는 것으로 친다", async () => {
    await render(
      <ModelPicker agent="claude" model="opus" observedModel="   " onChange={() => undefined} />,
    );
    expect(chip()?.textContent).toContain("Opus (별칭)");
  });

  it("드롭다운의 체크는 요청값 기준이다 — 관측이 있어도 '기본'이 선택으로 남는다", async () => {
    await render(
      <ModelPicker
        agent="claude"
        model=""
        observedModel="claude-sonnet-5"
        onChange={() => undefined}
      />,
    );
    await act(async () => chip()?.dispatchEvent(new MouseEvent("click", { bubbles: true })));

    const items = [...(container?.querySelectorAll("button") ?? [])];
    const fallback = items.find((b) => b.textContent?.includes("기본 (설정값)"));
    // 고르는 대상은 요청값이므로, 관측이 드롭다운의 선택 표시를 옮기면 거짓이 된다.
    expect(fallback?.className).toContain("text-text");
    expect(fallback?.querySelector(".text-primary-bright")).not.toBeNull();
  });
});

describe("modelChipTitle", () => {
  it("요청만 있고 관측이 없으면 미확인임을 밝힌다", () => {
    expect(modelChipTitle({ requested: "sonnet", running: "", vendorDefault: "" })).toBe(
      "이 세션의 모델: sonnet — 아직 실행으로 확인되지 않았습니다",
    );
  });

  it("둘 다 없으면 벤더 기본값을 안내한다", () => {
    expect(modelChipTitle({ requested: "", running: "", vendorDefault: "opus" })).toContain(
      "벤더 기본값 (opus)",
    );
    expect(modelChipTitle({ requested: "", running: "", vendorDefault: "" })).toContain("CLI 기본");
  });

  it("요청과 관측이 같으면 확인된 지정으로 말한다", () => {
    expect(
      modelChipTitle({ requested: "claude-opus-5", running: "claude-opus-5", vendorDefault: "" }),
    ).toBe("실행 중: claude-opus-5 (이 세션에 지정)");
  });

  it("어긋나면 실행 중인 것을 앞세우고 요청을 함께 밝힌다", () => {
    const title = modelChipTitle({
      requested: "opus",
      running: "claude-opus-5[1m]",
      vendorDefault: "",
    });
    expect(title).toContain("실행 중: claude-opus-5[1m]");
    expect(title).toContain("이 세션의 모델: opus");
  });

  it("기본값인데 관측이 있으면 실행 중인 것과 기본값 출처를 함께 보여준다", () => {
    const title = modelChipTitle({ requested: "", running: "claude-sonnet-5", vendorDefault: "opus" });
    expect(title).toContain("실행 중: claude-sonnet-5");
    expect(title).toContain("벤더 기본값 (opus)");
  });
});

const menu = () => container?.querySelector<HTMLElement>("[role='listbox']") ?? null;
const search = (): HTMLInputElement | null =>
  container?.querySelector<HTMLInputElement>("input[role='combobox']") ?? null;
const options = (): HTMLElement[] => [
  ...(container?.querySelectorAll<HTMLElement>("[role='option']") ?? []),
];

const openMenu = async (node: React.ReactElement) => {
  await render(node);
  await act(async () => chip()?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
};

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

describe("ModelPicker 검색", () => {
  it("조회 중 이동한 커서는 목록이 바뀌면 현재 선택으로 돌아간다", async () => {
    let resolve!: (models: ModelOption[]) => void;
    ipc.agentAvailableModels.mockReturnValue(new Promise<ModelOption[]>((done) => { resolve = done; }));
    const onChange = vi.fn();
    await openMenu(<ModelPicker agent="agy" model="" onChange={onChange} />);
    await press("ArrowDown");
    await press("ArrowDown");
    await act(async () => resolve([
      { id: "gemini-4-argon-high", label: "Gemini 4 Argon (High)" },
      { id: "gemini-4-argon-low", label: "Gemini 4 Argon (Low)" },
    ]));
    expect(options()[0].dataset.cursor).toBe("true");
    await press("Enter");
    expect(onChange).toHaveBeenCalledWith("");
  });

  it("메뉴를 열면 CLI의 아르곤 목록을 조회하고 선택한 ID를 전달한다", async () => {
    ipc.agentAvailableModels.mockResolvedValue([
      { id: "gemini-4-argon-high", label: "Gemini 4 Argon (High)" },
    ]);
    const onChange = vi.fn();
    await render(<ModelPicker agent="agy" model="" onChange={onChange} />);
    expect(ipc.agentAvailableModels).not.toHaveBeenCalled();
    await act(async () => chip()?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(ipc.agentAvailableModels).toHaveBeenCalledWith("agy");
    expect(options().map((o) => o.textContent).join(" ")).toContain("Gemini 4 Argon (High)");
    expect(options().map((o) => o.textContent).join(" ")).not.toContain("Gemini 3.6");
    await act(async () => options()[1].click());
    expect(onChange).toHaveBeenCalledWith("gemini-4-argon-high");
    await act(async () => chip()?.click());
    expect(ipc.agentAvailableModels).toHaveBeenCalledTimes(2);
  });

  it("CLI 조회가 실패해도 기존 모델과 자유입력을 사용할 수 있다", async () => {
    ipc.agentAvailableModels.mockRejectedValue(new Error("offline"));
    const onChange = vi.fn();
    await openMenu(<ModelPicker agent="agy" model="" onChange={onChange} />);
    expect(options().map((o) => o.textContent).join(" ")).toContain("Gemini 3.8");
    await type("gemini-4-argon-low");
    await press("Enter");
    expect(onChange).toHaveBeenCalledWith("gemini-4-argon-low");
  });

  it("벤더를 바꾸면 이전 벤더의 조회 결과를 표시하지 않는다", async () => {
    ipc.agentAvailableModels.mockResolvedValue([
      { id: "gemini-4-argon-high", label: "Gemini 4 Argon (High)" },
    ]);
    await openMenu(<ModelPicker agent="agy" model="" onChange={() => undefined} />);
    await render(<ModelPicker agent="claude" model="" onChange={() => undefined} />);
    expect(options().map((o) => o.textContent).join(" ")).not.toContain("Argon");
    expect(options().map((o) => o.textContent).join(" ")).toContain("Opus");
    expect(ipc.agentAvailableModels).toHaveBeenCalledTimes(1);
  });

  it("필드는 하나다 — 검색어가 곧 모델 ID라고 placeholder가 말한다", async () => {
    await openMenu(<ModelPicker agent="agy" model="" onChange={() => undefined} />);
    expect(search()?.placeholder).toBe("검색 또는 모델 ID 입력");
    expect(menu()).not.toBeNull();
    // 옛 "커스텀 모델 입력" 블록이 사라졌다는 뜻 — 열린 메뉴에 입력 필드는 이것 하나뿐이다.
    expect(container?.querySelectorAll("input")).toHaveLength(1);
  });

  it("'기본 (설정값)'은 질의와 무관하게 늘 최상단이다 — 탈출구다", async () => {
    await openMenu(<ModelPicker agent="agy" model="" onChange={() => undefined} />);
    await type("gemini-3.6-flash");
    expect(options()[0]?.textContent).toContain("기본 (설정값)");
  });

  it("매치가 있어도 직접 지정 행이 맨 아래에 남는다 — 접두 질의를 조용히 삼키지 않는다", async () => {
    const onChange = vi.fn();
    await openMenu(<ModelPicker agent="agy" model="" onChange={onChange} />);
    await type("gemini-3.6-flash");
    // 기본 + 매치 3건(high·medium·low) + 직접 지정.
    expect(options()).toHaveLength(5);
    expect(options()[4]?.textContent).toContain("직접 지정: 'gemini-3.6-flash'");
    // 커서는 첫 매치에 선다.
    expect(search()?.getAttribute("aria-activedescendant")).toBe("model-opt-1");
    await press("Enter");
    expect(onChange).toHaveBeenCalledWith("gemini-3.6-flash-high");
  });

  it("↓로 직접 지정 행까지 가서 Enter면 질의가 그대로 모델 ID가 된다", async () => {
    const onChange = vi.fn();
    await openMenu(<ModelPicker agent="agy" model="" onChange={onChange} />);
    await type("gemini-3.6-flash");
    await press("ArrowDown");
    await press("ArrowDown");
    await press("ArrowDown");
    await press("Enter");
    expect(onChange).toHaveBeenCalledWith("gemini-3.6-flash");
    expect(search()).toBeNull(); // 확정하면 닫힌다
  });

  it("매치가 0건이면 빈 상태 문구와 직접 지정 행만 남고 Enter가 그 질의를 확정한다", async () => {
    const onChange = vi.fn();
    await openMenu(<ModelPicker agent="agy" model="" onChange={onChange} />);
    await type("없는모델");
    expect(options()).toHaveLength(2);
    expect(container?.textContent).toContain("'없는모델'와 일치하는 모델이 없습니다");
    await press("Enter");
    expect(onChange).toHaveBeenCalledWith("없는모델");
  });

  it("질의가 후보 id와 정확히 같으면 직접 지정 행을 만들지 않는다", async () => {
    await openMenu(<ModelPicker agent="claude" model="" onChange={() => undefined} />);
    await type("claude-opus-5");
    expect(options().length).toBeGreaterThan(1);
    expect(container?.textContent).not.toContain("직접 지정");
  });

  it("라벨로도 찾는다 — id에 없는 말이 통한다", async () => {
    await openMenu(<ModelPicker agent="claude" model="" onChange={() => undefined} />);
    await type("별칭");
    const labels = options().map((o) => o.textContent ?? "");
    expect(labels.filter((t) => t.includes("(별칭)"))).toHaveLength(3);
  });

  it("Esc는 질의를 비우는 단계 없이 곧바로 닫는다", async () => {
    await openMenu(<ModelPicker agent="agy" model="" onChange={() => undefined} />);
    await type("gemini");
    await press("Escape");
    expect(search()).toBeNull();
  });
});
