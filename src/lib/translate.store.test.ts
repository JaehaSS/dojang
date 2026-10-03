import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

/** 모듈 상태(읽기 여부·저장 순서)를 테스트마다 새로 시작한다. */
async function freshStore() {
  vi.resetModules();
  return import("./translate");
}

const deferred = <T,>() => {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
};

describe("translate 설정 저장", () => {
  beforeEach(() => invoke.mockReset());

  it("읽기가 끝나기 전에 저장하면 저장된 다른 필드를 기본값으로 덮지 않는다", async () => {
    const stored = deferred<unknown>();
    invoke.mockImplementation((cmd: string) => (cmd === "translate_settings_get" ? stored.promise : Promise.resolve()));
    const store = await freshStore();

    const saved = store.saveTranslateSettings({ composer_mode: "replace" });
    stored.resolve({ composer_mode: "reference", model: "haiku" });
    await saved;

    expect(invoke).toHaveBeenCalledWith("translate_settings_set", { settings: { composer_mode: "replace", model: "haiku" } });
  });

  it("연달아 저장하면 앞의 변경 위에 합친다", async () => {
    invoke.mockImplementation((cmd: string) =>
      cmd === "translate_settings_get" ? Promise.resolve({ composer_mode: "reference", model: "sonnet" }) : Promise.resolve(),
    );
    const store = await freshStore();

    await Promise.all([store.saveTranslateSettings({ composer_mode: "replace" }), store.saveTranslateSettings({ model: "haiku" })]);

    expect(invoke).toHaveBeenLastCalledWith("translate_settings_set", { settings: { composer_mode: "replace", model: "haiku" } });
  });

  it("저장된 값을 읽지 못하면 저장하지 않는다", async () => {
    invoke.mockImplementation((cmd: string) =>
      cmd === "translate_settings_get" ? Promise.reject("DB가 초기화되지 않았습니다") : Promise.resolve(),
    );
    const store = await freshStore();

    await expect(store.saveTranslateSettings({ model: "haiku" })).rejects.toThrow("읽지 못해");
    expect(invoke).not.toHaveBeenCalledWith("translate_settings_set", expect.anything());
  });
});
