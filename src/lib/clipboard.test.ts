import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { copyText } from "./clipboard";

const writeText = vi.fn();

beforeEach(() => {
  invoke.mockReset();
  writeText.mockReset().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
});

afterEach(() => {
  delete (globalThis as { isTauri?: boolean }).isTauri;
});

describe("copyText", () => {
  it("writes through the native command inside the app, without touching the web clipboard", async () => {
    (globalThis as { isTauri?: boolean }).isTauri = true;
    invoke.mockResolvedValue(undefined);

    await copyText("결론");

    expect(invoke).toHaveBeenCalledWith("clipboard_write_text", { text: "결론" });
    expect(writeText).not.toHaveBeenCalled();
  });

  it("falls back to the web clipboard when the native command is unavailable", async () => {
    (globalThis as { isTauri?: boolean }).isTauri = true;
    invoke.mockRejectedValue("unsupported");

    await copyText("결론");

    expect(writeText).toHaveBeenCalledWith("결론");
  });

  it("uses the web clipboard outside the app and surfaces its rejection", async () => {
    writeText.mockRejectedValue(new DOMException("denied", "NotAllowedError"));

    await expect(copyText("결론")).rejects.toThrow("denied");
    expect(invoke).not.toHaveBeenCalled();
  });
});
