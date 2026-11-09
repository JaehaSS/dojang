// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RunTab } from "./RunTab";
import type { ModelOption } from "../../../lib/models";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const ipc = vi.hoisted(() => ({ available: vi.fn(), defaults: vi.fn(), save: vi.fn() }));
vi.mock("../../../lib/ipc", async (original) => ({
  ...(await original<typeof import("../../../lib/ipc")>()),
  agentAvailableModels: ipc.available,
  agentModelsGet: ipc.defaults,
  agentModelSet: ipc.save,
}));
vi.mock("../../../lib/use-observed-models", () => ({ useObservedModels: () => [] }));
vi.mock("./TranslateSection", () => ({ TranslateSection: () => null }));

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  ipc.defaults.mockReset().mockResolvedValue({ agy: "my-custom-model" });
  ipc.available.mockReset().mockResolvedValue([]);
  ipc.save.mockReset().mockResolvedValue(undefined);
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

const input = () => container.querySelector<HTMLInputElement>('input[aria-label="Antigravity · Gemini 모델"]');
const options = () => [...container.querySelectorAll<HTMLOptionElement>("#praxis-models-agy option")];

describe("작업 실행 설정의 CLI 모델 목록", () => {
  it("조회 완료 후 후보를 갱신하고 입력 중인 기본 모델은 보존한다", async () => {
    let resolve!: (models: ModelOption[]) => void;
    ipc.available.mockReturnValue(new Promise<ModelOption[]>((done) => { resolve = done; }));
    await act(async () => root.render(<RunTab onUseWorktreeChange={() => undefined} />));
    expect(input()?.value).toBe("my-custom-model");
    input()!.value = "draft-model";
    await act(async () => resolve([{ id: "gemini-4-argon-high", label: "Gemini 4 Argon (High)" }]));
    expect(ipc.available).toHaveBeenCalledWith("agy");
    expect(options().map((option) => option.value)).toEqual(["gemini-4-argon-high"]);
    expect(input()?.value).toBe("draft-model");
    expect(ipc.save).not.toHaveBeenCalled();
  });

  it("조회 실패 시 내장 후보와 저장된 모델을 유지한다", async () => {
    ipc.available.mockRejectedValue(new Error("offline"));
    await act(async () => root.render(<RunTab onUseWorktreeChange={() => undefined} />));
    expect(options().some((option) => option.value === "gemini-3.8-flash-high")).toBe(true);
    expect(input()?.value).toBe("my-custom-model");
    expect(ipc.save).not.toHaveBeenCalled();
  });
});
