import { beforeEach, describe, expect, it, vi } from "vitest";
import { readSessionImage } from "./session-image-reader";

const mocks = vi.hoisted(() => ({
  transport: { kind: "local", fsRead: vi.fn() },
  getTransport: vi.fn(),
  readLocalFile: vi.fn(),
}));
vi.mock("./transport", () => ({ getTransport: mocks.getTransport }));
vi.mock("./ipc", () => ({ readLocalFile: mocks.readLocalFile }));
beforeEach(() => {
  vi.clearAllMocks();
  mocks.transport.kind = "local";
  mocks.getTransport.mockReturnValue(mocks.transport);
});

describe("Session image reader host routing", () => {
  it("uses the guarded native reader for local absolute paths", async () => {
    const ref = { host: "local", id: 7 };
    await readSessionImage(ref, "/Users/owner/output.png");
    expect(mocks.readLocalFile).toHaveBeenCalledWith(ref, "/Users/owner/output.png");
    expect(mocks.transport.fsRead).not.toHaveBeenCalled();
  });
  it("reads relative images from the selected task workspace", async () => {
    await readSessionImage({ host: "local", id: 7 }, "assets/output.png");
    expect(mocks.transport.fsRead).toHaveBeenCalledWith(7, "assets/output.png");
    expect(mocks.readLocalFile).not.toHaveBeenCalled();
  });
  it.each(["assets/output.png", "/allowed/repository/output.png"])("uses the remote transport for %s", async (path) => {
    mocks.transport.kind = "runner";
    await readSessionImage({ host: "mini", id: 8 }, path);
    expect(mocks.getTransport).toHaveBeenCalledWith("mini");
    expect(mocks.transport.fsRead).toHaveBeenCalledWith(8, path);
    expect(mocks.readLocalFile).not.toHaveBeenCalled();
  });
  it("preserves a denied file read without falling back to another host", async () => {
    mocks.transport.kind = "runner";
    mocks.transport.fsRead.mockRejectedValueOnce(new Error("denied"));
    await expect(readSessionImage({ host: "mini", id: 8 }, "/private/output.png")).rejects.toThrow("denied");
    expect(mocks.readLocalFile).not.toHaveBeenCalled();
  });
});
